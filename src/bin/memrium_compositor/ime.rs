//! Text input for other writing systems: the `text-input` / `input-method` protocols that input-method programs
//! (fcitx5, ibus) talk to, and `virtual-keyboard` for on-screen keyboards and tools like `wtype`.
//!
//! The input method's candidate window (a popup surface) is not drawn yet: composing text and committing it work,
//! the list of candidates does not show.

use crate::state::State;
use smithay::{
    delegate_input_method_manager, delegate_text_input_manager, delegate_virtual_keyboard_manager,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::input_method::{InputMethodHandler, PopupSurface},
};

impl InputMethodHandler for State {
    fn new_popup(&mut self, _surface: PopupSurface) {}
    fn dismiss_popup(&mut self, _surface: PopupSurface) {}
    fn popup_repositioned(&mut self, _surface: PopupSurface) {}
    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.wins
            .iter()
            .find(|w| w.role.is_surface(parent))
            .map(|w| Rectangle::new((w.geo[0] as i32, w.geo[1] as i32).into(), (w.geo[2] as i32, w.geo[3] as i32).into()))
            .unwrap_or_default()
    }
}

delegate_text_input_manager!(State);
delegate_input_method_manager!(State);
delegate_virtual_keyboard_manager!(State);
