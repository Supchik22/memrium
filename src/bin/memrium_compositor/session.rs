//! Session backend: the compositor as a real desktop session on a TTY.
//!
//!   libseat  - permission to open the GPU and input devices without root (logind / seatd)
//!   DRM/KMS  - mode setting, two dumb buffers flipped on vblank, a hardware cursor plane
//!   libinput - keyboard, mouse, touchpad
//!
//! The glass renderer is wgpu running headless; each frame is rendered offscreen, copied into the
//! back dumb buffer and page-flipped. (Simple and portable; a zero-copy dmabuf path is future work.)
//!
//! NOTE: this backend needs real hardware and a free VT; it cannot run inside another desktop session.

use crate::state::{ClientState, State};
use memrium_de::display::{self, MonitorInfo, Placed};
use crate::Args;
use drm::buffer::{Buffer, DrmFourcc};
use drm::control::{self, connector, crtc, dumbbuffer::DumbBuffer, framebuffer, Device as ControlDevice, Event as DrmEvent, ModeTypeFlags, PageFlipFlags};
use drm::Device as DrmDevice;
use memrium_de::renderer::Renderer;
use input::event::keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait};
use input::event::pointer::{Axis, ButtonState, PointerEvent, PointerScrollEvent};
use input::event::gesture::{GestureEvent, GestureSwipeEvent};
use input::event::{DeviceEvent, EventTrait, Event as InputEvent};
use input::{AccelProfile, DeviceCapability};
use input::{Libinput, LibinputInterface};
use libseat::{Seat, SeatEvent};
use smithay::{reexports::wayland_server::Display, wayland::compositor::CompositorClientState};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

struct Card(OwnedFd);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl DrmDevice for Card {}
impl ControlDevice for Card {}

/// libinput opens evdev devices through libseat so no root is needed.
struct SeatInterface {
    seat: Rc<RefCell<Seat>>,
    devices: HashMap<RawFd, libseat::Device>,
}

impl LibinputInterface for SeatInterface {
    fn open_restricted(&mut self, path: &Path, _flags: i32) -> Result<OwnedFd, i32> {
        let dev = self.seat.borrow_mut().open_device(&path).map_err(|e| e.0)?;
        let fd = dev.as_fd().try_clone_to_owned().map_err(|e| e.raw_os_error().unwrap_or(5))?;
        self.devices.insert(fd.as_raw_fd(), dev);
        Ok(fd)
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        if let Some(dev) = self.devices.remove(&fd.as_raw_fd()) {
            let _ = self.seat.borrow_mut().close_device(dev);
        }
    }
}

/// An opened DRM device and what it can drive.
struct Gpu {
    card: Card,
    res: control::ResourceHandles,
}

impl Card {
    fn dup(&self) -> Card {
        Card(self.0.try_clone().expect("duplicate a DRM file descriptor"))
    }
}

/// A connected monitor the machine reports.
struct Found {
    gpu: usize,
    conn: connector::Info,
    info: MonitorInfo,
    /// the modes behind `info.modes`, in the same order
    modes: Vec<control::Mode>,
}

