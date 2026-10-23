//! Inactivity: lock the screen, switch the display off, suspend; and the idle-inhibit protocol that lets a video
//! player keep the screen awake.

use crate::state::State;
use memrium_de::idle::Step;
use smithay::{
    delegate_idle_inhibit,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    wayland::idle_inhibit::IdleInhibitHandler,
};
use std::time::Instant;

impl State {
    /// Any input: restart the inactivity clock and wake the display.
    pub fn activity(&mut self) {
        // test hook: pretend nobody touches the machine, whatever the window system reports
        if std::env::var_os("MEMRIUM_TEST_NO_ACTIVITY").is_some() {
            return;
        }
        self.last_input = Instant::now();
        self.idle_done = [false; 3];
        if self.screen_off {
            self.screen_off = false;
            self.invalidate();
        }
    }

    fn inhibited(&mut self) -> bool {
        self.inhibitors.retain(|s| s.is_alive());
        self.manual_block || !self.inhibitors.is_empty()
    }

    /// Called every tick: carry out the step that has become due.
    pub fn update_idle(&mut self) {
        if self.inhibited() {
            self.last_input = Instant::now();
            return;
        }
        let secs = self.last_input.elapsed().as_secs_f32();
        let Some(step) = self.config.power.due(secs, self.idle_done) else { return };
        match step {
            Step::Lock => {
                self.idle_done[0] = true;
                if self.lock.is_none() {
                    self.lock_screen();
                }
            }
            Step::ScreenOff => {
                self.idle_done[0] = true;
                self.idle_done[1] = true;
                if self.lock.is_none() {
                    // switching the display off without a lock behind it would show everything on wake-up
                    self.lock_screen();
                }
                self.screen_off = true;
                self.invalidate();
            }
            Step::Suspend => {
                self.idle_done = [true; 3];
                self.power("suspend");
            }
        }
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        if !self.inhibitors.contains(&surface) {
            self.inhibitors.push(surface);
        }
    }
    fn uninhibit(&mut self, surface: WlSurface) {
        self.inhibitors.retain(|s| s != &surface);
    }
}

delegate_idle_inhibit!(State);
