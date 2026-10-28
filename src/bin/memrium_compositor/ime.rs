//! Text input for other writing systems: the `text-input` / `input-method` protocols that input-method programs
//! (fcitx5, ibus) talk to, and `virtual-keyboard` for on-screen keyboards and tools like `wtype`.
//!
//! The input method's candidate window (a popup surface) is drawn under the text cursor of the program being typed in,
//! kept on screen, and is gone with the input method (see `State::ime_popup_origins`).

use crate::state::State;
use smithay::reexports::wayland_server::Resource;
use smithay::{
    delegate_input_method_manager, delegate_text_input_manager, delegate_virtual_keyboard_manager,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::input_method::{InputMethodHandler, PopupSurface},
};

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        self.ime_popups.push(surface);
    }
    fn dismiss_popup(&mut self, surface: PopupSurface) {
        self.ime_popups.retain(|p| p != &surface);
    }
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

impl State {
    /// Where each candidate window goes: (its surface, top left in the world). Below the cursor of the text field, or above it
    /// when there is no room, and never off the screen.
    pub fn ime_popup_origins(&mut self) -> Vec<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, [f32; 2])> {
        self.ime_popups.retain(|p| p.alive());
        let mut out = Vec::new();
        for p in &self.ime_popups {
            let Some(parent) = p.get_parent() else { continue };
            let Some(w) = self.wins.iter().find(|w| w.role.is_surface(&parent.surface)) else { continue };
            let cursor = p.text_input_rectangle();
            let size = crate::state::surface_tree(p.wl_surface()).first().and_then(|(s, _)| self.surfaces.get(&s.id()).and_then(|n| n.logical_size())).unwrap_or([0.0, 0.0]);
            let (bx, by) = (w.x.x + w.ins[0], w.y.x + w.ins[1]);
            let x = bx + cursor.loc.x as f32;
            let mut y = by + (cursor.loc.y + cursor.size.h) as f32;
            if y + size[1] > self.world[3] {
                y = by + cursor.loc.y as f32 - size[1];
            }
            let x = x.min(self.world[2] - size[0]).max(self.world[0]);
            out.push((p.wl_surface().clone(), [x, y.max(self.world[1])]));
        }
        out
    }
}
