//! The pieces of a Windows-style taskbar as ordinary bar widgets: start button, search box, task buttons, tray and
//! "show desktop". Each one reads its own options from the widget spec, so every piece is configured separately:
//!
//! ```toml
//! [[bars]]
//! name = "taskbar"
//! surface = "glass"
//! center = ["start", { kind = "search-box", width = 190 }, { kind = "dock", icon_size = 26, button_w = 44, magnify = false, indicator = "underline" }]
//! right  = [{ kind = "tray", icons = ["wifi", "volume-2"] }, { kind = "clock", format = "%l:%M %p", format2 = "%m/%d/%Y" }]
//! ```

use crate::renderer::{Align, Label, Panel};
use chebrium::lucide;
use chebrium::spring::Spring;
use crate::widget::{glyph, pack_icon, tone, BarClick, BarCtx, CcRole, Drawn, Widget, WidgetSpec};

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

fn hover_spring() -> Spring {
    Spring::new(0.0, 300.0, 0.8)
}

fn glow(c: &BarCtx) -> f32 {
    if c.theme.dark { 0.14 } else { 0.5 }
}

// ---------------------------------------------------------------------------------------- start

/// The start button: an icon of your choice (a glyph, or any icon of the icon pack in use) on its own, on a rounded
/// plate, in a glass orb or under a flat Breeze highlight, with a text label if wanted (as in Plasma's launcher).
/// Opens the start menu of its own choice, right where the button is.
pub struct Start {
    orb: bool,
    plate: bool,
    breeze: bool,
    /// the icon is left out (text only)
    text_only: bool,
    /// the menu it opens: "theme" (the theme's launcher) or a kind, where it sits, favourites as a grid
    menu: String,
    menu_align: String,
    grid: bool,
    size: f32,
    icon_size: f32,
    glyph: String,
    pack_icon: String,
    themed: bool,
    color: String,
    plate_color: String,
    hover_fx: String,
    stay_lit: bool,
    shine: bool,
    label: String,
    action: String,
    hover: Spring,
}

impl Start {
    pub fn new(spec: &WidgetSpec) -> Self {
        let style = spec.s("style");
        let display = spec.s("display");
        let label = spec.s("label");
        // (the label shows when asked for, or when there is one and nothing says otherwise)
        let label = match display.as_str() {
            "icon" => String::new(),
            "icon_text" | "text" if label.is_empty() => "Applications".to_string(),
            _ => label,
        };
        Start {
            orb: style == "orb",
            plate: style == "plate",
            breeze: style == "breeze",
            text_only: display == "text",
            menu: spec.s("menu"),
            menu_align: spec.s("menu_align"),
            grid: spec.flag("grid"),
            size: spec.n("size"),
            icon_size: spec.n("icon_size"),
            glyph: spec.s("glyph"),
            pack_icon: spec.s("pack_icon"),
            themed: spec.flag("themed"),
            color: spec.s("color"),
            plate_color: spec.s("plate_color"),
            hover_fx: spec.s("hover"),
            stay_lit: spec.flag("stay_lit"),
            shine: spec.flag("shine"),
            label,
            action: spec.str("action").unwrap_or("start").to_string(),
            hover: hover_spring(),
        }
    }

    /// Width of the icon part of the button (0 with text only).
    fn icon_w(&self) -> f32 {
        if self.text_only { 10.0 } else { self.size }
    }

    fn rect(&self, c: &BarCtx, r: [f32; 4]) -> [f32; 4] {
        let h = if self.orb { self.size } else { self.size.min(r[3] - 4.0).max(16.0) };
        let w = self.icon_w() + self.label_w(c);
        [r[0] + 4.0, r[1] + (r[3] - h) / 2.0, w, h]
    }

    fn label_w(&self, c: &BarCtx) -> f32 {
        if self.label.is_empty() { 0.0 } else { (c.measure)(&self.label, 13.0, true) + 6.0 }
    }
}