struct Output {
    card: Card,
    /// which of the opened DRM devices it belongs to
    gpu: usize,
    crtc: crtc::Handle,
    conn: connector::Handle,
    mode: control::Mode,
    bufs: Vec<(DumbBuffer, framebuffer::Handle)>,
    /// the two buffers stay mapped: mapping one anew every frame costs its page faults again
    maps: [Option<drm::control::dumbbuffer::DumbMapping<'static>>; 2],
    /// the same two buffers as GPU images the display scans out directly (no copy); None: the dumb buffers are used
    zc: Option<Zc>,
    cursor: DumbBuffer,
    size: (u32, u32),
    name: String,
    /// top-left on the renderer's canvas, device px
    pos: (i32, i32),
    flip_pending: bool,
    /// which of the two buffers is on screen
    front: usize,
    cursor_on: bool,
    /// the cursor picture's hot spot, device px
    cursor_hot: (i32, i32),
}

/// Two renderer-allocated images and their framebuffers.
struct Zc {
    imgs: [memrium_de::renderer::ScanoutImage; 2],
    fbs: [framebuffer::Handle; 2],
}

impl Output {
    /// The framebuffer of buffer `i`.
    fn fb(&self, i: usize) -> framebuffer::Handle {
        self.zc.as_ref().map_or(self.bufs[i].1, |z| z.fbs[i])
    }
}

/// A single-plane GPU buffer, as DRM wants to be told about it.
struct Planar {
    size: (u32, u32),
    modifier: u64,
    pitch: u32,
    offset: u32,
    handle: drm::buffer::Handle,
}

impl drm::buffer::PlanarBuffer for Planar {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn format(&self) -> DrmFourcc {
        DrmFourcc::Xrgb8888
    }
    fn modifier(&self) -> Option<drm::buffer::DrmModifier> {
        Some(self.modifier.into())
    }
    fn pitches(&self) -> [u32; 4] {
        [self.pitch, 0, 0, 0]
    }
    fn handles(&self) -> [Option<drm::buffer::Handle>; 4] {
        [Some(self.handle), None, None, None]
    }
    fn offsets(&self) -> [u32; 4] {
        [self.offset, 0, 0, 0]
    }
}

/// Make `o` show GPU images with this layout: allocate two, wrap them as framebuffers and switch the CRTC to the first.
fn try_zero_copy(o: &mut Output, device: &wgpu::Device, queue: &wgpu::Queue, modifier: u64) -> Result<(), String> {
    let (w, h) = o.size;
    let mut imgs = Vec::new();
    let mut fbs: Vec<framebuffer::Handle> = Vec::new();
    let result = (|| -> Result<(), String> {
        for _ in 0..2 {
            let img = memrium_de::renderer::create_scanout(device, w, h, modifier)?;
            let handle = o.card.prime_fd_to_buffer(img.fd.as_fd()).map_err(|e| format!("prime import: {e}"))?;
            let planar = Planar { size: (w, h), modifier: img.modifier, pitch: img.pitch, offset: img.offset, handle };
            let fb = o.card.add_planar_framebuffer(&planar, control::FbCmd2Flags::MODIFIERS);
            // the framebuffer keeps the buffer alive
            let _ = o.card.close_buffer(handle);
            fbs.push(fb.map_err(|e| format!("framebuffer: {e}"))?);
            imgs.push(img);
        }
        // start from black, not from whatever the memory held
        let mut enc = device.create_command_encoder(&Default::default());
        for img in &imgs {
            img.clear(&mut enc);
        }
        queue.submit([enc.finish()]);
        let _ = device.poll(wgpu::PollType::Wait);
        o.card.set_crtc(o.crtc, Some(fbs[0]), (0, 0), &[o.conn], Some(o.mode)).map_err(|e| format!("set_crtc: {e}"))
    })();
    match result {
        Ok(()) => {
            let (i1, i0) = (imgs.pop().unwrap(), imgs.pop().unwrap());
            o.zc = Some(Zc { imgs: [i0, i1], fbs: [fbs[0], fbs[1]] });
            o.front = 0;
            Ok(())
        }
        Err(e) => {
            for fb in fbs {
                let _ = o.card.destroy_framebuffer(fb);
            }
            Err(e)
        }
    }
}

/// Let the monitors that hang on the rendering GPU scan the renderer's images out directly. The others keep their dumb
/// buffers. `MEMRIUM_NO_ZEROCOPY=1` turns it off.
fn go_zero_copy(outs: &mut [Output], device: &wgpu::Device, queue: &wgpu::Queue) {
    if std::env::var_os("MEMRIUM_NO_ZEROCOPY").is_some() {
        return;
    }
    let Some(gpu_dev) = memrium_de::renderer::primary_dev(device) else { return };
    let mods = memrium_de::renderer::scanout_modifiers(device);
    if mods.is_empty() {
        eprintln!("zero-copy: the GPU cannot export scanout buffers");
        return;
    }
    for o in outs.iter_mut() {
        if o.zc.is_some() {
            continue;
        }
        let same = rustix::fs::fstat(&o.card).is_ok_and(|s| s.st_rdev as u64 == gpu_dev);
        if !same {
            eprintln!("zero-copy: {} hangs on another GPU than the renderer; copying", o.name);
            continue;
        }
        let mut last = String::new();
        for &m in &mods {
            match try_zero_copy(o, device, queue, m) {
                Ok(()) => {
                    eprintln!("zero-copy: {} scans out GPU images (modifier {m:#x})", o.name);
                    break;
                }
                Err(e) => last = format!("modifier {m:#x}: {e}"),
            }
        }
        if o.zc.is_none() {
            eprintln!("zero-copy: {} stays on dumb buffers ({last})", o.name);
            // the failed attempts may have left the CRTC without a picture
            let _ = o.card.set_crtc(o.crtc, Some(o.bufs[o.front].1), (0, 0), &[o.conn], Some(o.mode));
        }
    }
}

/// The monitors set up now, how they sit, and the interface scale.
struct Screens {
    outs: Vec<Output>,
    placed: Vec<Placed>,
    scale: f32,
    /// size of the renderer's canvas (all monitors), device px
    canvas: (u32, u32),
}

/// "eDP-1", "DP-3", "HDMI-A-1": the names the kernel uses in /sys/class/drm.
fn connector_name(c: &connector::Info) -> String {
    use connector::Interface as I;
    let base = match c.interface() {
        I::EmbeddedDisplayPort => "eDP".to_string(),
        I::DisplayPort => "DP".to_string(),
        I::HDMIA => "HDMI-A".to_string(),
        I::HDMIB => "HDMI-B".to_string(),
        I::VGA => "VGA".to_string(),
        I::DVII => "DVI-I".to_string(),
        I::DVID => "DVI-D".to_string(),
        I::DVIA => "DVI-A".to_string(),
        I::LVDS => "LVDS".to_string(),
        I::DSI => "DSI".to_string(),
        other => format!("{other:?}"),
    };
    format!("{base}-{}", c.interface_id())
}

/// Open every DRM device the seat gives us.
fn open_gpus(seat: &Rc<RefCell<Seat>>) -> Result<Vec<Gpu>, String> {
    let mut cards: Vec<_> = std::fs::read_dir("/dev/dri").map_err(|e| format!("/dev/dri: {e}"))?.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card"))).collect();
    cards.sort();
    let mut gpus = Vec::new();
    let mut last_err = String::from("no /dev/dri/card* devices");
    for path in cards {
        let dev = match seat.borrow_mut().open_device(&path) {
            Ok(d) => d,
            Err(e) => {
                last_err = format!("{}: {e:?}", path.display());
                continue;
            }
        };
        let fd = dev.as_fd().try_clone_to_owned().map_err(|e| e.to_string())?;
        let card = Card(fd);
        // become the master so mode setting is allowed (libseat usually has done this already)
        let _ = card.acquire_master_lock();
        match card.resource_handles() {
            Ok(res) => gpus.push(Gpu { card, res }),
            Err(e) => last_err = format!("{}: {e}", path.display()),
        }
        // keep the libseat device alive for the whole session
        std::mem::forget(dev);
    }
    if gpus.is_empty() {
        Err(last_err)
    } else {
        Ok(gpus)
    }
}

/// Every connected monitor (`force` asks the hardware again instead of trusting the kernel's last word).
fn scan_monitors(gpus: &[Gpu], force: bool) -> Vec<Found> {
    let mut out = Vec::new();
    for (gi, gpu) in gpus.iter().enumerate() {
        for &h in gpu.res.connectors() {
            let Ok(conn) = gpu.card.get_connector(h, force) else { continue };
            if conn.state() != connector::State::Connected || conn.modes().is_empty() {
                continue;
            }
            // the preferred mode first, then the rest from big to small
            let mut modes: Vec<control::Mode> = conn.modes().to_vec();
            let pref = modes.iter().position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(0);
            let first = modes.remove(pref);
            modes.sort_by_key(|m| std::cmp::Reverse((m.size().0 as u32 * m.size().1 as u32, m.vrefresh())));
            modes.insert(0, first);
            let mut seen = std::collections::HashSet::new();
            modes.retain(|m| seen.insert((m.size().0, m.size().1, m.vrefresh())));
            let mm = conn.size().unwrap_or((0, 0));
            let info = MonitorInfo { name: connector_name(&conn), modes: modes.iter().map(|m| (m.size().0 as u32, m.size().1 as u32, m.vrefresh())).collect(), mm: (mm.0, mm.1) };
            out.push(Found { gpu: gi, conn, info, modes });
        }
    }
    out
}

fn pick_crtc(gpu: &Gpu, conn: &connector::Info, used: &[(usize, crtc::Handle)], gi: usize) -> Option<crtc::Handle> {
    let free = |c: &crtc::Handle| !used.contains(&(gi, *c));
    let current = conn.current_encoder().and_then(|e| gpu.card.get_encoder(e).ok()).and_then(|e| e.crtc());
    if let Some(c) = current.filter(|c| free(c)) {
        return Some(c);
    }
    conn.encoders().iter().filter_map(|&e| gpu.card.get_encoder(e).ok()).flat_map(|e| gpu.res.filter_crtcs(e.possible_crtcs())).find(|c| free(c))
}

/// Mode-set every placed monitor and build the canvas layout.
fn build_screens(gpus: &[Gpu], found: &[Found], prefs: &memrium_de::display::DisplayPrefs) -> Result<Screens, String> {
    let infos: Vec<MonitorInfo> = found.iter().map(|f| f.info.clone()).collect();
    let placed = display::place(&infos, prefs);
    if placed.is_empty() {
        return Err("no connected display".into());
    }
    let (x0, y0, x1, y1) = display::bounds(&placed);
    let mut outs: Vec<Output> = Vec::new();
    let mut used: Vec<(usize, crtc::Handle)> = Vec::new();
    for p in &placed {
        let Some(f) = found.iter().find(|f| f.info.name == p.name) else { continue };
        let gpu = &gpus[f.gpu];
        let Some(mode) = f.modes.iter().find(|m| m.size().0 as u32 == p.w && m.size().1 as u32 == p.h && m.vrefresh() == p.hz).copied() else { continue };
        let result = (|| -> Result<Output, String> {
            let crtc = pick_crtc(gpu, &f.conn, &used, f.gpu).ok_or("no free CRTC for the connector")?;
            let (w, h) = (p.w, p.h);
            let mut bufs = Vec::new();
            for _ in 0..2 {
                let db = gpu.card.create_dumb_buffer((w, h), DrmFourcc::Xrgb8888, 32).map_err(|e| format!("dumb buffer: {e}"))?;
                let fb = gpu.card.add_framebuffer(&db, 24, 32).map_err(|e| format!("framebuffer: {e}"))?;
                bufs.push((db, fb));
            }
            gpu.card.set_crtc(crtc, Some(bufs[0].1), (0, 0), &[f.conn.handle()], Some(mode)).map_err(|e| format!("set_crtc: {e}"))?;
            let cursor = gpu.card.create_dumb_buffer((64, 64), DrmFourcc::Argb8888, 32).map_err(|e| format!("cursor buffer: {e}"))?;
            used.push((f.gpu, crtc));
            Ok(Output { card: gpu.card.dup(), gpu: f.gpu, crtc, conn: f.conn.handle(), mode, bufs, maps: [None, None], zc: None, cursor, size: (w, h), name: p.name.clone(), pos: (p.x - x0, p.y - y0), flip_pending: false, front: 0, cursor_on: false, cursor_hot: (0, 0) })
        })();
        match result {
            Ok(o) => {
                eprintln!("display: {} {}x{} @{}Hz at {},{}{}", p.name, p.w, p.h, p.hz, p.x, p.y, if p.primary { " (main)" } else { "" });
                outs.push(o);
            }
            Err(e) => {
                eprintln!("display {}: {e}", p.name);
                if p.primary {
                    return Err(format!("{}: {e}", p.name));
                }
            }
        }
    }
    // a monitor that failed to start is not part of the desktop
    let placed: Vec<Placed> = placed.into_iter().filter(|p| outs.iter().any(|o| o.name == p.name)).collect();
    let main = infos.iter().find(|i| placed.iter().any(|p| p.primary && p.name == i.name)).cloned().ok_or("no main display")?;
    let main_mode = placed.iter().find(|p| p.primary).map(|p| (p.w, p.h, p.hz)).unwrap_or((1920, 1080, 60));
    let auto = display::ui_scale(&main, main_mode, prefs);
    let scale: f32 = std::env::var("MEMRIUM_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(auto);
    // the canvas must not shrink when a monitor failed
    let (bx0, by0, bx1, by1) = display::bounds(&placed);
    for o in &mut outs {
        o.pos = (placed.iter().find(|p| p.name == o.name).map_or(0, |p| p.x) - bx0, placed.iter().find(|p| p.name == o.name).map_or(0, |p| p.y) - by0);
    }
    let _ = (x1, y1);
    Ok(Screens { outs, placed, scale, canvas: ((bx1 - bx0) as u32, (by1 - by0) as u32) })
}

/// Hand a finished screens' buffers back (the old layout, once the new one works).
fn release_screens(screens: Screens) {
    for o in screens.outs {
        drop(o.maps);
        if let Some(z) = o.zc {
            for fb in z.fbs {
                let _ = o.card.destroy_framebuffer(fb);
            }
        }
        for (db, fb) in o.bufs {
            let _ = o.card.destroy_framebuffer(fb);
            let _ = o.card.destroy_dumb_buffer(db);
        }
        let _ = o.card.destroy_dumb_buffer(o.cursor);
    }
}

fn make_canvas(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> (wgpu::Texture, wgpu::Buffer, usize) {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scanout"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let padded = ((w * 4 + 255) / 256 * 256) as usize;
    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (padded * h as usize) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    (target, staging, padded)
}

/// The cursor picture for the current shape: its size follows the preference and the monitors' scale.
/// The desktop's secret service (`memrium_secrets`, built next to this program): programs that use libsecret and
/// NetworkManager find their passwords there. It asks for a passphrase through `memrium_askpass`. A keyring that is
/// already running (KWallet, GNOME's) keeps the bus name and the service steps aside by itself.
fn start_secrets() {
    if std::env::var_os("MEMRIUM_NO_SECRETS").is_some() {
        return;
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf)) else { return };
    let service = dir.join("memrium_secrets");
    if !service.exists() {
        eprintln!("secrets: {} is not built, passwords are not kept (cargo build --release --workspace)", service.display());
        return;
    }
    let mut cmd = std::process::Command::new(service);
    if std::env::var_os("MEMRIUM_ASKPASS").is_none() {
        cmd.env("MEMRIUM_ASKPASS", dir.join("memrium_askpass"));
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("secrets: cannot start memrium_secrets: {e}");
    }
}

fn cursor_picture(state: &State, scale: f32, phase: f32) -> memrium_de::cursor::CursorImage {
    let prefs = &state.config.input.cursor;
    if state.cursor_shape == memrium_de::cursor::Shape::Client {
        if let Some(c) = &state.client_cursor_img {
            return c.image(scale);
        }
    }
    memrium_de::cursor::render(state.cursor_shape, (prefs.size * scale).round().max(1.0) as u32, prefs.look, state.theme.accent, phase)
}

/// Put a cursor picture on the monitor's hardware cursor plane (a 64 x 64 buffer).
fn draw_cursor(out: &mut Output, img: &memrium_de::cursor::CursorImage) {
    if let Ok(mut map) = out.card.map_dumb_buffer(&mut out.cursor) {
        let pitch = 64 * 4;
        let n = img.size as usize;
        for y in 0..64usize {
            for x in 0..64usize {
                let px = if x < n && y < n { img.argb[y * n + x] } else { 0 };
                let i = y * pitch + x * 4;
                map[i..i + 4].copy_from_slice(&px.to_le_bytes());
            }
        }
    }
    out.cursor_hot = img.hot;
    let _ = out.card.set_cursor(out.crtc, Some(&out.cursor));
}

/// Pointer speed, acceleration, scrolling and tapping for one device (config, or KDE's values for it).
fn apply_input(d: &mut input::Device, prefs: &memrium_de::input::InputPrefs, kde_text: &str) {
    use memrium_de::input::{effective, kde_device, Accel};
    let kde = kde_device(kde_text, d.id_vendor(), d.id_product());
    let (speed, accel, natural, tap, left) = effective(prefs, kde.as_ref());
    if d.has_capability(DeviceCapability::Pointer) {
        if d.config_accel_is_available() {
            let _ = d.config_accel_set_profile(if accel == Accel::Flat { AccelProfile::Flat } else { AccelProfile::Adaptive });
            let _ = d.config_accel_set_speed(speed as f64);
            eprintln!("input: {} speed {speed:.2} {accel:?}{}", d.name(), if kde.is_some() { " (from KDE)" } else { "" });
        }
        if d.config_scroll_has_natural_scroll() {
            // "natural" is a touchpad habit: a wheel mouse keeps the classic direction unless KDE had set it for that very mouse
            let touchpad = d.config_tap_finger_count() > 0;
            let natural = if touchpad { natural } else { kde.as_ref().and_then(|k| k.natural_scroll).unwrap_or(false) };
            let _ = d.config_scroll_set_natural_scroll_enabled(natural);
        }
        if d.config_left_handed_is_available() {
            let _ = d.config_left_handed_set(left);
        }
        if d.config_tap_finger_count() > 0 {
            let _ = d.config_tap_set_enabled(tap);
        }
    }
}

pub fn run(args: Args) {
    // the colours the shell gives KDE programs go back to what they were when the session ends
    let _kde_colors = memrium_de::kdecolors::Guard;
    // --- seat: permission to use the GPU and input devices
    let seat_events: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let seat = {
        let ev = seat_events.clone();
        Seat::open(move |seat, e| match e {
            SeatEvent::Enable => ev.set(Some(true)),
            SeatEvent::Disable => {
                ev.set(Some(false));
                let _ = seat.disable();
            }
        })
        .unwrap_or_else(|e| {
            eprintln!("libseat: could not open a seat ({e:?}). Run this from a TTY login with seatd or logind available.");
            std::process::exit(1);
        })
    };
    let seat = Rc::new(RefCell::new(seat));
    // the first dispatch delivers the initial "enable"
    let mut waited = 0;
    while seat_events.get() != Some(true) && waited < 50 {
        let _ = seat.borrow_mut().dispatch(100);
        waited += 1;
    }
    seat_events.set(None);

    // --- displays: every connected monitor, laid out as one desktop
    let prefs = memrium_de::theme::Config::load().display;
    let gpus = open_gpus(&seat).unwrap_or_else(|e| {
        eprintln!("could not set up a display: {e}");
        std::process::exit(1);
    });
    let mut found = scan_monitors(&gpus, true);
    let mut screens = build_screens(&gpus, &found, &prefs).unwrap_or_else(|e| {
        eprintln!("could not set up a display: {e}");
        std::process::exit(1);
    });
    let mut scale = screens.scale;
    let (w, h) = screens.canvas;
    eprintln!("scale {scale}, desktop {w}x{h}");

    // --- GPU renderer (headless wgpu)
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: if std::env::var("MEMRIUM_GPU").as_deref() == Ok("high") { wgpu::PowerPreference::HighPerformance } else { wgpu::PowerPreference::LowPower }, ..Default::default() })).expect("no GPU adapter");
    eprintln!("adapter: {}", adapter.get_info().name);
    let (device, queue) = memrium_de::renderer::create_device(&adapter, &wgpu::DeviceDescriptor::default()).expect("GPU device");
    let format = wgpu::TextureFormat::Bgra8Unorm; // byte order B,G,R,A = DRM XRGB8888
    let renderer = Renderer::new(device.clone(), queue.clone(), format, w, h, scale, crate::load_wallpaper(&args));
    let (mut target, mut staging, mut padded) = make_canvas(&device, format, w, h);
    go_zero_copy(&mut screens.outs, &device, &queue);

    // --- wayland side
    let mut display: Display<State> = Display::new().unwrap();
    let dh = display.handle();
    let listener = smithay::reexports::wayland_server::ListeningSocket::bind_auto("memrium", 1..32).expect("could not create a wayland socket");
    let socket = listener.socket_name().unwrap().to_string_lossy().to_string();
    eprintln!("listening on WAYLAND_DISPLAY={socket}");
    std::env::set_var("WAYLAND_DISPLAY", &socket);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::set_var("XDG_CURRENT_DESKTOP", "Memrium");
    let mut state = State::new(&dh, renderer, (w, h), scale);
    state.monitor_infos = found.iter().map(|f| f.info.clone()).collect();
    state.apply_layout(&screens.placed, scale);
    // timers and the X server's sockets (XWayland) run on a calloop that is dispatched from this loop
    let mut calloop: calloop::EventLoop<'static, State> = calloop::EventLoop::try_new().expect("event loop");
    state.init_dmabuf();
    state.init_screencopy();
    state.start_notifications();
    state.tray.start();
    state.appmenu.start();
    state.export_env = true;
    state.start_xwayland(calloop.handle());
    state.socket = socket.clone();
    state.default_wallpaper = Some(std::path::PathBuf::from(crate::wallpaper_path(&args)));
    state.init_wallpaper();
    start_secrets();
    if let Some(cmd) = &args.exec {
        let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
    }
    // autostart: one command per line in ~/.config/memrium/autostart
    if let Ok(text) = std::fs::read_to_string(memrium_de::theme::config_dir().join("autostart")) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let _ = std::process::Command::new("sh").arg("-c").arg(line).spawn();
        }
    }
    let (mut drawn_gen, mut drawn_step, mut drawn_prefs) = (u32::MAX, u32::MAX, state.config.input.cursor.clone());
    {
        let img = cursor_picture(&state, scale, 0.0);
        for o in &mut screens.outs {
            draw_cursor(o, &img);
        }
    }

    // --- input
    let mut libinput = Libinput::new_with_udev(SeatInterface { seat: seat.clone(), devices: HashMap::new() });
    libinput.udev_assign_seat("seat0").expect("libinput: could not assign seat0");
    let [mw, mh] = state.size;
    let mut cursor = [mw / 2.0, mh / 2.0];
    state.pointer_motion(cursor[0], cursor[1]);

    let mut devices: Vec<input::Device> = Vec::new();
    let kde_text = memrium_de::input::kcminputrc();
    // --- main loop
    let mut active = true;
    let mut dpms_off = false;
    let mut flip_failures = 0u32;
    let mut last = Instant::now();
    // what the frame before the last one left to be copied into the next back buffer (see `render_to_dumb`)
    let mut dumb_prev: Option<Option<(u32, u32, u32, u32)>> = None;
    // a frame already drawn and read back into `staging`, waiting for the flip in flight to finish so it can be copied into
    // the back buffer: the GPU works on the next frame while the display waits for its vblank
    let mut ahead: Option<Ahead> = None;
    let no_ahead = std::env::var_os("MEMRIUM_NO_AHEAD").is_some();
    let mut idle = false;
    let mut swipe_last = Instant::now();
    let mut last_probe = Instant::now();
    // `MEMRIUM_STATS=1`: how long the frames take, written to the log every five seconds
    let stats_on = std::env::var_os("MEMRIUM_STATS").is_some();
    let (mut st_n, mut st_sum, mut st_max, mut st_since) = (0u32, 0.0f32, 0.0f32, Instant::now());
    let mut stages = [0.0f32; 4];
    state.allow_skip = true;
    state.persistent_target = true;
    loop {
        // wait for something to do: events, or a vblank when a flip is in flight
        let flip_pending = screens.outs.iter().any(|o| o.flip_pending);
        {
            let seat_fd = seat.borrow_mut().get_fd().map(|f| f.as_raw_fd()).ok();
            let wl_fd = display.backend().poll_fd().as_raw_fd();
            let calloop_fd = std::os::fd::AsFd::as_fd(&calloop).as_raw_fd();
            let mut raw: Vec<Option<RawFd>> = vec![Some(wl_fd), Some(libinput.as_raw_fd()), Some(listener.as_raw_fd()), seat_fd, Some(calloop_fd)];
            // one fd per DRM device, for the vblank events
            let mut seen_gpu = Vec::new();
            for o in &screens.outs {
                if !seen_gpu.contains(&o.gpu) {
                    seen_gpu.push(o.gpu);
                    raw.push(Some(o.card.as_fd().as_raw_fd()));
                }
            }
            let mut fds: Vec<libc::pollfd> = raw.into_iter().flatten().map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
            // not waiting for a flip (or inactive) means there is a frame to draw right now
            // idle (the last frame was identical): wake ~30 times a second for the clock and slow clients; input and
            // client commits wake the poll at once
            let timeout: i32 = if flip_pending || !active { 100 } else if idle { 33 } else { 0 };
            // SAFETY: `fds` is a valid array of `fds.len()` pollfd structs
            unsafe {
                libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout);
            }
        }

        // seat: VT switches
        let _ = seat.borrow_mut().dispatch(0);
        if let Some(on) = seat_events.take() {
            if on {
                active = true;
                let _ = libinput.resume();
                for o in &mut screens.outs {
                    let _ = o.card.acquire_master_lock();
                    let _ = o.card.set_crtc(o.crtc, Some(o.fb(o.front)), (0, 0), &[o.conn], Some(o.mode));
                    draw_cursor(o, &cursor_picture(&state, scale, 0.0));
                    o.flip_pending = false;
                    o.cursor_on = true;
                }
                dumb_prev = None;
                drop_ahead(&mut ahead, &staging);
                state.invalidate();
                idle = false;
            } else {
                active = false;
                libinput.suspend();
                for o in &screens.outs {
                    let _ = o.card.release_master_lock();
                }
            }
        }

        // input
        let _ = libinput.dispatch();
        if state.input_dirty {
            state.input_dirty = false;
            for d in &mut devices {
                apply_input(d, &state.config.input, &kde_text);
            }
        }
        for ev in &mut libinput {
            match ev {
                InputEvent::Device(DeviceEvent::Added(a)) => {
                    let mut d = a.device();
                    apply_input(&mut d, &state.config.input, &kde_text);
                    devices.push(d);
                }
                InputEvent::Device(DeviceEvent::Removed(r)) => {
                    let name = r.device().sysname().to_string();
                    devices.retain(|d| d.sysname() != name);
                }
                InputEvent::Keyboard(KeyboardEvent::Key(k)) => state.key(k.key(), k.key_state() == KeyState::Pressed),
                InputEvent::Pointer(PointerEvent::Motion(m)) => {
                    // programs that asked for raw motion get it; a locked pointer (a game) stays where it is
                    state.pointer_relative(m.dx_unaccelerated() as f64 / scale as f64, m.dy_unaccelerated() as f64 / scale as f64);
                    if !state.pointer_locked() {
                        // the pointer moves over the whole desktop, from one monitor to the next
                        cursor = state.clamp_to_monitors([cursor[0] + m.dx() as f32 / scale, cursor[1] + m.dy() as f32 / scale]);
                        state.pointer_motion(cursor[0], cursor[1]);
                    }
                }
                InputEvent::Pointer(PointerEvent::MotionAbsolute(m)) => {
                    let [cw, ch] = state.renderer.logical_size();
                    cursor = state.clamp_to_monitors([state.world[0] + m.absolute_x_transformed(cw as u32) as f32, state.world[1] + m.absolute_y_transformed(ch as u32) as f32]);
                    state.pointer_motion(cursor[0], cursor[1]);
                }
                InputEvent::Pointer(PointerEvent::Button(b)) => {
                    use input::event::pointer::PointerEventTrait;
                    let _ = b.time();
                    state.pointer_button(input::event::pointer::PointerButtonEvent::button(&b), input::event::pointer::PointerButtonEvent::button_state(&b) == ButtonState::Pressed);
                }
                InputEvent::Gesture(GestureEvent::Swipe(sw)) => {
                    use input::event::gesture::{GestureEndEvent, GestureEventCoordinates, GestureEventTrait};
                    // three or four fingers going sideways move the desktops
                    match sw {
                        GestureSwipeEvent::Begin(b) if b.finger_count() >= 3 => state.ws_swipe_begin(),
                        GestureSwipeEvent::Update(u) if u.finger_count() >= 3 => {
                            let now = Instant::now();
                            let dt = (now - swipe_last).as_secs_f32();
                            swipe_last = now;
                            state.ws_swipe_move(u.dx() as f32, dt);
                        }
                        GestureSwipeEvent::End(e) => state.ws_swipe_end(e.cancelled()),
                        _ => {}
                    }
                }
                InputEvent::Pointer(PointerEvent::ScrollWheel(s)) => {
                    let dy = if s.has_axis(Axis::Vertical) { s.scroll_value(Axis::Vertical) } else { 0.0 };
                    let dx = if s.has_axis(Axis::Horizontal) { s.scroll_value(Axis::Horizontal) } else { 0.0 };
                    state.pointer_axis(dx, dy);
                }
                InputEvent::Pointer(PointerEvent::ScrollFinger(s)) => {
                    let dy = if s.has_axis(Axis::Vertical) { s.scroll_value(Axis::Vertical) } else { 0.0 };
                    let dx = if s.has_axis(Axis::Horizontal) { s.scroll_value(Axis::Horizontal) } else { 0.0 };
                    state.pointer_axis(dx, dy);
                }
                _ => {}
            }
        }
        if active {
            // the picture changes with the shape (and turns, for the busy ring)
            let step = if state.cursor_shape.animated() { ((state.start.elapsed().as_secs_f32() * 1.1).fract() * 14.0) as u32 } else { 0 };
            if state.cursor_gen != drawn_gen || step != drawn_step || state.config.input.cursor != drawn_prefs {
                drawn_gen = state.cursor_gen;
                drawn_step = step;
                drawn_prefs = state.config.input.cursor.clone();
                let img = cursor_picture(&state, scale, step as f32 / 14.0);
                for o in &mut screens.outs {
                    draw_cursor(o, &img);
                }
            }
            // the hardware cursor lives on the monitor under the pointer and is hidden on the others
            let (wx, wy) = (state.world[0], state.world[1]);
            for o in &mut screens.outs {
                let cx = ((cursor[0] - wx) * scale) as i32 - o.pos.0 - o.cursor_hot.0;
                let cy = ((cursor[1] - wy) * scale) as i32 - o.pos.1 - o.cursor_hot.1;
                let on = cx >= -63 && cy >= -63 && cx < o.size.0 as i32 && cy < o.size.1 as i32;
                if on {
                    if !o.cursor_on {
                        let _ = o.card.set_cursor(o.crtc, Some(&o.cursor));
                        o.cursor_on = true;
                    }
                    let _ = o.card.move_cursor(o.crtc, (cx, cy));
                } else if o.cursor_on {
                    let _ = o.card.set_cursor(o.crtc, None::<&DumbBuffer>);
                    o.cursor_on = false;
                }
            }
        }

        // page flips that completed (events of one device may belong to several of its monitors)
        let mut done_gpu = Vec::new();
        for i in 0..screens.outs.len() {
            let gi = screens.outs[i].gpu;
            if done_gpu.contains(&gi) {
                continue;
            }
            done_gpu.push(gi);
            if let Ok(events) = screens.outs[i].card.receive_events() {
                for e in events {
                    if let DrmEvent::PageFlip(ev) = e {
                        if let Some(o) = screens.outs.iter_mut().find(|o| o.gpu == gi && o.crtc == ev.crtc) {
                            o.flip_pending = false;
                            o.front = 1 - o.front;
                        }
                        if !screens.outs.iter().any(|o| o.flip_pending) {
                            state.send_frame_callbacks();
                        }
                    }
                }
            }
        }

        // wayland clients
        while let Ok(Some(stream)) = listener.accept() {
            let _ = dh.clone().insert_client(stream, Arc::new(ClientState { compositor: CompositorClientState::default() }));
        }
        let _ = calloop.dispatch(Some(std::time::Duration::ZERO), &mut state);
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();

        if let Some(vt) = state.pending_vt.take() {
            let _ = seat.borrow_mut().switch_session(vt);
        }
        if state.quit {
            break;
        }

        // monitors plugged in or out, or the Displays settings applied: set the desktop up again
        let any_pending = screens.outs.iter().any(|o| o.flip_pending);
        let probe_due = last_probe.elapsed().as_secs() >= 3;
        if active && !any_pending && (state.displays_dirty || probe_due) {
            let forced = std::mem::take(&mut state.displays_dirty);
            last_probe = Instant::now();
            let now_found = scan_monitors(&gpus, forced);
            let names = |f: &[Found]| -> Vec<String> {
                let mut v: Vec<String> = f.iter().map(|f| f.info.name.clone()).collect();
                v.sort();
                v
            };
            if forced || names(&now_found) != names(&found) {
                let prefs = state.config.display.clone();
                match build_screens(&gpus, &now_found, &prefs) {
                    Ok(new) => {
                        eprintln!("displays: {} monitor(s)", new.outs.len());
                        let old = std::mem::replace(&mut screens, new);
                        release_screens(old);
                        found = now_found;
                        scale = screens.scale;
                        state.monitor_infos = found.iter().map(|f| f.info.clone()).collect();
                        state.apply_layout(&screens.placed, scale);
                        let (cw, ch) = screens.canvas;
                        let (t, st, pd) = make_canvas(&device, format, cw, ch);
                        go_zero_copy(&mut screens.outs, &device, &queue);
                        target = t;
                        dumb_prev = None;
                        ahead = None;
                        staging = st;
                        padded = pd;
                        for o in &mut screens.outs {
                            draw_cursor(o, &cursor_picture(&state, scale, 0.0));
                            o.cursor_on = true;
                        }
                        cursor = state.clamp_to_monitors(cursor);
                        state.pointer_motion(cursor[0], cursor[1]);
                        state.invalidate();
                        idle = false;
                    }
                    Err(e) => eprintln!("displays: could not apply the change ({e}); keeping the current setup"),
                }
            }
        }

        // the displays follow the idle timer
        if state.screen_off != dpms_off && !any_pending {
            dpms_off = state.screen_off;
            for o in &screens.outs {
                set_dpms(o, !dpms_off);
            }
            if !dpms_off {
                for o in &screens.outs {
                    let _ = o.card.set_crtc(o.crtc, Some(o.fb(o.front)), (0, 0), &[o.conn], Some(o.mode));
                }
                state.invalidate();
            }
        }
        // draw the next frame (also while the last flip is in flight) and, once the back buffers are free, copy it in and flip
        if active {
            // `MEMRIUM_NO_AHEAD=1`: draw only when no flip is in flight, as before
            if ahead.is_none() && !(no_ahead && screens.outs.iter().any(|o| o.flip_pending)) {
                let now = Instant::now();
                let dt = (now - last).as_secs_f32().min(0.1);
                last = now;
                state.update(dt);
                let (cw, ch) = screens.canvas;
                let readback = screens.outs.iter().any(|o| o.zc.is_none());
                match render_ahead(&mut state, &target, &staging, cw, ch, padded, &mut dumb_prev, readback) {
                    Some(a) => ahead = Some(a),
                    None => {
                        idle = true;
                        state.send_frame_callbacks();
                        continue;
                    }
                }
            }
            if !screens.outs.iter().any(|o| o.flip_pending) {
                if let Some(a) = ahead.take() {
                    let (cw, ch) = screens.canvas;
                    scan_out(&staging, &target, &state.renderer, &a, cw, ch, padded, &mut screens.outs, &mut stages);
                    if stats_on {
                        let ms = a.started.elapsed().as_secs_f32() * 1000.0;
                        st_n += 1;
                        st_sum += ms;
                        st_max = st_max.max(ms);
                        if st_since.elapsed().as_secs() >= 5 {
                            let n = st_n.max(1) as f32;
                            eprintln!("stats: {} frames in {:.1}s ({:.0} fps), render+copy avg {:.1} ms, max {:.1} ms | build {:.1} + gpu {:.1} + copy {:.1} ms, redrawn {:.0} kpx", st_n, st_since.elapsed().as_secs_f32(), st_n as f32 / st_since.elapsed().as_secs_f32(), st_sum / n, st_max, stages[0] / n, stages[1] / n, stages[2] / n, stages[3] / n);
                            eprintln!("stats: {}", state.scale_report());
                            (st_n, st_sum, st_max, st_since) = (0, 0.0, 0.0, Instant::now());
                            stages = [0.0; 4];
                        }
                    }
                    idle = false;
                    for o in &mut screens.outs {
                        let fb = o.fb(1 - o.front);
                        match o.card.page_flip(o.crtc, fb, PageFlipFlags::EVENT, None) {
                            Ok(()) => {
                                o.flip_pending = true;
                                flip_failures = 0;
                            }
                            Err(e) => {
                                // usually we are not the DRM master (VT switch in progress): log once, retry the modeset gently
                                if flip_failures == 0 {
                                    eprintln!("page_flip: {e} (retrying)");
                                }
                                flip_failures += 1;
                                let _ = o.card.acquire_master_lock();
                                let _ = o.card.set_crtc(o.crtc, Some(o.fb(o.front)), (0, 0), &[o.conn], Some(o.mode));
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        }
                    }
                }
            }
        }
    }
    state.restore_session_env();
    eprintln!("session ended");
}

