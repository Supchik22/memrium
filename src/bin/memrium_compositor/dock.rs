//! What the Dock and the taskbar do beyond showing icons: window previews on hover (a thumbnail with the title and
//! a close button for each window of the app) and a context menu for an item (open a new window, pin, move to
//! another desktop, close...).

use crate::state::{win_id, MenuFor, State, SHELL_FILES, SHELL_SETTINGS, WORKSPACES};
use chebrium::spring::Spring;
use memrium_de::apps;
use smithay::reexports::wayland_server::Resource;
use memrium_de::menu::MenuItem;
use memrium_de::renderer::{Align, Item, Label, Panel};
use memrium_de::scene::DockWin;
use memrium_de::widget::DockHover;
use std::collections::HashMap;

const CARD_W: f32 = 204.0;
const THUMB_H: f32 = 116.0;
const TITLE_H: f32 = 28.0;
const CARD_H: f32 = THUMB_H + TITLE_H + 8.0;
const PAD: f32 = 10.0;
const COLS: usize = 5;
/// seconds the pointer rests on an item before its previews show, and how long they stay after it leaves
const SHOW_AFTER: f32 = 0.38;
const SWITCH_AFTER: f32 = 0.06;
const LINGER: f32 = 0.22;
/// seconds carried files rest on a Dock item before its window comes up
const DRAG_FOCUS_AFTER: f32 = 0.5;

const M_NEW: u32 = 8001;
const M_PIN: u32 = 8002;
const M_CLOSE: u32 = 8003;
const M_MIN: u32 = 8004;
const M_MAX: u32 = 8005;
const M_FULL: u32 = 8006;
const M_EDIT: u32 = 8007;
const M_WIN: u32 = 8100; // + index in the window list
const M_DESK: u32 = 8200; // + desktop number

/// The preview popup being shown (or fading away).
pub struct DockPreview {
    pub key: String,
    /// the hovered item on screen
    pub anchor: [f32; 4],
    pub k: Spring,
    pub closing: bool,
}

/// What the popup looks like this frame.
pub struct PreviewLayout {
    pub popup: [f32; 4],
    pub cards: Vec<PreviewCard>,
    pub fade: f32,
}

pub struct PreviewCard {
    pub win: DockWin,
    pub rect: [f32; 4],
    pub thumb: [f32; 4],
    pub close: [f32; 4],
    pub hover: bool,
    pub hover_close: bool,
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

impl State {
    /// The Dock / taskbar item under the pointer, in screen coordinates.
    pub fn dock_item_at(&self) -> Option<DockHover> {
        if !self.bar_active() {
            return None;
        }
        let i = self.mon_at(self.mouse);
        self.bar_mon.set(i);
        let o = self.mon_origin(i);
        let mut h = self.with_bar_ctx(|ctx| self.bars.iter().find_map(|b| b.item_at(ctx)))?;
        h.rect[0] += o[0];
        h.rect[1] += o[1];
        Some(h)
    }

    // ------------------------------------------------------------ widget actions and tooltips

    /// The widget under `p`: its bar, its index and its rectangle on screen.
    fn bar_widget_under(&self, p: [f32; 2]) -> Option<(usize, usize, [f32; 4])> {
        let b = self.bar_at(p)?;
        let o = self.mon_origin(self.mon_at(p));
        let i = self.panel_widget_at(b, [p[0] - o[0], p[1] - o[1]])?;
        let (_, r, _) = self.panel_widgets(b).into_iter().find(|(w, _, _)| *w == i)?;
        Some((b, i, [r[0] + o[0], r[1] + o[1], r[2], r[3]]))
    }

    /// Run the action a widget has under `key` (`on_right`, `on_middle`, `scroll_up`, `scroll_down`). True when it had one.
    pub fn bar_alt_action(&mut self, key: &str) -> bool {
        let Some((b, i, _)) = self.bar_widget_under(self.mouse) else { return false };
        let Some(action) = self.bars.get(b).and_then(|bar| bar.spec_of(i)).and_then(|s| s.opts.get(key)).and_then(|v| v.as_str()).map(str::to_string) else { return false };
        if action.trim().is_empty() || action == "none" {
            return false;
        }
        self.menu.close();
        self.bar_action(action.trim());
        true
    }