impl Widget for Start {
    fn kind(&self) -> &'static str {
        "start"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        self.icon_w() + self.label_w(c) + 8.0
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        vec![self.width(c)]
    }
    fn update(&mut self, dt: f32, c: &BarCtx, r: [f32; 4]) {
        self.hover.target = if inside(c.mouse, self.rect(c, r)) { 1.0 } else { 0.0 };
        self.hover.update(dt);
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        let b = self.rect(c, r);
        let sh = self.hover.x;
        let lit = self.stay_lit && c.start_open;
        let glow_on = self.hover_fx == "glow";
        // the icon sits in the square at the left; a label follows it
        let (cx, cy) = (b[0] + self.size / 2.0, b[1] + b[3] / 2.0);
        let grow = if self.hover_fx == "grow" { 1.0 + 0.12 * sh } else { 1.0 };
        let col = tone(&self.color, c);
        let pc = tone(&self.plate_color, c);
        let mut ink = col;
        if self.orb {
            let d = self.size * grow;
            let boost = if glow_on { 0.12 * sh } else { 0.0 } + if lit { 0.15 } else { 0.0 };
            let (hi, lo) = ([pc[0] * 0.55 + 0.45, pc[1] * 0.55 + 0.45, pc[2] * 0.55 + 0.45], [pc[0] * 0.45, pc[1] * 0.45, pc[2] * 0.45]);
            let up = |v: [f32; 3]| [(v[0] + boost).min(1.0), (v[1] + boost).min(1.0), (v[2] + boost).min(1.0)];
            out.panels.push(Panel::solid(cx, cy, d, d, d / 2.0, [lo[0] * 0.3; 3], [lo[0] * 0.3; 3]).flat().alpha(0.55));
            out.panels.push(Panel::solid(cx, cy, d - 3.0, d - 3.0, (d - 3.0) / 2.0, up(hi), up(lo)).flat());
            if self.shine {
                out.panels.push(Panel::solid(cx, cy - d * 0.20, d * 0.74, d * 0.42, d * 0.21, [1.0; 3], [1.0; 3]).flat().alpha(0.30));
            }
            if self.color == "accent" {
                ink = [1.0; 3];
            }
        } else if self.plate {
            let strength = if glow_on { 0.25 * sh } else { 0.0 } + if lit { 0.30 } else { 0.0 };
            out.panels.push(Panel::solid(cx, cy, self.size * grow, b[3] * grow, 9.0, pc, pc).flat().alpha(0.55 + strength * 0.4));
            if self.color == "accent" {
                ink = [1.0; 3];
            }
        } else if self.breeze {
            // Breeze: a faint plate under the pointer, the accent while the menu is open (over the icon and the label)
            let (mx, my) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
            if sh > 0.01 && glow_on {
                out.panels.push(Panel::solid(mx, my, b[2] + 4.0, b[3], 5.0, c.ink, c.ink).flat().alpha(0.10 * sh));
            }
            if lit {
                out.panels.push(Panel::solid(mx, my, b[2] + 4.0, b[3], 5.0, c.theme.accent, c.theme.accent).flat().alpha(0.30));
            }
        } else if (glow_on && sh > 0.01) || lit {
            out.panels.push(Panel::solid(cx, cy, self.size, b[3], 6.0, [1.0; 3], [1.0; 3]).flat().alpha(if glow_on { 0.45 * sh } else { 0.0 } + if lit { 0.25 } else { 0.0 }));
        }
        if !self.text_only {
            let isz = self.icon_size * grow;
            if !pack_icon(c, out, &self.pack_icon, cx, cy, isz, self.themed, 1.0) {
                // (a glyph while the pack's icon loads, or when none is chosen)
                let g = if lucide::ICONS.iter().any(|(n, _)| *n == self.glyph) { self.glyph.as_str() } else { "layout-grid" };
                out.panels.push(Panel::glyph(cx, cy, isz, g, ink, 0.95));
            }
        }
        if !self.label.is_empty() {
            let lx = if self.text_only { b[0] + 5.0 } else { b[0] + self.size + 2.0 };
            out.labels.push(Label::new(self.label.clone(), lx, cy + 4.8, 13.0, true, c.ink, 0.95, Align::Left));
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        // the start action carries the menu to open, where it sits and its favourites view
        let action = if self.action == "start" { format!("start:{}:{}:{}", self.menu, self.menu_align, self.grid as u8) } else { self.action.clone() };
        Some(BarClick::Action(action))
    }
}

