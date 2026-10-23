//! The shell scene (menu bar pill, Dock, Quick Settings) plus its spring-driven state.

use crate::controlcenter::ControlCenter;
use crate::renderer::{Item, Label, Panel};
use chebrium::spring::Spring;
use crate::theme::Theme;

const GAP: f32 = 14.0;
const PAD: f32 = 14.0;

/// (icon glyph id in icons.wgsl, background top, background bottom)
const ICON_COLORS: [(u32, [f32; 3], [f32; 3]); 8] = [
    (0, [0.60, 0.87, 1.00], [0.08, 0.44, 0.97]), // files
    (1, [0.99, 0.99, 1.00], [0.86, 0.88, 0.94]), // browser
    (2, [0.45, 0.92, 0.42], [0.10, 0.72, 0.28]), // chat
    (3, [0.38, 0.74, 1.00], [0.05, 0.40, 0.95]), // Mail
    (4, [1.00, 1.00, 1.00], [0.95, 0.95, 0.97]), // Calendar
    (7, [1.00, 1.00, 1.00], [0.94, 0.94, 0.96]), // Photos
    (6, [1.00, 0.40, 0.46], [0.98, 0.14, 0.26]), // Music
    (5, [0.76, 0.78, 0.82], [0.46, 0.48, 0.54]), // Settings
];

#[derive(Clone, Copy, PartialEq)]
enum WinKind {
    Files,
    Terminal,
    Notes,
}

struct Window {
    kind: WinKind,
    size: [f32; 2],
    /// top-left corner, spring-driven so dragging has a slightly liquid lag
    x: Spring,
    y: Spring,
    /// grow/shrink about the centre (grab = lift, release = settle)
    scale: Spring,
}

const TITLEBAR: f32 = 44.0;

impl Window {
    fn new(kind: WinKind, x: f32, y: f32, w: f32, h: f32) -> Self {
        let mut x = Spring::new(x, 700.0, 0.8);
        let mut y = Spring::new(y, 700.0, 0.8);
        x.target = x.x;
        y.target = y.x;
        Self { kind, size: [w, h], x, y, scale: Spring::new(1.0, 320.0, 0.45) }
    }

    fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x.x && p[0] <= self.x.x + self.size[0] && p[1] >= self.y.x && p[1] <= self.y.x + self.size[1]
    }

    fn in_titlebar(&self, p: [f32; 2]) -> bool {
        self.contains(p) && p[1] <= self.y.x + TITLEBAR
    }

    /// Build the window's panels (body, controls, fake content).
    fn panels(&self, focused: bool) -> Vec<Panel> {
        let k = self.scale.x;
        let (ox, oy) = (self.x.x, self.y.x);
        let (cx, cy) = (ox + self.size[0] / 2.0, oy + self.size[1] / 2.0);
        // local (top-left origin) rect -> world rect, scaled about the window centre
        let place = |lx: f32, ly: f32, w: f32, h: f32| -> (f32, f32, f32, f32) {
            let wx = ox + lx + w / 2.0;
            let wy = oy + ly + h / 2.0;
            (cx + (wx - cx) * k, cy + (wy - cy) * k, w * k, h * k)
        };
        let mut p = Vec::new();
        let (dark, top, bot) = match self.kind {
            WinKind::Terminal => (true, [0.13, 0.14, 0.17], [0.08, 0.09, 0.11]),
            WinKind::Notes => (false, [1.0, 0.995, 0.96], [0.985, 0.975, 0.93]),
            WinKind::Files => (false, [0.97, 0.97, 0.985], [0.94, 0.94, 0.965]),
        };
        let (bx, by, bw, bh) = place(0.0, 0.0, self.size[0], self.size[1]);
        let (sh_op, sh_blur, sh_dy) = if focused { (0.42, 46.0, 22.0) } else { (0.22, 30.0, 12.0) };
        p.push(Panel::solid(bx, by, bw, bh, 26.0 * k, top, bot).flat().shadow(sh_op, sh_blur, sh_dy));

        let mut rect = |lx: f32, ly: f32, w: f32, h: f32, r: f32, a: [f32; 3], b: [f32; 3]| {
            let (x, y, w, h) = place(lx, ly, w, h);
            p.push(Panel::solid(x, y, w, h, r * k, a, b).flat());
        };

        match self.kind {
            WinKind::Files => {
                // sidebar is frosted glass over the wallpaper: pushed after the loop below
            }
            WinKind::Terminal => {
                let green = [0.33, 0.86, 0.45];
                let white = [0.85, 0.87, 0.92];
                let dim = [0.45, 0.48, 0.56];
                for i in 0..9 {
                    let y = 66.0 + i as f32 * 26.0;
                    let prompt = i % 3 == 0;
                    rect(26.0, y, 34.0, 9.0, 4.5, green, green);
                    let w = [150.0, 210.0, 90.0, 260.0, 120.0, 180.0][i % 6];
                    rect(70.0, y, w, 9.0, 4.5, if prompt { white } else { dim }, if prompt { white } else { dim });
                }
            }
            WinKind::Notes => {
                let ink = [0.16, 0.16, 0.18];
                let gray = [0.80, 0.80, 0.82];
                rect(32.0, 70.0, 210.0, 20.0, 10.0, ink, ink); // title
                rect(32.0, 106.0, 110.0, 9.0, 4.5, [0.98, 0.78, 0.16], [0.98, 0.78, 0.16]); // date
                for i in 0..8 {
                    let w = [340.0, 360.0, 300.0, 350.0, 220.0, 340.0, 280.0, 180.0][i];
                    rect(32.0, 138.0 + i as f32 * 26.0, w, 9.0, 4.5, gray, gray);
                }
            }
        }
        // files extras (separate so the borrow of `p` above has ended)
        drop(rect);
        if self.kind == WinKind::Files {
            let (sx, sy, sw, sh) = place(10.0, 10.0, 184.0, self.size[1] - 20.0);
            let mut side = Panel::glass(sx, sy, sw, sh, 20.0 * k).shadow(0.0, 0.0, 0.0);
            side.tint = [0.92, 0.94, 0.98, 0.30]; // frosted, a frosted sidebar
            p.push(side);
            let blue = [0.30, 0.55, 1.0];
            let ink = [0.28, 0.30, 0.38];
            let (x, y, w, h) = place(20.0, 96.0, 164.0, 28.0);
            p.push(Panel::solid(x, y, w, h, 9.0 * k, [0.80, 0.87, 1.0], [0.80, 0.87, 1.0]).flat()); // selected row
            for i in 0..6 {
                let ly = 100.0 + i as f32 * 32.0;
                let (x, y, w, h) = place(32.0, ly, 16.0, 16.0);
                p.push(Panel::solid(x, y, w, h, 5.0 * k, blue, blue).flat());
                let (x, y, w, h) = place(58.0, ly + 4.0, [90.0, 70.0, 100.0, 60.0, 80.0, 74.0][i], 9.0);
                p.push(Panel::solid(x, y, w, h, 4.5 * k, ink, ink).flat());
            }
            // main area: title + file grid
            let (x, y, w, h) = place(224.0, 20.0, 150.0, 12.0);
            p.push(Panel::solid(x, y, w, h, 6.0 * k, [0.25, 0.26, 0.32], [0.25, 0.26, 0.32]).flat());
            for j in 0..2 {
                for i in 0..3 {
                    let lx = 232.0 + i as f32 * 128.0;
                    let ly = 86.0 + j as f32 * 140.0;
                    let (x, y, w, h) = place(lx, ly, 84.0, 66.0);
                    p.push(Panel::solid(x, y, w, h, 14.0 * k, [0.45, 0.75, 1.0], [0.20, 0.50, 0.98]).flat());
                    let (x, y, w, h) = place(lx + 6.0, ly + 80.0, 72.0, 9.0);
                    p.push(Panel::solid(x, y, w, h, 4.5 * k, [0.62, 0.64, 0.70], [0.62, 0.64, 0.70]).flat());
                }
            }
        }

        // traffic lights (grey when the window is not focused)
        let lights: [[f32; 3]; 3] = if focused {
            [[1.0, 0.37, 0.34], [1.0, 0.75, 0.18], [0.16, 0.80, 0.26]]
        } else {
            [[0.74, 0.74, 0.76]; 3]
        };
        for (i, c) in lights.iter().enumerate() {
            let (x, y, w, h) = place(26.0 + i as f32 * 21.0, 24.0, 13.0, 13.0);
            let hi = [c[0] * 0.8 + 0.2, c[1] * 0.8 + 0.2, c[2] * 0.8 + 0.2];
            p.push(Panel::solid(x, y, w, h, 6.5 * k, hi, *c).flat());
        }
        let _ = dark;
        p
    }
}

