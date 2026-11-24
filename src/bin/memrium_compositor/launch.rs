//! The launch animation on the desktop: where the icon flies from, where to, and who shows it after it lands (the maths
//! and the look are `memrium_de::launch_flight`).

use crate::state::State;
use memrium_de::cursor::Shape;
use memrium_de::launch_flight::{pick, Flight, Source};
use memrium_de::renderer::{Item, Panel};
use std::time::Instant;

/// More copies in the air than this: the oldest goes.
const MOST: usize = 4;

#[derive(Default)]
pub struct Launch {
    /// the icons of the last picture that was put together, in paint order: what a click was made on
    seen: Vec<Source>,
    flights: Vec<Flight>,
    /// the newest launch's icon is still on its way: the cursor does not show one yet
    in_air: bool,
    landed: Option<Instant>,
    /// what the busy cursor covers over a turn, which its icon stands by
    pub rest: Option<memrium_de::cursor::CursorImage>,
    /// the launch being handled came from the keyboard, not from a click
    pub typed: bool,
    /// the hardware cursor's largest picture; None: the backend's cursor shows no launch icon (nested, headless)
    pub plane: Option<u32>,
}

impl State {
    /// Remember the application and file icons of this picture: the desktop's, then the shell's over them.
    pub fn note_icons(&mut self, shell: &[Panel], desktop: Option<&Item>) {
        self.launch.seen.clear();
        if let Some(Item::Layer { panels, .. }) = desktop {
            self.launch.seen.extend(panels.iter().filter_map(Source::of));
        }
        self.launch.seen.extend(shell.iter().filter_map(Source::of));
    }

    /// The icons of the last picture, as a flight picks its start from.
    pub fn launch_seen(&self) -> &[Source] {
        &self.launch.seen
    }

    /// A program was just started: send a copy of the icon it was started from to the pointer.
    pub fn launch_fly(&mut self, id: &str, icon: &str) {
        self.launch.in_air = false;
        self.launch.landed = None;
        let typed = std::mem::take(&mut self.launch.typed);
        let prefs = &self.config.input.cursor;
        if !prefs.launch_icon {
            return;
        }
        let scale = self.renderer.scale();
        let box_px = (prefs.size * scale).round().max(1.0) as u32;
        let rest = memrium_de::cursor::busy_envelope(|phase| memrium_de::cursor_theme::picture(Shape::Progress, box_px, prefs, self.theme.accent, phase));
        let (land, side) = memrium_de::cursor::launch_spot(&rest, prefs, scale, self.launch.plane.unwrap_or(256));
        self.launch.rest = Some(rest);
        if !self.theme.animations.on("launch_flight") {
            return;
        }
        // the program's cell in the shell, and the one of a launcher lying on the desktop
        let cells: Vec<u32> = [id.to_string(), format!("fi:app:{icon}")].iter().filter_map(|k| self.renderer.app_icon_get(k)).collect();
        // (started by a shortcut with its icon nowhere to be seen: the cursor's icon alone, as without the animation)
        let Some(src) = pick(&self.launch.seen, &cells, (!typed).then_some(self.mouse)) else { return };
        let screen = self.mons.get(self.mon_at(src.at)).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        if self.launch.flights.len() >= MOST {
            self.launch.flights.remove(0);
        }
        self.launch.flights.push(Flight::new(src, self.mouse, land, side, screen));
        self.launch.in_air = true;
        // Enter in the launcher hid the pointer: the icon needs to be seen arriving somewhere
        self.typing_hid_cursor = false;
    }

    pub fn launch_tick(&mut self, raw_dt: f32) {
        if self.launch.flights.is_empty() {
            return;
        }
        let dt = self.theme.animations.dt("launch_flight", raw_dt);
        let mut flights = std::mem::take(&mut self.launch.flights);
        let newest = flights.len() - 1;
        for (i, f) in flights.iter_mut().enumerate() {
            if !f.step(dt) {
                continue;
            }
            let mut taken_over = false;
            if i == newest && self.launch.in_air {
                self.launch.in_air = false;
                self.launch.landed = Some(Instant::now());
                self.update_cursor_shape();
                self.cursor_gen = self.cursor_gen.wrapping_add(1);
                taken_over = self.launch.plane.is_some() && !self.typing_hid_cursor && self.launch_icon_on_cursor();
            }
            if !taken_over {
                f.burst();
            }
        }
        flights.retain(|f| !f.over());
        self.launch.flights = flights;
    }

    pub fn launch_panels(&self, shell: &mut Vec<Panel>) {
        for f in &self.launch.flights {
            shell.extend(f.panels(self.mouse));
        }
    }

    /// The cursor's picture carries the launch icon now.
    pub fn launch_icon_on_cursor(&self) -> bool {
        self.busy_icon.is_some() && !self.launch.in_air && self.cursor_shape == Shape::Progress && self.config.input.cursor.launch_icon
    }

    /// Seconds into the cursor icon's own animation.
    pub fn launch_time(&self) -> f32 {
        match self.launch.landed {
            Some(t) => memrium_de::cursor::landing_time(&self.config.input.cursor) + t.elapsed().as_secs_f32(),
            None => self.busy_until.map_or(0.0, |until| crate::pointer::BUSY_FOR.saturating_sub(until.saturating_duration_since(Instant::now())).as_secs_f32()),
        }
    }
}