    /// The tooltip of the widget the pointer rests on, once it has rested long enough.
    pub fn update_tooltip(&mut self, dt: f32) {
        let hov = if self.launcher_modal() || self.menu.open || self.grab.is_some() || self.panel_edit.is_some() { None } else { self.bar_widget_under(self.mouse) };
        let text = hov.and_then(|(b, i, r)| {
            let t = self.bars.get(b)?.spec_of(i)?.opts.get("tooltip")?.as_str()?.trim().to_string();
            (!t.is_empty()).then_some((b, i, r, t))
        });
        match text {
            Some((b, i, r, t)) => {
                if self.tip.0 != (b, i) {
                    self.tip = ((b, i), 0.0, None);
                }
                self.tip.1 += dt;
                self.tip.2 = Some((r, t));
            }
            None => self.tip = ((usize::MAX, 0), 0.0, None),
        }
    }

    /// A hover tooltip as a glass pill beside its widget.
    pub fn tooltip_panels(&mut self, shell: &mut Vec<Panel>) {
        let Some((r, text)) = self.tip.2.clone().filter(|_| self.tip.1 > 0.55) else { return };
        let w = self.renderer.text_width(&text, 12.5, false) + 24.0;
        let below = r[1] + r[3] / 2.0 < self.size[1] / 2.0;
        let cx = (r[0] + r[2] / 2.0).clamp(w / 2.0 + 6.0, self.size[0] - w / 2.0 - 6.0);
        let cy = if below { r[1] + r[3] + 22.0 } else { r[1] - 22.0 };
        let k = if self.theme.animations.on("popups") { ((self.tip.1 - 0.55) * 6.0 * self.theme.animations.fx_speed("popups")).clamp(0.0, 1.0) } else { 1.0 };
        let mut g = self.theme.glass.panel(cx, cy, w, 26.0, 13.0).tinted([0.07, 0.10, 0.18, 0.66]).shadow(0.18, 14.0, 5.0);
        g.fx[0] = g.fx[0].max(18.0);
        g.fade_by(k);
        shell.push(g);
        let l = Label::new(text, cx, cy + 4.5, 12.5, false, [1.0; 3], 0.95 * k, Align::Center);
        shell.extend(self.renderer.label_panels(&[l]));
    }

    // ------------------------------------------------------------ previews

    /// Track what the pointer rests on: open the previews after a moment, switch between items, close them after it left.
    pub fn update_dock_preview(&mut self, dt: f32) {
        self.update_dock_drag(dt);
        let an = self.theme.animations.clone();
        let busy = self.launcher_modal() || self.menu.open || self.grab.is_some() || self.panel_edit.is_some() || self.scene.edit_mode() || self.lock.is_some() || self.file_drag.is_some();
        let hov = if busy { None } else { self.dock_item_at() };
        let over_popup = self.preview_layout().is_some_and(|l| inside(self.mouse, l.popup));
        match hov.filter(|h| h.previews && !self.dock_windows(&h.key).is_empty()) {
            Some(h) => {
                if self.dock_wait.0 != h.key {
                    self.dock_wait = (h.key.clone(), 0.0);
                }
                self.dock_wait.1 += dt;
                self.dock_linger = 0.0;
                let open_key = self.preview.as_ref().filter(|p| !p.closing).map(|p| p.key.clone());
                let need = if open_key.is_some() { SWITCH_AFTER } else { SHOW_AFTER };
                if self.dock_wait.1 >= need && open_key.as_deref() != Some(h.key.as_str()) {
                    match &mut self.preview {
                        Some(p) => {
                            p.key = h.key.clone();
                            p.anchor = h.rect;
                            p.closing = false;
                            p.k.target = 1.0;
                        }
                        None => {
                            let mut k = Spring::new(0.0, 380.0, 0.8);
                            k.target = 1.0;
                            self.preview = Some(DockPreview { key: h.key.clone(), anchor: h.rect, k, closing: false });
                        }
                    }
                } else if let Some(p) = self.preview.as_mut().filter(|p| p.key == h.key) {
                    p.anchor = h.rect;
                }
            }
            None => {
                self.dock_wait = (String::new(), 0.0);
                if over_popup {
                    self.dock_linger = 0.0;
                } else {
                    self.dock_linger += dt;
                }
                if self.dock_linger > LINGER {
                    if let Some(p) = self.preview.as_mut() {
                        p.closing = true;
                        p.k.target = 0.0;
                    }
                }
            }
        }
        if let Some(p) = self.preview.as_mut() {
            an.step("popups", &mut p.k, dt);
        }
        if self.preview.as_ref().is_some_and(|p| p.closing && p.k.x < 0.02) {
            self.preview = None;
        }
        // the app has no window left: nothing to show
        if let Some(p) = &self.preview {
            if self.dock_windows(&p.key).is_empty() {
                self.preview = None;
            }
        }
    }

