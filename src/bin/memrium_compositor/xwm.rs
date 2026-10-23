//! X11 applications: XWayland plus a small window manager on top of smithay's `X11Wm`.
//!
//! Managed X11 windows become ordinary entries of `State::wins` (framed, focusable, animated like any other window).
//! Override-redirect windows (menus, tooltips, drop-downs) are placed by the client itself, drawn unframed above
//! everything and never take the focus.

use crate::role::Role;
use crate::state::{Deco, State, Win};
use chebrium::spring::Spring;
use smithay::{
    delegate_xwayland_shell,
    reexports::{calloop::LoopHandle, wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as XdgEdge},
    utils::{Logical, Point, Rectangle},
    wayland::{
        selection::{
            data_device::{clear_data_device_selection, current_data_device_selection_userdata, request_data_device_client_selection, set_data_device_selection},
            SelectionTarget,
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};
use std::process::Stdio;

impl State {
    /// Start XWayland; the window manager comes up when it reports ready. Failure only means no X11 support.
    pub fn start_xwayland(&mut self, handle: LoopHandle<'static, State>) {
        if std::env::var_os("MEMRIUM_NO_XWAYLAND").is_some() {
            return;
        }
        let spawned = XWayland::spawn(&self.dh, None, std::iter::empty::<(String, String)>(), true, Stdio::null(), Stdio::null(), |_| {});
        let (xwayland, client) = match spawned {
            Ok(v) => v,
            Err(e) => {
                eprintln!("xwayland: could not start: {e}");
                return;
            }
        };
        let wm_handle = handle.clone();
        let inserted = handle.insert_source(xwayland, move |event, _, state: &mut State| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => match X11Wm::start_wm(wm_handle.clone(), x11_socket, client.clone()) {
                Ok(wm) => {
                    state.xwm = Some(wm);
                    state.loop_handle = Some(wm_handle.clone());
                    state.x_display = Some(display_number);
                    // apps started from now on find the X server
                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                    eprintln!("xwayland: ready on DISPLAY=:{display_number}");
                    state.export_session_env();
                    state.set_x_dpi();
                }
                Err(e) => eprintln!("xwayland: window manager failed: {e}"),
            },
            XWaylandEvent::Error => eprintln!("xwayland: the X server failed to start"),
        });
        if let Err(e) = inserted {
            eprintln!("xwayland: could not watch the X server: {e}");
        }
    }

    /// The user manager's environment as `NAME=value` pairs (what D-Bus activated programs start with).
    fn user_environment() -> Vec<(String, String)> {
        let out = std::process::Command::new("systemctl").args(["--user", "show-environment"]).output().ok();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// Is another desktop (KDE, GNOME ...) running its session next to this one? Its Wayland socket answers.
    fn other_session_alive(env: &[(String, String)], ours: &str) -> bool {
        let Some((_, name)) = env.iter().find(|(k, _)| k == "WAYLAND_DISPLAY") else { return false };
        if name == ours {
            return false;
        }
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
        std::os::unix::net::UnixStream::connect(std::path::Path::new(&dir).join(name)).is_ok()
    }

    /// Tell D-Bus activated services (the portals) where this session's displays are. Only done for a real
    /// session, and only when no other desktop is running: its programs (Spectacle, the portals ...) share this
    /// environment and must not be pointed at our displays. What was there is kept and put back when we end.
    pub fn export_session_env(&mut self) {
        if !self.export_env {
            return;
        }
        let before = Self::user_environment();
        if Self::other_session_alive(&before, &self.socket) {
            eprintln!("session: another desktop is running; leaving the shared environment (portals, screen sharing) to it");
            return;
        }
        let mut vars = vec![format!("WAYLAND_DISPLAY={}", self.socket), "XDG_CURRENT_DESKTOP=Memrium".to_string(), "XDG_SESSION_TYPE=wayland".to_string()];
        if let Some(n) = self.x_display {
            vars.push(format!("DISPLAY=:{n}"));
        }
        if self.saved_env.is_empty() {
            for name in ["WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "DISPLAY"] {
                self.saved_env.push((name.to_string(), before.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())));
            }
        }
        match std::process::Command::new("dbus-update-activation-environment").arg("--systemd").args(&vars).status() {
            Ok(s) if s.success() => eprintln!("session: portals and services now see {}", vars.join(" ")),
            Ok(s) => eprintln!("session: dbus-update-activation-environment failed ({s})"),
            Err(e) => eprintln!("session: dbus-update-activation-environment is not available ({e}); screen sharing may not find this session"),
        }
    }

    /// Put the shared environment back as it was before this session changed it.
    pub fn restore_session_env(&mut self) {
        let saved = std::mem::take(&mut self.saved_env);
        if saved.is_empty() {
            return;
        }
        let set: Vec<String> = saved.iter().filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}"))).collect();
        if !set.is_empty() {
            let _ = std::process::Command::new("dbus-update-activation-environment").arg("--systemd").args(&set).status();
        }
        let unset: Vec<&str> = saved.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k.as_str()).collect();
        if !unset.is_empty() {
            let _ = std::process::Command::new("systemctl").args(["--user", "unset-environment"]).args(&unset).status();
        }
        eprintln!("session: the shared environment is back as it was");
    }

    fn x11_index(&self, window: &X11Surface) -> Option<usize> {
        self.wins.iter().position(|w| w.role.x11() == Some(window))
    }

    /// A fresh window entry for `role` at `(x, y)`.
    pub fn new_win(&mut self, role: Role, x: f32, y: f32) -> Win {
        self.next_id += 1;
        let stamp = self.next_stamp();
        let mut win = Win {
            id: self.next_id,
            role,
            or: false,
            x11_loc: None,
            deco: Deco::Legacy,
            app_id: String::new(),
            title: String::new(),
            x: Spring::new(x, 700.0, 0.8),
            y: Spring::new(y, 700.0, 0.8),
            scale: Spring::new(1.0, 320.0, 0.45),
            min_k: Spring::new(0.0, 260.0, 0.7),
            open_k: Spring::new(0.0, 240.0, 0.74),
            minimized: false,
            ws: self.workspace,
            ws_hidden: false,
            restore: None,
            tile: 0,
            fs: None,
            jel: [crate::state::jelly_spring(), crate::state::jelly_spring()],
            jprev: [x, y],
            jgrab: 0.5,
            geo: [0.0, 0.0, 0.0, 0.0],
            ins: [0.0; 4],
            mapped: false,
            content_dark: None,
            stamp,
        };
        win.x.target = x;
        win.y.target = y;
        win
    }

    fn x11_remove(&mut self, window: &X11Surface) {
        let Some(i) = self.x11_index(window) else { return };
        if !self.wins[i].or {
            self.make_ghost(i);
        }
        self.wins.remove(i);
        self.grab = None;
        self.apply_focus();
    }

    /// Keep each managed X11 window's client area where the client thinks it is on screen.
    /// Tell X programs how big the desktop's pixels are: `Xft.dpi` on the root window (what GTK and Qt programs on X read), 96 at
    /// 100%. The X windows have the device's own pixels, so their text and controls must grow by the scale themselves.
    pub fn set_x_dpi(&mut self) {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{AtomEnum, PropMode};
        use x11rb::wrapper::ConnectionExt as _;
        let Some(n) = self.x_display else { return };
        let dpi = (96.0 * crate::role::x_scale()).round() as u32;
        let Ok((conn, screen)) = x11rb::rust_connection::RustConnection::connect(Some(&format!(":{n}"))) else { return };
        let root = conn.setup().roots[screen].root;
        let text = format!("Xft.dpi:\t{dpi}\nXft.antialias:\t1\nXft.hinting:\t1\nXft.hintstyle:\thintslight\nXft.rgba:\tnone\n");
        let _ = conn.change_property8(PropMode::REPLACE, root, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, text.as_bytes());
        let _ = conn.flush();
    }

    pub fn sync_x11_positions(&mut self) {
        for w in &mut self.wins {
            let Role::X11(x) = &w.role else { continue };
            if w.or || !w.mapped {
                continue;
            }
            let loc = (crate::role::to_x((w.x.x + w.ins[0]).round() as i32), crate::role::to_x((w.y.x + w.ins[1]).round() as i32));
            if w.x11_loc != Some(loc) {
                w.x11_loc = Some(loc);
                let _ = x.configure(Rectangle::new(Point::from(loc), x.geometry().size));
            }
        }
    }
}

