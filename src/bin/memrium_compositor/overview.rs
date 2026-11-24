//! The overview (Exposé with the desktops along the top): every window of the desktop as a card, and every desktop as a
//! live miniature. Cards and miniatures can be dragged between the desktops, the desktops into another order.
//!
//! * Windows are named by `OvId` (a window's number, not its place in the list): one may open, close or come to the front
//!   while it is being dragged.
//! * A card's place is a spring (`Pose`) in its desktop's own coordinates, so the cards re-flow, and slide along with the
//!   row of desktops. What has left its place (carried, flying to a desktop, coming back) is a `Loose`, drawn over all.
//! * The shell takes the pointer and the keys: no program sees them until the overview closes (then `repick_pointer`
//!   gives the pointer back to what is under it).
//!
//! Drawing is in `overview_draw.rs`.

use super::*;
use memrium_de::cursor::Shape;

const SHELLS: [Sw; 1] = [Sw::Settings];
/// A press becomes a drag after this much travel (logical px).
const SLOP: f32 = 7.0;
/// Below a card: its icon and title.
pub(super) const LABEL_H: f32 = 34.0;
/// The close button of a card.
pub(super) const CLOSE: f32 = 26.0;
/// One step of the wheel, and the rest after it before the next desktop.
const WHEEL_STEP: f32 = 12.0;
const WHEEL_REST: std::time::Duration = std::time::Duration::from_millis(260);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OvId {
    /// a client window, by `Win::id`
    Win(u64),
    Shell(Sw),
}

/// A window as the overview sees it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    pub id: OvId,
    pub ws: usize,
    pub frame: [f32; 4],
    /// put away by the user (not merely by being on another desktop)
    pub minimized: bool,
    pub stamp: u64,
}

/// Where a card is going: the middle, and the share of the window's size.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Place {
    pub c: [f32; 2],
    pub s: f32,
}

impl Place {
    pub fn rect(self, size: [f32; 2]) -> [f32; 4] {
        let (w, h) = (size[0] * self.s, size[1] * self.s);
        [self.c[0] - w / 2.0, self.c[1] - h / 2.0, w, h]
    }
}

/// A place that moves. The scale is sprung times 1000: the spring's idea of rest is meant for pixels.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    x: Spring,
    y: Spring,
    s: Spring,
}

impl Motion {
    fn at(p: Place, stiffness: f32) -> Motion {
        let sp = |v: f32| Spring::new(v, stiffness, 0.86);
        Motion { x: sp(p.c[0]), y: sp(p.c[1]), s: sp(p.s * 1000.0) }
    }

    pub fn now(&self) -> Place {
        Place { c: [self.x.x, self.y.x], s: self.s.x / 1000.0 }
    }

    fn aim(&mut self, p: Place) {
        self.x.target = p.c[0];
        self.y.target = p.c[1];
        self.s.target = p.s * 1000.0;
    }

    fn step(&mut self, an: &memrium_de::theme::AnimStyle, dt: f32) {
        for s in [&mut self.x, &mut self.y, &mut self.s] {
            an.step("workspace_slide", s, dt);
        }
    }

    fn snap(&mut self) {
        for s in [&mut self.x, &mut self.y, &mut self.s] {
            s.snap();
        }
    }

    /// 1 standing in its place, less the further it still has to go.
    pub fn arrived(&self) -> f32 {
        1.0 - ((self.x.x - self.x.target).hypot(self.y.x - self.y.target) / 120.0).min(1.0)
    }

    /// Nothing left to see of the motion.
    fn settled(&self) -> bool {
        let near = |s: &Spring, d: f32| (s.x - s.target).abs() < d && s.v.abs() < d * 60.0;
        near(&self.x, 0.5) && near(&self.y, 0.5) && near(&self.s, 2.0)
    }
}

pub(super) struct Pose {
    ws: usize,
    pub m: Motion,
    /// 1 while the pointer or the arrow keys are on it
    pub lift: Spring,
    /// 0 -> 1 as a card that came while the overview was open grows in
    pub born: Spring,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Goal {
    /// in the hand: where in the picture it is held (shares of its size), and how big it was when picked up
    Hand([f32; 2], f32),
    /// to its card on the desktop shown
    Card,
    /// into the miniature of this desktop
    Mini(usize),
}

/// A window off its place.
pub(super) struct Loose {
    pub id: OvId,
    pub m: Motion,
    goal: Goal,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Spot {
    Close(OvId),
    Card(OvId),
    /// a window's miniature in the tile of a desktop
    Mini(OvId, usize),
    Tile(usize),
    Empty,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    button: u32,
    at: [f32; 2],
    spot: Spot,
    /// it has travelled too far to be a click
    moved: bool,
}

/// A desktop dragged to another place in the row: where it was, where it would land, and where the pointer holds it.
#[derive(Clone, Copy, Debug)]
pub(super) struct TileDrag {
    pub from: usize,
    pub to: usize,
    hold: f32,
}

/// The drop target of what is carried.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Drop {
    Desk(usize),
    /// the area of the cards: the desktop shown
    Here,
    Nowhere,
}

pub struct Overview {
    /// the monitor it is on: chosen as it opens, so nothing jumps while dragging over to another
    mon: usize,
    open: bool,
    pub(super) poses: HashMap<OvId, Pose>,
    pub(super) loose: Vec<Loose>,
    press: Option<Press>,
    /// buttons whose press the overview took: their release is its own too
    held: Vec<u32>,
    pub(super) hover: Option<Spot>,
    /// the card picked by the pointer or the arrow keys
    pub(super) sel: Option<OvId>,
    pub(super) tile_drag: Option<TileDrag>,
    /// how far each desktop's tile is from its place in the row
    pub(super) shift: [Spring; WORKSPACES],
    /// each tile lighting up as a target
    pub(super) glow: [Spring; WORKSPACES],
    /// the card area lighting up as a target
    pub(super) here: Spring,
    /// the wallpaper as the tiles show it, and what it was made from
    pub(super) wall: Option<(u64, SurfaceTex)>,
    wheel: f32,
    wheel_at: Option<Instant>,
}

impl Default for Overview {
    fn default() -> Self {
        let sp = || Spring::new(0.0, 520.0, 0.9);
        Overview {
            mon: 0,
            open: false,
            poses: HashMap::new(),
            loose: Vec::new(),
            press: None,
            held: Vec::new(),
            hover: None,
            sel: None,
            tile_drag: None,
            shift: std::array::from_fn(|_| sp()),
            glow: std::array::from_fn(|_| sp()),
            here: sp(),
            wall: None,
            wheel: 0.0,
            wheel_at: None,
        }
    }
}

impl Overview {
    pub(super) fn is_loose(&self, id: OvId) -> bool {
        self.loose.iter().any(|l| l.id == id)
    }

