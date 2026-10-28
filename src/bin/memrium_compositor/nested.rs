//! Nested backend: the compositor runs inside a window of the host session (winit + wgpu).

use crate::state::{ClientState, State};
use crate::Args;
use memrium_de::renderer::{render_to_rgba, Renderer};
use smithay::{
    reexports::wayland_server::{Display, ListeningSocket},
    wayland::compositor::CompositorClientState,
};
use std::sync::Arc;
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::PhysicalKey,
    platform::scancode::PhysicalKeyExtScancode,
    window::{Window, WindowId},
};

const BTN_LEFT: u32 = 0x110;

// Field order matters: fields drop top to bottom, so the wayland side and GPU objects go before the host window.
struct Gfx {
    listener: ListeningSocket,
    state: State,
    display: Display<State>,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    config: wgpu::SurfaceConfiguration,
    last: Instant,
    shot: Option<(String, u128)>,
    /// the last frame was identical to the one before it: sleep instead of redrawing
    idle: bool,
    /// timers and the X server's sockets (XWayland)
    calloop: calloop::EventLoop<'static, State>,
}

impl Gfx {
    /// Accept new clients and process their requests.
    fn pump_wayland(&mut self) {
        let _ = self.calloop.dispatch(Some(std::time::Duration::ZERO), &mut self.state);
        while let Ok(Some(stream)) = self.listener.accept() {
            let _ = self.display.handle().insert_client(stream, Arc::new(ClientState { compositor: CompositorClientState::default() }));
        }
        self.display.dispatch_clients(&mut self.state).unwrap();
        self.display.flush_clients().unwrap();
    }
}

