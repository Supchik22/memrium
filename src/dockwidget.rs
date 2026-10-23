//! The Dock as a widget: application icons with magnification, bounce, running dots and a name tooltip.
//! It goes into any bar (`"dock"`), floating or not.

use crate::renderer::{Align, Label, Panel};
use crate::scene::{DockIcon, DockItem};
use chebrium::spring::Spring;
use crate::widget::{BarClick, BarCtx, DockHover, Drawn, Widget, WidgetSpec};

const PAD: f32 = 14.0;
const SEP: f32 = 16.0;

struct Entry {
    item: DockItem,
    scale: Spring,
    bounce: Spring,
}

pub struct DockWidget {
    entries: Vec<Entry>,
    pulse: Spring,
    hot: bool,
    /// icon centres of the last layout, for minimise animations
    centers: Vec<(String, [f32; 2])>,
    /// per-widget overrides of the theme's `[dock]` values
    icon_size: Option<f32>,
    gap: Option<f32>,
    magnify: Option<bool>,
    amount: f32,
    range: f32,
    indicator: Indicator,
    tooltips: bool,
    previews: bool,
    running_only: bool,
    separators: bool,
    bounce: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Indicator {
    Dot,
    Line,
    Glow,
    None,
}

impl DockWidget {
    pub fn new() -> Self {
        DockWidget { entries: Vec::new(), pulse: Spring::new(0.0, 220.0, 0.3), hot: false, centers: Vec::new(), icon_size: None, gap: None, magnify: None, amount: 0.62, range: 105.0, indicator: Indicator::Dot, tooltips: true, previews: true, running_only: false, separators: true, bounce: true }
    }

    pub fn with_spec(spec: &WidgetSpec) -> Self {
        let indicator = match spec.s("indicator").as_str() {
            "line" => Indicator::Line,
            "glow" => Indicator::Glow,
            "none" => Indicator::None,
            _ => Indicator::Dot,
        };
        DockWidget {
            icon_size: spec.num("icon_size"),
            gap: spec.num("gap"),
            magnify: spec.bool("magnify"),
            amount: spec.n("magnify_amount"),
            range: spec.n("magnify_range").max(20.0),
            indicator,
            tooltips: spec.flag("tooltips"),
            previews: spec.flag("previews"),
            running_only: spec.flag("running_only"),
            separators: spec.flag("separators"),
            bounce: spec.flag("bounce"),
            ..Self::new()
        }
    }

    fn gap(&self, c: &BarCtx) -> f32 {
        self.gap.unwrap_or(c.theme.dock.gap)
    }

    fn icon(&self, c: &BarCtx) -> f32 {
        self.icon_size.unwrap_or(c.theme.dock.icon_size)
    }

    /// Left edge of every icon for the given icon sizes (separators add room), and the total width.
    fn positions(&self, c: &BarCtx, sizes: &[f32]) -> (f32, Vec<f32>) {
        let mut lefts = Vec::new();
        let mut x = PAD;
        for (i, s) in sizes.iter().enumerate() {
            if i > 0 {
                x += self.gap(c);
                if self.separators && self.entries[i].item.separator_before {
                    x += SEP;
                }
            }
            lefts.push(x);
            x += s;
        }
        (x + PAD, lefts)
    }

    fn live_sizes(&self, c: &BarCtx) -> Vec<f32> {
        self.entries.iter().map(|e| self.icon(c) * e.scale.x).collect()
    }

    /// (cx, cy, size) of each icon inside the dock rectangle `r`.
    fn icons(&self, c: &BarCtx, r: [f32; 4]) -> Vec<(f32, f32, f32)> {
        let sizes = self.live_sizes(c);
        let (_, lefts) = self.positions(c, &sizes);
        let bottom = r[1] + r[3] - PAD;
        // on a side bar the icons are centred across it
        let across = |s: f32| if c.vertical { r[1] + r[3] / 2.0 } else { bottom - s / 2.0 };
        sizes.iter().zip(&lefts).zip(&self.entries).map(|((s, l), e)| (r[0] + l + s / 2.0, across(*s) - e.bounce.x, *s)).collect()
    }
}

impl Default for DockWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for DockWidget {
    fn kind(&self) -> &'static str {
        "dock"
    }

