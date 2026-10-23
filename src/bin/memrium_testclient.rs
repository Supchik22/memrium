//! Developer tool: a Wayland client that exercises what simple terminals do not - a subsurface and an
//! xdg_popup - so the compositor's surface-tree and popup placement can be checked.
//!
//!   memrium_testclient [--csd]     (--csd asks for client-side decorations)

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_subcompositor, delegate_xdg_popup, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        xdg::{
            popup::{ConfigureKind, Popup, PopupConfigure, PopupHandler},
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgPositioner, XdgShell, XdgSurface,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
    subcompositor::SubcompositorState,
};
use std::time::Duration;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_subsurface::WlSubsurface, wl_surface},
    Connection, QueueHandle,
};
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity};

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    xdg_shell: XdgShell,
    subcompositor: SubcompositorState,
    window: Window,
    sub: Option<(WlSubsurface, wl_surface::WlSurface)>,
    popup: Option<Popup>,
    started: bool,
    exit: bool,
}

fn fill(pool: &mut SlotPool, surface: &wl_surface::WlSurface, w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 4]) {
    let (buffer, canvas) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888).expect("buffer");
    for y in 0..h {
        for x in 0..w {
            let [r, g, b, a] = f(x, y);
            // premultiplied ARGB, little endian
            let (pr, pg, pb) = (r as u32 * a as u32 / 255, g as u32 * a as u32 / 255, b as u32 * a as u32 / 255);
            let i = ((y * w + x) * 4) as usize;
            canvas[i..i + 4].copy_from_slice(&[pb as u8, pg as u8, pr as u8, a]);
        }
    }
    surface.damage_buffer(0, 0, w, h);
    buffer.attach_to(surface).expect("attach");
    surface.commit();
}

impl App {
    fn draw_main(&mut self, w: i32, h: i32) {
        let surface = self.window.wl_surface().clone();
        fill(&mut self.pool, &surface, w, h, |x, y| {
            // light window with a darker "button" where the popup is anchored
            if (20..120).contains(&x) && (20..44).contains(&y) {
                [70, 110, 220, 255]
            } else {
                [(235 - y / 4) as u8, (238 - y / 5) as u8, 245, 255]
            }
        });
    }

    fn start_extras(&mut self, qh: &QueueHandle<App>) {
        if self.started {
            return;
        }
        self.started = true;
        // subsurface: a red square with a green border, offset inside the window
        let (sub, surf) = self.subcompositor.create_subsurface(self.window.wl_surface().clone(), qh);
        sub.set_position(260, 90);
        sub.set_desync();
        fill(&mut self.pool, &surf, 140, 90, |x, y| if x < 4 || y < 4 || x >= 136 || y >= 86 { [40, 180, 80, 255] } else { [220, 70, 60, 255] });
        self.sub = Some((sub, surf));

        // popup below the blue "button"
        let positioner = XdgPositioner::new(&self.xdg_shell).expect("positioner");
        positioner.set_size(200, 130);
        positioner.set_anchor_rect(20, 20, 100, 24);
        positioner.set_anchor(Anchor::BottomLeft);
        positioner.set_gravity(Gravity::BottomRight);
        let popup = Popup::new(self.window.xdg_surface(), &positioner, qh, &self.compositor, &self.xdg_shell).expect("popup");
        self.popup = Some(popup);
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        let w = configure.new_size.0.map(|v| v.get()).unwrap_or(520) as i32;
        let h = configure.new_size.1.map(|v| v.get()).unwrap_or(340) as i32;
        self.draw_main(w, h);
        self.start_extras(qh);
    }
}

impl PopupHandler for App {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, config: PopupConfigure) {
        eprintln!("popup configure: {}x{} at ({},{}) {:?}", config.width, config.height, config.position.0, config.position.1, matches!(config.kind, ConfigureKind::Initial));
        let (w, h) = (config.width.max(1), config.height.max(1));
        let surface = popup.wl_surface().clone();
        fill(&mut self.pool, &surface, w, h, |x, y| {
            let row = (y / 26) % 2 == 0;
            if y % 26 < 2 {
                [200, 205, 215, 255]
            } else if row {
                [250, 250, 252, 255]
            } else {
                [238, 241, 247, 255]
            }
            .map(|c: u8| if x < 2 { c.saturating_sub(30) } else { c })
        });
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Popup) {
        eprintln!("popup dismissed by the compositor");
        self.popup = None;
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(App);
delegate_subcompositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_xdg_popup!(App);
delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

fn main() {
    let csd = std::env::args().any(|a| a == "--csd");
    let conn = Connection::connect_to_env().expect("no Wayland display");
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().unwrap();
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle()).unwrap();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let subcompositor = SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &qh).expect("wl_subcompositor");

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, if csd { WindowDecorations::RequestClient } else { WindowDecorations::RequestServer }, &qh);
    window.set_title("memrium_testclient");
    window.set_app_id("memrium-testclient");
    window.commit();

    let pool = SlotPool::new(4 * 1024 * 1024, &shm).expect("pool");
    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        xdg_shell,
        subcompositor,
        window,
        sub: None,
        popup: None,
        started: false,
        exit: false,
    };
    while !app.exit {
        if event_loop.dispatch(Duration::from_millis(100), &mut app).is_err() {
            break;
        }
    }
}