/// Switch the display on or off through the connector's DPMS property (what `xset dpms force off` does).
fn set_dpms(out: &Output, on: bool) {
    let Ok(props) = out.card.get_properties(out.conn) else { return };
    let (handles, _) = props.as_props_and_values();
    for &h in handles {
        if let Ok(info) = out.card.get_property(h) {
            if info.name().to_bytes() == b"DPMS" {
                let _ = out.card.set_property(out.conn, h, if on { 0 } else { 3 });
                return;
            }
        }
    }
}

/// A frame that is drawn and read back, not yet in a back buffer.
struct Ahead {
    /// what this frame redrew (None: everything)
    cur: Option<(u32, u32, u32, u32)>,
    /// what the back buffer to fill lacks: this frame's change and the one before it (None: everything)
    need: Option<(u32, u32, u32, u32)>,
    started: Instant,
    /// the changed part is being read back into the staging buffer (some monitor still uses dumb buffers)
    mapped: bool,
    built: f32,
    gpu: f32,
}

/// Let go of a frame that will not be shown (the staging buffer is mapped while it waits).
fn drop_ahead(ahead: &mut Option<Ahead>, staging: &wgpu::Buffer) {
    if ahead.take().is_some_and(|a| a.mapped) {
        staging.unmap();
    }
}

