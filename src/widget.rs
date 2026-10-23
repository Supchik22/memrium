//! Panels and widgets: the building blocks of the shell.
//!
//! A *bar* (panel) sits on a screen edge as a full-width strip or a floating pill and holds *widgets* in three
//! zones (left / centre / right). The macOS menu bar and the Dock are both just bars with different widgets and a
//! different look, and every part of them is listed in the theme's `[[bars]]` or in `config.toml`:
//!
//! ```toml
//! [[override.bars]]
//! name = "menubar"
//! edge = "top"
//! shape = "strip"
//! left  = ["logo", "title", "app-menu"]
//! right = ["battery", "wifi", { kind = "button", icon = "camera", action = "screenshot" }, "control-center", "clock"]
//! [override.bars.look]
//! kind = "fade"          # glass | solid | fade (blur that fades out downwards) | clear
//! ```
//!
//! A widget measures itself, draws itself into a rectangle and may report a click as a [`BarClick`].
//! New widgets are added by implementing [`Widget`] and registering the name in [`create`].

use crate::renderer::{Align, Label, Panel};
use crate::scene::DockItem;
use chebrium::spring::Spring;
use crate::theme::{Edge, GlassStyle, MenubarKind, MenubarStyle, Theme};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------------------- config

/// One entry in a zone: a bare name (`"clock"`) or a table with options (`{ kind = "clock", format = "%H:%M" }`).
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSpec {
    pub kind: String,
    pub opts: toml::Table,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SpecRepr {
    Name(String),
    Full(toml::Table),
}

impl From<SpecRepr> for WidgetSpec {
    fn from(r: SpecRepr) -> Self {
        match r {
            SpecRepr::Name(kind) => WidgetSpec { kind, opts: toml::Table::new() },
            SpecRepr::Full(mut opts) => {
                let kind = opts.remove("kind").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
                WidgetSpec { kind, opts }
            }
        }
    }
}

impl From<WidgetSpec> for SpecRepr {
    fn from(s: WidgetSpec) -> Self {
        if s.opts.is_empty() {
            SpecRepr::Name(s.kind)
        } else {
            let mut t = s.opts;
            t.insert("kind".into(), toml::Value::String(s.kind));
            SpecRepr::Full(t)
        }
    }
}

impl Serialize for WidgetSpec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        SpecRepr::from(self.clone()).serialize(s)
    }
}

impl<'de> Deserialize<'de> for WidgetSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        SpecRepr::deserialize(d).map(Into::into)
    }
}

impl WidgetSpec {
    pub fn new(kind: &str) -> Self {
        WidgetSpec { kind: kind.into(), opts: toml::Table::new() }
    }

    pub(crate) fn str(&self, key: &str) -> Option<&str> {
        self.opts.get(key).and_then(|v| v.as_str())
    }

    pub(crate) fn bool(&self, key: &str) -> Option<bool> {
        self.opts.get(key).and_then(|v| v.as_bool())
    }

    pub(crate) fn strings(&self, key: &str) -> Option<Vec<String>> {
        self.opts.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
    }

    pub(crate) fn num(&self, key: &str) -> Option<f32> {
        self.opts.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map(|f| f as f32)
    }
}

/// How a bar sits on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BarShape {
    /// full width, flat
    #[default]
    Strip,
    /// a glass pill that wraps its widgets, centred
    Floating,
}

/// What a strip bar is made of: the flat menu-bar look, or a full glass panel (a taskbar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BarSurface {
    #[default]
    Look,
    Glass,
}

/// When a bar gets out of the way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoHide {
    /// always there, and windows keep out of its space
    #[default]
    Off,
    /// hidden until the pointer touches its edge of the screen
    Always,
    /// hidden only while a window is in its way (shown again when nothing covers it, or at the edge)
    Smart,
}

/// One bar: where it is, what it looks like and which widgets it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub name: String,
    pub edge: Edge,
    pub shape: BarShape,
    /// strip height; `None` = the theme's menu bar height
    pub height: Option<f32>,
    /// gap between a floating bar and the screen edge
    pub margin: f32,
    /// strip background; `None` = the theme's `[menubar]` look
    pub look: Option<MenubarStyle>,
    /// `look` (flat menu-bar band) or `glass` (a glass panel with its own radius)
    pub surface: BarSurface,
    /// glass of a `surface = "glass"` bar; `None` = the theme's glass
    pub glass: Option<GlassStyle>,
    /// gap between a strip and the screen edges
    pub inset: f32,
    /// corner radius of a glass strip
    pub radius: f32,
    /// hides itself (see `AutoHide`); a bar that hides takes no space from windows
    pub autohide: AutoHide,
    pub left: Vec<WidgetSpec>,
    pub center: Vec<WidgetSpec>,
    pub right: Vec<WidgetSpec>,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig { name: "bar".into(), edge: Edge::Top, shape: BarShape::Strip, height: None, margin: 14.0, look: None, surface: BarSurface::Look, glass: None, inset: 0.0, radius: 0.0, autohide: AutoHide::Off, left: Vec::new(), center: Vec::new(), right: Vec::new() }
    }
}

impl BarConfig {
    pub fn zone_mut(&mut self, z: usize) -> &mut Vec<WidgetSpec> {
        match z {
            0 => &mut self.left,
            1 => &mut self.center,
            _ => &mut self.right,
        }
    }
}

/// The applications' own icons only (the machine's status icons have widgets of their own).
fn tray_apps() -> WidgetSpec {
    let mut s = WidgetSpec::new("tray");
    for k in ["chevron", "wifi", "volume", "battery", "bluetooth"] {
        s.opts.insert(k.into(), toml::Value::Boolean(false));
    }
    s
}

/// The macOS arrangement: a menu bar on top and the Dock at the bottom.
pub fn default_bars() -> Vec<BarConfig> {
    let w = WidgetSpec::new;
    vec![
        BarConfig {
            name: "menubar".into(),
            left: vec![w("logo"), w("window-buttons"), w("title"), w("app-menu")],
            right: vec![tray_apps(), w("keyboard"), w("notifications"), w("battery"), w("wifi"), w("search"), w("control-center"), w("clock")],
            ..Default::default()
        },
        // the Dock slides away until the pointer touches the bottom edge (the Jelly Glass look: windows get the whole screen)
        BarConfig { name: "dock".into(), edge: Edge::Bottom, shape: BarShape::Floating, center: vec![w("dock")], autohide: AutoHide::Always, ..Default::default() },
    ]
}

/// Widgets that can be added from the edit menu: (kind, label).
pub const CATALOG: &[(&str, &str)] = &[
    ("logo", "Logo menu"),
    ("title", "App name"),
    ("app-menu", "App menus"),
    ("window-buttons", "Window buttons"),
    ("clock", "Clock"),
    ("battery", "Battery"),
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("volume", "Volume"),
    ("search", "Search"),
    ("control-center", "Quick Settings"),
    ("dock", "Dock (apps)"),
    ("start", "Start button"),
    ("tasks", "Task buttons"),
    ("search-box", "Search box"),
    ("tray", "Tray icons"),
    ("keyboard", "Keyboard layout"),
    ("notifications", "Notifications"),
    ("show-desktop", "Show desktop"),
    ("workspaces", "Desktops"),
    ("button", "Button"),
    ("text", "Text"),
    ("command", "Command output"),
    ("divider", "Divider"),
    ("spacer", "Spacer"),
];

// ---------------------------------------------------------------------------------------- context

/// A pointer position on the bars: which bar, which widget, and which part of it (a menu title, an icon).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarHit {
    pub bar: usize,
    pub widget: usize,
    pub part: usize,
}

/// The Dock / taskbar item under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct DockHover {
    pub key: String,
    /// the item's rectangle on screen
    pub rect: [f32; 4],
    /// the widget wants window previews over this item
    pub previews: bool,
}

/// What a click on a bar asks the compositor to do.
#[derive(Clone, Debug, PartialEq)]
pub enum BarClick {
    /// open the drop-down menu called `name` below `x`
    Menu { name: String, x: f32 },
    /// run a named action (`launcher`, `settings`, `files`, `terminal`, `screenshot`, `quit`, ...)
    Action(String),
    /// a Dock item (its key)
    Dock(String),
}