// ---------------------------------------------------------------------------------------- search box

/// A search field that opens the launcher.
pub struct SearchBox {
    width: f32,
    text: String,
    action: String,
    hover: Spring,
}

impl SearchBox {
    pub fn new(spec: &WidgetSpec) -> Self {
        SearchBox { width: spec.n("width"), text: spec.s("text"), action: spec.str("action").unwrap_or("launcher").to_string(), hover: hover_spring() }
    }

    fn rect(&self, r: [f32; 4]) -> [f32; 4] {
        [r[0] + 4.0, r[1] + r[3] / 2.0 - 16.0, self.width, 32.0]
    }
}

impl Widget for SearchBox {
    fn kind(&self) -> &'static str {
        "search-box"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        self.width + 8.0
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        vec![self.width(c)]
    }
    fn update(&mut self, dt: f32, c: &BarCtx, r: [f32; 4]) {
        self.hover.target = if inside(c.mouse, self.rect(r)) { 1.0 } else { 0.0 };
        self.hover.update(dt);
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        let b = self.rect(r);
        let h = self.hover.x;
        let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
        let a = if c.theme.dark { 0.14 + 0.08 * h } else { 0.65 + 0.2 * h };
        let mut s = Panel::solid(cx, cy, b[2], b[3], 16.0, [1.0; 3], [1.0; 3]).flat().alpha(a);
        s.extra = [0.0; 4];
        out.panels.push(s);
        out.panels.push(Panel::glyph(b[0] + 20.0, cy, 15.0, "search", c.ink, 0.7));
        out.labels.push(Label::new(self.text.clone(), b[0] + 38.0, cy + 5.0, 13.0, false, c.ink, 0.7, Align::Left));
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        Some(BarClick::Action(self.action.clone()))
    }
}

// ---------------------------------------------------------------------------------------- tray

/// A row of status icons (and the "more" chevron).
pub struct Tray {
    icons: Vec<String>,
    chevron: bool,
    size: f32,
    /// what a click on a status icon does (the Quick Settings by default)
    action: Option<String>,
}

impl Tray {
    pub fn new(spec: &WidgetSpec) -> Self {
        Tray {
            icons: spec.strings("icons").unwrap_or_else(|| [("wifi", "wifi"), ("volume", "volume-2"), ("battery", "battery-full"), ("bluetooth", "bluetooth")].into_iter().filter(|(k, _)| spec.flag(k)).map(|(_, icon)| icon.to_string()).collect()),
            chevron: spec.flag("chevron"),
            size: spec.n("size"),
            action: Some(spec.s("action")).filter(|a| !a.is_empty() && a != "none"),
        }
    }

    fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        if self.chevron {
            v.push("chevron-up");
        }
        v.extend(self.icons.iter().map(String::as_str));
        v
    }
}