/// Debug switches (`--settings`, `--files`, `MEMRIUM_TEST_*` ...) that put the shell into a state worth a screenshot.
pub fn apply_test_hooks(state: &mut State, args: &mut Args, socket: &str) {
    if let Some(page) = args.settings {
        state.open_shell(crate::state::Sw::Settings);
        state.settings.set_page(page);
    }
    if let Some(dir) = &args.files {
        state.open_shell(crate::state::Sw::Files);
        state.files.goto(std::path::PathBuf::from(dir), true);
    }
    if args.cc {
        state.scene.set_cc_open(true);
    }
    if std::env::var_os("MEMRIUM_TEST_MAXIMIZE").is_some() {
        state.toggle_shell_maximize(crate::state::Sw::Settings);
    }
    if let Ok(m) = std::env::var("MEMRIUM_TEST_GLASS") {
        use memrium_de::theme::GlassMode;
        let mut t = state.theme.clone();
        t.glass.set_mode(match m.as_str() { "flat" => GlassMode::Flat, "glass" => GlassMode::Glass, "clay" => GlassMode::Clay, "paper" => GlassMode::Paper, "wood" => GlassMode::Wood, _ => GlassMode::Custom });
        state.apply_theme(t);
    }
    if std::env::var_os("MEMRIUM_TEST_WIDGETS").is_some() {
        state.toggle_widgets();
    }
    // test hook: desktop widgets of these kinds, `MEMRIUM_TEST_DESKWIDGET=visualizer,clock`
    if let Ok(kinds) = std::env::var("MEMRIUM_TEST_DESKWIDGET") {
        for k in kinds.split(if kinds.contains('|') { '|' } else { ',' }) {
            // `kind@x,y` puts it there (then the widgets are told apart by `|`)
            match k.split_once('@').and_then(|(kind, at)| at.split_once(',').map(|(x, y)| (kind, x.parse::<f32>().unwrap_or(0.0), y.parse::<f32>().unwrap_or(0.0)))) {
                Some((kind, x, y)) => {
                    state.config.desk_widgets.push(memrium_de::deskwidgets::DeskWidget { kind: kind.to_string(), x, y, ..Default::default() });
                }
                None => state.desk_add_widget(k),
            }
        }
    }
    if std::env::var_os("MEMRIUM_TEST_DARK").is_some() {
        state.mouse = [state.size[0] * 0.8, state.size[1] * 0.9];
        state.bar_action("dark-mode");
    }
    // test hook: a hardware key, `MEMRIUM_TEST_MEDIAKEY=volume_up|volume_down|mute|bright_up|bright_down|play|next|prev`
    if let Ok(k) = std::env::var("MEMRIUM_TEST_MEDIAKEY") {
        let raw = match k.as_str() {
            "volume_up" => 0x1008FF13,
            "volume_down" => 0x1008FF11,
            "mute" => 0x1008FF12,
            "bright_up" => 0x1008FF02,
            "bright_down" => 0x1008FF03,
            "play" => 0x1008FF14,
            "next" => 0x1008FF17,
            _ => 0x1008FF16,
        };
        state.media_key(raw);
    }
    if let Ok(k) = std::env::var("MEMRIUM_TEST_FLYOUT") {
        let kind = if k == "brightness" { crate::flyout::FlyKind::Brightness } else { crate::flyout::FlyKind::Volume };
        let x = state.size[0] - 120.0;
        state.toggle_flyout(kind, 0, x);
    }
    if args.lock {
        state.lock_screen();
        // test hook: type this on the lock screen (a trailing \n presses Enter)
        if let Ok(keys) = std::env::var("MEMRIUM_TEST_LOCK_KEYS") {
            let mods = smithay::input::keyboard::ModifiersState::default();
            for c in keys.chars() {
                let sym = if c == '\n' { smithay::input::keyboard::Keysym::Return } else { smithay::input::keyboard::Keysym::from_char(c) };
                state.lock_key(&mods, sym);
            }
        }
    }
    if let Ok(v) = std::env::var("MEMRIUM_TEST_GENIE") {
        // debug: a Settings window caught part-way into the Dock
        state.open_shell(crate::state::Sw::Settings);
        state.swin.genie = true;
        state.swin.k.x = v.parse().unwrap_or(0.5);
        state.swin.k.target = state.swin.k.x;
    }
    match std::env::var("MEMRIUM_TEST_PANEL").as_deref() {
        Ok("edit") => state.panel_edit = Some(0),
        Ok("add") => {
            state.add_widget(0, "button");
            state.panel_menu_command(7011, 0, None);
            state.panel_menu_command(7010 + 2, 0, None);
        }
        Ok("menu") => {
            state.mouse = [1560.0, 20.0];
            state.open_panel_menu(0);
        }
        _ => {}
    }
    match std::env::var("MEMRIUM_TEST_SHOT").as_deref() {
        Ok("full") => state.shot_req = Some(crate::state::ShotReq::Full),
        Ok("window") => state.shot_req = Some(crate::state::ShotReq::Window),
        Ok("region") => {
            state.mouse = [900.0, 520.0];
            state.shot_sel = Some(crate::state::ShotSel { start: Some([300.0, 200.0]), record: false });
        }
        _ => {}
    }
    if args.menu {
        state.mouse = [760.0, 300.0];
        state.open_desktop_menu();
    }
    if let Some(q) = &args.launcher {
        state.launcher.toggle();
        let apps = state.apps.clone();
        for c in q.chars() {
            state.launcher.type_char(c, &apps);
        }
    }
    if args.edit {
        state.scene.set_edit(true);
        if let Some(t) = std::env::var("MEMRIUM_TEST_GALLERY_TAB").ok().and_then(|v| v.parse().ok()) {
            state.scene.set_gallery_tab(t);
        }
    }
    if let Some(cmd) = args.exec.take() {
        let spawned = std::process::Command::new("sh").arg("-c").arg(&cmd).env("WAYLAND_DISPLAY", &socket).env("XDG_SESSION_TYPE", "wayland").spawn();
        match spawned {
            Ok(_) => eprintln!("started: {cmd}"),
            Err(e) => eprintln!("could not start {cmd}: {e}"),
        }
    }
}

