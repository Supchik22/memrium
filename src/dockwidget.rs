//! The Dock as a widget: application icons with magnification, bounce, running marks and a name tooltip.
//! It goes into any bar (`"dock"`), floating or not. The look is all in the options: with a button width (and names) the
//! icons sit on flat taskbar buttons with a plate under the pointer and the focused app, with magnification off and
//! the Windows-style underline as the running mark, it is the taskbar's task buttons; without them it is a macOS Dock.

use crate::renderer::{Align, Label, Panel};
use crate::scene::{DockIcon, DockItem};
use chebrium::spring::Spring;
use crate::widget::{rainbow, tint_icons, tone, BarClick, BarCtx, DockHover, Drawn, Widget, WidgetSpec};

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

const SEP: f32 = 16.0;

struct Entry {
    item: DockItem,
    scale: Spring,
    bounce: Spring,
    /// 1 while the pointer is over the icon (the plate under it fades in)
    hover: Spring,
    /// the icon dips when its button is pressed
    press: Spring,
    /// the app has the focus
    focused: bool,
}

/// What lights up behind an icon.
#[derive(Clone, Copy, PartialEq)]
enum Backdrop {
    None,
    /// a rounded square of colour
    Tint,
    /// a frosted glass slab with a hint of colour
    Glass,
    /// a tall pill from the top of the dock to its bottom
    Pill,
    /// the whole button, flat
    Button,
    /// the whole button as a lit glass key
    Key,
}

impl Backdrop {
    fn parse(s: &str) -> Backdrop {
        match s {
            "tint" => Backdrop::Tint,
            "glass" => Backdrop::Glass,
            "pill" => Backdrop::Pill,
            "button" => Backdrop::Button,
            "key" => Backdrop::Key,
            _ => Backdrop::None,
        }
    }
}

/// Where one entry sits: the centre and size of its icon, and the rectangle that reacts to the pointer.
#[derive(Clone, Copy)]
struct Slot {
    cx: f32,
    cy: f32,
    s: f32,
    rect: [f32; 4],
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
    /// buttons: each icon sits on a button this wide (0: just the icon); `labels` puts the name beside it
    button_w: f32,
    labels: bool,
    label_w: f32,
    /// room at both ends
    pad: f32,
    /// the plate behind the icon of a running app, and behind the one the pointer is over
    running_bg: Backdrop,
    running_color: String,
    running_opacity: f32,
    focus_boost: f32,
    hover_bg: Backdrop,
    hover_color: String,
    hover_opacity: f32,
    /// the mark under a running app: its colour ("auto": dark), size, one per window
    mark_color: String,
    mark_size: f32,
    mark_per_window: bool,
    mark_focus_long: bool,
    /// icons of apps that are not running fade by this much
    dim_idle: f32,
    /// the pointer lifts the icon under it by this many px
    lift: f32,
    badges: bool,
    rainbow: bool,
    rainbow_sat: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Indicator {
    Dot,
    Line,
    /// a short bar under the button, longer for the focused app
    Underline,
    Glow,
    None,
}

impl DockWidget {
    pub fn new() -> Self {
        Self::with_spec(&WidgetSpec::new("dock"))
    }

    pub fn with_spec(spec: &WidgetSpec) -> Self {
        let indicator = match spec.s("indicator").as_str() {
            "line" => Indicator::Line,
            "underline" => Indicator::Underline,
            "glow" => Indicator::Glow,
            "none" => Indicator::None,
            _ => Indicator::Dot,
        };
        DockWidget {
            entries: Vec::new(),
            pulse: Spring::new(0.0, 220.0, 0.3),
            hot: false,
            centers: Vec::new(),
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
            button_w: spec.n("button_w"),
            labels: spec.flag("labels"),
            label_w: spec.n("label_w"),
            pad: spec.n("pad"),
            running_bg: Backdrop::parse(&spec.s("running_bg")),
            running_color: spec.s("running_color"),
            running_opacity: spec.n("running_opacity"),
            focus_boost: spec.n("focus_boost"),
            hover_bg: Backdrop::parse(&spec.s("hover_bg")),
            hover_color: spec.s("hover_color"),
            hover_opacity: spec.n("hover_opacity"),
            mark_color: spec.s("mark_color"),
            mark_size: spec.n("mark_size"),
            mark_per_window: spec.flag("mark_per_window"),
            mark_focus_long: spec.flag("mark_focus_long"),
            dim_idle: spec.n("dim_idle"),
            lift: spec.n("lift"),
            badges: spec.flag("badges"),
            rainbow: spec.flag("rainbow"),
            rainbow_sat: spec.n("rainbow_sat"),
        }
    }

