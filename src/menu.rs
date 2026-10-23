//! Context menus drawn by the shell: glass panel, icons, shortcuts, separators and submenus.
//! The caller opens a menu with a list of items and gets the clicked item's `id` back.

use crate::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use crate::theme::Theme;

const ROW: f32 = 30.0;
const SEP: f32 = 11.0;
const PAD: f32 = 6.0;
const FONT: f32 = 13.5;

#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    /// returned when clicked; 0 for separators and submenu parents
    pub id: u32,
    pub label: String,
    pub icon: Option<&'static str>,
    pub shortcut: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub sub: Vec<MenuItem>,
    /// where a program's icon for the entry comes from, until the shell has made a picture of it (see `cell`)
    pub icon_src: Option<IconSrc>,
    /// the entry's picture in the app-icon atlas
    pub cell: Option<u32>,
}

/// An icon a program asked for: a name in the icon theme, or the PNG it sent.
#[derive(Clone, Debug, PartialEq)]
pub enum IconSrc {
    Name(String),
    Png(Vec<u8>),
}

impl MenuItem {
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        MenuItem { id, label: label.into(), enabled: true, ..Default::default() }
    }

    pub fn sep() -> Self {
        MenuItem { separator: true, enabled: false, ..Default::default() }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn key(mut self, s: &str) -> Self {
        self.shortcut = s.into();
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn sub(mut self, items: Vec<MenuItem>) -> Self {
        self.sub = items;
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuClick {
    Item(u32),
    /// on the menu but not on an actionable row
    Inside,
    Outside,
}

struct Level {
    rect: [f32; 4],
    rows: Vec<[f32; 4]>,
}

pub struct Menu {
    pub open: bool,
    items: Vec<MenuItem>,
    at: [f32; 2],
    /// indices of the submenu parents that are currently open (level n+1 shows `path[n]`'s children)
    path: Vec<usize>,
    /// keyboard / hover highlight: (level, row)
    hot: Option<(usize, usize)>,
    mouse: [f32; 2],
    levels: Vec<Level>,
    screen: [f32; 2],
    /// pop-in
    open_k: Spring,
    /// the sliding highlight: x, y, w, h and opacity
    hl: [Spring; 5],
    hl_init: bool,
    /// pop-in of each open submenu (level 0 has `open_k`)
    sub_k: Vec<Spring>,
    /// where each submenu's background is (centre and size): it glides when another row's submenu takes the place
    sub_rect: Vec<[Spring; 4]>,
    /// the content of the previous frame per level, and a crossfade from the content a submenu had before
    snap: Vec<(Vec<Panel>, Vec<Label>)>,
    xfade: Vec<(Option<(Vec<Panel>, Vec<Label>)>, Spring)>,
    last_path: Vec<usize>,
    /// since when the pointer has been on its way to a submenu (the "safe triangle")
    guard: Option<std::time::Instant>,
}

impl Default for Menu {
    fn default() -> Self {
        let sp = |v: f32| Spring::new(v, 2200.0, 1.0);
        Menu {
            open: false,
            items: Vec::new(),
            at: [0.0; 2],
            path: Vec::new(),
            hot: None,
            mouse: [0.0; 2],
            levels: Vec::new(),
            screen: [0.0; 2],
            open_k: Spring::new(0.0, 1100.0, 0.85),
            hl: [sp(0.0), sp(0.0), sp(0.0), sp(0.0), Spring::new(0.0, 2400.0, 1.0)],
            hl_init: false,
            sub_k: Vec::new(),
            sub_rect: Vec::new(),
            snap: Vec::new(),
            xfade: Vec::new(),
            last_path: Vec::new(),
            guard: None,
        }
    }
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

impl Menu {
    pub fn open_at(&mut self, at: [f32; 2], items: Vec<MenuItem>) {
        self.items = items;
        self.at = at;
        self.path.clear();
        self.hot = None;
        self.open = true;
        self.levels.clear();
        self.open_k = Spring::new(0.0, 1100.0, 0.85);
        self.open_k.target = 1.0;
        self.hl_init = false;
        self.sub_k.clear();
    }

    pub fn update(&mut self, dt: f32, an: &crate::theme::AnimStyle) {
        if !self.open {
            return;
        }
        an.step("menu_open", &mut self.open_k, dt);
        for k in &mut self.sub_k {
            an.step("submenu", k, dt);
        }
        for r in &mut self.sub_rect {
            r.iter_mut().for_each(|s| an.step("submenu", s, dt));
        }
        for (old, k) in &mut self.xfade {
            an.step("submenu", k, dt);
            if k.x > 0.995 {
                *old = None;
            }
        }
        for h in &mut self.hl {
            an.step("menu_highlight", h, dt);
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.levels.clear();
        self.sub_k.clear();
        self.sub_rect.clear();
        self.xfade.clear();
        self.snap.clear();
        self.last_path.clear();
    }

    pub fn set_mouse(&mut self, p: [f32; 2]) {
        let prev = self.mouse;
        self.mouse = p;
        if !self.open {
            return;
        }
        // moving diagonally towards an open submenu must not close it when the pointer crosses the rows in between
        if self.heading_for_submenu(prev, p) {
            // the guard lasts a moment: stopping on another row must still select it
            let since = *self.guard.get_or_insert_with(std::time::Instant::now);
            if since.elapsed().as_millis() < 400 {
                return;
            }
        } else {
            self.guard = None;
        }
        // hovering a row: highlight it, and open / close submenus accordingly
        let mut found = None;
        'outer: for (lv, level) in self.levels.iter().enumerate() {
            for (ri, r) in level.rows.iter().enumerate() {
                if inside(p, *r) {
                    found = Some((lv, ri));
                    break 'outer;
                }
            }
        }
        if let Some((lv, ri)) = found {
            let Some(item) = self.items_at(lv).get(ri) else { return };
            if item.separator {
                return;
            }
            let has_sub = !item.sub.is_empty() && item.enabled;
            self.hot = Some((lv, ri));
            self.path.truncate(lv);
            if has_sub {
                self.path.push(ri);
            }
        }
    }

    /// The "safe triangle": the pointer left the row that opened the deepest submenu, but it is still inside the
    /// triangle between where it was and the submenu's near edge, so it is on its way there.
    fn heading_for_submenu(&self, prev: [f32; 2], p: [f32; 2]) -> bool {
        let depth = self.path.len();
        if depth == 0 || self.levels.len() <= depth {
            return false;
        }
        let rows = &self.levels[depth].rows;
        let (Some(first), Some(parent)) = (rows.first(), self.levels[depth - 1].rows.get(self.path[depth - 1])) else { return false };
        let last = rows.last().unwrap_or(first);
        let (top, bottom) = (first[1], last[1] + last[3]);
        let right_of = first[0] >= parent[0];
        let edge = if right_of { first[0] } else { first[0] + first[2] };
        // already in the submenu, or still on the row that opened it: nothing to guard
        if inside(p, [first[0], top, first[2], bottom - top]) || inside(p, *parent) || !inside(prev, [parent[0], parent[1] - 40.0, parent[2] + 80.0, parent[3] + 80.0]) {
            return false;
        }
        let side = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (a[0] - c[0]) * (b[1] - c[1]) - (b[0] - c[0]) * (a[1] - c[1]);
        let (a, b, c) = (prev, [edge, top], [edge, bottom]);
        let (d1, d2, d3) = (side(p, a, b), side(p, b, c), side(p, c, a));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    }

    /// Items shown at `level` (0 = root).
    fn items_at(&self, level: usize) -> &[MenuItem] {
        let mut cur = &self.items;
        for &i in self.path.iter().take(level) {
            match cur.get(i) {
                Some(it) => cur = &it.sub,
                None => return &[],
            }
        }
        cur
    }

    fn layout(&mut self, theme: &Theme, screen: [f32; 2], measure: &dyn Fn(&str, f32, bool) -> f32) {
        self.screen = screen;
        let _ = theme;
        self.levels.clear();
        let mut x = self.at[0];
        let mut y = self.at[1];
        for lv in 0..=self.path.len() {
            let items = self.items_at(lv);
            if items.is_empty() {
                break;
            }
            let mut w = 190.0f32;
            for it in items {
                let mut need = 44.0 + measure(&it.label, FONT, false) + 28.0;
                if !it.shortcut.is_empty() {
                    need += 28.0 + measure(&crate::keys::display(&it.shortcut), FONT - 1.0, false);
                }
                w = w.max(need);
            }
            let h: f32 = items.iter().map(|i| if i.separator { SEP } else { ROW }).sum::<f32>() + PAD * 2.0;
            // keep on screen: flip a submenu to the left when it would overflow, shift up when too low
            let mut px = x;
            if px + w > screen[0] - 6.0 {
                px = if lv == 0 { (screen[0] - 6.0 - w).max(6.0) } else { (self.levels[lv - 1].rect[0] - w + 4.0).max(6.0) };
            }
            let py = y.min(screen[1] - 6.0 - h).max(6.0);
            let mut rows = Vec::new();
            let mut ry = py + PAD;
            for it in items {
                let rh = if it.separator { SEP } else { ROW };
                rows.push([px + PAD, ry, w - PAD * 2.0, rh]);
                ry += rh;
            }
            self.levels.push(Level { rect: [px, py, w, h], rows });
            // the next level opens beside the parent row
            if let Some(&pi) = self.path.get(lv) {
                if let Some(r) = self.levels[lv].rows.get(pi) {
                    x = px + w - 4.0;
                    y = r[1] - PAD;
                }
            }
        }
    }

    pub fn click(&self, p: [f32; 2]) -> MenuClick {
        for (lv, level) in self.levels.iter().enumerate() {
            if !inside(p, level.rect) {
                continue;
            }
            for (ri, r) in level.rows.iter().enumerate() {
                if inside(p, *r) {
                    let Some(it) = self.items_at(lv).get(ri) else { return MenuClick::Inside };
                    if it.enabled && !it.separator && it.sub.is_empty() {
                        return MenuClick::Item(it.id);
                    }
                    return MenuClick::Inside;
                }
            }
            return MenuClick::Inside;
        }
        MenuClick::Outside
    }

    /// Keyboard: Up/Down move, Right opens a submenu, Left closes one. Returns the id on Enter.
    pub fn key_nav(&mut self, dir: NavKey) -> Option<u32> {
        let lv = self.path.len().min(self.levels.len().saturating_sub(1));
        let lv = self.hot.map(|(l, _)| l).unwrap_or(lv);
        let n = self.items_at(lv).len();
        if n == 0 {
            return None;
        }
        let cur = self.hot.filter(|(l, _)| *l == lv).map(|(_, r)| r);
        let step = |from: Option<usize>, d: i32, items: &[MenuItem]| -> usize {
            let mut i = from.map(|v| v as i32).unwrap_or(if d > 0 { -1 } else { n as i32 });
            for _ in 0..n {
                i = (i + d).rem_euclid(n as i32);
                if !items[i as usize].separator && items[i as usize].enabled {
                    return i as usize;
                }
            }
            from.unwrap_or(0)
        };
        match dir {
            NavKey::Down => {
                let r = step(cur, 1, self.items_at(lv));
                self.hot = Some((lv, r));
                self.path.truncate(lv);
            }
            NavKey::Up => {
                let r = step(cur, -1, self.items_at(lv));
                self.hot = Some((lv, r));
                self.path.truncate(lv);
            }
            NavKey::Right => {
                if let Some(r) = cur {
                    if !self.items_at(lv)[r].sub.is_empty() {
                        self.path.truncate(lv);
                        self.path.push(r);
                        let first = self.items_at(lv + 1).iter().position(|i| !i.separator && i.enabled).unwrap_or(0);
                        self.hot = Some((lv + 1, first));
                    }
                }
            }
            NavKey::Left => {
                if lv > 0 {
                    let parent = self.path[lv - 1];
                    self.path.truncate(lv - 1);
                    self.hot = Some((lv - 1, parent));
                }
            }
            NavKey::Enter => {
                if let Some(r) = cur {
                    let it = &self.items_at(lv)[r];
                    if it.sub.is_empty() && it.enabled {
                        return Some(it.id);
                    }
                    return self.key_nav(NavKey::Right);
                }
            }
        }
        None
    }

    pub fn draw(&mut self, theme: &Theme, screen: [f32; 2], measure: &dyn Fn(&str, f32, bool) -> f32) -> (Vec<Panel>, Vec<Label>) {
        let mut p = Vec::new();
        let mut l = Vec::new();
        if !self.open {
            return (p, l);
        }
        self.layout(theme, screen, measure);
        let dark = theme.dark;
        let ink = if dark { [0.94, 0.95, 0.98] } else { [0.10, 0.12, 0.17] };
        let accent = theme.ui.highlight.unwrap_or(theme.accent);
        // the highlighted row decides where the sliding highlight goes
        let mut target = None;
        for lv in 0..self.levels.len() {
            let items = self.items_at(lv);
            for (ri, it) in items.iter().enumerate() {
                let r = self.levels[lv].rows[ri];
                let on_path = self.path.get(lv) == Some(&ri);
                if !it.separator && it.enabled && (self.hot == Some((lv, ri)) || on_path || (self.hot.is_none() && inside(self.mouse, r))) {
                    target = Some(r);
                }
            }
        }
        match target {
            Some(r) => {
                let vals = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3]];
                for (h, v) in self.hl.iter_mut().zip(vals) {
                    h.target = v;
                    if !self.hl_init {
                        h.x = v;
                        h.v = 0.0;
                    }
                }
                self.hl[4].target = 1.0;
                if !self.hl_init {
                    self.hl[4].x = 0.0;
                }
                self.hl_init = true;
            }
            None => self.hl[4].target = 0.0,
        }
        // a submenu that just opened starts at 0 and springs to 1
        while self.sub_k.len() < self.levels.len() {
            let mut k = Spring::new(0.0, 1500.0, 0.8);
            k.target = 1.0;
            self.sub_k.push(k);
        }
        self.sub_k.truncate(self.levels.len());
        // a submenu's background follows its level's rectangle with a spring, so that one submenu flows into another
        while self.sub_rect.len() < self.levels.len() {
            let r = self.levels[self.sub_rect.len()].rect;
            let sp = |v: f32| Spring::new(v, 900.0, 0.9);
            self.sub_rect.push([sp(r[0] + r[2] / 2.0), sp(r[1] + r[3] / 2.0), sp(r[2]), sp(r[3])]);
            self.xfade.push((None, Spring::new(1.0, 700.0, 1.0)));
        }
        self.sub_rect.truncate(self.levels.len());
        self.xfade.truncate(self.levels.len());
        for lv in 1..self.levels.len() {
            let r = self.levels[lv].rect;
            let t = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3]];
            if self.last_path.get(lv - 1).is_some_and(|&o| o != self.path[lv - 1]) {
                if let Some(old) = self.snap.get(lv).cloned() {
                    self.xfade[lv].0 = Some(old);
                    self.xfade[lv].1.x = 0.0;
                    self.xfade[lv].1.v = 0.0;
                }
            }
            self.xfade[lv].1.target = 1.0;
            for (s, v) in self.sub_rect[lv].iter_mut().zip(t) {
                s.target = v;
            }
        }
        let mut ranges: Vec<(usize, usize, usize, usize)> = vec![(0, 0, 0, 0); self.levels.len()];
        for lv in 0..self.levels.len() {
            let mut rect = self.levels[lv].rect;
            if lv >= 1 {
                let g = &self.sub_rect[lv];
                rect = [g[0].x - g[2].x / 2.0, g[1].x - g[3].x / 2.0, g[2].x, g[3].x];
            }
            ranges[lv].0 = p.len();
            let mut bg = theme.glass.panel(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, rect[2], rect[3], (theme.window.radius * 0.7).clamp(6.0, 14.0));
            bg.tint = if dark { [0.11, 0.11, 0.11, 0.78] } else { [0.97, 0.98, 1.0, 0.80] };
            bg.fx[0] = bg.fx[0].max(16.0);
            bg.extra[0] = bg.extra[0].max(0.28);
            p.push(bg);
            ranges[lv].1 = p.len();
        }
        let a = self.hl[4].x.clamp(0.0, 1.0);
        if a > 0.01 {
            p.push(Panel::solid(self.hl[0].x, self.hl[1].x, self.hl[2].x, self.hl[3].x, 7.0, accent, accent).flat().alpha(0.92 * a));
        }
        let mut item_ranges = vec![(0usize, 0usize, 0usize, 0usize); self.levels.len()];
        for lv in 0..self.levels.len() {
            let items = self.items_at(lv).to_vec();
            item_ranges[lv] = (p.len(), 0, l.len(), 0);
            for (ri, it) in items.iter().enumerate() {
                let r = self.levels[lv].rows[ri];
                if it.separator {
                    p.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] - 12.0, 1.0, 0.5, ink, ink).flat().alpha(0.14));
                    continue;
                }
                let on_path = self.path.get(lv) == Some(&ri);
                let _ = on_path;
                // ink follows the sliding highlight: white where it covers the row
                let (hx, hy) = (self.hl[0].x, self.hl[1].x);
                let near = (1.0 - ((hy - (r[1] + r[3] / 2.0)).abs() / r[3]).min(1.0)) * if (hx - (r[0] + r[2] / 2.0)).abs() < r[2] * 0.5 { 1.0 } else { 0.0 };
                let cover = near * near * (3.0 - 2.0 * near) * self.hl[4].x.clamp(0.0, 1.0);
                let fg = [ink[0] + (1.0 - ink[0]) * cover, ink[1] + (1.0 - ink[1]) * cover, ink[2] + (1.0 - ink[2]) * cover];
                let al = if it.enabled { 1.0 } else { 0.38 };
                let cy = r[1] + r[3] / 2.0;
                if let Some(cell) = it.cell {
                    p.push(Panel::app_icon(r[0] + 18.0, cy, 17.0, cell, al));
                } else if let Some(ic) = it.icon {
                    p.push(Panel::glyph(r[0] + 18.0, cy, 16.0, ic, fg, al * 0.92));
                } else if it.checked {
                    p.push(Panel::glyph(r[0] + 18.0, cy, 15.0, "check", fg, al));
                }
                l.push(Label::new(it.label.clone(), r[0] + 36.0, cy + 4.8, FONT, false, fg, al, Align::Left));
                if !it.shortcut.is_empty() {
                    l.push(Label::new(crate::keys::display(&it.shortcut), r[0] + r[2] - 12.0, cy + 4.5, FONT - 1.0, false, fg, al * 0.62, Align::Right));
                }
                if (it.icon.is_some() || it.cell.is_some()) && it.checked && it.sub.is_empty() {
                    // an icon takes the check's place on the left, so the choice is ticked on the right
                    p.push(Panel::glyph(r[0] + r[2] - 16.0, cy, 14.0, "check", fg, al));
                }
                if !it.sub.is_empty() {
                    p.push(Panel::glyph(r[0] + r[2] - 14.0, cy, 14.0, "chevron-right", fg, al * 0.9));
                }
            }
            item_ranges[lv].1 = p.len();
            item_ranges[lv].3 = l.len();
        }
        // one submenu turning into another: the old content fades out as the new one fades in
        let new_snap: Vec<(Vec<Panel>, Vec<Label>)> = (0..self.levels.len()).map(|lv| (p[item_ranges[lv].0..item_ranges[lv].1].to_vec(), l[item_ranges[lv].2..item_ranges[lv].3].to_vec())).collect();
        for lv in 1..self.levels.len() {
            let k = self.xfade[lv].1.x.clamp(0.0, 1.0);
            if self.xfade[lv].0.is_none() || k >= 0.999 {
                continue;
            }
            for q in p[item_ranges[lv].0..item_ranges[lv].1].iter_mut() {
                q.fade_by(k);
            }
            for t in l[item_ranges[lv].2..item_ranges[lv].3].iter_mut() {
                t.opacity *= k;
            }
            if let Some((op, ol)) = &self.xfade[lv].0 {
                for q in op {
                    let mut q = *q;
                    q.fade_by(1.0 - k);
                    p.push(q);
                }
                for t in ol {
                    let mut t = t.clone();
                    t.opacity *= 1.0 - k;
                    l.push(t);
                }
            }
        }
        self.snap = new_snap;
        self.last_path = self.path.clone();
        // submenus grow out of the row that opened them
        for lv in 1..self.levels.len() {
            let k = self.sub_k[lv].x.clamp(0.0, 1.0);
            if k >= 0.999 {
                continue;
            }
            let rect = self.levels[lv].rect;
            let parent = self.levels[lv - 1].rows.get(self.path[lv - 1]).copied().unwrap_or(rect);
            let ax = if rect[0] >= parent[0] { rect[0] } else { rect[0] + rect[2] };
            let pv = [ax, parent[1] + parent[3] / 2.0];
            let s = 0.86 + 0.14 * self.sub_k[lv].x.min(1.06);
            let mut idx: Vec<usize> = (ranges[lv].0..ranges[lv].1).collect();
            idx.extend(item_ranges[lv].0..item_ranges[lv].1);
            for i in idx {
                let q = &mut p[i];
                q.rect[0] = pv[0] + (q.rect[0] - pv[0]) * s;
                q.rect[1] = pv[1] + (q.rect[1] - pv[1]) * s;
                q.rect[2] *= s;
                q.rect[3] *= s;
                q.params[0] *= s;
                q.extra[1] *= s;
                q.extra[2] *= s;
                q.fade_by(k);
            }
            for t in l[item_ranges[lv].2..item_ranges[lv].3].iter_mut() {
                t.x = pv[0] + (t.x - pv[0]) * s;
                t.y = pv[1] + (t.y - pv[1]) * s;
                t.size *= s;
                t.opacity *= k;
            }
        }
        // pop-in: grow out of the point the menu opened at
        let k = self.open_k.x.clamp(0.0, 1.0);
        if k < 0.999 {
            let s = 0.92 + 0.08 * self.open_k.x.min(1.06);
            let pv = self.at;
            for q in p.iter_mut() {
                q.rect[0] = pv[0] + (q.rect[0] - pv[0]) * s;
                q.rect[1] = pv[1] + (q.rect[1] - pv[1]) * s;
                q.rect[2] *= s;
                q.rect[3] *= s;
                q.params[0] *= s;
                q.extra[1] *= s;
                q.extra[2] *= s;
                if q.params[1] != 0.0 {
                    q.tint[3] *= k;
                }
            }
            for t in l.iter_mut() {
                t.x = pv[0] + (t.x - pv[0]) * s;
                t.y = pv[1] + (t.y - pv[1]) * s;
                t.size *= s;
                t.opacity *= k;
            }
        }
        (p, l)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        let mut m = Menu::default();
        let mut items = vec![MenuItem::new(1, "One"), MenuItem::new(2, "Two"), MenuItem::new(3, "Three")];
        items[0] = MenuItem::new(0, "Sub").sub(vec![MenuItem::new(10, "A"), MenuItem::new(11, "B")]);
        m.open_at([100.0, 100.0], items);
        m.layout(&crate::theme::Theme::default(), [1000.0, 800.0], &|t: &str, s: f32, _| t.len() as f32 * s * 0.5);
        m
    }

    #[test]
    fn crossing_the_rows_towards_a_submenu_keeps_it_open() {
        let mut m = menu();
        // hover the first row: its submenu opens to the right
        let row0 = m.levels[0].rows[0];
        let start = [row0[0] + row0[2] - 10.0, row0[1] + row0[3] / 2.0];
        m.set_mouse(start);
        assert_eq!(m.path, vec![0]);
        // the app lays the menu out again on the next frame, which brings the submenu in
        m.layout(&crate::theme::Theme::default(), [1000.0, 800.0], &|t: &str, s: f32, _| t.len() as f32 * s * 0.5);
        // slide down-right across the second row, towards the submenu's lower part: still on the way
        let sub0 = m.levels[1].rows[0];
        let row1 = m.levels[0].rows[1];
        let step = [row0[0] + row0[2] - 2.0, row1[1] + 4.0];
        assert!(step[0] < sub0[0], "the pointer is still between the menus");
        m.set_mouse(step);
        assert_eq!(m.path, vec![0], "the submenu must stay open");
    }

    #[test]
    fn leaving_in_another_direction_switches_rows() {
        let mut m = menu();
        let row0 = m.levels[0].rows[0];
        m.set_mouse([row0[0] + 10.0, row0[1] + row0[3] / 2.0]);
        assert_eq!(m.path, vec![0]);
        // straight down on the left side is not towards the submenu
        let row2 = m.levels[0].rows[2];
        m.set_mouse([row2[0] + 10.0, row2[1] + 5.0]);
        assert!(m.path.is_empty());
    }
}
