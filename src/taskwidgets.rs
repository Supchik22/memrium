//! The pieces of a Windows-style taskbar as ordinary bar widgets: start button, search box, task buttons, tray and
//! "show desktop". Each one reads its own options from the widget spec, so every piece is configured separately:
//!
//! ```toml
//! [[bars]]
//! name = "taskbar"
//! surface = "glass"
//! center = ["start", { kind = "search-box", width = 190 }, { kind = "tasks", icon_size = 26, indicator = "underline" }]
//! right  = [{ kind = "tray", icons = ["wifi", "volume-2"] }, { kind = "clock", format = "%l:%M %p", format2 = "%m/%d/%Y" }]
//! ```

use crate::renderer::{Align, Label, Panel};
use crate::scene::{DockIcon, DockItem};
use chebrium::spring::Spring;
use crate::widget::{glyph, BarClick, BarCtx, CcRole, DockHover, Drawn, Widget, WidgetSpec};

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

/// The start button: the four-pane logo, or Aero's round orb. Opens the start menu.
pub struct Start {
    orb: bool,
    size: f32,
    action: String,
    hover: Spring,
}

impl Start {
    pub fn new(spec: &WidgetSpec) -> Self {
        let orb = spec.s("style") == "orb";
        Start { orb, size: spec.n("size"), action: spec.str("action").unwrap_or("start").to_string(), hover: hover_spring() }
    }

    fn rect(&self, r: [f32; 4]) -> [f32; 4] {
        let (w, h) = (self.size, if self.orb { self.size } else { 40.0 });
        [r[0] + (r[2] - w) / 2.0, r[1] + (r[3] - h) / 2.0, w, h]
    }
}

