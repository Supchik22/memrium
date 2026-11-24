//! What the overview draws (its model and input are in `overview.rs`). All of it is in the window layer, in this order:
//! the dim and the plates under the cards, the windows (`build_items` moves them onto their cards), the cards' names and
//! buttons, the strip of desktops with its miniatures, and over everything what is being carried.
//!
//! The renderer takes only so many draws a frame: when there are too many windows for a live miniature each, the rest
//! are drawn as plain plates in their tiles.

use super::overview::{Entry, OvId, Place, Spot, CLOSE, LABEL_H};
use super::*;
use memrium_de::renderer::{panel_box, MAX_PANELS, MAX_WINDOW_DRAWS};

/// A minimized window's card is this see-through.
const AWAY: f32 = 0.78;
/// The corners of a desktop's tile.
const TILE_R: f32 = 10.0;

/// One window drawn by the overview itself (a miniature, or one off its place).
struct Shot {
    id: OvId,
    frame: [f32; 4],
    at: Place,
    alpha: f32,
}

struct Tile {
    rect: [f32; 4],
    alpha: f32,
    /// the whole desktop's wallpaper in the tile's scale (cut to the tile)
    wall: [f32; 4],
    minis: Vec<Shot>,
    /// over the miniatures: the veil
    over: Vec<Panel>,
}

/// All the overview needs for one frame, worked out before the items borrow the windows.
pub(super) struct OvFrame {
    /// how far it is in (0: not shown)
    pub k: f32,
    /// where the windows' cards are, in their desktops' own coordinates
    pub cards: HashMap<OvId, Place>,
    /// minimized windows: their cards are a little see-through
    pub away: Vec<OvId>,
    /// drawn by the overview, not among the windows
    pub hidden: Vec<OvId>,
    /// on the dim, under the windows
    pub under: Vec<Panel>,
    over: Vec<Panel>,
    plates: Vec<Panel>,
    tiles: Vec<Tile>,
    loose: Vec<Shot>,
    /// the content of the shell windows, for their miniatures
    shells: Vec<(OvId, Vec<Panel>)>,
}

impl OvFrame {
    fn off() -> OvFrame {
        OvFrame { k: 0.0, cards: HashMap::new(), away: Vec::new(), hidden: Vec::new(), under: Vec::new(), over: Vec::new(), plates: Vec::new(), tiles: Vec::new(), loose: Vec::new(), shells: Vec::new() }
    }
}

fn solid(r: [f32; 4], radius: f32, c: [f32; 3], a: f32) -> Panel {
    Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], radius, c, c).flat().alpha(a)
}

fn grown(r: [f32; 4], d: f32) -> [f32; 4] {
    [r[0] - d, r[1] - d, r[2] + 2.0 * d, r[3] + 2.0 * d]
}

fn both(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x, y) = (a[0].max(b[0]), a[1].max(b[1]));
    [x, y, ((a[0] + a[2]).min(b[0] + b[2]) - x).max(0.0), ((a[1] + a[3]).min(b[1] + b[3]) - y).max(0.0)]
}

/// The box round these panels, and no further than `within`.
fn bounds(panels: &[Panel], within: Option<[f32; 4]>) -> Option<[f32; 4]> {
    let b = panels.iter().map(panel_box).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])?;
    Some(match within {
        Some(r) => [b[0].max(r[0]), b[1].max(r[1]), b[2].min(r[0] + r[2]), b[3].min(r[1] + r[3])],
        None => b,
    })
}

fn frames(out: &mut Vec<Item<'_>>, panels: &[Panel]) {
    for part in panels.chunks(MAX_PANELS) {
        if let Some(bbox) = bounds(part, None) {
            out.push(Item::Frame { panels: part.to_vec(), bbox, fade: 1.0 });
        }
    }
}

