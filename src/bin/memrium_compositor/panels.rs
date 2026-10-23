//! Editing the panels (bars): the right-click menu, and an edit mode where widgets can be dragged around,
//! removed or added. Everything goes through the theme's `bars`, so it is saved with the rest of the config.

use crate::state::{MenuFor, State};
use memrium_de::menu::MenuItem;
use memrium_de::renderer::{Align, Label, Panel};
use memrium_de::theme::{Edge, MenubarKind};
use memrium_de::widget::{self, BarConfig, BarShape, WidgetSpec};

const P_EDIT: u32 = 7000;
const P_KIND: u32 = 7010; // + 0..4: glass, solid, fade, none
const P_TOP: u32 = 7020;
const P_BOTTOM: u32 = 7021;
const P_STRIP: u32 = 7022;
const P_FLOATING: u32 = 7023;
const P_EDGE_LEFT: u32 = 7024;
const P_EDGE_RIGHT: u32 = 7025;
const P_REMOVE: u32 = 7030;
const P_LEFT: u32 = 7031;
const P_RIGHT: u32 = 7032;
const P_RESET: u32 = 7040;
const P_HIDE: u32 = 7050; // + 0..3: off, at the edge, smart
const P_ADD: u32 = 7100; // + catalog index

const KINDS: [(MenubarKind, &str); 4] = [(MenubarKind::Glass, "Glass"), (MenubarKind::Solid, "Solid"), (MenubarKind::Fade, "Gradient blur"), (MenubarKind::Clear, "None")];

/// A widget being dragged to a new place.
pub struct PanelDrag {
    pub bar: usize,
    pub widget: usize,
    pub start: [f32; 2],
    pub active: bool,
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

impl State {
    fn edit_theme(&mut self, f: impl FnOnce(&mut memrium_de::theme::Theme)) {
        let mut t = self.theme.clone();
        f(&mut t);
        self.apply_theme(t);
    }

    /// Widget rectangles of a bar: (widget index, rect, kind).
    pub(crate) fn panel_widgets(&self, bar: usize) -> Vec<(usize, [f32; 4], &'static str)> {
        self.with_bar_ctx(|ctx| self.bars.get(bar).map(|b| b.widget_rects(ctx)).unwrap_or_default())
    }

    pub(crate) fn panel_widget_at(&self, bar: usize, p: [f32; 2]) -> Option<usize> {
        self.panel_widgets(bar).into_iter().filter(|(_, r, _)| r[2] > 1.0).find(|(_, r, _)| inside(p, *r)).map(|(i, _, _)| i)
    }

    // ------------------------------------------------------------ the context menu

    /// Right click on a panel.
    pub fn open_panel_menu(&mut self, bar: usize) {
        let Some(cfg) = self.theme.bars.get(bar).cloned() else { return };
        let widget = self.panel_widget_at(bar, self.mouse);
        let look = cfg.look.clone().unwrap_or_else(|| self.theme.menubar.clone());
        let mut items = vec![MenuItem::new(P_EDIT, if self.panel_edit == Some(bar) { "Finish Editing" } else { "Edit Panel" }).icon("pencil"), MenuItem::sep()];
        if cfg.shape == BarShape::Strip {
            let sub: Vec<MenuItem> = KINDS.iter().enumerate().map(|(i, (k, name))| MenuItem::new(P_KIND + i as u32, *name).checked(look.kind == *k)).collect();
            items.push(MenuItem::new(0, "Background").icon("layers").sub(sub));
        }
        items.push(
            MenuItem::new(0, "Position")
                .icon("move")
                .sub(vec![MenuItem::new(P_TOP, "Top").checked(cfg.edge == Edge::Top), MenuItem::new(P_BOTTOM, "Bottom").checked(cfg.edge == Edge::Bottom), MenuItem::new(P_EDGE_LEFT, "Left").checked(cfg.edge == Edge::Left), MenuItem::new(P_EDGE_RIGHT, "Right").checked(cfg.edge == Edge::Right)]),
        );
        items.push(
            MenuItem::new(0, "Shape")
                .icon("panel-top")
                .sub(vec![MenuItem::new(P_STRIP, "Full-width strip").checked(cfg.shape == BarShape::Strip), MenuItem::new(P_FLOATING, "Floating").checked(cfg.shape == BarShape::Floating)]),
        );
        let hide: Vec<MenuItem> = [(widget::AutoHide::Off, "Off"), (widget::AutoHide::Always, "At the edge")]
            .iter()
            .enumerate()
            .map(|(i, (m, name))| MenuItem::new(P_HIDE + i as u32, *name).checked(cfg.autohide == *m))
            .collect();
        items.push(MenuItem::new(0, "Auto-hide").icon("eye").sub(hide));
        let add: Vec<MenuItem> = widget::CATALOG.iter().enumerate().map(|(i, (_, name))| MenuItem::new(P_ADD + i as u32, *name)).collect();
        items.push(MenuItem::new(0, "Add Widget").icon("plus").sub(add));
        if let Some(w) = widget {
            let kind = self.panel_widgets(bar).into_iter().find(|(i, _, _)| *i == w).map(|(_, _, k)| k).unwrap_or("widget");
            items.push(MenuItem::sep());
            items.push(MenuItem::new(P_LEFT, "Move Left").icon("arrow-left"));
            items.push(MenuItem::new(P_RIGHT, "Move Right").icon("arrow-right"));
            items.push(MenuItem::new(P_REMOVE, format!("Remove “{kind}”")).icon("trash-2"));
        }
        items.push(MenuItem::sep());
        items.push(MenuItem::new(P_RESET, "Reset Panels").icon("rotate-ccw"));
        self.menu_for = MenuFor::Panel(bar, widget);
        self.menu.open_at(self.mouse, items);
    }

