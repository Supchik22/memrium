//! Which cursor the pointer shows: what a client asked for while the pointer is over it, and what the shell itself
//! knows (a window's edge, a button, a drag, a program that is starting).

use crate::state::{Grab, Hit, State};
use memrium_de::cursor::Shape;
use std::time::{Duration, Instant};

/// How long the pointer shows "starting" after a program was launched, if no window turns up.
const BUSY_FOR: Duration = Duration::from_secs(6);

impl State {
    /// A program was started from the shell: show the busy cursor until its window appears.
    pub fn start_busy(&mut self) {
        self.busy_until = Some(Instant::now() + BUSY_FOR);
        self.busy_wins = self.open_window_count();
    }

    fn open_window_count(&self) -> usize {
        self.wins.iter().filter(|w| w.mapped && !w.or).count() + self.settings.open as usize + self.files.open as usize
    }

    /// The cursor the shell wants at the pointer, if it has an opinion (`None`: leave it to the client under it).
    fn shell_cursor(&self) -> Option<Shape> {
        match &self.grab {
            Some(Grab::Move { .. }) | Some(Grab::ShellMove { .. }) => return Some(Shape::Grabbing),
            Some(Grab::Resize { edges, .. }) | Some(Grab::ShellResize { edges, .. }) => return Some(Shape::for_edges(*edges)),
            None => {}
        }
        if self.file_drag.as_ref().is_some_and(|d| d.active) {
            return Some(Shape::Grabbing);
        }
        if self.menu.open {
            return None;
        }
        if self.launcher_modal() {
            return Some(if self.launcher.over_app(self.mouse) { Shape::Pointer } else { Shape::Default });
        }
        let m = self.mouse;
        if let Some((_, e)) = self.shell_edge_hit(m) {
            return Some(Shape::for_edges(e));
        }
        match self.chrome_at(m) {
            Some((_, Hit::Edge(e))) => return Some(Shape::for_edges(e)),
            Some((_, Hit::Button(_))) => return Some(Shape::Pointer),
            _ => {}
        }
        if self.preview_hit(m) || self.dock_item_at().is_some() || self.bar_hit(m).is_some() {
            return Some(Shape::Pointer);
        }
        // a control of the Settings window under the pointer
        if self.focused_shell() == Some(crate::state::Sw::Settings) || self.shell_covers(m) == Some(crate::state::Sw::Settings) {
            if self.shell_covers(m) == Some(crate::state::Sw::Settings) {
                use chebrium::ui::Cursor;
                match self.settings.cursor_hint() {
                    Cursor::Text => return Some(Shape::Text),
                    Cursor::Pointer => return Some(Shape::Pointer),
                    _ => {}
                }
            }
        }
        let busy = self.busy_until.is_some_and(|t| t > Instant::now());
        if busy && !self.pointer_on_client() {
            return Some(Shape::Progress);
        }
        None
    }

    fn pointer_on_client(&self) -> bool {
        self.cursor_focus.is_some()
    }

    /// Work out the cursor for the pointer's place.
    pub fn update_cursor_shape(&mut self) {
        let prefs = self.config.input.cursor.clone();
        let shell = if prefs.dynamic { self.shell_cursor() } else { None };
        let shape = match shell {
            Some(s) => s,
            None if self.pointer_on_client() => self.client_cursor,
            None => Shape::Default,
        };
        if shape != self.cursor_shape {
            if std::env::var_os("MEMRIUM_DEBUG_POINTER").is_some() {
                eprintln!("pointer: cursor {:?} -> {:?} (client picture: {})", self.cursor_shape, shape, self.client_cursor_img.is_some());
            }
            self.cursor_shape = shape;
            self.cursor_gen = self.cursor_gen.wrapping_add(1);
        }
        self.cursor_check = Instant::now();
    }

    /// Now and then, for what changes without the pointer moving (the busy cursor ending, a window opening under it).
    pub fn tick_cursor(&mut self) {
        if self.busy_until.is_some() && self.open_window_count() > self.busy_wins {
            self.busy_until = None;
        }
        if self.busy_until.is_some_and(|t| t <= Instant::now()) {
            self.busy_until = None;
        }
        if self.cursor_check.elapsed() > Duration::from_millis(120) {
            self.update_cursor_shape();
        }
    }
}
