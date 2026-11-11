//! Fettle: Memrium's own look for the controls dropdown (the third `CcStyle`, beside the macOS and the Windows ones).
//!
//! What sets it apart:
//! * one matte sheet that every control sits on, instead of floating pills;
//! * controls that say what they are doing, not just what they are: the network and how strong it is, the headphones
//!   and their battery, a day sky with a sun that turns into a night sky with a moon;
//! * three sizes per control (small: the icon and a word; medium: icon, name and state; large: a little page), stepped
//!   with a badge in edit mode;
//! * a control grows in place to show its list (networks, devices, outputs and apps): held, or by its chevron; the
//!   others slide down out of its way;
//! * the brightness and the volume are liquid in a tube: drag anywhere on it or turn the wheel over it, and the surface
//!   sloshes a little as the level moves.
//!
//! Everything is built from the same tiles, panels and labels as the other looks; the animations (growing, the sun
//! setting, the liquid) rebuild the dropdown each frame while they run and stop when they settle.

use super::*;

/// A Wi-Fi network in reach.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcNetwork {
    pub ssid: String,
    /// 0 .. 100
    pub signal: u8,
    pub secure: bool,
    pub active: bool,
    pub saved: bool,
}

/// A Bluetooth device that is paired (or in reach).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcDevice {
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    /// the BlueZ icon name (`audio-headset`, `input-keyboard`, ...)
    pub icon: String,
    pub battery: Option<u8>,
}

/// A PipeWire node: an output device or an app's stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcNode {
    pub id: u32,
    pub name: String,
    /// 0 .. 1 (PipeWire allows more)
    pub vol: f32,
    pub muted: bool,
    pub default: bool,
}

/// What the shell read for Fettle's controls and lists (see [`ControlCenter::set_lists`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcLists {
    /// None: not read yet ("looking..."), as opposed to read and empty
    pub networks: Option<Vec<CcNetwork>>,
    pub devices: Option<Vec<CcDevice>>,
    pub outputs: Vec<CcNode>,
    pub streams: Vec<CcNode>,
    /// a stream's slider was moved a moment ago: the machine's reading lags, so ours is kept for a while
    pub held: Option<std::time::Instant>,
}

/// Which lists the open dropdown shows, so that the shell reads only those (and how soon again).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CcWants {
    pub networks: bool,
    pub devices: bool,
    pub audio: bool,
    /// a list is grown open: read it often
    pub soon: bool,
}

/// What a row of a grown control does.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RowAct {
    /// join this network; `card`: it needs a password, so the full network card opens instead
    Wifi(String, bool),
    /// connect to (or leave) a device
    Device(String, bool),
    Output(u32),
    /// the full card of this kind
    More(&'static str),
    /// Focus on for this long (or off)
    Focus(FocusFor),
}

/// Is this layout entry the Focus control (do not disturb)? A pill with the moon and no action of its own is one too
/// (the preset before Focus did anything).
fn is_focus(spec: &CcTile) -> bool {
    matches!(spec.kind.as_str(), "toggle" | "pill") && (spec.opt("action") == Some("focus") || (spec.opt("action").unwrap_or("").is_empty() && spec.opt("icon") == Some("moon")))
}

/// The list a control can grow to show: `wifi`, `bluetooth`, `sound` or `focus`.
fn list_kind(spec: &CcTile) -> Option<&'static str> {
    match spec.kind.as_str() {
        "wifi" => Some("wifi"),
        "bluetooth" => Some("bluetooth"),
        "slider" if spec.opt("target") == Some("sound") => Some("sound"),
        _ if is_focus(spec) => Some("focus"),
        _ => None,
    }
}

/// Focus's choices: what a row says and what it sets.
const FOCUS_ROWS: [(&str, &str, FocusFor); 4] = [
    ("For 1 hour", "timer", FocusFor::Hour),
    ("Until this evening", "sun-moon", FocusFor::Evening),
    ("Until tomorrow morning", "alarm-clock", FocusFor::Morning),
    ("Until I turn it off", "moon", FocusFor::On),
];

/// The level and the sloshing surface of one liquid slider (brightness, volume).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Liquid {
    /// the level shown (it follows the value with a short ease), 0 .. 1
    level: f32,
    /// how far the surface leans (px at the top edge; the bottom leans the other way), and how fast it swings
    tilt: f32,
    swing: f32,
    /// the level has been placed at least once (the first reading is not poured in from empty)
    placed: bool,
}

impl Liquid {
    /// Move towards `value` for `dt` seconds; true while anything still moves.
    fn step(&mut self, value: f32, width: f32, dt: f32) -> bool {
        if !self.placed {
            *self = Liquid { level: value, placed: true, ..Liquid::default() };
            return false;
        }
        let dt = dt.min(0.05);
        let before = self.level;
        // the level eases towards the value (quick, no overshoot: it is a reading)
        self.level += (value - self.level) * (1.0 - (-dt * 18.0).exp());
        if (self.level - value).abs() < 0.0005 {
            self.level = value;
        }
        // the surface: a lightly damped pendulum, pushed by how fast the level moved
        let push = (self.level - before) * width;
        self.swing += -push * 9.0;
        let (k, c) = (240.0, 7.0);
        self.swing += (-k * self.tilt - c * self.swing) * dt;
        self.tilt = (self.tilt + self.swing * dt).clamp(-9.0, 9.0);
        if self.tilt.abs() < 0.02 && self.swing.abs() < 0.3 {
            self.tilt = 0.0;
            self.swing = 0.0;
        }
        self.level != value || self.tilt != 0.0
    }
}

/// The sizes a kind of control can take, in cells (the first is its own); one size only: not resizable.
pub(super) fn sizes(kind: &str) -> &'static [(u8, u8)] {
    match kind {
        "wifi" | "bluetooth" | "pill" | "toggle" => &[(1, 1), (2, 1), (2, 2)],
        _ => &[],
    }
}

/// Step a control to its next size that fits where it is; true when it changed.
pub(super) fn cycle_size(layout: &mut [CcTile], spec: usize) -> bool {
    let Some(t) = layout.get(spec) else { return false };
    let all = sizes(&t.kind);
    if all.len() < 2 {
        return false;
    }
    let now = t.span();
    let at = all.iter().position(|s| *s == now).unwrap_or(0);
    for step in 1..all.len() {
        let (w, h) = all[(at + step) % all.len()];
        let mut try_t = layout[spec].clone();
        try_t.w = w;
        try_t.h = h;
        if cctiles::fits(layout, &try_t, Some(spec)) {
            layout[spec] = try_t;
            return true;
        }
    }
    false
}

/// The row a grown control moves up to: its own, or higher while a taller control beside it crosses that line (the
/// grown list must not cut through anything).
fn cut_row(layout: &[CcTile], row: u8) -> u8 {
    let mut c = row;
    while c > 0 && layout.iter().any(|t| t.row < c && c < t.row + t.span().1) {
        c -= 1;
    }
    c
}

/// the grid's width (4 cells and 3 gaps) and the sheet's margin round it
const GRID_W: f32 = 334.0;
const SHEET_PAD: f32 = 12.0;
/// a list shown on its own beside the controls: its width, the gap to them, the height of its title
const LIST_W: f32 = 300.0;
const LIST_GAP: f32 = 12.0;
const LIST_HEAD: f32 = 44.0;
/// a grown list: its header, a row, the footer
const HEAD: f32 = 70.0;
const ROW_H: f32 = 40.0;
const FOOT: f32 = 40.0;
/// most rows a list shows (the full card has the rest)
const MAX_NETS: usize = 6;
const MAX_DEVS: usize = 5;
const MAX_OUTS: usize = 3;
const MAX_APPS: usize = 4;
/// the day and the night sky of the dark mode control, the sun and the moon
const DAY: [f32; 3] = [0.36, 0.66, 0.98];
const NIGHT: [f32; 3] = [0.09, 0.11, 0.27];
const SUN: [f32; 3] = [1.0, 0.84, 0.32];
const MOON: [f32; 3] = [0.96, 0.95, 0.88];

#[derive(Clone, Copy, PartialEq)]
enum Size {
    S,
    M,
    L,
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The word under a small control, by what it does.
fn caption(action: &str, icon: &str, title: &str) -> String {
    if !title.is_empty() {
        return title.to_string();
    }
    match (action, icon) {
        ("screenshot", _) => "Screenshot",
        ("lock", _) => "Lock",
        ("settings", _) => "Settings",
        ("files", _) => "Files",
        ("terminal", _) => "Terminal",
        ("show-desktop", _) => "Desktop",
        ("launcher", _) => "Search",
        ("dark-mode", _) => "Dark mode",
        (_, "layout-panel-left") => "Layout",
        (_, "screen-share") => "Screens",
        (_, "moon") => "Focus",
        _ => "Action",
    }
    .to_string()
}

/// The glyph of a Bluetooth device by what BlueZ says it is (names the shell's icon set has).
fn device_icon(icon: &str) -> &'static str {
    match icon {
        i if i.contains("headset") || i.contains("headphone") || i.contains("audio") => "headphones",
        i if i.contains("keyboard") => "keyboard",
        i if i.contains("phone") => "smartphone",
        _ => "bluetooth",
    }
}

impl ControlCenter {
    // ------------------------------------------------------------ what the shell tells and asks