/// Everything a widget may look at while measuring and drawing.
#[derive(Clone)]
pub struct BarCtx<'a> {
    pub theme: &'a Theme,
    pub screen: [f32; 2],
    pub measure: &'a dyn Fn(&str, f32, bool) -> f32,
    pub mouse: [f32; 2],
    /// the wallpaper behind the menu bar is dark (ink turns light)
    pub menu_light: bool,
    /// name of the focused app
    pub title: &'a str,
    /// titles of the focused app's menus (File, Edit, ...)
    pub menus: &'a [String],
    pub hover: Option<BarHit>,
    pub open: Option<BarHit>,
    /// the start menu is open (the start button stays lit)
    pub start_open: bool,
    /// the current virtual desktop
    pub workspace: usize,
    /// frames of the windows on screen (this monitor's coordinates): a bar that hides when covered looks at them
    pub windows: &'a [[f32; 4]],
    /// volume, Wi-Fi, Bluetooth, battery as the machine reports them
    pub sys: &'a crate::sysctl::SysInfo,
    /// the bar being laid out runs top to bottom (it is on the left or right of the screen): widgets that are lines of
    /// text leave it out or stack their lines
    pub vertical: bool,
    /// how much of the screen's height the bars along the top and bottom take: a side bar runs between them
    pub axis: [f32; 2],
    /// the icons applications have put in the tray (the ones shown now)
    pub tray: &'a [crate::tray::TrayIcon],
    /// the active keyboard layout's short name (`UA`); empty when there is only one layout
    pub layout: &'a str,
    /// notifications not looked at yet, and whether do-not-disturb is on (the bell)
    pub notif_unread: usize,
    pub dnd: bool,
    // filled per bar by `Bar`
    pub ink: [f32; 3],
    /// backdrop tint under the bar, for ink that adapts to the wallpaper
    pub under: [f32; 4],
    /// only glass bars darken what is behind them
    pub adaptive: bool,
    /// ink is fixed (light / dark theme), not picked from the wallpaper (taskbars)
    pub fixed_ink: bool,
    /// the window in front is maximized with its title bar left out: its buttons are in the bar
    pub window_buttons: bool,
}

// ---------------------------------------------------------------------------------------- widgets

#[derive(Default)]
pub struct Drawn {
    pub panels: Vec<Panel>,
    pub labels: Vec<Label>,
}

/// How a widget relates to the Quick Settings dropdown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CcRole {
    None,
    /// the Quick Settings draws its pill here
    Pill,
    /// clicking the widget opens the dropdown, which hangs from it
    Anchor,
}

pub trait Widget {
    fn kind(&self) -> &'static str;
    fn width(&self, c: &BarCtx) -> f32;
    /// natural height (floating bars wrap it); 0 = fills the bar
    fn height(&self, _c: &BarCtx) -> f32 {
        0.0
    }
    /// how far the widget may draw above its bar (magnified Dock icons, tooltips)
    fn overflow(&self, _c: &BarCtx) -> f32 {
        0.0
    }
    /// widths of the clickable parts, left to right; empty = not clickable
    fn parts(&self, _c: &BarCtx) -> Vec<f32> {
        Vec::new()
    }
    /// draw into `r` = (x, bar y, widget width, bar height); `lit` = the part under the pointer / holding an open menu
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn);
    fn click(&mut self, _c: &BarCtx, _r: [f32; 4], _part: usize, _x: f32) -> Option<BarClick> {
        None
    }
    /// does this widget host the Quick Settings (draw its pill) or only open it?
    fn cc(&self) -> CcRole {
        CcRole::None
    }
    fn update(&mut self, _dt: f32, _c: &BarCtx, _r: [f32; 4]) {}
    fn set_dock(&mut self, _items: &[DockItem], _focused: Option<&str>) {}
    fn item_center(&self, _key: &str) -> Option<[f32; 2]> {
        None
    }
    /// The Dock item under the pointer and its rectangle on screen (for previews and the context menu).
    fn item_at(&self, _c: &BarCtx, _r: [f32; 4]) -> Option<DockHover> {
        None
    }
    /// jelly stretch of the bar's background (the Dock)
    fn pulse(&self) -> f32 {
        0.0
    }
}

pub(crate) fn highlight(c: &BarCtx, r: [f32; 4], out: &mut Drawn, x: f32, w: f32) {
    let ink = c.ink;
    out.panels.push(Panel::solid(x + w / 2.0, r[1] + r[3] / 2.0, w - 4.0, r[3] - 6.0, 7.0, ink, ink).flat().alpha(0.14));
}

fn label(c: &BarCtx, r: [f32; 4], out: &mut Drawn, text: &str, x: f32, size: f32, bold: bool, align: Align) {
    let y = r[1] + r[3] / 2.0 + 4.8;
    let l = Label::new(text, x, y, size, bold, c.ink, 0.92, align);
    out.labels.push(if c.fixed_ink { l } else { l.auto_ink(c.under).flat(!c.adaptive) });
}

fn label_at(c: &BarCtx, out: &mut Drawn, text: &str, x: f32, y: f32, size: f32, alpha: f32) {
    let l = Label::new(text, x, y, size, false, c.ink, alpha, Align::Right);
    out.labels.push(if c.fixed_ink { l } else { l.auto_ink(c.under).flat(!c.adaptive) });
}

pub(crate) fn glyph(c: &BarCtx, r: [f32; 4], out: &mut Drawn, name: &str, cx: f32, size: f32) {
    let mut g = Panel::glyph(cx, r[1] + r[3] / 2.0, size, name, c.ink, 0.85);
    if !c.fixed_ink {
        g = g.auto_ink(c.under);
    }
    if !c.adaptive {
        g.fx[2] = 1.0;
    }
    out.panels.push(g);
}

fn show_title(c: &BarCtx) -> bool {
    c.theme.menubar.show_title && !c.vertical
}

/// The system logo; opens the system menu.
struct Logo {
    icon: String,
}

impl Widget for Logo {
    fn kind(&self) -> &'static str {
        "logo"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if show_title(c) { 40.0 } else { 0.0 }
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if show_title(c) { vec![40.0] } else { Vec::new() }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if !show_title(c) {
            return;
        }
        if lit.is_some() {
            highlight(c, r, out, r[0] + 3.0, 34.0);
        }
        glyph(c, r, out, &self.icon, r[0] + 16.0, 17.0);
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, x: f32) -> Option<BarClick> {
        Some(BarClick::Menu { name: "logo".into(), x })
    }
}

/// Name of the focused app, bold.
struct Title;

impl Widget for Title {
    fn kind(&self) -> &'static str {
        "title"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if show_title(c) { (c.measure)(c.title, 13.5, true) + 12.0 } else { 0.0 }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        if show_title(c) {
            label(c, r, out, c.title, r[0], 13.5, true, Align::Left);
        }
    }
}

/// The focused app's menus.
struct AppMenu;

impl Widget for AppMenu {
    fn kind(&self) -> &'static str {
        "app-menu"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        self.parts(c).iter().sum()
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if !show_title(c) {
            return Vec::new();
        }
        c.menus.iter().map(|m| (c.measure)(m, 13.5, false) + 22.0).collect()
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        let mut px = r[0];
        for (i, (m, w)) in c.menus.iter().zip(self.parts(c)).enumerate() {
            if lit == Some(i) {
                highlight(c, r, out, px, w);
            }
            label(c, r, out, m, px + 11.0, 13.5, false, Align::Left);
            px += w;
        }
    }
    fn click(&mut self, c: &BarCtx, _: [f32; 4], part: usize, x: f32) -> Option<BarClick> {
        c.menus.get(part).map(|m| BarClick::Menu { name: m.clone(), x })
    }
}

/// Close, minimize and zoom of a maximized window whose title bar is left out (`window.hide_title_max`). It takes no room
/// while there is no such window.
struct WindowButtons;

const WB_PITCH: f32 = 21.0;

impl Widget for WindowButtons {
    fn kind(&self) -> &'static str {
        "window-buttons"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if c.window_buttons { 3.0 * WB_PITCH + 6.0 } else { 0.0 }
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if c.window_buttons { vec![WB_PITCH; 3] } else { Vec::new() }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if !c.window_buttons {
            return;
        }
        let lights: [[f32; 3]; 3] = [[1.0, 0.37, 0.34], [1.0, 0.75, 0.18], [0.16, 0.80, 0.26]];
        let glyphs = ["x", "minus", "plus"];
        for (i, col) in lights.iter().enumerate() {
            let (cx, cy) = (r[0] + 10.0 + i as f32 * WB_PITCH, r[1] + r[3] / 2.0);
            let hi = [col[0] * 0.8 + 0.2, col[1] * 0.8 + 0.2, col[2] * 0.8 + 0.2];
            out.panels.push(Panel::solid(cx, cy, 13.0, 13.0, 6.5, hi, *col).flat());
            if lit.is_some() {
                out.panels.push(Panel::glyph(cx, cy, 9.0, glyphs[i], [0.25, 0.08, 0.06], 0.7));
            }
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], part: usize, _: f32) -> Option<BarClick> {
        ["win-close", "win-minimize", "win-zoom"].get(part).map(|a| BarClick::Action((*a).to_string()))
    }
}

/// An icon, optionally a button that runs an action.
struct Icon {
    kind: &'static str,
    icon: String,
    /// the config named an icon: it is used as it is
    custom: bool,
    size: f32,
    action: Option<String>,
}