    /// Files carried over a Dock item rest there for a moment: the app's window comes up, so they can be dropped into
    /// a program that is not the focused one.
    fn update_dock_drag(&mut self, dt: f32) {
        let carrying = self.file_drag.as_ref().is_some_and(|d| d.active);
        let key = if carrying && self.lock.is_none() { self.dock_item_at().map(|h| h.key) } else { None };
        let Some(key) = key else {
            self.dock_drag = (String::new(), 0.0);
            return;
        };
        if self.dock_drag.0 != key {
            self.dock_drag = (key, 0.0);
            return;
        }
        if self.dock_drag.1 < 0.0 {
            return;
        }
        self.dock_drag.1 += dt;
        if self.dock_drag.1 >= DRAG_FOCUS_AFTER {
            self.dock_drag.1 = -1.0;
            let wins = self.dock_windows(&self.dock_drag.0);
            if let Some(w) = wins.iter().find(|w| !w.minimized).or(wins.first()) {
                self.dock_focus_window(w.id);
            }
        }
    }

    pub fn preview_active(&self) -> bool {
        self.preview.is_some()
    }

    /// Where the cards go: above the item (below it when the bar is at the top), centred on it, kept on its monitor.
    pub fn preview_layout(&self) -> Option<PreviewLayout> {
        let p = self.preview.as_ref()?;
        let wins = self.dock_windows(&p.key);
        if wins.is_empty() {
            return None;
        }
        let n = wins.len().min(COLS * 2);
        let cols = n.min(COLS);
        let rows = n.div_ceil(cols);
        let (w, h) = (cols as f32 * CARD_W + (cols as f32 + 1.0) * PAD, rows as f32 * CARD_H + (rows as f32 + 1.0) * PAD);
        let a = p.anchor;
        let mon = self.mons.get(self.mon_at([a[0] + a[2] / 2.0, a[1] + a[3] / 2.0])).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        let below = a[1] + a[3] / 2.0 < mon[1] + mon[3] / 2.0;
        let x = (a[0] + a[2] / 2.0 - w / 2.0).clamp(mon[0] + 8.0, (mon[0] + mon[2] - w - 8.0).max(mon[0] + 8.0));
        // a Dock's name tooltip sits just above its icon: leave it room
        let gap = if a[3] > 40.0 { 46.0 } else { 14.0 };
        let y = if below { a[1] + a[3] + gap } else { a[1] - h - gap };
        let slide = (1.0 - p.k.x.clamp(0.0, 1.0)) * 10.0 * if below { -1.0 } else { 1.0 };
        let popup = [x, y + slide, w, h];
        let m = self.mouse;
        let cards = wins
            .into_iter()
            .take(n)
            .enumerate()
            .map(|(i, win)| {
                let (c, r) = (i % cols, i / cols);
                let rect = [popup[0] + PAD + c as f32 * (CARD_W + PAD), popup[1] + PAD + r as f32 * (CARD_H + PAD), CARD_W, CARD_H];
                let close = [rect[0] + rect[2] - 26.0, rect[1] + 4.0, 22.0, 22.0];
                PreviewCard { win, rect, thumb: [rect[0] + 6.0, rect[1] + TITLE_H, rect[2] - 12.0, THUMB_H], close, hover: inside(m, rect), hover_close: inside(m, close) }
            })
            .collect();
        Some(PreviewLayout { popup, cards, fade: p.k.x.clamp(0.0, 1.0) })
    }