    /// The lists the shell read (see [`CcWants`]). Rebuilds when they differ.
    pub fn set_lists(&mut self, lists: &CcLists) {
        let mut lists = lists.clone();
        // a stream's slider moved a moment ago: keep our volumes (the reading lags behind the pointer)
        if self.lists.held.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1500)) {
            lists.streams = self.lists.streams.clone();
            lists.held = self.lists.held;
        }
        if self.lists != lists {
            self.lists = lists;
            if self.style == CcStyle::Fettle {
                self.rebuild();
            }
        }
    }

    /// Which lists the shell should read now (nothing while closed, or in another look).
    pub fn wants_lists(&self) -> CcWants {
        if self.style != CcStyle::Fettle || !self.open {
            return CcWants::default();
        }
        let grown = self.expanded.and_then(|i| self.layout.get(i));
        let kind = grown.map(|t| t.kind.as_str());
        let sound = grown.is_some_and(|t| t.kind == "slider" && t.opt("target") == Some("sound"));
        let has = |k: &str| self.layout.iter().any(|t| t.kind == k);
        CcWants { networks: has("wifi"), devices: has("bluetooth"), audio: sound, soon: kind.is_some() }
    }

    /// Open the dropdown with this kind of control grown (`wifi`, `bluetooth`, `sound`): the bar's icons lead here.
    /// False when the look is not Fettle or there is no such control (the shell opens its card instead).
    pub fn open_grown(&mut self, kind: &str) -> bool {
        if self.style != CcStyle::Fettle {
            return false;
        }
        let at = self.layout.iter().position(|t| list_kind(t) == Some(kind));
        let Some(at) = at else { return false };
        if self.open && self.expanded == Some(at) {
            self.set_open(false);
            return true;
        }
        self.set_open(true);
        self.set_expanded(Some(at));
        true
    }

    /// The wheel turned over the dropdown (`dy` > 0: down): over a liquid slider it pours. True when it was taken.
    pub fn scroll(&mut self, m: [f32; 2], dy: f32) -> bool {
        if self.style != CcStyle::Fettle || !self.open || self.edit || dy == 0.0 {
            return false;
        }
        let over = self.acts.iter().rev().find(|(t, a)| matches!(a, TileAct::Slider(_)) && Self::rect_hit(m, self.tiles[*t].c, self.tiles[*t].size)).map(|(t, a)| (*t, a.clone()));
        let Some((tile, TileAct::Slider(which))) = over else { return self.hit(m) };
        let now = self.slider_value(which);
        // one notch of a wheel is a 5% step (a touchpad's small deltas add up smoothly)
        let v = (now - dy.signum() * (dy.abs() / 15.0).clamp(0.2, 3.0) * 0.05).clamp(0.0, 1.0);
        let track = self.tracks.iter().find(|(t, _)| *t == tile).map_or((0.0, 1.0), |(_, tr)| *tr);
        self.pour(which, track, v);
        true
    }

    /// Set a slider to `v` at once (the wheel, the keyboard).
    fn pour(&mut self, which: Slider, track: (f32, f32), v: f32) {
        self.drag = Some(which);
        self.drag_anchor = None;
        self.drag_track = track;
        self.drag_to(track.0 + (track.1 - track.0) * v.clamp(0.0, 1.0));
        self.drag = None;
    }

    // ------------------------------------------------------------ keyboard

    /// A key while Fettle is open: the arrows (and Tab) walk the controls, left / right move a slider, Enter switches,
    /// runs or mutes, Open grows a control's list, Esc shrinks a grown list (the shell closes the dropdown on the
    /// next one). True when taken.
    pub fn key(&mut self, key: CcKey) -> bool {
        if self.style != CcStyle::Fettle || !self.open || self.edit {
            return false;
        }
        if key == CcKey::Esc {
            if self.expanded.is_some() {
                self.set_expanded(None);
                return true;
            }
            return false;
        }
        let n = self.layout.len();
        if n == 0 {
            return false;
        }
        let Some(f) = self.kb_focus.filter(|&f| f < n) else {
            // the first key shows where the keyboard is
            self.kb_focus = Some(0);
            self.rebuild();
            return true;
        };
        let slider = self.acts.iter().find(|(t, a)| matches!(a, TileAct::Slider(_)) && self.owner.get(*t) == Some(&f)).map(|(t, a)| (*t, a.clone()));
        match key {
            CcKey::Left | CcKey::Right if slider.is_some() => {
                let Some((tile, TileAct::Slider(which))) = slider else { return true };
                let track = self.tracks.iter().find(|(t, _)| *t == tile).map_or((0.0, 1.0), |(_, tr)| *tr);
                let step = if key == CcKey::Right { 0.05 } else { -0.05 };
                let v = self.slider_value(which) + step;
                self.pour(which, track, v);
            }
            CcKey::Next | CcKey::Prev => {
                self.kb_focus = Some(if key == CcKey::Next { (f + 1) % n } else { (f + n - 1) % n });
                self.rebuild();
            }
            CcKey::Left | CcKey::Right | CcKey::Up | CcKey::Down => {
                if let Some(to) = Self::neighbour(&self.layout, f, key) {
                    self.kb_focus = Some(to);
                    self.rebuild();
                }
            }
            CcKey::Open => {
                let growable = list_kind(&self.layout[f]).is_some();
                if growable {
                    self.set_expanded(if self.expanded == Some(f) { None } else { Some(f) });
                }
            }
            CcKey::Enter => self.key_enter(f),
            CcKey::Esc => {}
        }
        true
    }

    /// The control `f` is "pressed" from the keyboard: what a click on it does.
    fn key_enter(&mut self, f: usize) {
        let main = self.spec_tile.get(f).copied().unwrap_or(FREE);
        let mine: Vec<(usize, TileAct)> = self.acts.iter().filter(|(t, _)| self.owner.get(*t) == Some(&f)).cloned().collect();
        // the media control plays or pauses (its middle button)
        if let Some(&(t, off, _, _)) = self.media_hits.iter().filter(|h| h.0 == main).min_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs())) {
            let c = self.tiles[t].c;
            self.run_act(t, TileAct::Media, [c[0] + off[0], c[1] + off[1]]);
            return;
        }
        let pick = mine.iter().find(|(_, a)| matches!(a, TileAct::Wifi | TileAct::Bluetooth | TileAct::Focus | TileAct::Action(_) | TileAct::Mute)).cloned();
        if let Some((t, act)) = pick {
            let c = self.tiles[t].c;
            self.run_act(t, act, c);
        }
    }

    /// The control next to `f` in the direction of an arrow key, by where they sit (the nearest along that way,
    /// favouring the one most in line).
    fn neighbour(layout: &[CcTile], f: usize, key: CcKey) -> Option<usize> {
        let centre = |t: &CcTile| {
            let r = t.rect();
            [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
        };
        let from = centre(&layout[f]);
        let dir: [f32; 2] = match key {
            CcKey::Left => [-1.0, 0.0],
            CcKey::Right => [1.0, 0.0],
            CcKey::Up => [0.0, -1.0],
            _ => [0.0, 1.0],
        };
        layout
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != f)
            .filter_map(|(j, t)| {
                let c = centre(t);
                let (dx, dy) = (c[0] - from[0], c[1] - from[1]);
                let along = dx * dir[0] + dy * dir[1];
                let across = (dx * dir[1] - dy * dir[0]).abs();
                (along > 1.0).then_some((j, along + 2.0 * across))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(j, _)| j)
    }

    // ------------------------------------------------------------ growing in place

    /// Grow this layout entry in place (None: shrink back what is grown).
    pub(super) fn set_expanded(&mut self, spec: Option<usize>) {
        if self.expanded == spec {
            return;
        }
        self.expanded = spec;
        match spec {
            Some(i) => {
                // another one was grown: it gives way at once, the new one grows from its own place
                if self.exp_shown != Some(i) {
                    self.exp_k = Spring::new(0.0, 260.0, 0.78);
                }
                self.exp_shown = Some(i);
                self.exp_k.target = 1.0;
            }
            None => self.exp_k.target = 0.0,
        }
        self.rebuild();
    }

    /// Nothing grown, at once (the dropdown closes, or the edit mode opens).
    pub(super) fn collapse_now(&mut self) {
        if self.expanded.is_some() || self.exp_shown.is_some() {
            self.expanded = None;
            self.exp_shown = None;
            self.exp_k = Spring::new(0.0, 260.0, 0.78);
            if self.style == CcStyle::Fettle {
                self.rebuild();
            }
        }
    }

    /// Fettle's motion for one frame: growing, the sun setting, the liquids. True when the dropdown must be built again.
    pub(super) fn fettle_step(&mut self, dt: f32, an: &crate::theme::AnimStyle) -> bool {
        let mut again = false;
        if !self.exp_k.at_rest() {
            an.step("control_center", &mut self.exp_k, dt);
            again = true;
            if self.expanded.is_none() && self.exp_k.x <= 0.01 && self.exp_k.target == 0.0 {
                self.exp_k.snap();
                self.exp_shown = None;
            }
        }
        if !self.dark_k.at_rest() {
            an.step("control_center", &mut self.dark_k, dt);
            again = true;
        }
        let values = [self.sys.brightness.unwrap_or(0.45), self.sys.volume.map_or(0.7, |v| v.min(1.0))];
        for (l, v) in self.liquid.iter_mut().zip(values) {
            again |= l.step(v, GRID_W, dt);
        }
        again && self.shown()
    }

    // ------------------------------------------------------------ pressing

    /// A press on the open dropdown (not in edit mode).
    pub(super) fn fettle_mouse_down(&mut self, m: [f32; 2]) -> bool {
        // the pointer takes over from the keyboard
        if self.kb_focus.take().is_some() {
            self.rebuild();
        }
        let hit = |s: &ControlCenter, t: usize| t != FREE && t < s.tiles.len() && Self::rect_hit(m, s.tiles[t].c, s.tiles[t].size);
        // a press outside the grown control shrinks it back (and still does what it does)
        if let Some(g) = self.expanded {
            let main = self.spec_tile.get(g).copied().unwrap_or(FREE);
            // opened over the others: the card is the control itself
            if !hit(self, main) && !(self.side_tile != FREE && hit(self, self.side_tile)) {
                self.set_expanded(None);
            }
        }
        for (tile, act) in self.acts.clone().into_iter().rev() {
            if !hit(self, tile) {
                continue;
            }
            let spec = self.owner.get(tile).copied().unwrap_or(NONE);
            let growable = self.layout.get(spec).and_then(list_kind).is_some_and(|k| k != "sound");
            match act {
                // switches and actions wait for the release: held, a control with a list grows, any other edits
                TileAct::Wifi | TileAct::Bluetooth | TileAct::Focus | TileAct::Action(_) => {
                    self.hold = Some(Hold { from: m, t: 0.0, since: std::time::Instant::now(), act: Some(act), expand: growable.then_some(spec) });
                }
                act => self.run_act(tile, act, m),
            }
            return true;
        }
        let inside = self.tiles.iter().enumerate().any(|(i, t)| i != self.menu_tile && Self::rect_hit(m, t.c, t.size));
        if !inside {
            self.set_open(false);
        } else {
            self.hold = Some(Hold { from: m, t: 0.0, since: std::time::Instant::now(), act: None, expand: None });
        }
        inside
    }

    /// A row of a grown list was pressed.
    pub(super) fn run_row(&mut self, row: RowAct) {
        match row {
            RowAct::Wifi(_, true) => {
                self.events.push(CcEvent::Card("network".into()));
                self.set_open(false);
            }
            RowAct::Wifi(ssid, false) => self.events.push(CcEvent::WifiConnect(ssid)),
            RowAct::Device(mac, connected) => self.events.push(CcEvent::Bluetooth(vec![if connected { "disconnect" } else { "connect" }.into(), mac])),
            RowAct::Output(id) => {
                for o in &mut self.lists.outputs {
                    o.default = o.id == id;
                }
                self.events.push(CcEvent::DefaultSink(id));
                self.rebuild();
            }
            RowAct::Focus(f) => {
                self.events.push(CcEvent::Focus(f));
                self.focus_on = f != FocusFor::Off;
                self.focus_until = None;
                self.set_expanded(None);
                self.rebuild();
            }
            RowAct::More(kind) => {
                self.events.push(CcEvent::Card(kind.into()));
                self.set_open(false);
            }
        }
    }

    // ------------------------------------------------------------ building

    /// The height a grown list may take: what is left of the screen above the Dock once the other controls are there.
    fn list_room(&self) -> f32 {
        if self.grow_mode != crate::theme::QuickGrow::InPlace {
            // beside the controls or over them: the screen's height down to the Dock
            return (self.size[1] - 130.0 - GRID_TOP).max(HEAD + ROW_H + FOOT);
        }
        let base = cctiles::rows(&self.layout) as f32 * (cctiles::CELL_H + cctiles::GAP) - cctiles::GAP;
        (self.size[1] - 130.0 - GRID_TOP - base - cctiles::GAP).max(HEAD + ROW_H + FOOT)
    }

    /// How many rows each part of a grown list shows: (networks / devices / outputs, apps), fitted to the room.
    fn list_rows(&self, spec: &CcTile) -> (usize, usize) {
        let room = self.list_room();
        let fit = |used: f32, each: f32| ((room - used) / each).floor().max(1.0) as usize;
        match spec.kind.as_str() {
            _ if is_focus(spec) => (FOCUS_ROWS.len() + usize::from(self.focus_on), 0),
            "wifi" => (self.lists.networks.as_ref().map_or(1, |l| l.len().clamp(1, MAX_NETS)).min(fit(HEAD + FOOT, ROW_H)), 0),
            "bluetooth" => (self.lists.devices.as_ref().map_or(1, |l| l.iter().filter(|d| d.paired).count().clamp(1, MAX_DEVS)).min(fit(HEAD + FOOT, ROW_H)), 0),
            _ => {
                let apps_for = |outs: usize| {
                    let left = room - (HEAD + 26.0 + outs as f32 * ROW_H + 26.0 + FOOT);
                    self.lists.streams.len().min(MAX_APPS).min((left / 46.0).floor().max(0.0) as usize)
                };
                let outs = self.lists.outputs.len().clamp(1, MAX_OUTS).min(fit(HEAD + 26.0 + FOOT, ROW_H));
                // the apps matter more than every output: short of room, only the output in use is listed
                let want = self.lists.streams.len().min(2);
                if apps_for(outs) < want && outs > 1 {
                    (1, apps_for(1))
                } else {
                    (outs, apps_for(outs))
                }
            }
        }
    }

    /// How tall a grown control is (its list included).
    fn grown_height(&self, spec: &CcTile) -> f32 {
        let (n, apps) = self.list_rows(spec);
        match spec.kind.as_str() {
            _ if is_focus(spec) => HEAD + n as f32 * ROW_H + FOOT,
            "wifi" | "bluetooth" => HEAD + n as f32 * ROW_H + FOOT,
            "slider" if spec.opt("target") == Some("sound") => HEAD + 26.0 + n as f32 * ROW_H + if apps > 0 { 26.0 + apps as f32 * 46.0 } else { 0.0 } + FOOT,
            _ => spec.rect()[3],
        }
    }

    pub(super) fn build_fettle(&mut self) {
        let x0 = self.size[0] - 14.0 - GRID_W;
        let pill_x = self.pill_x;
        self.menu_tile = self.tile(Kind::Light, pill_x, 19.0, 54.0, 26.0, 13.0);
        let menu = self.menu_tile;
        self.glyph(menu, pill_x, 19.0, 16.0, "sliders-horizontal", INK, 0.85);

        let layout = self.layout.clone();
        let k = self.exp_k.x.max(0.0);
        let shown = self.exp_shown.filter(|&i| i < layout.len() && !self.edit);
        // beside: the others stay as they are, the control opens over them to the left (built after them)
        let in_place = self.grow_mode == crate::theme::QuickGrow::InPlace;
        let beside = if in_place { None } else { shown };
        let grown = if in_place { shown } else { None };
        // the list alone beside the controls (the control stays in the grid); `Over` opens the control itself
        let list_only = beside.is_some() && self.grow_mode != crate::theme::QuickGrow::Over;
        self.list_beside = if list_only { beside } else { None };
        let (cut_y, full_h) = match grown {
            Some(i) => (cut_row(&layout, layout[i].row) as f32 * (cctiles::CELL_H + cctiles::GAP), self.grown_height(&layout[i])),
            None => (0.0, 0.0),
        };
        // a control that grows where it is (a full-width one) pushes the others by what it grew; one that moves up to
        // make room leaves its place behind, so they move by all of its new height
        let moved_up = grown.is_some_and(|i| {
            let r = layout[i].rect();
            r[2] < GRID_W - 1.0 || (r[1] - cut_y).abs() > 0.5
        });
        let shift = match grown {
            Some(_) if moved_up => (full_h + cctiles::GAP) * k,
            Some(i) => (full_h - layout[i].rect()[3]) * k,
            None => 0.0,
        };
        let rows_h = cctiles::rows(&layout) as f32 * (cctiles::CELL_H + cctiles::GAP) - cctiles::GAP + shift;

        // the sheet under everything
        self.building = NONE;
        let (mut sheet_l, mut sheet_h) = (x0 - SHEET_PAD, rows_h);
        if let (Some(i), crate::theme::QuickGrow::Sheet) = (beside, self.grow_mode) {
            // one sheet for the list and the controls: it widens to the left (and grows down for a long list)
            sheet_l -= (LIST_W + LIST_GAP) * k;
            sheet_h = rows_h.max(self.list_height(&layout[i]) * k);
            self.sheet_glass = k.clamp(0.0, 1.0);
        }
        let sheet_w = x0 + GRID_W + SHEET_PAD - sheet_l;
        self.sheet_tile = self.tile(Kind::Sheet, sheet_l + sheet_w / 2.0, GRID_TOP + sheet_h / 2.0, sheet_w, sheet_h + 2.0 * SHEET_PAD, 30.0);

        for (i, spec) in layout.iter().enumerate() {
            let [mut rx, mut ry, mut rw, mut rh] = spec.rect();
            let grow = if Some(i) == grown { k } else { 0.0 };
            if Some(i) == grown {
                let to = [0.0, cut_y, GRID_W, full_h];
                rx += (to[0] - rx) * k;
                ry += (to[1] - ry) * k;
                rw += (to[2] - rw) * k;
                rh += (to[3] - rh) * k;
            } else if grown.is_some() && ry >= cut_y - 0.5 {
                ry += shift;
            }
            let moved = { let r = spec.rect(); r[2] < GRID_W - 1.0 || (r[1] - cut_y).abs() > 0.5 };
            if Some(i) == grown && k > 0.05 && moved {
                // its own place waits for it, faintly marked, where the others moved down to
                let [ox, oy, ow, oh] = spec.rect();
                let oy = oy + if oy >= cut_y - 0.5 { shift } else { 0.0 };
                self.panels.push(Att { tile: FREE, vis: Vis::Always, item: Panel::solid(x0 + ox + ow / 2.0, GRID_TOP + oy + oh / 2.0, ow, oh, 22.0, PURE, PURE).flat().alpha(0.045 * k.min(1.0)) });
            }
            if Some(i) == beside && !list_only {
                // it opens over the others, so it is built last (on top); its place in `spec_tile` is filled then
                self.spec_tile.push(FREE);
                continue;
            }
            let (cx, cy) = (x0 + rx + rw / 2.0, GRID_TOP + ry + rh / 2.0);
            self.building = i;
            let main = self.fettle_tile(spec, cx, cy, rw, rh, grow);
            if self.kb_focus == Some(i) {
                // where the keyboard is: lit, and a touch bigger (see `update`)
                let r = self.tiles[main].radius;
                self.panels.push(Att { tile: main, vis: Vis::Always, item: Panel::solid(cx, cy, rw, rh, r, PURE, PURE).flat().alpha(0.24) });
            }
            if Some(i) == beside {
                // its list is out beside: the control is lit a little
                let r = self.tiles[main].radius;
                self.panels.push(Att { tile: main, vis: Vis::Always, item: Panel::solid(cx, cy, rw, rh, r, PURE, PURE).flat().alpha(0.16 * k.min(1.0)) });
            }
            self.spec_tile.push(main);
            // edit mode: "−" in the top-left corner, the size badge in the bottom-right one
            let (bx, by) = (cx - rw / 2.0 + 7.0, cy - rh / 2.0 + 7.0);
            self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::glass(bx, by, 18.0, 18.0, 9.0).tinted([0.38, 0.41, 0.50, 0.70]).shadow(0.0, 0.0, 0.0) });
            self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::glyph(bx, by, 11.0, "minus", WHITE, 1.0) });
            if sizes(&spec.kind).len() > 1 {
                let (sx, sy) = (cx + rw / 2.0 - 9.0, cy + rh / 2.0 - 9.0);
                let a = self.accent;
                self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::solid(sx, sy, 20.0, 20.0, 10.0, a, a).flat() });
                self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::glyph(sx, sy, 11.0, "maximize-2", PURE, 1.0) });
                self.size_hits.push((i, main, [sx - cx, sy - cy]));
            }
            if let Some((key, _)) = cctiles::info(&spec.kind).and_then(|k| k.cycle) {
                let value = spec.opt(key).unwrap_or("").replace('-', " ");
                self.labels.push(Att { tile: main, vis: Vis::Edit, item: Label::new(value, cx + 6.0, cy - rh / 2.0 + 16.0, 10.5, true, PURE, 0.95, Align::Center) });
            }
        }
        self.building = NONE;

        match beside {
            Some(i) if list_only => self.build_list_beside(&layout[i], i, x0, k, rows_h),
            Some(i) => self.build_beside(&layout[i], i, x0, k, rows_h),
            None => {}
        }

        self.edit_btn = self.tile(Kind::Clear, -1000.0, -1000.0, 0.0, 0.0, 0.0);
        self.dd_top = GRID_TOP;
        self.dd_bottom = GRID_TOP + rows_h + SHEET_PAD + 4.0;

        // edit mode only: the free cells to drop a control on, and the "+" on the bar
        let rows = (cctiles::rows(&layout) + 1).min(cctiles::MAX_ROWS);
        for row in 0..rows {
            for col in 0..cctiles::COLS {
                if cctiles::fits(&layout, &CcTile::new("toggle", col, row), None) {
                    let r = CcTile::new("toggle", col, row).rect();
                    self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::solid(x0 + r[0] + r[2] / 2.0, GRID_TOP + r[1] + r[3] / 2.0, r[2], r[3], 20.0, PURE, PURE).flat().alpha(0.06) });
                }
            }
        }
        let a = self.accent;
        self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::solid(pill_x - 40.0, 19.0, 34.0, 26.0, 13.0, a, a).flat() });
        self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::glyph(pill_x - 40.0, 19.0, 15.0, "plus", PURE, 1.0) });
    }

    /// Control `i` opened over the others: it keeps its right edge and its top (its bottom when the bar is at the
    /// bottom) and grows to the left and down into a card with its list (`k` 0 .. 1); the rest stay where they are.
    fn build_beside(&mut self, spec: &CcTile, i: usize, x0: f32, k: f32, rows_h: f32) {
        const CARD_W: f32 = 300.0;
        let full_h = self.grown_height(spec);
        let [rx, ry, rw, rh] = spec.rect();
        let (right, from_y) = (x0 + rx + rw, GRID_TOP + ry);
        let card_w = CARD_W.max(rw);
        let low = self.size[1] - 130.0;
        let to_y = if self.flip { (from_y + rh - full_h).max(2.0 * GRID_TOP + rows_h - low) } else { from_y.min(low - full_h).max(GRID_TOP) };
        let w = rw + (card_w - rw) * k;
        let h = rh + (full_h - rh) * k;
        let y = from_y + (to_y - from_y) * k;
        self.building = i;
        let main = self.fettle_tile(spec, right - w / 2.0, y + h / 2.0, w, h, k);
        self.building = NONE;
        if let Some(slot) = self.spec_tile.get_mut(i) {
            *slot = main;
        }
        self.side_tile = main;
    }

    /// How tall a list shown on its own is: a title over the rows of the grown control.
    fn list_height(&self, spec: &CcTile) -> f32 {
        self.grown_height(spec) - HEAD + LIST_HEAD
    }

    /// The list of control `i` on its own left of the controls (`k` 0 .. 1 how far it is out), as the grow mode says:
    /// a column as tall as the controls, a card level with the control with a tab pointing at it, or the rows straight
    /// on the sheet the controls share with it.
    fn build_list_beside(&mut self, spec: &CcTile, i: usize, x0: f32, k: f32, rows_h: f32) {
        use crate::theme::QuickGrow;
        let list_h = self.list_height(spec);
        let [rx, ry, rw, rh] = spec.rect();
        let low = self.size[1] - 130.0;
        // on the shared sheet the list keeps the gap of the grid; elsewhere it stays clear of the sheet as well
        let right = x0 - LIST_GAP - if self.grow_mode == QuickGrow::Sheet { 0.0 } else { SHEET_PAD };
        let (h, top) = match self.grow_mode {
            QuickGrow::Beside => {
                let from = GRID_TOP + ry;
                let top = if self.flip { (from + rh - list_h).max(2.0 * GRID_TOP + rows_h - low) } else { from.min(low - list_h).max(GRID_TOP) };
                (list_h, top)
            }
            _ => {
                let h = list_h.max(rows_h);
                (h, if self.flip { GRID_TOP + rows_h - h } else { GRID_TOP })
            }
        };
        // it comes out of the control: from its place to its own
        let (fx, fy) = (x0 + rx, GRID_TOP + ry);
        let x = fx + (right - LIST_W - fx) * k;
        let y = fy + (top - fy) * k;
        let w = rw + (LIST_W - rw) * k;
        let hh = rh + (h - rh) * k;
        // the list is in the accent, like a control that is on (on the shared sheet the sheet takes the accent)
        let kind = if self.grow_mode == QuickGrow::Sheet { Kind::Clear } else { Kind::ChipOn };
        self.building = i;
        let t = self.tile(kind, x + w / 2.0, y + hh / 2.0, w, hh, 24.0);
        let a = ((k - 0.45) / 0.55).clamp(0.0, 1.0);
        let name = match list_kind(spec) {
            Some("wifi") => "Wi-Fi",
            Some("bluetooth") => "Bluetooth",
            Some("focus") => "Focus",
            _ => "Sound",
        };
        self.text(t, name, x + 18.0, y + 27.0, 15.0, true, PURE, a, Align::Left);
        // the rows as the grown control lays them out under its head
        let rows_top = y + LIST_HEAD - HEAD;
        match list_kind(spec) {
            Some("wifi") => self.f_network_rows(t, x, rows_top, w, k),
            Some("bluetooth") => self.f_device_rows(t, x, rows_top, w, k),
            Some("focus") => self.f_focus_rows(t, x, rows_top, w, k),
            _ => self.f_mixer(t, x, rows_top, w, k),
        }
        match self.grow_mode {
            QuickGrow::Beside if a > 0.0 => {
                // a tab from the card towards the control
                let cy = (GRID_TOP + ry + rh / 2.0).clamp(y + 20.0, y + hh - 20.0);
                self.tile(Kind::ChipOn, right + 7.0, cy, 14.0 * a, 26.0, 7.0);
            }
            QuickGrow::Sheet if a > 0.0 => {
                // a hairline between the list and the controls
                self.panels.push(Att { tile: FREE, vis: Vis::Always, item: Panel::solid(right + LIST_GAP / 2.0, y + hh / 2.0, 1.0, hh - 24.0, 0.5, PURE, PURE).flat().alpha(0.14 * a) });
            }
            _ => {}
        }
        self.building = NONE;
        self.side_tile = t;
    }

    /// One control of the layout (centred on `cx`, `cy`, `w` x `h`; `grow` 0 .. 1 how far it is grown in place).
    fn fettle_tile(&mut self, spec: &CcTile, cx: f32, cy: f32, w: f32, h: f32, grow: f32) -> usize {
        let (sw, sh) = spec.span();
        let size = if grow > 0.0 || (sw >= 2 && sh >= 2) {
            Size::L
        } else if sw >= 2 {
            Size::M
        } else {
            Size::S
        };
        let size = if grow > 0.0 { Size::M } else { size };
        let opt = |k: &str| spec.opt(k).unwrap_or("").to_string();
        match spec.kind.as_str() {
            "wifi" => self.f_wifi(cx, cy, w, h, size, grow, sw >= 2 && sh >= 2),
            "bluetooth" => self.f_bluetooth(cx, cy, w, h, size, grow, sw >= 2 && sh >= 2),
            // the dark mode switch: a control like the others whose sun turns into the moon (`look = "sky"`: the piece of sky)
            "toggle" if opt("action") == "dark-mode" && opt("look") == "sky" => self.f_sky(cx, cy, w, h, size),
            "toggle" if opt("action") == "dark-mode" => self.f_theme(cx, cy, w, h, size),
            _ if is_focus(spec) => self.f_focus(cx, cy, w, h, size, grow),
            "toggle" | "pill" => {
                let on = spec.kind == "pill" && opt("style") == "active";
                let name = caption(&opt("action"), &opt("icon"), &opt("title"));
                let t = self.f_button(cx, cy, w, h, size, &opt("icon"), &name, &opt("subtitle"), on);
                self.push_action(t, &opt("action"));
                t
            }
            "media" => {
                let t = self.build_media(spec, cx, cy, w, h);
                self.tiles[t].kind = Kind::Chip;
                self.tiles[t].radius = 35.0;
                t
            }
            "slider" => self.f_slider(spec, cx, cy, w, h, grow),
            other => {
                let t = self.tile(Kind::Chip, cx, cy, w, h, (h / 2.0).min(35.0));
                self.text(t, other, cx, cy + 4.5, 11.0, true, PURE, 0.6, Align::Center);
                t
            }
        }
    }

    /// A chip with an icon: small (the icon and a word under it), medium (an icon disc, the name and a state), large
    /// (a big icon, the name and the state at the bottom).
    #[allow(clippy::too_many_arguments)]
    fn f_button(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size, icon: &str, name: &str, state: &str, on: bool) -> usize {
        let t = self.tile(if on { Kind::ChipOn } else { Kind::Chip }, cx, cy, w, h, (h / 2.0).min(35.0));
        let icon = if icon.is_empty() { "sparkles" } else { icon };
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        match size {
            Size::S => {
                self.glyph(t, cx, cy - 8.0, 22.0, icon, PURE, 0.95);
                self.text(t, &short(name, 11), cx, cy + 23.0, 10.5, false, PURE, 0.8, Align::Center);
            }
            Size::M => {
                self.f_disc(t, left + 32.0, cy, icon, on);
                self.f_title(t, left + 62.0, cy, name, state, w - 74.0);
            }
            Size::L => {
                self.f_disc(t, left + 34.0, top + 36.0, icon, on);
                self.text(t, &short(name, 18), left + 16.0, top + h - 34.0, 15.0, true, PURE, 1.0, Align::Left);
                let state = if state.is_empty() { if on { "On" } else { "Tap to run" } } else { state };
                self.text(t, &short(state, 24), left + 16.0, top + h - 16.0, 11.5, false, PURE, 0.7, Align::Left);
            }
        }
        t
    }

    /// The round icon at the start of a medium control: lit with the accent while on.
    fn f_disc(&mut self, t: usize, x: f32, y: f32, icon: &str, on: bool) {
        let fill = if on { [1.0, 1.0, 1.0] } else { self.accent };
        let (top, bottom) = if on { ([1.0, 1.0, 0.999], [0.93, 0.93, 0.95]) } else { (fill, fill) };
        let disc = Panel::solid(x, y, 40.0, 40.0, 20.0, top, bottom).flat().alpha(if on { 0.95 } else { 0.0 });
        self.panels.push(Att { tile: t, vis: Vis::Always, item: disc });
        if !on {
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x, y, 40.0, 40.0, 20.0, PURE, PURE).flat().alpha(0.10) });
        }
        let ink = if on { self.accent } else { PURE };
        self.glyph(t, x, y, 19.0, icon, ink, 1.0);
    }

    /// The name and the state beside a disc, cut to the room there is.
    fn f_title(&mut self, t: usize, x: f32, cy: f32, name: &str, state: &str, room: f32) {
        let chars = (room / 6.2).max(4.0) as usize;
        if state.is_empty() {
            self.text(t, &short(name, chars), x, cy + 4.5, 13.0, true, PURE, 1.0, Align::Left);
        } else {
            self.text(t, &short(name, chars), x, cy - 2.0, 13.0, true, PURE, 1.0, Align::Left);
            self.text(t, &short(state, chars + 2), x, cy + 14.0, 11.0, false, PURE, 0.72, Align::Left);
        }
    }

    /// Four bars of signal strength, the lit ones by `signal` (0 .. 100), bottom-left corner at `x`, `y`.
    fn f_bars(&mut self, t: usize, x: f32, y: f32, signal: u8, scale: f32, alpha: f32) {
        let lit = match signal {
            0..=19 => 1,
            20..=44 => 2,
            45..=69 => 3,
            _ => 4,
        };
        for b in 0..4 {
            let bh = (4.0 + b as f32 * 3.0) * scale;
            let bx = x + b as f32 * 4.5 * scale;
            let a = if b < lit { 0.95 } else { 0.25 } * alpha;
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(bx + 1.5 * scale, y - bh / 2.0, 3.0 * scale, bh, 1.5 * scale, PURE, PURE).flat().alpha(a) });
        }
    }

    /// A small battery: an outline and a fill by `pct`, its middle at `x`, `y`.
    fn f_battery(&mut self, t: usize, x: f32, y: f32, pct: u8, alpha: f32) {
        let (w, h) = (20.0, 10.0);
        self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x, y, w, h, 3.0, PURE, PURE).flat().alpha(0.35 * alpha) });
        let f = (w - 4.0) * (pct.min(100) as f32 / 100.0);
        let c = if pct <= 15 { [1.0, 0.36, 0.32] } else { [0.45, 0.92, 0.55] };
        if f > 0.5 {
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x - (w - 4.0) / 2.0 + f / 2.0, y, f, h - 4.0, 1.5, c, c).flat().alpha(alpha) });
        }
        self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x + w / 2.0 + 1.5, y, 2.0, 4.0, 1.0, PURE, PURE).flat().alpha(0.35 * alpha) });
    }

    /// The chevron at the end of a medium control: it grows the control (and shrinks it back).
    fn f_chevron(&mut self, cx: f32, cy: f32, grown: bool) {
        let spec = self.building;
        let hit = self.tile(Kind::Clear, cx, cy, 40.0, 56.0, 0.0);
        let icon = if self.list_beside == Some(spec) {
            "chevron-left"
        } else if grown {
            "chevron-down"
        } else {
            "chevron-right"
        };
        self.glyph(hit, cx, cy, 16.0, icon, PURE, 0.7);
        self.acts.push((hit, TileAct::Expand(spec)));
    }

    /// Focus (do not disturb): a tap turns it on or off, held (or its chevron) it shows for how long.
    fn f_focus(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size, grow: f32) -> usize {
        let on = self.focus_on;
        let state = if on { self.focus_until.clone().unwrap_or_else(|| "On".into()) } else { "Off".into() };
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let t = self.tile(if on { Kind::ChipOn } else { Kind::Chip }, cx, cy, w, h, (h / 2.0).min(35.0));
        self.acts.push((t, TileAct::Focus));
        match size {
            Size::S => {
                self.glyph(t, cx, cy - 8.0, 22.0, "moon", PURE, 0.95);
                self.text(t, "Focus", cx, cy + 23.0, 10.5, false, PURE, 0.8, Align::Center);
            }
            Size::M | Size::L => {
                let hy = if grow > 0.0 || size == Size::L { top + 35.0 } else { cy };
                self.f_disc(t, left + 32.0, hy, "moon", on);
                self.f_title(t, left + 62.0, hy, "Focus", &state, w - 96.0);
                self.f_chevron(left + w - 20.0, hy, grow > 0.5);
            }
        }
        if grow > 0.0 {
            self.f_focus_rows(t, left, top, w, grow);
        }
        t
    }

    /// Focus's choices: for an hour, until the evening, until the morning, until turned off (and off, while on).
    fn f_focus_rows(&mut self, t: usize, left: f32, top: f32, w: f32, grow: f32) {
        let a = ((grow - 0.45) / 0.55).clamp(0.0, 1.0);
        if a <= 0.02 {
            return;
        }
        let spec = self.building;
        let mut y = top + HEAD;
        let off = self.focus_on.then_some(("Turn off", "bell", FocusFor::Off));
        for (label, icon, what) in off.into_iter().chain(FOCUS_ROWS) {
            let rc = y + ROW_H / 2.0;
            let row = self.tile(Kind::Clear, left + w / 2.0, rc, w - 12.0, ROW_H - 4.0, 12.0);
            self.owner[row] = spec;
            self.glyph(row, left + 26.0, rc, 14.0, icon, PURE, 0.8 * a);
            self.text(row, label, left + 46.0, rc + 4.5, 12.5, what == FocusFor::Off, PURE, a, Align::Left);
            self.acts.push((row, TileAct::Row(RowAct::Focus(what))));
            y += ROW_H;
        }
        self.f_more(t, left, y, w, "notifications", "Notification settings…", a);
    }

    #[allow(clippy::too_many_arguments)]
    fn f_wifi(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size, grow: f32, large: bool) -> usize {
        let wifi = self.sys.wifi.clone();
        let on = wifi.as_ref().is_some_and(|x| x.enabled);
        let ssid = wifi.as_ref().and_then(|x| x.ssid.clone()).filter(|_| on);
        let nets = self.lists.networks.clone();
        let signal = nets.as_ref().and_then(|l| l.iter().find(|n| n.active)).map(|n| n.signal);
        let state = match (&wifi, &ssid) {
            (None, _) => "Not available".to_string(),
            (Some(_), _) if !on => "Off".to_string(),
            (_, Some(s)) => s.clone(),
            _ => "Not connected".to_string(),
        };
        let icon = if on { "wifi" } else { "wifi-off" };
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let t = self.tile(if on { Kind::ChipOn } else { Kind::Chip }, cx, cy, w, h, (h / 2.0).min(35.0));
        self.acts.push((t, TileAct::Wifi));
        match size {
            Size::S => {
                match signal {
                    Some(sig) => self.f_bars(t, cx - 9.0, cy - 2.0, sig, 1.2, 1.0),
                    None => self.glyph(t, cx, cy - 8.0, 22.0, icon, PURE, 0.95),
                }
                self.text(t, &short(if on { ssid.as_deref().unwrap_or("Wi-Fi") } else { "Wi-Fi" }, 10), cx, cy + 23.0, 10.5, false, PURE, 0.8, Align::Center);
            }
            Size::M | Size::L => {
                let hy = if grow > 0.0 || size == Size::L { top + 35.0 } else { cy };
                self.f_disc(t, left + 32.0, hy, icon, on);
                let bars = signal.filter(|_| w > 200.0);
                self.f_title(t, left + 62.0, hy, "Wi-Fi", &state, w - 96.0 - if bars.is_some() { 26.0 } else { 0.0 });
                if let Some(sig) = bars {
                    self.f_bars(t, left + w - 64.0, hy + 4.0, sig, 1.0, 1.0);
                }
                if on {
                    self.f_chevron(left + w - 20.0, hy, grow > 0.5);
                }
            }
        }
        // large: the strongest networks in reach under the header
        if large && grow == 0.0 && on {
            let shown: Vec<CcNetwork> = nets.unwrap_or_default().into_iter().filter(|n| !n.active).take(3).collect();
            for (j, n) in shown.iter().enumerate() {
                let y = top + 82.0 + j as f32 * 22.0;
                self.f_bars(t, left + 16.0, y + 3.0, n.signal, 0.8, 0.8);
                self.text(t, &short(&n.ssid, 18), left + 40.0, y + 4.0, 11.5, false, PURE, 0.8, Align::Left);
            }
        }
        if grow > 0.0 {
            self.f_network_rows(t, left, top, w, grow);
        }
        t
    }

    /// The networks of a grown Wi-Fi control, faded in as it grows.
    fn f_network_rows(&mut self, t: usize, left: f32, top: f32, w: f32, grow: f32) {
        let a = ((grow - 0.45) / 0.55).clamp(0.0, 1.0);
        if a <= 0.02 {
            return;
        }
        let spec = self.building;
        let rows = self.layout.get(spec).map_or(MAX_NETS, |t| self.list_rows(t).0);
        let mut y = top + HEAD;
        match self.lists.networks.clone() {
            None => {
                self.text(t, "Looking for networks…", left + 18.0, y + ROW_H / 2.0 + 4.0, 12.0, false, PURE, 0.6 * a, Align::Left);
                y += ROW_H;
            }
            Some(list) if list.is_empty() => {
                self.text(t, "No networks in reach", left + 18.0, y + ROW_H / 2.0 + 4.0, 12.0, false, PURE, 0.6 * a, Align::Left);
                y += ROW_H;
            }
            Some(list) => {
                for n in list.iter().take(rows) {
                    let rc = y + ROW_H / 2.0;
                    let row = self.tile(Kind::Clear, left + w / 2.0, rc, w - 12.0, ROW_H - 4.0, 12.0);
                    self.owner[row] = spec;
                    if n.active {
                        self.panels.push(Att { tile: row, vis: Vis::Always, item: Panel::solid(left + w / 2.0, rc, w - 16.0, ROW_H - 6.0, 12.0, PURE, PURE).flat().alpha(0.14 * a) });
                    }
                    self.f_bars(row, left + 18.0, rc + 4.0, n.signal, 0.9, a);
                    self.text(row, &short(&n.ssid, 24), left + 46.0, rc + 4.5, 12.5, n.active, PURE, a, Align::Left);
                    let tag = if n.active { "Connected" } else if n.saved { "Saved" } else { "" };
                    if !tag.is_empty() {
                        self.text(row, tag, left + w - 40.0, rc + 4.0, 10.5, false, PURE, 0.6 * a, Align::Right);
                    }
                    if n.secure {
                        self.glyph(row, left + w - 22.0, rc, 12.0, "lock", PURE, 0.55 * a);
                    }
                    if !n.active {
                        self.acts.push((row, TileAct::Row(RowAct::Wifi(n.ssid.clone(), n.secure && !n.saved))));
                    }
                    y += ROW_H;
                }
            }
        }
        self.f_more(t, left, y, w, "network", "Network settings…", a);
    }

    #[allow(clippy::too_many_arguments)]
    fn f_bluetooth(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size, grow: f32, large: bool) -> usize {
        let bt = self.sys.bluetooth.clone();
        let on = bt.as_ref().is_some_and(|b| b.powered);
        let devs = self.lists.devices.clone().unwrap_or_default();
        let linked: Vec<&CcDevice> = devs.iter().filter(|d| d.connected).collect();
        let first = linked.first().copied();
        let state = match (&bt, first) {
            (None, _) => "Not available".to_string(),
            (Some(_), _) if !on => "Off".to_string(),
            (_, Some(d)) => match d.battery {
                Some(p) => format!("{} · {p}%", d.name),
                None => d.name.clone(),
            },
            (Some(b), None) if b.connected > 0 => format!("{} connected", b.connected),
            _ => "On".to_string(),
        };
        let icon = if !on { "bluetooth-off" } else { first.map_or("bluetooth", |d| device_icon(&d.icon)) };
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let t = self.tile(if on { Kind::ChipOn } else { Kind::Chip }, cx, cy, w, h, (h / 2.0).min(35.0));
        self.acts.push((t, TileAct::Bluetooth));
        match size {
            Size::S => {
                self.glyph(t, cx, cy - 9.0, 22.0, icon, PURE, 0.95);
                match first.and_then(|d| d.battery) {
                    Some(p) => {
                        self.f_battery(t, cx - 12.0, cy + 21.0, p, 1.0);
                        self.text(t, &format!("{p}%"), cx + 4.0, cy + 25.0, 10.5, true, PURE, 0.9, Align::Left);
                    }
                    None => self.text(t, "Bluetooth", cx, cy + 23.0, 10.5, false, PURE, 0.8, Align::Center),
                }
            }
            Size::M | Size::L => {
                let hy = if grow > 0.0 || size == Size::L { top + 35.0 } else { cy };
                self.f_disc(t, left + 32.0, hy, icon, on);
                self.f_title(t, left + 62.0, hy, "Bluetooth", &state, w - 96.0);
                if on {
                    self.f_chevron(left + w - 20.0, hy, grow > 0.5);
                }
            }
        }
        // large: each connected device with its battery
        if large && grow == 0.0 && on {
            for (j, d) in linked.iter().take(3).enumerate() {
                let y = top + 82.0 + j as f32 * 22.0;
                self.glyph(t, left + 24.0, y, 13.0, device_icon(&d.icon), PURE, 0.8);
                self.text(t, &short(&d.name, 14), left + 40.0, y + 4.0, 11.5, false, PURE, 0.85, Align::Left);
                if let Some(p) = d.battery {
                    self.f_battery(t, left + w - 30.0, y, p, 0.9);
                }
            }
            if linked.is_empty() {
                self.text(t, "Nothing connected", left + 16.0, top + 86.0, 11.5, false, PURE, 0.6, Align::Left);
            }
        }
        if grow > 0.0 {
            self.f_device_rows(t, left, top, w, grow);
        }
        t
    }

    /// The paired devices of a grown Bluetooth control.
    fn f_device_rows(&mut self, t: usize, left: f32, top: f32, w: f32, grow: f32) {
        let a = ((grow - 0.45) / 0.55).clamp(0.0, 1.0);
        if a <= 0.02 {
            return;
        }
        let spec = self.building;
        let rows = self.layout.get(spec).map_or(MAX_DEVS, |t| self.list_rows(t).0);
        let mut y = top + HEAD;
        let paired: Option<Vec<CcDevice>> = self.lists.devices.clone().map(|l| l.into_iter().filter(|d| d.paired).collect());
        match paired {
            None => {
                self.text(t, "Looking for devices…", left + 18.0, y + ROW_H / 2.0 + 4.0, 12.0, false, PURE, 0.6 * a, Align::Left);
                y += ROW_H;
            }
            Some(list) if list.is_empty() => {
                self.text(t, "No paired devices", left + 18.0, y + ROW_H / 2.0 + 4.0, 12.0, false, PURE, 0.6 * a, Align::Left);
                y += ROW_H;
            }
            Some(list) => {
                for d in list.iter().take(rows) {
                    let rc = y + ROW_H / 2.0;
                    let row = self.tile(Kind::Clear, left + w / 2.0, rc, w - 12.0, ROW_H - 4.0, 12.0);
                    self.owner[row] = spec;
                    if d.connected {
                        self.panels.push(Att { tile: row, vis: Vis::Always, item: Panel::solid(left + w / 2.0, rc, w - 16.0, ROW_H - 6.0, 12.0, PURE, PURE).flat().alpha(0.14 * a) });
                    }
                    self.glyph(row, left + 26.0, rc, 15.0, device_icon(&d.icon), PURE, 0.9 * a);
                    self.text(row, &short(&d.name, 22), left + 46.0, rc + 4.5, 12.5, d.connected, PURE, a, Align::Left);
                    match (d.connected, d.battery) {
                        (true, Some(p)) => {
                            self.f_battery(row, left + w - 70.0, rc, p, a);
                            self.text(row, &format!("{p}%"), left + w - 18.0, rc + 4.0, 10.5, false, PURE, 0.7 * a, Align::Right);
                        }
                        (true, None) => self.text(row, "Connected", left + w - 18.0, rc + 4.0, 10.5, false, PURE, 0.6 * a, Align::Right),
                        _ => self.text(row, "Connect", left + w - 18.0, rc + 4.0, 10.5, false, PURE, 0.45 * a, Align::Right),
                    }
                    self.acts.push((row, TileAct::Row(RowAct::Device(d.mac.clone(), d.connected))));
                    y += ROW_H;
                }
            }
        }
        self.f_more(t, left, y, w, "bluetooth", "Bluetooth settings…", a);
    }

    /// The last line of a grown list: the full card, for what the list cannot do.
    #[allow(clippy::too_many_arguments)]
    fn f_more(&mut self, t: usize, left: f32, y: f32, w: f32, kind: &'static str, text: &str, a: f32) {
        let _ = t;
        let row = self.tile(Kind::Clear, left + w / 2.0, y + FOOT / 2.0, w - 12.0, FOOT - 6.0, 12.0);
        self.owner[row] = self.building;
        self.panels.push(Att { tile: row, vis: Vis::Always, item: Panel::solid(left + w / 2.0, y + 1.0, w - 32.0, 1.0, 0.5, PURE, PURE).flat().alpha(0.12 * a) });
        self.text(row, text, left + 18.0, y + FOOT / 2.0 + 4.0, 12.0, false, PURE, 0.75 * a, Align::Left);
        self.glyph(row, left + w - 22.0, y + FOOT / 2.0, 13.0, "chevron-right", PURE, 0.5 * a);
        self.acts.push((row, TileAct::Row(RowAct::More(kind))));
    }

    /// The dark mode switch as a control like the others: its sun shrinks away as the moon grows in its place while
    /// the shell goes dark (and back), following the same spring as the rest of the dark mode.
    fn f_theme(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size) -> usize {
        let n = self.dark_k.x.clamp(0.0, 1.0);
        let t = self.tile(Kind::Chip, cx, cy, w, h, (h / 2.0).min(35.0));
        self.push_action(t, "dark-mode");
        let name = if self.dark { "Dark" } else { "Light" };
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        match size {
            Size::S => {
                self.f_sun_moon(t, cx, cy - 8.0, 22.0, n, PURE);
                self.text(t, name, cx, cy + 23.0, 10.5, false, PURE, 0.8, Align::Center);
            }
            Size::M => {
                let (x, y) = (left + 32.0, cy);
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x, y, 40.0, 40.0, 20.0, PURE, PURE).flat().alpha(0.10) });
                self.f_sun_moon(t, x, y, 19.0, n, PURE);
                self.f_title(t, left + 62.0, cy, "Appearance", name, w - 74.0);
            }
            Size::L => {
                let (x, y) = (left + 34.0, top + 36.0);
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x, y, 40.0, 40.0, 20.0, PURE, PURE).flat().alpha(0.10) });
                self.f_sun_moon(t, x, y, 19.0, n, PURE);
                self.text(t, "Appearance", left + 16.0, top + h - 34.0, 15.0, true, PURE, 1.0, Align::Left);
                self.text(t, name, left + 16.0, top + h - 16.0, 11.5, false, PURE, 0.7, Align::Left);
            }
        }
        t
    }

    /// A sun at `n` = 0 turning into a moon at 1: one shrinks and fades as the other grows in its place.
    #[allow(clippy::too_many_arguments)]
    fn f_sun_moon(&mut self, t: usize, x: f32, y: f32, size: f32, n: f32, ink: [f32; 3]) {
        let e = n * n * (3.0 - 2.0 * n);
        if e < 0.999 {
            self.glyph(t, x, y, size * (1.0 - 0.5 * e), "sun", ink, 1.0 - e);
        }
        if e > 0.001 {
            self.glyph(t, x, y, size * (0.5 + 0.5 * e), "moon", ink, e);
        }
    }

    /// The dark mode control as a piece of sky: by day blue with a sun and its rays, by night deep blue with a moon
    /// and a few stars; the sun turns into the moon (a disc of night slides over it) as the shell goes dark.
    fn f_sky(&mut self, cx: f32, cy: f32, w: f32, h: f32, size: Size) -> usize {
        let n = self.dark_k.x.clamp(0.0, 1.0);
        let t = self.tile(Kind::Paint, cx, cy, w, h, (h / 2.0).min(35.0));
        let sky = mix3(DAY, NIGHT, n);
        self.tiles[t].color = sky;
        self.push_action(t, "dark-mode");
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let r = if size == Size::L { 17.0 } else { 11.0 };
        let (sx, sy) = match size {
            Size::S => (cx, cy - 8.0),
            Size::M => (left + 32.0, cy),
            Size::L => (left + 36.0, top + 38.0),
        };
        // stars come out
        if n > 0.05 {
            for (dx, dy, s) in [(0.62, 0.22, 2.2), (0.80, 0.55, 1.6), (0.40, 0.70, 1.4), (0.88, 0.18, 1.2)] {
                let (x, y) = (left + w * dx, top + h * dy);
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(x, y, s, s, s / 2.0, PURE, PURE).flat().alpha(0.85 * n) });
            }
        }
        // the sun's rays draw in as it sets
        let rays = 1.0 - n;
        if rays > 0.02 {
            for i in 0..8 {
                let ang = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = r + 4.0 + 2.5 * rays;
                let s = 3.2 * rays;
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(sx + d * ang.cos(), sy + d * ang.sin(), s, s, s / 2.0, SUN, SUN).flat().alpha(rays) });
            }
        }
        // the disc: the sun's yellow pales into the moon's
        let disc = mix3(SUN, MOON, n);
        self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(sx, sy, 2.0 * r, 2.0 * r, r, disc, disc).flat() });
        // night slides over it from the top right and leaves a crescent
        let off = r * (2.4 - 1.65 * n);
        let (ox, oy) = (sx + off * 0.62, sy - off * 0.62);
        let cover = 0.92 * r;
        self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(ox, oy, 2.0 * cover, 2.0 * cover, cover, sky, sky).flat() });
        let word = if self.dark { "Dark" } else { "Light" };
        match size {
            Size::S => self.text(t, word, cx, cy + 23.0, 10.5, false, PURE, 0.9, Align::Center),
            Size::M => self.f_title(t, left + 62.0, cy, "Appearance", word, w - 74.0),
            Size::L => {
                self.text(t, "Appearance", left + 16.0, top + h - 34.0, 15.0, true, PURE, 1.0, Align::Left);
                self.text(t, word, left + 16.0, top + h - 16.0, 11.5, false, PURE, 0.75, Align::Left);
            }
        }
        t
    }

    /// A liquid slider: the whole control is the tube; the liquid's surface leans and swings back as the level moves.
    /// Grown (the sound), it shows the outputs and every app that plays.
    fn f_slider(&mut self, spec: &CcTile, cx: f32, cy: f32, w: f32, h: f32, grow: f32) -> usize {
        let sound = spec.opt("target") == Some("sound");
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let t = self.tile(Kind::Chip, cx, cy, w, h, 24.0);
        // the tube is the control's top 70 px (all of it, until it grows)
        let tube_y = top + 35.0;
        let tube = self.tile(Kind::Clear, cx, tube_y, w, 70.0, 24.0);
        self.owner[tube] = self.building;
        let which = if sound { Slider::Sound } else { Slider::Display };
        let liq = self.liquid[if sound { 1 } else { 0 }];
        let muted = sound && self.sys.muted;
        let value = if sound { self.sys.volume.map(|v| v.min(1.0)) } else { self.sys.brightness };
        let level = if liq.placed { liq.level } else { value.unwrap_or(0.5) };
        // the liquid fills the control from edge to edge, in its own shape
        self.f_liquid(t, left, tube_y, w, 70.0, 24.0, level, liq.tilt, if muted { 0.35 } else { 1.0 });
        // a white badge with the icon in the accent says what it is (no name: the icon is enough)
        let icon = if sound {
            if muted {
                "volume-x"
            } else if level < 0.34 {
                "volume-1"
            } else {
                "volume-2"
            }
        } else {
            "sun"
        };
        let (bx, bs) = (left + 35.0, 46.0);
        self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(bx, tube_y, bs, bs, 14.0, [1.0, 1.0, 0.999], [0.93, 0.93, 0.95]).flat().alpha(0.96) });
        self.glyph(t, bx, tube_y, 22.0, icon, self.accent, 1.0);
        let pct = match value {
            Some(v) => format!("{}%", (v * 100.0).round() as i32),
            None => "—".into(),
        };
        self.text(t, &pct, left + w - if sound { 48.0 } else { 20.0 }, tube_y + 4.5, 12.5, true, PURE, 0.9, Align::Right);
        self.acts.push((tube, TileAct::Slider(which)));
        self.tracks.push((tube, (left + 10.0, left + w - 10.0)));
        if sound {
            // the speaker on the left mutes; the chevron grows the mixer
            let mute = self.tile(Kind::Clear, bx, tube_y, 52.0, 56.0, 0.0);
            self.owner[mute] = self.building;
            self.acts.push((mute, TileAct::Mute));
            self.f_chevron(left + w - 20.0, tube_y, grow > 0.5);
        }
        if grow > 0.0 && sound {
            self.f_mixer(t, left, top, w, grow);
        }
        t
    }

    /// Liquid in a tube (`x`, `cy`, `w` x `h`, squircle corners of `radius` like the control around it), drawn by the
    /// shader in one piece (`Panel::liquid`): a straight surface that leans by `tilt` px at the top (the other way at the
    /// bottom) while it moves.
    #[allow(clippy::too_many_arguments)]
    fn f_liquid(&mut self, t: usize, x: f32, cy: f32, w: f32, h: f32, radius: f32, level: f32, tilt: f32, alpha: f32) {
        let a = self.accent;
        let (hi, lo) = ([(a[0] * 1.12 + 0.04).min(1.0), (a[1] * 1.12 + 0.04).min(1.0), (a[2] * 1.12 + 0.04).min(1.0)], [a[0] * 0.82, a[1] * 0.82, a[2] * 0.82]);
        let level = level.clamp(0.0, 1.0);
        if w * level < 2.0 {
            return;
        }
        let (hw, hh) = (w / 2.0, h / 2.0);
        // the surface: a straight edge where the level is (as a share of the half width), leaning while it moves (a share
        // of the half height)
        let surf = w * level / hw - 1.0;
        let item = Panel::liquid(x + hw, cy, w, h, -radius, hi, lo, alpha, surf, tilt / hh, 0.0, 0.20);
        self.panels.push(Att { tile: t, vis: Vis::Always, item });
    }

    /// The grown sound control: the outputs (the one in use marked) and every app that plays, each with its own liquid.
    fn f_mixer(&mut self, t: usize, left: f32, top: f32, w: f32, grow: f32) {
        let a = ((grow - 0.45) / 0.55).clamp(0.0, 1.0);
        if a <= 0.02 {
            return;
        }
        let spec = self.building;
        let (outs_n, apps_n) = self.layout.get(spec).map_or((MAX_OUTS, MAX_APPS), |t| self.list_rows(t));
        let mut y = top + HEAD;
        self.text(t, "OUTPUT", left + 18.0, y + 16.0, 10.5, true, PURE, 0.55 * a, Align::Left);
        y += 26.0;
        let outs = self.lists.outputs.clone();
        if outs.is_empty() {
            self.text(t, "Looking for outputs…", left + 18.0, y + ROW_H / 2.0 + 4.0, 12.0, false, PURE, 0.6 * a, Align::Left);
            y += ROW_H;
        }
        for o in outs.iter().take(outs_n) {
            let rc = y + ROW_H / 2.0;
            let row = self.tile(Kind::Clear, left + w / 2.0, rc, w - 12.0, ROW_H - 4.0, 12.0);
            self.owner[row] = spec;
            if o.default {
                self.panels.push(Att { tile: row, vis: Vis::Always, item: Panel::solid(left + w / 2.0, rc, w - 16.0, ROW_H - 6.0, 12.0, PURE, PURE).flat().alpha(0.14 * a) });
                self.glyph(row, left + w - 24.0, rc, 14.0, "check", PURE, 0.9 * a);
            }
            let icon = if o.name.to_lowercase().contains("head") { "headphones" } else { "volume-2" };
            self.glyph(row, left + 26.0, rc, 15.0, icon, PURE, 0.85 * a);
            self.text(row, &short(&o.name, 30), left + 46.0, rc + 4.5, 12.0, o.default, PURE, a, Align::Left);
            self.acts.push((row, TileAct::Row(RowAct::Output(o.id))));
            y += ROW_H;
        }
        let apps = self.lists.streams.clone();
        if !apps.is_empty() && apps_n > 0 {
            self.text(t, "APPS", left + 18.0, y + 16.0, 10.5, true, PURE, 0.55 * a, Align::Left);
            y += 26.0;
            for s in apps.iter().take(apps_n) {
                let rc = y + 23.0;
                let tube = self.tile(Kind::Clear, left + w / 2.0, rc, w - 24.0, 38.0, 14.0);
                self.owner[tube] = spec;
                self.panels.push(Att { tile: tube, vis: Vis::Always, item: Panel::solid(left + w / 2.0, rc, w - 24.0, 38.0, 14.0, PURE, PURE).flat().alpha(0.07 * a) });
                self.f_liquid(tube, left + 15.0, rc, w - 30.0, 32.0, 11.0, s.vol.min(1.0), 0.0, if s.muted { 0.35 * a } else { 0.9 * a });
                self.text(tube, &short(&s.name, 28), left + 26.0, rc + 4.5, 12.0, true, PURE, a, Align::Left);
                self.text(tube, &format!("{}%", (s.vol.min(1.0) * 100.0).round() as i32), left + w - 26.0, rc + 4.5, 11.5, false, PURE, 0.85 * a, Align::Right);
                self.acts.push((tube, TileAct::Slider(Slider::Node(s.id))));
                self.tracks.push((tube, (left + 14.0, left + w - 14.0)));
                y += 46.0;
            }
        }
        self.f_more(t, left, y, w, "audio", "Sound settings…", a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysctl::{Bluetooth, Wifi};

    fn fettle() -> ControlCenter {
        let mut cc = ControlCenter::new(1600.0, 1200.0);
        cc.set_style(CcStyle::Fettle, false, [0.9, 0.3, 0.6]);
        cc.set_sys(&SysInfo { volume: Some(0.5), brightness: Some(0.4), wifi: Some(Wifi { enabled: true, ssid: Some("Home".into()) }), bluetooth: Some(Bluetooth { powered: true, connected: 1 }), ..SysInfo::default() });
        cc.set_open(true);
        cc
    }

    fn settle(cc: &mut ControlCenter, frames: usize) {
        for _ in 0..frames {
            cc.update(0.016, &Default::default(), [0.0, 0.0], false, false);
        }
    }

    fn spec_of(cc: &ControlCenter, kind: &str) -> usize {
        cc.layout.iter().position(|t| t.kind == kind).unwrap()
    }

    fn centre(cc: &ControlCenter, spec: usize) -> [f32; 2] {
        cc.tiles[cc.spec_tile[spec]].c
    }

    #[test]
    fn controls_sit_on_one_sheet_and_show_their_state() {
        let mut cc = fettle();
        cc.set_lists(&CcLists { devices: Some(vec![CcDevice { mac: "AA".into(), name: "Buds".into(), connected: true, paired: true, icon: "audio-headset".into(), battery: Some(80) }]), ..CcLists::default() });
        assert_ne!(cc.sheet_tile, FREE);
        assert!(cc.tiles[cc.sheet_tile].kind == Kind::Sheet);
        // the sheet is under every control
        let s = &cc.tiles[cc.sheet_tile];
        for &t in &cc.spec_tile {
            let c = cc.tiles[t].c;
            assert!((c[0] - s.c[0]).abs() < s.size[0] / 2.0 && (c[1] - s.c[1]).abs() < s.size[1] / 2.0);
        }
        // a switched-on control takes the accent; the headphones and their battery are named
        assert!(cc.tiles[cc.spec_tile[spec_of(&cc, "wifi")]].kind == Kind::ChipOn);
        assert!(cc.labels.iter().any(|l| l.item.text.contains("Buds · 80%")), "{:?}", cc.labels.iter().map(|l| l.item.text.clone()).collect::<Vec<_>>());
        assert!(cc.labels.iter().any(|l| l.item.text == "Home"));
    }

    #[test]
    fn a_tap_switches_and_a_hold_grows_in_place() {
        let mut cc = fettle();
        let wifi = spec_of(&cc, "wifi");
        // the left part of the control (the chevron on its right grows it)
        let at = { let c = centre(&cc, wifi); [c[0] - 40.0, c[1]] };
        cc.mouse_down(at);
        cc.update(0.016, &Default::default(), at, false, false);
        assert_eq!(cc.take_events(), vec![CcEvent::SetWifi(false)]);
        assert!(cc.expanded.is_none());
        cc.mouse_down(at);
        for _ in 0..40 {
            cc.update(0.016, &Default::default(), at, true, false);
        }
        cc.update(0.016, &Default::default(), at, false, false);
        assert_eq!(cc.expanded, Some(wifi), "held: it grows (not the edit mode)");
        assert!(!cc.edit);
        assert!(cc.take_events().is_empty());
    }

    #[test]
    fn a_grown_list_pushes_the_others_down_and_its_rows_act() {
        let mut cc = fettle();
        cc.set_grow(crate::theme::QuickGrow::InPlace);
        let nets = vec![CcNetwork { ssid: "Home".into(), signal: 80, secure: true, active: true, saved: true }, CcNetwork { ssid: "Cafe".into(), signal: 50, secure: false, active: false, saved: false }, CcNetwork { ssid: "Lab".into(), signal: 30, secure: true, active: false, saved: false }];
        cc.set_lists(&CcLists { networks: Some(nets), ..CcLists::default() });
        let (wifi, bt) = (spec_of(&cc, "wifi"), spec_of(&cc, "bluetooth"));
        let before = centre(&cc, bt);
        assert!(cc.open_grown("wifi"));
        settle(&mut cc, 120);
        let after = centre(&cc, bt);
        assert!(after[1] > before[1] + 100.0, "the control under it moved down: {before:?} -> {after:?}");
        // the grown control spans the sheet's width
        assert!((cc.tiles[cc.spec_tile[wifi]].size[0] - GRID_W).abs() < 1.0);
        // an open network joins; a locked unknown one opens the full card
        let row = |cc: &ControlCenter, want: &RowAct| cc.acts.iter().find(|(_, a)| *a == TileAct::Row(want.clone())).map(|(t, _)| cc.tiles[*t].c).unwrap();
        let cafe = row(&cc, &RowAct::Wifi("Cafe".into(), false));
        cc.mouse_down(cafe);
        assert_eq!(cc.take_events(), vec![CcEvent::WifiConnect("Cafe".into())]);
        let lab = row(&cc, &RowAct::Wifi("Lab".into(), true));
        cc.mouse_down(lab);
        assert_eq!(cc.take_events(), vec![CcEvent::Card("network".into())]);
        assert!(!cc.open);
    }

    #[test]
    fn a_control_opens_over_the_others_to_the_left() {
        let mut cc = fettle();
        let nets = vec![CcNetwork { ssid: "Home".into(), signal: 80, secure: true, active: true, saved: true }, CcNetwork { ssid: "Cafe".into(), signal: 50, secure: false, active: false, saved: false }];
        cc.set_lists(&CcLists { networks: Some(nets), ..CcLists::default() });
        let (wifi, bt) = (spec_of(&cc, "wifi"), spec_of(&cc, "bluetooth"));
        let bt_before = centre(&cc, bt);
        let w0 = &cc.tiles[cc.spec_tile[wifi]];
        let (w0_right, w0_top) = (w0.c[0] + w0.size[0] / 2.0, w0.c[1] - w0.size[1] / 2.0);
        assert!(cc.open_grown("wifi"));
        settle(&mut cc, 120);
        assert_eq!(centre(&cc, bt), bt_before, "nothing else moved");
        // the control itself grew to the left and down: same right edge and top, drawn over the others
        let card = &cc.tiles[cc.spec_tile[wifi]];
        assert_eq!(cc.side_tile, cc.spec_tile[wifi]);
        assert!((card.c[0] + card.size[0] / 2.0 - w0_right).abs() < 0.5);
        assert!((card.c[1] - card.size[1] / 2.0 - w0_top).abs() < 0.5);
        let sheet = &cc.tiles[cc.sheet_tile];
        assert!(card.c[0] - card.size[0] / 2.0 < sheet.c[0] - sheet.size[0] / 2.0, "it reaches out past the sheet");
        // its rows act; a press elsewhere shrinks it back
        let cafe = cc.acts.iter().find(|(_, a)| *a == TileAct::Row(RowAct::Wifi("Cafe".into(), false))).map(|(t, _)| cc.tiles[*t].c).unwrap();
        cc.mouse_down(cafe);
        assert_eq!(cc.take_events(), vec![CcEvent::WifiConnect("Cafe".into())]);
        assert_eq!(cc.expanded, Some(wifi));
        let files = cc.layout.iter().position(|t| t.opt("icon") == Some("folder")).or_else(|| cc.layout.iter().position(|t| t.kind == "slider")).unwrap();
        let c = centre(&cc, files);
        cc.mouse_down(c);
        assert_eq!(cc.expanded, None);
    }

    #[test]
    fn a_list_alone_comes_out_beside_the_controls_in_every_mode() {
        use crate::theme::QuickGrow;
        for mode in [QuickGrow::Column, QuickGrow::Beside, QuickGrow::Sheet] {
            let mut cc = fettle();
            cc.set_grow(mode);
            let nets = vec![CcNetwork { ssid: "Cafe".into(), signal: 50, secure: false, active: false, saved: false }];
            cc.set_lists(&CcLists { networks: Some(nets), ..CcLists::default() });
            let wifi = spec_of(&cc, "wifi");
            let before = centre(&cc, wifi);
            assert!(cc.open_grown("wifi"));
            settle(&mut cc, 120);
            assert_eq!(centre(&cc, wifi), before, "{mode:?}: the control stays in the grid");
            assert_eq!(cc.list_beside, Some(wifi));
            let side = &cc.tiles[cc.side_tile];
            let grid_left = cc.tiles[cc.spec_tile[wifi]].c[0] - cc.tiles[cc.spec_tile[wifi]].size[0] / 2.0;
            assert!(side.c[0] + side.size[0] / 2.0 < grid_left, "{mode:?}: the list is left of the controls");
            let sheet = &cc.tiles[cc.sheet_tile];
            let sheet_left = sheet.c[0] - sheet.size[0] / 2.0;
            assert_eq!(sheet_left < side.c[0] - side.size[0] / 2.0, mode == QuickGrow::Sheet, "{mode:?}: only one sheet takes the list in");
            let cafe = cc.acts.iter().find(|(_, a)| *a == TileAct::Row(RowAct::Wifi("Cafe".into(), false))).map(|(t, _)| cc.tiles[*t].c).unwrap();
            cc.mouse_down(cafe);
            assert_eq!(cc.take_events(), vec![CcEvent::WifiConnect("Cafe".into())], "{mode:?}");
            assert_eq!(cc.expanded, Some(wifi));
        }
    }

    #[test]
    fn a_slider_follows_the_pointer_from_its_level_and_mutes_at_the_bottom() {
        let mut cc = fettle();
        settle(&mut cc, 5);
        let sound = cc.layout.iter().position(|t| t.kind == "slider" && t.opt("target") == Some("sound")).unwrap();
        let c = centre(&cc, sound);
        // a press far right does not blast the sound
        let right = [c[0] + 100.0, c[1]];
        cc.mouse_down(right);
        cc.update(0.016, &Default::default(), right, true, false);
        assert!(cc.take_events().is_empty(), "a press alone changes nothing");
        // moving 10% of the track to the left takes 10% off where it was (0.5), near a half it sticks
        let (a, b) = cc.drag_track;
        let to = [right[0] - 0.1 * (b - a), c[1]];
        cc.update(0.016, &Default::default(), to, true, false);
        let ev = cc.take_events();
        assert!(matches!(ev.as_slice(), [CcEvent::SetVolume(v)] if (*v - 0.4).abs() < 0.011), "{ev:?}");
        // all the way down and let go: muted, the level it had kept
        let bottom = [right[0] - 2.0 * (b - a), c[1]];
        cc.update(0.016, &Default::default(), bottom, true, false);
        cc.update(0.016, &Default::default(), bottom, false, false);
        let ev = cc.take_events();
        assert!(ev.contains(&CcEvent::SetVolume(0.5)) && ev.contains(&CcEvent::ToggleMute), "{ev:?}");
        assert!(cc.sys.muted);
        assert_eq!(snap_level(0.49), 0.5);
        assert_eq!(snap_level(0.98), 1.0);
        assert_eq!(snap_level(0.3), 0.3);
    }

    #[test]
    fn the_keyboard_walks_switches_and_pours() {
        let mut cc = fettle();
        settle(&mut cc, 5);
        let (wifi, bt) = (spec_of(&cc, "wifi"), spec_of(&cc, "bluetooth"));
        // the first key shows where the keyboard is: the first control
        assert!(cc.key(CcKey::Down));
        assert_eq!(cc.kb_focus, Some(0));
        assert_eq!(cc.layout[0].kind, "wifi");
        // down from Wi-Fi is Bluetooth; Enter switches it off
        assert!(cc.key(CcKey::Down));
        assert_eq!(cc.kb_focus, Some(bt));
        assert!(cc.key(CcKey::Enter));
        assert_eq!(cc.take_events(), vec![CcEvent::SetBluetooth(false)]);
        // Shift+Enter on Wi-Fi opens its list, Esc closes the list first
        assert!(cc.key(CcKey::Up));
        assert_eq!(cc.kb_focus, Some(wifi));
        assert!(cc.key(CcKey::Open));
        assert_eq!(cc.expanded, Some(wifi));
        assert!(cc.key(CcKey::Esc));
        assert_eq!(cc.expanded, None);
        assert!(!cc.key(CcKey::Esc), "the next Esc is the shell's (it closes the dropdown)");
        // on a slider, right pours 5%
        let sound = cc.layout.iter().position(|t| t.kind == "slider" && t.opt("target") == Some("sound")).unwrap();
        cc.kb_focus = Some(sound);
        cc.rebuild();
        assert!(cc.key(CcKey::Right));
        let ev = cc.take_events();
        assert!(matches!(ev.as_slice(), [CcEvent::SetVolume(v)] if (*v - 0.55).abs() < 0.011), "{ev:?}");
        // the pointer takes over
        let c = centre(&cc, bt);
        cc.mouse_down(c);
        assert_eq!(cc.kb_focus, None);
    }

    #[test]
    fn focus_switches_and_offers_how_long() {
        let mut cc = fettle();
        settle(&mut cc, 5);
        let focus = cc.layout.iter().position(is_focus).expect("the default layout has Focus");
        assert_eq!(list_kind(&cc.layout[focus]), Some("focus"));
        // a tap turns it on (on release: a hold would grow it)
        let c = { let c = centre(&cc, focus); [c[0] - 40.0, c[1]] };
        cc.mouse_down(c);
        cc.update(0.016, &Default::default(), c, false, false);
        assert_eq!(cc.take_events(), vec![CcEvent::Focus(FocusFor::On)]);
        assert!(cc.focus_on);
        // its list: how long, and off while on
        assert!(cc.open_grown("focus"));
        settle(&mut cc, 120);
        let rows: Vec<FocusFor> = cc.acts.iter().filter_map(|(_, a)| match a {
            TileAct::Row(RowAct::Focus(f)) => Some(*f),
            _ => None,
        }).collect();
        assert_eq!(rows, vec![FocusFor::Off, FocusFor::Hour, FocusFor::Evening, FocusFor::Morning, FocusFor::On]);
        let hour = cc.acts.iter().find(|(_, a)| *a == TileAct::Row(RowAct::Focus(FocusFor::Hour))).map(|(t, _)| cc.tiles[*t].c).unwrap();
        cc.mouse_down(hour);
        assert_eq!(cc.take_events(), vec![CcEvent::Focus(FocusFor::Hour)]);
        // the shell says until when
        cc.set_focus(true, Some("Until 18:00".into()));
        assert!(cc.labels.iter().any(|l| l.item.text.contains("Until 18:00")));
    }

    #[test]
    fn the_wheel_pours_and_the_liquid_settles() {
        let mut cc = fettle();
        settle(&mut cc, 5);
        let sound = cc.layout.iter().position(|t| t.kind == "slider" && t.opt("target") == Some("sound")).unwrap();
        let c = centre(&cc, sound);
        assert!(cc.scroll([c[0], c[1]], -15.0));
        let ev = cc.take_events();
        assert!(matches!(ev.as_slice(), [CcEvent::SetVolume(v)] if (*v - 0.55).abs() < 0.011), "{ev:?}");
        // the surface swings while the level moves, then comes to rest
        let mut swung = false;
        for _ in 0..30 {
            cc.update(0.016, &Default::default(), [0.0, 0.0], false, false);
            swung |= cc.liquid[1].tilt.abs() > 0.05;
        }
        assert!(swung);
        settle(&mut cc, 400);
        assert_eq!(cc.liquid[1].tilt, 0.0);
        assert!((cc.liquid[1].level - 0.55).abs() < 0.011);
    }

    #[test]
    fn the_sun_sets_into_a_moon() {
        let mut cc = fettle();
        let dark_k = |cc: &ControlCenter| cc.dark_k.x;
        assert_eq!(dark_k(&cc), 0.0);
        cc.set_style(CcStyle::Fettle, true, [0.9, 0.3, 0.6]);
        settle(&mut cc, 6);
        let mid = dark_k(&cc);
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        settle(&mut cc, 300);
        assert!((dark_k(&cc) - 1.0).abs() < 1e-3);
        assert!(cc.labels.iter().any(|l| l.item.text == "Dark"));
    }

    #[test]
    fn sizes_step_through_what_fits() {
        let mut layout = vec![CcTile::new("toggle", 0, 0).with("icon", "lock").with("action", "lock"), CcTile::new("toggle", 2, 0)];
        assert!(cycle_size(&mut layout, 0));
        assert_eq!(layout[0].span(), (2, 1));
        assert!(cycle_size(&mut layout, 0));
        assert_eq!(layout[0].span(), (2, 2));
        assert!(cycle_size(&mut layout, 0));
        assert_eq!(layout[0].span(), (1, 1));
        // no room to the right: medium and large are skipped... and so is everything, so nothing changes
        let mut tight = vec![CcTile::new("toggle", 3, 0), CcTile::new("toggle", 0, 0)];
        assert!(!cycle_size(&mut tight, 0));
        // a slider has one size only
        let mut s = vec![CcTile::new("slider", 0, 0)];
        assert!(!cycle_size(&mut s, 0));
    }

    #[test]
    fn a_grown_control_moves_above_a_tall_neighbour() {
        // the media control spans rows 0 and 1 beside Bluetooth on row 1: the grown list starts at row 0
        let l = cctiles::default_layout();
        let bt = l.iter().position(|t| t.kind == "bluetooth").unwrap();
        assert_eq!(cut_row(&l, l[bt].row), 0);
        assert_eq!(cut_row(&l, 4), 4);
    }
}
