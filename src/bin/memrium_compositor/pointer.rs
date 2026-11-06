//! Which cursor the pointer shows: what a client asked for while the pointer is over it, and what the shell itself
//! knows (a window's edge, a button, a drag, a program that is starting).

use crate::state::{Grab, Hit, State};
use memrium_de::cursor::Shape;
use smithay::reexports::wayland_server::Resource;
use std::time::{Duration, Instant};

/// How long the pointer shows "starting" after a program was launched, if no window turns up.
pub const BUSY_FOR: Duration = Duration::from_secs(6);

impl State {
    /// A program was started from the shell: show the busy cursor until its window appears.
    pub fn start_busy(&mut self) {
        self.busy_until = Some(Instant::now() + BUSY_FOR);
        self.busy_wins = self.open_window_count();
        self.busy_icon = None;
        self.cursor_gen = self.cursor_gen.wrapping_add(1);
    }

    /// A program that draws for 100% on a fractional desktop is stretched and soft. After a few seconds of that the shell
    /// remembers it (it starts on X11 next time, see softapps.rs) and says so once.
    pub fn watch_soft(&mut self) {
        if self.soft_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.soft_check = Instant::now();
        let want = self.renderer.scale();
        memrium_de::softapps::set_scale(want);
        if want < 1.01 {
            return;
        }
        let mut now_soft: Vec<String> = Vec::new();
        for w in self.wins.iter().filter(|w| w.mapped && !w.or && !w.minimized && !w.role.is_x11() && !w.app_id.is_empty()) {
            let Some(root) = w.role.surface() else { continue };
            let Some(n) = self.surfaces.get(&root.id()) else { continue };
            let (Some(t), Some(l)) = (n.tex.as_ref(), n.logical_size()) else { continue };
            // (a buffer smaller than the desktop's scale asks for; a window in the middle of a resize reads wrong, hence the wait)
            if t.size.0 as f32 / l[0].max(1.0) < want - 0.05 {
                now_soft.push(w.app_id.clone());
            }
        }
        self.soft_since.retain(|k, _| now_soft.contains(k));
        for id in now_soft {
            let since = *self.soft_since.entry(id.clone()).or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(4) && memrium_de::softapps::learn(&id) {
                eprintln!("soft: {id} is stretched at {want:.2}x; it starts on X11 from now on");
                self.shot_notice = Some((format!("{id} is drawn soft at this scale: next start uses a sharp mode"), 4.0));
            }
        }
    }

    fn open_window_count(&self) -> usize {
        self.wins.iter().filter(|w| w.mapped && !w.or).count() + self.settings.open as usize
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
        if self.busy_until.is_none() {
            self.busy_icon = None;
        }
        if self.cursor_check.elapsed() > Duration::from_millis(120) {
            self.update_cursor_shape();
        }
    }
}
