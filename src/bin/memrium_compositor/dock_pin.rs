//! Adding a program to the Dock from a launcher: dragged onto the Dock, or from its right-click menu. The Dock comes up
//! over the launcher and makes room, a copy of the icon hops over in an arc (the launch animation's flight) and the icon
//! takes it over with a squash.

use crate::state::{MenuFor, State};
use memrium_de::launch_flight::{pick, Flight};
use memrium_de::menu::MenuItem;
use memrium_de::renderer::{Label, Panel};
use memrium_de::theme::Edge;

const L_OPEN: u32 = 8301;
const L_PIN: u32 = 8302;
const L_DESKTOP: u32 = 8303;
/// the Dock takes a drop this far off it as well
const REACH: f32 = 36.0;
/// how long the Dock stays up after a landing, seconds
const LINGER: f32 = 1.1;

struct Hop {
    key: String,
    flight: Flight,
    /// where the slot was last seen
    to: [f32; 2],
}

#[derive(Default)]
pub struct Pinning {
    hops: Vec<Hop>,
    /// the program of the launcher's context menu and where it was right-clicked
    menu: Option<(String, [f32; 2])>,
    /// seconds the Dock still stays up (counted in the steps of the animations)
    linger: f32,
    /// it was called up over the launcher, and is that until it has slid away again
    over: bool,
}

impl Pinning {
    /// The icon of `key` is still in the air: its slot is empty.
    pub fn arriving(&self, key: &str) -> bool {
        self.hops.iter().any(|h| h.key == key)
    }
}

/// Where in the pins a drop at `along` goes: `slots` are the middles of the pinned icons along the Dock, in their order.
fn insert_at(slots: &[Option<f32>], along: f32) -> usize {
    slots.iter().rposition(|m| m.is_some_and(|m| m < along)).map_or(0, |i| i + 1)
}

impl State {
    fn dock_bar(&self) -> Option<usize> {
        if !self.bar_active() {
            return None;
        }
        self.bars.iter().position(|b| b.has("dock"))
    }

    /// The bar with the Dock, its box on the main screen.
    fn dock_box(&self) -> Option<(usize, [f32; 4], Edge)> {
        let i = self.dock_bar()?;
        self.bar_mon.set(self.main_mon);
        let o = self.mon_origin(self.main_mon);
        let r = self.with_bar_ctx(|c| self.bars[i].rect(c));
        Some((i, [r[0] + o[0], r[1] + o[1], r[2], r[3]], self.bars[i].cfg.edge))
    }

    /// A drop at `p` is onto the Dock: which place among the pins it takes.
    pub fn dock_drop_at(&self, p: [f32; 2]) -> Option<usize> {
        let (_, r, edge) = self.dock_box()?;
        let m = self.mons.get(self.main_mon).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        let (across, along) = if edge.vertical() { (0, 1) } else { (1, 0) };
        // (from the screen's edge to a little beyond the Dock, also while it is still sliding in)
        let far = matches!(edge, Edge::Bottom | Edge::Right);
        let depth = r[2 + across] + REACH + 14.0;
        let deep = if far { p[across] >= m[across] + m[2 + across] - depth } else { p[across] <= m[across] + depth };
        let beside = p[along] >= r[along] - REACH * 2.0 && p[along] <= r[along] + r[2 + along] + REACH * 2.0;
        if !(deep && beside) {
            return None;
        }
        let o = self.mon_origin(self.main_mon);
        let slots: Vec<Option<f32>> = self.pinned.iter().map(|k| self.item_center(k).map(|c| c[along] + o[along])).collect();
        Some(insert_at(&slots, p[along]))
    }

    /// The pointer let go of a program dragged out of a launcher: was that onto the Dock? Then it goes there.
    pub fn launcher_drop_on_dock(&mut self) -> bool {
        let Some(id) = self.launcher.dragged_app(&self.apps).map(|i| self.apps[i].id.clone()) else { return false };
        let Some(at) = self.dock_drop_at(self.mouse) else { return false };
        self.launcher.cancel_drag();
        self.pin_flying(&id, Some(at), self.mouse);
        true
    }

