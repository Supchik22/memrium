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
    /// which buffer is on screen
    front: usize,
    /// the buffer the pending flip puts on screen
    queued: usize,
    /// the buffer written last (the next frame goes into the one after it)
    back: usize,
    /// `back` holds a finished frame that waits for the pending flip to end before it is flipped to
    ready: bool,
    cursor_on: bool,
    /// the cursor picture's hot spot, device px
    cursor_hot: (i32, i32),
    /// the side of the hardware cursor plane's buffer (64, 128 or 256: what the driver takes, see `cursor_plane_dim`)
    cursor_dim: u32,
    /// how much of that buffer (rows and columns from the top left) the last picture filled: only that is cleared again
    cursor_extent: u32,
}

/// Renderer-allocated images (ZC_IMAGES) and their framebuffers.
struct Zc {
    imgs: Vec<memrium_de::renderer::ScanoutImage>,
    fbs: Vec<framebuffer::Handle>,
}

/// Scan-out images per monitor: three, so a frame finished while a flip is still pending waits in the third one and is
/// flipped to at the very next vblank (with two the GPU copy could only start after the flip, and a frame that took a
/// little long missed a whole refresh: 144 Hz fell to 72).
const ZC_IMAGES: usize = 3;

impl Output {
    /// The framebuffer of buffer `i`.
    fn fb(&self, i: usize) -> framebuffer::Handle {
        // (not `map_or`: its fallback is evaluated first, and there are only two dumb buffers)
        match &self.zc {
            Some(z) => z.fbs[i],
            None => self.bufs[i].1,
        }
    }

    /// How many buffers it cycles through.
    fn count(&self) -> usize {
        self.zc.as_ref().map_or(self.bufs.len(), |z| z.fbs.len())
    }

    /// A new frame may be written now: into the buffer after `back`, which is then neither on screen nor about to be.
    fn writable(&self) -> bool {
        if self.count() >= 3 { !self.ready } else { !self.flip_pending }
    }

    /// The buffer the next frame goes into.
    fn next(&self) -> usize {
        (self.back + 1) % self.count()
    }

    /// Back to the buffer on screen, nothing in flight (after a mode set or a VT switch).
    fn settle(&mut self) {
        self.flip_pending = false;
        self.ready = false;
        self.back = self.front;
        self.queued = self.front;
    }
}

