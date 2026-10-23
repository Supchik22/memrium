//! Jelly Glass shell preview.
//!
//!   glass-preview            live window (hover the Dock / Quick Settings, click icons)
//!   glass-preview --frames   headless: simulate pointer motion, write frames/*.png

use memrium_de::renderer::{render_to_rgba, Item, Panel, Renderer};
use memrium_de::scene::Scene;
use std::sync::Arc;
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

fn main() {
    if std::env::args().any(|a| a == "--frames") {
        frames();
    } else {
        let ev = EventLoop::new().unwrap();
        ev.run_app(&mut App::default()).unwrap();
    }
}

const DEFAULT_WALLPAPER: &str = "/nix/store/1n95gvf26ipr5d6vavyjzam7879h8qps-plasma-workspace-wallpapers-6.5.6/share/wallpapers/Altai/contents/images/5120x2880.png";

/// `--wallpaper PATH`, else the Altai test wallpaper; falls back to the procedural one.
fn load_wallpaper() -> Option<image::RgbaImage> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.iter().position(|a| a == "--wallpaper").and_then(|i| args.get(i + 1).cloned()).unwrap_or_else(|| DEFAULT_WALLPAPER.to_string());
    match image::open(&path) {
        Ok(img) => {
            eprintln!("wallpaper: {path}");
            Some(img.to_rgba8())
        }
        Err(e) => {
            eprintln!("wallpaper {path}: {e} (using procedural)");
            None
        }
    }
}

/// Window layer + shell layer (with the shell's text labels appended).
fn frame_layers(r: &mut Renderer, scene: &Scene) -> (Vec<Item<'static>>, Vec<Panel>) {
    let (win, mut shell) = scene.layers();
    shell.extend(r.label_panels(&scene.labels()));
    (win, shell)
}

fn gpu() -> (wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("no adapter");
    eprintln!("adapter: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
    (instance, adapter, device, queue)
}

// ---------------------------------------------------------------- headless

fn frames() {
    const W: u32 = 1600;
    const H: u32 = 900;
    const S: u32 = 2;
    let (_i, _a, device, queue) = gpu();
    let mut r = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, W * S, H * S, S as f32, load_wallpaper());
    let mut scene = Scene::new(W as f32, H as f32);
    std::fs::create_dir_all("frames").unwrap();

    // (time s, label, crop in logical px: x, y, w, h)
    let dock_crop = (380u32, 690u32, 840u32, 210u32);
    let cc_crop = (1230u32, 30u32, 370u32, 470u32);
    let dt = 1.0 / 120.0;
    let mut t = 0.0f32;
    let mut shots: Vec<(f32, &str, (u32, u32, u32, u32))> = vec![
        (0.05, "dock_rest", dock_crop),
        (0.55, "dock_enter", dock_crop),
        (1.00, "dock_hover", dock_crop),
        (1.20, "dock_click", dock_crop),
        (1.28, "dock_bounce", dock_crop),
        (2.40, "tile_hover", cc_crop),
        (2.70, "tile_press", cc_crop),
        (3.30, "window_drag", (0, 0, 1600, 900)),
    ];
    shots.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut next = 0;
    while next < shots.len() {
        // scripted pointer
        if t < 0.4 {
            scene.set_mouse(-500.0, -500.0);
        } else if t < 1.0 {
            let k = (t - 0.4) / 0.6;
            scene.set_mouse(640.0 + 320.0 * k, 838.0); // sweep across the Dock
        } else if t < 2.0 {
            scene.set_mouse(960.0, 838.0);
        } else if t < 3.0 {
            scene.set_mouse(1420.0, 340.0); // over the Display tile
        } else {
            let k = ((t - 3.0) / 0.4).min(1.0);
            scene.set_mouse(820.0 + 100.0 * k, 170.0 + 90.0 * k); // drag the Notes window by its title bar
        }
        if (t - 3.0).abs() < dt * 0.5 {
            scene.set_mouse(820.0, 170.0);
            scene.mouse_down();
        }
        if (t - 1.2).abs() < dt * 0.5 {
            scene.mouse_down();
        }
        if (t - 1.24).abs() < dt * 0.5 || (t - 2.9).abs() < dt * 0.5 {
            scene.mouse_up();
        }
        if (t - 2.5).abs() < dt * 0.5 {
            scene.mouse_down();
        }
        scene.update(dt);
        t += dt;
        if t >= shots[next].0 {
            let (_, name, (cx, cy, cw, ch)) = shots[next];
            let px = { let (w, sh) = frame_layers(&mut r, &scene); render_to_rgba(&mut r, W * S, H * S, &w, &sh) };
            let img = image::RgbaImage::from_raw(W * S, H * S, px).unwrap();
            let crop = image::imageops::crop_imm(&img, cx * S, cy * S, cw * S, ch * S).to_image();
            crop.save(format!("frames/{name}.png")).unwrap();
            eprintln!("frames/{name}.png");
            next += 1;
        }
    }
    // also a full-frame still of the resting state
    let scene = Scene::new(W as f32, H as f32);
    let px = { let (w, sh) = frame_layers(&mut r, &scene); render_to_rgba(&mut r, W * S, H * S, &w, &sh) };
    image::save_buffer("out.png", &px, W * S, H * S, image::ColorType::Rgba8).unwrap();

    // the "Edit Controls" mode on a bare desktop (springs settled), like the macOS reference
    let mut edit = Scene::shell_only(W as f32, H as f32);
    edit.set_edit(true);
    for _ in 0..360 {
        edit.update(1.0 / 120.0);
    }
    let (w, sh) = frame_layers(&mut r, &edit);
    let px = render_to_rgba(&mut r, W * S, H * S, &w, &sh);
    image::save_buffer("edit.png", &px, W * S, H * S, image::ColorType::Rgba8).unwrap();
    eprintln!("edit.png");
}