/// Lay the window's canvas out as the desktop: one monitor, or (test hook) an imaginary one on the left.
pub(crate) fn nested_layout(state: &mut State, w_px: u32, h_px: u32, sf: f32) {
    use memrium_de::display::Placed;
    let fake: f32 = std::env::var("MEMRIUM_FAKE_LEFT").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let left = (fake * sf).round() as u32;
    if left == 0 || left + 200 >= w_px {
        state.resize_output(w_px, h_px, sf);
        return;
    }
    let main_w = w_px - left;
    let placed = [
        Placed { name: "main".into(), x: 0, y: 0, w: main_w, h: h_px, hz: 60, primary: true },
        Placed { name: "fake-left".into(), x: -(left as i32), y: 0, w: left, h: h_px, hz: 60, primary: false },
    ];
    state.apply_layout(&placed, sf);
}

struct App {
    gfx: Option<Gfx>,
    args: Args,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        el.set_control_flow(ControlFlow::Poll);
        // test hook: MEMRIUM_FAKE_LEFT=<logical px> adds an imaginary monitor of that width left of the main one
        let fake_left: f64 = std::env::var("MEMRIUM_FAKE_LEFT").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let attrs = Window::default_attributes().with_title("Jelly Glass").with_inner_size(winit::dpi::LogicalSize::new(1600.0 + fake_left, 900.0));
        let window = Arc::new(el.create_window(attrs).unwrap());

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("no adapter");
        eprintln!("adapter: {}", adapter.get_info().name);
        let (device, queue) = memrium_de::renderer::create_device(&adapter, &wgpu::DeviceDescriptor::default()).expect("device");
        let surface = instance.create_surface(window.clone()).unwrap();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let sf = window.scale_factor() as f32;
        let renderer = Renderer::new(device, queue, format, config.width, config.height, sf, crate::load_wallpaper(&self.args));

        let display: Display<State> = Display::new().unwrap();
        let dh = display.handle();
        let listener = ListeningSocket::bind_auto("memrium", 1..32).expect("could not create a wayland socket");
        let socket = listener.socket_name().unwrap().to_string_lossy().to_string();
        eprintln!("listening on WAYLAND_DISPLAY={socket}");