impl Widget for Icon {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn width(&self, _: &BarCtx) -> f32 {
        36.0
    }
    fn parts(&self, _: &BarCtx) -> Vec<f32> {
        if self.action.is_some() { vec![36.0] } else { Vec::new() }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if lit.is_some() {
            highlight(c, r, out, r[0], r[2]);
        }
        glyph(c, r, out, &self.icon_for(c.sys), r[0] + r[2] / 2.0, self.size);
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        self.action.clone().map(BarClick::Action)
    }
}

impl Icon {
    /// The icon to show: a status widget follows the machine unless the config picked an icon.
    fn icon_for(&self, sys: &crate::sysctl::SysInfo) -> String {
        if self.custom {
            self.icon.clone()
        } else {
            status_icon(&self.icon, sys)
        }
    }
}

/// A status icon (`wifi`, `bluetooth`, `volume-2`, `battery-full`, or any of their variants) as the machine's state
/// calls for it: Wi-Fi off, Bluetooth off, muted, a battery that is low or charging.
pub fn status_icon(name: &str, sys: &crate::sysctl::SysInfo) -> String {
    match name.split('-').next().unwrap_or("") {
        "wifi" => match &sys.wifi {
            Some(w) if !w.enabled => "wifi-off",
            _ => "wifi",
        },
        "bluetooth" => match &sys.bluetooth {
            Some(b) if !b.powered => "bluetooth-off",
            _ => "bluetooth",
        },
        "volume" => match (sys.volume, sys.muted) {
            (_, true) => "volume-x",
            (Some(v), _) if v < 0.34 => "volume-1",
            _ => "volume-2",
        },
        "battery" => match sys.battery {
            Some((_, true)) => "battery-charging",
            Some((p, _)) if p < 20 => "battery-low",
            Some((p, _)) if p < 60 => "battery-medium",
            _ => "battery-full",
        },
        _ => return name.to_string(),
    }
    .to_string()
}

/// Date and time; `format` understands %a %A %b %B %d %e %H %I %l %M %S %p %P %Y %y %m %%.
struct Clock {
    /// the widget has a format of its own (otherwise the preferences decide: 24-hour, seconds, the date)
    own_format: bool,
    format: String,
    /// optional second line (date under the time)
    format2: Option<String>,
    action: Option<String>,
}

impl Clock {
    /// The format to show: the widget's own, or the one the preferences make (two-line clocks keep time alone on the top
    /// line, the date being on the second).
    fn fmt(&self, c: &BarCtx) -> String {
        if self.own_format {
            return self.format.clone();
        }
        let mut p = c.theme.ui.clock;
        if self.format2.is_some() {
            p.date = false;
        }
        p.format()
    }
}

impl Widget for Clock {
    fn kind(&self) -> &'static str {
        "clock"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        // on a side bar the time stands over the date, short
        if c.vertical {
            return if self.format2.is_some() { 46.0 } else { 30.0 };
        }
        let one = (c.measure)(&format_time(&self.fmt(c)), 12.5, self.format2.is_none());
        match &self.format2 {
            Some(f2) => one.max((c.measure)(&format_time(f2), 11.5, false)) + 14.0,
            None => one + 12.0,
        }
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if self.action.is_some() { vec![self.width(c)] } else { Vec::new() }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if lit.is_some() {
            highlight(c, r, out, r[0], r[2]);
        }
        if c.vertical {
            // (a label's place along the bar is its x; it is centred across the bar when turned upright)
            let mut line = |text: String, x: f32, size: f32, alpha: f32| {
                let l = Label::new(text, x, r[1] + r[3] / 2.0, size, false, c.ink, alpha, Align::Center);
                out.labels.push(if c.fixed_ink { l } else { l.auto_ink(c.under).flat(!c.adaptive) });
            };
            let time = if self.format.contains("%H") || self.format.contains("%I") || self.format.contains("%l") { "%H:%M" } else { &self.format };
            match &self.format2 {
                Some(_) => {
                    line(format_time(time), r[0] + 14.0, 12.5, 0.95);
                    line(format_time("%d.%m"), r[0] + 33.0, 11.0, 0.8);
                }
                None => line(format_time(time), r[0] + r[2] / 2.0, 12.5, 0.95),
            }
            return;
        }
        match &self.format2 {
            None => label(c, r, out, &format_time(&self.fmt(c)), r[0] + 6.0, 12.5, true, Align::Left),
            Some(f2) => {
                let cy = r[1] + r[3] / 2.0;
                let right = r[0] + r[2] - 6.0;
                label_at(c, out, &format_time(&self.fmt(c)), right, cy - 3.5, 12.5, 0.95);
                label_at(c, out, &format_time(f2), right, cy + 12.0, 11.5, 0.85);
            }
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        self.action.clone().map(BarClick::Action)
    }
}

/// The keyboard layout in use (`EN`, `UA`); a click opens the list. Nothing when there is only one layout.
struct Keyboard {
    action: Option<String>,
}

impl Widget for Keyboard {
    fn kind(&self) -> &'static str {
        "keyboard"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if c.layout.is_empty() || c.vertical { 0.0 } else { (c.measure)(c.layout, 12.5, true) + 16.0 }
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if c.layout.is_empty() || c.vertical { Vec::new() } else { vec![self.width(c)] }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if c.layout.is_empty() || c.vertical {
            return;
        }
        if lit.is_some() {
            highlight(c, r, out, r[0], r[2]);
        }
        label(c, r, out, c.layout, r[0] + r[2] / 2.0, 12.5, true, Align::Center);
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        self.action.clone().map(BarClick::Action)
    }
}

/// The bell of the notification centre: a dot with the number of what came while it was shut, a crossed bell while
/// do-not-disturb is on.
struct Bell {
    size: f32,
    action: Option<String>,
}

impl Widget for Bell {
    fn kind(&self) -> &'static str {
        "notifications"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        36.0
    }
    fn parts(&self, _: &BarCtx) -> Vec<f32> {
        vec![36.0]
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if lit.is_some() {
            highlight(c, r, out, r[0], r[2]);
        }
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        glyph(c, r, out, if c.dnd { "bell-off" } else { "bell" }, cx, self.size);
        if c.notif_unread > 0 && !c.dnd {
            out.panels.push(Panel::solid(cx + 6.0, cy - 6.0, 8.0, 8.0, 4.0, [1.0, 0.3, 0.3], [1.0, 0.3, 0.3]).flat());
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        self.action.clone().map(BarClick::Action)
    }
}

/// Fixed text.
struct Text {
    text: String,
    bold: bool,
}

impl Widget for Text {
    fn kind(&self) -> &'static str {
        "text"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if c.vertical { 0.0 } else { (c.measure)(&self.text, 13.0, self.bold) + 12.0 }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        if c.vertical {
            return;
        }
        label(c, r, out, &self.text, r[0] + 6.0, 13.0, self.bold, Align::Left);
    }
}

/// The first line a shell command prints, refreshed every few seconds (weather, a counter, a script of your own).
struct Command {
    out: std::sync::Arc<std::sync::Mutex<String>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    icon: Option<String>,
    size: f32,
    bold: bool,
    max_w: f32,
    action: Option<String>,
}

impl Command {
    fn new(spec: &WidgetSpec, action: Option<String>) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let out = Arc::new(Mutex::new(String::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let cmd = spec.s("command");
        let every = spec.n("interval").max(1.0);
        if !cmd.trim().is_empty() {
            let (out, stop) = (out.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let text = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .stdin(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .output()
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string())
                        .unwrap_or_default();
                    if let Ok(mut g) = out.lock() {
                        *g = text;
                    }
                    let mut waited = 0.0;
                    while waited < every && !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        waited += 0.25;
                    }
                }
            });
        }
        let icon = spec.str("icon").filter(|i| !i.is_empty() && *i != "none").map(str::to_string);
        Command { out, stop, icon, size: spec.n("size"), bold: spec.flag("bold"), max_w: spec.n("max_width"), action }
    }

    fn text(&self) -> String {
        self.out.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for Command {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Widget for Command {
    fn kind(&self) -> &'static str {
        "command"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        if c.vertical {
            return 0.0;
        }
        let t = self.text();
        let tw = if t.is_empty() { 0.0 } else { (c.measure)(&t, 13.0, self.bold).min(self.max_w) };
        let iw = if self.icon.is_some() { self.size + if tw > 0.0 { 6.0 } else { 0.0 } } else { 0.0 };
        if tw + iw == 0.0 { 0.0 } else { tw + iw + 16.0 }
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        if self.action.is_some() { vec![self.width(c)] } else { Vec::new() }
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        if c.vertical {
            return;
        }
        if lit.is_some() {
            highlight(c, r, out, r[0], r[2]);
        }
        let mut x = r[0] + 8.0;
        if let Some(i) = &self.icon {
            glyph(c, r, out, i, x + self.size / 2.0, self.size);
            x += self.size + 6.0;
        }
        let mut t = self.text();
        while !t.is_empty() && (c.measure)(&t, 13.0, self.bold) > self.max_w {
            t.pop();
        }
        label(c, r, out, &t, x, 13.0, self.bold, Align::Left);
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        self.action.clone().map(BarClick::Action)
    }
}

/// Empty space.
struct Spacer(f32);

impl Widget for Spacer {
    fn kind(&self) -> &'static str {
        "spacer"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        self.0
    }
    fn draw(&self, _: &BarCtx, _: [f32; 4], _: Option<usize>, _: &mut Drawn) {}
}