/// How a Dock item draws its icon.
#[derive(Clone)]
pub enum DockIcon {
    /// built-in Apple-style tile (glyph id in icons.wgsl, background top, bottom)
    Procedural(u32, [f32; 3], [f32; 3]),
    /// application icon from the renderer's app-icon atlas
    App(u32),
}

#[derive(Clone)]
pub struct DockItem {
    /// stable identity (desktop file id): springs follow the key across updates
    pub key: String,
    pub name: String,
    pub icon: DockIcon,
    pub running: bool,
    /// draw a divider (and extra gap) before this item
    pub separator_before: bool,
    /// the windows the app has open (previews and the context menu list them)
    pub windows: Vec<DockWin>,
    /// the app is kept in the Dock (pinned), not there only because it runs
    pub pinned: bool,
}

/// One window of a Dock item.
#[derive(Clone, Debug, PartialEq)]
pub struct DockWin {
    /// the compositor's id of the window (stable while it lives)
    pub id: u64,
    pub title: String,
    pub minimized: bool,
}

struct DockEntry {
    item: DockItem,
    scale: Spring,
    bounce: Spring,
}

/// Things the shell wants the compositor to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellEvent {
    /// a Dock / taskbar item was clicked (its key)
    DockClick(String),
}

const SEP: f32 = 16.0;

fn new_entry(item: DockItem) -> DockEntry {
    DockEntry { item, scale: Spring::new(1.0, 320.0, 0.55), bounce: Spring::new(0.0, 190.0, 0.32) }
}

fn default_dock() -> Vec<DockItem> {
    let names = ["Files", "Browser", "Chat", "Mail", "Calendar", "Photos", "Music", "Settings"];
    ICON_COLORS
        .iter()
        .zip(names)
        .enumerate()
        .map(|(i, (&(id, top, bot), name))| DockItem { key: format!("demo{i}"), name: name.into(), icon: DockIcon::Procedural(id, top, bot), running: i == 0, separator_before: false, windows: Vec::new(), pinned: true })
        .collect()
}

pub struct Scene {
    anim: crate::theme::AnimStyle,
    size: [f32; 2],
    mouse: [f32; 2],
    pressed: bool,
    dock: Vec<DockEntry>,
    events: Vec<ShellEvent>,
    dock_pulse: Spring,
    dock_hot: bool,
    theme: Theme,
    /// Quick Settings (any bar can host its pill)
    cc: Option<ControlCenter>,
    windows: Vec<Window>,
    /// back-to-front draw order (indices into `windows`); last = focused
    z: Vec<usize>,
    drag: Option<(usize, [f32; 2])>,
    /// name of the focused app, shown at the left of the menu bar
    menu_title: String,
    menu_light: bool,
    /// the Dock is part of the scene (the standalone preview); the compositor gets it from its bars instead
    legacy_dock: bool,
}

impl Scene {
    pub fn new(w: f32, h: f32) -> Self {
        Self::with_theme(w, h, &Theme::default())
    }