// -------------------------------------------------------------------- live

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    scene: Scene,
    last: Instant,
}

#[derive(Default)]
struct App {
    gfx: Option<Gfx>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes().with_title("Jelly Glass").with_inner_size(winit::dpi::LogicalSize::new(1600, 900));
        let window = Arc::new(el.create_window(attrs).unwrap());
        let (instance, adapter, device, queue) = gpu();
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
        let renderer = Renderer::new(device, queue, format, config.width, config.height, sf, load_wallpaper());
        let [lw, lh] = renderer.logical_size();
        window.request_redraw();
        self.gfx = Some(Gfx { window, surface, config, renderer, scene: Scene::new(lw, lh), last: Instant::now() });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let Some(g) = self.gfx.as_mut() else { return };
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                g.config.width = s.width.max(1);
                g.config.height = s.height.max(1);
                g.surface.configure(g.renderer.device(), &g.config);
                let sf = g.window.scale_factor() as f32;
                g.renderer.resize(g.config.width, g.config.height, sf);
                let [lw, lh] = g.renderer.logical_size();
                g.scene.resize(lw, lh);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let sf = g.window.scale_factor();
                g.scene.set_mouse((position.x / sf) as f32, (position.y / sf) as f32);
            }
            WindowEvent::CursorLeft { .. } => g.scene.set_mouse(-1000.0, -1000.0),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => g.scene.mouse_down(),
                ElementState::Released => g.scene.mouse_up(),
            },
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - g.last).as_secs_f32();
                g.last = now;
                g.scene.update(dt);
                let Ok(frame) = g.surface.get_current_texture() else { return };
                let view = frame.texture.create_view(&Default::default());
                let (win, shell) = frame_layers(&mut g.renderer, &g.scene);
                let enc = g.renderer.render(&view, &win, &shell);
                g.renderer.queue().submit([enc.finish()]);
                g.window.pre_present_notify();
                frame.present();
                g.window.request_redraw();
            }
            _ => {}
        }
    }
}