fn draws(items: &[Item<'_>]) -> usize {
    items
        .iter()
        .map(|i| match i {
            Item::Frame { .. } | Item::Surface { .. } => 1,
            // (a layer that is kept in a texture takes two)
            Item::Layer { .. } => 2,
            Item::Warp(_) | Item::Opaque(_) => 0,
        })
        .sum()
}

impl State {
    /// The overview's part of this frame.
    pub(super) fn ov_frame(&mut self) -> OvFrame {
        if self.ov.x <= 0.001 {
            return OvFrame::off();
        }
        let mut out = OvFrame::off();
        let k = self.ov.x.max(0.0);
        out.k = k;
        let all = self.ov_entries();
        let shown = |e: &Entry, s: &State| desk_dx(e.ws, s.ws_pos.x, s.size[0]).abs() < s.size[0];
        for e in &all {
            if let Some(p) = self.ov_card_on_desk(e.id) {
                out.cards.insert(e.id, p);
            }
            if e.minimized {
                out.away.push(e.id);
            }
            if let OvId::Shell(sw) = e.id {
                out.shells.push((e.id, self.shell_win(sw).last.clone()));
            }
        }
        let fade = ((k - 0.3) / 0.7).clamp(0.0, 1.0);
        let dark = self.theme.dark;
        let accent = self.theme.accent;
        let (bg, ink): ([f32; 3], [f32; 3]) = if dark { ([0.11, 0.11, 0.125], [0.96, 0.97, 1.0]) } else { ([0.97, 0.975, 0.99], [0.08, 0.10, 0.16]) };
        // (over the dimmed desktop the names are light in both looks)
        let name_ink = [0.97, 0.98, 1.0];
        let radius = self.theme.window.radius.min(22.0);

        // ---- the cards
        let mut labels: Vec<Label> = Vec::new();
        let mut over_text: Vec<Label> = Vec::new();
        let closable = self.overview.closable();
        let mut budget = 4u32;
        for e in &all {
            if self.overview.is_loose(e.id) || !shown(e, self) {
                continue;
            }
            let Some(card) = self.ov_card(e.id, e.ws) else { continue };
            let Some(pose) = self.overview.poses.get(&e.id) else { continue };
            let (lift, born, arrived) = ((pose.lift.x / 100.0).clamp(0.0, 1.0), (pose.born.x / 100.0).clamp(0.0, 1.0), pose.m.arrived());
            let r = card.rect([e.frame[2], e.frame[3]]);
            let a = fade * born;
            if lift > 0.01 {
                out.under.push(solid(grown(r, 5.0), radius * card.s + 5.0, accent, 0.92 * a * lift));
            }
            let (title, app) = match e.id {
                OvId::Win(_) => {
                    let Some(w) = self.ov_index(e.id).map(|i| &self.wins[i]) else { continue };
                    (if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() }, self.app_index_for_window(w))
                }
                OvId::Shell(Sw::Settings) => ("Settings".to_string(), self.apps.iter().position(|x| x.id == apps::SETTINGS_ID)),
            };
            if let Some(i) = app {
                self.ensure_icon(i, &mut budget);
            }
            // (a card on its way passes over others: its name waits until it is nearly there)
            let named = a * arrived * arrived;
            let cell = app.and_then(|i| self.renderer.app_icon_get(&self.apps[i].id));
            let lit = self.overview.sel == Some(e.id);
            let (icon, gap) = (20.0, 7.0);
            let lead = icon + gap;
            let room = (r[2] + 16.0).max(120.0) - lead;
            let title = ellipsize(&self.renderer, &title, 13.0, room);
            let tw = self.renderer.text_width(&title, 13.0, lit);
            let mid = r[1] + r[3] + LABEL_H * 0.56;
            let x = r[0] + r[2] / 2.0 - (lead + tw) / 2.0;
            match cell {
                Some(c) => out.over.push(Panel::app_icon(x + icon / 2.0, mid, icon, c, named)),
                None => out.over.push(Panel::glyph(x + icon / 2.0, mid, 16.0, "app-window", name_ink, 0.9 * named)),
            }
            let name = Label::new(title, x + lead, mid + 4.6, 13.0, lit, name_ink, if e.minimized { 0.72 } else { 0.97 } * named, chebrium::draw::Align::Left);
            // (a program's title is not a text of the shell's to translate)
            labels.push(if matches!(e.id, OvId::Win(_)) { name.verbatim() } else { name });
            if e.minimized {
                let at = [r[0] + r[2] - 15.0, r[1] + r[3] - 15.0];
                out.over.push(solid([at[0] - 10.0, at[1] - 10.0, 20.0, 20.0], 10.0, bg, 0.92 * a));
                out.over.push(Panel::glyph(at[0], at[1], 12.0, "minus", ink, 0.9 * a));
            }
            if closable == Some(e.id) {
                let b = Self::ov_close_button(r);
                let hot = self.overview.hover == Some(Spot::Close(e.id));
                let (fill, mark) = if hot { ([0.92, 0.28, 0.28], [1.0; 3]) } else { (bg, ink) };
                out.over.push(solid(b, CLOSE / 2.0, fill, 0.97 * a).shadow(0.30, 8.0, 2.0));
                out.over.push(Panel::glyph(b[0] + CLOSE / 2.0, b[1] + CLOSE / 2.0, 14.0, "x", mark, a));
            }
        }
        let mon = self.ov_monitor();
        let strip = self.ov_strip();
        let (_, first) = strip[0];
        let here = (self.overview.here.x / 100.0).clamp(0.0, 1.0);
        if here > 0.01 {
            let top = first[1] + first[3] + 42.0;
            out.under.insert(0, solid([mon[0] + 24.0, top, mon[2] - 48.0, mon[1] + mon[3] - 30.0 - top], 24.0, accent, 0.16 * here * fade));
        }
        if fade > 0.0 && all.iter().all(|e| e.ws != self.workspace) && self.overview.carried().is_none() {
            let y = first[1] + first[3] + (mon[1] + mon[3] - first[1] - first[3]) * 0.46;
            labels.push(Label::new("No windows on this desktop", mon[0] + mon[2] / 2.0, y, 15.0, false, name_ink, 0.62 * fade, chebrium::draw::Align::Center));
        }

        // ---- the strip
        let k_strip = ((k - 0.1) / 0.6).clamp(0.0, 1.0);
        if k_strip > 0.001 {
            let slide = -(1.0 - k_strip) * 30.0;
            let mut order = self.ov_tiles();
            if let Some(d) = self.overview.tile_drag {
                // the one in the hand lies over the others
                order.sort_by_key(|(n, _)| *n == d.from);
            }
            let hover = match self.overview.hover {
                Some(Spot::Tile(n) | Spot::Mini(_, n)) => Some(n),
                _ => None,
            };
            let off = self.canvas_off();
            let whole = [-off[0], -off[1], self.world[2] - self.world[0], self.world[3] - self.world[1]];
            for (n, r) in order {
                let dragged = self.overview.tile_drag.is_some_and(|d| d.from == n);
                let glow = (self.overview.glow[n].x / 100.0).clamp(0.0, 1.0);
                let r = [r[0], r[1] + slide, r[2], r[3]];
                let cur = (1.0 - (n as f32 - self.ws_pos.x).abs()).clamp(0.0, 1.0);
                let ring = cur.max(glow);
                let grow = 2.5 * ring + 3.0 * glow;
                if ring > 0.01 {
                    out.plates.push(solid(grown(r, grow), TILE_R + grow, accent, 0.96 * ring * k_strip));
                }
                let mut plate = solid(r, TILE_R, bg, 0.80 * k_strip);
                if dragged {
                    plate = plate.shadow(0.4, 22.0, 8.0);
                }
                out.plates.push(plate);
                let sc = r[2] / mon[2].max(1.0);
                let wall = [r[0] + (whole[0] - mon[0]) * sc, r[1] + (whole[1] - mon[1]) * sc, whole[2] * sc, whole[3] * sc];
                let minis = all
                    .iter()
                    .filter(|e| e.ws == n && !e.minimized && !self.overview.is_loose(e.id))
                    .map(|e| Shot { id: e.id, frame: e.frame, at: self.ov_mini(e, r), alpha: k_strip })
                    .collect();
                let mut over = Vec::new();
                // the desktops not shown stand back a little
                let veil = 0.22 * (1.0 - cur) * (1.0 - glow) * if hover == Some(n) { 0.2 } else { 1.0 };
                if veil > 0.005 && !dragged {
                    over.push(solid(r, TILE_R, [0.0; 3], veil * k_strip));
                }
                if glow > 0.01 {
                    over.push(solid(r, TILE_R, accent, 0.26 * glow * k_strip));
                }
                let strong = n == self.workspace || hover == Some(n) || glow > 0.5;
                over_text.push(Label::new(format!("Desktop {}", n + 1), r[0] + r[2] / 2.0, r[1] + r[3] + 19.0, 12.5, strong, name_ink, k_strip * if strong { 1.0 } else { 0.8 }, chebrium::draw::Align::Center));
                out.tiles.push(Tile { rect: r, alpha: k_strip, wall, minis, over });
            }
        }

        // ---- what is off its place
        for l in &self.overview.loose {
            out.hidden.push(l.id);
            if let Some(e) = all.iter().find(|e| e.id == l.id) {
                let (at, f) = (l.m.now(), e.frame);
                let c = [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0];
                // closing, it goes to where the window is
                let at = Place { c: [c[0] + (at.c[0] - c[0]) * k, c[1] + (at.c[1] - c[1]) * k], s: 1.0 + (at.s - 1.0) * k };
                let alpha = if e.ws == self.workspace { 1.0 } else { k.min(1.0) };
                out.loose.push(Shot { id: e.id, frame: f, at, alpha });
            }
        }
        let text = self.renderer.label_panels(&labels);
        out.over.extend(text);
        let text = self.renderer.label_panels(&over_text);
        out.over.extend(text);
        out
    }

    /// A closing window's ghost, drawn on the card it had.
    pub(super) fn ov_ghost(&self, i: usize) -> Option<Option<Xf>> {
        if self.ov.x <= 0.001 {
            return None;
        }
        let w = &self.wins[i];
        let id = OvId::Win(w.id);
        if w.ws != self.workspace || self.overview.is_loose(id) {
            // nothing of it was shown large
            return Some(None);
        }
        let card = self.ov_card_on_desk(id)?;
        let f = Self::frame_rect(w);
        let mut xf = Xf { c: [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0], s: 1.0, t: [0.0; 2], fade: 1.0 };
        xf.ov(card, self.ov.x);
        Some(Some(xf))
    }
}

