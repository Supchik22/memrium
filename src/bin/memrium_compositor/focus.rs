//! Who holds the keyboard: a Wayland surface, or an X11 window. An X11 window has to be told through the X server
//! as well (`SetInputFocus` / `WM_TAKE_FOCUS`): Xwayland alone does not give it the X input focus, and programs that
//! read their keys the X11 way (SDL and LWJGL games such as Mindustry, many Steam games) then get no keys at all.

use crate::state::State;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::input::Seat;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Serial};
use smithay::wayland::seat::WaylandFocus;
use smithay::xwayland::X11Surface;
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum KbFocus {
    Wl(WlSurface),
    X11(X11Surface),
}

impl From<WlSurface> for KbFocus {
    fn from(s: WlSurface) -> Self {
        KbFocus::Wl(s)
    }
}

impl IsAlive for KbFocus {
    fn alive(&self) -> bool {
        match self {
            KbFocus::Wl(s) => s.alive(),
            KbFocus::X11(x) => x.alive(),
        }
    }
}

impl WaylandFocus for KbFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KbFocus::Wl(s) => Some(Cow::Borrowed(s)),
            KbFocus::X11(x) => x.wl_surface().map(Cow::Owned),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            KbFocus::Wl(s) => s.same_client_as(object_id),
            KbFocus::X11(x) => x.wl_surface().is_some_and(|s| s.same_client_as(object_id)),
        }
    }
}

impl KeyboardTarget<State> for KbFocus {
    fn enter(&self, seat: &Seat<State>, data: &mut State, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        match self {
            KbFocus::Wl(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            KbFocus::X11(x) => KeyboardTarget::enter(x, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        match self {
            KbFocus::Wl(s) => KeyboardTarget::leave(s, seat, data, serial),
            KbFocus::X11(x) => KeyboardTarget::leave(x, seat, data, serial),
        }
    }

    fn key(&self, seat: &Seat<State>, data: &mut State, key: KeysymHandle<'_>, state: KeyState, serial: Serial, time: u32) {
        match self {
            KbFocus::Wl(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            KbFocus::X11(x) => KeyboardTarget::key(x, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(&self, seat: &Seat<State>, data: &mut State, modifiers: ModifiersState, serial: Serial) {
        match self {
            KbFocus::Wl(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            KbFocus::X11(x) => KeyboardTarget::modifiers(x, seat, data, modifiers, serial),
        }
    }
}
