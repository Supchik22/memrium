//! Menu bar status items + Quick Settings, with the "Edit Controls" mode (a gallery panel on the left,
//! "−" badges and empty slots), modelled on macOS 26. Everything is built from glass tiles, Lucide
//! glyphs and text labels; hover/press/appear motion comes from springs.

use crate::cctiles::{self, CcTile};
use crate::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use crate::sysctl::SysInfo;

const FREE: usize = usize::MAX;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// light glass (menu bar item)
    Light,
    /// dark-tinted glass tile with white content
    Dark,
    /// active blue tile
    Active,
    /// small circle inside a tile (off state)
    Circle,
    /// brighter circle (on/active state)
    Glow,
    /// opaque white toggle
    White,
    /// Windows: the flat frosted panel everything sits on
    Acrylic,
    /// Windows: a quick-settings tile that is off
    FlatOff,
    /// Windows: a tile that is on (filled with the accent colour)
    FlatOn,
    /// takes clicks but draws nothing (slider rows, the menu-bar pill of a Windows layout)
    Clear,
}

/// How the Quick Settings looks: macOS glass pills, or Fluent quick settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CcStyle {
    #[default]
    Mac,
    Windows,
}

/// How the Now Playing tile looks: the classic player card, a whole record turning, or half a record peeking up
/// from the bottom of the tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CcMediaStyle {
    #[default]
    Player,
    Record,
    HalfRecord,
    /// the cover fills the whole tile, the title and the buttons on top of it
    Cover,
}

impl CcMediaStyle {
    pub fn from_name(s: &str) -> CcMediaStyle {
        match s {
            "record" => CcMediaStyle::Record,
            "half-record" => CcMediaStyle::HalfRecord,
            "cover" => CcMediaStyle::Cover,
            _ => CcMediaStyle::Player,
        }
    }

    pub fn is_record(self) -> bool {
        matches!(self, CcMediaStyle::Record | CcMediaStyle::HalfRecord)
    }
}

/// What the Now Playing tile shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcMedia {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub present: bool,
    /// the cover, as a cell of the renderer's picture atlas
    pub cover: Option<u32>,
    /// the cover's main colour: a record takes it
    pub tone: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CcMediaCmd {
    Previous,
    PlayPause,
    Next,
}

/// What the Quick Settings asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum CcEvent {
    Media(CcMediaCmd),
    SetWifi(bool),
    SetBluetooth(bool),
    SetVolume(f32),
    SetBrightness(f32),
    ToggleMute,
    ToggleDark,
    Screenshot,
    Lock,
    OpenSettings,
    /// a tile ran a shell action (`dark-mode`, `terminal`, `exec:...`)
    Action(String),
    /// the layout was edited (to be saved)
    Layout(Vec<CcTile>),
    /// put a widget of this kind on the desktop (picked in the edit mode's gallery)
    AddWidget(String),
}

/// What pressing a tile does.
#[derive(Clone, Debug, PartialEq)]
enum TileAct {
    Wifi,
    Bluetooth,
    Mute,
    Action(String),
    Slider(Slider),
    Media,
}

/// A press on a tile in edit mode: a tap steps through its options, a drag moves it.
#[derive(Clone, Copy)]
struct TilePress {
    /// index into the layout
    spec: usize,
    from: [f32; 2],
    /// where in the tile the pointer took hold (offset from its top-left)
    grab: [f32; 2],
    mouse: [f32; 2],
    moved: bool,
}

/// the tile belongs to no layout entry (the bar pill, the Edit button)
const NONE: usize = usize::MAX;
/// top of the tile grid (just under the menu bar), as built
const GRID_TOP: f32 = 54.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slider {
    Display,
    Sound,
}

pub struct Tile {
    pub kind: Kind,
    pub c: [f32; 2],
    pub size: [f32; 2],
    pub radius: f32,
    pub scale: Spring,
}

#[derive(Clone, Copy, PartialEq)]
enum Vis {
    Always,
    Edit,
}

struct Att<T> {
    /// tile whose hover scale this item follows (FREE = none)
    tile: usize,
    vis: Vis,
    item: T,
}

pub struct ControlCenter {
    size: [f32; 2],
    tiles: Vec<Tile>,
    panels: Vec<Att<Panel>>,
    labels: Vec<Att<Label>>,
    menu_tile: usize,
    pub edit: bool,
    edit_k: Spring,
    /// the dropdown is shown (opened from the menu bar pill)
    pub open: bool,
    open_k: Spring,
    edit_btn: usize,
    glass: crate::theme::GlassStyle,
    /// tint (rgb + strength) of the menu bar background, for adaptive ink
    bar_under: [f32; 4],
    /// centre of the bar pill that opens the dropdown (set by the bar layout) and whether the bar is at the bottom
    pill_x: f32,
    pill_y: f32,
    flip: bool,
    /// draw the pill itself (a bar can instead open the dropdown from another widget)
    pill_on: bool,
    /// menu bar content turns white on dark wallpapers
    menu_light: bool,
    /// how far the dropdown was moved from its built (menu bar) position
    dd_dy: f32,
    /// the machine's state the tiles show, the tiles that act on it, and the slider being dragged
    sys: SysInfo,
    media: CcMedia,
    /// the tiles of the dropdown, as data (see `cctiles`)
    layout: Vec<CcTile>,
    /// how far the record has turned (changes every frame, so it is not part of the built tiles)
    record_angle: f32,
    /// what each tile does when pressed, and which layout entry each built tile belongs to / is the main tile of
    acts: Vec<(usize, TileAct)>,
    owner: Vec<usize>,
    spec_tile: Vec<usize>,
    /// the layout entry being built (so `tile` can note the owner)
    building: usize,
    /// the buttons of Now Playing tiles: (tile, offset from its centre, hit radius, what it does)
    media_hits: Vec<(usize, [f32; 2], f32, CcMediaCmd)>,
    /// slider tracks by tile, and the track of the slider being dragged
    tracks: Vec<(usize, (f32, f32))>,
    drag_track: (f32, f32),
    press: Option<TilePress>,
    /// which list the edit mode's gallery shows: 0 = controls, 1 = desktop widgets
    pub(crate) gallery_tab: usize,
    media_tile: usize,
    wifi_tile: usize,
    bt_tile: usize,
    dark_tile: usize,
    shot_tile: usize,
    disp_tile: usize,
    snd_tile: usize,
    mute_tile: usize,
    disp_track: (f32, f32),
    snd_track: (f32, f32),
    drag: Option<Slider>,
    /// when a slider was last moved (the machine's reading is not trusted for a moment after)
    last_drag: Option<std::time::Instant>,
    events: Vec<CcEvent>,
    style: CcStyle,
    dark: bool,
    accent: [f32; 3],
    /// vertical extent of the dropdown as built (it is moved to the real bar afterwards)
    dd_top: f32,
    dd_bottom: f32,
    lock_tile: usize,
    settings_tile: usize,
    /// which panels / labels belong to the bar itself (decided before they were moved)
    bar_p: Vec<bool>,
    bar_l: Vec<bool>,
}

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
/// white that is not "ink": it stays white over a bright picture (the cover look)
const PURE: [f32; 3] = [1.0, 1.0, 0.997];
const INK: [f32; 3] = [0.10, 0.12, 0.19];

// gallery geometry (logical px)
const GX: f32 = 36.0;
const GY: f32 = 66.0;
const GW: f32 = 552.0;
const GH: f32 = 676.0;

impl ControlCenter {
    pub fn new(w: f32, h: f32) -> Self {
        let mut cc = ControlCenter {
            size: [w, h],
            tiles: Vec::new(),
            panels: Vec::new(),
            labels: Vec::new(),
            menu_tile: 0,
            edit: false,
            edit_k: Spring::new(0.0, 240.0, 0.55),
            open: false,
            open_k: Spring::new(0.0, 300.0, 0.62),
            edit_btn: 0,
            glass: Default::default(),
            bar_under: [0.0; 4],
            pill_x: w - 173.0 + 0.0,
            pill_y: 19.0,
            flip: false,
            pill_on: true,
            menu_light: false,
            dd_dy: 0.0,
            sys: SysInfo::default(),
            media: CcMedia::default(),
            layout: cctiles::default_layout(),
            record_angle: 0.0,
            acts: Vec::new(),
            owner: Vec::new(),
            spec_tile: Vec::new(),
            building: NONE,
            media_hits: Vec::new(),
            tracks: Vec::new(),
            drag_track: (0.0, 1.0),
            press: None,
            gallery_tab: 0,
            media_tile: FREE,
            wifi_tile: FREE,
            bt_tile: FREE,
            dark_tile: FREE,
            shot_tile: FREE,
            disp_tile: FREE,
            snd_tile: FREE,
            mute_tile: FREE,
            disp_track: (0.0, 1.0),
            snd_track: (0.0, 1.0),
            drag: None,
            last_drag: None,
            events: Vec::new(),
            style: CcStyle::Mac,
            dark: false,
            accent: [0.04, 0.52, 1.0],
            dd_top: 54.0,
            dd_bottom: 607.0,
            lock_tile: FREE,
            settings_tile: FREE,
            bar_p: Vec::new(),
            bar_l: Vec::new(),
        };
        cc.build();
        cc.relocate();
        cc
    }

    /// Move the pill (the bar layout decides where it sits); rebuilds the tiles when it changed.
    /// `flip` = the bar is at the bottom, so the dropdown opens upwards.
    pub fn set_pill(&mut self, x: f32, y: f32, flip: bool, visible: bool) {
        self.pill_on = visible;
        if (self.pill_x - x).abs() < 0.5 && (self.pill_y - y).abs() < 0.5 && self.flip == flip {
            return;
        }
        self.pill_x = x;
        self.pill_y = y;
        self.flip = flip;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.tiles.clear();
        self.panels.clear();
        self.labels.clear();
        self.acts.clear();
        self.owner.clear();
        self.spec_tile.clear();
        self.media_hits.clear();
        self.tracks.clear();
        self.building = NONE;
        self.build();
        self.relocate();
    }