impl Xf {
    /// Towards an overview card, `k` of the way (`c` is the middle of the window).
    pub(super) fn ov(&mut self, card: Place, k: f32) {
        self.s *= 1.0 + (card.s - 1.0) * k;
        self.t[0] += (card.c[0] - self.c[0]) * k;
        self.t[1] += (card.c[1] - self.c[1]) * k;
    }

    /// A minimized window on its card: out of its Dock icon as the overview comes, back into it as it goes. `m`: how far
    /// it is put away.
    pub(super) fn ov_away(&mut self, dock: [f32; 2], m: f32, k: f32) {
        let (k, m) = (k.clamp(0.0, 1.0), m.clamp(0.0, 1.0));
        let out = m * (1.0 - k);
        self.s *= 1.0 - 0.94 * out;
        self.t[0] += (dock[0] - self.c[0]) * out;
        self.t[1] += (dock[1] - self.c[1]) * out;
        self.fade *= (1.0 - out) * (1.0 - (1.0 - AWAY) * m * k);
    }
}

/// What drawing a window small needs to read.
pub(super) struct Src<'a, 't> {
    pub wins: &'a [Win],
    pub surfaces: &'a HashMap<ObjectId, SurfNode>,
    pub titles: &'t HashMap<usize, Vec<Panel>>,
    pub theme: &'t Theme,
    pub rules: &'t memrium_de::apprules::Rules,
}