    pub fn panel_menu_command(&mut self, id: u32, bar: usize, widget: Option<usize>) {
        match id {
            P_EDIT => self.panel_edit = if self.panel_edit == Some(bar) { None } else { Some(bar) },
            i if (P_KIND..P_KIND + 4).contains(&i) => {
                let kind = KINDS[(i - P_KIND) as usize].0;
                self.edit_theme(|t| match t.bars.get_mut(bar).and_then(|b| b.look.as_mut()) {
                    Some(look) => look.kind = kind,
                    None => t.menubar.kind = kind,
                });
            }
            P_TOP | P_BOTTOM | P_EDGE_LEFT | P_EDGE_RIGHT => {
                let edge = match id {
                    P_TOP => Edge::Top,
                    P_EDGE_LEFT => Edge::Left,
                    P_EDGE_RIGHT => Edge::Right,
                    _ => Edge::Bottom,
                };
                self.edit_theme(|t| {
                    if let Some(b) = t.bars.get_mut(bar) {
                        b.edge = edge;
                    }
                });
            }
            i if (P_HIDE..P_HIDE + 2).contains(&i) => {
                let mode = [widget::AutoHide::Off, widget::AutoHide::Always][(i - P_HIDE) as usize];
                self.edit_theme(|t| {
                    if let Some(b) = t.bars.get_mut(bar) {
                        b.autohide = mode;
                    }
                });
            }
            P_STRIP | P_FLOATING => {
                let shape = if id == P_STRIP { BarShape::Strip } else { BarShape::Floating };
                self.edit_theme(|t| {
                    if let Some(b) = t.bars.get_mut(bar) {
                        b.shape = shape;
                    }
                });
            }
            i if (P_ADD..P_ADD + widget::CATALOG.len() as u32).contains(&i) => {
                let kind = widget::CATALOG[(i - P_ADD) as usize].0;
                self.add_widget(bar, kind);
            }
            P_REMOVE => {
                if let Some(w) = widget {
                    self.remove_widget(bar, w);
                }
            }
            P_LEFT | P_RIGHT => {
                if let Some(w) = widget {
                    self.nudge_widget(bar, w, if id == P_LEFT { -1 } else { 1 });
                }
            }
            P_RESET => {
                self.panel_edit = None;
                self.edit_theme(|t| {
                    t.bars = widget::default_bars();
                });
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ changing widgets

    pub fn add_widget(&mut self, bar: usize, kind: &str) {
        let spec = match kind {
            "button" => {
                let mut s = WidgetSpec::new("button");
                s.opts.insert("icon".into(), toml::Value::String("camera".into()));
                s.opts.insert("action".into(), toml::Value::String("screenshot".into()));
                s
            }
            "text" => {
                let mut s = WidgetSpec::new("text");
                s.opts.insert("text".into(), toml::Value::String("Memrium".into()));
                s
            }
            k => WidgetSpec::new(k),
        };
        self.edit_theme(|t| {
            if let Some(b) = t.bars.get_mut(bar) {
                let zone = if b.shape == BarShape::Floating { 1 } else { 2 };
                // before the clock when there is one, so new icons sit with the other status icons
                let list = b.zone_mut(zone);
                let at = list.iter().position(|w| w.kind == "clock").unwrap_or(list.len());
                list.insert(at, spec);
            }
        });
    }

    fn remove_widget(&mut self, bar: usize, widget: usize) {
        let Some((z, i)) = self.bars.get(bar).map(|b| b.origin(widget)) else { return };
        self.edit_theme(|t| {
            if let Some(b) = t.bars.get_mut(bar) {
                let list = b.zone_mut(z);
                if i < list.len() {
                    list.remove(i);
                }
            }
        });
    }

    fn nudge_widget(&mut self, bar: usize, widget: usize, d: i32) {
        let Some((z, i)) = self.bars.get(bar).map(|b| b.origin(widget)) else { return };
        self.edit_theme(|t| {
            if let Some(b) = t.bars.get_mut(bar) {
                let list = b.zone_mut(z);
                let j = i as i32 + d;
                if j >= 0 && (j as usize) < list.len() {
                    list.swap(i, j as usize);
                }
            }
        });
    }

    /// Drop a dragged widget at `x`: it goes before or after the widget it landed on (possibly in another zone).
    fn move_widget(&mut self, bar: usize, widget: usize, x: f32) {
        let rects = self.panel_widgets(bar);
        let Some(b) = self.bars.get(bar) else { return };
        let (sz, si) = b.origin(widget);
        // the nearest other widget decides the place
        let Some((target, rect, _)) = rects.into_iter().filter(|(i, r, _)| *i != widget && r[2] > 1.0).min_by(|a, b| {
            let da = (a.1[0] + a.1[2] / 2.0 - x).abs();
            let db = (b.1[0] + b.1[2] / 2.0 - x).abs();
            da.total_cmp(&db)
        }) else {
            return;
        };
        let after = x > rect[0] + rect[2] / 2.0;
        let (tz, ti) = b.origin(target);
        self.edit_theme(|t| {
            let Some(cfg) = t.bars.get_mut(bar) else { return };
            if si >= cfg.zone_mut(sz).len() {
                return;
            }
            let spec = cfg.zone_mut(sz).remove(si);
            let mut at = ti + after as usize;
            if sz == tz && si < ti {
                at -= 1;
            }
            let list = cfg.zone_mut(tz);
            list.insert(at.min(list.len()), spec);
        });
    }

    // ------------------------------------------------------------ edit mode

    fn toolbar(&self, bar: usize) -> Option<([f32; 4], [f32; 4])> {
        let r = self.with_bar_ctx(|ctx| self.bars.get(bar).map(|b| b.rect(ctx)))?;
        let cfg = self.theme.bars.get(bar)?;
        let (cx, cy) = match cfg.edge {
            Edge::Top => (self.size[0] / 2.0, r[1] + r[3] + 30.0),
            Edge::Left => (r[0] + r[2] + 150.0, self.size[1] / 2.0),
            Edge::Right => (r[0] - 150.0, self.size[1] / 2.0),
            Edge::Bottom => (self.size[0] / 2.0, r[1] - 34.0),
        };
        Some(([cx - 118.0, cy - 15.0, 150.0, 30.0], [cx + 40.0, cy - 15.0, 78.0, 30.0]))
    }

    /// Outlines, remove badges and the toolbar of the panel being edited.
    pub fn panel_edit_overlay(&self) -> (Vec<Panel>, Vec<Label>) {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        let Some(bar) = self.panel_edit else { return (p, l) };
        let acc = self.theme.accent;
        let dragging = self.panel_drag.as_ref().filter(|d| d.active);
        for (i, r, kind) in self.panel_widgets(bar) {
            if r[2] < 1.0 {
                continue;
            }
            let lifted = dragging.is_some_and(|d| d.widget == i);
            let (x, y, w, h) = (r[0] + 2.0, r[1] + 4.0, r[2] - 4.0, r[3] - 8.0);
            p.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 8.0, acc, acc).flat().alpha(if lifted { 0.60 } else { 0.30 }));
            // remove badge
            let (bx, by) = (x + w - 1.0, y + 1.0);
            p.push(Panel::solid(bx, by, 17.0, 17.0, 8.5, [0.95, 0.27, 0.25], [0.95, 0.27, 0.25]).shadow(0.25, 4.0, 1.0));
            p.push(Panel::glyph(bx, by, 11.0, "x", [1.0; 3], 1.0));
            if w > 46.0 {
                l.push(Label::new(kind, x + w / 2.0, y + h + 11.0, 10.5, false, [1.0; 3], 0.8, Align::Center));
            }
        }
        if let Some((tr, ok)) = self.toolbar(bar) {
            let bg = [tr[0] - 8.0, tr[1] - 4.0, ok[0] + ok[2] - tr[0] + 16.0, tr[3] + 8.0];
            let mut g = self.theme.glass.panel(bg[0] + bg[2] / 2.0, bg[1] + bg[3] / 2.0, bg[2], bg[3], 19.0).tinted([0.06, 0.08, 0.14, 0.62]);
            g.extra[0] = 0.25;
            p.push(g);
            let hover = |r: [f32; 4]| inside(self.mouse, r);
            p.push(Panel::solid(tr[0] + tr[2] / 2.0, tr[1] + tr[3] / 2.0, tr[2], tr[3], 15.0, [1.0; 3], [1.0; 3]).flat().alpha(if hover(tr) { 0.28 } else { 0.12 }));
            p.push(Panel::solid(ok[0] + ok[2] / 2.0, ok[1] + ok[3] / 2.0, ok[2], ok[3], 15.0, acc, acc).flat().alpha(if hover(ok) { 1.0 } else { 0.85 }));
            p.push(Panel::glyph(tr[0] + 20.0, tr[1] + tr[3] / 2.0, 14.0, "plus", [1.0; 3], 0.95));
            l.push(Label::new("Add widget", tr[0] + 38.0, tr[1] + tr[3] / 2.0 + 4.5, 13.0, true, [1.0; 3], 0.95, Align::Left));
            l.push(Label::new("Done", ok[0] + ok[2] / 2.0, ok[1] + ok[3] / 2.0 + 4.5, 13.0, true, [1.0; 3], 1.0, Align::Center));
        }
        (p, l)
    }

    /// Left press while a panel is being edited. True when it was used.
    pub fn panel_edit_press(&mut self) -> bool {
        let Some(bar) = self.panel_edit else { return false };
        let m = self.mouse;
        if let Some((add, done)) = self.toolbar(bar) {
            if inside(m, done) {
                self.panel_edit = None;
                return true;
            }
            if inside(m, add) {
                let items: Vec<MenuItem> = widget::CATALOG.iter().enumerate().map(|(i, (_, name))| MenuItem::new(P_ADD + i as u32, *name)).collect();
                self.menu_for = MenuFor::Panel(bar, None);
                self.menu.open_at([add[0], add[1] + add[3] + 4.0], items);
                return true;
            }
        }
        for (i, r, _) in self.panel_widgets(bar) {
            if r[2] < 1.0 {
                continue;
            }
            // the remove badge
            let (bx, by) = (r[0] + r[2] - 3.0, r[1] + 5.0);
            if (m[0] - bx).hypot(m[1] - by) < 10.0 {
                self.remove_widget(bar, i);
                return true;
            }
            if inside(m, r) {
                self.panel_drag = Some(PanelDrag { bar, widget: i, start: m, active: false });
                return true;
            }
        }
        // a click anywhere else ends editing
        if self.bar_at(m) != Some(bar) {
            self.panel_edit = None;
            return false;
        }
        true
    }

    pub fn panel_edit_motion(&mut self) {
        let m = self.mouse;
        if let Some(d) = &mut self.panel_drag {
            if !d.active && (m[0] - d.start[0]).abs() > 6.0 {
                d.active = true;
            }
        }
    }

    /// Left button released: finish a drag.
    pub fn panel_edit_release(&mut self) -> bool {
        let Some(d) = self.panel_drag.take() else { return false };
        if d.active {
            self.move_widget(d.bar, d.widget, self.mouse[0]);
        }
        true
    }

    /// Does `cfg` contain a widget of this kind?
    #[allow(dead_code)]
    pub fn bar_has(cfg: &BarConfig, kind: &str) -> bool {
        cfg.left.iter().chain(&cfg.center).chain(&cfg.right).any(|w| w.kind == kind)
    }
}
