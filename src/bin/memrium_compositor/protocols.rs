//! Smaller Wayland protocols that programs expect from a desktop: raw (relative) mouse motion and pointer locking
//! for games, cursor shapes, and window activation requests.

use crate::state::State;
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

    /// Raw device motion: sent to the focused client's relative-pointer objects.
    pub fn pointer_relative(&mut self, dx: f64, dy: f64) {
        let ptr = self.pointer.clone();
        let focus = ptr.current_focus().map(|s| (s, Point::<f64, Logical>::from((0.0, 0.0))));
        let utime = self.start.elapsed().as_micros() as u64;
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

    fn cursor_position_hint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>, _location: Point<f64, Logical>) {}
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

    /// A program asks for a window to be brought forward (opening a link from a terminal, "show me" in a chat).
    fn request_activation(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData, surface: WlSurface) {
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