/// Ask for buffer `idx` of `o` to be shown at the next vblank.
fn flip_to(o: &mut Output, idx: usize, failures: &mut u32) {
    match o.card.page_flip(o.crtc, o.fb(idx), PageFlipFlags::EVENT, None) {
        Ok(()) => {
            o.flip_pending = true;
            o.queued = idx;
            o.ready = false;
            *failures = 0;
        }
        Err(e) => {
            // usually we are not the DRM master (VT switch in progress): log once, retry the modeset gently
            if *failures == 0 {
                eprintln!("page_flip: {e} (retrying)");
            }
            *failures += 1;
            let _ = o.card.acquire_master_lock();
            let _ = o.card.set_crtc(o.crtc, Some(o.fb(o.front)), (0, 0), &[o.conn], Some(o.mode));
            o.settle();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
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
        // `MEMRIUM_DOUBLE_BUFFER=1`: two images, as before
        let count = if std::env::var_os("MEMRIUM_DOUBLE_BUFFER").is_some() { 2 } else { ZC_IMAGES };
        for _ in 0..count {
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
            o.zc = Some(Zc { imgs, fbs });
            o.front = 0;
            o.settle();
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
            let cursor_dim = cursor_plane_dim(&gpu.card);
            let cursor = gpu.card.create_dumb_buffer((cursor_dim, cursor_dim), DrmFourcc::Argb8888, 32).map_err(|e| format!("cursor buffer: {e}"))?;
            used.push((f.gpu, crtc));
            Ok(Output { card: gpu.card.dup(), gpu: f.gpu, crtc, conn: f.conn.handle(), mode, bufs, maps: [None, None], zc: None, cursor, size: (w, h), name: p.name.clone(), pos: (p.x - x0, p.y - y0), flip_pending: false, front: 0, queued: 0, back: 0, ready: false, cursor_on: false, cursor_hot: (0, 0), cursor_dim, cursor_extent: cursor_dim })
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
    CURSOR_PLANE.store(outs.iter().map(|o| o.cursor_dim).min().unwrap_or(64), std::sync::atomic::Ordering::Relaxed);
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

/// The cursor picture for the current shape: its size follows the preference and the monitors' scale (times the growth of
/// "shake to find"); drawn, or taken from the installed cursor theme picked in Settings.
fn cursor_picture(state: &State, scale: f32, phase: f32) -> memrium_de::cursor::CursorImage {
    let prefs = &state.config.input.cursor;
    if state.cursor_shape == memrium_de::cursor::Shape::Client {
        if let Some(c) = &state.client_cursor_img {
            return c.image(scale);
        }
    }
    let img = memrium_de::cursor_theme::picture(state.cursor_shape, (prefs.size * scale).round().max(1.0) as u32, prefs, state.theme.accent, phase);
    match &state.busy_icon {
        Some(icon) if state.cursor_shape == memrium_de::cursor::Shape::Progress && prefs.launch_icon => with_app_icon(state, img, icon, scale),
        _ => img,
    }
}

/// The side of the hardware cursor buffer for a GPU: the driver's largest cursor (DRM_CAP_CURSOR_WIDTH/HEIGHT), as the
/// square power of two the planes take (i915 takes 64, 128 and 256), at most 256; 64 when the driver does not say.
fn cursor_plane_dim(card: &Card) -> u32 {
    let cap = |c| DrmDevice::get_driver_capability(card, c).ok().filter(|&v| v > 0).unwrap_or(64);
    let most = cap(drm::DriverCapability::CursorWidth).min(cap(drm::DriverCapability::CursorHeight));
    [256u32, 128, 64].into_iter().find(|&d| d as u64 <= most).unwrap_or(64)
}

/// The smallest hardware cursor buffer of all monitors (the one picture is put on each of them).
static CURSOR_PLANE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(64);

/// Seconds since the program that the busy cursor waits for was launched.
fn launch_time(state: &State) -> f32 {
    state.busy_until.map_or(0.0, |until| crate::pointer::BUSY_FOR.saturating_sub(until.saturating_duration_since(Instant::now())).as_secs_f32())
}

/// The busy cursor shows a launch icon that moves (it needs drawing again many times a second).
fn launch_moving(state: &State) -> bool {
    let prefs = &state.config.input.cursor;
    state.busy_icon.is_some() && state.cursor_shape == memrium_de::cursor::Shape::Progress && prefs.launch_icon && prefs.launch_anim != memrium_de::cursor::LaunchAnim::Still
}

/// Like `cursor_theme::animation` (loops per second, pictures per loop), with enough pictures for a moving launch icon
/// (about 60 a second; the loop draws one when it turns, at least every 16 ms while it moves, see `launch_moving`).
fn cursor_animation(state: &State) -> Option<(f32, u32)> {
    let base = memrium_de::cursor_theme::animation(state.cursor_shape, &state.config.input.cursor);
    if !launch_moving(state) {
        return base;
    }
    // (whole multiples of the loop's own pictures, so the ring or the theme's frames still change where they did)
    let (rate, steps) = base.unwrap_or((1.0, 1));
    let more = (60.0 / (rate * steps as f32).max(0.01)).ceil().clamp(1.0, 1000.0) as u32;
    Some((rate, steps * more))
}

/// The starting program's icon beside the busy cursor (like KDE's launch feedback), moving as the preferences say, on a
/// larger picture that still fits the cursor planes.
fn with_app_icon(state: &State, img: memrium_de::cursor::CursorImage, icon: &image::RgbaImage, scale: f32) -> memrium_de::cursor::CursorImage {
    let max = CURSOR_PLANE.load(std::sync::atomic::Ordering::Relaxed);
    memrium_de::cursor::with_launch_icon(img, icon, &state.config.input.cursor, scale, launch_time(state), max)
}

/// Put a cursor picture on the monitor's hardware cursor plane (a `cursor_dim` square buffer; a larger picture is cut).
fn draw_cursor(out: &mut Output, img: &memrium_de::cursor::CursorImage) {
    let dim = out.cursor_dim as usize;
    let n = img.size as usize;
    let pitch = out.cursor.pitch() as usize;
    if let Ok(mut map) = out.card.map_dumb_buffer(&mut out.cursor) {
        // only the corner the last picture or this one covers: the rest of the buffer is clear already
        let reach = n.min(dim).max(out.cursor_extent as usize).min(dim);
        for y in 0..reach {
            for x in 0..reach {
                let px = if x < n && y < n { img.argb[y * n + x] } else { 0 };
                let i = y * pitch + x * 4;
                map[i..i + 4].copy_from_slice(&px.to_le_bytes());
            }
        }
        out.cursor_extent = n.min(dim) as u32;
    }
    out.cursor_hot = img.hot;
    // a plane that is off stays off (the pointer is on another monitor, or typing hid it): the loop turns it on
    if out.cursor_on {
        let _ = out.card.set_cursor(out.crtc, Some(&out.cursor));
    }
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
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: if std::env::var("MEMRIUM_GPU").map_or_else(|_| memrium_de::gpu::shell_dedicated(), |v| v == "high") { wgpu::PowerPreference::HighPerformance } else { wgpu::PowerPreference::LowPower }, ..Default::default() })).expect("no GPU adapter");
    eprintln!("adapter: {}", adapter.get_info().name);
    let (device, queue) = memrium_de::renderer::create_device(&adapter, &wgpu::DeviceDescriptor { required_features: if std::env::var_os("MEMRIUM_STATS").is_some() { adapter.features() & wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() }, ..Default::default() }).expect("GPU device");
    let format = wgpu::TextureFormat::Bgra8Unorm; // byte order B,G,R,A = DRM XRGB8888
    let mut renderer = Renderer::new(device.clone(), queue.clone(), format, w, h, scale, crate::load_wallpaper(&args));
    if std::env::var_os("MEMRIUM_STATS").is_some() {
        renderer.enable_gpu_timer();
    }
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
    state.export_cursor_theme();
    state.monitor_infos = found.iter().map(|f| f.info.clone()).collect();
    state.apply_layout(&screens.placed, scale);
    // timers and the X server's sockets (XWayland) run on a calloop that is dispatched from this loop
    let mut calloop: calloop::EventLoop<'static, State> = calloop::EventLoop::try_new().expect("event loop");
    state.init_dmabuf();
    state.init_syncobj(adapter.get_info().vendor);
    // our frames run on the GPU after the loop has moved on: buffers go back to clients once the frames that read them are done
    state.defer_release = true;
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
    // "shake to find": how much bigger the cursor is drawn now, and the growth it was last drawn at
    let mut shake = memrium_de::cursor::Shake::default();
    let mut drawn_zoom = 1.0f32;
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
    let mut dumb_prev: Vec<Option<(u32, u32, u32, u32)>> = Vec::new();
    // a frame already drawn and read back into `staging`, waiting for the flip in flight to finish so it can be copied into
    // the back buffer: the GPU works on the next frame while the display waits for its vblank
    let mut ahead: Option<Ahead> = None;
    // a frame whose copy into the scan-out images the GPU is still doing (`waiter` says when it is done; the flip waits
    // for it, input does not): the request number (None: the copy was waited for on the spot), when the copy began, and the
    // client buffers that go back once it is done
    let mut copying: Option<(Ahead, Done, Instant, Vec<crate::state::Held>)> = None;
    // the kernel tells when the GPU is done (a sync file polled with the rest); without it, a thread that waits
    let done_fence = memrium_de::renderer::DoneFence::new(&device);
    let waiter = if done_fence.is_none() { GpuWaiter::new(&device) } else { None };
    if done_fence.is_some() {
        eprintln!("gpu: frames are waited for with sync files");
    }
    let no_ahead = std::env::var_os("MEMRIUM_NO_AHEAD").is_some();
    let mut idle = false;
    // frames in a row that came out the same as the one before: the longer the desktop stays still, the less often it is
    // looked at (see `idle_wait`)
    let mut idle_streak = 0u32;
    // the kernel's device events: a monitor plugged in or out is heard here instead of asking every GPU every few seconds
    // (asking wakes a sleeping dedicated GPU of a hybrid laptop); without it, the old asking every 3 s
    let hotplug = HotplugWatch::open();
    // a DRM event heard and not yet acted on (it waits while a flip is in flight)
    let mut hotplug_seen = false;
    let mut swipe_last = Instant::now();
    let mut last_probe = Instant::now();
    // `MEMRIUM_STATS=1`: how long the frames take, written to the log every five seconds
    let stats_on = std::env::var_os("MEMRIUM_STATS").is_some();
    let (mut st_n, mut st_sum, mut st_max, mut st_since) = (0u32, 0.0f32, 0.0f32, Instant::now());
    let mut stages = [0.0f32; 4];
    state.allow_skip = true;
    state.persistent_target = true;
    let mut laps = Laps::default();
    if stats_on {
        watch_stalls();
    }
    // when the next frame is begun (see `Pacer`); `MEMRIUM_NO_PACE=1`: at once, as before
    let pace_on = std::env::var_os("MEMRIUM_NO_PACE").is_none();
    let mut pacer = Pacer::default();
    pacer.reset(screens.outs.first().map_or(0, |o| mode_period_us(&o.mode)));
    // a frame start put off until this moment (µs of CLOCK_MONOTONIC)
    let mut due: Option<u64> = None;
    // when the GPU was last seen done with our frame, and when it should be done with the one in `copying` (µs)
    let (mut gpu_free_us, mut gpu_due_us) = (0u64, 0u64);
    // where each monitor's hardware cursor was put last: moving it to the same place again is a wasted ioctl (and one that
    // may wait for the display)
    let mut cursor_at: Vec<Option<(i32, i32)>> = Vec::new();
    // MEMRIUM_STATS: the oldest pointer motion not yet on the cursor plane (libinput's CLOCK_MONOTONIC stamp, µs), how long
    // motions took to reach the cursor plane, the slowest cursor ioctl, and the frame starts put off by the pacer
    let mut motion_since: Option<u64> = None;
    let (mut lat_n, mut lat_sum, mut lat_max, mut ioctl_max) = (0u32, 0u64, 0u64, 0u64);
    let (mut put_off_n, mut put_off_sum) = (0u32, 0u64);
    loop {
        // a pass of the loop that held everything up (input waited meanwhile) is named in the log, phase by phase
        if stats_on {
            laps.report();
        }
        // wait for something to do: events, or a vblank when a flip is in flight
        let flip_pending = screens.outs.iter().any(|o| o.flip_pending);
        {
            let seat_fd = seat.borrow_mut().get_fd().map(|f| f.as_raw_fd()).ok();
            let wl_fd = display.backend().poll_fd().as_raw_fd();
            let calloop_fd = std::os::fd::AsFd::as_fd(&calloop).as_raw_fd();
            let fence_fd = match &copying {
                Some((_, Done::Fence(fd), ..)) => Some(fd.as_raw_fd()),
                _ => None,
            };
            let mut raw: Vec<Option<RawFd>> = vec![Some(wl_fd), Some(libinput.as_raw_fd()), Some(listener.as_raw_fd()), seat_fd, Some(calloop_fd), hotplug.as_ref().map(|h| h.fd()), waiter.as_ref().map(|w| w.fd()), fence_fd];
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
            // idle (the last frame was identical): wake now and then for the clock, the widgets and what other threads
            // bring (less often the longer nothing changes); input and client commits wake the poll at once
            let timeout: i32 = if flip_pending || copying.is_some() || !active { 100 } else if idle { idle_wait(idle_streak, state.screen_off).min(if state.recording.is_some() { 8 } else if launch_moving(&state) { 16 } else { i32::MAX }) } else { 0 };
            // a frame start put off by the pacer: wake for it (to the microsecond; poll's milliseconds would lose most of
            // what putting it off gains)
            let mut wait_us = timeout as i64 * 1000;
            if let Some(d) = due.filter(|_| active) {
                let left = d.saturating_sub(mono_us()) as i64;
                wait_us = if timeout == 0 { left } else { wait_us.min(left) };
            }
            let ts = libc::timespec { tv_sec: (wait_us / 1_000_000) as libc::time_t, tv_nsec: ((wait_us % 1_000_000) * 1000) as libc::c_long };
            // SAFETY: `fds` is a valid array of `fds.len()` pollfd structs, `ts` outlives the call, no signal mask
            unsafe {
                libc::ppoll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, &ts, std::ptr::null());
            }
        }
        laps.start();

        // nothing of ours in flight on the GPU (every frame so far was waited for): replaced buffers go back at once
        if copying.is_none() && ahead.is_none() && !state.retired.is_empty() {
            state.take_retired().into_iter().for_each(crate::state::Held::done);
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
                    o.cursor_on = false;
                    draw_cursor(o, &cursor_picture(&state, scale, 0.0));
                    o.settle();
                }
                dumb_prev.clear();
                cursor_at.clear();
                due = None;
                drop_ahead(&mut ahead, &staging);
                if let Some((.., held)) = copying.take() {
                    let _ = device.poll(wgpu::PollType::Wait);
                    held.into_iter().for_each(crate::state::Held::done);
                }
                state.invalidate();
                idle = false;
            } else {
                active = false;
                due = None;
                libinput.suspend();
                for o in &screens.outs {
                    let _ = o.card.release_master_lock();
                }
            }
        }

        laps.mark("seat");
        // input
        let _ = libinput.dispatch();
        let mut had_input = false;
        if state.input_dirty {
            state.input_dirty = false;
            state.export_cursor_theme();
            for d in &mut devices {
                apply_input(d, &state.config.input, &kde_text);
            }
        }
        if let Some(w) = state.warp_to.take() {
            cursor = state.clamp_to_monitors(w);
            if state.pointer_locked() {
                // only remembered while the lock holds: a motion sent now would make Xwayland see the pointer jump and
                // feed the game a huge delta, which it warps back again (the view spins)
                state.mouse = cursor;
            } else {
                state.pointer_motion(cursor[0], cursor[1]);
            }
        }
        for ev in &mut libinput {
            had_input = true;
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
                    if stats_on {
                        use input::event::pointer::PointerEventTrait as _;
                        motion_since.get_or_insert(m.time_usec());
                    }
                    // programs that asked for raw motion get it; a locked pointer (a game) stays where it is
                    state.pointer_relative(m.dx_unaccelerated() as f64 / scale as f64, m.dy_unaccelerated() as f64 / scale as f64);
                    if !state.pointer_locked() {
                        // the pointer moves over the whole desktop, from one monitor to the next
                        cursor = state.clamp_to_monitors([cursor[0] + m.dx() as f32 / scale, cursor[1] + m.dy() as f32 / scale]);
                        state.pointer_motion(cursor[0], cursor[1]);
                        if state.config.input.cursor.shake_to_find {
                            shake.motion(m.dx() as f32 / scale, Instant::now());
                        }
                    }
                }
                InputEvent::Pointer(PointerEvent::MotionAbsolute(m)) => {
                    let [cw, ch] = state.renderer.logical_size();
                    cursor = state.clamp_to_monitors([state.world[0] + m.absolute_x_transformed(cw as u32) as f32, state.world[1] + m.absolute_y_transformed(ch as u32) as f32]);
                    state.pointer_motion(cursor[0], cursor[1]);
                }
                InputEvent::Pointer(PointerEvent::Button(b)) => {
                    use input::event::pointer::PointerEventTrait;
                    // the aim moves first, then the shot
                    state.flush_relative();
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
        state.flush_relative();
        laps.mark("input");
        if active {
            // the picture changes with the shape (and turns, for the busy ring or a theme's animated cursor)
            let (step, phase) = match cursor_animation(&state) {
                Some((rate, steps)) => {
                    let step = ((state.start.elapsed().as_secs_f32() * rate).fract() * steps as f32) as u32;
                    (step, step as f32 / steps as f32)
                }
                None => (0, 0.0),
            };
            // (in steps of 1/16, so a growing cursor is drawn again only a few times)
            let now = Instant::now();
            let zoom = (shake.zoom(now) * 16.0).round() / 16.0;
            if shake.moving(now) || launch_moving(&state) {
                // keep the loop turning at the quick rate until it has shrunk back, even with the mouse still
                idle_streak = 0;
            }
            if state.cursor_gen != drawn_gen || step != drawn_step || zoom != drawn_zoom || state.config.input.cursor != drawn_prefs {
                drawn_gen = state.cursor_gen;
                drawn_step = step;
                drawn_zoom = zoom;
                drawn_prefs = state.config.input.cursor.clone();
                let img = cursor_picture(&state, scale * zoom, phase);
                for o in &mut screens.outs {
                    draw_cursor(o, &img);
                }
            }
            // the hardware cursor lives on the monitor under the pointer and is hidden on the others
            let (wx, wy) = (state.world[0], state.world[1]);
            cursor_at.resize(screens.outs.len(), None);
            // typing hid it: off everywhere until the mouse moves
            let hidden = state.typing_hid_cursor;
            for (i, o) in screens.outs.iter_mut().enumerate() {
                let cx = ((cursor[0] - wx) * scale) as i32 - o.pos.0 - o.cursor_hot.0;
                let cy = ((cursor[1] - wy) * scale) as i32 - o.pos.1 - o.cursor_hot.1;
                let on = !hidden && cx > -(o.cursor_dim as i32) && cy > -(o.cursor_dim as i32) && cx < o.size.0 as i32 && cy < o.size.1 as i32;
                if on {
                    if !o.cursor_on {
                        let _ = o.card.set_cursor(o.crtc, Some(&o.cursor));
                        o.cursor_on = true;
                    }
                    if cursor_at[i] != Some((cx, cy)) {
                        let t = Instant::now();
                        let _ = o.card.move_cursor(o.crtc, (cx, cy));
                        cursor_at[i] = Some((cx, cy));
                        if stats_on {
                            ioctl_max = ioctl_max.max(t.elapsed().as_micros() as u64);
                        }
                    }
                } else if o.cursor_on {
                    let _ = o.card.set_cursor(o.crtc, None::<&DumbBuffer>);
                    o.cursor_on = false;
                    cursor_at[i] = None;
                }
            }
            if let Some(t0) = motion_since.take() {
                let l = mono_us().saturating_sub(t0);
                lat_n += 1;
                lat_sum += l;
                lat_max = lat_max.max(l);
            }
        }

        laps.mark("cursor");
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
                        // the pacer keeps to the first monitor's refresh
                        if screens.outs.first().is_some_and(|o| o.gpu == gi && o.crtc == ev.crtc) {
                            pacer.vblank_seen(ev.duration.as_micros() as u64, ev.frame);
                        }
                        if let Some(o) = screens.outs.iter_mut().find(|o| o.gpu == gi && o.crtc == ev.crtc) {
                            o.flip_pending = false;
                            o.front = o.queued;
                            // a frame finished meanwhile: on screen at the next vblank
                            if o.ready {
                                let b = o.back;
                                flip_to(o, b, &mut flip_failures);
                            }
                        }
                        // a frame reached the screen: the programs may draw their next one (also when the third buffer was
                        // flipped to right away and a flip is pending again)
                        state.send_frame_callbacks();
                    }
                }
            }
        }

        laps.mark("flip events");
        // wayland clients
        while let Ok(Some(stream)) = listener.accept() {
            let _ = dh.clone().insert_client(stream, Arc::new(ClientState { compositor: CompositorClientState::default() }));
        }
        let _ = calloop.dispatch(Some(std::time::Duration::ZERO), &mut state);
        laps.mark("timers and services");
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        laps.mark("clients");

        if let Some(vt) = state.pending_vt.take() {
            if let Err(e) = seat.borrow_mut().switch_session(vt) {
                eprintln!("vt: could not switch to tty{vt}: {e}");
            }
        }
        if state.quit {
            break;
        }

        // something to react to soon (a hover that waits for its tooltip): look at the desktop at the quick rate again
        if had_input {
            idle_streak = 0;
        }

        // monitors plugged in or out, or the Displays settings applied: set the desktop up again
        let any_pending = screens.outs.iter().any(|o| o.flip_pending);
        let probe_due = match &hotplug {
            Some(h) => {
                hotplug_seen |= h.drm_changed();
                hotplug_seen
            }
            None => last_probe.elapsed().as_secs() >= 3,
        };
        if active && !any_pending && copying.is_none() && (state.displays_dirty || probe_due) {
            hotplug_seen = false;
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
                        dumb_prev.clear();
                        cursor_at.clear();
                        due = None;
                        pacer.reset(screens.outs.first().map_or(0, |o| mode_period_us(&o.mode)));
                        ahead = None;
                        staging = st;
                        padded = pd;
                        for o in &mut screens.outs {
                            o.cursor_on = false;
                            draw_cursor(o, &cursor_picture(&state, scale, 0.0));
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
            // (not while a finished frame still waits for its flip: one more in the queue would only show the input later)
            let can_draw = ahead.is_none() && !screens.outs.iter().any(|o| o.ready) && !(no_ahead && screens.outs.iter().any(|o| o.flip_pending));
            // begun as late as it can be while still in time for the first vblank it can reach, the frame shows the newest
            // input (one monitor only: with more, their vblanks do not line up)
            let start = match screens.outs.as_slice() {
                [o] if can_draw && pace_on => {
                    let now_us = mono_us();
                    let next_v = pacer.vblank_from(now_us + 1);
                    let taken = match (&copying, o.flip_pending) {
                        (Some(_), pending) => {
                            let v = pacer.vblank_from(gpu_due_us + PACE_MARGIN);
                            Some(if pending { v.max(next_v + pacer.period) } else { v })
                        }
                        (None, true) => Some(next_v),
                        (None, false) => None,
                    };
                    let gpu_free = if copying.is_some() { gpu_due_us.max(now_us) } else { now_us };
                    pacer.start(now_us, taken, gpu_free)
                }
                _ => 0,
            };
            if !can_draw {
                // a frame already waits: nothing to wake for (a stale moment would spin the loop)
                due = None;
            } else if start > mono_us() + 200 {
                if due.is_none() && stats_on {
                    put_off_n += 1;
                    put_off_sum += start.saturating_sub(mono_us());
                }
                due = Some(start);
            } else {
                due = None;
                let began_us = mono_us();
                let now = Instant::now();
                let dt = (now - last).as_secs_f32().min(0.1);
                last = now;
                laps.mark("displays");
                state.update(dt);
                laps.mark("update");
                let (cw, ch) = screens.canvas;
                let readback = screens.outs.iter().any(|o| o.zc.is_none());
                let age = screens.outs.iter().map(Output::count).max().unwrap_or(2);
                let drawn = render_ahead(&mut state, &target, &staging, cw, ch, padded, &mut dumb_prev, age, readback);
                laps.mark("render");
                match drawn {
                    Some(a) => {
                        pacer.learn_build(mono_us().saturating_sub(began_us));
                        ahead = Some(a);
                    }
                    None => {
                        idle = true;
                        idle_streak = idle_streak.saturating_add(1);
                        state.send_frame_callbacks();
                        continue;
                    }
                }
            }
            if screens.outs.iter().all(Output::writable) && copying.is_none() {
                if let Some(a) = ahead.take() {
                    let (cw, ch) = screens.canvas;
                    let began = Instant::now();
                    let (sub, armed) = scan_out(&staging, &target, &state.renderer, &a, cw, ch, padded, &mut screens.outs, done_fence.as_ref());
                    // the picture must be complete before the display is told to show it: waited for off the loop, so the
                    // input that comes meanwhile reaches the programs at once (a game no longer gets the mouse in lumps)
                    let fenced = match (armed, &done_fence) {
                        (true, Some(f)) => f.export().ok(),
                        _ => None,
                    };
                    let done = match (sub, fenced, &waiter) {
                        (None, ..) => Done::Now,
                        (Some(_), Some(Some(fd)), _) => Done::Fence(fd),
                        (Some(_), Some(None), _) => Done::Now,
                        (Some(i), None, Some(w)) => Done::Ask(w.start(i)),
                        (Some(i), None, None) => {
                            let _ = state.renderer.device().poll(wgpu::PollType::wait_for(i));
                            Done::Now
                        }
                    };
                    gpu_due_us = a.submit_us.max(gpu_free_us) + pacer.gpu as u64;
                    copying = Some((a, done, began, state.take_retired()));
                    laps.mark("scan out");
                }
            }
            let copied = match &copying {
                Some((_, Done::Ask(n), ..)) => waiter.as_ref().is_none_or(|w| w.finished(*n)),
                Some((_, Done::Fence(fd), ..)) => fence_signalled(fd),
                Some((_, Done::Now, ..)) => true,
                None => false,
            };
            if copied {
                let gpu_work = matches!(&copying, Some((_, Done::Ask(_) | Done::Fence(_), ..)));
                let (a, _, began, held) = copying.take().unwrap();
                let now_us = mono_us();
                if gpu_work {
                    pacer.learn_gpu(now_us.saturating_sub(a.submit_us.max(gpu_free_us)));
                }
                gpu_free_us = now_us;
                held.into_iter().for_each(crate::state::Held::done);
                if stats_on {
                    let ms = a.started.elapsed().as_secs_f32() * 1000.0;
                    stages[0] += a.built;
                    stages[1] += a.gpu;
                    stages[2] += began.elapsed().as_secs_f32() * 1000.0;
                    stages[3] += a.cur.map_or((screens.canvas.0 * screens.canvas.1) as f32, |c| (c.2 * c.3) as f32) / 1000.0;
                    st_n += 1;
                    st_sum += ms;
                    st_max = st_max.max(ms);
                    if st_since.elapsed().as_secs() >= 5 {
                        let n = st_n.max(1) as f32;
                        eprintln!("stats: {} frames in {:.1}s ({:.0} fps), render+copy avg {:.1} ms, max {:.1} ms | build {:.1} + gpu {:.1} + copy {:.1} ms, redrawn {:.0} kpx", st_n, st_since.elapsed().as_secs_f32(), st_n as f32 / st_since.elapsed().as_secs_f32(), st_sum / n, st_max, stages[0] / n, stages[1] / n, stages[2] / n, stages[3] / n);
                        if let Some((win, shell, all)) = state.renderer.gpu_times() {
                            eprintln!("stats: gpu passes of the last frame: windows {win:.1} ms, shell {shell:.1} ms, whole frame {all:.1} ms");
                        }
                        eprintln!("stats: {}", state.scale_report());
                        let ms = |us: u64| us as f32 / 1000.0;
                        eprintln!("stats: pointer: motion to cursor plane avg {:.2} ms, max {:.2} ms over {} moves, slowest cursor ioctl {:.2} ms | pacer: build {:.1} + gpu {:.1} ms expected, refresh {:.2} ms, {} frame starts put off by {:.1} ms on average", ms(lat_sum / lat_n.max(1) as u64), ms(lat_max), lat_n, ms(ioctl_max), pacer.build / 1000.0, pacer.gpu / 1000.0, ms(pacer.period), put_off_n, ms(put_off_sum / put_off_n.max(1) as u64));
                        (lat_n, lat_sum, lat_max, ioctl_max, put_off_n, put_off_sum) = (0, 0, 0, 0, 0, 0);
                        (st_n, st_sum, st_max, st_since) = (0, 0.0, 0.0, Instant::now());
                        stages = [0.0; 4];
                    }
                }
                idle = false;
                idle_streak = 0;
                for o in &mut screens.outs {
                    if o.flip_pending {
                        // the third buffer: flipped to as soon as the pending flip ends
                        o.ready = true;
                    } else {
                        let b = o.back;
                        flip_to(o, b, &mut flip_failures);
                    }
                }
            }
        }
    }
    state.restore_session_env();
    eprintln!("session ended");
}