/// A thin vertical divider.
struct Divider;

impl Widget for Divider {
    fn kind(&self) -> &'static str {
        "divider"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        13.0
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        let ink = c.ink;
        out.panels.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, 1.0, r[3] * 0.5, 0.5, ink, ink).flat().alpha(0.25));
    }
}

/// Reserves room for the Quick Settings's pill (the Quick Settings draws and handles the pill itself).
struct Pill;

impl Widget for Pill {
    fn kind(&self) -> &'static str {
        "control-center"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        62.0
    }
    fn draw(&self, _: &BarCtx, _: [f32; 4], _: Option<usize>, _: &mut Drawn) {}
    fn cc(&self) -> CcRole {
        CcRole::Pill
    }
}

/// Build a widget from its spec; `None` for an unknown name.
pub fn create(spec: &WidgetSpec) -> Option<Box<dyn Widget>> {
    let icon = |default: &str| spec.str("icon").unwrap_or(default).to_string();
    let action = match spec.str("action") {
        Some("none") | Some("") | None => None,
        Some(a) => Some(a.to_string()),
    };
    let ic = |kind: &'static str, default: &str, action: Option<String>| -> Box<dyn Widget> { Box::new(Icon { kind, icon: icon(default), custom: spec.str("icon").is_some(), size: spec.n("size"), action }) };
    Some(match spec.kind.as_str() {
        "logo" => Box::new(Logo { icon: icon("layout-grid") }),
        "title" => Box::new(Title),
        "app-menu" | "menus" => Box::new(AppMenu),
        "window-buttons" => Box::new(WindowButtons),
        "clock" => Box::new(Clock { own_format: !spec.s("format").is_empty(), format: Some(spec.s("format")).filter(|f| !f.is_empty()).unwrap_or_else(|| "%a %e %b %l:%M %P".to_string()), format2: Some(spec.s("format2")).filter(|f| !f.is_empty()), action: if spec.str("action").is_none() { Some("calendar".into()) } else { action } }),
        "battery" => ic("battery", "battery-full", if spec.str("action").is_none() { Some("power".into()) } else { action }),
        "wifi" => ic("wifi", "wifi", if spec.str("action").is_none() { Some("network".into()) } else { action }),
        "bluetooth" => ic("bluetooth", "bluetooth", if spec.str("action").is_none() { Some("bluetooth".into()) } else { action }),
        "volume" => ic("volume", "volume-2", if spec.str("action").is_none() { Some("audio".into()) } else { action }),
        "search" => ic("search", "search", if spec.str("action").is_none() { Some("search".into()) } else { action }),
        "button" | "icon" => ic("button", "app-window", action),
        "command" => Box::new(Command::new(spec, Some(spec.s("run")).filter(|r| !r.trim().is_empty()).map(|r| format!("exec:{}", r.trim())).or(action))),
        "text" => Box::new(Text { text: spec.str("text").unwrap_or("").to_string(), bold: spec.opts.get("bold").and_then(|v| v.as_bool()).unwrap_or(false) }),
        "spacer" => Box::new(Spacer(spec.n("width"))),
        "divider" | "separator" => Box::new(Divider),
        "control-center" | "cc" => Box::new(Pill),
        "dock" => Box::new(crate::dockwidget::DockWidget::with_spec(spec)),
        "start" => Box::new(crate::taskwidgets::Start::new(spec)),
        "tasks" => Box::new(crate::taskwidgets::Tasks::new(spec)),
        "search-box" => Box::new(crate::taskwidgets::SearchBox::new(spec)),
        "tray" => Box::new(crate::taskwidgets::Tray::new(spec)),
        "notifications" | "bell" => Box::new(Bell { size: spec.n("size").max(14.0), action: Some(action.clone().unwrap_or_else(|| "notifications".into())) }),
        "keyboard" | "layout" => Box::new(Keyboard { action: Some(action.clone().unwrap_or_else(|| "keyboard".into())) }),
        "show-desktop" => Box::new(crate::taskwidgets::ShowDesktop::new(spec)),
        "workspaces" => Box::new(crate::taskwidgets::Workspaces),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------- options

/// The kind of control a widget option needs in the settings.
#[derive(Clone, Copy, Debug)]
pub enum OptKind {
    /// min, max, default
    Num(f32, f32, f32),
    Bool(bool),
    /// (value, label) pairs and the default index
    Choice(&'static [(&'static str, &'static str)], usize),
    /// free text, with its default
    Text(&'static str),
}

/// One configurable option of a widget (`key` is the key in the widget's spec).
#[derive(Clone, Copy, Debug)]
pub struct Opt {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: OptKind,
}

const ACTIONS: &[(&str, &str)] = &[
    ("none", "Nothing"),
    ("search", "Search"),
    ("launcher", "Launcher"),
    ("settings", "Settings"),
    ("files", "Files"),
    ("terminal", "Terminal"),
    ("screenshot", "Screenshot"),
    ("control-center", "Quick Settings"),
    ("dark-mode", "Dark mode"),
    ("show-desktop", "Show desktop"),
    ("volume", "Volume slider"),
    ("brightness", "Brightness slider"),
    ("widgets", "Widgets panel"),
    ("power", "Power and battery"),
    ("calendar", "Calendar"),
    ("audio", "Sound mixer"),
    ("network", "Networks"),
    ("notifications", "Notification centre"),
    ("clipboard", "Clipboard history"),
    ("bluetooth", "Bluetooth devices"),
    ("mute", "Mute"),
    ("volume-up", "Volume up"),
    ("volume-down", "Volume down"),
    ("brightness-up", "Brightness up"),
    ("brightness-down", "Brightness down"),
    ("next-desktop", "Next desktop"),
    ("prev-desktop", "Previous desktop"),
];
const ICONS: &[(&str, &str)] = &[
    ("layout-grid", "Grid"),
    ("house", "Home"),
    ("search", "Search"),
    ("settings", "Gear"),
    ("terminal", "Terminal"),
    ("camera", "Camera"),
    ("star", "Star"),
    ("globe", "Globe"),
    ("power", "Power"),
    ("monitor", "Monitor"),
    ("app-window", "Window"),
];
const CMD_ICONS: &[(&str, &str)] = &[
    ("none", "None"),
    ("layout-grid", "Grid"),
    ("house", "Home"),
    ("search", "Search"),
    ("settings", "Gear"),
    ("terminal", "Terminal"),
    ("camera", "Camera"),
    ("star", "Star"),
    ("globe", "Globe"),
    ("power", "Power"),
    ("monitor", "Monitor"),
    ("app-window", "Window"),
];
const INDICATORS: &[(&str, &str)] = &[("underline", "Underline"), ("dot", "Dot"), ("glass", "Glass button")];
const DOCK_INDICATORS: &[(&str, &str)] = &[("dot", "Dot"), ("line", "Line"), ("glow", "Glow"), ("none", "None")];
const START_STYLES: &[(&str, &str)] = &[("logo", "Logo"), ("orb", "Orb")];

const fn num(key: &'static str, label: &'static str, min: f32, max: f32, def: f32) -> Opt {
    Opt { key, label, kind: OptKind::Num(min, max, def) }
}
const fn flag(key: &'static str, label: &'static str, def: bool) -> Opt {
    Opt { key, label, kind: OptKind::Bool(def) }
}
const fn text(key: &'static str, label: &'static str, def: &'static str) -> Opt {
    Opt { key, label, kind: OptKind::Text(def) }
}
const fn pick(key: &'static str, label: &'static str, choices: &'static [(&'static str, &'static str)], def: usize) -> Opt {
    Opt { key, label, kind: OptKind::Choice(choices, def) }
}

/// Options every interactive widget has: what the other mouse buttons and the wheel do, and a hover tooltip. An action
/// is a name from the list above, or `exec:` and a command line.
const COMMON: [Opt; 5] = [
    text("on_right", "Right click", ""),
    text("on_middle", "Middle click", ""),
    text("scroll_up", "Scroll up", ""),
    text("scroll_down", "Scroll down", ""),
    text("tooltip", "Tooltip", ""),
];

/// The options a widget kind understands, for the settings page.
pub fn options(kind: &str) -> Vec<Opt> {
    let mut v = kind_options(kind);
    if !matches!(kind, "spacer" | "divider" | "separator" | "dock" | "tasks" | "app-menu" | "menus" | "window-buttons") {
        v.extend(COMMON);
    }
    v
}

fn kind_options(kind: &str) -> Vec<Opt> {
    match kind {
        "logo" => vec![pick("icon", "Icon", ICONS, 0)],
        "clock" => vec![text("format", "Format", "%a %e %b %l:%M %P"), text("format2", "Second line", ""), pick("action", "On click", ACTIONS, 0)],
        "battery" => vec![num("size", "Icon size", 12.0, 28.0, 19.0), pick("action", "On click", ACTIONS, 0)],
        "wifi" | "bluetooth" | "volume" | "notifications" | "bell" => vec![num("size", "Icon size", 12.0, 28.0, 17.0), pick("action", "On click", ACTIONS, 0)],
        "search" => vec![num("size", "Icon size", 12.0, 28.0, 17.0), pick("action", "On click", ACTIONS, 1)],
        "button" => vec![pick("icon", "Icon", ICONS, 10), num("size", "Icon size", 12.0, 28.0, 17.0), pick("action", "On click", ACTIONS, 0)],
        "text" => vec![text("text", "Text", ""), flag("bold", "Bold", false)],
        "command" => vec![
            text("command", "Command", ""),
            num("interval", "Refresh (seconds)", 1.0, 3600.0, 10.0),
            pick("icon", "Icon", CMD_ICONS, 0),
            num("size", "Icon size", 12.0, 28.0, 16.0),
            flag("bold", "Bold", false),
            num("max_width", "Max width", 40.0, 500.0, 200.0),
            pick("action", "On click", ACTIONS, 0),
            text("run", "Or run a command", ""),
        ],
        "spacer" => vec![num("width", "Width", 0.0, 240.0, 12.0)],
        "dock" => vec![
            num("icon_size", "Icon size", 28.0, 96.0, 64.0),
            num("gap", "Gap", 0.0, 30.0, 14.0),
            flag("magnify", "Magnify on hover", true),
            num("magnify_amount", "Magnification", 0.1, 1.4, 0.62),
            num("magnify_range", "Magnify reach", 50.0, 240.0, 105.0),
            pick("indicator", "Running mark", DOCK_INDICATORS, 0),
            flag("tooltips", "Name tooltips", true),
            // (a window preview over a Dock icon is a Windows habit: the taskbar has it on, the Dock does not)
            flag("previews", "Window previews", false),
            flag("running_only", "Only running apps", false),
            flag("separators", "Divider before extras", true),
            flag("bounce", "Bounce on click", true),
        ],
        "start" => vec![pick("style", "Style", START_STYLES, 0), num("size", "Size", 24.0, 64.0, 40.0)],
        "search-box" => vec![text("text", "Placeholder", "Search"), num("width", "Width", 80.0, 420.0, 190.0)],
        "tasks" => vec![
            num("icon_size", "Icon size", 14.0, 52.0, 26.0),
            num("button_w", "Button width", 24.0, 120.0, 44.0),
            flag("labels", "Show names", false),
            flag("previews", "Window previews", true),
            num("label_w", "Name width", 80.0, 260.0, 150.0),
            pick("indicator", "Running mark", INDICATORS, 0),
            num("gap", "Gap", 0.0, 16.0, 4.0),
        ],
        "tray" => vec![flag("chevron", "More arrow", true), flag("wifi", "Wi-Fi", true), flag("volume", "Volume", true), flag("battery", "Battery", true), flag("bluetooth", "Bluetooth", false), num("size", "Icon size", 12.0, 24.0, 15.0), pick("action", "On click", ACTIONS, 6)],
        "show-desktop" => vec![num("width", "Width", 4.0, 40.0, 14.0)],
        _ => Vec::new(),
    }
}

/// Typed reads with the default taken from the widget's option schema (`options`), so a default lives in one place.
impl WidgetSpec {
    fn schema(&self, key: &str) -> Option<Opt> {
        options(&self.kind).into_iter().find(|o| o.key == key)
    }

    pub(crate) fn n(&self, key: &str) -> f32 {
        self.num(key).unwrap_or_else(|| match self.schema(key).map(|o| o.kind) {
            Some(OptKind::Num(_, _, d)) => d,
            _ => 0.0,
        })
    }

    pub(crate) fn flag(&self, key: &str) -> bool {
        self.bool(key).unwrap_or_else(|| matches!(self.schema(key).map(|o| o.kind), Some(OptKind::Bool(true))))
    }

    /// A text or choice option's value (the default's value when unset).
    pub(crate) fn s(&self, key: &str) -> String {
        if let Some(v) = self.str(key) {
            return v.to_string();
        }
        match self.schema(key).map(|o| o.kind) {
            Some(OptKind::Text(d)) => d.to_string(),
            Some(OptKind::Choice(c, d)) => c[d].0.to_string(),
            _ => String::new(),
        }
    }
}

impl WidgetSpec {
    /// Current value of a numeric option (its default when unset).
    pub fn opt_num(&self, o: &Opt) -> f32 {
        match o.kind {
            OptKind::Num(_, _, d) => self.num(o.key).unwrap_or(d),
            _ => 0.0,
        }
    }

    pub fn opt_bool(&self, o: &Opt) -> bool {
        match o.kind {
            OptKind::Bool(d) => self.bool(o.key).unwrap_or(d),
            _ => false,
        }
    }

    /// Index of the chosen value in a choice option.
    pub fn opt_choice(&self, o: &Opt) -> usize {
        match o.kind {
            OptKind::Choice(c, d) => self.str(o.key).and_then(|v| c.iter().position(|(val, _)| *val == v)).unwrap_or(d),
            _ => 0,
        }
    }

    pub fn opt_str(&self, o: &Opt) -> String {
        match o.kind {
            OptKind::Text(d) => self.str(o.key).unwrap_or(d).to_string(),
            _ => String::new(),
        }
    }

    pub fn set_num(&mut self, key: &str, v: f32) {
        self.opts.insert(key.into(), toml::Value::Float((v as f64 * 100.0).round() / 100.0));
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.opts.insert(key.into(), toml::Value::Boolean(v));
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.opts.insert(key.into(), toml::Value::String(v.into()));
    }
}

// ---------------------------------------------------------------------------------------- the bar

struct Placed {
    widget: usize,
    x: f32,
    w: f32,
}

/// Where a bar ended up this frame.
pub struct Frame {
    pub rect: [f32; 4],
    placed: Vec<Placed>,
}

pub struct Bar {
    pub index: usize,
    widgets: Vec<(usize, Box<dyn Widget>)>,
    /// where each widget came from in the config: (zone, index in that zone's list)
    origin: Vec<(usize, usize)>,
    pub cfg: BarConfig,
    /// how far each widget is still from where the layout puts it (it glides there when the layout changes), and
    /// where the layout put it last time
    shift: Vec<f32>,
    last_x: Vec<f32>,
    /// 1 = shown, 0 = slid off the screen (auto-hide), and how long it has had no reason to be shown
    reveal: Spring,
    away: f32,
    /// the real screen's width as of the last update (the Dock's item positions are asked for without a context)
    screen_w: std::cell::Cell<f32>,
}

impl Bar {
    pub fn new(index: usize, cfg: &BarConfig) -> Self {
        let mut widgets = Vec::new();
        let mut origin = Vec::new();
        for zone in 0..3 {
            let specs = match zone {
                0 => &cfg.left,
                1 => &cfg.center,
                _ => &cfg.right,
            };
            for (i, s) in specs.iter().enumerate() {
                match create(s) {
                    Some(w) => {
                        widgets.push((zone, w));
                        origin.push((zone, i));
                    }
                    None => eprintln!("bar: unknown widget {:?}", s.kind),
                }
            }
        }
        Bar { index, widgets, origin, cfg: cfg.clone(), shift: Vec::new(), last_x: Vec::new(), reveal: Spring::new(1.0, 240.0, 0.9), away: 0.0, screen_w: std::cell::Cell::new(0.0) }
    }

    /// Rebuild when the config changed; widget state (the Dock's springs) survives otherwise.
    pub fn sync(&mut self, cfg: &BarConfig) {
        if &self.cfg != cfg {
            // keep the Dock's springs across looks-only changes
            let same_widgets = self.cfg.left == cfg.left && self.cfg.center == cfg.center && self.cfg.right == cfg.right;
            if same_widgets {
                self.cfg = cfg.clone();
            } else {
                *self = Bar::new(self.index, cfg);
            }
        }
    }

    pub fn look(&self, c: &BarCtx) -> MenubarStyle {
        self.cfg.look.clone().unwrap_or_else(|| c.theme.menubar.clone())
    }

    fn vertical(&self) -> bool {
        self.cfg.edge.vertical()
    }

    /// The edge as the layout sees it: a bar on the left or right is laid out as a strip along the top of a screen that
    /// has been turned on its side (its "width" is the screen's height), and everything it reports is turned back.
    fn vedge(&self) -> Edge {
        if self.vertical() { Edge::Top } else { self.cfg.edge }
    }

    /// A point of the real screen as the layout sees it, and back (`sw` is the real screen's width).
    fn to_virtual(&self, sw: f32, p: [f32; 2]) -> [f32; 2] {
        match self.cfg.edge {
            Edge::Left => [p[1], p[0]],
            Edge::Right => [p[1], sw - p[0]],
            _ => p,
        }
    }

    fn to_screen(&self, sw: f32, p: [f32; 2]) -> [f32; 2] {
        match self.cfg.edge {
            Edge::Left => [p[1], p[0]],
            Edge::Right => [sw - p[1], p[0]],
            _ => p,
        }
    }

    /// A rectangle (x, y, w, h) of the layout's space on the real screen.
    fn rect_to_screen(&self, sw: f32, r: [f32; 4]) -> [f32; 4] {
        match self.cfg.edge {
            Edge::Left => [r[1], r[0], r[3], r[2]],
            Edge::Right => [sw - r[1] - r[3], r[0], r[3], r[2]],
            _ => r,
        }
    }

    fn rect_to_virtual(&self, sw: f32, r: [f32; 4]) -> [f32; 4] {
        match self.cfg.edge {
            Edge::Left => [r[1], r[0], r[3], r[2]],
            Edge::Right => [r[1], sw - r[0] - r[2], r[3], r[2]],
            _ => r,
        }
    }

    /// The context as the layout sees it (the screen on its side for a vertical bar).
    fn vctx<'a>(&self, c: &BarCtx<'a>) -> BarCtx<'a> {
        let mut c2 = c.clone();
        if self.vertical() {
            let sw = c.screen[0];
            c2.screen = [c.screen[1], c.screen[0]];
            c2.mouse = self.to_virtual(sw, c.mouse);
            c2.vertical = true;
            c2.axis = c.axis;
        }
        c2
    }

    /// What a widget drew in the layout's space, on the real screen: shapes keep their sizes turned, a line of text
    /// stays upright and is centred in the bar where its run along the bar was.
    fn out_to_screen(&self, c: &BarCtx, f: &Frame, out: &mut Drawn) {
        if !self.vertical() {
            return;
        }
        let sw = c.screen[1];
        for p in &mut out.panels {
            let (cx, cy, hw, hh) = (p.rect[0], p.rect[1], p.rect[2], p.rect[3]);
            let s = self.to_screen(sw, [cx, cy]);
            p.rect = [s[0], s[1], hh, hw];
        }
        let cross = self.to_screen(sw, [0.0, f.rect[1] + f.rect[3] / 2.0])[0];
        for l in &mut out.labels {
            let w = (c.measure)(&l.text, l.size, l.bold);
            let axis = match l.align {
                Align::Left => l.x + w / 2.0,
                Align::Center => l.x,
                Align::Right => l.x - w / 2.0,
            };
            l.x = cross;
            l.y = axis + l.size * 0.36;
            l.align = Align::Center;
        }
    }

    /// The context with this bar's ink and backdrop tint filled in (in the layout's space).
    fn ctx<'a>(&self, c: &BarCtx<'a>) -> BarCtx<'a> {
        let c = &self.vctx(c);
        let look = self.look(c);
        // an opaque bar has its own light or dark (a solid bar with a colour of its own, like GNOME's black one, by that colour)
        let bar_dark = match (look.kind, look.color) {
            (MenubarKind::Solid, Some(col)) => 0.299 * col[0] + 0.587 * col[1] + 0.114 * col[2] < 0.5,
            _ => c.theme.dark,
        };
        let light = if look.bar && look.opacity > 0.6 && matches!(look.kind, MenubarKind::Glass | MenubarKind::Solid) { bar_dark } else { c.menu_light };
        let mut c2 = c.clone();
        if self.cfg.surface == BarSurface::Glass {
            // a taskbar: fixed ink that follows light / dark, no wallpaper adaptation
            // light ink on dark glass (an Aero taskbar stays dark blue in a light theme), dark ink on light glass
            let g = self.cfg.glass.as_ref().unwrap_or(&c.theme.glass);
            let luma = 0.299 * g.tint[0] + 0.587 * g.tint[1] + 0.114 * g.tint[2];
            let dark_glass = g.tint[3] > 0.35 && luma < 0.52;
            c2.ink = if c.theme.dark || dark_glass { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.17] };
            c2.under = [0.0; 4];
            c2.adaptive = false;
            c2.fixed_ink = true;
            return c2;
        }
        c2.ink = if light { [1.0; 3] } else { [0.10, 0.12, 0.18] };
        c2.under = under_of(c.theme, &look);
        c2.adaptive = look.kind == MenubarKind::Glass;
        c2
    }

    /// The layout as it is now, with every widget still on its way to its place shown part-way.
    fn frame(&self, c: &BarCtx) -> Frame {
        let mut f = self.frame_raw(c);
        for p in &mut f.placed {
            p.x += self.shift.get(p.widget).copied().unwrap_or(0.0);
        }
        // an auto-hiding bar slides out past its screen edge
        if self.cfg.autohide != AutoHide::Off && self.reveal.x < 0.999 {
            let up = self.widgets.iter().map(|(_, w)| w.overflow(c)).fold(0.0, f32::max);
            let dist = f.rect[3] + self.cfg.margin.max(self.cfg.inset) + up + 8.0;
            let dy = (1.0 - self.reveal.x) * dist;
            f.rect[1] += if self.vedge() == Edge::Top { -dy } else { dy };
        }
        f
    }

    fn frame_raw(&self, c: &BarCtx) -> Frame {
        let widths: Vec<f32> = self.widgets.iter().map(|(_, w)| w.width(c)).collect();
        let (sw, sh) = (c.screen[0], c.screen[1]);
        let top = self.vedge() == Edge::Top;
        let mut placed = Vec::new();
        let rect;
        match self.cfg.shape {
            BarShape::Strip => {
                let h = self.cfg.height.unwrap_or(if self.vertical() { 64.0 } else { c.theme.menubar.height });
                let m = self.cfg.inset;
                // a side bar stops short of the bars along the top and bottom
                let (a0, a1) = if self.vertical() { (c.axis[0], c.axis[1]) } else { (0.0, 0.0) };
                rect = [m + a0, if top { m } else { sh - h - m }, sw - 2.0 * m - a0 - a1, h];
                let zone_w = |z: usize| -> f32 { self.widgets.iter().zip(&widths).filter(|((zz, _), _)| *zz == z).map(|(_, w)| *w).sum() };
                let (pl, pr) = if self.cfg.surface == BarSurface::Glass { (0.0, 4.0) } else { (10.0, 14.0) };
                let mut cursor = [rect[0] + pl, rect[0] + (rect[2] - zone_w(1)) / 2.0, rect[0] + rect[2] - pr - zone_w(2)];
                for (i, ((z, _), w)) in self.widgets.iter().zip(&widths).enumerate() {
                    placed.push(Placed { widget: i, x: cursor[*z], w: *w });
                    cursor[*z] += w;
                }
            }
            BarShape::Floating => {
                let total: f32 = widths.iter().sum();
                let h = self.widgets.iter().map(|(_, w)| w.height(c)).fold(0.0, f32::max).max(28.0);
                let m = self.cfg.margin;
                let (a0, a1) = if self.vertical() { (c.axis[0], c.axis[1]) } else { (0.0, 0.0) };
                rect = [a0 + (sw - a0 - a1 - total) / 2.0, if top { m } else { sh - m - h }, total, h];
                let mut x = rect[0];
                for (i, w) in widths.iter().enumerate() {
                    placed.push(Placed { widget: i, x, w: *w });
                    x += w;
                }
            }
        }
        Frame { rect, placed }
    }

    /// The bar's rectangle this frame.
    pub fn rect(&self, c: &BarCtx) -> [f32; 4] {
        let v = self.vctx(c);
        self.rect_to_screen(c.screen[0], self.frame(&v).rect)
    }

    /// Screen area this bar takes away from windows: (left, top, right, bottom).
    pub fn reserved(&self, c: &BarCtx) -> [f32; 4] {
        if self.cfg.autohide != AutoHide::Off {
            return [0.0; 4];
        }
        let r = self.rect(c);
        match self.cfg.edge {
            Edge::Top => [0.0, r[1] + r[3], 0.0, 0.0],
            Edge::Bottom => [0.0, 0.0, 0.0, c.screen[1] - r[1]],
            Edge::Left => [r[0] + r[2], 0.0, 0.0, 0.0],
            Edge::Right => [0.0, 0.0, c.screen[0] - r[0], 0.0],
        }
    }

    pub fn widget_rects(&self, c: &BarCtx) -> Vec<(usize, [f32; 4], &'static str)> {
        let v = self.vctx(c);
        let f = self.frame(&v);
        f.placed.iter().map(|p| (p.widget, self.rect_to_screen(c.screen[0], [p.x, f.rect[1], p.w, f.rect[3]]), self.widgets[p.widget].1.kind())).collect()
    }

    /// (zone, index within the zone's list) of a widget, as in the config.
    pub fn origin(&self, widget: usize) -> (usize, usize) {
        self.origin[widget]
    }

    /// Is `p` over the bar's own surface (the part that must not pass clicks through)?
    pub fn contains(&self, c: &BarCtx, p: [f32; 2]) -> bool {
        let p = self.to_virtual(c.screen[0], p);
        let c = &self.vctx(c);
        let f = self.frame(c);
        let up = self.widgets.iter().map(|(_, w)| w.overflow(c)).fold(0.0, f32::max);
        let r = f.rect;
        match self.cfg.shape {
            BarShape::Strip => p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3],
            BarShape::Floating => p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] - up && p[1] <= r[1] + r[3],
        }
    }

    /// Does the bar hold a widget of this kind?
    pub fn has(&self, kind: &str) -> bool {
        self.widgets.iter().any(|(_, w)| w.kind() == kind)
    }

    /// Does the bar swallow clicks at `p` (a glass strip or a floating bar), even between its widgets?
    pub fn blocks(&self, c: &BarCtx, p: [f32; 2]) -> bool {
        (self.cfg.shape == BarShape::Floating || self.cfg.surface == BarSurface::Glass) && self.contains(c, p)
    }

    /// Rectangle (x, y, w, h) of the widget the Quick Settings hangs from, and its role (pill or anchor).
    pub fn cc_rect(&self, c: &BarCtx) -> Option<([f32; 4], CcRole)> {
        let sw = c.screen[0];
        let c = &self.vctx(c);
        let f = self.frame(c);
        let pick = |role: CcRole| f.placed.iter().find(|p| self.widgets[p.widget].1.cc() == role).map(|p| (self.rect_to_screen(sw, [p.x, f.rect[1], p.w, f.rect[3]]), role));
        pick(CcRole::Pill).or_else(|| pick(CcRole::Anchor))
    }

    /// Which tray icon part `part` of widget `widget` is, when the widget is a tray (past its "more" arrow, if shown).
    pub fn tray_part(&self, widget: usize, part: usize) -> Option<usize> {
        let spec = self.spec_of(widget)?;
        (spec.kind == "tray").then(|| part.checked_sub(spec.flag("chevron") as usize)).flatten()
    }

    /// The configuration of widget `widget` (its options: what the other buttons do, its tooltip).
    pub fn spec_of(&self, widget: usize) -> Option<&WidgetSpec> {
        let (zone, i) = *self.origin.get(widget)?;
        match zone {
            0 => self.cfg.left.get(i),
            1 => self.cfg.center.get(i),
            _ => self.cfg.right.get(i),
        }
    }

    pub fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        let p = self.widgets.iter().find_map(|(_, w)| w.item_center(key))?;
        Some(self.to_screen(self.screen_w.get(), p))
    }

    /// The Dock / taskbar item under the pointer (its key and screen rectangle), if a widget of this bar has one there.
    pub fn item_at(&self, c: &BarCtx) -> Option<DockHover> {
        let sw = c.screen[0];
        let c = self.ctx(c);
        let f = self.frame(&c);
        let mut h = f.placed.iter().find_map(|p| self.widgets[p.widget].1.item_at(&c, [p.x, f.rect[1], p.w, f.rect[3]]))?;
        h.rect = self.rect_to_screen(sw, h.rect);
        Some(h)
    }

    pub fn set_dock(&mut self, items: &[DockItem], focused: Option<&str>) {
        for (_, w) in &mut self.widgets {
            w.set_dock(items, focused);
        }
    }

    pub fn update(&mut self, dt: f32, c: &BarCtx) {
        self.screen_w.set(c.screen[0]);
        let c = self.ctx(c);
        self.update_reveal(dt, &c);
        // when something changes width (the app's name in the menu bar), what follows it glides to its new place
        let raw = self.frame_raw(&c);
        let n = self.widgets.len();
        if self.shift.len() != n {
            self.shift = vec![0.0; n];
            self.last_x = vec![f32::NAN; n];
        }
        for p in &raw.placed {
            let last = self.last_x[p.widget];
            if !last.is_nan() && (last - p.x).abs() > 0.5 {
                self.shift[p.widget] += last - p.x;
            }
            self.last_x[p.widget] = p.x;
        }
        let k = (-13.0 * dt).exp();
        for s in &mut self.shift {
            *s *= k;
            if s.abs() < 0.05 {
                *s = 0.0;
            }
        }
        let f = self.frame(&c);
        for p in &f.placed {
            let r = [p.x, f.rect[1], p.w, f.rect[3]];
            self.widgets[p.widget].1.update(dt, &c, r);
        }
    }

    /// Decide whether an auto-hiding bar is shown, and glide it there.
    fn update_reveal(&mut self, dt: f32, c: &BarCtx) {
        if self.cfg.autohide == AutoHide::Off {
            self.reveal.x = 1.0;
            self.reveal.v = 0.0;
            self.reveal.target = 1.0;
            return;
        }
        // the bar's place when it is shown
        let full = self.frame_raw(c).rect;
        let up = self.widgets.iter().map(|(_, w)| w.overflow(c)).fold(0.0, f32::max);
        let (m, h) = (c.mouse, c.screen[1]);
        let at_edge = if self.vedge() == Edge::Top { m[1] <= 1.5 } else { m[1] >= h - 1.5 } && m[0] >= 0.0 && m[0] <= c.screen[0];
        let on_bar = self.reveal.x > 0.3 && m[0] >= full[0] - 4.0 && m[0] <= full[0] + full[2] + 4.0 && m[1] >= full[1] - up - 6.0 && m[1] <= full[1] + full[3] + 6.0;
        let menu = c.open.is_some_and(|o| o.bar == self.index);
        let covered = c.windows.iter().any(|r| r[0] < full[0] + full[2] && r[0] + r[2] > full[0] && r[1] < full[1] + full[3] && r[1] + r[3] > full[1]);
        let show = at_edge || on_bar || menu || (self.cfg.autohide == AutoHide::Smart && !covered);
        if show {
            self.away = 0.0;
            self.reveal.target = 1.0;
        } else {
            self.away += dt;
            if self.away > 0.35 {
                self.reveal.target = 0.0;
            }
        }
        c.theme.animations.step("dock", &mut self.reveal, dt);
    }

    pub fn hit(&self, c: &BarCtx, p: [f32; 2]) -> Option<BarHit> {
        let p = self.to_virtual(c.screen[0], p);
        let c = self.ctx(c);
        let f = self.frame(&c);
        let up = self.widgets.iter().map(|(_, w)| w.overflow(&c)).fold(0.0, f32::max);
        let (y0, y1) = match self.cfg.shape {
            BarShape::Strip => (f.rect[1], f.rect[1] + f.rect[3]),
            BarShape::Floating => (f.rect[1] - up, f.rect[1] + f.rect[3]),
        };
        if p[1] < y0 || p[1] > y1 {
            return None;
        }
        for pl in &f.placed {
            let mut x = pl.x;
            for (part, w) in self.widgets[pl.widget].1.parts(&c).into_iter().enumerate() {
                if p[0] >= x && p[0] < x + w {
                    return Some(BarHit { bar: self.index, widget: pl.widget, part });
                }
                x += w;
            }
        }
        None
    }

    pub fn click(&mut self, c: &BarCtx, hit: BarHit) -> Option<BarClick> {
        let c = self.ctx(c);
        let f = self.frame(&c);
        let pl = f.placed.iter().find(|p| p.widget == hit.widget)?;
        let r = [pl.x, f.rect[1], pl.w, f.rect[3]];
        let x = pl.x + self.widgets[hit.widget].1.parts(&c).iter().take(hit.part).sum::<f32>();
        self.widgets.get_mut(hit.widget)?.1.click(&c, r, hit.part, x)
    }

    /// Backdrop and every widget.
    pub fn draw(&self, c: &BarCtx) -> (Vec<Panel>, Vec<Label>) {
        let c = self.ctx(c);
        let f = self.frame(&c);
        let mut out = Drawn::default();
        match self.cfg.shape {
            BarShape::Strip if self.cfg.surface == BarSurface::Glass => {
                let g = self.cfg.glass.as_ref().unwrap_or(&c.theme.glass);
                let (cx, cy) = (f.rect[0] + f.rect[2] / 2.0, f.rect[1] + f.rect[3] / 2.0);
                out.panels.push(g.panel(cx, cy, f.rect[2], f.rect[3], self.cfg.radius));
            }
            BarShape::Strip => strip_background(&c, &self.look(&c), self.vedge(), f.rect, &mut out.panels),
            BarShape::Floating => {
                if f.rect[2] > 1.0 {
                    let pulse = self.widgets.iter().map(|(_, w)| w.pulse()).fold(0.0f32, |a, b| if b.abs() > a.abs() { b } else { a });
                    let (cx, cy) = (f.rect[0] + f.rect[2] / 2.0, f.rect[1] + f.rect[3] / 2.0);
                    out.panels.push(c.theme.glass.panel(cx, cy, f.rect[2] * (1.0 + 0.012 * pulse), f.rect[3] * (1.0 - 0.02 * pulse), c.theme.dock.radius).adaptive());
                }
            }
        }
        for pl in &f.placed {
            let lit = [c.open, c.hover].into_iter().flatten().find(|h| h.bar == self.index && h.widget == pl.widget).map(|h| h.part);
            let r = [pl.x, f.rect[1], pl.w, f.rect[3]];
            self.widgets[pl.widget].1.draw(&c, r, lit, &mut out);
        }
        self.out_to_screen(&c, &f, &mut out);
        (out.panels, out.labels)
    }
}