    /// Keep `id` in the Dock (at place `at`, else last); a copy of its icon hops there from the one at `from`.
    fn pin_flying(&mut self, id: &str, at: Option<usize>, from: [f32; 2]) {
        let mut at = at.unwrap_or(self.pinned.len());
        if let Some(was) = self.pinned.iter().position(|k| k == id) {
            self.pinned.remove(was);
            if was < at {
                at -= 1;
            }
        }
        self.pinned.insert(at.min(self.pinned.len()), id.to_string());
        self.save_pins();
        self.pinning.linger = LINGER;
        if !self.theme.animations.on("launch_flight") {
            return;
        }
        let (Some((_, r, edge)), Some(cell)) = (self.dock_box(), self.renderer.app_icon_get(id)) else { return };
        let Some(src) = pick(self.launch_seen(), &[cell], Some(from)) else { return };
        // until the new slot is laid out: the Dock's far end
        let m = self.mons.get(self.main_mon).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        let to = if edge.vertical() {
            [(r[0] + r[2] / 2.0).clamp(m[0], m[0] + m[2]), r[1] + r[3]]
        } else {
            [r[0] + r[2], (r[1] + r[3] / 2.0).clamp(m[1], m[1] + m[3])]
        };
        let side = self.theme.dock.icon_size;
        let screen = self.mons.get(self.mon_at(src.at)).copied().unwrap_or(m);
        self.pinning.hops.retain(|h| h.key != id);
        self.pinning.hops.push(Hop { key: id.to_string(), flight: Flight::new(src, to, [0.0; 2], side, screen), to });
    }

    /// The Dock is called up: a program is being dragged, is on its way, or has just landed.
    pub fn dock_called(&self) -> bool {
        self.launcher.dragged_app(&self.apps).is_some() || !self.pinning.hops.is_empty() || self.pinning.linger > 0.0
    }

    pub fn pinning_tick(&mut self, raw_dt: f32) {
        self.pinning.linger = (self.pinning.linger - raw_dt.min(0.05)).max(0.0);
        match self.dock_bar() {
            Some(i) => {
                let called = self.dock_called();
                self.bars[i].set_called(called);
                self.pinning.over = self.launcher.visible() && (called || (self.pinning.over && self.bars[i].revealed()));
                self.bars[i].set_quiet(self.launcher.visible() && called);
            }
            None => self.pinning.over = false,
        }
        if self.pinning.hops.is_empty() {
            return;
        }
        let dt = self.theme.animations.dt("launch_flight", raw_dt);
        let o = self.mon_origin(self.main_mon);
        let mut hops = std::mem::take(&mut self.pinning.hops);
        hops.retain_mut(|h| {
            if let Some((c, size)) = self.bars.iter().find_map(|b| Some((b.item_center(&h.key)?, b.item_size(&h.key)?))) {
                h.to = [c[0] + o[0], c[1] + o[1]];
                h.flight.set_side(size);
            }
            if !h.flight.step(dt) {
                return true;
            }
            for b in &mut self.bars {
                b.landed(&h.key);
            }
            self.pinning.linger = LINGER;
            false
        });
        self.pinning.hops = hops;
    }