    pub(super) fn carried(&self) -> Option<OvId> {
        self.loose.iter().find(|l| matches!(l.goal, Goal::Hand(..))).map(|l| l.id)
    }

    pub(super) fn closable(&self) -> Option<OvId> {
        match self.hover {
            Some(Spot::Card(id) | Spot::Close(id)) if self.carried().is_none() && self.tile_drag.is_none() => Some(id),
            _ => None,
        }
    }
}

fn centre(r: [f32; 4]) -> [f32; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn ease(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Windows of these sizes in `area`: the number of columns that shows them biggest, none bigger than it is, the last row
/// centred, room under each for its title.
fn grid(area: [f32; 4], sizes: &[[f32; 2]], label: f32) -> Vec<Place> {
    let n = sizes.len();
    if n == 0 {
        return Vec::new();
    }
    let gap = if n > 9 { 22.0 } else { 36.0 };
    let layout = |cols: usize| -> (f32, Vec<Place>) {
        let rows = n.div_ceil(cols);
        let cw = ((area[2] - gap * (cols as f32 - 1.0)) / cols as f32).max(8.0);
        let ch = ((area[3] - gap * (rows as f32 - 1.0)) / rows as f32).max(label + 8.0);
        let mut shown = 0.0;
        let places = sizes
            .iter()
            .enumerate()
            .map(|(j, sz)| {
                let (c, r) = (j % cols, j / cols);
                let in_row = if r == rows - 1 { n - r * cols } else { cols };
                let x0 = area[0] + (cols - in_row) as f32 * (cw + gap) / 2.0 + c as f32 * (cw + gap);
                let y0 = area[1] + r as f32 * (ch + gap);
                let s = (cw / sz[0].max(1.0)).min((ch - label) / sz[1].max(1.0)).min(1.0);
                shown += s;
                Place { c: [x0 + cw / 2.0, y0 + (ch - label) / 2.0], s }
            })
            .collect();
        (shown, places)
    };
    let mut best = layout(1);
    for cols in 2..=n {
        let next = layout(cols);
        // (as big either way, as with small windows: beside each other rather than one under the other)
        if next.0 >= best.0 - 0.001 {
            best = next;
        }
    }
    best.1
}

/// Where desktop `i` goes when desktop `from` is moved to place `to`.
fn moved_slot(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// The card an arrow key goes to from `at`: the nearest that way, one straight ahead before one to the side.
fn toward(at: [f32; 2], dir: [f32; 2], cards: &[(OvId, [f32; 2])]) -> Option<OvId> {
    cards
        .iter()
        .filter_map(|(id, c)| {
            let d = [c[0] - at[0], c[1] - at[1]];
            let ahead = d[0] * dir[0] + d[1] * dir[1];
            let aside = (d[0] * dir[1] - d[1] * dir[0]).abs();
            (ahead > 4.0).then_some((ahead + 2.2 * aside, *id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|c| c.1)
}

impl State {
    pub fn ov_active(&self) -> bool {
        self.ov.target > 0.5 || self.ov_drag.is_some()
    }

    /// The overview gets the pointer and the keys (not while something that lies over it does).
    pub fn ov_takes_input(&self) -> bool {
        self.ov_active() && self.lock.is_none() && self.shot_sel.is_none() && self.shot_edit.is_none() && self.live_shot.is_none()
    }

    /// Show or hide the overview.
    pub fn ov_set(&mut self, on: bool) {
        self.ov_drag = None;
        self.ov.target = if on { 1.0 } else { 0.0 };
        if on {
            self.ov_begin();
            return;
        }
        self.overview.open = false;
        self.ov_cancel();
        self.overview.hover = None;
        self.update_cursor_shape();
    }

    /// The overview comes into sight (by its key, or under the fingers).
    pub(super) fn ov_begin(&mut self) {
        if self.overview.open {
            return;
        }
        if self.grab.is_some() {
            self.end_grab();
        }
        self.overview.open = true;
        self.alt_tab = None;
        self.preview = None;
        self.menu.close();
        self.launcher.close();
        // what the shell had open would lie over the cards with none of the clicks
        self.scene.set_cc_open(false);
        self.close_card();
        self.close_flyout();
        if self.widgets_open() {
            self.toggle_widgets();
        }
        for p in self.popups.iter().rev().filter(|p| p.grab) {
            p.surface.send_popup_done();
        }
        self.drop_popups(|p| p.grab);
        self.take_pointer_from_client();
        self.cursor_focus = None;
        if self.ov.x < 0.02 {
            self.overview.mon = self.mon_at(self.mouse);
            self.overview.poses.clear();
            self.overview.loose.clear();
        }
        self.overview.sel = None;
        self.ov_tick(0.0);
        self.overview.poses.values_mut().for_each(|p| p.born.snap());
        self.ov_point();
    }

    pub(super) fn ov_monitor(&self) -> [f32; 4] {
        self.mons.get(self.overview.mon).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]])
    }

    /// For the headless test step `windows`: the desktops, the windows, and what the overview holds.
    pub fn ov_dump(&self) -> String {
        let mut out = format!("desk={} ov={:.3} frames={} cursor={:?} ptr_focus={} loose={} |", self.workspace, self.ov.x, self.frames_drawn, self.cursor_shape, self.pointer.current_focus().is_some(), self.overview.loose.len());
        let front = self.wins.iter().rposition(|w| !w.minimized && !w.or).filter(|_| self.focused_shell().is_none());
        for (i, w) in self.wins.iter().enumerate().filter(|(_, w)| w.mapped) {
            let f = Self::frame_rect(w).map(|v| v.round() as i32);
            out += &format!(" {}:{}:ws={}{}{}{}:{f:?}", w.id, w.app_id, w.ws, if w.ws_hidden { ":hidden" } else if w.minimized { ":min" } else { "" }, if w.or { ":or" } else { "" }, if front == Some(i) { ":front" } else { "" });
        }
        for k in SHELLS.into_iter().filter(|&k| self.is_open(k)) {
            out += &format!(" {k:?}:ws={}{}", self.shell_win(k).ws, if self.shell_win(k).minimized { ":min" } else { "" });
        }
        if self.overview.open {
            let all = self.ov_entries();
            for e in all.iter().filter(|e| e.ws == self.workspace) {
                if let Some(c) = self.ov_card(e.id, e.ws) {
                    out += &format!(" card {:?}@{:?}", e.id, c.rect([e.frame[2], e.frame[3]]).map(|v| v.round() as i32));
                }
            }
        }
        out
    }

    // ------------------------------------------------------------ what there is, and where it goes

    /// The managed windows and shell windows of every desktop, in the order they stack.
    pub(super) fn ov_entries(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = self
            .wins
            .iter()
            .filter(|w| w.mapped && !w.or && !w.desk)
            .map(|w| Entry { id: OvId::Win(w.id), ws: w.ws, frame: Self::frame_rect(w), minimized: w.minimized && !w.ws_hidden, stamp: w.stamp })
            .collect();
        for k in SHELLS {
            if self.is_open(k) {
                let w = self.shell_win(k);
                out.push(Entry { id: OvId::Shell(k), ws: w.ws, frame: self.sw_frame(k), minimized: w.minimized, stamp: w.stamp });
            }
        }
        out.sort_by_key(|e| e.stamp);
        out
    }

    /// The cards' places, desktop by desktop, by window number (they stay put when the focus changes); the minimized
    /// windows on a shelf of their own along the bottom.
    fn ov_places(&self, all: &[Entry]) -> Vec<(OvId, Place)> {
        let m = self.ov_monitor();
        let (_, tile) = self.ov_strip()[0];
        let top = tile[1] + tile[3] + 58.0;
        let side = (m[2] * 0.045).clamp(24.0, 90.0);
        let bottom = m[1] + m[3] - 84.0;
        let mut out = Vec::new();
        for ws in 0..WORKSPACES {
            let mut mine: Vec<&Entry> = all.iter().filter(|e| e.ws == ws).collect();
            mine.sort_by_key(|e| match e.id {
                OvId::Win(id) => id,
                OvId::Shell(_) => SHELL_SETTINGS,
            });
            let (away, shown): (Vec<&Entry>, Vec<&Entry>) = mine.into_iter().partition(|e| e.minimized);
            let mut end = bottom;
            if !away.is_empty() {
                let h = if shown.is_empty() { bottom - top } else { ((bottom - top) * 0.26).clamp(70.0, 190.0) };
                let sizes: Vec<[f32; 2]> = away.iter().map(|e| [e.frame[2], e.frame[3]]).collect();
                let places = grid([m[0] + side, bottom - h, (m[2] - 2.0 * side).max(100.0), h], &sizes, LABEL_H);
                out.extend(away.iter().map(|e| e.id).zip(places));
                end -= h + 22.0;
            }
            let sizes: Vec<[f32; 2]> = shown.iter().map(|e| [e.frame[2], e.frame[3]]).collect();
            let places = grid([m[0] + side, top, (m[2] - 2.0 * side).max(100.0), (end - top).max(80.0)], &sizes, LABEL_H);
            out.extend(shown.iter().map(|e| e.id).zip(places));
        }
        out
    }

    /// The strip of desktops along the top: each one's place in the row.
    pub(super) fn ov_strip(&self) -> Vec<(usize, [f32; 4])> {
        let m = self.ov_monitor();
        let gap = 18.0f32;
        let aspect = m[2] / m[3].max(1.0);
        let n = WORKSPACES as f32;
        let room = (m[2] - 40.0 - (n - 1.0) * gap) / n;
        let tw = ((m[3] * 0.115).clamp(80.0, 136.0) * aspect).min(room).max(40.0);
        let th = tw / aspect.max(0.1);
        let x0 = m[0] + (m[2] - n * tw - (n - 1.0) * gap) / 2.0;
        (0..WORKSPACES).map(|i| (i, [x0 + i as f32 * (tw + gap), m[1] + 44.0, tw, th])).collect()
    }

    /// The same as they are drawn: sliding along while one is dragged.
    pub(super) fn ov_tiles(&self) -> Vec<(usize, [f32; 4])> {
        self.ov_strip().into_iter().map(|(n, r)| (n, [r[0] + self.overview.shift[n].x, r[1], r[2], r[3]])).collect()
    }

    /// Where a window's miniature is in `tile` (a window of another monitor is shown as it lies on that one).
    pub(super) fn ov_mini(&self, e: &Entry, tile: [f32; 4]) -> Place {
        let c = centre(e.frame);
        let m = self.mons.get(self.mon_at(c)).copied().unwrap_or(self.ov_monitor());
        let k = (tile[2] / m[2].max(1.0)).min(tile[3] / m[3].max(1.0));
        Place { c: [tile[0] + tile[2] / 2.0 + (c[0] - m[0] - m[2] / 2.0) * k, tile[1] + tile[3] / 2.0 + (c[1] - m[1] - m[3] / 2.0) * k], s: k }
    }

    /// A card as it stands now on the screen: the desktop's slide included.
    pub(super) fn ov_card(&self, id: OvId, ws: usize) -> Option<Place> {
        let p = self.ov_card_on_desk(id)?;
        Some(Place { c: [p.c[0] + desk_dx(ws, self.ws_pos.x, self.size[0]), p.c[1]], s: p.s })
    }

    /// The same in its desktop's own coordinates, the hover's lift and its growing in included.
    pub(super) fn ov_card_on_desk(&self, id: OvId) -> Option<Place> {
        let p = self.overview.poses.get(&id)?;
        let at = p.m.now();
        let lift = 1.0 + 0.035 * p.lift.x / 100.0;
        let born = 0.6 + 0.4 * p.born.x / 100.0;
        Some(Place { c: at.c, s: at.s * lift * born })
    }

    /// Once a frame: the springs follow the layout.
    pub(super) fn ov_tick(&mut self, dt: f32) {
        if !self.ov_active() && self.ov.x < 0.001 {
            if self.overview.open || !self.overview.poses.is_empty() || self.overview.wall.is_some() {
                let held = std::mem::take(&mut self.overview.held);
                self.overview = Overview { held, ..Default::default() };
            }
            return;
        }
        let all = self.ov_entries();
        let places = self.ov_places(&all);
        let an = self.theme.animations.clone();
        let closing = !self.ov_active();
        // a shell window that was closed keeps its card while it fades away
        let fading: Vec<OvId> = SHELLS.into_iter().filter(|&k| self.shell_win(k).k.x > 0.01).map(OvId::Shell).collect();
        self.overview.poses.retain(|id, _| fading.contains(id) || all.iter().any(|e| e.id == *id));
        self.overview.loose.retain(|l| all.iter().any(|e| e.id == l.id));
        if self.overview.sel.is_some_and(|id| !all.iter().any(|e| e.id == id && e.ws == self.workspace)) {
            self.overview.sel = None;
        }
        let lit = if closing { None } else { self.overview.sel.filter(|_| self.overview.carried().is_none() && self.overview.tile_drag.is_none()) };
        for e in &all {
            let Some((_, place)) = places.iter().find(|(id, _)| *id == e.id).copied() else { continue };
            let p = self.overview.poses.entry(e.id).or_insert_with(|| {
                let mut born = Spring::new(0.0, 360.0, 0.8);
                born.target = 100.0;
                Pose { ws: e.ws, m: Motion::at(place, 260.0), lift: Spring::new(0.0, 520.0, 0.9), born }
            });
            p.m.aim(place);
            if p.ws != e.ws {
                // on another desktop it is simply there
                p.ws = e.ws;
                p.m.snap();
            }
            p.lift.target = if lit == Some(e.id) { 100.0 } else { 0.0 };
            p.m.step(&an, dt);
            an.step("workspace_slide", &mut p.lift, dt);
            an.step("workspace_slide", &mut p.born, dt);
        }
        let tiles = self.ov_tiles();
        for i in 0..self.overview.loose.len() {
            let (id, goal) = (self.overview.loose[i].id, self.overview.loose[i].goal);
            let Some(e) = all.iter().find(|e| e.id == id) else { continue };
            let to = match goal {
                Goal::Hand(hold, start) => self.ov_in_hand(e, hold, start),
                Goal::Card => match self.ov_card(id, e.ws) {
                    Some(p) => p,
                    None => continue,
                },
                Goal::Mini(n) => {
                    let mut p = self.ov_mini(e, tiles[n].1);
                    if e.minimized {
                        // a minimized window has no miniature: it goes into the middle of the tile
                        p = Place { c: centre(tiles[n].1), s: p.s * 0.3 };
                    }
                    p
                }
            };
            let m = &mut self.overview.loose[i].m;
            m.aim(to);
            m.step(&an, dt);
        }
        self.overview.loose.retain(|l| matches!(l.goal, Goal::Hand(..)) || !l.m.settled());

        // targets lighting up, and the tiles making room
        let drop = match self.overview.carried().and_then(|id| all.iter().find(|e| e.id == id)) {
            Some(e) => match self.ov_drop_at(self.mouse) {
                Drop::Desk(n) if n != e.ws => Drop::Desk(n),
                Drop::Here if e.ws != self.workspace => Drop::Here,
                _ => Drop::Nowhere,
            },
            None => Drop::Nowhere,
        };
        let strip = self.ov_strip();
        for n in 0..WORKSPACES {
            let o = &mut self.overview;
            o.glow[n].target = if drop == Drop::Desk(n) { 100.0 } else { 0.0 };
            o.shift[n].target = 0.0;
            if let Some(d) = o.tile_drag {
                if d.from == n {
                    let (lo, hi) = (strip[0].1[0] - 12.0, strip[WORKSPACES - 1].1[0] + 12.0);
                    o.shift[n].target = (self.mouse[0] - d.hold).clamp(lo, hi) - strip[n].1[0];
                    o.shift[n].snap();
                } else {
                    o.shift[n].target = strip[moved_slot(n, d.from, d.to)].1[0] - strip[n].1[0];
                }
            }
            an.step("workspace_slide", &mut o.glow[n], dt);
            an.step("workspace_slide", &mut o.shift[n], dt);
        }
        self.overview.here.target = if drop == Drop::Here { 100.0 } else { 0.0 };
        an.step("workspace_slide", &mut self.overview.here, dt);

        let key = self.renderer.wall_key();
        if self.overview.wall.as_ref().map(|w| w.0) != Some(key) {
            self.overview.wall = Some((key, self.renderer.wall_surface(strip[0].1[2] * self.world_over_monitor())));
        }
    }

    fn world_over_monitor(&self) -> f32 {
        (self.world[2] - self.world[0]) / self.ov_monitor()[2].max(1.0)
    }

    /// Where the carried window is: at the point it is held by, the smaller the nearer it is to the strip.
    fn ov_in_hand(&self, e: &Entry, hold: [f32; 2], start: f32) -> Place {
        let tile = self.ov_strip()[0].1;
        let (edge, p) = (tile[1] + tile[3], self.mouse);
        let t = ease(1.0 - (p[1] - edge) / (self.ov_monitor()[3] * 0.32).max(1.0));
        let (w, h) = (e.frame[2].max(1.0), e.frame[3].max(1.0));
        let small = (tile[2] * 0.62 / w).min(tile[3] * 0.72 / h).min(start);
        // a miniature picked up grows on the way down
        let big = start.max((self.ov_monitor()[2] * 0.22 / w).min(1.0)).min(1.0);
        let s = mix(big, small, t);
        Place { c: [p[0] + (0.5 - hold[0]) * w * s, p[1] + (0.5 - hold[1]) * h * s], s }
    }

    // ------------------------------------------------------------ hit testing

    fn ov_tile_at(&self, p: [f32; 2]) -> Option<usize> {
        self.ov_tiles().into_iter().find(|(_, r)| Self::in_rect(p, [r[0] - 8.0, r[1] - 8.0, r[2] + 16.0, r[3] + 36.0])).map(|(n, _)| n)
    }

    fn ov_drop_at(&self, p: [f32; 2]) -> Drop {
        if let Some(n) = self.ov_tile_at(p) {
            return Drop::Desk(n);
        }
        let tile = self.ov_strip()[0].1;
        if Self::in_rect(p, self.ov_monitor()) && p[1] > tile[1] + tile[3] + 34.0 {
            Drop::Here
        } else {
            Drop::Nowhere
        }
    }

    pub(super) fn ov_close_button(card: [f32; 4]) -> [f32; 4] {
        [card[0] - CLOSE * 0.36, card[1] - CLOSE * 0.36, CLOSE, CLOSE]
    }

    fn ov_spot(&self, p: [f32; 2]) -> Spot {
        let all = self.ov_entries();
        if let Some(n) = self.ov_tile_at(p) {
            let tile = self.ov_tiles()[n].1;
            // (the bottom of the tile and the name under it always hold the desktop itself)
            if Self::in_rect(p, tile) && p[1] < tile[1] + tile[3] - 8.0 {
                for e in all.iter().rev().filter(|e| e.ws == n && !e.minimized) {
                    if Self::in_rect(p, self.ov_mini(e, tile).rect([e.frame[2], e.frame[3]])) {
                        return Spot::Mini(e.id, n);
                    }
                }
            }
            return Spot::Tile(n);
        }
        let here: Vec<&Entry> = all.iter().filter(|e| e.ws == self.workspace).collect();
        let rect = |e: &Entry| self.ov_card(e.id, e.ws).map(|c| c.rect([e.frame[2], e.frame[3]]));
        for e in here.iter().rev().filter(|e| self.overview.closable() == Some(e.id)) {
            if rect(e).is_some_and(|r| Self::in_rect(p, Self::ov_close_button(r))) {
                return Spot::Close(e.id);
            }
        }
        for e in here.iter().rev() {
            if rect(e).is_some_and(|r| Self::in_rect(p, [r[0], r[1], r[2], r[3] + LABEL_H])) {
                return Spot::Card(e.id);
            }
        }
        Spot::Empty
    }

    /// What is under the pointer now.
    fn ov_point(&mut self) {
        let spot = self.ov_spot(self.mouse);
        self.overview.hover = Some(spot);
        if let Spot::Card(id) | Spot::Close(id) = spot {
            self.overview.sel = Some(id);
        }
    }

    /// The overview's cursor. None: it is not the overview's to say.
    pub fn ov_cursor(&self) -> Option<Shape> {
        if !self.ov_takes_input() {
            return None;
        }
        if self.overview.carried().is_some() || self.overview.tile_drag.is_some() {
            return Some(Shape::Grabbing);
        }
        if self.menu.open {
            return Some(Shape::Default);
        }
        if self.bar_hit(self.mouse).is_some() {
            return Some(Shape::Pointer);
        }
        if self.busy_until.is_some_and(|t| t > Instant::now()) && self.overview.hover == Some(Spot::Empty) {
            return Some(Shape::Progress);
        }
        Some(match self.overview.hover {
            Some(Spot::Card(_) | Spot::Mini(..)) => Shape::Grab,
            Some(Spot::Close(_) | Spot::Tile(_)) => Shape::Pointer,
            _ => Shape::Default,
        })
    }

    // ------------------------------------------------------------ the pointer

    pub fn ov_motion(&mut self) {
        let p = self.mouse;
        if let Some(mut press) = self.overview.press {
            if !press.moved && (p[0] - press.at[0]).hypot(p[1] - press.at[1]) > SLOP {
                press.moved = true;
                self.overview.press = Some(press);
                if press.button == BTN_LEFT {
                    self.ov_lift(press);
                }
            }
            if let Some(mut d) = self.overview.tile_drag {
                let strip = self.ov_strip();
                let mid = p[0] - d.hold + strip[0].1[2] / 2.0;
                let off = |r: &[f32; 4]| (r[0] + r[2] / 2.0 - mid).abs();
                d.to = strip.iter().min_by(|a, b| off(&a.1).total_cmp(&off(&b.1))).map_or(d.from, |s| s.0);
                self.overview.tile_drag = Some(d);
            }
            self.update_cursor_shape();
            return;
        }
        // the bar works as usual over the overview
        self.scene.set_mouse(p[0], p[1]);
        self.menu.set_mouse(p);
        self.bar_hover = self.bar_hit(p);
        self.bar_hover_mon = self.bar_mon.get();
        if self.bar_hover.is_some() || self.menu.open {
            self.overview.hover = None;
        } else {
            self.ov_point();
        }
        self.update_cursor_shape();
    }

    /// A press has become a drag: what it was on comes off its place.
    fn ov_lift(&mut self, press: Press) {
        match press.spot {
            Spot::Card(id) | Spot::Mini(id, _) => {
                let all = self.ov_entries();
                let Some(e) = all.iter().find(|e| e.id == id).copied() else { return };
                let from = match press.spot {
                    Spot::Mini(_, n) => Some(self.ov_mini(&e, self.ov_tiles()[n].1)),
                    _ => self.ov_card(id, e.ws),
                };
                let Some(from) = from else { return };
                let r = from.rect([e.frame[2], e.frame[3]]);
                // (held by the label under the card: by its lower edge)
                let hold = [((press.at[0] - r[0]) / r[2].max(1.0)).clamp(0.0, 1.0), ((press.at[1] - r[1]) / r[3].max(1.0)).clamp(0.0, 1.0)];
                self.overview.loose.retain(|l| l.id != id);
                self.overview.loose.push(Loose { id, m: Motion::at(from, 1300.0), goal: Goal::Hand(hold, from.s) });
            }
            Spot::Tile(n) => {
                let x = self.ov_tiles()[n].1[0];
                self.overview.tile_drag = Some(TileDrag { from: n, to: n, hold: press.at[0] - x });
            }
            Spot::Close(_) | Spot::Empty => {}
        }
    }

    /// The overview's mouse. True: the overview took it.
    pub fn ov_button(&mut self, button: u32, pressed: bool) -> bool {
        if !pressed {
            let Some(i) = self.overview.held.iter().position(|b| *b == button) else { return false };
            self.overview.held.remove(i);
            if self.overview.press.is_some_and(|p| p.button == button) {
                self.ov_release();
            }
            return true;
        }
        self.overview.held.retain(|b| *b != button);
        if !self.ov_takes_input() || self.menu.open {
            return false;
        }
        if self.overview.press.is_none() {
            // a notification is over the overview; the bar does what it does, and the overview makes way for it
            if self.toast_at(self.mouse).is_some() {
                return false;
            }
            if self.bar_hit(self.mouse).is_some() {
                self.ov_set(false);
                return false;
            }
            let spot = self.ov_spot(self.mouse);
            self.overview.press = Some(Press { button, at: self.mouse, spot, moved: false });
        } else if button == BTN_RIGHT {
            // the other button lets go of what is carried
            self.ov_cancel();
        }
        self.overview.held.push(button);
        true
    }

    fn ov_release(&mut self) {
        let Some(press) = self.overview.press.take() else { return };
        if let Some(d) = self.overview.tile_drag.take() {
            self.ov_reorder(d.from, d.to);
        } else if let Some(id) = self.overview.carried() {
            self.ov_drop(id);
        } else if !press.moved && self.ov_active() {
            match (press.button, press.spot) {
                (BTN_LEFT, Spot::Close(id)) | (BTN_MIDDLE, Spot::Card(id) | Spot::Close(id) | Spot::Mini(id, _)) => self.ov_close(id),
                (BTN_LEFT, Spot::Card(id)) => self.ov_pick(id),
                (BTN_LEFT, Spot::Tile(n) | Spot::Mini(_, n)) => self.switch_workspace(n),
                (BTN_LEFT, Spot::Empty) => self.ov_set(false),
                _ => {}
            }
        }
        if self.ov_active() {
            self.ov_point();
        }
        self.update_cursor_shape();
    }

    fn ov_drop(&mut self, id: OvId) {
        let Some(ws) = self.ov_ws(id) else { return };
        let to = match self.ov_drop_at(self.mouse) {
            Drop::Desk(n) => n,
            Drop::Here => self.workspace,
            Drop::Nowhere => ws,
        };
        // into a miniature, unless it lands on the desktop shown: there it has a card to go to
        let into_tile = to != self.workspace;
        if let Some(l) = self.overview.loose.iter_mut().find(|l| l.id == id) {
            l.goal = if into_tile { Goal::Mini(to) } else { Goal::Card };
            let now = l.m.now();
            l.m = Motion::at(now, 340.0);
        }
        if to != ws {
            self.ov_move_to(id, to);
        }
    }

    /// Let go of whatever is carried: all springs back.
    fn ov_cancel(&mut self) {
        self.overview.press = None;
        self.overview.tile_drag = None;
        let Some(id) = self.overview.carried() else { return };
        let (ws, cur) = (self.ov_ws(id), self.workspace);
        if let Some(l) = self.overview.loose.iter_mut().find(|l| l.id == id) {
            l.goal = match ws {
                Some(n) if n != cur => Goal::Mini(n),
                _ => Goal::Card,
            };
            let now = l.m.now();
            l.m = Motion::at(now, 340.0);
        }
    }

    pub fn ov_wheel(&mut self, dx: f32, dy: f32) {
        if self.overview.wheel_at.is_some_and(|t| t.elapsed() < WHEEL_REST) {
            self.overview.wheel = 0.0;
            return;
        }
        self.overview.wheel += if dx.abs() > dy.abs() { dx } else { dy };
        if self.overview.wheel.abs() < WHEEL_STEP {
            return;
        }
        let way = self.overview.wheel.signum() as i32;
        self.overview.wheel = 0.0;
        self.overview.wheel_at = Some(Instant::now());
        self.step_workspace(way);
        if self.overview.press.is_none() {
            self.ov_point();
        }
    }

    // ------------------------------------------------------------ the keys

    /// A key while the overview has them (all of them are its own; `bound`: the shortcut the key is).
    pub(super) fn ov_key(&mut self, mods: &ModifiersState, sym: Keysym, bound: Option<&str>) {
        let dragging = self.overview.carried().is_some() || self.overview.tile_drag.is_some();
        if sym == Keysym::Escape {
            if dragging {
                self.ov_cancel();
                self.update_cursor_shape();
            } else {
                self.ov_set(false);
            }
            return;
        }
        if let Some(act) = bound {
            match act {
                "overview" => self.ov_set(false),
                "close-window" | "close-window-2" => self.ov_close_selected(),
                "move-prev" | "move-next" => {
                    let n = (self.workspace as i32 + if act == "move-next" { 1 } else { -1 }).rem_euclid(WORKSPACES as i32) as usize;
                    self.ov_send_selected(n);
                }
                // (these would act on a window behind the overview)
                "minimize" | "maximize" | "snap-left" | "snap-right" | "snap-down" | "switch-window" => {}
                _ => {
                    let stays = act.starts_with("workspace-") || act.starts_with("screenshot") || matches!(act, "screen-record" | "fps-overlay" | "wallpaper");
                    if !stays {
                        self.ov_set(false);
                    }
                    self.key_action(act);
                }
            }
            return;
        }
        match sym {
            Keysym::Return | Keysym::KP_Enter => match self.overview.sel {
                Some(id) => self.ov_pick(id),
                None => self.ov_set(false),
            },
            Keysym::Left => self.ov_select([-1.0, 0.0]),
            Keysym::Right => self.ov_select([1.0, 0.0]),
            Keysym::Up => self.ov_select([0.0, -1.0]),
            Keysym::Down => self.ov_select([0.0, 1.0]),
            Keysym::Tab | Keysym::ISO_Left_Tab if !mods.alt => self.ov_select_next(mods.shift || sym == Keysym::ISO_Left_Tab),
            Keysym::Delete | Keysym::BackSpace => self.ov_close_selected(),
            _ => {}
        }
    }

    /// The number keys, by where they are (a layout may have other signs there): the desktop, with Shift the selected
    /// window is sent to it. True: it was one.
    pub(super) fn ov_digit(&mut self, evdev: u32, mods: &ModifiersState) -> bool {
        if !self.ov_takes_input() || mods.ctrl || mods.alt || mods.logo {
            return false;
        }
        let n = match evdev {
            2 | 79 => 0,
            3 | 80 => 1,
            4 | 81 => 2,
            5 | 75 => 3,
            _ => return false,
        };
        if mods.shift {
            self.ov_send_selected(n);
        } else {
            self.switch_workspace(n);
        }
        true
    }

    /// The cards of the desktop shown, where they are going, in reading order.
    fn ov_row(&self) -> Vec<(OvId, [f32; 2])> {
        let all = self.ov_entries();
        let mut cards: Vec<(OvId, [f32; 2])> = self.ov_places(&all).into_iter().filter(|(id, _)| all.iter().any(|e| e.id == *id && e.ws == self.workspace)).map(|(id, p)| (id, p.c)).collect();
        cards.sort_by(|a, b| ((a.1[1] / 40.0).round(), a.1[0]).partial_cmp(&((b.1[1] / 40.0).round(), b.1[0])).unwrap_or(std::cmp::Ordering::Equal));
        cards
    }

    fn ov_select(&mut self, dir: [f32; 2]) {
        let cards = self.ov_row();
        let at = self.overview.sel.and_then(|id| cards.iter().find(|c| c.0 == id)).map(|c| c.1);
        self.overview.sel = match at {
            Some(c) => toward(c, dir, &cards).or(self.overview.sel),
            None if dir[0] < 0.0 || dir[1] < 0.0 => cards.last().map(|c| c.0),
            None => cards.first().map(|c| c.0),
        };
        self.overview.hover = None;
    }

    fn ov_select_next(&mut self, back: bool) {
        let cards = self.ov_row();
        let n = cards.len();
        if n == 0 {
            return;
        }
        let at = self.overview.sel.and_then(|id| cards.iter().position(|c| c.0 == id));
        let next = match at {
            Some(i) => (i + if back { n - 1 } else { 1 }) % n,
            None => if back { n - 1 } else { 0 },
        };
        self.overview.sel = Some(cards[next].0);
        self.overview.hover = None;
    }

    /// The card beside the selected one, for when that one goes.
    fn ov_neighbour(&self, id: OvId) -> Option<OvId> {
        let cards = self.ov_row();
        let i = cards.iter().position(|c| c.0 == id)?;
        cards.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| cards.get(j))).map(|c| c.0)
    }

    fn ov_close_selected(&mut self) {
        let Some(id) = self.overview.sel else { return };
        let next = self.ov_neighbour(id);
        self.ov_close(id);
        self.overview.sel = next;
        self.overview.hover = None;
    }

    fn ov_send_selected(&mut self, to: usize) {
        let Some(id) = self.overview.sel else { return };
        let Some(ws) = self.ov_ws(id).filter(|ws| *ws != to) else { return };
        let next = self.ov_neighbour(id);
        if let Some(from) = self.ov_card(id, ws) {
            self.overview.loose.retain(|l| l.id != id);
            self.overview.loose.push(Loose { id, m: Motion::at(from, 300.0), goal: Goal::Mini(to) });
        }
        self.ov_move_to(id, to);
        self.overview.sel = next;
        self.overview.hover = None;
    }

    // ------------------------------------------------------------ what is done with a window

    pub(super) fn ov_index(&self, id: OvId) -> Option<usize> {
        match id {
            OvId::Win(wid) => self.wins.iter().position(|w| w.id == wid && !w.or),
            OvId::Shell(_) => None,
        }
    }

    fn ov_ws(&self, id: OvId) -> Option<usize> {
        match id {
            OvId::Win(_) => self.ov_index(id).map(|i| self.wins[i].ws),
            OvId::Shell(k) => self.is_open(k).then(|| self.shell_win(k).ws),
        }
    }

    /// A card was chosen: the overview closes and that window comes to the front.
    pub(super) fn ov_pick(&mut self, id: OvId) {
        match id {
            OvId::Win(_) => {
                let Some(i) = self.ov_index(id) else { return };
                // a minimized one grows out of its card, not out of the Dock
                self.wins[i].min_k.target = 0.0;
                self.wins[i].min_k.snap();
                self.focus_window(i);
            }
            OvId::Shell(k) => {
                self.open_shell(k);
                let w = self.shell_win_mut(k);
                w.genie = false;
                w.k.target = 1.0;
                w.k.snap();
            }
        }
        self.ov_set(false);
    }

    fn ov_close(&mut self, id: OvId) {
        match id {
            OvId::Win(_) => {
                if let Some(i) = self.ov_index(id) {
                    self.close_window(i);
                }
            }
            OvId::Shell(Sw::Settings) => {
                self.swin.genie = false;
                self.settings.close();
            }
        }
    }

    /// The window this one is a dialog of.
    fn ov_parent(&self, i: usize) -> Option<usize> {
        match &self.wins[i].role {
            Role::Xdg(t) => {
                let parent = t.parent()?;
                self.wins.iter().position(|w| w.role.is_surface(&parent))
            }
            Role::X11(x) => {
                let parent = x.is_transient_for()?;
                self.wins.iter().position(|w| !w.or && w.role.x11().is_some_and(|o| o.window_id() == parent))
            }
        }
    }

    /// Put a window on desktop `to`, with the dialogs of its program's window (a dialog takes its parent along).
    fn ov_move_to(&mut self, id: OvId, to: usize) {
        match id {
            OvId::Shell(k) => self.shell_win_mut(k).ws = to,
            OvId::Win(_) => {
                let Some(i) = self.ov_index(id) else { return };
                let top = |mut at: usize| {
                    // (bounded: programs have set parents in a circle)
                    for _ in 0..8 {
                        match self.ov_parent(at).filter(|p| *p != at) {
                            Some(p) => at = p,
                            None => break,
                        }
                    }
                    at
                };
                let root = top(i);
                let family: Vec<usize> = (0..self.wins.len()).filter(|&j| j == i || (!self.wins[j].or && top(j) == root)).collect();
                for j in family {
                    self.wins[j].ws = to;
                }
            }
        }
        // who is put away, and who has the keys, follows
        self.switch_workspace(self.workspace);
    }

    /// Desktop `from` goes to place `to` in the row.
    fn ov_reorder(&mut self, from: usize, to: usize) {
        if from == to {
            return;
        }
        let map = |i: usize| moved_slot(i, from, to);
        let strip = self.ov_strip();
        for w in self.wins.iter_mut().filter(|w| !w.or) {
            w.ws = map(w.ws);
        }
        for k in SHELLS {
            let ws = self.shell_win(k).ws;
            self.shell_win_mut(k).ws = map(ws);
        }
        self.overview.poses.values_mut().for_each(|p| p.ws = map(p.ws));
        for l in &mut self.overview.loose {
            if let Goal::Mini(n) = l.goal {
                l.goal = Goal::Mini(map(n));
            }
        }
        // each tile carries on from where it was drawn
        let (shift, glow) = (self.overview.shift, self.overview.glow);
        for i in 0..WORKSPACES {
            let n = map(i);
            self.overview.shift[n] = shift[i];
            self.overview.shift[n].x = strip[i].1[0] + shift[i].x - strip[n].1[0];
            self.overview.shift[n].target = 0.0;
            self.overview.glow[n] = glow[i];
        }
        // the desktop shown is the same one, at its new number: nothing slides
        let cur = map(self.workspace);
        self.workspace = cur;
        self.ws_pos.target = cur as f32;
        self.ws_pos.snap();
        self.switch_workspace(cur);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_stay_in_their_area_and_apart() {
        let area = [70.0, 220.0, 1460.0, 600.0];
        for n in 1..=24 {
            let sizes: Vec<[f32; 2]> = (0..n).map(|i| if i % 3 == 0 { [1600.0, 875.0] } else { [420.0 + i as f32 * 9.0, 520.0] }).collect();
            let places = grid(area, &sizes, LABEL_H);
            assert_eq!(places.len(), n);
            let rects: Vec<[f32; 4]> = places.iter().zip(&sizes).map(|(p, s)| p.rect(*s)).collect();
            for (i, r) in rects.iter().enumerate() {
                assert!(places[i].s > 0.0 && places[i].s <= 1.0, "{n}: scale");
                assert!(r[0] >= area[0] - 0.1 && r[0] + r[2] <= area[0] + area[2] + 0.1, "{n}: inside across");
                assert!(r[1] >= area[1] - 0.1 && r[1] + r[3] + LABEL_H <= area[1] + area[3] + 0.1, "{n}: inside down");
                for o in &rects[i + 1..] {
                    let apart = r[0] + r[2] <= o[0] || o[0] + o[2] <= r[0] || r[1] + r[3] <= o[1] || o[1] + o[3] <= r[1];
                    assert!(apart, "{n}: two cards overlap");
                }
            }
        }
    }

    #[test]
    fn a_small_window_is_never_blown_up() {
        assert_eq!(grid([0.0, 0.0, 1400.0, 700.0], &[[300.0, 200.0]], LABEL_H)[0].s, 1.0);
    }

    #[test]
    fn moving_a_desktop_keeps_every_desktop_once() {
        for from in 0..WORKSPACES {
            for to in 0..WORKSPACES {
                let mut got: Vec<usize> = (0..WORKSPACES).map(|i| moved_slot(i, from, to)).collect();
                assert_eq!(got[from], to);
                // the others keep their order
                let rest: Vec<usize> = (0..WORKSPACES).filter(|i| *i != from).map(|i| got[i]).collect();
                assert!(rest.windows(2).all(|w| w[0] < w[1]));
                got.sort_unstable();
                assert_eq!(got, (0..WORKSPACES).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn arrows_go_to_the_card_that_way() {
        let (a, b, c, d) = (OvId::Win(1), OvId::Win(2), OvId::Win(3), OvId::Win(4));
        let cards = [(a, [100.0, 100.0]), (b, [500.0, 110.0]), (c, [100.0, 400.0]), (d, [500.0, 400.0])];
        assert_eq!(toward([100.0, 100.0], [1.0, 0.0], &cards), Some(b));
        assert_eq!(toward([100.0, 100.0], [0.0, 1.0], &cards), Some(c));
        assert_eq!(toward([500.0, 400.0], [-1.0, 0.0], &cards), Some(c));
        assert_eq!(toward([500.0, 400.0], [0.0, -1.0], &cards), Some(b));
        assert_eq!(toward([500.0, 400.0], [1.0, 0.0], &cards), None);
    }
}