impl Widget for Tray {
    fn kind(&self) -> &'static str {
        "tray"
    }
    fn width(&self, c: &BarCtx) -> f32 {
        (self.names().len() + c.tray.len()) as f32 * 28.0
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        vec![28.0; self.names().len() + c.tray.len()]
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        let names = self.names();
        // the "more" arrow first, then the applications' icons, then the machine's own
        let lead = usize::from(self.chevron);
        for i in 0..names.len() + c.tray.len() {
            let (x, cy) = (r[0] + i as f32 * 28.0 + 14.0, r[1] + r[3] / 2.0);
            if lit == Some(i) {
                out.panels.push(Panel::solid(x, cy, 26.0, 32.0, 5.0, [1.0; 3], [1.0; 3]).flat().alpha(glow(c)));
            }
            if i < lead {
                glyph(c, r, out, "chevron-up", x, self.size);
            } else if i < lead + c.tray.len() {
                let item = &c.tray[i - lead];
                let size = self.size + 3.0;
                // a white icon on a light bar (or a black one on a dark bar) is turned over so that it can be seen
                let bar_is_light = (0.299 * c.ink[0] + 0.587 * c.ink[1] + 0.114 * c.ink[2]) < 0.5;
                let cell = match item.inverse {
                    Some((inv, icon_is_light)) if icon_is_light == bar_is_light => Some(inv),
                    _ => item.cell,
                };
                match cell {
                    Some(cell) => out.panels.push(Panel::app_icon(x, cy, size, cell, 1.0)),
                    None => glyph(c, r, out, "app-window", x, self.size),
                }
                // a program that wants attention gets a dot
                if item.status == crate::tray::TrayStatus::Attention {
                    out.panels.push(Panel::solid(x + size / 2.0 - 2.0, cy - size / 2.0 + 2.0, 7.0, 7.0, 3.5, [1.0, 0.3, 0.3], [1.0, 0.3, 0.3]).flat());
                }
            } else {
                let name = names[i - c.tray.len()];
                glyph(c, r, out, &crate::widget::status_icon(name, c.sys), x, self.size);
            }
        }
    }
    fn click(&mut self, c: &BarCtx, _: [f32; 4], part: usize, _: f32) -> Option<BarClick> {
        let lead = usize::from(self.chevron);
        if part < lead {
            return Some(BarClick::Action("tray-more".into()));
        }
        if part < lead + c.tray.len() {
            return Some(BarClick::Action(format!("tray:{}", c.tray[part - lead].id)));
        }
        // the volume icon opens its own slider; the rest follow the widget's action
        let icon = self.names().get(part - c.tray.len()).copied().unwrap_or("");
        if icon.starts_with("volume") {
            return Some(BarClick::Action("volume".into()));
        }
        self.action.clone().map(BarClick::Action)
    }
    fn cc(&self) -> CcRole {
        if self.action.as_deref() == Some("control-center") { CcRole::Anchor } else { CcRole::None }
    }
}

// ---------------------------------------------------------------------------------------- show desktop

/// Aero's thin strip at the far right.
pub struct ShowDesktop {
    width: f32,
    action: String,
}

impl ShowDesktop {
    pub fn new(spec: &WidgetSpec) -> Self {
        ShowDesktop { width: spec.n("width"), action: spec.str("action").unwrap_or("show-desktop").to_string() }
    }
}

impl Widget for ShowDesktop {
    fn kind(&self) -> &'static str {
        "show-desktop"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        self.width + 4.0
    }
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        vec![self.width(c)]
    }
    fn draw(&self, _: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        let a = if lit.is_some() { 0.55 } else { 0.28 };
        out.panels.push(Panel::solid(r[0] + self.width / 2.0 + 2.0, r[1] + r[3] / 2.0, self.width, r[3] - 4.0, 2.0, [1.0; 3], [0.85, 0.92, 1.0]).flat().alpha(a));
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        Some(BarClick::Action(self.action.clone()))
    }
}

// ---------------------------------------------------------------------------------------- virtual desktops

/// One small button per virtual desktop; the current one is lit.
pub struct Workspaces;

const DESKTOPS: usize = 4;

impl Widget for Workspaces {
    fn kind(&self) -> &'static str {
        "workspaces"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        DESKTOPS as f32 * 26.0 + 4.0
    }
    fn parts(&self, _: &BarCtx) -> Vec<f32> {
        vec![26.0; DESKTOPS]
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], lit: Option<usize>, out: &mut Drawn) {
        for i in 0..DESKTOPS {
            let (cx, cy) = (r[0] + 2.0 + i as f32 * 26.0 + 13.0, r[1] + r[3] / 2.0);
            let now = i == c.workspace;
            if now {
                out.panels.push(Panel::solid(cx, cy, 22.0, 22.0, 6.0, c.theme.accent, c.theme.accent).flat().alpha(0.9));
            } else if lit == Some(i) {
                out.panels.push(Panel::solid(cx, cy, 22.0, 22.0, 6.0, [1.0; 3], [1.0; 3]).flat().alpha(glow(c)));
            }
            let ink = if now { [1.0; 3] } else { c.ink };
            out.labels.push(Label::new((i + 1).to_string(), cx, cy + 4.5, 12.5, now, ink, if now { 1.0 } else { 0.8 }, Align::Center));
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], part: usize, _: f32) -> Option<BarClick> {
        Some(BarClick::Action(format!("workspace:{}", part + 1)))
    }
}
