//! smithay protocol handlers: surfaces, xdg-shell (toplevels + popups), decorations, seat, clipboard, output.

use crate::state::{Deco, Popup, State, Win};
use memrium_de::scene::Scene;
use chebrium::spring::Spring;
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm, delegate_xdg_decoration, delegate_xdg_shell,
    input::{
        keyboard::XkbConfig,
        pointer::CursorImageStatus,
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        wayland_protocols::xdg::{decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode, shell::server::xdg_toplevel},
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output, wl_seat::WlSeat, wl_surface::WlSurface},
            Client, DisplayHandle, Resource,
        },
    },
    utils::{Logical, Rectangle, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        selection::{
            data_device::{set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler},
            SelectionHandler,
        },
        shell::xdg::{
            decoration::{XdgDecorationHandler, XdgDecorationState},
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};
use std::collections::HashMap;
use std::time::Instant;

impl State {
    /// Create all protocol globals. `size_px` is the output in device pixels, `scale` the device scale factor.
    pub fn new(dh: &DisplayHandle, renderer: memrium_de::renderer::Renderer, size_px: (u32, u32), scale: f32) -> State {
        let out_scale = scale.round().max(1.0) as i32;
        crate::role::set_scale_grid(scale);
        let output = Output::new(
            "memrium-0".into(),
            PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "Memrium".into(), model: "Glass".into() },
        );
        let _ = output.create_global::<State>(dh);
        let mode = OutMode { size: (size_px.0 as i32, size_px.1 as i32).into(), refresh: 60_000 };
        output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Integer(out_scale)), Some((0, 0).into()));
        output.set_preferred(mode);

        let mut seat_state = SeatState::<State>::new();
        let mut seat = seat_state.new_wl_seat(dh, "seat0");
        let pointer = seat.add_pointer();
        let config = memrium_de::theme::Config::load();
        memrium_de::i18n::set_language(&config.language);
        let (layout, variant, options) = memrium_de::input::xkb_settings(&config.input);
        eprintln!("keyboard: layout {layout} {variant} options {options}");
        let xkb = XkbConfig { layout: &layout, variant: &variant, options: (!options.is_empty()).then(|| options.clone()), ..XkbConfig::default() };
        let keyboard = seat.add_keyboard(xkb, config.input.repeat_delay, config.input.repeat_rate).expect("keyboard (is XKB_CONFIG_ROOT set?)");

        memrium_de::keys::set_style(&config.keys.style);
        let [lw, lh] = renderer.logical_size();
        let theme = config.resolve();
        // the programs follow the shell's look: leave it where they can read it (changes are written when they happen)
        memrium_de::theme::export_appearance(&theme, &config.language);
        let apps = memrium_de::apps::scan();
        memrium_de::apps::set_icon_theme(&theme.icons.theme_name(theme.dark));
        // icon themes are big: index them off-thread
        let icons: std::sync::Arc<memrium_de::apps::IconSlot> = Default::default();
        {
            let icons = icons.clone();
            std::thread::spawn(move || {
                icons.set(memrium_de::apps::IconIndex::build());
            });
        }
        let action_icons: std::sync::Arc<memrium_de::apps::IconSlot> = Default::default();
        {
            let action_icons = action_icons.clone();
            std::thread::spawn(move || {
                action_icons.set(memrium_de::apps::IconIndex::build_actions());
            });
        }
        let icon_loader = crate::icon_loader::IconLoader::start(icons.clone());
        State {
            dh: dh.clone(),
            renderer,
            compositor_state: CompositorState::new::<State>(dh),
            xdg_shell_state: XdgShellState::new::<State>(dh),
            xdg_decoration_state: XdgDecorationState::new::<State>(dh),
            shm_state: ShmState::new::<State>(dh, vec![]),
            seat_state,
            data_device_state: DataDeviceState::new::<State>(dh),
            seat,
            pointer,
            keyboard,
            output,
            wins: Vec::new(),
            popups: Vec::new(),
            rules: memrium_de::apprules::Rules::load(),
            rules_checked: std::time::Instant::now(),
            surfaces: HashMap::new(),
            scene: Scene::shell_only_with(lw, lh, &theme),
            mouse: [-1000.0, -1000.0],
            grab: None,
            out_scale,
            size: [lw, lh],
            start: Instant::now(),
            next_id: 0,
            config: config.clone(),
            theme: theme.clone(),
            hover_btn: None,
            last_title_click: None,
            settings: Default::default(),
            swin: crate::state::ShellWin::new(),
            fwin: crate::state::ShellWin::new(),
            ghosts: Vec::new(),
            z: Vec::new(),
            stack_clock: 0,
            files: memrium_de::files::Files::new(config.files.clone()),
            fm: Default::default(),
            menu: Default::default(),
            menu_for: crate::state::MenuFor::Desktop,
            bar_hover: None,
            panel_edit: None,
            panel_drag: None,
            bars: theme.bars.iter().enumerate().map(|(i, c)| memrium_de::widget::Bar::new(i, c)).collect(),
            fileicons: crate::fileicons::IconCache::new(),
            desktop: memrium_de::desktop::Desktop::new(config.desktop.clone()),
            file_drag: None,
            input_dirty: false,
            presets: memrium_de::theme::available_themes(),
            walls: Vec::new(),
            wall_paths: Vec::new(),
            walls_loaded: false,
            thumb_rx: None,
            wall_rx: None,
            wall_accent: None,
            pending_theme: None,
            pending_actions: Vec::new(),
            cfg_dirty: None,
            default_wallpaper: None,
            menu_light: false,
            socket: String::new(),
            apps: apps.clone(),
            icons: icons.clone(),
            action_icons,
            title_menu: None,
            menu_search_target: None,
            icon_failed: Default::default(),
            icons_rebuilt: Default::default(),
            icons_marker: (std::time::Instant::now(), std::fs::metadata(memrium_de::iconrender::marker_path()).and_then(|m| m.modified()).ok()),
            icon_loader,
            pinned: default_pins(&apps),
            cursor_shape: Default::default(),
            client_cursor: Default::default(),
            client_cursor_surface: None,
            client_cursor_img: None,
            cursor_focus: None,
            cursor_check: std::time::Instant::now(),
            cursor_gen: 0,
            busy_until: None,
            busy_wins: 0,
            preview: None,
            dock_wait: (String::new(), 0.0),
            dock_linger: 0.0,
            dock_menu: None,
            tray: crate::tray_service::TrayService::new(),
            tray_items: Vec::new(),
            tray_expanded: false,
            appmenu: crate::appmenu_service::AppMenuService::new(),
            appmenu_target: None,
            events_from_desktop: false,
            appstyle_last: None,
            gtk_rx: None,
            gtk_next: std::time::Instant::now(),
            gtk_last: None,
            tray_shown: Vec::new(),
            clipboard: crate::clipboard::Clipboard::new(),
            kbd_layouts: layout.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            tray_retry: std::time::Instant::now(),
            axis_cache: std::cell::Cell::new([0.0; 2]),
            tray_menu: None,
            tip: ((usize::MAX, 0), 0.0, None),
            launcher: { let mut l = memrium_de::launcher::Launcher::default(); l.files = Some(memrium_de::figlet::FileIndex::new()); l },
            intercepted: Default::default(),
            quit: false,
            allow_skip: false,
            persistent_target: false,
            damage: Default::default(),
            shot_sel: None,
            shot_req: None,
            capturing: false,
            shot_notice: None,
            shot_flash: 0.0,
            snap: 0.0,
            snap_on: false,
            snap_side: 0,
            move_start: None,
            move_samples: Vec::new(),
            frame_skipped: false,
            frames_drawn: 0,
            last_hash: 0,
            pending_vt: None,
            xwm: None,
            xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState::new::<State>(dh),
            x_display: None,
            lock: None,
            sys_watcher: memrium_de::sysctl::SysWatcher::start(),
            sys: Default::default(),
            notifications: crate::notify::Notifications::new(),
            flyout: None,
            card: None,
            manual_block: false,
            reveal: None,
            flyout_last_tick: std::time::Instant::now(),
            workspace: 0,
            ws_pos: Spring::new(0.0, 170.0, 0.9),
            ws_drag: None,
            alt_tab: None,
            saved_env: Vec::new(),
            media: None,
            album: Default::default(),
            wall_ready: false,
            bar_mon: std::cell::Cell::new(0),
            bar_hover_mon: 0,
            bar_menu_mon: 0,
            desk_drag: None,
            desk_target: None,
            world: [0.0, 0.0, lw, lh],
            mons: vec![[0.0, 0.0, lw, lh]],
            main_mon: 0,
            monitor_infos: Vec::new(),
            displays_dirty: false,
            widgets_panel: crate::widgets_panel::WidgetsPanel::new(),
            text_input_state: smithay::wayland::text_input::TextInputManagerState::new::<State>(dh),
            input_method_state: smithay::wayland::input_method::InputMethodManagerState::new::<State, _>(dh, |_| true),
            virtual_keyboard_state: smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<State, _>(dh, |_| true),
            loop_handle: None,
            auto_dark_checked: None,
            last_input: std::time::Instant::now(),
            idle_done: [false; 3],
            screen_off: false,
            inhibitors: Vec::new(),
            idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<State>(dh),
            theme_next_anim: Default::default(),
            protocols: crate::protocols::Protocols::new(dh),
            export_env: false,
            screencopies: Vec::new(),
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            dmabuf_global: None,
            dmabuf_tex: HashMap::new(),
            dmabuf_logged: 0,
        }
    }

    fn remove_popup(&mut self, surface: &PopupSurface) {
        let gone = surface.wl_surface().clone();
        self.drop_popups(|p| p.surface.wl_surface() == &gone);
        self.apply_focus();
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // our own clients, or XWayland (which carries its own client data)
        if let Some(c) = client.get_data::<crate::state::ClientState>() {
            return &c.compositor;
        }
        &client.get_data::<smithay::xwayland::XWaylandClientData>().expect("unknown client type").compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        self.on_commit(surface);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, buffer: &WlBuffer) {
        self.dmabuf_tex.remove(&buffer.id());
    }
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let n = (self.wins.iter().filter(|w| !w.or).count() % 6) as f32;
        let (x, y) = (200.0 + 44.0 * n, 80.0 + 40.0 * n);
        let win = self.new_win(crate::role::Role::Xdg(surface), x, y);
        let at = self.wins.iter().rposition(|o| !o.or).map(|p| p + 1).unwrap_or(0);
        self.wins.insert(at, win);
        self.apply_focus();
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        // place it, keeping it on screen (flip/slide per the client's constraint adjustment)
        let mut geometry = positioner.get_geometry();
        if let Some(parent) = surface.get_parent_surface() {
            if let Some(o) = self.popup_parent_origin(&parent) {
                let target = Rectangle::<i32, Logical>::new((self.world[0] as i32 - o[0] as i32, self.world[1] as i32 - o[1] as i32).into(), ((self.world[2] - self.world[0]) as i32, (self.world[3] - self.world[1]) as i32).into());
                geometry = positioner.get_unconstrained_geometry(target);
            }
        }
        surface.with_pending_state(|s| {
            s.geometry = geometry;
            s.positioner = positioner;
        });
        let _ = surface.send_configure();
        self.popups.push(Popup::new(surface));
    }

    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // menus ask for a grab: they take the keyboard and are dismissed by a click elsewhere
        if let Some(p) = self.popups.iter_mut().find(|p| p.surface.wl_surface() == surface.wl_surface()) {
            p.grab = true;
        }
        let kb = self.keyboard.clone();
        kb.set_focus(self, Some(surface.wl_surface().clone().into()), SERIAL_COUNTER.next_serial());
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|s| {
            s.geometry = positioner.get_geometry();
            s.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        self.remove_popup(&surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(i) = self.win_index(surface.wl_surface()) {
            self.make_ghost(i);
            self.wins.remove(i);
            self.grab = None;
            self.apply_focus();
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial) {
        self.begin_client_move(surface.wl_surface());
    }

    fn resize_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial, edges: xdg_toplevel::ResizeEdge) {
        self.begin_client_resize(surface.wl_surface(), edges);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(i) = self.win_index(surface.wl_surface()) {
            if self.wins[i].restore.is_none() {
                self.toggle_maximize(i);
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(i) = self.win_index(surface.wl_surface()) {
            if self.wins[i].restore.is_some() {
                self.toggle_maximize(i);
            }
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<wl_output::WlOutput>) {
        match self.win_index(surface.wl_surface()) {
            Some(i) => self.set_fullscreen(i, true),
            None => {
                // asked before the window is mapped: the first configure carries the state
                surface.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                    s.size = Some((self.size[0] as i32, self.size[1] as i32).into());
                });
                if surface.is_initial_configure_sent() {
                    surface.send_pending_configure();
                }
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(i) = self.win_index(surface.wl_surface()) {
            self.set_fullscreen(i, false);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(i) = self.win_index(surface.wl_surface()) {
            self.minimize(i);
        }
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {}
    fn app_id_changed(&mut self, _surface: ToplevelSurface) {}
}

impl State {
    /// Geometry origin of the popup's parent (a toplevel or another popup), for placing it on screen.
    fn popup_parent_origin(&self, parent: &WlSurface) -> Option<[f32; 2]> {
        self.geometry_origin_pub(parent)
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.set_deco(&toplevel, DecoMode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecoMode) {
        // clients that draw their own chrome (memrium-term, GTK) keep it; everyone else gets ours
        self.set_deco(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.set_deco(&toplevel, DecoMode::ServerSide);
    }
}

impl State {
    fn set_deco(&mut self, toplevel: &ToplevelSurface, mode: DecoMode) {
        let mode = if mode == DecoMode::ClientSide { DecoMode::ClientSide } else { DecoMode::ServerSide };
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if let Some(i) = self.win_index(toplevel.wl_surface()) {
            self.wins[i].deco = if mode == DecoMode::ServerSide { Deco::Server } else { Deco::Client };
            let ins = if mode == DecoMode::ServerSide { self.client_ins(&self.wins[i]) } else { [0.0; 4] };
            self.wins[i].ins = ins;
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = crate::focus::KbFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&crate::focus::KbFocus>) {
        use smithay::wayland::seat::WaylandFocus;
        let client = focused.and_then(|f| f.wl_surface()).and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        use memrium_de::cursor::Shape;
        // a client that names its cursor (`cursor-shape-v1`) gets the shell's drawing of it; one that draws its own
        // (Xwayland, old toolkits) gets that picture, read from its buffer
        self.client_cursor_surface = None;
        self.client_cursor = match image {
            CursorImageStatus::Hidden => Shape::Hidden,
            CursorImageStatus::Named(icon) => Shape::from_name(icon.name()),
            CursorImageStatus::Surface(s) => {
                self.client_cursor_surface = Some(s);
                self.read_client_cursor();
                if self.client_cursor_img.is_some() { Shape::Client } else { Shape::Default }
            }
        };
        self.update_cursor_shape();
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    /// A Wayland program copied: tell X11 programs there is something to paste.
    fn new_selection(&mut self, ty: smithay::wayland::selection::SelectionTarget, source: Option<smithay::wayland::selection::SelectionSource>, _seat: Seat<Self>) {
        let mimes = source.map(|s| s.mime_types());
        if ty == smithay::wayland::selection::SelectionTarget::Clipboard {
            if let Some(m) = &mimes {
                self.clip_copied(m.clone());
            }
        }
        if let Some(xwm) = &mut self.xwm {
            if let Err(e) = xwm.new_selection(ty, mimes) {
                eprintln!("clipboard: {e}");
            }
        }
    }

    /// A Wayland program pastes what an X11 program copied.
    fn send_selection(&mut self, ty: smithay::wayland::selection::SelectionTarget, mime_type: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>, _user_data: &()) {
        // the clipboard is an entry of the history that was chosen: we serve it
        let fd = if ty == smithay::wayland::selection::SelectionTarget::Clipboard {
            match self.clip_serve(&mime_type, fd) {
                Ok(()) => return,
                Err(fd) => fd,
            }
        } else {
            fd
        };
        if let (Some(xwm), Some(handle)) = (&mut self.xwm, self.loop_handle.clone()) {
            if let Err(e) = xwm.send_selection(ty, mime_type, fd, handle) {
                eprintln!("clipboard: {e}");
            }
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}
impl OutputHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);

/// Dock favourites: ~/.config/memrium/dock (one desktop id per line) or a sensible default set.
fn default_pins(apps: &[memrium_de::apps::AppEntry]) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    if let Ok(text) = std::fs::read_to_string(format!("{home}/.config/memrium/dock")) {
        let ids: Vec<String> = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        if !ids.is_empty() {
            return ids;
        }
    }
    let wanted = [
        "memrium-term", "memrium-files", "memrium-settings", "org.kde.dolphin", "org.gnome.Nautilus", "thunar", "firefox", "org.mozilla.firefox", "brave-browser", "google-chrome", "chromium",
        "org.telegram.desktop", "discord", "org.kde.kate", "code", "org.kde.okular", "spotify", "kdesystemsettings", "org.kde.systemsettings",
    ];
    wanted.iter().filter(|id| apps.iter().any(|a| &a.id == *id)).take(8).map(|s| s.to_string()).collect()
}

impl smithay::wayland::dmabuf::DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }

    /// A client created a GPU buffer: import it now, so a bad one is refused before it is ever shown.
    fn dmabuf_imported(&mut self, _global: &smithay::wayland::dmabuf::DmabufGlobal, dmabuf: smithay::backend::allocator::dmabuf::Dmabuf, notifier: smithay::wayland::dmabuf::ImportNotifier) {
        match self.import_dmabuf(&dmabuf) {
            Ok(tex) => {
                if self.dmabuf_logged < 4 {
                    self.dmabuf_logged += 1;
                    use smithay::backend::allocator::Buffer;
                    eprintln!("dmabuf: imported {}x{} {:?}", tex.size.0, tex.size.1, dmabuf.format());
                }
                if let Ok(buf) = notifier.successful::<State>() {
                    self.dmabuf_tex.insert(buf.id(), std::rc::Rc::new(tex));
                }
            }
            Err(e) => {
                eprintln!("dmabuf: {e}");
                notifier.failed();
            }
        }
    }
}
smithay::delegate_dmabuf!(State);