impl Widget for Start {
    fn kind(&self) -> &'static str {
        "start"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        self.size + 8.0
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
        let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
        let sh = self.hover.x;
        if self.orb {
            let d = b[2];
            let boost = 0.12 * sh + if c.start_open { 0.15 } else { 0.0 };
            out.panels.push(Panel::solid(cx, cy, d, d, d / 2.0, [0.02, 0.06, 0.16], [0.02, 0.06, 0.16]).flat().alpha(0.55));
            out.panels.push(Panel::solid(cx, cy, d - 3.0, d - 3.0, (d - 3.0) / 2.0, [0.42 + boost, 0.72 + boost, 1.0], [0.06 + boost, 0.28 + boost, 0.66]).flat());
            out.panels.push(Panel::solid(cx, cy - d * 0.20, d * 0.74, d * 0.42, d * 0.21, [1.0; 3], [1.0; 3]).flat().alpha(0.30));
            for (dx, dy, col) in [(-4.5, -4.5, [0.98, 0.35, 0.22]), (4.5, -4.0, [0.45, 0.85, 0.30]), (-4.5, 4.5, [0.25, 0.60, 1.0]), (4.5, 5.0, [1.0, 0.82, 0.20])] {
                out.panels.push(Panel::solid(cx + dx, cy + dy, 8.0, 8.0, 2.0, col, col).flat());
            }
        } else {
            if sh > 0.01 || c.start_open {
                out.panels.push(Panel::solid(cx, cy, b[2], b[3], 6.0, [1.0; 3], [1.0; 3]).flat().alpha(0.45 * sh + if c.start_open { 0.25 } else { 0.0 }));
            }
            for (dx, dy) in [(-5.0, -5.0), (5.0, -5.0), (-5.0, 5.0), (5.0, 5.0)] {
                out.panels.push(Panel::solid(cx + dx, cy + dy, 9.0, 9.0, 1.6, c.theme.accent, c.theme.accent).flat());
            }
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], _: usize, _: f32) -> Option<BarClick> {
        Some(BarClick::Action(self.action.clone()))
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

// ---------------------------------------------------------------------------------------- tasks

#[derive(Clone, Copy, PartialEq)]
enum Indicator {
    Underline,
    Dot,
    Glass,
}

struct Entry {
    item: DockItem,
    focused: bool,
    hover: Spring,
    press: Spring,
}

/// One button per Dock / running app, with a running indicator.
pub struct Tasks {
    icon_size: f32,
    button_w: f32,
    labels: bool,
    previews: bool,
    label_w: f32,
    gap: f32,
    indicator: Indicator,
    entries: Vec<Entry>,
    centers: Vec<(String, [f32; 2])>,
}

impl Tasks {
    pub fn new(spec: &WidgetSpec) -> Self {
        let indicator = match spec.s("indicator").as_str() {
            "dot" => Indicator::Dot,
            "glass" | "glass_button" => Indicator::Glass,
            _ => Indicator::Underline,
        };
        Tasks {
            icon_size: spec.n("icon_size"),
            button_w: spec.n("button_w"),
            labels: spec.flag("labels"),
            previews: spec.flag("previews"),
            label_w: spec.n("label_w"),
            gap: spec.n("gap"),
            indicator,
            entries: Vec::new(),
            centers: Vec::new(),
        }
    }

    fn bw(&self) -> f32 {
        if self.labels { self.label_w } else { self.button_w }
    }

    /// Rectangle of every button inside the widget rectangle `r`.
    fn buttons(&self, r: [f32; 4]) -> Vec<[f32; 4]> {
        let h = (r[3] - 8.0).min(self.icon_size + 18.0).max(8.0);
        let cy = r[1] + r[3] / 2.0;
        (0..self.entries.len()).map(|i| [r[0] + self.gap + i as f32 * (self.bw() + self.gap), cy - h / 2.0, self.bw(), h]).collect()
    }
}

impl Widget for Tasks {
    fn kind(&self) -> &'static str {
        "tasks"
    }
    fn width(&self, _: &BarCtx) -> f32 {
        let n = self.entries.len() as f32;
        if n == 0.0 { 0.0 } else { n * self.bw() + (n + 1.0) * self.gap }
    }
    fn parts(&self, _: &BarCtx) -> Vec<f32> {
        (0..self.entries.len()).map(|i| self.bw() + self.gap + if i == 0 { self.gap } else { 0.0 }).collect()
    }
    fn update(&mut self, dt: f32, c: &BarCtx, r: [f32; 4]) {
        let rects = self.buttons(r);
        for (e, b) in self.entries.iter_mut().zip(&rects) {
            e.hover.target = if inside(c.mouse, *b) { 1.0 } else { 0.0 };
            e.hover.update(dt);
            e.press.target = 0.0;
            e.press.update(dt);
        }
        self.centers = self.entries.iter().zip(&rects).map(|(e, b)| (e.item.key.clone(), [b[0] + b[2] / 2.0, b[1] + b[3] / 2.0])).collect();
    }
    fn draw(&self, c: &BarCtx, r: [f32; 4], _: Option<usize>, out: &mut Drawn) {
        let dark = c.theme.dark;
        for (e, b) in self.entries.iter().zip(self.buttons(r)) {
            let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
            let hov = e.hover.x;
            match self.indicator {
                Indicator::Glass => {
                    if e.item.running || hov > 0.02 {
                        let strength = (if e.item.running { 0.34 + if e.focused { 0.22 } else { 0.0 } } else { 0.0 } + 0.30 * hov).min(0.9);
                        out.panels.push(Panel::solid(cx, cy, b[2] - 2.0, b[3], 4.0, [1.0; 3], [0.72, 0.84, 1.0]).flat().alpha(strength));
                        out.panels.push(Panel::solid(cx, cy - b[3] * 0.22, b[2] - 4.0, b[3] * 0.5, 3.0, [1.0; 3], [1.0; 3]).flat().alpha(0.18 * strength.min(1.0)));
                    }
                }
                Indicator::Underline | Indicator::Dot => {
                    if hov > 0.02 || e.focused {
                        let a = glow(c) * (hov * 0.8 + if e.focused { 0.45 } else { 0.0 }).min(1.0) * if dark { 1.0 } else { 1.1 };
                        out.panels.push(Panel::solid(cx, cy, b[2], b[3], 6.0, [1.0; 3], [1.0; 3]).flat().alpha(a));
                    }
                    if e.item.running {
                        let acc = if e.focused { c.theme.accent } else { [0.45, 0.47, 0.52] };
                        let (iw, ih) = if self.indicator == Indicator::Underline { (if e.focused { 16.0 } else { 6.0 }, 3.0) } else { (5.0, 5.0) };
                        out.panels.push(Panel::solid(cx, b[1] + b[3] - 4.0, iw, ih, ih / 2.0, acc, acc).flat());
                    }
                }
            }
            let sz = self.icon_size * (1.0 + 0.06 * hov) + e.press.x;
            let ix = if self.labels { b[0] + 8.0 + sz / 2.0 } else { cx };
            match &e.item.icon {
                DockIcon::Procedural(id, top, bot) => out.panels.push(Panel::icon(ix, cy - 1.0, sz, *id, *top, *bot).shadow(0.18, 6.0, 2.0)),
                DockIcon::App(cell) => crate::theme::push_app_icon(c.theme, &mut out.panels, ix, cy - 1.0, sz, *cell, (0.16, 6.0, 2.0)),
            }
            if self.labels {
                out.labels.push(Label::new(e.item.name.clone(), b[0] + 8.0 + self.icon_size + 8.0, cy + 4.5, 12.5, false, c.ink, 0.95, Align::Left));
            }
        }
    }
    fn click(&mut self, _: &BarCtx, _: [f32; 4], part: usize, _: f32) -> Option<BarClick> {
        let e = self.entries.get_mut(part)?;
        e.press.kick(-6.0);
        Some(BarClick::Dock(e.item.key.clone()))
    }
    fn set_dock(&mut self, items: &[DockItem], focused: Option<&str>) {
        let mut old: Vec<Entry> = std::mem::take(&mut self.entries);
        for it in items {
            let is_focus = focused == Some(it.key.as_str());
            if let Some(pos) = old.iter().position(|e| e.item.key == it.key) {
                let mut e = old.remove(pos);
                e.item = it.clone();
                e.focused = is_focus;
                self.entries.push(e);
            } else {
                self.entries.push(Entry { item: it.clone(), focused: is_focus, hover: hover_spring(), press: Spring::new(0.0, 400.0, 0.6) });
            }
        }
    }
    fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        self.centers.iter().find(|(k, _)| k == key).map(|(_, p)| *p)
    }
    fn item_at(&self, c: &BarCtx, r: [f32; 4]) -> Option<DockHover> {
        self.entries.iter().zip(self.buttons(r)).find(|(_, b)| inside(c.mouse, *b)).map(|(e, b)| DockHover { key: e.item.key.clone(), rect: b, previews: self.previews })
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
                match item.cell {
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