/// How long to sleep (ms) when the desktop has not changed for `streak` frames: the quick rate first (an animation that is
/// about to start, a tooltip that waits for the pointer to rest), then slower, never past the next whole second (a clock
/// with seconds still ticks on time). With the screen off there is nothing to show at all.
fn idle_wait(streak: u32, screen_off: bool) -> i32 {
    if screen_off {
        return 1000;
    }
    let wait = match streak {
        0..=2 => 33,
        3 => 50,
        4 => 100,
        _ => 200,
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let to_second = 1000 - now.subsec_millis() as i32 + 2;
    wait.min(to_second)
}

/// The kernel's device events (a netlink socket of uevents): a DRM device changed when a monitor is plugged in or out.
struct HotplugWatch(std::os::fd::OwnedFd);

impl HotplugWatch {
    fn open() -> Option<HotplugWatch> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain socket calls; the fd is owned right after it is made
        unsafe {
            let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT);
            if fd < 0 {
                return None;
            }
            let owned = std::os::fd::OwnedFd::from_raw_fd(fd);
            let mut addr: libc::sockaddr_nl = std::mem::zeroed();
            addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            // group 1: the kernel's own events (any user may listen)
            addr.nl_groups = 1;
            if libc::bind(fd, &addr as *const libc::sockaddr_nl as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t) != 0 {
                return None;
            }
            Some(HotplugWatch(owned))
        }
    }

    fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Read every event that came; true when one of them was about a DRM device.
    fn drm_changed(&self) -> bool {
        let mut changed = false;
        let mut buf = [0u8; 8192];
        loop {
            // SAFETY: reading into a buffer of its own length from a socket we own
            let n = unsafe { libc::recv(self.fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len(), libc::MSG_DONTWAIT) };
            if n <= 0 {
                return changed;
            }
            // "change@/devices/.../drm/card0\0ACTION=change\0...\0SUBSYSTEM=drm\0HOTPLUG=1\0..."
            changed |= buf[..n as usize].split(|&b| b == 0).any(|f| f == b"SUBSYSTEM=drm");
        }
    }
}

