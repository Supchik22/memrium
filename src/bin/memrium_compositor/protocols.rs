//! Smaller Wayland protocols that programs expect from a desktop: raw (relative) mouse motion and pointer locking
//! for games, cursor shapes, and window activation requests.

use crate::state::State;
use smithay::reexports::wayland_server::Resource;
use smithay::{
    delegate_cursor_shape, delegate_pointer_constraints, delegate_relative_pointer, delegate_xdg_activation,
    input::pointer::{PointerHandle, RelativeMotionEvent},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
    wayland::{
        pointer_constraints::{with_pointer_constraint, PointerConstraint, PointerConstraintsHandler, PointerConstraintsState},
        relative_pointer::RelativePointerManagerState,
        cursor_shape::CursorShapeManagerState,
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};

pub struct Protocols {
    pub relative_pointer: RelativePointerManagerState,
    pub constraints: PointerConstraintsState,
    pub cursor_shape: CursorShapeManagerState,
    pub activation: XdgActivationState,
    pub fractional_scale: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub viewporter: smithay::wayland::viewporter::ViewporterState,
    /// the surface whose pointer constraint is currently switched on
    pub constraint_focus: Option<WlSurface>,
    /// raw mouse motion gathered since it was last handed on (`flush_relative`)
    pub rel_pending: (f64, f64),
    /// where the programs that said (`org_kde_kwin_appmenu`) keep their menus
    pub appmenu: Vec<crate::appmenu_wayland::MenuAddress>,
    /// where GTK programs (`gtk_shell1`) keep theirs
    pub gtk_menus: Vec<crate::gtk_shell::GtkAddress>,
}

impl Protocols {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> Protocols {
        crate::appmenu_wayland::create_global(dh);
        crate::gtk_shell::create_global(dh);
        crate::blur::create_global(dh);
        Protocols {
            relative_pointer: RelativePointerManagerState::new::<State>(dh),
            constraints: PointerConstraintsState::new::<State>(dh),
            cursor_shape: CursorShapeManagerState::new::<State>(dh),
            activation: XdgActivationState::new::<State>(dh),
            fractional_scale: smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<State>(dh),
            viewporter: smithay::wayland::viewporter::ViewporterState::new::<State>(dh),
            constraint_focus: None,
            rel_pending: (0.0, 0.0),
            appmenu: Vec::new(),
            gtk_menus: Vec::new(),
        }
    }
}

impl State {
    /// Is the pointer locked in place by the focused client (a game looking around)?
    pub fn pointer_locked(&self) -> bool {
        let ptr = self.pointer.clone();
        let Some(focus) = ptr.current_focus() else { return false };
        with_pointer_constraint(&focus, &ptr, |c| c.is_some_and(|c| c.is_active() && matches!(&*c, PointerConstraint::Locked(_))))
    }

    /// Raw device motion (logical px): gathered, and handed on by `flush_relative` once the input that came together is read.
    pub fn pointer_relative(&mut self, dx: f64, dy: f64) {
        self.protocols.rel_pending.0 += dx;
        self.protocols.rel_pending.1 += dy;
    }

    /// Send the gathered raw motion to the focused client's relative-pointer objects, as one event. A mouse at 1000 Hz
    /// brings several events per pass of the loop; sent one by one in a burst, Xwayland gives them all the same time and
    /// SDL takes the equal ones for duplicates (the view of CS2 drifts and swings back). An X11 window counts in device
    /// pixels, so its motion is told in those (as KWin does).
    pub fn flush_relative(&mut self) {
        let (mut dx, mut dy) = std::mem::take(&mut self.protocols.rel_pending);
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let ptr = self.pointer.clone();
        if ptr.current_focus().is_some_and(|s| self.surfaces.get(&s.id()).is_some_and(|n| n.x11)) {
            let xs = crate::role::x_scale() as f64;
            dx *= xs;
            dy *= xs;
        }
        let focus = ptr.current_focus().map(|s| (s, Point::<f64, Logical>::from((0.0, 0.0))));
        let utime = self.start.elapsed().as_micros() as u64;
        if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
            self.dbg_rel = self.dbg_rel.wrapping_add(1);
            // every change of the lock, and the first few motions after it (are they not zero?)
            let locked = self.pointer_locked();
            if locked != self.dbg_lock.0 {
                eprintln!("pointer: lock {} -> {} (after {} raw motions, focus {:?})", self.dbg_lock.0, locked, self.dbg_lock.1, ptr.current_focus().map(|s| smithay::reexports::wayland_server::Resource::id(&s)));
                self.dbg_lock = (locked, 0);
            }
            self.dbg_lock.1 += 1;
            if locked && self.dbg_lock.1 <= 3 {
                eprintln!("pointer: locked raw motion dx {dx:.2} dy {dy:.2}");
            }
            if self.dbg_rel % 200 == 1 {
                eprintln!("pointer: raw motion #{} to focus {:?}, locked {}", self.dbg_rel, ptr.current_focus().map(|s| smithay::reexports::wayland_server::Resource::id(&s)), self.pointer_locked());
            }
        }
        let delta = Point::<f64, Logical>::from((dx, dy));
        ptr.relative_motion(self, focus, &RelativeMotionEvent { delta, delta_unaccel: delta, utime });
        ptr.frame(self);
    }

    /// Switch pointer constraints on for the surface that has the pointer now, and off for the one it left.
    pub fn update_pointer_constraints(&mut self) {
        let ptr = self.pointer.clone();
        let focus = ptr.current_focus();
        if self.protocols.constraint_focus == focus {
            return;
        }
        if let Some(old) = self.protocols.constraint_focus.take() {
            if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
                eprintln!("pointer: constraint released, the pointer left {:?} for {:?}", smithay::reexports::wayland_server::Resource::id(&old), focus.as_ref().map(smithay::reexports::wayland_server::Resource::id));
            }
            with_pointer_constraint(&old, &ptr, |c| {
                if let Some(c) = c {
                    c.deactivate();
                }
            });
        }
        if let Some(new) = &focus {
            with_pointer_constraint(new, &ptr, |c| {
                if let Some(c) = c.filter(|c| !c.is_active()) {
                    c.activate();
                }
            });
        }
        self.protocols.constraint_focus = focus;
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
            let kind = with_pointer_constraint(surface, pointer, |c| c.map(|c| if matches!(&*c, PointerConstraint::Locked(_)) { "locked" } else { "confined" }));
            eprintln!("pointer: {kind:?} asked by a surface; it has the pointer now: {}", pointer.current_focus().as_ref() == Some(surface));
        }
        // a constraint asked for by the surface that already has the pointer takes effect at once
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
            self.protocols.constraint_focus = Some(surface.clone());
        }
    }

    /// A program that holds the pointer says where it wants it to be when released (SDL on X11 warps it to the middle of the
    /// window all the time; Xwayland turns that into a lock plus this hint). Without it the pointer stays where it was, the
    /// program thinks it is in the middle, and the view jumps back after each move.
    fn cursor_position_hint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>, location: Point<f64, Logical>) {
        let locked = with_pointer_constraint(surface, pointer, |c| c.is_some_and(|c| c.is_active() && matches!(&*c, PointerConstraint::Locked(_))));
        if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
            eprintln!("pointer: position hint ({:.1}, {:.1}) locked {locked}", location.x, location.y);
        }
        if !locked {
            return;
        }
        let Some((under, origin)) = self.surface_under(self.mouse).filter(|(s, _)| s == surface) else { return };
        // an X11 window counts in device pixels
        let xs = if self.surfaces.get(&surface.id()).is_some_and(|n| n.x11) { crate::role::x_scale() as f64 } else { 1.0 };
        let _ = under;
        self.warp_to = Some([origin[0] + (location.x / xs) as f32, origin[1] + (location.y / xs) as f32]);
    }
}
impl smithay::wayland::tablet_manager::TabletSeatHandler for State {}
delegate_pointer_constraints!(State);
delegate_relative_pointer!(State);
delegate_cursor_shape!(State);

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.protocols.activation
    }

    fn token_created(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData) -> bool {
        true
    }

    /// A program asks for a window to be brought forward (opening a link from a terminal, "show me" in a chat). It is done only
    /// when the token came from the window that has the focus now: that is the user's own doing. A program in the background
    /// (a messenger with a new message) does not get to take the focus; it is left where it is.
    fn request_activation(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData, surface: WlSurface) {
        let focused = self.wins.last().and_then(|w| w.role.surface());
        let from_focused = match (&data.surface, &focused) {
            (Some(asked_by), Some(f)) => crate::state::root_of(asked_by) == *f,
            _ => false,
        };
        if !from_focused {
            if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
                eprintln!("activation: refused for a window in the background (the token was not from the focused window)");
            }
            return;
        }
        if let Some(i) = self.win_index(&surface) {
            if !self.wins[i].or {
                self.focus_window(i);
            }
        }
    }
}
delegate_xdg_activation!(State);

/// Programs that can draw for a fractional scale (GTK 4, Qt 6, Firefox ...) are told the exact one, and then make buffers of
/// that many pixels per logical pixel and say with a viewport how big the surface is: no scaling by the shell, so no blur.
impl smithay::wayland::fractional_scale::FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.send_preferred_scale(&surface);
    }
}
smithay::delegate_fractional_scale!(State);
smithay::delegate_viewporter!(State);

impl State {
    /// Tell a surface the scale of the desktop.
    pub fn send_preferred_scale(&self, surface: &WlSurface) {
        let scale = self.renderer.scale() as f64;
        smithay::wayland::compositor::with_states(surface, |st| {
            smithay::wayland::fractional_scale::with_fractional_scale(st, |fs| fs.set_preferred_scale(scale));
        });
    }

    /// The scale changed (a monitor was set up again): tell every program.
    pub fn send_preferred_scale_to_all(&self) {
        let mut surfaces: Vec<WlSurface> = self.wins.iter().filter_map(|w| w.role.surface()).collect();
        surfaces.extend(self.popups.iter().map(|p| p.surface.wl_surface().clone()));
        for s in surfaces {
            self.send_preferred_scale(&s);
        }
    }
}