fn xdg_edge(e: ResizeEdge) -> XdgEdge {
    match e {
        ResizeEdge::Top => XdgEdge::Top,
        ResizeEdge::Bottom => XdgEdge::Bottom,
        ResizeEdge::Left => XdgEdge::Left,
        ResizeEdge::Right => XdgEdge::Right,
        ResizeEdge::TopLeft => XdgEdge::TopLeft,
        ResizeEdge::TopRight => XdgEdge::TopRight,
        ResizeEdge::BottomLeft => XdgEdge::BottomLeft,
        ResizeEdge::BottomRight => XdgEdge::BottomRight,
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// Xwayland links an X11 window to its surface only after the surface's first commits: what those commits would
    /// have done for the window (showing it, giving it the keyboard) is done now, or a program that draws once and
    /// then waits (most dialogs) would stay hidden and without keys.
    fn surface_associated(&mut self, _xwm: XwmId, wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, _window: X11Surface) {
        self.on_commit(&wl_surface);
    }
}
delegate_xwayland_shell!(State);

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("the window manager is running")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_mapped(true);
        if self.x11_index(&window).is_some() {
            return;
        }
        let n = (self.wins.iter().filter(|w| !w.or).count() % 6) as f32;
        let (x, y) = (200.0 + 44.0 * n, 80.0 + 40.0 * n);
        let mut win = self.new_win(Role::X11(window.clone()), x, y);
        // a window that draws its own decorations (motif hints) gets no frame from us
        win.deco = if window.is_decorated() { Deco::Client } else { Deco::Server };
        win.ins = if win.deco == Deco::Server { memrium_de::decor::insets(&self.theme) } else { [0.0; 4] };
        win.app_id = window.class();
        win.title = window.title();
        let loc = (crate::role::to_x((x + win.ins[0]) as i32), crate::role::to_x((y + win.ins[1]) as i32));
        win.x11_loc = Some(loc);
        let at = self.wins.iter().rposition(|o| !o.or).map(|p| p + 1).unwrap_or(0);
        self.wins.insert(at, win);
        let _ = window.configure(Rectangle::new(Point::from(loc), window.geometry().size));
        self.apply_focus();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if self.x11_index(&window).is_some() {
            return;
        }
        let geo = window.geometry();
        let mut win = self.new_win(Role::X11(window), crate::role::from_x(geo.loc.x), crate::role::from_x(geo.loc.y));
        win.or = true;
        // menus and tooltips appear at once
        win.open_k = Spring::new(1.0, 240.0, 0.74);
        win.x11_loc = Some((geo.loc.x, geo.loc.y));
        self.wins.push(win);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_remove(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_remove(&window);
    }

    fn configure_request(&mut self, _xwm: XwmId, window: X11Surface, x: Option<i32>, y: Option<i32>, w: Option<u32>, h: Option<u32>, _reorder: Option<Reorder>) {
        let mut geo: Rectangle<i32, Logical> = window.geometry();
        let managed = self.x11_index(&window).filter(|&i| !self.wins[i].or);
        match managed {
            // we place managed windows ourselves; only the size request is honoured
            Some(i) => {
                if let Some(loc) = self.wins[i].x11_loc {
                    geo.loc = Point::from(loc);
                }
            }
            None => {
                if let Some(x) = x {
                    geo.loc.x = x;
                }
                if let Some(y) = y {
                    geo.loc.y = y;
                }
            }
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(&mut self, _xwm: XwmId, window: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        // override-redirect windows move themselves: follow them
        if let Some(i) = self.x11_index(&window).filter(|&i| self.wins[i].or) {
            let w = &mut self.wins[i];
            w.x.x = crate::role::from_x(geometry.loc.x);
            w.x.target = w.x.x;
            w.y.x = crate::role::from_x(geometry.loc.y);
            w.y.target = w.y.x;
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window).filter(|&i| self.wins[i].restore.is_none()) {
            self.toggle_maximize(i);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window).filter(|&i| self.wins[i].restore.is_some()) {
            self.toggle_maximize(i);
        }
    }

    fn fullscreen_request(&mut self, xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window) {
            self.set_fullscreen(i, true);
        }
    }

    fn unfullscreen_request(&mut self, xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window) {
            self.set_fullscreen(i, false);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window) {
            self.minimize(i);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(i) = self.x11_index(&window) {
            self.focus_window(i);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edge: ResizeEdge) {
        if let Some(root) = window.wl_surface() {
            self.begin_client_resize(&root, xdg_edge(edge));
        }
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        if let Some(root) = window.wl_surface() {
            self.begin_client_move(&root);
        }
    }

    // ---- clipboard: X11 <-> Wayland (the copy-paste selection; the primary one is not offered by this shell)

    fn allow_selection_access(&mut self, _xwm: XwmId, selection: SelectionTarget) -> bool {
        selection == SelectionTarget::Clipboard
    }

    /// An X11 program wants to read what a Wayland program copied.
    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: std::os::fd::OwnedFd) {
        if selection == SelectionTarget::Clipboard {
            if let Err(e) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                eprintln!("clipboard: {e:?}");
            }
        }
    }

    /// An X11 program copied something: offer it to Wayland programs.
    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        if selection == SelectionTarget::Clipboard {
            set_data_device_selection(&self.dh, &self.seat, mime_types, ());
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        // only when the clipboard is still the X11 one (it carries our marker); a Wayland owner's is left alone
        if selection == SelectionTarget::Clipboard && current_data_device_selection_userdata(&self.seat).is_some() {
            clear_data_device_selection(&self.dh, &self.seat);
        }
    }
}