    fn gap(&self, c: &BarCtx) -> f32 {
        self.gap.unwrap_or(c.theme.dock.gap)
    }

    fn icon(&self, c: &BarCtx) -> f32 {
        self.icon_size.unwrap_or(c.theme.dock.icon_size)
    }

    /// Entries sit on buttons (wider than the icon) rather than being bare icons.
    fn buttons(&self) -> bool {
        self.labels || self.button_w > 0.0
    }

    fn magnifies(&self, c: &BarCtx) -> bool {
        self.magnify.unwrap_or(c.theme.dock.magnify)
    }

    /// Width of the cell of an icon of size `s`.
    fn cell_w(&self, s: f32) -> f32 {
        (if self.labels { self.label_w } else { self.button_w }).max(s)
    }

    /// Where the icon is inside its cell, from the cell's left edge.
    fn icon_dx(&self, s: f32) -> f32 {
        if self.labels { 8.0 + s / 2.0 } else { self.cell_w(s) / 2.0 }
    }

    /// Left edge of every cell for the given icon sizes (separators add room), and the total width.
    fn positions(&self, c: &BarCtx, sizes: &[f32]) -> (f32, Vec<f32>) {
        let mut lefts = Vec::new();
        let mut x = self.pad;
        for (i, s) in sizes.iter().enumerate() {
            if i > 0 {
                x += self.gap(c);
                if self.separators && self.entries[i].item.separator_before {
                    x += SEP;
                }
            }
            lefts.push(x);
            x += self.cell_w(*s);
        }
        (x + self.pad, lefts)
    }

    /// A lit plate behind the icon in `slot`.
    fn plate(&self, c: &BarCtx, out: &mut Drawn, kind: Backdrop, color: &str, alpha: f32, slot: &Slot, r: [f32; 4]) {
        let (cx, cy, s) = (slot.cx, slot.cy, slot.s);
        let col = tone(color, c);
        let b = slot.rect;
        let (bx, by) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
        match kind {
            Backdrop::None => {}
            Backdrop::Tint => out.panels.push(Panel::solid(cx, cy, s * 1.16, s * 1.16, s * 0.28, col, col).flat().alpha(alpha)),
            Backdrop::Glass => out.panels.push(Panel::glass(cx, cy, s * 1.16, s * 1.16, s * 0.28).tinted([col[0], col[1], col[2], alpha * 0.6]).shadow(0.10, 8.0, 3.0)),
            Backdrop::Pill => {
                let (w, h) = if c.vertical { (r[2] - 12.0, s * 1.2) } else { (s * 1.2, r[3] - 8.0) };
                let (px, py) = if c.vertical { (r[0] + r[2] / 2.0, cy) } else { (cx, r[1] + r[3] / 2.0) };
                out.panels.push(Panel::solid(px, py, w, h, w.min(h) / 2.0, col, col).flat().alpha(alpha));
            }
            Backdrop::Button => out.panels.push(Panel::solid(bx, by, b[2], b[3], 6.0, col, col).flat().alpha(alpha)),
            Backdrop::Key => {
                out.panels.push(Panel::solid(bx, by, b[2] - 2.0, b[3], 4.0, [1.0; 3], col).flat().alpha(alpha));
                out.panels.push(Panel::solid(bx, by - b[3] * 0.22, b[2] - 4.0, b[3] * 0.5, 3.0, [1.0; 3], [1.0; 3]).flat().alpha(0.18 * alpha.min(1.0)));
            }
        }
    }

    fn live_sizes(&self, c: &BarCtx) -> Vec<f32> {
        self.entries.iter().map(|e| self.icon(c) * e.scale.x).collect()
    }