/// Render the whole desktop offscreen and read the changed part back into `staging`. None: nothing changed.
fn render_ahead(state: &mut State, target: &wgpu::Texture, staging: &wgpu::Buffer, w: u32, h: u32, padded: usize, prev: &mut Option<Option<(u32, u32, u32, u32)>>, readback: bool) -> Option<Ahead> {
    let t_start = Instant::now();
    let view = target.create_view(&Default::default());
    let mut enc = state.render(&view);
    if state.frame_skipped {
        return None;
    }
    // what this frame redrew (None: everything). The buffer we are about to fill held the frame before the last one, so
    // it needs what the last frame changed as well as this one.
    let cur = state.renderer.last_region();
    let need = match (cur, *prev) {
        (Some(a), Some(Some(b))) => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            Some((x0, y0, (a.0 + a.2).max(b.0 + b.2) - x0, (a.1 + a.3).max(b.1 + b.3) - y0))
        }
        _ => None,
    };
    *prev = Some(cur);
    let (cx, cy, cw, ch) = cur.unwrap_or((0, 0, w, h));
    if readback {
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d { x: cx, y: cy, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: staging, layout: wgpu::TexelCopyBufferLayout { offset: cy as u64 * padded as u64 + cx as u64 * 4, bytes_per_row: Some(padded as u32), rows_per_image: Some(ch) } },
            wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
        );
    }
    state.renderer.queue().submit([enc.finish()]);
    let t_built = Instant::now();
    // with scan-out images only, the GPU keeps drawing while the display waits: the wait comes with the copy into the image
    if readback {
        staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        state.renderer.device().poll(wgpu::PollType::Wait).unwrap();
    }
    let t_gpu = Instant::now();
    Some(Ahead { cur, need, started: t_start, mapped: readback, built: (t_built - t_start).as_secs_f32() * 1000.0, gpu: (t_gpu - t_built).as_secs_f32() * 1000.0 })
}

