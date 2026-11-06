//! The nested window takes the keyboard's shortcuts (`zwp_keyboard_shortcuts_inhibit_v1`), as a virtual machine does:
//! while it has the focus the desktop around it (KWin) leaves Super, Super+D, Super+V, Alt+Tab... to it, so the shell
//! inside can be tried with its own keys. Ctrl+Alt+G gives them back (and takes them again).
//!
//! winit does not offer the protocol, so a second connection object is made over winit's own `wl_display`, and the
//! inhibitor is asked for winit's `wl_surface`.

use std::ffi::c_void;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1};

pub struct KeyGrab {
    queue: EventQueue<Seen>,
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    surface: WlSurface,
    seat: WlSeat,
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
    /// the user wants the shortcuts taken (Ctrl+Alt+G); `hold` lets them go for a moment without changing this
    pub wanted: bool,
}

/// What the host said about the grab.
#[derive(Default)]
struct Seen {
    active: Option<bool>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Seen {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
wayland_client::delegate_noop!(Seen: ignore WlSeat);
wayland_client::delegate_noop!(Seen: ZwpKeyboardShortcutsInhibitManagerV1);

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for Seen {
    fn event(s: &mut Self, _: &ZwpKeyboardShortcutsInhibitorV1, e: zwp_keyboard_shortcuts_inhibitor_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => s.active = Some(true),
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => s.active = Some(false),
            _ => {}
        }
    }
}

impl KeyGrab {
    /// Over winit's display (`display`) for its window's surface (`surface`), both raw pointers of the window.
    pub fn new(display: *mut c_void, surface: *mut c_void) -> Result<KeyGrab, String> {
        // SAFETY: the pointers are winit's live wl_display and wl_surface; the grab is dropped before the window
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Seen>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let manager: ZwpKeyboardShortcutsInhibitManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| "the desktop does not offer keyboard-shortcuts-inhibit".to_string())?;
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        // SAFETY: a live wl_surface of this display
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface as *mut _) }.map_err(|e| e.to_string())?;
        let surface = WlSurface::from_id(&conn, id).map_err(|e| e.to_string())?;
        let mut g = KeyGrab { queue, manager, surface, seat, inhibitor: None, wanted: true };
        g.set(true);
        Ok(g)
    }

    pub fn on(&self) -> bool {
        self.inhibitor.is_some()
    }

    /// Ctrl+Alt held (true): the shortcuts go back to the desktop around until they are let go.
    pub fn hold(&mut self, ctrl_alt: bool) {
        if self.wanted {
            self.set_quiet(!ctrl_alt);
        }
    }

    fn set_quiet(&mut self, on: bool) {
        if on == self.on() {
            return;
        }
        let qh = self.queue.handle();
        if on {
            self.inhibitor = Some(self.manager.inhibit_shortcuts(&self.surface, &self.seat, &qh, ()));
        } else if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
        let _ = self.queue.roundtrip(&mut Seen::default());
    }

    /// Take the shortcuts (true) or give them back.
    pub fn set(&mut self, on: bool) {
        if on == self.on() {
            return;
        }
        let qh = self.queue.handle();
        if on {
            self.inhibitor = Some(self.manager.inhibit_shortcuts(&self.surface, &self.seat, &qh, ()));
        } else if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
        let mut seen = Seen::default();
        let _ = self.queue.roundtrip(&mut seen);
        eprintln!("keyboard: shortcuts {} the desktop around ({})", if on { "taken from" } else { "given back to" }, match seen.active {
            Some(true) => "it agreed",
            Some(false) => "it said no for now",
            None => "no answer yet",
        });
    }
}