impl<'a> Src<'a, '_> {
    /// Window `shot` as it looks, in the scale and place it is given. `tile`: a miniature, cut to that (no shadow or title).
    fn window(&self, f: &OvFrame, shot: &Shot, tile: Option<[f32; 4]>) -> Vec<Item<'a>> {
        let r = shot.frame;
        let c = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        let xf = Xf { c, s: shot.at.s, t: [shot.at.c[0] - c[0], shot.at.c[1] - c[1]], fade: shot.alpha };
        let mut out = Vec::new();
        let frame = |panels: Vec<Panel>, out: &mut Vec<Item<'a>>| {
            let panels: Vec<Panel> = panels
                .into_iter()
                .map(|p| {
                    let p = xf.panel(p);
                    match tile {
                        Some(t) if p.clip[2] > 0.0 => p.cut_to(both([p.clip[0] - p.clip[2], p.clip[1] - p.clip[3], 2.0 * p.clip[2], 2.0 * p.clip[3]], t)),
                        Some(t) => p.cut_to(t),
                        None => p,
                    }
                })
                .collect();
            if let Some(bbox) = bounds(&panels, tile) {
                out.push(Item::Frame { panels, bbox, fade: xf.fade });
            }
        };
        let wid = match shot.id {
            OvId::Win(wid) => wid,
            OvId::Shell(_) => {
                if let Some((_, panels)) = f.shells.iter().find(|s| s.0 == shot.id) {
                    frame(panels.clone(), &mut out);
                }
                return out;
            }
        };
        let Some((i, w)) = self.wins.iter().enumerate().find(|(_, w)| w.id == wid) else { return out };
        let full = w.fs.is_some();
        let maximized = w.restore.is_some();
        let square = full || (maximized && self.theme.window.max_gap < 0.5);
        let rounded = if square { 0.0 } else { self.theme.window.radius };
        if w.deco == Deco::Server && !full {
            let mut panels = decor::frame_panels(&w.chrome_theme(self.theme), r, true, maximized, None);
            if tile.is_none() {
                panels.extend(self.titles.get(&i).into_iter().flatten().copied());
            }
            frame(panels, &mut out);
        } else if tile.is_none() {
            let round = w.deco == Deco::Client && !square;
            let radius = if round { rounded.min(18.0) } else { 0.0 };
            frame(vec![decor::client_shadow(self.theme, r, radius, self.theme.window.shadow, true, round)], &mut out);
        }
        let Some(root) = w.role.surface() else { return out };
        let client = [r[0] + w.ins[0], r[1] + w.ins[1], r[2] - w.ins[0] - w.ins[2], r[3] - w.ins[1] - w.ins[3]];
        let (shape, radius) = match w.deco {
            Deco::Server if decor::glass_frame(self.theme) => (client, 0.0),
            Deco::Server => (r, rounded),
            Deco::Client => (r, rounded.min(18.0)),
            _ => (r, 0.0),
        };
        let mut shape = xf.rect(shape);
        if let Some(t) = tile {
            shape = both(shape, t);
        }
        let tree = surface_tree(&root);
        let has_under = tree.first().is_some_and(|(s, _)| *s != root);
        let origin = [client[0] - w.geo[0], client[1] - w.geo[1]];
        let rule = self.rules.find(&[w.app_id.as_str()]);
        for (s, off) in tree {
            let Some(n) = self.surfaces.get(&s.id()) else { continue };
            let (Some(tex), Some(sz)) = (n.tex.as_deref(), n.logical_size()) else { continue };
            let rect = xf.rect([origin[0] + off[0] as f32, origin[1] + off[1] as f32, sz[0], sz[1]]);
            let at_root = s == root;
            let veil = if at_root { rule.and_then(|r| r.opacity).unwrap_or(1.0) } else { 1.0 };
            let frosted = at_root && rule.and_then(|r| r.frost).unwrap_or((!tex.opaque || veil < 0.999) && !has_under) && !memrium_de::efficiency::enabled();
            out.push(Item::Surface { tex, rect, clip: Some((shape, radius * xf.s)), frosted, alpha: xf.fade, veil });
        }
        out
    }

    /// The overview over the windows.
    pub fn items(&self, f: &OvFrame, wall: Option<&'a SurfaceTex>, out: &mut Vec<Item<'a>>) {
        if f.k <= 0.001 {
            return;
        }
        frames(out, &f.over);
        frames(out, &f.plates);
        let loose: Vec<Vec<Item<'a>>> = f.loose.iter().map(|s| self.window(f, s, None)).collect();
        // (a tile takes three besides its miniatures: wallpaper, plates of those that did not fit, veil)
        let used = draws(out) + loose.iter().map(|l| draws(l)).sum::<usize>() + 3 * f.tiles.len();
        let mut left = MAX_WINDOW_DRAWS.saturating_sub(used + 2);
        let plain: [f32; 3] = if self.theme.dark { [0.30, 0.31, 0.34] } else { [0.86, 0.87, 0.90] };
        for t in &f.tiles {
            let alpha = t.alpha;
            if let Some(tex) = wall {
                out.push(Item::Surface { tex, rect: t.wall, clip: Some((t.rect, TILE_R)), frosted: false, alpha, veil: 1.0 });
            }
            // the front windows first: those are the ones seen
            let mut live: Vec<Vec<Item<'a>>> = Vec::new();
            let mut flat: Vec<Panel> = Vec::new();
            for shot in t.minis.iter().rev() {
                let made = self.window(f, shot, Some(t.rect));
                let n = draws(&made);
                if n > 0 && n <= left {
                    left -= n;
                    live.push(made);
                } else {
                    let r = both(shot.at.rect([shot.frame[2], shot.frame[3]]), t.rect);
                    flat.insert(0, solid(r, 3.0, plain, 0.95 * shot.alpha));
                }
            }
            frames(out, &flat);
            out.extend(live.into_iter().rev().flatten());
            frames(out, &t.over);
        }
        out.extend(loose.into_iter().flatten());
    }
}