    /// The Dock over the launcher, under what is dragged: its panels and names, and where a drop would go.
    pub fn dock_over_launcher(&self) -> (Vec<Panel>, Vec<Label>) {
        if !self.launcher.visible() || !(self.pinning.over || self.dock_called()) {
            return (Vec::new(), Vec::new());
        }
        let Some((i, r, edge)) = self.dock_box() else { return (Vec::new(), Vec::new()) };
        let (mut p, mut l) = self.with_bar_ctx(|c| self.bars[i].draw(c));
        let o = self.mon_origin(self.main_mon);
        for q in &mut p {
            q.rect[0] += o[0];
            q.rect[1] += o[1];
        }
        for t in &mut l {
            t.x += o[0];
            t.y += o[1];
        }
        if self.launcher.dragged_app(&self.apps).is_none() {
            return (p, l);
        }
        let Some(at) = self.dock_drop_at(self.mouse) else { return (p, l) };
        let a = self.theme.accent;
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        p.push(Panel::solid(cx, cy, r[2], r[3], self.theme.dock.radius, a, a).flat().alpha(0.16));
        // a line between the two it would come between
        let along = if edge.vertical() { 1 } else { 0 };
        let mid = |k: Option<&String>| k.and_then(|k| self.item_center(k)).map(|c| c[along] + o[along]);
        let half = self.theme.dock.icon_size / 2.0 + 4.0;
        let place = match (mid(at.checked_sub(1).and_then(|j| self.pinned.get(j))), mid(self.pinned.get(at))) {
            (Some(before), Some(after)) => (before + after) / 2.0,
            (Some(before), None) => before + half,
            (None, Some(after)) => after - half,
            (None, None) => r[along] + r[2 + along] / 2.0,
        };
        let thick = (r[3 - along] * 0.66).max(12.0);
        let line = if edge.vertical() { [cx, place, thick, 3.0] } else { [place, cy, 3.0, thick] };
        p.push(Panel::solid(line[0], line[1], line[2], line[3], 1.5, a, a).flat().alpha(0.95));
        (p, l)
    }

    pub fn pinning_panels(&self, shell: &mut Vec<Panel>) {
        for h in &self.pinning.hops {
            shell.extend(h.flight.panels(h.to));
        }
    }

    /// The right button on a program in a launcher: its menu.
    pub fn open_launcher_menu(&mut self) -> bool {
        let Some(app) = self.launcher.app_at(self.mouse, &self.apps).and_then(|i| self.apps.get(i)) else { return false };
        let id = app.id.clone();
        let mut items = vec![MenuItem::new(L_OPEN, "Open").icon("external-link"), MenuItem::sep()];
        if self.dock_bar().is_some() {
            items.push(MenuItem::new(L_PIN, if self.pinned.contains(&id) { "Remove from Dock" } else { "Add to Dock" }).icon("star"));
        }
        items.push(MenuItem::new(L_DESKTOP, "Add to Desktop").icon("monitor"));
        self.pinning.menu = Some((id, self.mouse));
        self.menu_for = MenuFor::Launcher;
        self.menu.open_at(self.mouse, items);
        true
    }

    pub fn launcher_menu_command(&mut self, cmd: u32) {
        let Some((id, from)) = self.pinning.menu.take() else { return };
        let Some(app) = self.apps.iter().find(|a| a.id == id).cloned() else { return };
        match cmd {
            L_OPEN => {
                self.launch_app(&app);
                self.launcher.close();
            }
            L_PIN => match self.pinned.iter().position(|k| *k == id) {
                Some(i) => {
                    self.pinned.remove(i);
                    self.save_pins();
                }
                None => self.pin_flying(&id, None, from),
            },
            L_DESKTOP => {
                let dir = self.desktop.dir().to_path_buf();
                match memrium_de::apps::write_shortcut(&app, &dir) {
                    Ok(_) => {
                        self.fm.gen += 1;
                        self.desktop.reload();
                    }
                    Err(e) => eprintln!("no launcher for {}: {e}", app.name),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::insert_at;

    #[test]
    fn a_drop_takes_the_place_between_the_icons_it_falls_between() {
        let slots = [Some(100.0), Some(160.0), None, Some(220.0)];
        assert_eq!(insert_at(&slots, 40.0), 0);
        assert_eq!(insert_at(&slots, 130.0), 1);
        assert_eq!(insert_at(&slots, 190.0), 2);
        assert_eq!(insert_at(&slots, 900.0), 4);
        assert_eq!(insert_at(&[], 5.0), 0);
    }
}
