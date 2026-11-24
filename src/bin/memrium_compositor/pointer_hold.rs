//! What a program knows of the pointer: the buttons it was told are down, and the surface and origin its positions are
//! counted from. Both have to follow when the shell takes over or the window moves.
//!
//! * A press that reached a program is always followed by its release, also when the shell uses that release itself (the end
//!   of a move the program asked for). Without it smithay's click grab stays, with the window's origin from the press: every
//!   position the program got after a drag by its own header was off by the distance of the drag, until the next click.
//! * A program that hands the pointer over (an interactive move or resize) gets a leave, as with other compositors.
//! * What is under a still pointer changes (a window glides, is maximized, closes, comes to the front, changes its shadow
//!   margins): the program is told at once, or the next click would land where the window was.

use crate::state::State;
use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

#[derive(Default)]
pub struct Hold {
    /// the buttons a program was sent a press of and no release yet
    down: Vec<u32>,
    /// the focus the pointer was last given
    sent: Option<(WlSurface, [f32; 2])>,
    /// a grab was in the way since then: the focus has to be given again
    stale: bool,
}

/// Is `now` another place for a program to count from than `was`?
fn moved<T: PartialEq>(was: &Option<(T, [f32; 2])>, now: &Option<(T, [f32; 2])>) -> bool {
    match (was, now) {
        (None, None) => false,
        (Some((a, o)), Some((b, p))) => a != b || (o[0] - p[0]).abs() > 0.25 || (o[1] - p[1]).abs() > 0.25,
        _ => true,
    }
}

impl State {
    /// Hand a button on to the program that has the pointer.
    pub fn send_button(&mut self, button: u32, pressed: bool) {
        use smithay::backend::input::ButtonState;
        self.hold.down.retain(|b| *b != button);
        if pressed {
            self.hold.down.push(button);
        }
        let ptr = self.pointer.clone();
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        let time = self.time_ms();
        ptr.button(self, &ButtonEvent { button, state, serial: SERIAL_COUNTER.next_serial(), time });
        ptr.frame(self);
    }

    /// The shell is done with a release: if the press went to a program, the release does too.
    pub fn settle_release(&mut self, button: u32) {
        if self.hold.down.contains(&button) {
            self.send_button(button, false);
            self.hold.stale = true;
        }
    }

    /// The program asked for an interactive move or resize: the pointer is the shell's until the button is up.
    pub fn take_pointer_from_client(&mut self) {
        let ptr = self.pointer.clone();
        let (serial, time) = (SERIAL_COUNTER.next_serial(), self.time_ms());
        ptr.unset_grab(self, serial, time);
        let location = Point::<f64, Logical>::from((self.mouse[0] as f64, self.mouse[1] as f64));
        ptr.motion(self, None, &MotionEvent { location, serial, time });
        ptr.frame(self);
        self.hold.sent = None;
    }

    /// The pointer moved and was given this focus.
    pub fn note_pointer_focus(&mut self, focus: Option<(WlSurface, [f32; 2])>) {
        self.hold.stale = self.pointer.is_grabbed();
        self.hold.sent = focus;
    }

    /// Once a frame: has what is under the still pointer changed?
    pub fn repick_pointer(&mut self) {
        if self.pointer.is_grabbed() {
            self.hold.stale = true;
            return;
        }
        // (these take the pointer before any program gets it)
        if self.lock.is_some() || self.shot_sel.is_some() || self.shot_edit.is_some() || self.live_shot.is_some() || self.ov_active() || self.grab.is_some() || self.pointer_locked() {
            return;
        }
        let now = self.client_focus(self.mouse);
        if self.hold.stale || moved(&self.hold.sent, &now) {
            self.hold.stale = false;
            self.pointer_motion(self.mouse[0], self.mouse[1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::moved;

    #[test]
    fn a_moved_window_or_another_surface_is_picked_again() {
        let at = |s: u8, x: f32| Some((s, [x, 80.0]));
        assert!(!moved::<u8>(&None, &None));
        assert!(!moved(&at(1, 200.0), &at(1, 200.1)), "sub-pixel noise is no move");
        assert!(moved(&at(1, 200.0), &at(1, 400.0)), "the window moved");
        assert!(moved(&at(1, 200.0), &at(2, 200.0)), "another surface");
        assert!(moved(&at(1, 200.0), &None), "the window went away");
    }
}