/// Give every monitor its part of the frame in its back buffer (which the last flip has freed).
fn scan_out(staging: &wgpu::Buffer, target: &wgpu::Texture, renderer: &Renderer, a: &Ahead, w: u32, h: u32, padded: usize, outs: &mut [Output], stages: &mut [f32; 4]) {
    let t_copy = Instant::now();
    let (nx, ny, nw, nh) = a.need.unwrap_or((0, 0, w, h));
    let mut enc = renderer.device().create_command_encoder(&Default::default());
    let mut gpu_copies = false;
    {
        let mapped = a.mapped.then(|| staging.slice(..).get_mapped_range());
        for o in outs.iter_mut() {
            let idx = 1 - o.front;
            let (ow, oh) = (o.size.0 as usize, o.size.1 as usize);
            let (px, py) = (o.pos.0.max(0) as usize, o.pos.1.max(0) as usize);
            // the part of the redone region that falls on this monitor, in its own columns and rows
            let (rx0, rx1) = ((nx as usize).max(px), ((nx + nw) as usize).min(px + ow));
            if rx1 <= rx0 {
                continue;
            }
            let rows = (ny as usize).max(py)..((ny + nh) as usize).min(py + oh).min(h as usize);
            if rows.end <= rows.start {
                continue;
            }
            if let Some(z) = &o.zc {
                // the GPU copies it from the canvas into the image that is about to be shown
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d { x: rx0 as u32, y: rows.start as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyTextureInfo { texture: &z.imgs[idx].texture, mip_level: 0, origin: wgpu::Origin3d { x: (rx0 - px) as u32, y: (rows.start - py) as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                    wgpu::Extent3d { width: (rx1 - rx0) as u32, height: (rows.end - rows.start) as u32, depth_or_array_layers: 1 },
                );
                gpu_copies = true;
                continue;
            }
            let Some(mapped) = mapped.as_ref() else { continue };
            let (card, bufs) = (&o.card, &mut o.bufs);
            let pitch = bufs[idx].0.pitch() as usize;
            if o.maps[idx].is_none() {
                // SAFETY: the mapping does not point into the buffer object, only borrows its handle for the call; the
                // buffers live as long as the output and are destroyed only after it is released
                o.maps[idx] = card.map_dumb_buffer(&mut bufs[idx].0).ok().map(|m| unsafe { std::mem::transmute::<drm::control::dumbbuffer::DumbMapping<'_>, drm::control::dumbbuffer::DumbMapping<'static>>(m) });
            }
            if let Some(map) = o.maps[idx].as_mut() {
                for y in rows {
                    let row = y - py;
                    let src = &mapped[y * padded + rx0 * 4..y * padded + rx1 * 4];
                    map[row * pitch + (rx0 - px) * 4..row * pitch + (rx1 - px) * 4].copy_from_slice(src);
                }
            }
        }
    }
    if a.mapped {
        staging.unmap();
    }
    if gpu_copies {
        renderer.queue().submit([enc.finish()]);
        // the picture must be complete before the display is told to show it
        let _ = renderer.device().poll(wgpu::PollType::Wait);
    }
    stages[0] += a.built;
    stages[1] += a.gpu;
    stages[2] += t_copy.elapsed().as_secs_f32() * 1000.0;
    stages[3] += a.cur.map_or((w * h) as f32, |c| (c.2 * c.3) as f32) / 1000.0;
}