        let mut state = State::new(&dh, renderer, (config.width, config.height), sf);
        nested_layout(&mut state, config.width, config.height, sf);
        if std::env::var_os("MEMRIUM_FAKE_LEFT").is_some() {
            // lets the Displays page be tried without a second monitor
            use memrium_de::display::MonitorInfo;
            state.monitor_infos = vec![
                MonitorInfo { name: "eDP-1".into(), modes: vec![(1920, 1080, 60), (1920, 1080, 48), (1280, 720, 60)], mm: (344, 194) },
                MonitorInfo { name: "DP-3".into(), modes: vec![(1920, 1080, 60), (1680, 1050, 60), (1280, 1024, 60)], mm: (527, 296) },
            ];
        }
        state.socket = socket.clone();
        state.default_wallpaper = Some(std::path::PathBuf::from(crate::wallpaper_path(&self.args)));
        if let Some(id) = &self.args.live {
            let kind = if memrium_de::video::is_video(std::path::Path::new(id)) { memrium_de::theme::WallKind::Video } else { memrium_de::theme::WallKind::Live };
        state.theme.wallpaper = memrium_de::theme::WallpaperSpec { kind, path: id.clone() };
        }
        state.init_wallpaper();
        apply_test_hooks(&mut state, &mut self.args, &socket);
        // unchanged frames are skipped (and cost no GPU time); screenshots turn this off while they render
        state.allow_skip = true;
        let calloop = calloop::EventLoop::try_new().expect("event loop");
        state.init_dmabuf();
        state.init_syncobj(adapter.get_info().vendor);
        state.init_screencopy();
        state.start_notifications();
        state.tray.start();
        state.appmenu.start();
        state.start_xwayland(calloop.handle());
        window.request_redraw();
        self.gfx = Some(Gfx { listener, state, display, surface, window, config, last: Instant::now(), shot: self.args.shot.take(), idle: false, calloop });
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.gfx = None;
    }

    /// The idle timer fired: look at the clients and the clock again.
    fn new_events(&mut self, _el: &ActiveEventLoop, cause: winit::event::StartCause) {
        if let (Some(g), winit::event::StartCause::ResumeTimeReached { .. }) = (self.gfx.as_ref(), cause) {
            g.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(g) = self.gfx.as_mut() {
            g.pump_wayland();
            // the window's own cursor follows the shell's
            static SHOWN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
            if SHOWN.swap(g.state.cursor_gen, std::sync::atomic::Ordering::Relaxed) != g.state.cursor_gen {
                use memrium_de::cursor::Shape;
                use winit::window::CursorIcon as C;
                let icon = match g.state.cursor_shape {
                    Shape::Default | Shape::Hidden => C::Default,
                    Shape::Text => C::Text,
                    Shape::Pointer => C::Pointer,
                    Shape::Grab => C::Grab,
                    Shape::Grabbing => C::Grabbing,
                    Shape::Move => C::Move,
                    Shape::ResizeEw => C::EwResize,
                    Shape::ResizeNs => C::NsResize,
                    Shape::ResizeNesw => C::NeswResize,
                    Shape::ResizeNwse => C::NwseResize,
                    Shape::Wait => C::Wait,
                    Shape::Progress => C::Progress,
                    Shape::NotAllowed => C::NotAllowed,
                    Shape::Crosshair => C::Crosshair,
                    Shape::Copy => C::Copy,
                    Shape::Client => C::Default,
                };
                // a program's own picture becomes a cursor of the window system
                let own = g.state.client_cursor_img.as_ref().filter(|_| g.state.cursor_shape == Shape::Client).and_then(|c| {
                    let (w, h) = (c.w.min(u16::MAX as u32) as u16, c.h.min(u16::MAX as u32) as u16);
                    let (hx, hy) = (c.hot.0.clamp(0, w as i32 - 1) as u16, c.hot.1.clamp(0, h as i32 - 1) as u16);
                    winit::window::CustomCursor::from_rgba(c.rgba(), w, h, hx, hy).ok().map(|s| el.create_custom_cursor(s))
                });
                match own {
                    Some(c) => g.window.set_cursor(winit::window::Cursor::Custom(c)),
                    None => g.window.set_cursor(icon),
                }
                g.window.set_cursor_visible(g.state.cursor_shape != Shape::Hidden);
            }
            if g.idle {
                // nothing moved: sleep ~30 ms (input events wake the loop at once); a redraw is only requested when
                // the timer fires or something happens
                el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + std::time::Duration::from_millis(33)));
            } else {
                // busy: draw as fast as the display allows
                g.window.request_redraw();
                el.set_control_flow(ControlFlow::Poll);
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let bench = self.args.bench;
        let Some(g) = self.gfx.as_mut() else { return };
        // any input or window event may change the picture: wake up and draw
        if !matches!(ev, WindowEvent::RedrawRequested) {
            g.idle = false;
            g.window.request_redraw();
        }
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                g.config.width = s.width.max(1);
                g.config.height = s.height.max(1);
                g.surface.configure(g.state.renderer.device(), &g.config);
                let sf = g.window.scale_factor() as f32;
                nested_layout(&mut g.state, g.config.width, g.config.height, sf);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let sf = g.window.scale_factor();
                // the window shows the whole desktop; its left edge is the desktop's
                g.state.pointer_motion((position.x / sf) as f32 + g.state.world[0], (position.y / sf) as f32 + g.state.world[1]);
            }
            WindowEvent::CursorLeft { .. } => g.state.pointer_motion(-1000.0, -1000.0),
            WindowEvent::MouseInput { state, button, .. } => {
                let code = match button {
                    MouseButton::Left => BTN_LEFT,
                    MouseButton::Right => 0x111,
                    MouseButton::Middle => 0x112,
                    _ => return,
                };
                g.state.pointer_button(code, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                MouseScrollDelta::LineDelta(x, y) => g.state.pointer_axis(-x as f64 * 15.0, -y as f64 * 15.0),
                MouseScrollDelta::PixelDelta(p) => g.state.pointer_axis(-p.x, -p.y),
            },
            WindowEvent::KeyboardInput { event, .. } if !event.repeat => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if let Some(sc) = PhysicalKey::Code(code).to_scancode() {
                        g.state.key(sc, event.state == ElementState::Pressed);
                    }
                }
            }
            // the window contents may have been lost (minimised, covered, moved to another screen): redraw for sure
            WindowEvent::Occluded(false) | WindowEvent::Focused(true) | WindowEvent::ScaleFactorChanged { .. } => g.state.invalidate(),
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - g.last).as_secs_f32();
                g.last = now;
                g.state.update(dt);

                let mut frame = None;
                let surface = &g.surface;
                let enc = g.state.render_lazy(|| {
                    frame = surface.get_current_texture().ok();
                    frame.as_ref().map(|f| f.texture.create_view(&Default::default()))
                });
                g.idle = enc.is_none() && g.state.frame_skipped;
                if let (Some(enc), Some(frame)) = (enc, frame) {
                    g.state.renderer.queue().submit([enc.finish()]);
                    g.window.pre_present_notify();
                    frame.present();
                }

                g.state.send_frame_callbacks();
                let _ = g.display.flush_clients();

                if let Some((path, after)) = g.shot.clone() {
                    if g.state.start.elapsed().as_millis() > after {
                        // render the same frame offscreen so we can write it to disk
                        let (w, h) = (g.config.width, g.config.height);
                        let tex = g.state.renderer.device().create_texture(&wgpu::TextureDescriptor {
                            label: None,
                            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: g.config.format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        });
                        if let Some(n) = bench {
                            let _ = capture(&mut g.state, &tex, w, h, g.config.format);
                            let t = Instant::now();
                            for _ in 0..n {
                                let _ = capture(&mut g.state, &tex, w, h, g.config.format);
                            }
                            eprintln!("bench: {:.2} ms/frame at {w}x{h} ({} fps)", t.elapsed().as_secs_f64() * 1000.0 / n as f64, (n as f64 / t.elapsed().as_secs_f64()) as u32);
                        }
                        let px = capture(&mut g.state, &tex, w, h, g.config.format);
                        image::save_buffer(&path, &px, w, h, image::ColorType::Rgba8).unwrap();
                        eprintln!("saved {path} ({} windows)", g.state.wins.len());
                        el.exit();
                    }
                }
            }
            _ => {}
        }
    }
}

/// Render the current frame into `tex` and read it back as RGBA8.
pub(crate) fn capture(state: &mut State, tex: &wgpu::Texture, w: u32, h: u32, format: wgpu::TextureFormat) -> Vec<u8> {
    let view = tex.create_view(&Default::default());
    // a capture must really draw, even when the frame is identical to the last one
    let skip = std::mem::replace(&mut state.allow_skip, false);
    let mut enc = state.render(&view);
    state.allow_skip = skip;
    let padded = (w * 4 + 255) / 256 * 256;
    let buf = state.renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    state.renderer.queue().submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |x| x.unwrap());
    state.renderer.device().poll(wgpu::PollType::Wait).unwrap();
    let mapped = slice.get_mapped_range();
    let bgra = matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let line = &mapped[row * padded as usize..row * padded as usize + (w * 4) as usize];
        if bgra {
            for px in line.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(line);
        }
    }
    out
}

pub fn run(args: Args) {
    let ev = EventLoop::new().unwrap();
    ev.run_app(&mut App { gfx: None, args }).unwrap();
}

#[allow(dead_code)]
fn _unused(_: &mut Renderer) {
    let _ = render_to_rgba;
}