/// Tint of a strip's background, for ink that adapts to it (rgb + strength).
pub fn under_of(theme: &Theme, look: &MenubarStyle) -> [f32; 4] {
    if !look.bar || look.kind == MenubarKind::Clear {
        return [0.0; 4];
    }
    let c = match (look.kind, theme.dark) {
        (MenubarKind::Solid, _) if look.color.is_some() => look.color.unwrap_or_default(),
        (MenubarKind::Glass | MenubarKind::Fade, true) => [0.06, 0.08, 0.13],
        (MenubarKind::Glass | MenubarKind::Fade, false) => [0.92, 0.95, 1.0],
        (_, true) => [0.10, 0.12, 0.18],
        (_, false) => [0.96, 0.97, 1.0],
    };
    let a = if look.kind == MenubarKind::Fade { look.opacity * 0.55 } else { look.opacity };
    [c[0], c[1], c[2], a.clamp(0.0, 1.0)]
}

/// A strip's backdrop: a flat glass or solid band, a blur that fades out away from the edge, or nothing.
fn strip_background(c: &BarCtx, look: &MenubarStyle, edge: Edge, r: [f32; 4], p: &mut Vec<Panel>) {
    if !look.bar || look.kind == MenubarKind::Clear {
        return;
    }
    let (w, h) = (r[2], r[3]);
    let dark = c.theme.dark;
    let cy = r[1] + h / 2.0;
    match look.kind {
        MenubarKind::Glass => {
            let mut g = c.theme.glass.panel(w / 2.0, cy, w, h, 0.0);
            // a flat strip: no edge refraction, no outer shadow
            g.params[3] = 0.0;
            g.fx = [look.blur, 0.0, 0.0, 0.0];
            g.extra = [0.0; 4];
            g.color2 = [0.0; 4];
            let t = if dark { [0.079, 0.079, 0.079] } else { [0.92, 0.95, 1.0] };
            g.tint = [t[0], t[1], t[2], look.opacity.clamp(0.0, 1.0)];
            p.push(g.adaptive());
        }
        MenubarKind::Solid => {
            let t = look.color.unwrap_or(if dark { [0.12, 0.12, 0.12] } else { [0.96, 0.97, 1.0] });
            p.push(Panel::solid(w / 2.0, cy, w, h, 0.0, t, t).flat().alpha(look.opacity.clamp(0.0, 1.0)));
        }
        MenubarKind::Fade => {
            // blur and tint are strongest at the screen edge and fade out over ~2.4 bar heights
            let fh = h * 2.4;
            let top = edge == Edge::Top;
            let y0 = if top { r[1] } else { r[1] + h - fh };
            let mut g = Panel::glass(w / 2.0, y0 + fh / 2.0, w, fh, 0.0);
            g.params[1] = 8.0;
            g.params[2] = 0.0;
            g.params[3] = if top { 1.0 } else { -1.0 };
            g.fx = [look.blur, 0.0, 0.0, 0.0];
            g.extra = [0.0; 4];
            let t = if dark { [0.079, 0.079, 0.079] } else { [0.92, 0.95, 1.0] };
            g.tint = [t[0], t[1], t[2], look.opacity.clamp(0.0, 1.0)];
            p.push(g);
        }
        MenubarKind::Clear => {}
    }
    if look.border && look.kind != MenubarKind::Fade {
        let t = if dark { [1.0; 3] } else { [0.0; 3] };
        let y = if edge == Edge::Top { r[1] + h - 0.5 } else { r[1] + 0.5 };
        p.push(Panel::solid(w / 2.0, y, w, 1.0, 0.0, t, t).flat().alpha(0.10));
    }
}