    pub fn preview_hit(&self, p: [f32; 2]) -> bool {
        self.preview_layout().is_some_and(|l| l.fade > 0.3 && inside(p, l.popup))
    }

    /// A left click on the popup: close a window, or bring it up. True when the click was on the popup.
    pub fn preview_click(&mut self) -> bool {
        let Some(l) = self.preview_layout().filter(|l| l.fade > 0.3) else { return false };
        if !inside(self.mouse, l.popup) {
            return false;
        }
        for c in &l.cards {
            if inside(self.mouse, c.close) {
                self.dock_close_window(c.win.id);
                return true;
            }
            if inside(self.mouse, c.rect) {
                self.dock_focus_window(c.win.id);
                if let Some(p) = self.preview.as_mut() {
                    p.closing = true;
                    p.k.target = 0.0;
                }
                return true;
            }
        }
        true
    }

    /// Middle click on a card closes its window.
    pub fn preview_middle_click(&mut self) -> bool {
        let Some(l) = self.preview_layout().filter(|l| l.fade > 0.3) else { return false };
        if let Some(c) = l.cards.iter().find(|c| inside(self.mouse, c.rect)) {
            self.dock_close_window(c.win.id);
            return true;
        }
        inside(self.mouse, l.popup)
    }

    fn win_by_id(&self, id: u64) -> Option<usize> {
        self.wins.iter().position(|w| win_id(w) == Some(id))
    }

    pub fn dock_focus_window(&mut self, id: u64) {
        match id {
            SHELL_SETTINGS => self.open_shell(crate::state::Sw::Settings),
            SHELL_FILES => self.open_shell(crate::state::Sw::Files),
            _ => {
                if let Some(i) = self.win_by_id(id) {
                    self.focus_window(i);
                }
            }
        }
    }

    pub fn dock_close_window(&mut self, id: u64) {
        match id {
            SHELL_SETTINGS => self.settings.close(),
            SHELL_FILES => self.files.close(),
            _ => {
                if let Some(i) = self.win_by_id(id) {
                    self.close_window(i);
                }
            }
        }
    }

    /// The popup as window-layer items (cards and thumbnails) and shell labels. Thumbnails are the windows' own
    /// textures, so they stay live.
    pub fn preview_shell(&mut self, l: &PreviewLayout) -> Vec<Label> {
        let ink = if self.theme.dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let mut labels = Vec::new();
        for c in &l.cards {
            let max = CARD_W - 12.0 - 28.0 - 24.0;
            let title = crate::state::ellipsize(&self.renderer, &c.win.title, 12.5, max);
            labels.push(Label::new(title, c.rect[0] + 34.0, c.rect[1] + 19.0, 12.5, c.hover, ink, 0.95 * l.fade, Align::Left));
        }
        labels
    }

    // ------------------------------------------------------------ context menu