    /// Where every entry sits inside the dock rectangle `r`.
    fn slots(&self, c: &BarCtx, r: [f32; 4]) -> Vec<Slot> {
        let sizes = self.live_sizes(c);
        let (_, lefts) = self.positions(c, &sizes);
        let icon = self.icon(c);
        // icons rest in the middle of the bar and grow upwards from there
        let bottom = r[1] + r[3] - ((r[3] - icon) / 2.0).max(0.0);
        // on a side bar the icons are centred across it
        let across = |s: f32| if c.vertical { r[1] + r[3] / 2.0 } else { bottom - s / 2.0 };
        let bh = (r[3] - 8.0).min(icon + 18.0).max(8.0);
        sizes
            .iter()
            .zip(&lefts)
            .zip(&self.entries)
            .map(|((&s, &l), e)| {
                let cx = r[0] + l + self.icon_dx(s);
                let cy = across(s) - e.bounce.x;
                let rect = if self.buttons() { [r[0] + l, r[1] + r[3] / 2.0 - bh / 2.0, self.cell_w(s), bh] } else { [cx - s / 2.0, cy - s / 2.0, s, s] };
                Slot { cx, cy, s, rect }
            })
            .collect()
    }
}

/// The red badge with the number of unread notifications, at the top right of an icon.
pub fn push_badge(out: &mut Drawn, count: u32, cx: f32, cy: f32, s: f32, measure: &dyn Fn(&str, f32, bool) -> f32) {
    if count == 0 {
        return;
    }
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let w = (measure(&text, 11.0, true) + 10.0).max(18.0);
    let (bx, by) = (cx + s / 2.0 - w / 2.0 + 3.0, cy - s / 2.0 + 5.0);
    out.panels.push(Panel::solid(bx, by, w, 18.0, 9.0, [1.0, 0.36, 0.33], [0.90, 0.17, 0.17]).shadow(0.30, 6.0, 2.0));
    out.labels.push(Label::new(text, bx, by + 4.0, 11.0, true, [1.0; 3], 1.0, Align::Center));
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
        // (buttons fill the bar they are in)
        if self.buttons() {
            return 0.0;
        }
        self.icon(c) + 2.0 * self.pad
    }

    fn overflow(&self, c: &BarCtx) -> f32 {
        if self.magnifies(c) { self.icon(c) * self.amount + 30.0 } else { 0.0 }
    }

    /// One part per entry: the cell plus the gap after it (the first also owns the left padding).
    fn parts(&self, c: &BarCtx) -> Vec<f32> {
        let sizes = self.live_sizes(c);
        let (total, lefts) = self.positions(c, &sizes);
        let ws: Vec<f32> = sizes.iter().map(|&s| self.cell_w(s)).collect();
        let mut out = Vec::new();
        for i in 0..sizes.len() {
            let start = if i == 0 { 0.0 } else { lefts[i] - (lefts[i] - lefts[i - 1] - ws[i - 1]) / 2.0 };
            let end = if i + 1 == sizes.len() { total } else { lefts[i + 1] - (lefts[i + 1] - lefts[i] - ws[i]) / 2.0 };
            out.push(end - start);
        }
        out
    }

