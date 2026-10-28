//! Themes: the whole look and layout of the shell as data (TOML). Presets are embedded; the user's
//! `~/.config/memrium/config.toml` picks one and may override any field.

use chebrium::spring::Spring;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// menu bar on top + floating Dock + Quick Settings
    #[default]
    Macos,
    /// a taskbar with start button, apps, tray and clock
    Taskbar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecoKind {
    /// traffic lights on the left, big rounded glass-era corners
    #[default]
    Mac,
    /// flat titlebar, buttons on the right
    Win11,
    /// thick glass frame, red close button
    Aero,
    /// GNOME / Adwaita: a flat header bar with the title in the middle and round buttons on the right
    Gnome,
    /// no titlebar: only shape and shadow
    Minimal,
    /// built from the parts in `WindowStyle::frame`
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TitleAlign {
    #[default]
    Center,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl Edge {
    /// A bar on the left or right side of the screen (its widgets run top to bottom).
    pub fn vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    #[default]
    // configs and themes written before the rename say "spotlight"
    #[serde(alias = "spotlight")]
    Figlet,
    StartMenuWin11,
    StartMenuWin7,
    /// a Plasma "Kickoff" style launcher: the user and a search field on top, favourites as a list or grid
    Plasma,
    /// a full-screen, paged grid of every app with a search field on top
    Launchpad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconShape {
    None,
    #[default]
    Squircle,
    Circle,
    Rounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Plate {
    #[default]
    None,
    Glass,
    Solid,
    /// every icon on its own opaque tile, as on macOS (see `renderer::TileStyle`)
    Tile,
}

/// How the tile of a `Plate::Tile` icon is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TileMode {
    /// light for coloured icons, dark for icons that are mostly white
    #[default]
    Auto,
    Light,
    Dark,
    /// the icon's own main colour
    Vivid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WallKind {
    #[default]
    Image,
    Live,
    Video,
}

/// How the glass fields are driven: flat panels, one "glassiness" amount, or every field by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlassMode {
    Flat,
    Glass,
    /// soft, opaque, puffy pastel surfaces
    Clay,
    /// sheets of paper with torn edges, fibres and creases
    Paper,
    /// varnished wooden boards with grain, rings and knots
    Wood,
    #[default]
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassStyle {
    /// the fields below are always the values in use; `Flat` and `Glass` compute them
    pub mode: GlassMode,
    /// glassiness in `Glass` mode: 0 = flat, 1 = the reference glass
    pub amount: f32,
    /// hand-tuned values kept while another mode is active
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<Box<GlassStyle>>,
    pub blur: f32,
    pub refraction: f32,
    pub bezel: f32,
    pub dispersion: f32,
    pub rim: f32,
    /// how thin and crisp the edge light is: 0 = soft glow, 1 = a sharp hairline
    pub rim_sharp: f32,
    /// diagonal sheen strength (Aero)
    pub gloss: f32,
    pub saturation: f32,
    /// darken glass over bright backdrops so white content stays readable (0 = off)
    pub adapt: f32,
    /// rgb tint + strength
    pub tint: [f32; 4],
    /// rgb + opacity of a 1px inner border
    pub border: [f32; 4],
    /// opacity, blur px, y offset
    pub shadow: [f32; 3],
}

impl GlassStyle {
    /// Smooth Jelly Glass lettering: hardly any bending, a soft frosted fill that glows from the edge.
    pub fn lettering_liquid(&self) -> GlassStyle {
        GlassStyle { bezel: 22.0, refraction: 7.0, blur: 16.0, dispersion: 0.0, rim: 0.9, rim_sharp: 0.55, gloss: 0.0, saturation: 1.05, adapt: 0.0, border: [1.0, 1.0, 1.0, 0.0], shadow: [0.0, 0.0, 0.0], ..self.clone() }
    }

    /// The glass big lettering is made of: a clear slab with a bright bevel, bending what is behind it.
    pub fn lettering(&self) -> GlassStyle {
        GlassStyle { bezel: 22.0, refraction: self.refraction.max(30.0) * 1.1, blur: 8.0, dispersion: 0.15, rim: 1.0, rim_sharp: 0.30, gloss: 0.0, saturation: 1.0, adapt: 0.0, border: [0.08, 0.12, 0.2, 0.18], shadow: [0.0, 0.0, 0.0], ..self.clone() }
    }
}

impl Default for GlassStyle {
    fn default() -> Self {
        GlassStyle { mode: GlassMode::Custom, amount: 1.0, custom: None, rim_sharp: 0.7842, blur: 5.9902, refraction: 30.5948, bezel: 90.0, dispersion: 0.0469, rim: 0.5293, gloss: 0.0694, saturation: 1.2873, adapt: 0.1203, tint: [0.7336, 0.84, 0.92, 0.1109], border: [0.0, 0.0, 0.0, 0.0325], shadow: [0.1836, 22.0, 8.0] }
    }
}

impl GlassStyle {
    /// Frosted but flat: no refraction, edge light or gloss.
    fn flat() -> GlassStyle {
        GlassStyle { blur: 18.0, refraction: 0.0, bezel: 8.0, dispersion: 0.0, rim: 0.0, gloss: 0.0, saturation: 1.0, adapt: 0.12, tint: [0.80, 0.84, 0.92, 0.45], border: [1.0, 1.0, 1.0, 0.10], shadow: [0.14, 18.0, 6.0], ..GlassStyle::default() }
    }

    /// Claymorphism: opaque pastel, a wide soft inner highlight, top-lit gradient and a deep soft shadow.
    /// (`bezel` is the width of the puffy rim, `dispersion` the surface texture, `rim` the highlight, `gloss` the glaze)
    fn clay() -> GlassStyle {
        GlassStyle { blur: 0.0, refraction: 0.0, bezel: 26.0, dispersion: 0.6, rim: 0.8, gloss: 0.5, saturation: 1.0, adapt: 0.0, tint: [0.96, 0.84, 0.80, 1.0], border: [1.0, 1.0, 1.0, 0.0], shadow: [0.26, 30.0, 14.0], ..GlassStyle::default() }
    }

    /// Paper: the fields mean something else here, since the shader reads them for the sheet: `blur` is how far the
    /// torn edge wanders (px), `dispersion` the strength of grain and fibres, `rim` how crumpled the sheet is.
    pub fn paper(dark: bool) -> GlassStyle {
        GlassStyle { blur: 6.0, refraction: 0.0, bezel: 0.0, dispersion: 0.35, rim: 0.12, rim_sharp: 0.0, gloss: 0.0, saturation: 1.0, adapt: 0.0, tint: Self::paper_tint(dark), border: [0.0; 4], shadow: [0.30, 14.0, 6.0], ..GlassStyle::default() }
    }

    /// Wood: `blur` is the ring density, `dispersion` the strength of the grain, `rim` the bevel's light and `gloss`
    /// the varnish.
    pub fn wood(dark: bool) -> GlassStyle {
        GlassStyle { blur: 1.0, refraction: 0.0, bezel: 0.0, dispersion: 0.6, rim: 0.7, rim_sharp: 0.0, gloss: 0.3, saturation: 1.0, adapt: 0.0, tint: Self::wood_tint(dark), border: [0.0; 4], shadow: [0.34, 18.0, 8.0], ..GlassStyle::default() }
    }

    /// Light oak by day, dark walnut by night.
    pub fn wood_tint(dark: bool) -> [f32; 4] {
        if dark { [0.36, 0.24, 0.15, 1.0] } else { [0.80, 0.62, 0.42, 1.0] }
    }

    /// Cream paper by day, charcoal paper by night.
    pub fn paper_tint(dark: bool) -> [f32; 4] {
        if dark { [0.21, 0.21, 0.21, 1.0] } else { [0.96, 0.94, 0.89, 1.0] }
    }

    /// Switch mode, keeping hand-tuned values for when `Custom` comes back.
    pub fn set_mode(&mut self, mode: GlassMode) {
        if mode == self.mode {
            return;
        }
        if self.mode == GlassMode::Custom {
            let mut keep = self.clone();
            keep.custom = None;
            self.custom = Some(Box::new(keep));
        }
        self.mode = mode;
        match mode {
            GlassMode::Custom => {
                if let Some(c) = self.custom.take() {
                    let (amount, mode) = (self.amount, self.mode);
                    *self = GlassStyle { amount, mode, custom: None, ..*c };
                }
            }
            _ => self.apply_mode(),
        }
    }

    /// Recompute the fields from `mode` / `amount` (no-op in `Custom`).
    pub fn apply_mode(&mut self) {
        let t = match self.mode {
            GlassMode::Custom => return,
            GlassMode::Clay => {
                *self = GlassStyle { mode: GlassMode::Clay, amount: self.amount, custom: self.custom.take(), ..GlassStyle::clay() };
                return;
            }
            GlassMode::Paper => {
                *self = GlassStyle { mode: GlassMode::Paper, amount: self.amount, custom: self.custom.take(), ..GlassStyle::paper(false) };
                return;
            }
            GlassMode::Wood => {
                *self = GlassStyle { mode: GlassMode::Wood, amount: self.amount, custom: self.custom.take(), ..GlassStyle::wood(false) };
                return;
            }
            GlassMode::Flat => 0.0,
            GlassMode::Glass => self.amount.clamp(0.0, 1.0),
        };
        let (a, b) = (GlassStyle::flat(), GlassStyle::default());
        // exact at both ends: amount 1 must give the reference glass, not something 1e-8 away from it
        let l = |x: f32, y: f32| if t >= 1.0 { y } else { x + (y - x) * t };
        self.blur = l(a.blur, b.blur);
        self.refraction = l(a.refraction, b.refraction);
        self.bezel = l(a.bezel, b.bezel);
        self.dispersion = l(a.dispersion, b.dispersion);
        self.rim = l(a.rim, b.rim);
        self.rim_sharp = l(a.rim_sharp, b.rim_sharp);
        self.gloss = l(a.gloss, b.gloss);
        self.saturation = l(a.saturation, b.saturation);
        self.adapt = l(a.adapt, b.adapt);
        self.tint = std::array::from_fn(|i| l(a.tint[i], b.tint[i]));
        self.border = std::array::from_fn(|i| l(a.border[i], b.border[i]));
        self.shadow = std::array::from_fn(|i| l(a.shadow[i], b.shadow[i]));
    }

    /// A glass panel in this style (`cx, cy` centre, `w, h` size, `r` corner radius, logical px).
    pub fn panel(&self, cx: f32, cy: f32, w: f32, h: f32, r: f32) -> crate::renderer::Panel {
        use crate::renderer::Panel;
        let mut p = Panel::glass(cx, cy, w, h, r);
        p.params[2] = self.bezel.min(w.min(h) / 2.0);
        p.params[3] = self.refraction;
        p.tint = self.tint;
        p.fx = [self.blur, self.dispersion, self.rim, self.gloss];
        // the shader reads the edge sharpness from the fractional part of the kind (glass = 0)
        p.params[1] = crate::renderer::glass_code(self.rim_sharp, 0.0);
        // paper sheets are drawn by their own part of the shader (kind 12)
        if self.mode == GlassMode::Paper {
            p.params[1] = 12.0;
        }
        // boards are shaded by the wood part of the shader (kind 13); the shape stays a rounded rectangle
        if self.mode == GlassMode::Wood {
            p.params[1] = 13.0;
            p.fx = [self.blur, self.dispersion, self.rim, self.gloss];
        }
        if self.mode == GlassMode::Clay {
            p.params[1] = 14.0;
            p.fx = [0.0, self.dispersion, self.rim, self.gloss];
        }
        p.color2 = self.border;
        p.extra = [self.shadow[0], self.shadow[1], self.shadow[2], self.saturation];
        p
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowStyle {
    pub decoration: DecoKind,
    pub radius: f32,
    pub titlebar: f32,
    pub title_align: TitleAlign,
    /// focused / unfocused shadow: opacity, blur, y offset
    pub shadow: [f32; 3],
    pub shadow_idle: [f32; 3],
    /// how opaque the content area of shell windows (Files, Settings) is, 0 .. 1
    pub client_opacity: f32,
    /// the parts of a window frame, used when `decoration` is `Custom`
    pub frame: FrameStyle,
    /// space left around a maximized window, px (0 = it fills the area)
    pub max_gap: f32,
    /// the frame of a program's window follows how light or dark the program's own top edge is (a dark editor under a
    /// light title bar looks pasted on)
    pub adapt: bool,
    /// a maximized window loses its title bar, and its close / minimize / zoom buttons go to the menu bar (`window-buttons`)
    pub hide_title_max: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonSide {
    #[default]
    Left,
    Right,
}

/// Which button comes first, counted from the window's edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonOrder {
    /// close, minimize, maximize, counted from the window's edge (macOS)
    #[default]
    CloseMinMax,
    /// minimize, maximize, close (Windows, seen from the left)
    MinMaxClose,
    /// close, maximize, minimize (GNOME, seen from the right)
    CloseMaxMin,
    /// minimize, close, maximize
    MinCloseMax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonShape {
    #[default]
    Circle,
    Square,
    Rounded,
    /// no plate until the pointer is over it
    Flat,
    /// as tall as the bar and hardly wider than tall (Windows)
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonColors {
    /// red, yellow and green
    #[default]
    Traffic,
    Accent,
    /// plates in the ink colour
    Mono,
    /// grey plates, a red close button
    CloseRed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlyphShow {
    /// all of them while the pointer is over any button (macOS)
    #[default]
    Hover,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BarMaterial {
    /// as the window body
    #[default]
    Body,
    /// a little lighter or darker than the body, with a line under it
    Tone,
    /// the accent colour
    Accent,
    /// the glass of the theme (the whole frame is glass)
    Glass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BodyTone {
    /// light or dark with the theme
    #[default]
    Auto,
    Light,
    Dark,
}

/// The parts a window frame is built from (Settings → Windows, "Custom").
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameStyle {
    pub side: ButtonSide,
    pub order: ButtonOrder,
    pub shape: ButtonShape,
    pub colors: ButtonColors,
    pub glyphs: GlyphShow,
    pub bar: BarMaterial,
    pub body: BodyTone,
    /// button size (for `Bar`: its width), px
    pub button: f32,
    pub gap: f32,
    /// frame thickness around the content, px
    pub border: f32,
    /// a hairline around the window
    pub outline: bool,
    pub title_size: f32,
    pub title_bold: bool,
}

impl Default for FrameStyle {
    fn default() -> Self {
        FrameStyle::from_kind(DecoKind::Mac)
    }
}

impl FrameStyle {
    /// The parts that make a frame look like one of the ready-made ones: where "Custom" starts from.
    pub fn from_kind(kind: DecoKind) -> FrameStyle {
        let base = FrameStyle { side: ButtonSide::Left, order: ButtonOrder::CloseMinMax, shape: ButtonShape::Circle, colors: ButtonColors::Traffic, glyphs: GlyphShow::Hover, bar: BarMaterial::Body, body: BodyTone::Auto, button: 13.0, gap: 8.0, border: 0.0, outline: false, title_size: 13.0, title_bold: true };
        match kind {
            DecoKind::Win11 => FrameStyle { side: ButtonSide::Right, order: ButtonOrder::CloseMaxMin, shape: ButtonShape::Bar, colors: ButtonColors::CloseRed, glyphs: GlyphShow::Always, bar: BarMaterial::Body, button: 46.0, gap: 0.0, outline: true, title_size: 12.0, title_bold: false, ..base },
            DecoKind::Gnome => FrameStyle { side: ButtonSide::Right, order: ButtonOrder::CloseMaxMin, shape: ButtonShape::Circle, colors: ButtonColors::Mono, glyphs: GlyphShow::Always, bar: BarMaterial::Tone, button: 24.0, gap: 8.0, ..base },
            DecoKind::Aero => FrameStyle { side: ButtonSide::Right, order: ButtonOrder::CloseMaxMin, shape: ButtonShape::Rounded, colors: ButtonColors::CloseRed, glyphs: GlyphShow::Always, bar: BarMaterial::Glass, button: 24.0, gap: 2.0, border: 8.0, title_size: 12.0, title_bold: false, ..base },
            _ => base,
        }
    }
}

impl Default for WindowStyle {
    fn default() -> Self {
        WindowStyle { decoration: DecoKind::Mac, radius: 26.0, titlebar: 40.0, title_align: TitleAlign::Center, shadow: [0.42, 46.0, 22.0], shadow_idle: [0.22, 30.0, 12.0], client_opacity: 0.80, frame: FrameStyle::default(), max_gap: 0.0, adapt: true, hide_title_max: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScrollShow {
    /// always there (there is no way to tell a mouse from a touchpad here)
    #[default]
    Auto,
    /// only while scrolling
    Scrolling,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScrollClick {
    /// a click in the track moves a page towards it
    #[default]
    Page,
    /// a click in the track jumps to that spot
    Spot,
}

/// Small look-and-feel choices shared by the shell's windows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiStyle {
    pub sidebar: SidebarSize,
    /// let the wallpaper colour tint shell windows
    pub wall_tint: bool,
    pub scrollbars: ScrollShow,
    pub scroll_click: ScrollClick,
    /// colour of selections; none = the accent colour
    pub highlight: Option<[f32; 3]>,
    /// how switches, sliders and pickers are built
    pub controls: ControlStyle,
    /// take light / dark and the accent from the GTK theme, and keep following it
    pub follow_gtk: bool,
    /// hand light / dark, the accent and the icon theme to other programs (GTK, Qt), in a real session only
    pub style_apps: bool,
    /// the icon theme those programs get (None: leave theirs alone)
    pub icon_theme: Option<String>,
    /// how the clock and the calendar show time
    pub clock: ClockPrefs,
    /// how dense the popups of other programs (menus, tool tips) are, 0 = like the windows
    pub popup_opacity: f32,
    /// the highlight of a menu or a list glides from one row to the next
    pub slide: bool,
}

impl Default for UiStyle {
    fn default() -> Self {
        UiStyle { sidebar: SidebarSize::Medium, wall_tint: true, scrollbars: ScrollShow::Auto, scroll_click: ScrollClick::Page, highlight: None, controls: ControlStyle::default(), follow_gtk: false, style_apps: true, icon_theme: None, clock: ClockPrefs::default(), popup_opacity: 0.0, slide: true }
    }
}

/// How the clock in a panel and the calendar show time (a clock widget with a format of its own keeps it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockPrefs {
    /// 13:45 rather than 1:45 pm
    pub hour24: bool,
    pub seconds: bool,
    /// the day and the date before the time
    pub date: bool,
    /// the calendar's week starts on Monday (otherwise on Sunday)
    pub week_monday: bool,
}

impl Default for ClockPrefs {
    fn default() -> Self {
        ClockPrefs { hour24: false, seconds: false, date: true, week_monday: true }
    }
}

impl ClockPrefs {
    /// The format string of a panel's clock for these preferences.
    pub fn format(&self) -> String {
        let time = match (self.hour24, self.seconds) {
            (true, false) => "%H:%M",
            (true, true) => "%H:%M:%S",
            (false, false) => "%l:%M %P",
            (false, true) => "%l:%M:%S %P",
        };
        if self.date { format!("%a %e %b {time}") } else { time.to_string() }
    }
}

/// Which corner the notifications stack in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    /// under a top bar on the right, or the bottom right over a taskbar
    #[default]
    Auto,
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub fn left(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::BottomLeft)
    }
}

/// How notifications are shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyPrefs {
    /// do not disturb: only urgent ones are shown
    pub dnd: bool,
    pub corner: Corner,
    /// how long one stays when its sender did not say
    pub seconds: f32,
    /// the most that are shown at once
    pub max_shown: u32,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        NotifyPrefs { dnd: false, corner: Corner::Auto, seconds: 5.0, max_shown: 5 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ControlLook {
    /// domed keys with a pale ring, sunk grooves
    #[default]
    Keys,
    Flat,
}

/// The look of switches, sliders and segmented pickers, for the shell and for every Chebrium program.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlStyle {
    pub look: ControlLook,
    /// 0 .. 1.5: how much the keys stand out
    pub depth: f32,
    /// 0 .. 1: how round the corners are
    pub round: f32,
    /// the chosen segment in the accent colour
    pub accent_pick: bool,
    /// several accent colours by context, the main one staying the accent
    pub multi: bool,
}

impl Default for ControlStyle {
    fn default() -> Self {
        ControlStyle { look: ControlLook::Keys, depth: 1.0, round: 1.0, accent_pick: false, multi: false }
    }
}

impl ControlStyle {
    pub fn to_chebrium(&self) -> chebrium::style::Controls {
        chebrium::style::Controls { kind: if self.look == ControlLook::Flat { chebrium::style::ControlKind::Flat } else { chebrium::style::ControlKind::Keys }, depth: self.depth, round: self.round, accent_pick: self.accent_pick, multi: self.multi }
    }
}

/// What happens on screen when the theme switches between light and dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SwitchAnim {
    /// a circle grows from the pointer
    #[default]
    Circle,
    /// the old picture fades out
    Fade,
    /// instant
    None,
}

/// Motion settings for the whole shell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimStyle {
    /// off = everything jumps to its end state at once
    pub enabled: bool,
    /// time scale of every spring (windows, Dock, menus, ...): 1 = as designed
    pub speed: f32,
    pub switch: SwitchAnim,
    /// how long the light / dark switch takes
    pub switch_ms: f32,
    /// the bright rim on the circle's edge
    pub rim: bool,
    /// short sounds for notifications, unlocking, the theme switch
    pub sounds: bool,
    pub sound_volume: f32,
    /// per-effect settings by id (see `EFFECTS`); an effect that is not listed is on at normal speed
    pub effects: std::collections::BTreeMap<String, Effect>,
}

/// One animation's own switch and speed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effect {
    pub on: bool,
    pub speed: f32,
}

impl Default for Effect {
    fn default() -> Self {
        Effect { on: true, speed: 1.0 }
    }
}

/// Every animation that can be switched off and slowed down on its own: (group, id, name).
pub const EFFECTS: &[(&str, &str, &str)] = &[
    ("Windows", "window_open", "Opening"),
    ("Windows", "window_close", "Closing"),
    ("Windows", "window_minimize", "Minimizing (genie)"),
    ("Windows", "window_jelly", "Jelly while moving"),
    ("Windows", "workspace_slide", "Switching desktops"),
    ("Windows", "window_move", "Gliding to a place"),
    ("Windows", "window_focus", "Raising"),
    ("Menus", "menu_open", "Menus opening"),
    ("Menus", "submenu", "Submenus"),
    ("Menus", "menu_highlight", "Sliding highlight"),
    ("Shell", "launcher", "Launcher"),
    ("Shell", "control_center", "Quick Settings"),
    ("Shell", "dock", "Dock (magnify, bounce)"),
    ("Shell", "notifications", "Notifications"),
    ("Shell", "popups", "Previews and tooltips"),
];

/// How an effect is set when the theme says nothing about it: everything plays, except the jelly wobble of a window
/// being dragged, which is an option to turn on.
pub fn default_effect(id: &str) -> Effect {
    Effect { on: id != "window_jelly", speed: 1.0 }
}

impl Default for AnimStyle {
    fn default() -> Self {
        AnimStyle { enabled: true, speed: 1.0, switch: SwitchAnim::Circle, switch_ms: 800.0, rim: true, sounds: true, sound_volume: 0.8, effects: Default::default() }
    }
}

impl AnimStyle {
    /// Is this animation played at all? (When not, what it animates jumps to its end state.)
    pub fn on(&self, id: &str) -> bool {
        self.enabled && self.effects.get(id).copied().unwrap_or_else(|| default_effect(id)).on
    }

    /// The effect's own speed (1 = as designed).
    pub fn fx_speed(&self, id: &str) -> f32 {
        self.effects.get(id).map_or(1.0, |e| e.speed.clamp(0.2, 4.0))
    }

    /// Move spring `s` for effect `id` by `dt` (already scaled by the global speed): at the effect's speed, or, when the
    /// effect is off, straight to its target.
    pub fn step(&self, id: &str, s: &mut Spring, dt: f32) {
        if self.on(id) {
            s.update(dt * self.fx_speed(id));
        } else {
            s.snap();
        }
    }

    /// The time step for the springs of effect `id`: the real step times the global and the effect's speed.
    pub fn dt(&self, id: &str, dt: f32) -> f32 {
        dt * self.speed.clamp(0.2, 4.0) * self.effects.get(id).map_or(1.0, |e| e.speed.clamp(0.2, 4.0))
    }

    /// The time step springs should take for a real step of `dt` seconds.
    pub fn scale_dt(&self, dt: f32) -> f32 {
        if !self.enabled {
            // long enough for any spring to settle within a frame or two
            dt * 40.0
        } else {
            dt * self.speed.clamp(0.2, 4.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenubarStyle {
    pub height: f32,
    /// draw a full-width bar behind the menu bar items
    pub bar: bool,
    pub kind: MenubarKind,
    /// glass: how strong the tint is; solid: how opaque the bar is (0 = invisible, 1 = fully opaque)
    pub opacity: f32,
    pub blur: f32,
    /// hairline along the bottom edge
    pub border: bool,
    /// logo and the focused app's name on the left
    pub show_title: bool,
    /// solid bar: its own colour (GNOME's top bar is black in a light theme); `None` follows light / dark
    pub color: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MenubarKind {
    #[default]
    Glass,
    Solid,
    /// blur and tint that fade out away from the screen edge
    Fade,
    /// no background at all
    Clear,
}

impl Default for MenubarStyle {
    fn default() -> Self {
        MenubarStyle { height: 38.0, bar: true, kind: MenubarKind::Glass, opacity: 0.35, blur: 16.0, border: true, show_title: true, color: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockStyle {
    pub icon_size: f32,
    pub magnify: bool,
    pub radius: f32,
    pub gap: f32,
}

impl Default for DockStyle {
    fn default() -> Self {
        DockStyle { icon_size: 64.0, magnify: true, radius: 36.0, gap: 14.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LauncherStyle {
    pub style: LauncherKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IconStyle {
    /// icon theme name, or "auto" (take the desktop's setting)
    pub theme: String,
    pub shape: IconShape,
    pub plate: Plate,
    /// inner padding as a fraction of the icon size
    pub padding: f32,
    /// tile colour for `plate = "tile"`
    pub tile: TileMode,
    /// one-colour tint (monochrome look); None keeps the original colours
    pub tint: Option<[f32; 3]>,
    /// application id -> icon name or file path
    pub overrides: HashMap<String, String>,
    /// folder of SVGs that replace UI glyphs by file name (e.g. wifi.svg)
    pub ui_pack: Option<String>,
    /// what the simple icons with a plate were set to when "our" icons were chosen, to come back to
    pub simple: Option<SimpleIcons>,
    /// which look of our icons the shell shows (see `OWN_LOOKS`)
    pub own_look: String,
}

/// The settings of the simple icons (a pack and the shell's own plate), kept while "our" icons are in use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimpleIcons {
    pub theme: String,
    pub shape: IconShape,
    pub plate: Plate,
    pub padding: f32,
    pub tile: TileMode,
}

/// The icon theme the Memrium Icons editor writes: our own icons.
pub const OWN_ICON_THEME: &str = "Memrium-Icons";

/// The looks of our icons the shell can show: (key, name, the theme of that look; empty for the light/dark one that follows
/// the shell).
pub const OWN_LOOKS: [(&str, &str, &str); 8] = [
    ("auto", "Follow light / dark", ""),
    ("default", "Default", "Memrium-Icons"),
    ("dark", "Dark", "Memrium-Icons-Dark"),
    ("clear-light", "Clear (light)", "Memrium-Icons-Clear-Light"),
    ("clear-dark", "Clear (dark)", "Memrium-Icons-Clear-Dark"),
    ("tinted-light", "Tinted (light)", "Memrium-Icons-Tinted-Light"),
    ("tinted-dark", "Tinted (dark)", "Memrium-Icons-Tinted-Dark"),
    ("tinted-icon", "Tinted by icon", "Memrium-Icons-Tinted-Icon"),
];

impl Default for IconStyle {
    fn default() -> Self {
        IconStyle { theme: "auto".into(), shape: IconShape::Rounded, plate: Plate::Tile, padding: 0.16, tile: TileMode::Vivid, tint: None, overrides: HashMap::new(), ui_pack: None, simple: None, own_look: "auto".into() }
    }
}

impl IconStyle {
    /// The icon theme to look icons up in: the chosen pack, or the theme of the look of our icons that is chosen (for a
    /// dark shell when `dark`, if the look follows light and dark).
    pub fn theme_name(&self, dark: bool) -> String {
        if !self.is_own() {
            return self.theme.clone();
        }
        match OWN_LOOKS.iter().find(|l| l.0 == self.own_look) {
            Some((_, _, name)) if !name.is_empty() => name.to_string(),
            _ => if dark { "Memrium-Icons-Dark".to_string() } else { OWN_ICON_THEME.to_string() },
        }
    }

    /// The icon theme the shell itself looks icons up in: `theme_name`, except that for a clear look of our icons it is the
    /// version without plates (when the editor wrote one), whose plates the shell draws as live glass (`push_app_icon`).
    /// The other programs keep the theme with the plates painted in.
    pub fn shell_theme_name(&self, dark: bool) -> String {
        let name = self.theme_name(dark);
        if self.is_own() && self.live_look() {
            let live = format!("{name}{}", crate::iconrender::LIVE_SUFFIX);
            if crate::iconrender::theme_dir().with_file_name(&live).join("index.theme").exists() {
                return live;
            }
        }
        name
    }

    /// Is a clear look of our icons chosen (their plate drawn live by the shell)?
    pub fn live_look(&self) -> bool {
        matches!(self.own_look.as_str(), "clear-light" | "clear-dark")
    }

    /// Are our own icons (made in the editor, each with its own plate) in use?
    pub fn is_own(&self) -> bool {
        self.theme == OWN_ICON_THEME
    }

    /// Switch between our own icons (no plate of the shell's: they bring theirs) and the simple icons of a pack with the
    /// shell's plate. The settings of the one that is left are kept for the way back.
    pub fn use_own(&mut self, own: bool) {
        if own == self.is_own() {
            return;
        }
        if own {
            self.simple = Some(SimpleIcons { theme: self.theme.clone(), shape: self.shape, plate: self.plate, padding: self.padding, tile: self.tile });
            self.theme = OWN_ICON_THEME.into();
            self.shape = IconShape::None;
            self.plate = Plate::None;
            self.padding = 0.0;
        } else {
            let back = self.simple.take().unwrap_or(SimpleIcons { theme: "auto".into(), shape: IconShape::Rounded, plate: Plate::Tile, padding: 0.16, tile: TileMode::Vivid });
            self.theme = back.theme;
            self.shape = back.shape;
            self.plate = back.plate;
            self.padding = back.padding;
            self.tile = back.tile;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperSpec {
    pub kind: WallKind,
    /// image path, live wallpaper name or .wgsl path, or video path
    pub path: String,
}

impl Default for WallpaperSpec {
    fn default() -> Self {
        WallpaperSpec { kind: WallKind::Image, path: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub layout: Layout,
    pub dark: bool,
    pub accent: [f32; 3],
    pub glass: GlassStyle,
    pub window: WindowStyle,
    pub menubar: MenubarStyle,
    /// which widgets the bar holds (macOS layout)
    pub bars: Vec<crate::widget::BarConfig>,
    pub dock: DockStyle,
    pub launcher: LauncherStyle,
    pub icons: IconStyle,
    pub wallpaper: WallpaperSpec,
    /// the file manager preset this template brings ("Dolphin", "Icons", "Details", "Minimal")
    pub files: Option<String>,
    pub animations: AnimStyle,
    pub ui: UiStyle,
    /// follow the time of day: dark in the evening and at night (the `dark` field is then set by the shell)
    pub auto_dark: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            name: "Jelly Glass".into(),
            layout: Layout::Macos,
            dark: false,
            accent: [0.04, 0.52, 1.0],
            glass: GlassStyle::default(),
            // (the built-in Jelly Glass is the macos.toml preset)
            window: WindowStyle { hide_title_max: true, ..WindowStyle::default() },
            menubar: MenubarStyle::default(),
            bars: crate::widget::default_bars(),
            dock: DockStyle::default(),
            launcher: LauncherStyle::default(),
            icons: IconStyle::default(),
            wallpaper: WallpaperSpec::default(),
            files: None,
            animations: AnimStyle::default(),
            ui: UiStyle::default(),
            auto_dark: false,
        }
    }
}

impl IconStyle {
    fn shape_id(&self) -> f32 {
        match self.shape {
            IconShape::None => 0.0,
            IconShape::Squircle => 1.0,
            IconShape::Circle => 2.0,
            IconShape::Rounded => 3.0,
        }
    }

    /// The tile every icon is set on, when the plate is `Tile`.
    pub fn tile_style(&self) -> Option<crate::renderer::TileStyle> {
        (self.plate == Plate::Tile).then(|| crate::renderer::TileStyle { shape: if self.shape == IconShape::None { IconShape::Squircle } else { self.shape }, padding: if self.padding < 0.04 { 0.16 } else { self.padding }, mode: self.tile })
    }

    /// Corner radius of the icon shape for an icon of `size` px (used for plates and shadows).
    pub fn radius(&self, size: f32) -> f32 {
        match self.shape {
            IconShape::None => size * 0.12,
            IconShape::Squircle => size * 0.225,
            IconShape::Circle => size * 0.5,
            IconShape::Rounded => size * 0.22,
        }
    }
}

/// The application icons whose plate the shell draws live (atlas cells of icons from a `-Live` theme), and the glass
/// settings of the editor that say what shape that plate is. Kept here because the icons are drawn from many places that
/// only know the cell.
struct LiveIcons {
    cells: std::collections::HashSet<u32>,
    tune: crate::iconrender::GlassTune,
}

static LIVE_ICONS: std::sync::RwLock<Option<LiveIcons>> = std::sync::RwLock::new(None);

/// Forget the live icons (the icons are loaded again) and read the editor's glass settings again.
pub fn reset_live_icons() {
    if let Ok(mut l) = LIVE_ICONS.write() {
        *l = Some(LiveIcons { cells: Default::default(), tune: crate::iconrender::GlassTune::load() });
    }
}

/// Mark atlas cell `cell` as an icon whose plate is drawn live (or not).
pub fn set_live_icon(cell: u32, live: bool) {
    if let Ok(mut l) = LIVE_ICONS.write() {
        let l = l.get_or_insert_with(|| LiveIcons { cells: Default::default(), tune: crate::iconrender::GlassTune::load() });
        if live {
            l.cells.insert(cell);
        } else {
            l.cells.remove(&cell);
        }
    }
}

/// The live plate of the icon in `cell` of `size` px: its side and corner radius (see `iconrender::live_plate`).
fn live_plate(cell: u32, size: f32) -> Option<(f32, f32)> {
    let l = LIVE_ICONS.read().ok()?;
    let l = l.as_ref()?;
    if !l.cells.contains(&cell) {
        return None;
    }
    crate::iconrender::live_plate(&l.tune, size)
}

/// Push the panels of an application icon (optional plate, then the icon) in the theme's icon style.
pub fn push_app_icon(theme: &Theme, out: &mut Vec<crate::renderer::Panel>, cx: f32, cy: f32, size: f32, cell: u32, shadow: (f32, f32, f32)) {
    use crate::renderer::Panel;
    let st = &theme.icons;
    let r = st.radius(size);
    // a clear icon of ours: its plate is glass drawn here, over what is really behind it (the picture holds the layers only)
    if st.is_own() && st.live_look() {
        if let Some((side, radius)) = live_plate(cell, size) {
            let dark = st.own_look == "clear-dark";
            let mut g = theme.glass.panel(cx, cy, side, side, radius);
            // a small slab, not a big lens: the bending and bezel scale with it (as the glass plates of the simple icons)
            g.params[2] = g.params[2].min(side * 0.20);
            g.params[3] = g.params[3].min(side * 0.12);
            g.fx[0] = g.fx[0].clamp(6.0, 9.0);
            g.fx[1] = g.fx[1].min(0.04);
            // the clear plates of the editor: a pale frost in the light look, a smoky one in the dark
            g.tint = if dark { [0.10, 0.10, 0.12, 0.42] } else { [0.97, 0.97, 1.0, 0.24] };
            g.extra = [shadow.0, shadow.1, shadow.2, theme.glass.saturation];
            out.push(g);
            let mut ic = Panel::app_icon(cx, cy, size, cell, 1.0);
            ic.params[0] = r;
            ic.params[3] = 0.0;
            out.push(ic);
            return;
        }
    }
    match st.plate {
        Plate::None => {}
        Plate::Glass => {
            let mut g = theme.glass.panel(cx, cy, size, size, r);
            // a plate is a small slab under an icon, not a lens: the strong bending and wide bezel a big panel is set to would
            // smear the busy picture behind it into a coloured mess round the icon, so they scale with the plate
            g.params[2] = g.params[2].min(size * 0.22);
            g.params[3] = g.params[3].min(size * 0.14);
            g.fx[0] = g.fx[0].clamp(8.0, 10.0);
            g.fx[1] = g.fx[1].min(0.04);
            // frosted enough that the icon reads on any picture
            g.tint[3] = g.tint[3].max(0.38);
            g.extra = [shadow.0, shadow.1, shadow.2, theme.glass.saturation];
            out.push(g);
        }
        Plate::Solid => {
            let c = if theme.dark { [0.219, 0.219, 0.219] } else { [0.99, 0.99, 1.0] };
            out.push(Panel::solid(cx, cy, size, size, r, c, c).flat().shadow(shadow.0, shadow.1, shadow.2));
        }
        Plate::Tile => {}
    }
    if st.plate == Plate::Tile {
        // the tile (shape, padding, colours) is already part of the icon image
        let mut ic = Panel::app_icon(cx, cy, size, cell, 1.0);
        ic.params[0] = r;
        ic.params[3] = 0.0;
        out.push(ic.shadow(shadow.0, shadow.1, shadow.2));
        return;
    }
    let mut ic = Panel::app_icon(cx, cy, size, cell, 1.0);
    ic.params[0] = r;
    ic.params[3] = st.shape_id();
    ic.fx[0] = st.padding.clamp(0.0, 0.6) + if st.plate != Plate::None { 0.12 } else { 0.0 };
    if let Some(t) = st.tint {
        ic.fx[1] = 1.0;
        ic.color2 = [t[0], t[1], t[2], 1.0];
    }
    if st.plate == Plate::None {
        ic = ic.shadow(shadow.0, shadow.1, shadow.2);
    }
    out.push(ic);
}

impl Theme {
}

// ------------------------------------------------------------------ presets and config

pub const PRESETS: [(&str, &str); 5] = [
    ("macos", include_str!("../themes/macos.toml")),
    ("unity", include_str!("../themes/unity.toml")),
    ("gnome", include_str!("../themes/gnome.toml")),
    ("windows11", include_str!("../themes/windows11.toml")),
    ("windows7", include_str!("../themes/windows7.toml")),
];

fn parse_theme(text: &str) -> Result<Theme, toml::de::Error> {
    toml::from_str(text)
}

/// A built-in theme by id ("macos", "gnome", "windows11", "windows7").
pub fn preset(id: &str) -> Option<Theme> {
    PRESETS.iter().find(|(n, _)| *n == id).and_then(|(_, t)| parse_theme(t).ok())
}

/// What programs that are not part of the shell (Chebrium apps) need to look like it: light or dark, the accent, the
/// shape of windows, the glass and the language. The shell writes it whenever the look changes; Chebrium reads it
/// (`chebrium::theme::Appearance`) and follows live.
pub fn appearance_text(theme: &Theme, language: &str) -> String {
    let deco = match theme.window.decoration {
        DecoKind::Mac => "mac",
        DecoKind::Win11 => "win11",
        DecoKind::Aero => "aero",
        DecoKind::Gnome => "gnome",
        DecoKind::Minimal => "minimal",
        DecoKind::Custom => "mac",
    };
    let g = &theme.glass;
    format!(
        "# written by the shell: the look the programs follow\ndark = {}\naccent = [{:.4}, {:.4}, {:.4}]\nradius = {:.1}\ntitlebar = {:.1}\ndecoration = \"{deco}\"\nopacity = {:.3}\nlanguage = \"{}\"\nglass_tint = [{:.4}, {:.4}, {:.4}, {:.4}]\nglass_border = [{:.4}, {:.4}, {:.4}, {:.4}]\ncontrols = \"{}\"\ncontrols_depth = {:.3}\ncontrols_round = {:.3}\ncontrols_accent = {}\ncontrols_multi = {}\nmenubar = {}\npopup_opacity = {:.3}\nslide = {}\n",
        theme.dark, theme.accent[0], theme.accent[1], theme.accent[2], theme.window.radius, theme.window.titlebar, theme.window.client_opacity, language, g.tint[0], g.tint[1], g.tint[2], g.tint[3], g.border[0], g.border[1], g.border[2], g.border[3], if theme.ui.controls.look == ControlLook::Flat { "flat" } else { "keys" }, theme.ui.controls.depth, theme.ui.controls.round, theme.ui.controls.accent_pick, theme.ui.controls.multi, theme.bars.iter().any(|b| b.left.iter().chain(&b.center).chain(&b.right).any(|w| w.kind == "app-menu")), theme.ui.popup_opacity, theme.ui.slide
    )
}

/// Write `appearance.toml` next to the config (only when it changed).
pub fn export_appearance(theme: &Theme, language: &str) {
    crate::gtkcss::export(theme);
    crate::kdecolors::export(theme);
    crate::iconrender::follow_accent(theme.accent);
    let text = appearance_text(theme, language);
    let path = config_dir().join("appearance.toml");
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return;
    }
    let _ = std::fs::create_dir_all(config_dir());
    // written next to the file and renamed, so a program that reads it at that moment never sees half of it
    let tmp = path.with_extension("toml.tmp");
    if std::fs::write(&tmp, &text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("memrium")
}

/// What is stored in config.toml: which theme, and the differences from it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// preset id, or the file name (without .toml) of a theme in ~/.config/memrium/themes/
    pub theme: String,
    /// any Theme fields, merged over the chosen theme
    #[serde(rename = "override")]
    pub overrides: toml::Table,
    /// favourite apps in the Dock/taskbar (desktop ids); empty = automatic
    pub pinned: Vec<String>,
    /// file manager preferences
    pub files: crate::files::FilesPrefs,
    /// icons on the desktop
    pub desktop: crate::desktop::DesktopPrefs,
    /// pointer, scrolling, keyboard repeat
    pub input: crate::input::InputPrefs,
    /// PrtSc screenshots
    pub screenshots: crate::shots::ScreenshotPrefs,
    /// auto-lock, screen off and suspend after inactivity
    pub power: crate::idle::IdlePrefs,
    /// keyboard shortcuts of the shell
    pub keys: crate::keys::KeyBinds,
    /// city for the weather in the widgets panel (empty = no weather, nothing is fetched)
    pub weather_city: String,
    /// monitors: which is the main one, where the others sit, scale, modes
    pub display: crate::display::DisplayPrefs,
    /// widgets placed on the desktop
    pub desk_widgets: Vec<crate::deskwidgets::DeskWidget>,
    /// the shell's language: "auto" (follow the environment) or a code such as "uk"
    pub language: String,
    /// the tiles of the Quick Settings (empty = the built-in layout), see `cctiles`
    pub cc_tiles: Vec<crate::cctiles::CcTile>,
    /// how notifications are shown
    pub notifications: NotifyPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Config { theme: "macos".into(), overrides: toml::Table::new(), pinned: Vec::new(), files: Default::default(), desktop: Default::default(), input: Default::default(), screenshots: Default::default(), power: Default::default(), keys: Default::default(), weather_city: String::new(), display: Default::default(), desk_widgets: Vec::new(), language: "auto".into(), cc_tiles: Vec::new(), notifications: NotifyPrefs::default() }
    }
}

fn merge(base: &mut toml::Table, over: &toml::Table) {
    for (k, v) in over {
        match (base.get_mut(k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

fn same(a: &toml::Value, b: &toml::Value) -> bool {
    use toml::Value::*;
    match (a, b) {
        (Float(x), Float(y)) => (x - y).abs() < 1e-4,
        (Float(x), Integer(y)) | (Integer(y), Float(x)) => (x - *y as f64).abs() < 1e-4,
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

/// Round floats so the stored file stays readable (0.20000000298 -> 0.2).
fn tidy(v: &toml::Value) -> toml::Value {
    match v {
        toml::Value::Float(f) => toml::Value::Float((f * 1e4).round() / 1e4),
        toml::Value::Array(a) => toml::Value::Array(a.iter().map(tidy).collect()),
        toml::Value::Table(t) => toml::Value::Table(t.iter().map(|(k, v)| (k.clone(), tidy(v))).collect()),
        other => other.clone(),
    }
}

/// Changed leaves of `cur` compared to `base` (tables recurse; arrays and scalars compare whole).
fn diff(base: &toml::Table, cur: &toml::Table) -> toml::Table {
    let mut out = toml::Table::new();
    for (k, v) in cur {
        match (base.get(k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(c)) => {
                let d = diff(b, c);
                if !d.is_empty() {
                    out.insert(k.clone(), toml::Value::Table(d));
                }
            }
            (Some(b), c) if same(b, c) => {}
            _ => {
                out.insert(k.clone(), tidy(v));
            }
        }
    }
    out
}

/// The chosen theme as a full table: file contents with every missing field filled from the defaults.
fn base_table(theme_id: &str) -> toml::Table {
    let text = PRESETS
        .iter()
        .find(|(n, _)| *n == theme_id)
        .map(|(_, t)| t.to_string())
        .or_else(|| std::fs::read_to_string(config_dir().join("themes").join(format!("{theme_id}.toml"))).ok())
        .unwrap_or_else(|| PRESETS[0].1.to_string());
    let theme = parse_theme(&text).or_else(|_| parse_theme(PRESETS[0].1)).unwrap_or_default();
    match toml::Value::try_from(&theme) {
        Ok(toml::Value::Table(t)) => t,
        _ => toml::Table::new(),
    }
}

impl Config {
    /// The resolved theme: preset + overrides. A broken config falls back to the macOS preset.
    pub fn resolve(&self) -> Theme {
        let mut t = base_table(&self.theme);
        merge(&mut t, &self.overrides);
        match toml::Value::Table(t).try_into::<Theme>() {
            Ok(theme) => theme,
            Err(e) => {
                eprintln!("theme: {e}; using the built-in Jelly Glass theme");
                preset("macos").unwrap_or_default()
            }
        }
    }

    /// Choose another base theme and forget overrides (a clean template switch).
    pub fn select(&mut self, theme: &str) {
        self.theme = theme.to_string();
        self.overrides = toml::Table::new();
    }

    /// Store `current` as base theme + minimal overrides.
    pub fn set_from(&mut self, current: &Theme) {
        let base = base_table(&self.theme);
        if let Ok(toml::Value::Table(cur)) = toml::Value::try_from(current) {
            self.overrides = diff(&base, &cur);
        }
    }

    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Config {
        let mut cfg = match std::fs::read_to_string(Self::path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("config: {e}; using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        // handy for trying a template without touching the saved config
        if let Ok(t) = std::env::var("MEMRIUM_THEME") {
            cfg.select(&t);
        }
        cfg
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(config_dir());
        match toml::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(Self::path(), text) {
                    eprintln!("config: could not save: {e}");
                }
            }
            Err(e) => eprintln!("config: {e}"),
        }
    }
}

/// "My Theme!" -> "my-theme"
pub fn slug(name: &str) -> String {
    let s: String = name.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "theme".into() } else { s }
}

/// Keep `theme` as a template of its own (it shows up next to the built-in ones).
pub fn save_template(theme: &Theme, name: &str) -> Result<PathBuf, String> {
    let mut t = theme.clone();
    t.name = name.trim().to_string();
    let dir = config_dir().join("themes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.toml", slug(name)));
    let text = toml::to_string_pretty(&t).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Take a theme file somebody shared into the templates folder; returns its name.
pub fn import_template(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let theme = parse_theme(&text).map_err(|e| format!("not a theme: {e}"))?;
    let dir = config_dir().join("themes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{}.toml", slug(&theme.name))), text).map_err(|e| e.to_string())?;
    Ok(theme.name)
}

/// Theme ids that can be chosen: presets first, then the user's own files.
pub fn available_themes() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = PRESETS.iter().map(|(id, t)| (id.to_string(), parse_theme(t).map(|t| t.name).unwrap_or_else(|_| id.to_string()))).collect();
    if let Ok(rd) = std::fs::read_dir(config_dir().join("themes")) {
        for e in rd.flatten() {
            if let Some(stem) = e.path().file_stem().and_then(|s| s.to_str()) {
                if e.path().extension().and_then(|x| x.to_str()) == Some("toml") && !v.iter().any(|(id, _)| id == stem) {
                    let name = std::fs::read_to_string(e.path()).ok().and_then(|t| parse_theme(&t).ok()).map(|t| t.name).unwrap_or_else(|| stem.to_string());
                    v.push((stem.to_string(), name));
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_names_become_file_names() {
        assert_eq!(slug("My Theme!"), "my-theme");
        assert_eq!(slug("  "), "theme");
        assert_eq!(slug("Аква 2"), "аква-2");
    }

    #[test]
    fn glass_amount_runs_from_flat_to_the_reference() {
        let mut g = GlassStyle::default();
        g.set_mode(GlassMode::Glass);
        g.amount = 0.0;
        g.apply_mode();
        let flat = GlassStyle::flat();
        assert_eq!((g.blur, g.refraction, g.rim, g.gloss), (flat.blur, flat.refraction, flat.rim, flat.gloss));
        g.amount = 1.0;
        g.apply_mode();
        let reference = GlassStyle::default();
        assert_eq!((g.blur, g.refraction, g.bezel, g.tint), (reference.blur, reference.refraction, reference.bezel, reference.tint));
    }

    #[test]
    fn custom_values_come_back_after_visiting_another_mode() {
        let mut g = GlassStyle { blur: 13.0, refraction: 77.0, ..GlassStyle::default() };
        g.set_mode(GlassMode::Flat);
        assert_ne!(g.refraction, 77.0);
        g.set_mode(GlassMode::Custom);
        assert_eq!((g.blur, g.refraction), (13.0, 77.0));
        assert_eq!(g.mode, GlassMode::Custom);
    }

    #[test]
    fn panel_overrides_round_trip_through_the_config() {
        let mut cfg = Config::default();
        cfg.select("windows11");
        let mut t = cfg.resolve();
        t.bars[0].height = Some(60.0);
        t.bars[0].center[2].opts.insert("icon_size".into(), toml::Value::Float(30.0));
        t.glass.rim_sharp = 0.8;
        cfg.set_from(&t);
        assert_eq!(cfg.resolve(), t);
    }

    #[test]
    fn bars_can_be_overridden_from_the_config_file() {
        let text = "theme = \"macos\"\n[override]\n[[override.bars]]\nname = \"menubar\"\nedge = \"top\"\nleft = [\"logo\"]\nright = [{ kind = \"command\", command = \"date\" }, \"clock\"]\n";
        let cfg: Config = toml::from_str(text).unwrap();
        let t = cfg.resolve();
        assert_eq!(t.bars.len(), 1);
        assert_eq!(t.bars[0].right[0].kind, "command");
    }

    #[test]
    fn only_jelly_glass_hides_the_title_bar_of_a_maximized_window() {
        for (id, _) in PRESETS {
            let t = preset(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(t.window.hide_title_max, id == "macos", "{id}");
        }
        assert!(Theme::default().window.hide_title_max);
    }

    #[test]
    fn old_configs_with_a_taskbar_table_still_load() {
        let t: Result<Theme, _> = toml::from_str("layout = \"taskbar\"\n[taskbar]\nsize = 40.0\n");
        assert!(t.is_ok(), "unknown legacy tables must be ignored");
    }
}

/// Hue (0 .. 1), saturation, value to RGB (all 0 .. 1).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let (i, f) = (h.floor(), h - h.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as u32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// RGB to (hue, saturation, value), all 0 .. 1.
pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 1e-6 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if mx == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

/// "#RRGGBB" for a colour.
pub fn to_hex(c: [f32; 3]) -> String {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]))
}

/// A colour from "#RRGGBB", "RRGGBB" or "#RGB".
pub fn from_hex(text: &str) -> Option<[f32; 3]> {
    let t = text.trim().trim_start_matches('#');
    let digits: Vec<u32> = t.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
    let (r, g, b) = match digits.len() {
        6 => (digits[0] * 16 + digits[1], digits[2] * 16 + digits[3], digits[4] * 16 + digits[5]),
        3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17),
        _ => return None,
    };
    Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0])
}

#[cfg(test)]
mod colour_tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [[0.04, 0.52, 1.0], [0.9, 0.3, 0.1], [0.2, 0.72, 0.4], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]] {
            let h = rgb_to_hsv(c);
            let back = hsv_to_rgb(h[0], h[1], h[2]);
            assert!((0..3).all(|i| (back[i] - c[i]).abs() < 1e-4), "{c:?} -> {h:?} -> {back:?}");
        }
    }

    #[test]
    fn hex_parses_and_prints() {
        assert_eq!(to_hex([1.0, 0.0, 0.5019608]), "#FF0080");
        assert_eq!(from_hex("#ff0080").map(to_hex), Some("#FF0080".into()));
        assert_eq!(from_hex("0af").map(to_hex), Some("#00AAFF".into()));
        assert_eq!(from_hex("#12"), None);
        assert_eq!(from_hex("zzzzzz"), None);
    }

    #[test]
    fn our_icons_and_simple_icons_can_be_switched_and_come_back() {
        let mut icons = IconStyle { theme: "Papirus".into(), padding: 0.2, tile: TileMode::Dark, ..IconStyle::default() };
        assert!(!icons.is_own());
        icons.use_own(true);
        assert!(icons.is_own());
        assert_eq!((icons.plate, icons.shape), (Plate::None, IconShape::None), "our icons bring their own plate");
        icons.use_own(false);
        assert_eq!((icons.theme.as_str(), icons.plate, icons.shape, icons.tile), ("Papirus", Plate::Tile, IconShape::Rounded, TileMode::Dark), "the simple icons are as they were");
        assert!((icons.padding - 0.2).abs() < 1e-6);
        icons.use_own(true);
        assert_eq!(icons.theme_name(false), "Memrium-Icons");
        assert_eq!(icons.theme_name(true), "Memrium-Icons-Dark", "the default choice follows the dark theme");
        icons.own_look = "clear-light".into();
        assert_eq!(icons.theme_name(true), "Memrium-Icons-Clear-Light", "a look that was chosen stays");
        icons.use_own(false);
        // without anything kept, the defaults come back
        let mut own = IconStyle { theme: OWN_ICON_THEME.into(), plate: Plate::None, shape: IconShape::None, ..IconStyle::default() };
        own.use_own(false);
        assert_eq!((own.theme.as_str(), own.plate), ("auto", Plate::Tile));
    }
}