    fn width(&self, c: &BarCtx) -> f32 {
        if self.entries.is_empty() {
            return 0.0;
        }
        self.positions(c, &self.live_sizes(c)).0
    }

    fn height(&self, c: &BarCtx) -> f32 {
        self.icon(c) + 2.0 * PAD
    }

    fn overflow(&self, c: &BarCtx) -> f32 {
        self.icon(c) * self.amount + 30.0
    }

    /// One part per icon: the icon plus the gap after it (the first also owns the left padding).
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        let sizes = self.live_sizes(c);
        let (total, lefts) = self.positions(c, &sizes);
        let mut out = Vec::new();
        for i in 0..sizes.len() {
            let start = if i == 0 { 0.0 } else { lefts[i] - (lefts[i] - lefts[i - 1] - sizes[i - 1]) / 2.0 };
            let end = if i + 1 == sizes.len() { total } else { lefts[i + 1] - (lefts[i + 1] - lefts[i] - sizes[i]) / 2.0 };
            out.push(end - start);
        }
        out
    }

    fn draw(&self, c: &BarCtx, r: [f32; 4], _lit: Option<usize>, out: &mut Drawn) {
        let icons = self.icons(c, r);
        let dh = r[3];
        let dcy = r[1] + dh / 2.0;
        for (i, ((&(cx, cy, s), e), _)) in icons.iter().zip(&self.entries).zip(0..).enumerate() {
            match &e.item.icon {
                DockIcon::Procedural(id, top, bot) => out.panels.push(Panel::icon(cx, cy, s, *id, *top, *bot).shadow(0.22, 10.0, 4.0)),
                DockIcon::App(cell) => crate::theme::push_app_icon(c.theme, &mut out.panels, cx, cy, s, *cell, (0.20, 10.0, 4.0)),
            }
            if self.separators && e.item.separator_before && i > 0 {
                let sx = cx - s / 2.0 - (self.gap(c) + SEP) / 2.0;
                out.panels.push(Panel::solid(sx, dcy, 1.5, self.icon(c) * 0.7, 0.75, [0.35, 0.42, 0.55], [0.35, 0.42, 0.55]).flat().alpha(0.35));
            }
            if e.item.running {
                // (on a side bar the mark sits on the outer edge)
                let y = if c.vertical { r[1] + 6.0 } else { dcy + dh / 2.0 - 6.0 };
                match self.indicator {
                    Indicator::Dot => out.panels.push(Panel::solid(cx, y, 5.0, 5.0, 2.5, [0.1; 3], [0.1; 3])),
                    Indicator::Line => out.panels.push(Panel::solid(cx, y + 1.0, (s * 0.4).max(10.0), 3.0, 1.5, [0.1; 3], [0.1; 3])),
                    Indicator::Glow => out.panels.push(Panel::solid(cx, y, s * 0.7, 9.0, 4.5, c.theme.accent, c.theme.accent).flat().alpha(0.55)),
                    Indicator::None => {}
                }
            }
        }
        // tooltip with the app name above the hovered icon
        if let Some(i) = icons.iter().position(|&(cx, cy, s)| (c.mouse[0] - cx).abs() < s / 2.0 && (c.mouse[1] - cy).abs() < s / 2.0).filter(|_| self.tooltips && !c.vertical) {
            let (cx, cy, s) = icons[i];
            let name = &self.entries[i].item.name;
            let w = (c.measure)(name, 13.0, true) + 28.0;
            out.panels.push(Panel::glass(cx, cy - s / 2.0 - 24.0, w, 28.0, 14.0).tinted([0.07, 0.10, 0.18, 0.62]).shadow(0.18, 14.0, 5.0));
            out.labels.push(Label::new(name.clone(), cx, cy - s / 2.0 - 19.5, 13.0, true, [1.0, 1.0, 1.0], 0.95, Align::Center));
        }
    }

    fn click(&mut self, c: &BarCtx, r: [f32; 4], part: usize, _x: f32) -> Option<BarClick> {
        let icons = self.icons(c, r);
        let &(cx, cy, s) = icons.get(part)?;
        // only the icon itself counts, not the gap next to it
        if (c.mouse[0] - cx).abs() >= s / 2.0 || (c.mouse[1] - cy).abs() >= s / 2.0 {
            return None;
        }
        if self.bounce {
            self.entries[part].bounce.kick(420.0);
        }
        Some(BarClick::Dock(self.entries[part].item.key.clone()))
    }

    fn update(&mut self, dt: f32, c: &BarCtx, r: [f32; 4]) {
        // magnification: Gaussian falloff around the pointer
        let n = self.entries.len();
        let near = self.magnify.unwrap_or(c.theme.dock.magnify) && n > 0 && c.mouse[0] > r[0] - 30.0 && c.mouse[0] < r[0] + r[2] + 30.0 && c.mouse[1] > r[1] - 30.0 && c.mouse[1] < r[1] + r[3] + 30.0;
        if near && !self.hot {
            self.pulse.kick(5.0); // jelly: the dock stretches when the pointer arrives
        }
        self.hot = near;
        let base = vec![self.icon(c); n];
        let (base_w, lefts) = self.positions(c, &base);
        let dcx = r[0] + r[2] / 2.0;
        for (i, left) in lefts.iter().enumerate() {
            let rest_x = dcx - base_w / 2.0 + left + self.icon(c) / 2.0;
            let d = (c.mouse[0] - rest_x) / self.range;
            self.entries[i].scale.target = if near { 1.0 + self.amount * (-d * d).exp() } else { 1.0 };
            let an = &c.theme.animations;
            an.step("dock", &mut self.entries[i].scale, dt);
            an.step("dock", &mut self.entries[i].bounce, dt);
        }
        c.theme.animations.step("dock", &mut self.pulse, dt);
        let icons = self.icons(c, r);
        self.centers = self.entries.iter().zip(&icons).map(|(e, &(cx, cy, _))| (e.item.key.clone(), [cx, cy])).collect();
    }

    fn set_dock(&mut self, items: &[DockItem], _focused: Option<&str>) {
        let mut old: Vec<Entry> = std::mem::take(&mut self.entries);
        for it in items.iter().filter(|it| !self.running_only || it.running) {
            if let Some(pos) = old.iter().position(|e| e.item.key == it.key) {
                let mut e = old.remove(pos);
                e.item = it.clone();
                self.entries.push(e);
            } else {
                let mut scale = Spring::new(1.0, 320.0, 0.55);
                scale.x = 0.2; // pops in
                self.entries.push(Entry { item: it.clone(), scale, bounce: Spring::new(0.0, 190.0, 0.32) });
            }
        }
    }

    fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        self.centers.iter().find(|(k, _)| k == key).map(|(_, p)| *p)
    }

    fn item_at(&self, c: &BarCtx, r: [f32; 4]) -> Option<DockHover> {
        let icons = self.icons(c, r);
        let i = icons.iter().position(|&(cx, cy, s)| (c.mouse[0] - cx).abs() < s / 2.0 && (c.mouse[1] - cy).abs() < s / 2.0)?;
        let (cx, cy, s) = icons[i];
        Some(DockHover { key: self.entries[i].item.key.clone(), rect: [cx - s / 2.0, cy - s / 2.0, s, s], previews: self.previews })
    }

    fn pulse(&self) -> f32 {
        self.pulse.x
    }
}