    fn draw(&self, c: &BarCtx, r: [f32; 4], _lit: Option<usize>, out: &mut Drawn) {
        let slots = self.slots(c, r);
        let dh = r[3];
        let dcy = r[1] + dh / 2.0;
        for (i, (slot, e)) in slots.iter().zip(&self.entries).enumerate() {
            let (cx, s) = (slot.cx, slot.s);
            let cy = slot.cy - self.lift * e.hover.x;
            let sz = s + e.press.x;
            // the plates first, so the icon is over them
            if e.item.running && self.running_bg != Backdrop::None {
                let a = (self.running_opacity + if e.focused { self.focus_boost } else { 0.0 }).clamp(0.0, 1.0);
                self.plate(c, out, self.running_bg, &self.running_color, a, slot, r);
            }
            if self.hover_bg != Backdrop::None && e.hover.x > 0.01 {
                self.plate(c, out, self.hover_bg, &self.hover_color, self.hover_opacity * e.hover.x, slot, r);
            }
            let before = out.panels.len();
            match &e.item.icon {
                DockIcon::Procedural(id, top, bot) => out.panels.push(Panel::icon(cx, cy, sz, *id, *top, *bot).shadow(0.22, 10.0, 4.0)),
                DockIcon::App(cell) => crate::theme::push_app_icon(c.theme, &mut out.panels, cx, cy, sz, *cell, (0.20, 10.0, 4.0)),
            }
            if self.rainbow {
                tint_icons(&mut out.panels[before..], rainbow(i, slots.len(), self.rainbow_sat));
            }
            if self.dim_idle > 0.0 && !e.item.running {
                for p in &mut out.panels[before..] {
                    p.tint[3] *= 1.0 - self.dim_idle;
                }
            }
            if self.labels {
                out.labels.push(Label::new(e.item.name.clone(), slot.rect[0] + 8.0 + s + 8.0, cy + 4.5, 12.5, false, c.ink, 0.95, Align::Left));
            }
            if self.separators && e.item.separator_before && i > 0 {
                let sx = slot.rect[0] - (self.gap(c) + SEP) / 2.0;
                out.panels.push(Panel::solid(sx, dcy, 1.5, self.icon(c) * 0.7, 0.75, [0.35, 0.42, 0.55], [0.35, 0.42, 0.55]).flat().alpha(0.35));
            }
            if e.item.running {
                // (on a side bar the mark sits on the outer edge; on a button it is just inside its bottom)
                let y = if c.vertical { r[1] + 6.0 } else if self.buttons() { slot.rect[1] + slot.rect[3] - 4.0 } else { dcy + dh / 2.0 - 6.0 };
                let mc = if self.mark_color.is_empty() || self.mark_color == "auto" { [0.1; 3] } else { tone(&self.mark_color, c) };
                let m = self.mark_size;
                match self.indicator {
                    Indicator::Dot => {
                        // one dot per open window (up to three) when asked, else one; the focused app's is longer when asked
                        let n = if self.mark_per_window { e.item.windows.len().clamp(1, 3) } else { 1 };
                        for k in 0..n {
                            let x = cx + (k as f32 - (n as f32 - 1.0) / 2.0) * (m + 3.0);
                            if self.mark_focus_long && e.focused && n == 1 {
                                out.panels.push(Panel::solid(x, y, m * 3.0, m, m / 2.0, mc, mc));
                            } else {
                                out.panels.push(Panel::solid(x, y, m, m, m / 2.0, mc, mc));
                            }
                        }
                    }
                    Indicator::Line => out.panels.push(Panel::solid(cx, y + 1.0, (s * 0.4).max(10.0) * if self.mark_focus_long && e.focused { 1.5 } else { 1.0 }, (m * 0.6).max(2.0), (m * 0.6).max(2.0) / 2.0, mc, mc)),
                    Indicator::Underline => {
                        // (the default colour: the accent for the focused app, grey for the rest)
                        let col = if self.mark_color.is_empty() || self.mark_color == "auto" { if e.focused { c.theme.accent } else { [0.45, 0.47, 0.52] } } else { mc };
                        let w = if e.focused { 16.0 } else { 6.0 };
                        out.panels.push(Panel::solid(cx, y, w, 3.0, 1.5, col, col).flat());
                    }
                    Indicator::Glow => {
                        let g = if self.mark_color.is_empty() || self.mark_color == "auto" { c.theme.accent } else { mc };
                        out.panels.push(Panel::solid(cx, y, s * 0.7, 9.0, 4.5, g, g).flat().alpha(0.55))
                    }
                    Indicator::None => {}
                }
            }
        }
        if self.badges {
            for (slot, e) in slots.iter().zip(&self.entries) {
                push_badge(out, e.item.badge, slot.cx, slot.cy - self.lift * e.hover.x, slot.s + e.press.x, c.measure);
            }
        }
        // tooltip with the app name above the hovered icon
        if let Some(i) = slots.iter().position(|sl| inside(c.mouse, sl.rect)).filter(|_| self.tooltips && !c.vertical) {
            let sl = slots[i];
            let name = &self.entries[i].item.name;
            let w = (c.measure)(name, 13.0, true) + 28.0;
            out.panels.push(Panel::glass(sl.cx, sl.cy - sl.s / 2.0 - 24.0, w, 28.0, 14.0).tinted([0.07, 0.10, 0.18, 0.62]).shadow(0.18, 14.0, 5.0));
            out.labels.push(Label::new(name.clone(), sl.cx, sl.cy - sl.s / 2.0 - 19.5, 13.0, true, [1.0, 1.0, 1.0], 0.95, Align::Center));
        }
    }