    /// Right click on a Dock / taskbar item.
    pub fn open_dock_menu(&mut self, h: &DockHover) {
        let key = h.key.clone();
        let wins = self.dock_windows(&key);
        let app = self.apps.iter().find(|a| a.id == key).cloned();
        let pinned = self.pinned.contains(&key);
        let mut items: Vec<MenuItem> = Vec::new();
        if wins.len() > 1 {
            for (i, w) in wins.iter().enumerate().take(12) {
                let t: String = w.title.chars().take(34).collect();
                items.push(MenuItem::new(M_WIN + i as u32, t).icon(if w.minimized { "minus" } else { "app-window" }));
            }
            items.push(MenuItem::sep());
        }
        if app.as_ref().is_some_and(|a| a.id != apps::SETTINGS_ID && a.id != apps::FILES_ID) || (app.is_some() && wins.is_empty()) {
            let name = app.as_ref().map(|a| a.name.clone()).unwrap_or_default();
            items.push(MenuItem::new(M_NEW, if wins.is_empty() { format!("Open {name}") } else { "Open New Window".to_string() }).icon("plus"));
        }
        if app.is_some() {
            items.push(MenuItem::new(M_PIN, if pinned { "Unpin from Dock" } else { "Pin to Dock" }).icon("star"));
        }
        if !wins.is_empty() {
            items.push(MenuItem::sep());
            let front = wins.last().map(|w| w.id).and_then(|id| self.win_by_id(id));
            let desks: Vec<MenuItem> = (0..WORKSPACES)
                .map(|n| MenuItem::new(M_DESK + n as u32, format!("Desktop {}", n + 1)).checked(front.is_some_and(|i| self.wins[i].ws == n)))
                .collect();
            if wins.iter().any(|w| w.id < SHELL_FILES) {
                items.push(MenuItem::new(0, "Move to Desktop").icon("layout-grid").sub(desks));
                items.push(MenuItem::new(M_MIN, if wins.iter().all(|w| w.minimized) { "Restore" } else { "Minimize" }).icon("minus"));
                let maxed = front.is_some_and(|i| self.wins[i].restore.is_some());
                items.push(MenuItem::new(M_MAX, if maxed { "Restore Size" } else { "Maximize" }).icon("maximize-2"));
                items.push(MenuItem::new(M_FULL, "Fullscreen").icon("square"));
            }
            items.push(MenuItem::sep());
            items.push(MenuItem::new(M_CLOSE, if wins.len() > 1 { "Close All" } else { "Close" }).icon("x"));
        }
        items.push(MenuItem::sep());
        items.push(MenuItem::new(M_EDIT, "Edit Dock…").icon("pencil"));
        self.dock_menu = Some((key, wins));
        self.menu_for = MenuFor::Dock;
        self.preview = None;
        let above = h.rect[1] + h.rect[3] / 2.0 > self.size[1] / 2.0;
        // a menu from a Dock at the bottom opens upwards: the menu draws downwards from its point, so lift it by its height
        let height: f32 = items.iter().map(|i| if i.separator { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
        let at = if above { [h.rect[0] + h.rect[2] / 2.0, (h.rect[1] - 10.0 - height).max(8.0)] } else { [h.rect[0] + h.rect[2] / 2.0, h.rect[1] + h.rect[3] + 6.0] };
        self.menu.open_at(at, items);
    }

    pub fn dock_menu_command(&mut self, id: u32) {
        let Some((key, wins)) = self.dock_menu.clone() else { return };
        match id {
            M_NEW => {
                if let Some(app) = self.apps.iter().find(|a| a.id == key).cloned() {
                    self.launch_app(&app);
                }
            }
            M_PIN => {
                if let Some(pos) = self.pinned.iter().position(|k| *k == key) {
                    self.pinned.remove(pos);
                } else {
                    self.pinned.push(key.clone());
                }
                self.save_pins();
            }
            M_CLOSE => {
                for w in &wins {
                    self.dock_close_window(w.id);
                }
            }
            M_MIN => {
                let restore = wins.iter().all(|w| w.minimized);
                for w in &wins {
                    if restore {
                        self.dock_focus_window(w.id);
                    } else if let Some(i) = self.win_by_id(w.id) {
                        self.minimize(i);
                    }
                }
            }
            M_MAX => {
                if let Some(i) = wins.last().and_then(|w| self.win_by_id(w.id)) {
                    let i = self.focus_window(i);
                    self.toggle_maximize(i);
                }
            }
            M_FULL => {
                if let Some(i) = wins.last().and_then(|w| self.win_by_id(w.id)) {
                    let i = self.focus_window(i);
                    let on = self.wins[i].fs.is_none();
                    self.set_fullscreen(i, on);
                }
            }
            M_EDIT => {
                if let Some(b) = self.bar_at(self.mouse).or(Some(0)) {
                    self.panel_edit = Some(b);
                }
            }
            i if (M_WIN..M_WIN + 12).contains(&i) => {
                if let Some(w) = wins.get((i - M_WIN) as usize) {
                    self.dock_focus_window(w.id);
                }
            }
            i if (M_DESK..M_DESK + WORKSPACES as u32).contains(&i) => {
                let n = (i - M_DESK) as usize;
                for w in &wins {
                    if let Some(k) = self.win_by_id(w.id) {
                        self.wins[k].ws = n;
                    }
                }
                let cur = self.workspace;
                self.switch_workspace(cur);
                self.shot_notice = Some((format!("Moved to desktop {}", n + 1), 1.2));
            }
            _ => {}
        }
    }

    /// Write the pinned apps where the shell reads them at start.
    pub fn save_pins(&self) {
        let home = std::env::var("HOME").unwrap_or_default();
        let dir = format!("{home}/.config/memrium");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(format!("{dir}/dock"), self.pinned.join("\n") + "\n");
    }
}

/// The preview's backdrop, cards and thumbnails as window-layer items.
pub fn preview_items<'a>(
    l: &PreviewLayout,
    wins: &'a [crate::state::Win],
    surfaces: &'a HashMap<smithay::reexports::wayland_server::backend::ObjectId, crate::state::SurfNode>,
    dark: bool,
    accent: [f32; 3],
) -> Vec<Item<'a>> {
    let f = l.fade;
    let mut out = Vec::new();
    let (bg, ink): ([f32; 3], [f32; 3]) = if dark { ([0.101, 0.101, 0.101], [0.96, 0.97, 1.0]) } else { ([0.97, 0.97, 0.985], [0.10, 0.12, 0.18]) };
    let pc = |r: [f32; 4]| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    let (cx, cy) = pc(l.popup);
    let mut panels = vec![Panel::solid(cx, cy, l.popup[2], l.popup[3], 16.0, bg, bg).flat().alpha(0.92).shadow(0.34, 30.0, 10.0)];
    for c in &l.cards {
        let (x, y) = pc(c.rect);
        if c.hover {
            panels.push(Panel::solid(x, y, c.rect[2], c.rect[3], 11.0, accent, accent).flat().alpha(0.22));
        }
        let (tx, ty) = pc(c.thumb);
        panels.push(Panel::solid(tx, ty, c.thumb[2], c.thumb[3], 7.0, [0.0; 3], [0.0; 3]).flat().alpha(if dark { 0.45 } else { 0.10 }));
        let (qx, qy) = pc(c.close);
        if c.hover_close {
            panels.push(Panel::solid(qx, qy, c.close[2], c.close[3], 11.0, [0.92, 0.28, 0.28], [0.92, 0.28, 0.28]).flat().alpha(0.95));
        }
        if c.hover || c.hover_close {
            panels.push(Panel::glyph(qx, qy, 12.0, "x", ink, if c.hover_close { 1.0 } else { 0.7 }));
        }
        if c.win.id >= SHELL_FILES {
            panels.push(Panel::glyph(tx, ty, 40.0, "app-window", ink, 0.55));
        }
    }
    for p in &mut panels {
        p.fade_by(f);
    }
    out.push(Item::Frame { panels, bbox: [l.popup[0] - 50.0, l.popup[1] - 50.0, l.popup[0] + l.popup[2] + 50.0, l.popup[1] + l.popup[3] + 70.0], fade: 1.0 });
    for c in &l.cards {
        let Some(w) = wins.iter().find(|w| win_id(w) == Some(c.win.id)) else { continue };
        let Some(root) = w.role.surface() else { continue };
        let Some(n) = surfaces.get(&root.id()) else { continue };
        let Some(tex) = n.tex.as_deref() else { continue };
        let (tw, th) = (tex.size.0 as f32, tex.size.1 as f32);
        if tw < 1.0 || th < 1.0 {
            continue;
        }
        // the whole texture, as large as the box allows, centred
        let s = (c.thumb[2] / tw).min(c.thumb[3] / th);
        let (rw, rh) = (tw * s, th * s);
        let r = [c.thumb[0] + (c.thumb[2] - rw) / 2.0, c.thumb[1] + (c.thumb[3] - rh) / 2.0, rw, rh];
        out.push(Item::Surface { tex, rect: r, clip: Some((r, 5.0)), frosted: false, alpha: f * if c.win.minimized { 0.55 } else { 1.0 }, veil: 1.0 });
    }
    out
}