    /// The machine's state (Wi-Fi, Bluetooth, volume, brightness): the tiles show it.
    pub fn set_sys(&mut self, info: &SysInfo) {
        let mut info = info.clone();
        // while a slider is being moved (and a moment after) the machine's reading lags behind the pointer: keep ours
        if self.last_drag.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1500)) {
            info.volume = self.sys.volume;
            info.brightness = self.sys.brightness;
        }
        if self.sys != info {
            self.sys = info;
            self.rebuild();
        }
    }

    pub fn set_media(&mut self, m: &CcMedia) {
        if &self.media != m {
            self.media = m.clone();
            self.rebuild();
        }
    }

    /// Replace the layout (an empty one means the default). Rebuilds when it differs.
    pub fn set_layout(&mut self, layout: &[CcTile]) {
        let layout = cctiles::or_default(layout);
        if self.layout != layout {
            self.layout = layout;
            self.press = None;
            self.rebuild();
        }
    }

    /// Does a Now Playing tile show a record (so the shell has to keep it turning)?
    pub fn wants_record(&self) -> bool {
        self.layout.iter().any(|t| t.kind == "media" && CcMediaStyle::from_name(t.opt("look").unwrap_or("")).is_record())
    }

    /// The layout was changed here: rebuild and tell the shell to save it.
    fn layout_changed(&mut self) {
        self.rebuild();
        self.events.push(CcEvent::Layout(self.layout.clone()));
    }

    /// Turn the record (every frame while it plays); nothing is rebuilt.
    pub fn set_record_angle(&mut self, angle: f32) {
        self.record_angle = angle;
    }

    pub fn take_events(&mut self) -> Vec<CcEvent> {
        std::mem::take(&mut self.events)
    }

    /// Move the dragged slider to where the pointer is.
    fn drag_to(&mut self, x: f32) {
        let Some(which) = self.drag else { return };
        self.last_drag = Some(std::time::Instant::now());
        let (a, b) = self.drag_track;
        let v = ((x - a) / (b - a)).clamp(0.0, 1.0);
        match which {
            Slider::Display if self.sys.brightness.is_some() => {
                if self.sys.brightness.is_some_and(|c| (c - v).abs() < 0.01) {
                    return;
                }
                self.sys.brightness = Some(v);
                self.events.push(CcEvent::SetBrightness(v));
            }
            Slider::Sound if self.sys.volume.is_some() => {
                if self.sys.volume.is_some_and(|c| (c.min(1.0) - v).abs() < 0.01) {
                    return;
                }
                self.sys.volume = Some(v);
                self.events.push(CcEvent::SetVolume(v));
            }
            _ => return,
        }
        // show the new value at once; the machine's own reading catches up a moment later
        self.rebuild();
    }

    /// Move the built layout to the pill's real position: the bar items follow the pill, the dropdown goes under it
    /// (or above it for a bottom bar).
    fn relocate(&mut self) {
        let dy_bar = self.pill_y - 19.0;
        // the dropdown spans dd_top..dd_bottom as built (54..607 for the Mac one); keep the same gap to the pill
        let dy_dd = if self.flip { self.pill_y - 21.0 - self.dd_bottom } else { dy_bar + 54.0 - self.dd_top };
        let menu = self.menu_tile;
        let is_bar = |tile: usize, y: f32| tile == menu || (tile == FREE && y < 40.0);
        self.bar_p = self.panels.iter().map(|a| self.is_bar(a.tile, a.item.rect[1], a.vis)).collect();
        self.bar_l = self.labels.iter().map(|a| self.is_bar(a.tile, a.item.y, a.vis)).collect();
        for (i, t) in self.tiles.iter_mut().enumerate() {
            t.c[1] += if i == menu { dy_bar } else { dy_dd };
        }
        for a in &mut self.panels {
            a.item.rect[1] += if is_bar(a.tile, a.item.rect[1]) { dy_bar } else { dy_dd };
        }
        for a in &mut self.labels {
            a.item.y += if is_bar(a.tile, a.item.y) { dy_bar } else { dy_dd };
        }
        self.dd_dy = dy_dd;
    }

    pub fn set_menu_light(&mut self, light: bool) {
        self.menu_light = light;
    }

    pub fn set_open(&mut self, on: bool) {
        self.open = on;
        self.open_k.target = if on { 1.0 } else { 0.0 };
        if !on {
            self.set_edit(false);
        }
    }

    fn shown(&self) -> bool {
        self.open || self.open_k.x > 0.01
    }

    /// Anchor of the open/close zoom (just under the menu bar pill).
    fn anchor(&self) -> [f32; 2] {
        if self.style == CcStyle::Windows {
            return [self.size[0] - 60.0, if self.flip { self.dd_bottom } else { self.dd_top } + self.dd_dy];
        }
        [self.pill_x + 90.0, if self.flip { self.dd_bottom } else { 32.0 } + self.dd_dy]
    }

    /// Items of the menu bar itself stay put; everything else belongs to the dropdown.
    fn is_bar(&self, tile: usize, y: f32, vis: Vis) -> bool {
        vis == Vis::Always && (tile == self.menu_tile || (tile == FREE && y < 40.0))
    }

    pub fn set_edit(&mut self, on: bool) {
        if on {
            self.set_open(true);
        }
        self.edit = on;
        self.edit_k.target = if on { 1.0 } else { 0.0 };
    }

    // ------------------------------------------------------------ construction

    /// Fluent quick settings: a flat frosted panel with six toggle tiles (icon and name side by side, filled with
    /// the accent colour while on), brightness and volume sliders, and a footer with the battery and the settings gear.
    /// Built with its bottom edge at y = 607 like the Mac dropdown, so the same moving code places it.
    fn build_windows(&mut self) {
        let w = self.size[0];
        let ink = if self.dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let (pw, pad, gap, th) = (376.0f32, 16.0f32, 8.0f32, 52.0f32);
        let px = w - 12.0 - pw;
        let panel_h = pad + 2.0 * th + gap + 14.0 + 2.0 * 44.0 + 10.0 + 44.0 + 6.0;
        let (top, bottom) = (607.0 - panel_h, 607.0);
        self.dd_top = top;
        self.dd_bottom = bottom;

        // the bar pill: no glass, just the glyph (a Windows bar normally opens the panel from the tray instead)
        self.menu_tile = self.tile(Kind::Clear, self.pill_x, 19.0, 36.0, 26.0, 8.0);
        let menu = self.menu_tile;
        self.glyph(menu, self.pill_x, 19.0, 16.0, "sliders-horizontal", INK, 0.85);

        self.tile(Kind::Acrylic, px + pw / 2.0, top + panel_h / 2.0, pw, panel_h, 8.0);

        // toggle tiles, three across
        let tw = (pw - 2.0 * pad - 2.0 * gap) / 3.0;
        let tile_at = |col: usize, row: usize| [px + pad + col as f32 * (tw + gap) + tw / 2.0, top + pad + row as f32 * (th + gap) + th / 2.0];
        let wifi_on = self.sys.wifi.as_ref().is_some_and(|x| x.enabled);
        let bt_on = self.sys.bluetooth.as_ref().is_some_and(|x| x.powered);
        let ssid = self.sys.wifi.as_ref().and_then(|x| x.ssid.as_deref()).map(|n| if n.chars().count() > 11 { format!("{}…", n.chars().take(10).collect::<String>()) } else { n.to_string() });
        let bt_sub = self.sys.bluetooth.as_ref().map(|b| if !b.powered { "Off".to_string() } else if b.connected > 0 { format!("{} connected", b.connected) } else { "On".to_string() });
        let items: [(usize, usize, &str, &str, Option<String>, bool); 6] = [
            (0, 0, if wifi_on { "wifi" } else { "wifi-off" }, "Wi-Fi", Some(ssid.unwrap_or_else(|| if self.sys.wifi.is_none() { "Not available".into() } else if wifi_on { "On".into() } else { "Off".into() })), wifi_on),
            (1, 0, if bt_on { "bluetooth" } else { "bluetooth-off" }, "Bluetooth", Some(bt_sub.unwrap_or_else(|| "Not available".into())), bt_on),
            (2, 0, "moon", "Dark mode", Some(if self.dark { "On".into() } else { "Off".into() }), self.dark),
            (0, 1, if self.sys.muted { "volume-x" } else { "volume-2" }, "Mute", Some(if self.sys.muted { "On".into() } else { "Off".into() }), self.sys.muted),
            (1, 1, "camera", "Screenshot", None, false),
            (2, 1, "lock", "Lock", None, false),
        ];
        let mut ids = Vec::new();
        for (col, row, icon, label, sub, on) in items {
            let c = tile_at(col, row);
            let t = self.tile(if on { Kind::FlatOn } else { Kind::FlatOff }, c[0], c[1], tw, th, 6.0);
            let fg = if on { WHITE } else { ink };
            self.glyph(t, c[0] - tw / 2.0 + 20.0, c[1], 18.0, icon, fg, 0.95);
            let tx = c[0] - tw / 2.0 + 38.0;
            let size = if label.chars().count() > 9 { 11.0 } else { 12.0 };
            match sub {
                Some(sub) => {
                    self.text(t, label, tx, c[1] - 4.0, size, true, fg, 0.97, Align::Left);
                    self.text(t, &sub, tx, c[1] + 11.0, 10.5, false, fg, 0.72, Align::Left);
                }
                None => self.text(t, label, tx, c[1] + 4.5, size, true, fg, 0.97, Align::Left),
            }
            ids.push(t);
        }
        self.wifi_tile = ids[0];
        self.bt_tile = ids[1];
        self.dark_tile = ids[2];
        self.mute_tile = ids[3];
        self.shot_tile = ids[4];
        self.lock_tile = ids[5];

        // sliders
        let row_y = |i: usize| top + pad + 2.0 * th + gap + 14.0 + i as f32 * 44.0 + 22.0;
        let brightness = self.sys.brightness.unwrap_or(0.5);
        let volume = self.sys.volume.map(|v| v.min(1.0)).unwrap_or(0.5);
        let (a, b) = (px + pad + 44.0, px + pw - pad - 14.0);
        for (i, (icon, value)) in [("sun", brightness), (if self.sys.muted { "volume-x" } else { "volume-2" }, volume)].into_iter().enumerate() {
            let cy = row_y(i);
            let t = self.tile(Kind::Clear, px + pw / 2.0, cy, pw - 2.0 * pad, 40.0, 6.0);
            self.glyph(t, px + pad + 18.0, cy, 18.0, icon, ink, 0.9);
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid((a + b) / 2.0, cy, b - a, 4.0, 2.0, ink, ink).flat().alpha(0.25) });
            let fw = (b - a) * value;
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(a + fw / 2.0, cy, fw.max(1.0), 4.0, 2.0, self.accent, self.accent).flat() });
            // the thumb: a white disc with a thin dark ring
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(a + fw, cy, 18.0, 18.0, 9.0, ink, ink).flat().alpha(0.22) });
            self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(a + fw, cy, 16.0, 16.0, 8.0, [1.0; 3], [0.93, 0.95, 0.98]).flat() });
            if i == 0 {
                self.disp_tile = t;
                self.disp_track = (a, b);
            } else {
                self.snd_tile = t;
                self.snd_track = (a, b);
            }
        }

        // footer: battery on the left, settings on the right
        let fy = bottom - 6.0 - 22.0;
        let bat = self.sys.battery;
        let bat_icon = crate::widget::status_icon("battery-full", &self.sys);
        let fl = self.tile(Kind::Clear, px + pad + 60.0, fy, 120.0, 36.0, 6.0);
        match bat {
            Some((p, charging)) => {
                self.glyph(fl, px + pad + 14.0, fy, 18.0, &bat_icon, ink, 0.9);
                self.text(fl, &format!("{p}%{}", if charging { " · charging" } else { "" }), px + pad + 32.0, fy + 4.5, 12.5, false, ink, 0.9, Align::Left);
            }
            None => {
                self.glyph(fl, px + pad + 14.0, fy, 18.0, "battery-charging", ink, 0.9);
                self.text(fl, "Plugged in", px + pad + 32.0, fy + 4.5, 12.5, false, ink, 0.9, Align::Left);
            }
        }
        let gear = self.tile(Kind::FlatOff, px + pw - pad - 18.0, fy, 36.0, 36.0, 6.0);
        self.glyph(gear, px + pw - pad - 18.0, fy, 18.0, "settings", ink, 0.92);
        self.settings_tile = gear;
        // the Mac "Edit Controls" button does not exist here, but the rest of the code expects a tile for it
        self.edit_btn = self.tile(Kind::Clear, -1000.0, -1000.0, 0.0, 0.0, 0.0);
    }

    fn tile(&mut self, kind: Kind, cx: f32, cy: f32, w: f32, h: f32, r: f32) -> usize {
        self.tiles.push(Tile { kind, c: [cx, cy], size: [w, h], radius: r, scale: Spring::new(1.0, 380.0, 0.5) });
        self.owner.push(self.building);
        self.tiles.len() - 1
    }

    /// A record turning on `tile`, centred on `c`. It is cut to the tile's rounded rectangle when the panels are made.
    fn record(&mut self, tile: usize, c: (f32, f32), radius: f32) {
        let tone = self.media.tone.filter(|_| self.media.cover.is_some()).unwrap_or(self.accent);
        self.panels.push(Att { tile, vis: Vis::Always, item: Panel::record(c.0, c.1, radius, tone, self.media.cover, 0.0) });
    }

    fn glyph(&mut self, tile: usize, x: f32, y: f32, size: f32, name: &str, c: [f32; 3], a: f32) {
        self.panels.push(Att { tile, vis: Vis::Always, item: Panel::glyph(x, y, size, name, c, a) });
    }

    fn text(&mut self, tile: usize, text: &str, x: f32, y: f32, size: f32, bold: bool, c: [f32; 3], a: f32, align: Align) {
        self.labels.push(Att { tile, vis: Vis::Always, item: Label::new(text, x, y, size, bold, c, a, align) });
    }

    fn build(&mut self) {
        match self.style {
            CcStyle::Mac => self.build_mac(),
            CcStyle::Windows => self.build_windows(),
        }
    }

    /// Switch between the macOS and the Windows look (and follow light / dark and the accent colour).
    pub fn set_style(&mut self, style: CcStyle, dark: bool, accent: [f32; 3]) {
        if self.style != style || self.dark != dark || self.accent != accent {
            self.style = style;
            self.dark = dark;
            self.accent = accent;
            self.rebuild();
        }
    }

    fn build_mac(&mut self) {
        let w = self.size[0];
        let x0 = w - 14.0 - 334.0;
        let pill_x = self.pill_x;

        // ---- menu bar
        self.menu_tile = self.tile(Kind::Light, pill_x, 19.0, 54.0, 26.0, 13.0);
        let menu = self.menu_tile;
        self.glyph(menu, pill_x, 19.0, 16.0, "sliders-horizontal", INK, 0.85);

        // ---- the tiles: one entry of the layout each
        let layout = self.layout.clone();
        for (i, spec) in layout.iter().enumerate() {
            let [rx, ry, rw, rh] = spec.rect();
            let (cx, cy) = (x0 + rx + rw / 2.0, GRID_TOP + ry + rh / 2.0);
            self.building = i;
            let main = self.build_tile(spec, cx, cy, rw, rh);
            self.spec_tile.push(main);
            // edit mode: a "−" badge in the corner, and a caption on tiles whose look a tap changes
            let (bx, by) = (cx - rw / 2.0 + 7.0, cy - rh / 2.0 + 7.0);
            self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::glass(bx, by, 18.0, 18.0, 9.0).tinted([0.38, 0.41, 0.50, 0.70]).shadow(0.0, 0.0, 0.0) });
            self.panels.push(Att { tile: main, vis: Vis::Edit, item: Panel::glyph(bx, by, 11.0, "minus", WHITE, 1.0) });
            if let Some((key, _)) = cctiles::info(&spec.kind).and_then(|k| k.cycle) {
                let value = spec.opt(key).unwrap_or("").replace('-', " ");
                self.labels.push(Att { tile: main, vis: Vis::Edit, item: Label::new(value, cx + 6.0, cy - rh / 2.0 + 16.0, 10.5, true, WHITE, 0.95, Align::Center) });
            }
        }
        self.building = NONE;

        // ---- the Edit button under the last row
        let bottom = GRID_TOP + cctiles::rows(&layout) as f32 * (cctiles::CELL_H + cctiles::GAP) - cctiles::GAP;
        let scx = x0 + 167.0;
        let eb = self.tile(Kind::Dark, scx, bottom + 30.0, 112.0, 26.0, 13.0);
        self.text(eb, "Edit controls", scx, bottom + 34.5, 11.5, true, WHITE, 1.0, Align::Center);
        self.edit_btn = eb;
        self.dd_top = GRID_TOP;
        self.dd_bottom = bottom + 43.0;

        // ---- edit mode only: free cells to drop a tile on, and the Accessibility-style pill on the bar
        let rows = (cctiles::rows(&layout) + 1).min(cctiles::MAX_ROWS);
        for row in 0..rows {
            for col in 0..cctiles::COLS {
                if cctiles::fits(&layout, &CcTile::new("toggle", col, row), None) {
                    let r = CcTile::new("toggle", col, row).rect();
                    self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::glass(x0 + r[0] + r[2] / 2.0, GRID_TOP + r[1] + r[3] / 2.0, r[2], r[3], 35.0).tinted([0.80, 0.88, 1.0, 0.20]).shadow(0.06, 12.0, 4.0) });
                }
            }
        }
        self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::glass(pill_x - 40.0, 19.0, 34.0, 26.0, 13.0).tinted([0.10, 0.42, 0.98, 0.80]).shadow(0.12, 10.0, 3.0) });
        self.panels.push(Att { tile: FREE, vis: Vis::Edit, item: Panel::glyph(pill_x - 40.0, 19.0, 15.0, "plus", WHITE, 1.0) });
    }

    /// Build the tile of one layout entry (centred on `cx`, `cy`, `w` x `h`); returns its main tile.
    fn build_tile(&mut self, spec: &CcTile, cx: f32, cy: f32, w: f32, h: f32) -> usize {
        let radius = (h / 2.0).min(35.0);
        let left = cx - w / 2.0;
        let opt = |k: &str| spec.opt(k).unwrap_or("").to_string();
        match spec.kind.as_str() {
            "wifi" => {
                let (icon, sub, on) = match &self.sys.wifi {
                    Some(x) if x.enabled => ("wifi", x.ssid.as_deref().map(|n| if n.chars().count() > 13 { format!("{}…", n.chars().take(12).collect::<String>()) } else { n.to_string() }).unwrap_or_else(|| "On".into()), true),
                    Some(_) => ("wifi-off", "Off".to_string(), false),
                    None => ("wifi-off", "Not available".to_string(), false),
                };
                let t = self.pill(cx, cy, w, h, icon, "Wi-Fi", &sub, on, false);
                self.acts.push((t, TileAct::Wifi));
                t
            }
            "bluetooth" => {
                let (icon, sub, on) = match &self.sys.bluetooth {
                    Some(b) if b.powered => ("bluetooth", if b.connected > 0 { format!("{} connected", b.connected) } else { "On".to_string() }, true),
                    Some(_) => ("bluetooth-off", "Off".to_string(), false),
                    None => ("bluetooth-off", "Not available".to_string(), false),
                };
                let t = self.pill(cx, cy, w, h, icon, "Bluetooth", &sub, on, false);
                self.acts.push((t, TileAct::Bluetooth));
                t
            }
            "pill" => {
                let active = opt("style") == "active";
                let t = self.pill(cx, cy, w, h, &opt("icon"), &opt("title"), &opt("subtitle"), active, active);
                self.push_action(t, &opt("action"));
                t
            }
            "toggle" => {
                let t = self.tile(Kind::Dark, cx, cy, w, h, radius);
                self.glyph(t, cx, cy, 26.0, &opt("icon"), WHITE, 0.95);
                self.push_action(t, &opt("action"));
                t
            }
            "media" => self.build_media(spec, cx, cy, w, h),
            "slider" => {
                let sound = opt("target") == "sound";
                let (title, value, lefticon) = if sound { ("Sound", self.sys.volume.map(|v| v.min(1.0)).unwrap_or(0.7), "volume-1") } else { ("Display", self.sys.brightness.unwrap_or(0.45), "sun") };
                let t = self.tile(Kind::Dark, cx, cy, w, h, radius.min(30.0));
                self.text(t, title, left + 16.0, cy - 15.0, 12.5, true, WHITE, 1.0, Align::Left);
                let ty = cy + 17.0;
                // the Sound slider leaves room for the mute button on its right
                let track = (left + 34.0, left + w - if sound { 72.0 } else { 34.0 });
                let (a, b) = track;
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid((a + b) / 2.0, ty, b - a, 5.0, 2.5, WHITE, WHITE).alpha(0.28) });
                let fw = (b - a) * value;
                self.panels.push(Att { tile: t, vis: Vis::Always, item: Panel::solid(a + fw / 2.0, ty, fw, 5.0, 2.5, WHITE, WHITE).alpha(0.95) });
                let righticon = if sound && self.sys.muted { "volume-x" } else if sound { "volume-2" } else { "sun" };
                self.glyph(t, left + 18.0, ty, 13.0, lefticon, WHITE, 0.85);
                self.glyph(t, b + 16.0, ty, 15.0, righticon, WHITE, 0.85);
                self.acts.push((t, TileAct::Slider(if sound { Slider::Sound } else { Slider::Display })));
                self.tracks.push((t, track));
                if sound {
                    let mute = self.tile(Kind::Circle, left + w - 27.0, ty, 28.0, 28.0, 14.0);
                    self.glyph(mute, left + w - 27.0, ty, 15.0, if self.sys.muted { "volume-x" } else { "airplay" }, WHITE, 0.95);
                    self.acts.push((mute, TileAct::Mute));
                }
                t
            }
            other => {
                // a kind this version does not know (a layout from a newer one): keep its place, say so
                let t = self.tile(Kind::Dark, cx, cy, w, h, radius);
                self.text(t, other, cx, cy + 4.5, 11.0, true, WHITE, 0.6, Align::Center);
                t
            }
        }
    }

    /// A two-line pill: a round badge with an icon on the left, a title and a subtitle.
    #[allow(clippy::too_many_arguments)]
    fn pill(&mut self, cx: f32, cy: f32, w: f32, h: f32, icon: &str, title: &str, sub: &str, on: bool, active_tile: bool) -> usize {
        let left = cx - w / 2.0;
        let t = self.tile(if active_tile { Kind::Active } else { Kind::Dark }, cx, cy, w, h, (h / 2.0).min(35.0));
        let badge = self.tile(if on { Kind::Glow } else { Kind::Circle }, left + 35.0, cy, 46.0, 46.0, 23.0);
        self.glyph(badge, left + 35.0, cy, 22.0, icon, WHITE, if on { 1.0 } else { 0.95 });
        if sub.is_empty() {
            self.text(t, title, left + 68.0, cy + 5.0, 13.0, true, WHITE, 1.0, Align::Left);
        } else {
            self.text(t, title, left + 68.0, cy - 1.0, 13.0, true, WHITE, 1.0, Align::Left);
            self.text(t, sub, left + 68.0, cy + 15.0, 11.0, false, WHITE, 0.7, Align::Left);
        }
        t
    }

    fn push_action(&mut self, tile: usize, action: &str) {
        if !action.is_empty() {
            self.acts.push((tile, TileAct::Action(action.to_string())));
        }
    }

    /// Now Playing: the player card, a whole record, or half a record (`look` option).
    fn build_media(&mut self, spec: &CcTile, cx: f32, cy: f32, w: f32, h: f32) -> usize {
        let np = self.tile(Kind::Dark, cx, cy, w, h, 35.0);
        if self.media_tile == FREE {
            self.media_tile = np;
        }
        let look = CcMediaStyle::from_name(spec.opt("look").unwrap_or(""));
        // y positions below were designed for a card 154 px tall centred on 131; `y(v)` maps them to this tile
        let y = |v: f32| cy + v - 131.0;
        let short = |s: &str, n: usize| if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() };
        let (title, artist) = if self.media.present { (short(&self.media.title, 17), short(&self.media.artist, 20)) } else { ("Not Playing".to_string(), String::new()) };
        let (back, play, next) = ("skip-back", if self.media.playing { "pause" } else { "play" }, "skip-forward");
        let mut hits: Vec<([f32; 2], f32, CcMediaCmd)> = Vec::new();
        match look {
            CcMediaStyle::Player => {
                match self.media.cover.filter(|_| self.media.present) {
                    // the cover of what is playing, as a rounded square
                    Some(cell) => {
                        let mut art = Panel::app_icon(cx - 44.0, y(90.0), 44.0, cell, 1.0);
                        art.params[3] = 1.0;
                        self.panels.push(Att { tile: np, vis: Vis::Always, item: art });
                    }
                    None => {
                        self.panels.push(Att { tile: np, vis: Vis::Always, item: Panel::glass(cx - 44.0, y(90.0), 44.0, 44.0, 12.0).tinted([0.75, 0.82, 0.95, 0.22]).shadow(0.0, 0.0, 0.0) });
                        self.glyph(np, cx - 44.0, y(90.0), 20.0, "audio-lines", WHITE, 0.55);
                    }
                }
                self.text(np, &title, cx - 66.0, y(if artist.is_empty() { 148.0 } else { 140.0 }), 13.0, true, WHITE, 1.0, Align::Left);
                if !artist.is_empty() {
                    self.text(np, &artist, cx - 66.0, y(156.0), 11.0, false, WHITE, 0.65, Align::Left);
                }
                self.glyph(np, cx - 46.0, y(184.0), 22.0, back, WHITE, 0.95);
                self.glyph(np, cx, y(184.0), 26.0, play, WHITE, 1.0);
                self.glyph(np, cx + 46.0, y(184.0), 22.0, next, WHITE, 0.95);
                hits = vec![([-46.0, 53.0], 24.0, CcMediaCmd::Previous), ([0.0, 53.0], 24.0, CcMediaCmd::PlayPause), ([46.0, 53.0], 24.0, CcMediaCmd::Next)];
            }
            CcMediaStyle::Record => {
                // the whole record in the upper part of the tile, the title under it, skip buttons at its sides
                let rc = (cx, y(106.0));
                self.record(np, rc, 48.0);
                if !self.media.playing {
                    self.glyph(np, rc.0, rc.1, 22.0, "play", WHITE, 0.9);
                }
                self.glyph(np, cx - 66.0, rc.1, 20.0, back, WHITE, 0.95);
                self.glyph(np, cx + 66.0, rc.1, 20.0, next, WHITE, 0.95);
                self.text(np, &title, cx, y(if artist.is_empty() { 186.0 } else { 178.0 }), 13.0, true, WHITE, 1.0, Align::Center);
                if !artist.is_empty() {
                    self.text(np, &artist, cx, y(194.0), 11.0, false, WHITE, 0.65, Align::Center);
                }
                hits = vec![([-66.0, -25.0], 18.0, CcMediaCmd::Previous), ([66.0, -25.0], 18.0, CcMediaCmd::Next), ([0.0, -25.0], 48.0, CcMediaCmd::PlayPause)];
            }
            CcMediaStyle::Cover => {
                // the cover is the tile: a square a little larger than the tile, cut to its shape, darkened so that the
                // white text and buttons read on any picture
                match self.media.cover.filter(|_| self.media.present) {
                    Some(cell) => {
                        let mut art = Panel::app_icon(cx, cy, w.max(h), cell, 1.0);
                        // (asks for the cut to the tile's shape when the panels are made)
                        art.color2 = [0.0, 0.0, -1.0, 0.0];
                        self.panels.push(Att { tile: np, vis: Vis::Always, item: art });
                        self.panels.push(Att { tile: np, vis: Vis::Always, item: Panel::solid(cx, cy, w, h, 35.0, [0.0; 3], [0.0; 3]).flat().alpha(0.42) });
                    }
                    None => self.glyph(np, cx, y(131.0), 40.0, "audio-lines", PURE, 0.30),
                }
                self.text(np, &title, cx - w / 2.0 + 16.0, y(82.0), 14.0, true, PURE, 1.0, Align::Left);
                if !artist.is_empty() {
                    self.text(np, &artist, cx - w / 2.0 + 16.0, y(98.0), 11.5, false, PURE, 0.78, Align::Left);
                }
                self.panels.push(Att { tile: np, vis: Vis::Always, item: Panel::solid(cx, y(184.0), 46.0, 46.0, 23.0, [1.0; 3], [1.0; 3]).flat().alpha(0.22) });
                self.glyph(np, cx - 56.0, y(184.0), 22.0, back, PURE, 0.95);
                self.glyph(np, cx, y(184.0), 26.0, play, PURE, 1.0);
                self.glyph(np, cx + 56.0, y(184.0), 22.0, next, PURE, 0.95);
                hits = vec![([-56.0, 53.0], 24.0, CcMediaCmd::Previous), ([0.0, 53.0], 26.0, CcMediaCmd::PlayPause), ([56.0, 53.0], 24.0, CcMediaCmd::Next)];
            }
            CcMediaStyle::HalfRecord => {
                // the text and buttons on top, the upper half of a record whose centre is on the tile's bottom edge
                self.text(np, &title, cx - 66.0, y(78.0), 13.0, true, WHITE, 1.0, Align::Left);
                if !artist.is_empty() {
                    self.text(np, &artist, cx - 66.0, y(93.0), 11.0, false, WHITE, 0.65, Align::Left);
                }
                self.record(np, (cx, cy + h / 2.0), 78.0);
                self.glyph(np, cx - 46.0, y(114.0), 20.0, back, WHITE, 0.95);
                self.glyph(np, cx, y(114.0), 22.0, play, WHITE, 1.0);
                self.glyph(np, cx + 46.0, y(114.0), 20.0, next, WHITE, 0.95);
                hits = vec![([-46.0, -17.0], 22.0, CcMediaCmd::Previous), ([0.0, -17.0], 22.0, CcMediaCmd::PlayPause), ([46.0, -17.0], 22.0, CcMediaCmd::Next), ([0.0, 62.0], 34.0, CcMediaCmd::PlayPause)];
            }
        }
        self.media_hits.extend(hits.into_iter().map(|(o, r, c)| (np, o, r, c)));
        self.acts.push((np, TileAct::Media));
        np
    }

    // ------------------------------------------------------------ interaction

    fn rect_hit(m: [f32; 2], c: [f32; 2], s: [f32; 2]) -> bool {
        (m[0] - c[0]).abs() < s[0] / 2.0 && (m[1] - c[1]).abs() < s[1] / 2.0
    }

    fn done_rect(&self) -> ([f32; 2], [f32; 2]) {
        self.mapped(492.0, 644.0, 46.0, 22.0)
    }

    /// Shrink the gallery when the screen is too short to fit it above the Dock.
    fn fit(&self) -> f32 {
        ((self.size[1] - 182.0) / GH).clamp(0.6, 1.0)
    }

    fn gallery_k(&self) -> f32 {
        (0.9 + 0.1 * self.edit_k.x.clamp(0.0, 1.1)) * self.fit()
    }

    /// Done button / gallery rect in screen space (centre, size), following the same mapping as the drawing.
    fn mapped(&self, x: f32, y: f32, w: f32, h: f32) -> ([f32; 2], [f32; 2]) {
        let k = self.gallery_k();
        let pivot = [GX + GW / 2.0, GY + GH / 2.0];
        let (cx, cy) = (GX + x + w / 2.0, GY + y + h / 2.0);
        ([pivot[0] + (cx - pivot[0]) * k, pivot[1] + (cy - pivot[1]) * k], [w * k, h * k])
    }

    fn gallery_visible(&self) -> bool {
        self.edit || self.edit_k.x > 0.02
    }

    /// Is the pointer over any control (so windows underneath must not get the click)?
    pub fn hit(&self, m: [f32; 2]) -> bool {
        let (gc, gs) = self.mapped(0.0, 0.0, GW, GH);
        let any = self.tiles.iter().enumerate().any(|(i, t)| ((i == self.menu_tile && self.pill_on) || (self.shown() && !(self.edit && i == self.edit_btn))) && Self::rect_hit(m, t.c, t.size));
        any || (self.edit && Self::rect_hit(m, gc, gs))
    }

    /// Returns true when the click was consumed.
    pub fn mouse_down(&mut self, m: [f32; 2]) -> bool {
        let mt = &self.tiles[self.menu_tile];
        if self.pill_on && Self::rect_hit(m, mt.c, mt.size) {
            let on = !self.open;
            self.set_open(on);
            return true;
        }
        if self.edit {
            return self.edit_mouse_down(m);
        }
        if self.open {
            let eb = &self.tiles[self.edit_btn];
            if Self::rect_hit(m, eb.c, eb.size) {
                self.set_edit(true);
                return true;
            }
            // the tiles that do something (the one drawn last is on top)
            let hit = |t: usize| t != FREE && t < self.tiles.len() && Self::rect_hit(m, self.tiles[t].c, self.tiles[t].size);
            for (tile, act) in self.acts.clone().into_iter().rev() {
                if !hit(tile) {
                    continue;
                }
                match act {
                    TileAct::Wifi => {
                        if let Some(w) = &self.sys.wifi {
                            self.events.push(CcEvent::SetWifi(!w.enabled));
                        }
                    }
                    TileAct::Bluetooth => {
                        if let Some(b) = &self.sys.bluetooth {
                            self.events.push(CcEvent::SetBluetooth(!b.powered));
                        }
                    }
                    TileAct::Mute => self.events.push(CcEvent::ToggleMute),
                    TileAct::Action(name) => {
                        // switching the look keeps the dropdown open; running something closes it
                        let keep = name == "dark-mode";
                        self.events.push(CcEvent::Action(name));
                        if !keep {
                            self.set_open(false);
                        }
                    }
                    TileAct::Slider(which) => {
                        self.drag = Some(which);
                        self.drag_track = self.tracks.iter().find(|(t, _)| *t == tile).map_or((0.0, 1.0), |(_, tr)| *tr);
                        self.drag_to(m[0]);
                    }
                    TileAct::Media => {
                        let c = self.tiles[tile].c;
                        let cmd = self.media_hits.iter().find(|(t, off, r, _)| *t == tile && (m[0] - (c[0] + off[0])).hypot(m[1] - (c[1] + off[1])) < *r).map(|h| h.3);
                        if let Some(cmd) = cmd {
                            self.events.push(CcEvent::Media(cmd));
                        }
                    }
                }
                return true;
            }
            // Windows quick settings keep their tiles in fields
            if hit(self.wifi_tile) {
                if let Some(w) = &self.sys.wifi {
                    self.events.push(CcEvent::SetWifi(!w.enabled));
                }
                return true;
            }
            if hit(self.bt_tile) {
                if let Some(b) = &self.sys.bluetooth {
                    self.events.push(CcEvent::SetBluetooth(!b.powered));
                }
                return true;
            }
            if hit(self.mute_tile) {
                self.events.push(CcEvent::ToggleMute);
                return true;
            }
            if hit(self.dark_tile) {
                self.events.push(CcEvent::ToggleDark);
                return true;
            }
            if hit(self.lock_tile) {
                self.events.push(CcEvent::Lock);
                self.set_open(false);
                return true;
            }
            if hit(self.settings_tile) {
                self.events.push(CcEvent::OpenSettings);
                self.set_open(false);
                return true;
            }
            if hit(self.shot_tile) {
                self.events.push(CcEvent::Screenshot);
                self.set_open(false);
                return true;
            }
            for (tile, which) in [(self.disp_tile, Slider::Display), (self.snd_tile, Slider::Sound)] {
                if hit(tile) {
                    self.drag = Some(which);
                    self.drag_track = if which == Slider::Display { self.disp_track } else { self.snd_track };
                    self.drag_to(m[0]);
                    return true;
                }
            }
            // a click outside the dropdown closes it (and still reaches whatever is underneath)
            let inside = self.tiles.iter().enumerate().any(|(i, t)| i != self.menu_tile && Self::rect_hit(m, t.c, t.size));
            if !inside {
                self.set_open(false);
            }
            return inside;
        }
        false
    }

    /// Edit mode: Done, the gallery (add a control), and the tiles (remove, tap to change the look, drag to move).
    fn edit_mouse_down(&mut self, m: [f32; 2]) -> bool {
        let (c, s) = self.done_rect();
        if Self::rect_hit(m, c, s) {
            self.set_edit(false);
            return true;
        }
        // the two tabs of the gallery
        for tab in 0..2 {
            let [x, y, w, h] = gallery_tab(tab);
            let (c, sz) = self.mapped(x, y, w, h);
            if Self::rect_hit(m, c, sz) {
                self.gallery_tab = tab;
                return true;
            }
        }
        // the gallery: a tap on a control adds it in the first free place, on a widget puts it on the desktop
        if self.gallery_tab == 0 {
            for (i, p) in cctiles::PRESETS.iter().enumerate() {
                let [x, y, w, h] = gallery_cell(i);
                let (c, s) = self.mapped(x, y, w, h);
                if Self::rect_hit(m, c, s) {
                    let (cw, ch) = p.span();
                    if let Some((col, row)) = cctiles::first_free(&self.layout, p.kind, cw, ch) {
                        self.layout.push(p.tile(col, row));
                        self.layout_changed();
                    }
                    return true;
                }
            }
        } else {
            for (i, (kind, ..)) in crate::deskwidgets::KINDS.iter().enumerate() {
                let [x, y, w, h] = widget_cell(i);
                let (c, s) = self.mapped(x, y, w, h);
                if Self::rect_hit(m, c, s) {
                    self.events.push(CcEvent::AddWidget(kind.to_string()));
                    return true;
                }
            }
        }
        // the tiles, topmost first
        for si in (0..self.layout.len()).rev() {
            let Some(&t) = self.spec_tile.get(si) else { continue };
            let tile = &self.tiles[t];
            if !Self::rect_hit(m, tile.c, tile.size) {
                continue;
            }
            let tl = [tile.c[0] - tile.size[0] / 2.0, tile.c[1] - tile.size[1] / 2.0];
            // the "−" badge removes the tile
            if (m[0] - (tl[0] + 7.0)).hypot(m[1] - (tl[1] + 7.0)) < 12.0 {
                self.layout.remove(si);
                self.layout_changed();
                return true;
            }
            self.press = Some(TilePress { spec: si, from: m, grab: [m[0] - tl[0], m[1] - tl[1]], mouse: m, moved: false });
            return true;
        }
        let (gc, gs) = self.mapped(0.0, 0.0, GW, GH);
        Self::rect_hit(m, gc, gs)
    }

    /// The pointer was released over a tile pressed in edit mode: a tap changes its look, a drag drops it on a cell.
    fn finish_press(&mut self, p: TilePress) {
        let Some(&t) = self.spec_tile.get(p.spec) else { return };
        if !p.moved {
            if cctiles::cycle(&mut self.layout[p.spec]) {
                self.layout_changed();
            }
            return;
        }
        // the tile's top-left where it was let go, against the grid's top-left (as built, then moved with the dropdown)
        let x0 = self.size[0] - 14.0 - 334.0;
        let tl = [p.mouse[0] - p.grab[0], p.mouse[1] - p.grab[1]];
        let _ = t;
        let (col, row) = cctiles::cell_near([tl[0] - x0, tl[1] - (GRID_TOP + self.dd_dy)]);
        let mut moved = self.layout[p.spec].clone();
        moved.col = col;
        moved.row = row;
        if (col, row) != (self.layout[p.spec].col, self.layout[p.spec].row) && cctiles::fits(&self.layout, &moved, Some(p.spec)) {
            self.layout[p.spec] = moved;
            self.layout_changed();
        }
    }

    /// How far the tile (or a part of one) is carried while it is dragged.
    fn carried(&self, tile: usize) -> [f32; 2] {
        match self.press {
            Some(p) if p.moved && self.owner.get(tile).copied() == Some(p.spec) => [p.mouse[0] - p.from[0], p.mouse[1] - p.from[1]],
            _ => [0.0, 0.0],
        }
    }

    pub fn update(&mut self, dt: f32, an: &crate::theme::AnimStyle, mouse: [f32; 2], pressed: bool, dragging: bool) {
        let dragging_tile = self.press.is_some_and(|p| p.moved);
        for (i, t) in self.tiles.iter_mut().enumerate() {
            let hit = Self::rect_hit(mouse, t.c, t.size);
            let carried = dragging_tile && self.press.is_some_and(|p| self.owner.get(i).copied() == Some(p.spec));
            t.scale.target = match (hit, pressed, self.style) {
                _ if carried => 1.06,
                (true, true, CcStyle::Mac) if !dragging => 0.93,
                (true, true, CcStyle::Windows) if !dragging => 0.97,
                (true, true, _) => 1.0,
                (true, false, CcStyle::Mac) => 1.06,
                _ => 1.0,
            };
            an.step("control_center", &mut t.scale, dt);
        }
        an.step("control_center", &mut self.edit_k, dt);
        an.step("control_center", &mut self.open_k, dt);
        if let Some(mut p) = self.press {
            if pressed {
                p.mouse = mouse;
                p.moved |= (mouse[0] - p.from[0]).hypot(mouse[1] - p.from[1]) > 5.0;
                self.press = Some(p);
            } else {
                self.press = None;
                self.finish_press(p);
            }
        }
        if self.drag.is_some() {
            if pressed {
                self.drag_to(mouse[0]);
            } else {
                self.drag = None;
            }
        }
    }

    // ------------------------------------------------------------ drawing

    pub fn set_glass(&mut self, g: &crate::theme::GlassStyle) {
        self.glass = g.clone();
    }

    pub fn set_bar_under(&mut self, u: [f32; 4]) {
        self.bar_under = u;
    }

    /// Glass tint under an item attached to `tile`, when its ink should adapt to the backdrop.
    fn under(&self, tile: usize, bar: bool) -> Option<[f32; 4]> {
        if tile == FREE {
            return bar.then_some(self.bar_under);
        }
        match self.tiles[tile].kind {
            Kind::Light => Some(self.glass.tint),
            // the glass tiles: white ink on a dark backdrop, dark ink over a bright wallpaper
            Kind::Dark if self.style == CcStyle::Mac => Some(self.glass.tint),
            _ => None,
        }
    }

    fn is_ink(c: [f32; 3]) -> bool {
        let near = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3 && (a[2] - b[2]).abs() < 1e-3;
        near(c, WHITE) || near(c, INK)
    }

    fn tile_panel(&self, t: &Tile) -> Panel {
        let k = t.scale.x;
        let (cx, cy, w, h, r) = (t.c[0], t.c[1], t.size[0] * k, t.size[1] * k, t.radius * k);
        let g = |tint: [f32; 4]| self.glass.panel(cx, cy, w, h, r).tinted(tint);
        match t.kind {
            Kind::Light => self.glass.panel(cx, cy, w, h, r).adaptive(),
            Kind::Dark => g([0.50, 0.55, 0.66, 0.26]).adaptive(),
            Kind::Active => g([0.10, 0.40, 0.98, 0.80]),
            Kind::Circle => g([0.30, 0.35, 0.46, 0.40]).shadow(0.0, 0.0, 0.0).adaptive(),
            Kind::Glow => g([0.42, 0.58, 0.98, 0.60]).shadow(0.0, 0.0, 0.0),
            Kind::White => Panel::white(cx, cy, w, h, r),
            Kind::Acrylic => {
                let tint = if self.dark { [0.12, 0.12, 0.12, 0.84] } else { [0.96, 0.97, 0.99, 0.84] };
                let mut p = g(tint);
                // a frosted pane: no refraction or edge light, a hairline border
                p.params[3] = 0.0;
                p.fx = [p.fx[0].max(32.0), 0.0, 0.0, 0.0];
                p.color2 = if self.dark { [1.0, 1.0, 1.0, 0.10] } else { [0.0, 0.0, 0.0, 0.10] };
                p
            }
            Kind::FlatOff => {
                let (top, bot, a) = if self.dark { ([1.0; 3], [1.0; 3], 0.09) } else { ([1.0; 3], [0.95, 0.96, 0.98], 0.75) };
                Panel::solid(cx, cy, w, h, r, top, bot).flat().alpha(a)
            }
            Kind::FlatOn => Panel::solid(cx, cy, w, h, r, self.accent, [self.accent[0] * 0.9, self.accent[1] * 0.9, self.accent[2] * 0.95]).flat(),
            Kind::Clear => Panel::solid(cx, cy, w.max(1.0), h.max(1.0), r, [0.0; 3], [0.0; 3]).flat().alpha(0.0),
        }
    }

    fn scaled_about(&self, tile: usize, p: [f32; 2]) -> ([f32; 2], f32) {
        if tile == FREE {
            return (p, 1.0);
        }
        let t = &self.tiles[tile];
        let k = t.scale.x;
        ([t.c[0] + (p[0] - t.c[0]) * k, t.c[1] + (p[1] - t.c[1]) * k], k)
    }

    /// Zoom factor of the dropdown while it opens / closes.
    fn zoom(&self) -> f32 {
        0.78 + 0.22 * self.open_k.x.clamp(0.0, 1.05)
    }

    fn zoom_pt(&self, p: [f32; 2]) -> [f32; 2] {
        let (a, k) = (self.anchor(), self.zoom());
        [a[0] + (p[0] - a[0]) * k, a[1] + (p[1] - a[1]) * k]
    }

    /// A big tile or card in the Mac look gets squircle corners (a negative radius says so to the shader).
    fn squircle(&self, mut p: Panel) -> Panel {
        if self.style == CcStyle::Mac && p.params[0] > 0.0 && matches!(p.params[1].floor() as u32, 0..=2) && p.rect[3] > 14.0 {
            p.params[0] = -p.params[0];
        }
        p
    }

    pub fn panels(&self) -> Vec<Panel> {
        let shown = self.shown();
        let k = self.zoom();
        // the dropdown fades with its zoom: all of a glass tile (blur, refraction, shadow, rim) at once
        let fade = (self.open_k.x / 0.85).clamp(0.0, 1.0);
        // drawing order: the empty cells of edit mode at the bottom, then the tiles, then the one being carried
        let (mut slots, mut out, mut top): (Vec<Panel>, Vec<Panel>, Vec<Panel>) = (Vec::new(), Vec::new(), Vec::new());
        // where each tile ends up (centre, half size, corner radius): a record on it is cut to this shape
        let mut shapes: Vec<Option<([f32; 2], [f32; 2], f32)>> = vec![None; self.tiles.len()];
        for (i, t) in self.tiles.iter().enumerate() {
            if i == self.menu_tile {
                // the pill is glass only while the Quick Settings is open (it fades in and out with it); closed, it is
                // just the glyph on the bar
                if self.pill_on && fade > 0.01 {
                    let mut p = self.squircle(self.tile_panel(t));
                    p.fade_by(fade);
                    out.push(p);
                }
            } else if shown && !(self.edit && i == self.edit_btn) {
                let mut p = self.tile_panel(t);
                let c = self.zoom_pt([p.rect[0], p.rect[1]]);
                p.rect = [c[0], c[1], p.rect[2] * k, p.rect[3] * k];
                p.params[0] *= k;
                let off = self.carried(i);
                p.rect[0] += off[0];
                p.rect[1] += off[1];
                shapes[i] = Some(([p.rect[0], p.rect[1]], [p.rect[2], p.rect[3]], p.params[0]));
                let mut p = self.squircle(p);
                p.fade_by(fade);
                if self.lifted(i) {
                    top.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        for (pi, a) in self.panels.iter().enumerate() {
            if a.vis == Vis::Edit && !self.edit {
                continue;
            }
            let bar = self.bar_p.get(pi).copied().unwrap_or(false);
            if bar && !self.pill_on {
                continue;
            }
            if !bar && (!shown || (self.edit && a.tile == self.edit_btn)) {
                continue;
            }
            let mut p = a.item;
            // the pill's glyph with no glass under it sits on the bar itself: it takes the bar's ink like the other widgets
            // (the glass tint says nothing about an opaque bar, and over a dark wallpaper it made the glyph white on a white bar)
            let on_bare_bar = bar && a.tile == self.menu_tile && !(self.pill_on && fade > 0.01);
            if on_bare_bar && p.params[1] == 5.0 && Self::is_ink([p.tint[0], p.tint[1], p.tint[2]]) {
                let ink = if self.menu_light { WHITE } else { INK };
                p.tint[..3].copy_from_slice(&ink);
            } else if matches!(p.params[1] as u32, 1 | 5) && Self::is_ink([p.tint[0], p.tint[1], p.tint[2]]) {
                if let Some(u) = self.under(a.tile, bar) {
                    p = p.auto_ink(u);
                }
            }
            // the Mac pill is light glass: its dark glyph stays dark, white on it would vanish (the Windows one has no glass)
            let on_light_pill = self.style == CcStyle::Mac && a.tile == self.menu_tile;
            if self.menu_light && bar && !on_light_pill && p.params[1] == 5.0 && (p.tint[0] - INK[0]).abs() < 1e-4 && (p.tint[2] - INK[2]).abs() < 1e-4 {
                p.tint[0] = 1.0;
                p.tint[1] = 1.0;
                p.tint[2] = 1.0;
            }
            let (c, tk) = self.scaled_about(a.tile, [p.rect[0], p.rect[1]]);
            p.rect = [c[0], c[1], p.rect[2] * tk, p.rect[3] * tk];
            p.params[0] *= tk;
            if !bar {
                let c = self.zoom_pt([p.rect[0], p.rect[1]]);
                p.rect = [c[0], c[1], p.rect[2] * k, p.rect[3] * k];
                p.params[0] *= k;
            }
            let off = self.carried(a.tile);
            p.rect[0] += off[0];
            p.rect[1] += off[1];
            if !bar {
                p.fade_by(fade);
            }
            if p.params[1] == 15.0 || (p.params[1] == 7.0 && p.color2[2] < 0.0) {
                // a record: it turns, and stays inside its tile (so does a cover that fills one)
                if p.params[1] == 15.0 {
                    p.fx[0] = self.record_angle;
                }
                if let Some((c, half, r)) = shapes.get(a.tile).copied().flatten() {
                    p = p.clipped_to(c, half, r);
                }
                if self.lifted(a.tile) {
                    top.push(p);
                } else {
                    out.push(p);
                }
                continue;
            }
            let p = self.squircle(p);
            if a.tile == FREE && a.vis == Vis::Edit && !bar {
                slots.push(p);
            } else if self.lifted(a.tile) {
                top.push(p);
            } else {
                out.push(p);
            }
        }
        slots.extend(out);
        slots.extend(top);
        if self.gallery_visible() {
            slots.extend(self.gallery().0);
        }
        slots
    }

    /// Does the (estimated) box of a label overlap `rect` (centre, half size)?
    fn label_touches(l: &Label, rect: ([f32; 2], [f32; 2])) -> bool {
        let w = l.text.chars().count() as f32 * l.size * 0.55;
        let x0 = match l.align {
            Align::Left => l.x,
            Align::Center => l.x - w / 2.0,
            Align::Right => l.x - w,
        };
        let (y0, y1) = (l.y - l.size, l.y + l.size * 0.3);
        let (c, h) = rect;
        x0 < c[0] + h[0] && x0 + w > c[0] - h[0] && y0 < c[1] + h[1] && y1 > c[1] - h[1]
    }

    /// Is this tile (or a part of it) the one being carried in edit mode?
    fn lifted(&self, tile: usize) -> bool {
        self.press.is_some_and(|p| p.moved && self.owner.get(tile).copied() == Some(p.spec))
    }

    pub fn labels(&self) -> Vec<Label> {
        let shown = self.shown();
        let k = self.zoom();
        let mut out = Vec::new();
        // the rectangle (centre, half size) of the tile being carried, where other tiles' text must not show
        let lift = self.press.filter(|p| p.moved).and_then(|p| {
            let t = self.tiles.get(*self.spec_tile.get(p.spec)?)?;
            let off = self.carried(self.spec_tile[p.spec]);
            let c = self.zoom_pt(t.c);
            Some(([c[0] + off[0], c[1] + off[1]], [t.size[0] * t.scale.x * k / 2.0, t.size[1] * t.scale.x * k / 2.0]))
        });
        for (li, a) in self.labels.iter().enumerate() {
            if a.vis == Vis::Edit && !self.edit {
                continue;
            }
            let bar = self.bar_l.get(li).copied().unwrap_or(false);
            if bar && !self.pill_on {
                continue;
            }
            if !bar && (!shown || self.open_k.x < 0.7 || (self.edit && a.tile == self.edit_btn)) {
                continue;
            }
            let mut l = a.item.clone();
            if Self::is_ink(l.color) {
                if let Some(u) = self.under(a.tile, bar) {
                    l = l.auto_ink(u);
                }
            }
            if self.menu_light && bar && l.color == INK {
                l.color = WHITE;
            }
            let (c, _) = self.scaled_about(a.tile, [l.x, l.y]);
            l.x = c[0];
            l.y = c[1];
            if !bar {
                let c = self.zoom_pt([l.x, l.y]);
                l.x = c[0];
                l.y = c[1];
                l.size = (l.size * k * 2.0).round() / 2.0;
            }
            if !bar {
                l.opacity *= (self.open_k.x / 0.85).clamp(0.0, 1.0);
            }
            let off = self.carried(a.tile);
            l.x += off[0];
            l.y += off[1];
            // text is drawn above every panel, so the words of tiles under the one being carried would show through it
            if !self.lifted(a.tile) && lift.is_some_and(|r| Self::label_touches(&l, r)) {
                continue;
            }
            out.push(l);
        }
        if self.gallery_visible() {
            out.extend(self.gallery().1);
        }
        out
    }

    // ------------------------------------------------------------ the "Edit Controls" gallery

    fn gallery(&self) -> (Vec<Panel>, Vec<Label>) {
        let k = self.gallery_k();
        let pivot = [GX + GW / 2.0, GY + GH / 2.0];
        let mut g = Gal { p: Vec::new(), l: Vec::new(), pivot, k, fit: self.fit() };

        // frame
        let mut base = g.glass(0.0, 0.0, GW, GH, 34.0, [0.92, 0.96, 1.0, 0.44]);
        base.fx[0] = 16.0; // frosted
        base.extra = [0.30, 64.0, 26.0, 0.0];
        g.p.push(base);

        // the tabs: what can be added to the Quick Settings, and what to the desktop
        for (tab, label) in ["Controls", "Widgets"].into_iter().enumerate() {
            let [x, y, w, h] = gallery_tab(tab);
            if tab == self.gallery_tab {
                g.glass_t(x, y, w, h, h / 2.0, [0.10, 0.45, 1.0, 0.70]);
                g.text(x + w / 2.0, y + h / 2.0 + 4.5, 12.5, true, WHITE, 1.0, Align::Center, label);
            } else {
                g.glass_t(x, y, w, h, h / 2.0, [0.58, 0.62, 0.74, 0.30]);
                g.text(x + w / 2.0, y + h / 2.0 + 4.5, 12.5, false, INK, 0.85, Align::Center, label);
            }
        }

        if self.gallery_tab == 0 {
            // every control that can be added: a coloured badge, its name, and a "+" while there is room for it
            for (i, p) in cctiles::PRESETS.iter().enumerate() {
                let [x, y, w, h] = gallery_cell(i);
                let (cw, ch) = p.span();
                let room = cctiles::first_free(&self.layout, p.kind, cw, ch).is_some();
                let a = if room { 1.0 } else { 0.4 };
                g.glass_t(x, y, w, h, 14.0, [0.58, 0.62, 0.74, 0.30 * a + 0.05]);
                let bg = match (p.kind, p.opts.iter().find(|(k, _)| *k == "target").map(|(_, v)| *v)) {
                    ("wifi" | "bluetooth", _) => [0.20, 0.60, 1.0],
                    ("media", _) => [1.0, 0.30, 0.40],
                    ("slider", Some("sound")) => [1.0, 0.32, 0.32],
                    ("slider", _) => [0.15, 0.58, 1.0],
                    ("pill", _) => [0.46, 0.32, 0.95],
                    _ => [0.30, 0.33, 0.42],
                };
                g.solid(x + 8.0, y + 8.0, 28.0, 28.0, 8.0, bg, bg, a);
                g.glyph(x + 22.0, y + 22.0, 15.0, p.icon, WHITE, a);
                g.text(x + 46.0, y + 27.0, 12.5, false, INK, 0.92 * a, Align::Left, p.label);
                g.glyph(x + w - 22.0, y + h / 2.0, 14.0, "plus", INK, 0.7 * a);
            }
        } else {
            // the widgets of the desktop: a card with a little picture of each, its name and its size
            for (i, (kind, label, size)) in crate::deskwidgets::KINDS.iter().enumerate() {
                let [x, y, w, h] = widget_cell(i);
                let (icon, bg) = widget_look(kind);
                g.glass_t(x, y, w, h, 18.0, [0.62, 0.66, 0.78, 0.42]);
                let dark = [bg[0] * 0.62, bg[1] * 0.62, bg[2] * 0.62];
                let (px, py, pw, ph) = (x + 10.0, y + 10.0, 60.0, 60.0);
                g.solid(px, py, pw, ph, 13.0, bg, dark, 1.0);
                match *kind {
                    "clock" | "glassclock" => {
                        g.text(px + pw / 2.0, py + ph / 2.0 + 6.0, 17.0, true, WHITE, 1.0, Align::Center, "12:45");
                    }
                    "calendar" => {
                        g.solid(px + 8.0, py + 12.0, pw - 16.0, ph - 20.0, 5.0, [1.0; 3], [0.93, 0.94, 0.97], 1.0);
                        g.solid(px + 8.0, py + 12.0, pw - 16.0, 11.0, 5.0, [1.0, 0.30, 0.30], [0.85, 0.20, 0.25], 1.0);
                        for n in 0..9 {
                            let (cx, cy) = (px + 15.0 + (n % 3) as f32 * 12.0, py + 31.0 + (n / 3) as f32 * 7.5);
                            g.solid(cx, cy, 5.0, 4.0, 2.0, [0.35, 0.38, 0.46], [0.35, 0.38, 0.46], if n == 4 { 1.0 } else { 0.45 });
                        }
                    }
                    "system" => {
                        for (n, f) in [0.8, 0.5, 0.65].into_iter().enumerate() {
                            let by = py + 14.0 + n as f32 * 14.0;
                            g.solid(px + 9.0, by, pw - 18.0, 5.0, 2.5, WHITE, WHITE, 0.3);
                            g.solid(px + 9.0, by, (pw - 18.0) * f, 5.0, 2.5, WHITE, WHITE, 0.95);
                        }
                    }
                    "disk" => {
                        g.glyph(px + pw / 2.0, py + 24.0, 22.0, icon, WHITE, 1.0);
                        g.solid(px + 9.0, py + 44.0, pw - 18.0, 5.0, 2.5, WHITE, WHITE, 0.3);
                        g.solid(px + 9.0, py + 44.0, (pw - 18.0) * 0.6, 5.0, 2.5, WHITE, WHITE, 0.95);
                    }
                    "vinyl" => {
                        g.solid(px + 6.0, py + 6.0, pw - 12.0, ph - 12.0, 24.0, [0.10, 0.10, 0.13], [0.04, 0.04, 0.06], 1.0);
                        g.solid(px + 20.0, py + 20.0, 20.0, 20.0, 10.0, [1.0, 0.55, 0.75], [0.60, 0.40, 1.0], 1.0);
                        g.solid(px + 28.5, py + 28.5, 3.0, 3.0, 1.5, [0.05; 3], [0.05; 3], 1.0);
                    }
                    "visualizer" => {
                        for (n, v) in [0.35, 0.6, 0.9, 0.5, 0.75, 0.4, 0.55].into_iter().enumerate() {
                            let bh = 36.0 * v;
                            g.solid(px + 9.0 + n as f32 * 6.8, py + ph - 11.0 - bh, 4.0, bh, 2.0, WHITE, [0.85, 0.80, 1.0], 0.95);
                        }
                    }
                    "music" => {
                        g.glyph(px + pw / 2.0, py + 24.0, 22.0, icon, WHITE, 1.0);
                        g.solid(px + 9.0, py + 46.0, pw - 18.0, 4.0, 2.0, WHITE, WHITE, 0.3);
                        g.solid(px + 9.0, py + 46.0, (pw - 18.0) * 0.4, 4.0, 2.0, WHITE, WHITE, 0.95);
                    }
                    _ => g.glyph(px + pw / 2.0, py + ph / 2.0, 26.0, icon, WHITE, 1.0),
                }
                g.text(x + 82.0, y + 33.0, 13.5, true, INK, 0.95, Align::Left, label);
                g.text(x + 82.0, y + 53.0, 11.0, false, INK, 0.6, Align::Left, &format!("{} \u{00d7} {}", size[0] as i32, size[1] as i32));
                g.glyph(x + w - 24.0, y + h / 2.0, 15.0, "plus", INK, 0.7);
            }
        }

        // how to use it
        if self.gallery_tab == 0 {
            let hy = 72.0 + 8.0 * 52.0 + 18.0;
            g.hairline(24.0, hy - 8.0, GW - 48.0);
            for (i, line) in ["Drag a tile to move it, tap the \u{2212} on its corner to remove it.", "Tap a Now Playing tile to change its look (player, record, half record),", "or a slider to switch between Display and Sound.", "The layout is saved as [[cc_tiles]] in config.toml; there you can also", "give a tile any icon and a shell action (or exec:program)."].iter().enumerate() {
                g.text(24.0, hy + 14.0 + i as f32 * 16.0, 11.0, false, INK, 0.7, Align::Left, line);
            }
        } else {
            g.hairline(24.0, 510.0, GW - 48.0);
            g.text(24.0, 528.0, 11.0, false, INK, 0.7, Align::Left, "Tap a card to put the widget on the desktop. Right-click a widget for its options.");
            // what the keys do while a widget is dragged
            for (i, (key, what)) in [("Drag", "lines up with the other widgets and the screen"), ("Ctrl", "snaps to a 20 px grid"), ("Shift", "moves along one line only"), ("Ctrl + Shift", "sits next to the others with an even gap"), ("Alt", "free, nothing pulls")].into_iter().enumerate() {
                let y = 548.0 + i as f32 * 17.0;
                g.glass_t(24.0, y, 82.0, 15.0, 7.5, [0.58, 0.62, 0.74, 0.40]);
                g.text(65.0, y + 11.0, 10.5, true, INK, 0.9, Align::Center, key);
                g.text(118.0, y + 11.5, 11.0, false, INK, 0.75, Align::Left, what);
            }
        }

        // footer
        g.hairline(196.0, 636.0, 342.0);
        let done = g.glass_t(492.0, 644.0, 46.0, 22.0, 11.0, [0.10, 0.48, 1.0, 0.92]);
        let _ = done;
        g.text(515.0, 659.0, 11.0, true, WHITE, 1.0, Align::Center, "Done");
        (g.p, g.l)
    }
}

/// A tab of the gallery (relative to the gallery's top-left).
fn gallery_tab(i: usize) -> [f32; 4] {
    [24.0 + i as f32 * 150.0, 18.0, 140.0, 32.0]
}

/// The icon and colour a desktop widget has in the gallery.
fn widget_look(kind: &str) -> (&'static str, [f32; 3]) {
    match kind {
        "clock" | "glassclock" => ("clock", [0.30, 0.33, 0.42]),
        "calendar" => ("calendar", [1.0, 0.32, 0.32]),
        "system" => ("monitor", [0.15, 0.58, 1.0]),
        "disk" => ("hard-drive", [0.56, 0.58, 0.63]),
        "weather" => ("sun", [1.0, 0.62, 0.12]),
        "music" => ("music-2", [1.0, 0.30, 0.40]),
        "vinyl" => ("audio-waveform", [0.46, 0.32, 0.95]),
        _ => ("layout-grid", [0.30, 0.33, 0.42]),
    }
}

/// A control's cell in the gallery (relative to the gallery's top-left): two columns, reading order.
/// A desktop widget's card in the gallery: two columns of cards with a small picture of the widget.
fn widget_cell(i: usize) -> [f32; 4] {
    [24.0 + (i % 2) as f32 * 258.0, 72.0 + (i / 2) as f32 * 88.0, 246.0, 80.0]
}

fn gallery_cell(i: usize) -> [f32; 4] {
    [24.0 + (i % 2) as f32 * 258.0, 72.0 + (i / 2) as f32 * 52.0, 246.0, 44.0]
}

/// Gallery builder: positions are relative to the gallery's top-left; everything scales about its centre.
struct Gal {
    p: Vec<Panel>,
    l: Vec<Label>,
    pivot: [f32; 2],
    k: f32,
    /// static shrink factor for small screens; text follows it (but not the appear animation)
    fit: f32,
}

impl Gal {
    fn map(&self, x: f32, y: f32) -> [f32; 2] {
        let (ax, ay) = (GX + x, GY + y);
        [self.pivot[0] + (ax - self.pivot[0]) * self.k, self.pivot[1] + (ay - self.pivot[1]) * self.k]
    }

    /// Glass panel with its top-left at (x, y) (relative); returned for further tweaking and not yet pushed.
    fn glass(&self, x: f32, y: f32, w: f32, h: f32, r: f32, tint: [f32; 4]) -> Panel {
        let c = self.map(x + w / 2.0, y + h / 2.0);
        Panel::glass(c[0], c[1], w * self.k, h * self.k, r * self.k).tinted(tint)
    }

    /// Push a glass panel.
    fn glass_t(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, tint: [f32; 4]) -> usize {
        let mut p = self.glass(x, y, w, h, r, tint);
        p.params[2] = (h.min(w) / 2.0).min(16.0) * self.k; // bezel never wider than half the shape
        p.extra = [0.10, 14.0, 5.0, 0.0];
        self.p.push(p);
        self.p.len() - 1
    }

    fn solid(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, top: [f32; 3], bot: [f32; 3], alpha: f32) {
        let c = self.map(x + w / 2.0, y + h / 2.0);
        self.p.push(Panel::solid(c[0], c[1], w * self.k, h * self.k, r * self.k, top, bot).flat().alpha(alpha));
    }

    fn hairline(&mut self, x: f32, y: f32, w: f32) {
        self.solid(x, y, w, 1.0, 0.5, [0.35, 0.42, 0.55], [0.35, 0.42, 0.55], 0.28);
    }

    fn glyph(&mut self, cx: f32, cy: f32, size: f32, name: &str, c: [f32; 3], a: f32) {
        let m = self.map(cx, cy);
        self.p.push(Panel::glyph(m[0], m[1], size * self.k, name, c, a));
    }

    fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, c: [f32; 3], a: f32, align: Align, text: &str) {
        let m = self.map(x, y);
        self.l.push(Label::new(text, m[0], m[1], (size * self.fit * 2.0).round() / 2.0, bold, c, a, align));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc() -> ControlCenter {
        ControlCenter::new(1600.0, 900.0)
    }

    fn centre(cc: &ControlCenter, spec: usize) -> [f32; 2] {
        cc.tiles[cc.spec_tile[spec]].c
    }

    fn kinds(cc: &ControlCenter) -> Vec<&str> {
        cc.layout.iter().map(|t| t.kind.as_str()).collect()
    }

    /// The dropdown open, the pointer pressed on `at` and released at `to` (the frames in between are what drags).
    fn press_release(cc: &mut ControlCenter, at: [f32; 2], to: [f32; 2]) {
        cc.mouse_down(at);
        cc.update(0.016, &Default::default(), to, true, false);
        cc.update(0.016, &Default::default(), to, false, false);
    }

    #[test]
    fn a_tile_runs_its_action() {
        let mut cc = cc();
        cc.set_open(true);
        let dark = cc.layout.iter().position(|t| t.opt("action") == Some("dark-mode")).unwrap();
        cc.mouse_down(centre(&cc, dark));
        assert_eq!(cc.take_events(), vec![CcEvent::Action("dark-mode".into())]);
        // dark mode keeps the dropdown open, a screenshot closes it
        assert!(cc.open);
        let shot = cc.layout.iter().position(|t| t.opt("action") == Some("screenshot")).unwrap();
        cc.mouse_down(centre(&cc, shot));
        assert_eq!(cc.take_events(), vec![CcEvent::Action("screenshot".into())]);
        assert!(!cc.open);
    }

    #[test]
    fn the_player_buttons_follow_the_look() {
        let mut cc = cc();
        cc.set_open(true);
        let media = cc.layout.iter().position(|t| t.kind == "media").unwrap();
        let c = centre(&cc, media);
        // the player card has its play button 53 px below the middle
        cc.mouse_down([c[0], c[1] + 53.0]);
        assert_eq!(cc.take_events(), vec![CcEvent::Media(CcMediaCmd::PlayPause)]);
        cc.layout[media].opts.insert("look".into(), toml::Value::String("half-record".into()));
        cc.rebuild();
        cc.mouse_down([c[0] - 46.0, c[1] - 17.0]);
        assert_eq!(cc.take_events(), vec![CcEvent::Media(CcMediaCmd::Previous)]);
    }

    #[test]
    fn a_control_from_the_gallery_is_added_in_the_first_free_place() {
        let mut cc = cc();
        cc.set_edit(true);
        let n = cc.layout.len();
        let terminal = cctiles::PRESETS.iter().position(|p| p.label == "Terminal").unwrap();
        let [x, y, w, h] = gallery_cell(terminal);
        let (c, _) = cc.mapped(x, y, w, h);
        assert!(cc.mouse_down(c));
        assert_eq!(cc.layout.len(), n + 1);
        let added = cc.layout.last().unwrap();
        assert_eq!((added.kind.as_str(), added.opt("action")), ("toggle", Some("terminal")));
        assert_eq!((added.col, added.row), (2, 3));
        assert!(matches!(cc.take_events().as_slice(), [CcEvent::Layout(l)] if l.len() == n + 1));
        // and it is built: it has a tile you can press
        assert_eq!(cc.spec_tile.len(), n + 1);
    }

    #[test]
    fn the_minus_badge_removes_a_tile() {
        let mut cc = cc();
        cc.set_edit(true);
        let before = kinds(&cc).len();
        let bt = cc.layout.iter().position(|t| t.kind == "bluetooth").unwrap();
        let tile = &cc.tiles[cc.spec_tile[bt]];
        let badge = [tile.c[0] - tile.size[0] / 2.0 + 7.0, tile.c[1] - tile.size[1] / 2.0 + 7.0];
        assert!(cc.mouse_down(badge));
        assert_eq!(cc.layout.len(), before - 1);
        assert!(!kinds(&cc).contains(&"bluetooth"));
        assert!(matches!(cc.take_events().as_slice(), [CcEvent::Layout(_)]));
    }

    #[test]
    fn tapping_a_tile_in_edit_mode_changes_its_look() {
        let mut cc = cc();
        cc.set_edit(true);
        let media = cc.layout.iter().position(|t| t.kind == "media").unwrap();
        let c = centre(&cc, media);
        press_release(&mut cc, c, c);
        assert_eq!(cc.layout[media].opt("look"), Some("record"));
        assert!(cc.wants_record());
        assert!(matches!(cc.take_events().as_slice(), [CcEvent::Layout(_)]));
    }

    #[test]
    fn dragging_a_tile_drops_it_on_the_nearest_free_cell() {
        let mut cc = cc();
        cc.set_edit(true);
        let dark = cc.layout.iter().position(|t| t.opt("action") == Some("dark-mode")).unwrap();
        assert_eq!((cc.layout[dark].col, cc.layout[dark].row), (0, 3));
        let c = centre(&cc, dark);
        // two cells to the right: (2, 3) is free
        let to = [c[0] + 2.0 * (cctiles::CELL_W + cctiles::GAP), c[1]];
        press_release(&mut cc, c, to);
        assert_eq!((cc.layout[dark].col, cc.layout[dark].row), (2, 3));
        assert!(matches!(cc.take_events().as_slice(), [CcEvent::Layout(_)]));
        // dropped on a taken cell: it stays where it is
        let c = centre(&cc, dark);
        let wifi = centre(&cc, 0);
        press_release(&mut cc, c, wifi);
        assert_eq!((cc.layout[dark].col, cc.layout[dark].row), (2, 3));
        assert!(cc.take_events().is_empty());
    }

    #[test]
    fn a_layout_from_the_config_replaces_the_default() {
        let mut cc = cc();
        let own = vec![CcTile::new("toggle", 3, 0).with("icon", "terminal").with("action", "terminal")];
        cc.set_layout(&own);
        assert_eq!(cc.layout, own);
        assert_eq!(cc.spec_tile.len(), 1);
        // an empty list means the built-in layout again
        cc.set_layout(&[]);
        assert_eq!(cc.layout, cctiles::default_layout());
        // and a kind this version does not know is kept, not dropped
        let odd = vec![CcTile::new("from-the-future", 0, 0)];
        cc.set_layout(&odd);
        assert_eq!(cc.layout, odd);
        assert_eq!(cc.spec_tile.len(), 1);
    }

    #[test]
    fn the_empty_cells_are_drawn_under_every_tile_even_the_one_being_dragged() {
        let mut cc = cc();
        cc.set_edit(true);
        let dark = cc.layout.iter().position(|t| t.opt("action") == Some("dark-mode")).unwrap();
        let c = centre(&cc, dark);
        // pick the tile up and carry it well away from where it was
        cc.mouse_down(c);
        cc.update(0.016, &Default::default(), [c[0] + 120.0, c[1] - 90.0], true, false);
        assert!(cc.press.is_some_and(|p| p.moved));
        let is_slot = |p: &Panel| p.tint == [0.80, 0.88, 1.0, 0.20];
        let panels = cc.panels();
        let first_other = panels.iter().position(|p| !is_slot(p)).unwrap();
        assert!(first_other > 0, "there are free cells in edit mode");
        assert!(panels[first_other..].iter().all(|p| !is_slot(p)), "a free cell is drawn above a tile");
    }

    #[test]
    fn text_of_other_tiles_does_not_show_through_the_tile_being_carried() {
        let mut cc = cc();
        cc.set_edit(true);
        // let the dropdown finish opening (its text only appears once it is nearly there)
        for _ in 0..90 {
            cc.update(0.016, &Default::default(), [0.0, 0.0], false, false);
        }
        let media = cc.layout.iter().position(|t| t.kind == "media").unwrap();
        let bt_c = centre(&cc, cc.layout.iter().position(|t| t.kind == "bluetooth").unwrap());
        // (the gallery on the left has a "Bluetooth" of its own; the tile is on the right)
        let shows = |cc: &ControlCenter| cc.labels().iter().any(|l| l.text == "Bluetooth" && l.x > 800.0);
        assert!(shows(&cc));
        // carry the big Now Playing card over the Bluetooth pill
        let c = centre(&cc, media);
        cc.mouse_down(c);
        cc.update(0.016, &Default::default(), [bt_c[0] + 40.0, bt_c[1]], true, false);
        assert!(cc.press.is_some_and(|p| p.moved));
        assert!(!shows(&cc), "the Bluetooth label shows through the carried tile");
        // the carried tile keeps its own text
        assert!(cc.labels().iter().any(|l| l.text == "record" || l.text == "player" || l.text == "Not Playing"));
    }

    #[test]
    fn the_gallery_has_a_tab_for_desktop_widgets() {
        let mut cc = cc();
        cc.set_edit(true);
        assert_eq!(cc.gallery_tab, 0);
        let [x, y, w, h] = gallery_tab(1);
        let (c, _) = cc.mapped(x, y, w, h);
        assert!(cc.mouse_down(c));
        assert_eq!(cc.gallery_tab, 1);
        // a tap on a widget asks the shell to put it on the desktop; no tile is added
        let n = cc.layout.len();
        let vinyl = crate::deskwidgets::KINDS.iter().position(|(k, ..)| *k == "vinyl").unwrap();
        let [x, y, w, h] = widget_cell(vinyl);
        let (c, _) = cc.mapped(x, y, w, h);
        assert!(cc.mouse_down(c));
        assert_eq!(cc.take_events(), vec![CcEvent::AddWidget("vinyl".into())]);
        assert_eq!(cc.layout.len(), n);
        // back to the controls
        let [x, y, w, h] = gallery_tab(0);
        let (c, _) = cc.mapped(x, y, w, h);
        cc.mouse_down(c);
        assert_eq!(cc.gallery_tab, 0);
    }
}