    fn click(&mut self, c: &BarCtx, r: [f32; 4], part: usize, _x: f32) -> Option<BarClick> {
        let slots = self.slots(c, r);
        let slot = slots.get(part)?;
        // only the icon (or its button) itself counts, not the gap next to it
        if !inside(c.mouse, slot.rect) {
            return None;
        }
        let e = &mut self.entries[part];
        if self.bounce {
            e.bounce.kick(420.0);
        } else {
            e.press.kick(-6.0);
        }
        Some(BarClick::Dock(e.item.key.clone()))
    }

    fn update(&mut self, dt: f32, c: &BarCtx, r: [f32; 4]) {
        // magnification: Gaussian falloff around the pointer
        let n = self.entries.len();
        let near = self.magnifies(c) && n > 0 && c.mouse[0] > r[0] - 30.0 && c.mouse[0] < r[0] + r[2] + 30.0 && c.mouse[1] > r[1] - 30.0 && c.mouse[1] < r[1] + r[3] + 30.0;
        if near && !self.hot {
            self.pulse.kick(5.0); // jelly: the dock stretches when the pointer arrives
        }
        self.hot = near;
        let base = vec![self.icon(c); n];
        let (base_w, lefts) = self.positions(c, &base);
        let dcx = r[0] + r[2] / 2.0;
        for (i, left) in lefts.iter().enumerate() {
            let rest_x = dcx - base_w / 2.0 + left + self.icon_dx(self.icon(c));
            let d = (c.mouse[0] - rest_x) / self.range;
            self.entries[i].scale.target = if near { 1.0 + self.amount * (-d * d).exp() } else { 1.0 };
            let an = &c.theme.animations;
            an.step("dock", &mut self.entries[i].scale, dt);
            an.step("dock", &mut self.entries[i].bounce, dt);
        }
        let slots = self.slots(c, r);
        for (e, sl) in self.entries.iter_mut().zip(&slots) {
            let grown = [sl.rect[0] - 2.0, sl.rect[1] - 2.0, sl.rect[2] + 4.0, sl.rect[3] + 4.0];
            e.hover.target = if inside(c.mouse, grown) { 1.0 } else { 0.0 };
            e.hover.update(dt);
            e.press.target = 0.0;
            e.press.update(dt);
        }
        c.theme.animations.step("dock", &mut self.pulse, dt);
        self.centers = self.entries.iter().zip(&slots).map(|(e, sl)| (e.item.key.clone(), [sl.cx, sl.cy])).collect();
    }

    fn set_dock(&mut self, items: &[DockItem], focused: Option<&str>) {
        let mut old: Vec<Entry> = std::mem::take(&mut self.entries);
        for it in items.iter().filter(|it| !self.running_only || it.running) {
            if let Some(pos) = old.iter().position(|e| e.item.key == it.key) {
                let mut e = old.remove(pos);
                e.item = it.clone();
                e.focused = focused == Some(it.key.as_str());
                self.entries.push(e);
            } else {
                let mut scale = Spring::new(1.0, 320.0, 0.55);
                scale.x = 0.2; // pops in
                self.entries.push(Entry { item: it.clone(), scale, bounce: Spring::new(0.0, 190.0, 0.32), hover: Spring::new(0.0, 300.0, 0.8), press: Spring::new(0.0, 400.0, 0.6), focused: focused == Some(it.key.as_str()) });
            }
        }
    }

    fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        self.centers.iter().find(|(k, _)| k == key).map(|(_, p)| *p)
    }

    fn item_at(&self, c: &BarCtx, r: [f32; 4]) -> Option<DockHover> {
        let slots = self.slots(c, r);
        let i = slots.iter().position(|sl| inside(c.mouse, sl.rect))?;
        Some(DockHover { key: self.entries[i].item.key.clone(), rect: slots[i].rect, previews: self.previews })
    }

    fn pulse(&self) -> f32 {
        self.pulse.x
    }
}