    pub fn with_theme(w: f32, h: f32, theme: &Theme) -> Self {
        let dock = default_dock().into_iter().map(new_entry).collect();
        // the Quick Settings is available in every layout (any bar can host its pill)
        let mut c = ControlCenter::new(w, h);
        c.set_style(Self::cc_style(theme), theme.dark, theme.accent);
        c.set_glass(&theme.glass);
        c.set_bar_under(Self::bar_under_of(theme));
        let cc = Some(c);

        Self {
            anim: theme.animations.clone(),
            size: [w, h],
            mouse: [-1000.0, -1000.0],
            pressed: false,
            dock,
            events: Vec::new(),
            dock_pulse: Spring::new(0.0, 220.0, 0.3),
            dock_hot: false,
            theme: theme.clone(),
            cc,
            windows: vec![
                Window::new(WinKind::Files, 120.0, 96.0, 640.0, 420.0),
                Window::new(WinKind::Notes, 800.0, 150.0, 420.0, 380.0),
                Window::new(WinKind::Terminal, 470.0, 330.0, 580.0, 330.0),
            ],
            z: vec![0, 1, 2],
            drag: None,
            menu_title: String::new(),
            menu_light: false,
            legacy_dock: true,
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Apply a changed theme: cheap field updates, or a rebuild when the layout kind changed.
    pub fn set_theme(&mut self, theme: &Theme) {
        if theme.layout != self.theme.layout {
            let dock_items: Vec<DockItem> = self.dock.iter().map(|e| e.item.clone()).collect();
            let edit = self.edit_mode();
            let open = self.cc_open();
            *self = Scene::shell_only_with(self.size[0], self.size[1], theme);
            self.set_dock(dock_items);
            self.set_cc_open(open);
            self.set_edit(edit);
            return;
        }
        self.theme = theme.clone();
        if let Some(c) = &mut self.cc {
            c.set_style(Self::cc_style(theme), theme.dark, theme.accent);
            c.set_glass(&theme.glass);
            c.set_bar_under(Self::bar_under_of(theme));
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        *self = Scene::with_theme(w, h, &self.theme.clone());
    }

    /// Dock + Quick Settings only (the compositor supplies the real windows).
    pub fn shell_only(w: f32, h: f32) -> Self {
        Self::shell_only_with(w, h, &Theme::default())
    }

    pub fn shell_only_with(w: f32, h: f32, theme: &Theme) -> Self {
        let mut s = Scene::with_theme(w, h, theme);
        s.windows.clear();
        s.z.clear();
        s.legacy_dock = false;
        s
    }

    /// Area left for windows (x, y, w, h): above the Dock, below the menu bar.
    pub fn work_area(&self) -> [f32; 4] {
        [0.0, self.theme.menubar.height, self.size[0], self.size[1] - self.theme.menubar.height - 112.0]
    }

    /// Where a window should shrink to when minimized: the centre of its Dock/taskbar item.
    pub fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        if !self.legacy_dock {
            return None;
        }
        let (_, _, l) = self.icon_layout();
        let i = self.dock.iter().position(|e| e.item.key == key)?;
        Some([l[i].0, l[i].1])
    }

    /// Dock icon index under the pointer, if any.
    pub fn dock_icon_at_pointer(&self) -> Option<usize> {
        self.icon_under_mouse()
    }

    pub fn set_mouse(&mut self, x: f32, y: f32) {
        self.mouse = [x, y];
        if let Some((wi, grab)) = self.drag {
            let w = &mut self.windows[wi];
            w.x.target = x - grab[0];
            w.y.target = y - grab[1];
        }
    }

    /// Is the pointer over the Dock or a Quick Settings tile?
    pub fn shell_hit(&self) -> bool {
        (self.legacy_dock && self.icon_under_mouse().is_some()) || self.cc.as_ref().is_some_and(|c| c.hit(self.mouse))
    }

    pub fn mouse_down(&mut self) {
        self.pressed = true;
        if self.legacy_dock {
            if let Some(i) = self.icon_under_mouse() {
            self.dock[i].bounce.kick(420.0);
            self.events.push(ShellEvent::DockClick(self.dock[i].item.key.clone()));
            return;
            }
        }
        if self.cc.as_mut().is_some_and(|c| c.mouse_down(self.mouse)) || self.shell_hit() {
            return;
        }
        // topmost window under the pointer: focus it, and drag by the title bar
        let m = self.mouse;
        let hit = self.z.iter().rposition(|&wi| self.windows[wi].contains(m));
        if let Some(pos) = hit {
            let wi = self.z.remove(pos);
            self.z.push(wi);
            let w = &mut self.windows[wi];
            if w.in_titlebar(m) {
                self.drag = Some((wi, [m[0] - w.x.x, m[1] - w.y.x]));
                w.scale.target = 1.018; // lifted while held
            }
        }
    }

    pub fn mouse_up(&mut self) {
        self.pressed = false;
        if let Some((wi, _)) = self.drag.take() {
            self.windows[wi].scale.target = 1.0;
        }
    }

    /// Replace the Dock contents; springs are kept for items whose key survives.
    pub fn set_dock(&mut self, items: Vec<DockItem>) {
        let mut old: Vec<DockEntry> = std::mem::take(&mut self.dock);
        for it in items {
            if let Some(pos) = old.iter().position(|e| e.item.key == it.key) {
                let mut e = old.remove(pos);
                e.item = it;
                self.dock.push(e);
            } else {
                let mut e = new_entry(it);
                e.scale.x = 0.2; // pops in
                self.dock.push(e);
            }
        }
    }

    pub fn take_events(&mut self) -> Vec<ShellEvent> {
        std::mem::take(&mut self.events)
    }

    /// Like `set_dock`; the focused item is only used by bar widgets, not by the preview Dock.
    pub fn set_dock_focus(&mut self, items: Vec<DockItem>, _focused: Option<&str>) {
        self.set_dock(items);
    }

    /// Left edge of every icon for the given icon sizes (separators add a gap), and the total width.
    fn dock_positions(&self, sizes: &[f32]) -> (f32, Vec<f32>) {
        let mut lefts = Vec::new();
        let mut x = PAD;
        for (i, s) in sizes.iter().enumerate() {
            if i > 0 {
                x += GAP;
                if self.dock[i].item.separator_before {
                    x += SEP;
                }
            }
            lefts.push(x);
            x += s;
        }
        (x + PAD, lefts)
    }

    fn dock_geometry(&self) -> (f32, f32, f32, f32) {
        let sizes = vec![self.theme.dock.icon_size; self.dock.len()];
        let (total_base, _) = self.dock_positions(&sizes);
        let dh = self.theme.dock.icon_size + 2.0 * PAD;
        (self.size[0] / 2.0, self.size[1] - 14.0 - dh / 2.0, total_base, dh)
    }

    /// Current icon rects (cx, cy, size) following the live springs.
    fn icon_layout(&self) -> (f32, f32, Vec<(f32, f32, f32)>) {
        let (dcx, dcy, _, dh) = self.dock_geometry();
        let sizes: Vec<f32> = self.dock.iter().map(|i| self.theme.dock.icon_size * i.scale.x).collect();
        let (total, lefts) = self.dock_positions(&sizes);
        let bottom = dcy + dh / 2.0 - PAD;
        let x0 = dcx - total / 2.0;
        let out = sizes.iter().zip(&lefts).zip(&self.dock).map(|((s, l), ic)| (x0 + l + s / 2.0, bottom - s / 2.0 - ic.bounce.x, *s)).collect();
        (total, dh, out)
    }

    fn icon_under_mouse(&self) -> Option<usize> {
        let (_, _, l) = self.icon_layout();
        l.iter().position(|&(cx, cy, s)| (self.mouse[0] - cx).abs() < s / 2.0 && (self.mouse[1] - cy).abs() < s / 2.0)
    }

    /// The animation settings (which effects are on, and how fast) the next `update` follows.
    pub fn set_animation(&mut self, an: &crate::theme::AnimStyle) {
        if &self.anim != an {
            self.anim = an.clone();
        }
    }

    pub fn update(&mut self, dt: f32) {
        // Dock magnification: Gaussian falloff around the pointer.
        let (dcx, dcy, base_w, dh) = self.dock_geometry();
        let near = self.theme.dock.magnify && !self.dock.is_empty() && (self.mouse[0] - dcx).abs() < base_w / 2.0 + 30.0 && (self.mouse[1] - dcy).abs() < dh / 2.0 + 30.0;
        if near && !self.dock_hot {
            self.dock_pulse.kick(5.0); // jelly: dock stretches when the pointer arrives
        }
        self.dock_hot = near;
        let (_, lefts) = self.dock_positions(&vec![self.theme.dock.icon_size; self.dock.len()]);
        for (i, left) in lefts.iter().enumerate() {
            let rest_x = dcx - base_w / 2.0 + left + self.theme.dock.icon_size / 2.0;
            let d = (self.mouse[0] - rest_x) / 105.0;
            self.dock[i].scale.target = if near { 1.0 + 0.62 * (-d * d).exp() } else { 1.0 };
            self.anim.step("dock", &mut self.dock[i].scale, dt);
            self.anim.step("dock", &mut self.dock[i].bounce, dt);
        }
        self.anim.step("dock", &mut self.dock_pulse, dt);
        for w in &mut self.windows {
            w.x.update(dt);
            w.y.update(dt);
            w.scale.update(dt);
        }

        if let Some(c) = &mut self.cc {
            c.update(dt, &self.anim, self.mouse, self.pressed, self.drag.is_some());
        }
    }

    /// (window layer, shell layer): the shell's glass refracts everything in the window layer.
    pub fn layers(&self) -> (Vec<Item<'static>>, Vec<Panel>) {
        let mut win = Vec::new();
        for (pos, &wi) in self.z.iter().enumerate() {
            let w = &self.windows[wi];
            let pad = 90.0; // shadow reach
            win.push(Item::Frame {
                panels: w.panels(pos + 1 == self.z.len()),
                bbox: [w.x.x - pad, w.y.x - pad, w.x.x + w.size[0] + pad, w.y.x + w.size[1] + pad],
                fade: 1.0,
            });
        }
        (win, self.shell_panels())
    }

    pub fn shell_panels(&self) -> Vec<Panel> {
        let mut p = Vec::new();
        let (dcx, dcy, _, _) = self.dock_geometry();

        let (total, dh, icons) = self.icon_layout();
        let stretch = self.dock_pulse.x;
        if self.legacy_dock && !self.dock.is_empty() {
            p.push(self.theme.glass.panel(dcx, dcy, total * (1.0 + 0.012 * stretch), dh * (1.0 - 0.02 * stretch), self.theme.dock.radius).adaptive());
        }
        let dock_items: &[DockEntry] = if self.legacy_dock { &self.dock } else { &[] };
        for ((&(cx, cy, s), e), i) in icons.iter().zip(dock_items).zip(0..) {
            match &e.item.icon {
                DockIcon::Procedural(id, top, bot) => p.push(Panel::icon(cx, cy, s, *id, *top, *bot).shadow(0.22, 10.0, 4.0)),
                DockIcon::App(cell) => crate::theme::push_app_icon(&self.theme, &mut p, cx, cy, s, *cell, (0.20, 10.0, 4.0)),
            }
            if e.item.separator_before && i > 0 {
                let sx = cx - s / 2.0 - (GAP + SEP) / 2.0;
                p.push(Panel::solid(sx, dcy, 1.5, self.theme.dock.icon_size * 0.7, 0.75, [0.35, 0.42, 0.55], [0.35, 0.42, 0.55]).flat().alpha(0.35));
            }
            if e.item.running {
                p.push(Panel::solid(cx, dcy + dh / 2.0 - 6.0, 5.0, 5.0, 2.5, [0.1; 3], [0.1; 3]));
            }
        }
        // tooltip with the app name above the hovered icon
        if let Some(i) = self.icon_under_mouse().filter(|_| self.legacy_dock) {
            let (cx, cy, s) = icons[i];
            let w = self.dock[i].item.name.chars().count() as f32 * 7.2 + 28.0;
            p.push(Panel::glass(cx, cy - s / 2.0 - 24.0, w, 28.0, 14.0).tinted([0.07, 0.10, 0.18, 0.62]).shadow(0.18, 14.0, 5.0));
        }

        if let Some(c) = &self.cc {
            p.extend(c.panels());
        }
        p
    }

    /// Text labels of the shell (Quick Settings, menu bar, Dock tooltip); drawn on top of the shell panels.
    pub fn labels(&self) -> Vec<Label> {
        let mut l = self.cc.as_ref().map(|c| c.labels()).unwrap_or_default();
        if let Some(i) = self.icon_under_mouse().filter(|_| self.legacy_dock) {
            let (_, _, icons) = self.icon_layout();
            let (cx, cy, s) = icons[i];
            l.push(Label::new(self.dock[i].item.name.clone(), cx, cy - s / 2.0 - 19.5, 13.0, true, [1.0, 1.0, 1.0], 0.95, crate::renderer::Align::Center));
        }
        l
    }

    pub fn edit_mode(&self) -> bool {
        self.cc.as_ref().is_some_and(|c| c.edit)
    }

    /// Open/close the "Edit Controls" mode (also toggled by clicking the menu bar pill).
    pub fn set_edit(&mut self, on: bool) {
        if let Some(c) = &mut self.cc {
            c.set_edit(on);
        }
    }

    /// Show / hide the Quick Settings dropdown.
    pub fn set_cc_open(&mut self, on: bool) {
        if let Some(c) = &mut self.cc {
            c.set_open(on);
        }
    }

    /// Tell the Quick Settings what the machine looks like (it shows Wi-Fi, volume, ...).
    pub fn cc_set_media(&mut self, m: &crate::controlcenter::CcMedia) {
        if let Some(c) = &mut self.cc {
            c.set_media(m);
        }
    }

    /// The tiles of the Quick Settings (from the config; empty = the default layout).
    pub fn cc_set_layout(&mut self, layout: &[crate::cctiles::CcTile]) {
        if let Some(c) = &mut self.cc {
            c.set_layout(layout);
        }
    }

    /// Does the Quick Settings show a turning record (the shell has to keep it turning)?
    pub fn cc_wants_record(&self) -> bool {
        self.cc.as_ref().is_some_and(|c| c.wants_record())
    }

    /// How far the record in the Now Playing tile has turned.
    pub fn cc_set_record_angle(&mut self, angle: f32) {
        if let Some(c) = &mut self.cc {
            c.set_record_angle(angle);
        }
    }

    pub fn cc_set_sys(&mut self, info: &crate::sysctl::SysInfo) {
        if let Some(c) = &mut self.cc {
            c.set_sys(info);
        }
    }

    /// What the Quick Settings wants done (toggle Wi-Fi, set the volume, ...).
    pub fn take_cc_events(&mut self) -> Vec<crate::controlcenter::CcEvent> {
        self.cc.as_mut().map(|c| c.take_events()).unwrap_or_default()
    }

    pub fn cc_open(&self) -> bool {
        self.cc.as_ref().is_some_and(|c| c.open)
    }

    /// The Quick Settings looks like Windows quick settings in the taskbar layout.
    fn cc_style(theme: &Theme) -> crate::controlcenter::CcStyle {
        match theme.layout {
            crate::theme::Layout::Macos => crate::controlcenter::CcStyle::Mac,
            crate::theme::Layout::Taskbar => crate::controlcenter::CcStyle::Windows,
        }
    }

    /// Dark wallpaper: menu bar text turns light.
    fn bar_under_of(theme: &Theme) -> [f32; 4] {
        crate::widget::under_of(theme, &theme.menubar)
    }

    /// Tint of the menu bar background, for ink that adapts to it.
    pub fn bar_under(&self) -> [f32; 4] {
        Self::bar_under_of(&self.theme)
    }

    /// Where the Quick Settings hangs: centre of its pill (or of the widget that opens it), whether the bar is at
    /// the bottom, and whether the pill itself is drawn.
    pub fn set_pill(&mut self, x: f32, y: f32, flip: bool, visible: bool) {
        if let Some(c) = &mut self.cc {
            c.set_pill(x, y, flip, visible);
        }
    }

    pub fn set_menu_title(&mut self, title: &str) {
        if self.menu_title != title {
            self.menu_title = title.to_string();
        }
    }

    pub fn set_menu_light(&mut self, light: bool) {
        self.menu_light = light;
        if let Some(c) = &mut self.cc {
            c.set_menu_light(light);
        }
    }
}