/// How long each phase of one pass of the main loop took (after the wait for events); `report` logs a pass that took
/// long, so a stall can be pinned on what caused it.
#[derive(Default)]
struct Laps {
    at: Option<Instant>,
    began: Option<Instant>,
    marks: Vec<(&'static str, f32)>,
}

/// What the watchdog (`watch_stalls`) reads: when the pass in progress began (ms on `WATCH_EPOCH`, 0: none) and the
/// last phase it finished.
static PASS_BEGAN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PASS_PHASE: std::sync::Mutex<&'static str> = std::sync::Mutex::new("start");
static WATCH_EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn watch_ms() -> u64 {
    WATCH_EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// A thread that looks at the main thread while a pass of the loop hangs: what system call it sits in, on what file,
/// in which kernel function. Logged at 0.4 s, 1 s and 1.8 s into a stall (`stall in progress: ...`).
fn watch_stalls() {
    // SAFETY: plain syscall
    let tid = unsafe { libc::syscall(libc::SYS_gettid) };
    let _ = WATCH_EPOCH.get_or_init(Instant::now);
    let _ = std::thread::Builder::new().name("stall watch".into()).spawn(move || {
        let mut seen = (0u64, 0usize);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let began = PASS_BEGAN.load(std::sync::atomic::Ordering::Relaxed);
            if began == 0 {
                continue;
            }
            let ms = watch_ms().saturating_sub(began);
            if seen.0 != began {
                seen = (began, 0);
            }
            let marks = [400u64, 1000, 1800];
            if seen.1 >= marks.len() || ms < marks[seen.1] {
                continue;
            }
            seen.1 += 1;
            let task = format!("/proc/self/task/{tid}");
            let wchan = std::fs::read_to_string(format!("{task}/wchan")).unwrap_or_default();
            let sys = std::fs::read_to_string(format!("{task}/syscall")).unwrap_or_default();
            let mut words = sys.split_whitespace();
            let nr = words.next().unwrap_or("?").to_string();
            let fd = words.next().and_then(|a| i64::from_str_radix(a.trim_start_matches("0x"), 16).ok());
            let file = fd.filter(|f| (0..65536).contains(f)).and_then(|f| std::fs::read_link(format!("/proc/self/fd/{f}")).ok()).map(|p| p.display().to_string()).unwrap_or_default();
            let phase = PASS_PHASE.lock().map(|p| *p).unwrap_or("?");
            eprintln!("stall in progress: {ms} ms, after phase '{phase}', syscall {nr} (fd {} {file}), kernel wait '{}'", fd.unwrap_or(-1), wchan.trim());
        }
    });
}

impl Laps {
    fn start(&mut self) {
        let now = Instant::now();
        self.at = Some(now);
        self.began = Some(now);
        self.marks.clear();
        PASS_BEGAN.store(watch_ms(), std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut p) = PASS_PHASE.lock() {
            *p = "wait";
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let now = Instant::now();
        if let Some(at) = self.at {
            self.marks.push((phase, (now - at).as_secs_f32() * 1000.0));
        }
        self.at = Some(now);
        if let Ok(mut p) = PASS_PHASE.lock() {
            *p = phase;
        }
    }

    fn report(&mut self) {
        PASS_BEGAN.store(0, std::sync::atomic::Ordering::Relaxed);
        let Some(began) = self.began.take() else { return };
        let total = began.elapsed().as_secs_f32() * 1000.0;
        if total > 40.0 {
            let parts: Vec<String> = self.marks.iter().filter(|(_, ms)| *ms >= 1.0).map(|(p, ms)| format!("{p} {ms:.0}")).collect();
            eprintln!("stall: one pass of the loop took {total:.0} ms ({})", parts.join(", "));
        }
    }
}

/// How the loop learns that the copy into the scan-out images has finished.
enum Done {
    /// it was waited for on the spot, or there was no GPU work
    Now,
    /// a request to `GpuWaiter`
    Ask(u64),
    /// a sync file that turns readable
    Fence(OwnedFd),
}

/// The sync file is signalled (the GPU work behind it is done).
fn fence_signalled(fd: &OwnedFd) -> bool {
    let mut p = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, no waiting
    unsafe { libc::poll(&mut p, 1, 0) > 0 && p.revents & libc::POLLIN != 0 }
}

/// Waits for the GPU off the main loop: the loop hands over the work that must finish before a flip and goes on reading
/// input and serving clients; the eventfd wakes it when the work is done.
struct GpuWaiter {
    ask: std::sync::mpsc::Sender<(u64, wgpu::SubmissionIndex)>,
    /// the number of the last request that has finished
    done: Arc<std::sync::atomic::AtomicU64>,
    asked: Cell<u64>,
    fd: OwnedFd,
}

impl GpuWaiter {
    fn new(device: &wgpu::Device) -> Option<GpuWaiter> {
        use std::os::fd::FromRawFd;
        use std::sync::atomic::{AtomicU64, Ordering};
        // SAFETY: a plain eventfd, owned right after it is made
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let wake = fd.try_clone().ok()?;
        let done = Arc::new(AtomicU64::new(0));
        let (ask, asks) = std::sync::mpsc::channel::<(u64, wgpu::SubmissionIndex)>();
        let (device, finished) = (device.clone(), done.clone());
        std::thread::Builder::new()
            .name("gpu wait".into())
            .spawn(move || {
                while let Ok((n, sub)) = asks.recv() {
                    let _ = device.poll(wgpu::PollType::wait_for(sub));
                    finished.store(n, Ordering::Release);
                    let one = 1u64;
                    // SAFETY: writing 8 bytes from a u64 to an eventfd we hold
                    unsafe { libc::write(wake.as_raw_fd(), &one as *const u64 as *const libc::c_void, 8) };
                }
            })
            .ok()?;
        Some(GpuWaiter { ask, done, asked: Cell::new(0), fd })
    }

    fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Start waiting for this submission; the request's number for `finished`.
    fn start(&self, sub: wgpu::SubmissionIndex) -> u64 {
        let n = self.asked.get() + 1;
        self.asked.set(n);
        let _ = self.ask.send((n, sub));
        n
    }

    fn finished(&self, n: u64) -> bool {
        let mut buf = 0u64;
        // SAFETY: reading 8 bytes into a u64 from a non-blocking eventfd we own (clears its count)
        unsafe { libc::read(self.fd(), &mut buf as *mut u64 as *mut libc::c_void, 8) };
        self.done.load(std::sync::atomic::Ordering::Acquire) >= n
    }
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
    /// when its drawing was handed to the GPU (µs of CLOCK_MONOTONIC)
    submit_us: u64,
}

/// Let go of a frame that will not be shown (the staging buffer is mapped while it waits).
fn drop_ahead(ahead: &mut Option<Ahead>, staging: &wgpu::Buffer) {
    if ahead.take().is_some_and(|a| a.mapped) {
        staging.unmap();
    }
}

/// Render the whole desktop offscreen and read the changed part back into `staging`. None: nothing changed.
/// `prev` holds what the frames before it redrew (newest last); a buffer about to be filled holds the frame `age` frames
/// back, so it lacks what the `age - 1` frames since then changed as well as this one.
fn render_ahead(state: &mut State, target: &wgpu::Texture, staging: &wgpu::Buffer, w: u32, h: u32, padded: usize, prev: &mut Vec<Option<(u32, u32, u32, u32)>>, age: usize, readback: bool) -> Option<Ahead> {
    let t_start = Instant::now();
    let view = target.create_view(&Default::default());
    let mut enc = state.render(&view);
    if state.frame_skipped {
        return None;
    }
    // what this frame redrew (None: everything), and what the buffer to fill lacks besides
    let cur = state.renderer.last_region();
    let older = age.saturating_sub(1);
    let need = if prev.len() < older {
        None
    } else {
        prev[prev.len() - older..].iter().fold(cur, |acc, r| match (acc, *r) {
            (Some(a), Some(b)) => {
                let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
                Some((x0, y0, (a.0 + a.2).max(b.0 + b.2) - x0, (a.1 + a.3).max(b.1 + b.3) - y0))
            }
            _ => None,
        })
    };
    prev.push(cur);
    if prev.len() > 4 {
        prev.remove(0);
    }
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
    let submit_us = mono_us();
    // with scan-out images only, the GPU keeps drawing while the display waits: the wait comes with the copy into the image
    if readback {
        staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        state.renderer.device().poll(wgpu::PollType::Wait).unwrap();
    }
    let t_gpu = Instant::now();
    Some(Ahead { cur, need, started: t_start, mapped: readback, built: (t_built - t_start).as_secs_f32() * 1000.0, gpu: (t_gpu - t_built).as_secs_f32() * 1000.0, submit_us })
}

/// Give every monitor its part of the frame in its back buffer (which the last flip has freed). Returns the GPU's part
/// of the work when there is one (copies into scan-out images): the frame may be shown once it has finished.
/// With `fence`, it is armed for that submit (the second value says so).
fn scan_out(staging: &wgpu::Buffer, target: &wgpu::Texture, renderer: &Renderer, a: &Ahead, w: u32, h: u32, padded: usize, outs: &mut [Output], fence: Option<&memrium_de::renderer::DoneFence>) -> (Option<wgpu::SubmissionIndex>, bool) {
    let (nx, ny, nw, nh) = a.need.unwrap_or((0, 0, w, h));
    let mut enc = renderer.device().create_command_encoder(&Default::default());
    let mut gpu_copies = false;
    {
        let mapped = a.mapped.then(|| staging.slice(..).get_mapped_range());
        for o in outs.iter_mut() {
            // every monitor moves on to its next buffer, also one this frame does not touch: what that buffer lacks
            // is all in `need`, and none of it falls on this monitor
            let idx = o.next();
            o.back = idx;
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
    if !gpu_copies {
        return (None, false);
    }
    if let Some(f) = fence {
        f.arm(renderer.queue());
    }
    (Some(renderer.queue().submit([enc.finish()])), fence.is_some())
}

/// Microseconds of CLOCK_MONOTONIC: the clock DRM stamps vblanks with and libinput stamps events with.
fn mono_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write into
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

/// One refresh of a display mode, µs (0: unknown).
fn mode_period_us(m: &control::Mode) -> u64 {
    let (ht, vt) = (m.hsync().2 as u64, m.vsync().2 as u64);
    if m.clock() == 0 || ht == 0 || vt == 0 {
        return 0;
    }
    ht * vt * 1000 / m.clock() as u64
}

/// Slack between the expected end of a frame and the vblank it is meant for (the fence seen by the loop, the flip ioctl,
/// the driver's vblank evasion), µs.
const PACE_MARGIN: u64 = 1500;

/// When to begin the next frame. Begun at once, a frame takes the input of that moment and then waits: behind the GPU's
/// previous frame and for a vblank after the one already taken, so the pointer's position it shows is up to two
/// refreshes old (the triple buffer made this worse for light frames: drawn right after a flip, shown two vblanks later).
/// Begun as late as it can be while still in time for the first vblank it can reach, it shows the newest input; this is
/// what KWin does. Times are µs of CLOCK_MONOTONIC.
#[derive(Default)]
struct Pacer {
    /// one refresh (0: unknown, frames are begun at once)
    period: u64,
    /// the last vblank and its frame number
    vblank: u64,
    frame: u32,
    /// what a frame is expected to cost: the loop's CPU part (update + render) and the GPU's (render + copy), µs
    build: f64,
    gpu: f64,
}

impl Pacer {
    fn reset(&mut self, period: u64) {
        *self = Pacer { period, ..Pacer::default() };
    }

    /// A page flip ended: the vblank's time stamp keeps the pacer's grid on the display's own clock.
    fn vblank_seen(&mut self, t: u64, frame: u32) {
        if self.vblank != 0 && frame > self.frame && t > self.vblank {
            // the refresh measured over the frames between, to follow a mode the table rounds (143.98 Hz for "144")
            let measured = (t - self.vblank) / (frame - self.frame) as u64;
            if self.period == 0 {
                self.period = measured;
            } else if measured.abs_diff(self.period) * 10 < self.period {
                self.period = (self.period * 7 + measured + 4) / 8;
            }
        }
        self.vblank = t;
        self.frame = frame;
    }

    /// The first vblank at or after `t`.
    fn vblank_from(&self, t: u64) -> u64 {
        if self.period == 0 || t <= self.vblank {
            return self.vblank.max(t);
        }
        self.vblank + (t - self.vblank).div_ceil(self.period) * self.period
    }

    /// A cost seen: a dearer frame is believed at once, a cheaper one little by little (a miss costs a whole refresh, a
    /// frame begun a little early costs only latency).
    fn learn(est: &mut f64, us: u64) {
        let x = us.min(50_000) as f64;
        *est = if x > *est { x } else { *est + (x - *est) * 0.08 };
    }

    fn learn_build(&mut self, us: u64) {
        Self::learn(&mut self.build, us);
    }

    fn learn_gpu(&mut self, us: u64) {
        Self::learn(&mut self.gpu, us);
    }

    /// When to begin the next frame, given that the vblank `taken` already belongs to another frame and the GPU is busy
    /// with ours until `gpu_free`.
    fn start(&self, now: u64, taken: Option<u64>, gpu_free: u64) -> u64 {
        // no grid yet, or a stale one (the desktop sat still): at once
        if self.period == 0 || self.vblank == 0 || now.saturating_sub(self.vblank) > 250_000 {
            return now;
        }
        let (b, g) = (self.build as u64, self.gpu as u64);
        // a frame dearer than one refresh gains nothing by waiting: it misses the next vblank anyway, and putting it off
        // only adds latency on top (the GPU estimate is the time to the sync file, which on a busy or clocked-down iGPU
        // was 15-50 ms against 0.3-7 ms of our own passes: frames were put off 17-24 ms and lagged behind the cursor)
        if b + g + PACE_MARGIN >= self.period {
            return now;
        }
        let done = (now + b).max(gpu_free) + g + PACE_MARGIN;
        let mut v = self.vblank_from(done);
        if let Some(t) = taken {
            v = v.max(self.vblank_from(t + self.period / 2));
        }
        v.saturating_sub(b + g + PACE_MARGIN).max(now)
    }
}

#[cfg(test)]
mod tests {
    use super::idle_wait;

    #[test]
    fn a_light_frame_is_begun_late_enough_to_show_the_newest_input() {
        let mut p = super::Pacer::default();
        p.reset(7000);
        p.vblank_seen(1_000_000, 100);
        p.learn_build(2000);
        p.learn_gpu(2000);
        // 1 ms after a vblank, a flip pending for the next one (1_007_000): aim for 1_014_000, begin 5.5 ms before it
        let now = 1_001_000;
        let s = p.start(now, Some(p.vblank_from(now + 1)), now);
        assert_eq!(s, 1_014_000 - 2000 - 2000 - super::PACE_MARGIN);
        // nothing taken: the next vblank is still reachable
        assert_eq!(p.start(now, None, now), 1_007_000 - 5500);
    }

    #[test]
    fn a_heavy_frame_is_begun_at_once() {
        let mut p = super::Pacer::default();
        p.reset(7000);
        p.vblank_seen(1_000_000, 100);
        p.learn_build(3000);
        p.learn_gpu(12_000);
        let now = 1_001_000;
        // dearer than a refresh: never put off, whether the GPU is free, busy, or a vblank is taken
        assert_eq!(p.start(now, None, now), now);
        assert_eq!(p.start(now, Some(1_014_000), 1_010_000), now);
        // a stale grid: at once
        assert_eq!(p.start(2_000_000, None, 2_000_000), 2_000_000);
    }

    #[test]
    fn a_frame_just_under_a_refresh_is_still_paced() {
        let mut p = super::Pacer::default();
        p.reset(16_700);
        p.vblank_seen(1_000_000, 100);
        p.learn_build(2000);
        p.learn_gpu(4000);
        // 1 ms after a vblank, nothing taken: aim for the next one, 7.5 ms before it
        let now = 1_001_000;
        assert_eq!(p.start(now, None, now), 1_016_700 - 2000 - 4000 - super::PACE_MARGIN);
    }

    #[test]
    fn the_refresh_follows_the_flip_stamps_and_costs_fall_slowly() {
        let mut p = super::Pacer::default();
        p.reset(6944);
        p.vblank_seen(1_000_000, 10);
        p.vblank_seen(1_000_000 + 2 * 6950, 12);
        assert!(p.period > 6944 && p.period <= 6950);
        p.learn_gpu(10_000);
        p.learn_gpu(1000);
        assert!(p.gpu > 9000.0);
    }

    #[test]
    fn a_still_desktop_is_looked_at_less_often_but_never_past_the_next_second() {
        // the quick rate first, so that what is about to move moves at once
        assert!(idle_wait(0, false) <= 33);
        // then slower; never more than a second (a clock with seconds), and a second at most with the screen off
        for streak in 0..50 {
            let w = idle_wait(streak, false);
            assert!((0..=200).contains(&w), "{streak}: {w}");
        }
        assert_eq!(idle_wait(0, true), 1000);
    }
}
