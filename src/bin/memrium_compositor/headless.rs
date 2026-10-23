//! Headless backend: the whole shell drawn offscreen, no window and no display needed. For screenshots in tests
//! (`--headless --shot out.png --after-ms 3000`), also when the desktop the nested window would need is not
//! on screen. Size: `MEMRIUM_HEADLESS_SIZE=1600x900`, scale: `MEMRIUM_HEADLESS_SCALE=1`.

use crate::nested::{apply_test_hooks, capture};
use crate::state::{ClientState, State};
use crate::Args;
use memrium_de::renderer::Renderer;
use smithay::{reexports::wayland_server::{Display, ListeningSocket}, wayland::compositor::CompositorClientState};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn run(mut args: Args) {
    let size = std::env::var("MEMRIUM_HEADLESS_SIZE").ok().and_then(|v| v.split_once('x').map(|(w, h)| (w.parse().unwrap_or(1600u32), h.parse().unwrap_or(900u32)))).unwrap_or((1600, 900));
    let scale: f32 = std::env::var("MEMRIUM_HEADLESS_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let (w, h) = ((size.0 as f32 * scale) as u32, (size.1 as f32 * scale) as u32);

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("no GPU adapter");
    eprintln!("adapter: {}", adapter.get_info().name);
    let (device, queue) = memrium_de::renderer::create_device(&adapter, &wgpu::DeviceDescriptor { required_features: if std::env::var_os("MEMRIUM_BENCH").is_some() { adapter.features() & wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() }, ..Default::default() }).expect("GPU device");
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let mut renderer = Renderer::new(device.clone(), queue, format, w, h, scale, crate::load_wallpaper(&args));

    if std::env::var_os("MEMRIUM_BENCH").is_some() && !renderer.enable_gpu_timer() {
        eprintln!("bench: this GPU has no timestamp queries");
    }
    let mut display: Display<State> = Display::new().unwrap();
    let dh = display.handle();
    let listener = ListeningSocket::bind_auto("memrium", 1..32).expect("could not create a wayland socket");
    let socket = listener.socket_name().unwrap().to_string_lossy().to_string();
    eprintln!("listening on WAYLAND_DISPLAY={socket}");
    let mut state = State::new(&dh, renderer, (w, h), scale);
    crate::nested::nested_layout(&mut state, w, h, scale);
    state.socket = socket.clone();
    state.default_wallpaper = Some(std::path::PathBuf::from(crate::wallpaper_path(&args)));
    if let Some(id) = &args.live {
        state.theme.wallpaper = memrium_de::theme::WallpaperSpec { kind: memrium_de::theme::WallKind::Live, path: id.clone() };
    }
    state.init_wallpaper();
    apply_test_hooks(&mut state, &mut args, &socket);
    state.allow_skip = true;
    state.persistent_target = true;
    let mut calloop: calloop::EventLoop<'static, State> = calloop::EventLoop::try_new().expect("event loop");
    state.init_dmabuf();
    state.init_screencopy();
    state.start_notifications();
    state.tray.start();
    state.appmenu.start();
    state.start_xwayland(calloop.handle());

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let (path, after) = args.shot.clone().unwrap_or_else(|| ("headless.png".into(), 3000));
    let mut last = Instant::now();
    // test hook: open Files part-way through, to catch the menu bar while its names glide
    let files_at: Option<u128> = std::env::var("MEMRIUM_TEST_FILES_AT").ok().and_then(|v| v.parse().ok());
    let mut files_opened = false;
    // test hook: the same for Settings (`MEMRIUM_TEST_SETTINGS_AT`)
    let settings_at: Option<u128> = std::env::var("MEMRIUM_TEST_SETTINGS_AT").ok().and_then(|v| v.parse().ok());
    let mut settings_opened = false;
    // test hook: the launcher (`MEMRIUM_TEST_LAUNCHER_AT`)
    let launcher_at: Option<u128> = std::env::var("MEMRIUM_TEST_LAUNCHER_AT").ok().and_then(|v| v.parse().ok());
    let mut launcher_opened = false;
    // test hook: a bar action part-way through (`MEMRIUM_TEST_ACTION=audio`, at `MEMRIUM_TEST_ACTION_AT` ms)
    let action: Option<(u128, String)> = std::env::var("MEMRIUM_TEST_ACTION").ok().map(|a| (std::env::var("MEMRIUM_TEST_ACTION_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(500), a));
    let mut action_done = false;
    // test hook: scripted pointer input, `MEMRIUM_TEST_INPUT="2000:move 520,300;2100:down;2200:move 640,300;2300:up"`
    // (milliseconds since the start; desk N | swipe begin|dx|end | move x,y | down | up | rdown | rup | scroll dy | key enter|esc|backspace|delete|f2 | text abc)
    let mut script: Vec<(u128, String)> = std::env::var("MEMRIUM_TEST_INPUT")
        .ok()
        .map(|v| v.split(';').filter_map(|step| step.trim().split_once(':').and_then(|(t, c)| Some((t.trim().parse().ok()?, c.trim().to_string())))).collect())
        .unwrap_or_default();
    script.sort_by_key(|s| s.0);
    script.reverse();
    // test hook: MEMRIUM_BENCH=1 times every frame that is drawn (CPU and GPU) and prints the figures at the end
    let bench = std::env::var_os("MEMRIUM_BENCH").is_some();
    let bench_from: u128 = std::env::var("MEMRIUM_BENCH_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut times: Vec<f32> = Vec::new();
    let mut cpus: Vec<f32> = Vec::new();
    // how much of the screen each benched frame redrew (kpx)
    let mut redrawn: Vec<f32> = Vec::new();
    let mut gpus: Vec<(f32, f32)> = Vec::new();
    loop {
        while script.last().is_some_and(|(at, _)| state.start.elapsed().as_millis() >= *at) {
            let (_, cmd) = script.pop().unwrap();
            let mut it = cmd.split_whitespace();
            match it.next() {
                Some("move") => {
                    if let Some((x, y)) = it.next().and_then(|c| c.split_once(',')) {
                        state.pointer_motion(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                    }
                }
                Some("down") => state.pointer_button(0x110, true),
                Some("up") => state.pointer_button(0x110, false),
                Some("rdown") => state.pointer_button(0x111, true),
                Some("rup") => state.pointer_button(0x111, false),
                Some("key") => {
                    use smithay::input::keyboard::Keysym as K;
                    let spec = it.next().unwrap_or("");
                    let mut mods = smithay::input::keyboard::ModifiersState::default();
                    let mut parts: Vec<&str> = spec.split('+').collect();
                    let last = parts.pop().unwrap_or("");
                    for m in parts {
                        match m {
                            "ctrl" => mods.ctrl = true,
                            "alt" => mods.alt = true,
                            "shift" => mods.shift = true,
                            "super" => mods.logo = true,
                            _ => {}
                        }
                    }
                    let sym = match Some(last) {
                        Some("space") => K::space,
                        Some("slash") => K::slash,
                        Some("enter") => K::Return,
                        Some("esc") => K::Escape,
                        Some("backspace") => K::BackSpace,
                        Some("delete") => K::Delete,
                        Some("f2") => K::F2,
                        Some("left") => K::Left,
                        Some("right") => K::Right,
                        Some("up") => K::Up,
                        Some("down") => K::Down,
                        other => {
                            eprintln!("test input: unknown key {other:?}");
                            continue;
                        }
                    };
                    state.test_chord(mods, sym);
                }
                // `text abc`: the letters one after the other (no spaces: the steps are split at spaces)
                Some("text") => {
                    for c in it.next().unwrap_or("").chars() {
                        state.test_key(smithay::input::keyboard::Keysym::from_char(c));
                    }
                }
                // real key presses (evdev codes, as from the keyboard), so they reach the focused program: `type ls_-la`
                // (`_` is a space, `~` is Enter)
                Some("type") => {
                    for c in it.next().unwrap_or("").chars() {
                        let (code, shift) = evdev_of(c);
                        if code == 0 {
                            continue;
                        }
                        if shift {
                            state.key(42, true);
                        }
                        state.key(code, true);
                        state.key(code, false);
                        if shift {
                            state.key(42, false);
                        }
                    }
                }
                // `desk 2`: go to a desktop; `swipe begin|-300|end`: a three-finger swipe
                Some("desk") => state.switch_workspace(it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1)),
                Some("swipe") => match it.next() {
                    Some("begin") => state.ws_swipe_begin(),
                    Some("end") => state.ws_swipe_end(false),
                    Some(v) => state.ws_swipe_move(v.parse().unwrap_or(0.0), 0.016),
                    None => {}
                },
                Some("notify") => state.test_notify(it.next().unwrap_or("")),
                Some("scroll") => state.pointer_axis(0.0, it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0)),
                _ => eprintln!("test input: unknown step {cmd:?}"),
            }
        }
        if let (Some(at), false) = (files_at, files_opened) {
            if state.start.elapsed().as_millis() > at {
                state.open_shell(crate::state::Sw::Files);
                files_opened = true;
            }
        }
        if let (Some(at), false) = (settings_at, settings_opened) {
            if state.start.elapsed().as_millis() > at {
                state.open_shell(crate::state::Sw::Settings);
                settings_opened = true;
            }
        }
        if let (Some(at), false) = (launcher_at, launcher_opened) {
            if state.start.elapsed().as_millis() > at {
                state.launcher.toggle();
                launcher_opened = true;
            }
        }
        if let (Some((at, name)), false) = (&action, action_done) {
            if state.start.elapsed().as_millis() > *at {
                state.bar_action(name);
                action_done = true;
            }
        }
        let _ = calloop.dispatch(Some(Duration::ZERO), &mut state);
        while let Ok(Some(stream)) = listener.accept() {
            let _ = dh.clone().insert_client(stream, Arc::new(ClientState { compositor: CompositorClientState::default() }));
        }
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        let now = Instant::now();
        state.update((now - last).as_secs_f32());
        last = now;
        let view = target.create_view(&Default::default());
        let t0 = Instant::now();
        if let Some(enc) = state.render_lazy(|| Some(view)) {
            let cpu = t0.elapsed().as_secs_f32() * 1000.0;
            state.renderer.queue().submit([enc.finish()]);
            if bench && state.start.elapsed().as_millis() >= bench_from {
                cpus.push(cpu);
                redrawn.push(state.renderer.last_region().map_or((w * h) as f32, |r| (r.2 * r.3) as f32) / 1000.0);
                if let Some(g) = state.renderer.gpu_times() {
                    gpus.push(g);
                }
                let _ = state.renderer.device().poll(wgpu::PollType::Wait);
                times.push(t0.elapsed().as_secs_f32() * 1000.0);
            }
        }
        state.send_frame_callbacks();
        let _ = display.flush_clients();
        if state.start.elapsed().as_millis() > after {
            let px = capture(&mut state, &target, w, h, format);
            image::save_buffer(&path, &px, w, h, image::ColorType::Rgba8).unwrap();
            eprintln!("saved {path} ({} windows)", state.wins.len());
            if bench && !times.is_empty() {
                let warm = if bench_from > 0 { 0 } else { times.len() / 5 };
                let mut t: Vec<f32> = times[warm..].to_vec();
                t.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let avg = t.iter().sum::<f32>() / t.len() as f32;
                if !gpus.is_empty() {
                    let g = &gpus[warm.min(gpus.len() - 1)..];
                    eprintln!("bench: GPU window pass avg {:.2} ms, shell pass avg {:.2} ms", g.iter().map(|x| x.0).sum::<f32>() / g.len() as f32, g.iter().map(|x| x.1).sum::<f32>() / g.len() as f32);
                }
                eprintln!("bench: CPU (building the frame) avg {:.2} ms", cpus[warm..].iter().sum::<f32>() / cpus[warm..].len() as f32);
                eprintln!("bench: {} frames drawn, avg {:.2} ms, median {:.2}, p95 {:.2}, max {:.2}", t.len(), avg, t[t.len() / 2], t[t.len() * 95 / 100], t[t.len() - 1]);
                eprintln!("bench: redrawn avg {:.0} kpx of {:.0} per frame", redrawn[warm..].iter().sum::<f32>() / redrawn[warm..].len() as f32, (w * h) as f32 / 1000.0);
            }
            // a debugging aid: where the GPU memory went
            if std::env::var_os("MEMRIUM_MEM_REPORT").is_some() {
                if let Some(r) = state.renderer.device().generate_allocator_report() {
                    eprintln!("gpu memory: {:.1} MB allocated in {} blocks, {:.1} MB reserved", r.total_allocated_bytes as f64 / 1e6, r.blocks.len(), r.total_reserved_bytes as f64 / 1e6);
                    for a in r.allocations.iter().filter(|a| a.size > 1_000_000) {
                        eprintln!("  {:>7.1} MB  {}", a.size as f64 / 1e6, a.name);
                    }
                }
            }
            return;
        }
        if !bench {
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}

/// The evdev key code of a character on a US layout, and whether it needs Shift (0: no such key here).
fn evdev_of(c: char) -> (u32, bool) {
    const ROWS: [(&str, u32); 4] = [("1234567890-=", 2), ("qwertyuiop[]", 16), ("asdfghjkl;'", 30), ("zxcvbnm,./", 44)];
    const SHIFTED: [(&str, u32); 4] = [("!@#$%^&*()_+", 2), ("QWERTYUIOP{}", 16), ("ASDFGHJKL:\"", 30), ("ZXCVBNM<>?", 44)];
    match c {
        '_' => return (57, false),
        '~' => return (28, false),
        _ => {}
    }
    for (row, first) in ROWS {
        if let Some(i) = row.find(c) {
            return (first + i as u32, false);
        }
    }
    for (row, first) in SHIFTED {
        if let Some(i) = row.find(c) {
            return (first + i as u32, true);
        }
    }
    (0, false)
}