// ---------------------------------------------------------------------------------------- time

/// Format the local time with a small strftime subset.
pub fn format_time(fmt: &str) -> String {
    // SAFETY: time/localtime_r are given valid pointers to initialised values
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    let uk = crate::i18n::language() == "uk";
    let days = if uk { ["Нд", "Пн", "Вт", "Ср", "Чт", "Пт", "Сб"] } else { ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] };
    let days_long = if uk { ["неділя", "понеділок", "вівторок", "середа", "четвер", "пʼятниця", "субота"] } else { ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"] };
    let months = if uk { ["січ", "лют", "бер", "кві", "тра", "чер", "лип", "сер", "вер", "жов", "лис", "гру"] } else { ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] };
    // with a day number the long month is genitive in Ukrainian ("16 жовтня")
    let months_long = if uk { ["січня", "лютого", "березня", "квітня", "травня", "червня", "липня", "серпня", "вересня", "жовтня", "листопада", "грудня"] } else { ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"] };
    let h12 = if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 };
    let (wd, mo) = (tm.tm_wday as usize % 7, tm.tm_mon as usize % 12);
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(ch) = it.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match it.next() {
            Some('a') => out.push_str(days[wd]),
            Some('A') => out.push_str(days_long[wd]),
            Some('b') => out.push_str(months[mo]),
            Some('B') => out.push_str(months_long[mo]),
            Some('d') => out.push_str(&format!("{:02}", tm.tm_mday)),
            Some('e') => out.push_str(&tm.tm_mday.to_string()),
            Some('H') => out.push_str(&format!("{:02}", tm.tm_hour)),
            Some('I') => out.push_str(&format!("{:02}", h12)),
            Some('l') => out.push_str(&h12.to_string()),
            Some('M') => out.push_str(&format!("{:02}", tm.tm_min)),
            Some('S') => out.push_str(&format!("{:02}", tm.tm_sec)),
            Some('p') => out.push_str(if tm.tm_hour < 12 { "AM" } else { "PM" }),
            Some('P') => out.push_str(if tm.tm_hour < 12 { "am" } else { "pm" }),
            Some('Y') => out.push_str(&(tm.tm_year + 1900).to_string()),
            Some('y') => out.push_str(&format!("{:02}", (tm.tm_year + 1900) % 100)),
            Some('m') => out.push_str(&format!("{:02}", tm.tm_mon + 1)),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_widget_can_be_created() {
        for (kind, _) in CATALOG {
            assert!(create(&WidgetSpec::new(kind)).is_some(), "catalog widget {kind:?} does not build");
        }
    }

    #[test]
    fn option_schema_is_consistent() {
        for (kind, _) in CATALOG {
            let spec = WidgetSpec::new(kind);
            for o in options(kind) {
                match o.kind {
                    OptKind::Num(lo, hi, d) => {
                        assert!(lo < hi && (lo..=hi).contains(&d), "{kind}.{}: default {d} outside {lo}..{hi}", o.key);
                        assert_eq!(spec.n(o.key), d, "{kind}.{} default is not read back", o.key);
                    }
                    OptKind::Bool(d) => assert_eq!(spec.flag(o.key), d, "{kind}.{}", o.key),
                    OptKind::Choice(c, d) => {
                        assert!(d < c.len(), "{kind}.{}: default index out of range", o.key);
                        assert_eq!(spec.s(o.key), c[d].0, "{kind}.{}", o.key);
                    }
                    OptKind::Text(d) => assert_eq!(spec.s(o.key), d, "{kind}.{}", o.key),
                }
            }
        }
    }

    #[test]
    fn options_survive_a_toml_round_trip() {
        let mut spec = WidgetSpec::new("tasks");
        spec.set_num("icon_size", 31.0);
        spec.set_bool("labels", true);
        spec.set_str("indicator", "dot");
        let text = toml::to_string(&toml::Table::from_iter([("w".to_string(), toml::Value::try_from(&spec).unwrap())])).unwrap();
        let back: toml::Table = toml::from_str(&text).unwrap();
        let spec2: WidgetSpec = back["w"].clone().try_into().unwrap();
        assert_eq!(spec, spec2);
        assert_eq!(spec2.n("icon_size"), 31.0);
        assert!(spec2.flag("labels"));
        assert_eq!(spec2.s("indicator"), "dot");
    }

    #[test]
    fn every_preset_panel_builds_with_known_widgets() {
        for (id, _) in crate::theme::PRESETS {
            let theme = crate::theme::preset(id).expect("preset parses");
            assert!(!theme.bars.is_empty(), "{id} has no panels");
            for (i, cfg) in theme.bars.iter().enumerate() {
                let bar = Bar::new(i, cfg);
                let wanted = cfg.left.len() + cfg.center.len() + cfg.right.len();
                assert_eq!(bar.widgets.len(), wanted, "{id}/{}: some widget names are unknown", cfg.name);
            }
        }
    }
}
