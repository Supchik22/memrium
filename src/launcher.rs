//! Application launcher with three looks (chosen by the theme): macOS Figlet, Fluent start
//! menu and Aero start menu. Typing searches; clicking an app launches it.

use crate::apps::{self, AppEntry};
use crate::renderer::{Align, Label, Panel};
use crate::figlet::{self, FileIndex, Item, Target};
use crate::launchpad::{Layout, Slot};
use chebrium::spring::Spring;
use crate::sysinfo;
use crate::theme::{Edge, LauncherKind, Theme};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// Rows of the menu-command search.
const MAX_MENU_ROWS: usize = 8;
const MAX_RESULTS: usize = 7;

pub struct Launcher {
    pub open: bool,
    pub query: String,
    sel: usize,
    /// indices into the app list
    results: Vec<usize>,
    /// Figlet rows: apps, calculator, actions, files, web
    items: Vec<Item>,
    /// the theme uses the Figlet look (selection runs over `items` instead of `results`)
    pub figlet: bool,
    /// the search lists the commands of a program's menu instead (see `open_menu_search`)
    pub menu_mode: bool,
    menu_cmds: Vec<crate::figlet::MenuCmd>,
    menu_program: String,
    pub files: Option<Arc<FileIndex>>,
    seen_index: u64,
    /// Figlet: sliding selection and list height (targets are set while drawing)
    sel_y: Spring,
    list_h: Spring,
    tgt: Cell<[f32; 2]>,
    fresh: Cell<bool>,
    /// appear animation (springs past 1.0 for a little overshoot)
    k: Spring,
    t: f32,
    mouse: [f32; 2],
    /// clickable app rectangles of the last drawn frame
    hits: RefCell<Vec<([f32; 4], usize)>>,
    /// the menu's outer rectangle of the last drawn frame (to tell inside from outside clicks)
    menu_rect: Cell<[f32; 4]>,
    /// the power / shut down button of the last frame (start menus)
    power_rect: Cell<[f32; 4]>,
    /// Launchpad: the look is a full-screen grid (more results fit), the page being shown (and where it slides to),
    /// the page dots of the last frame and how many pages there are
    pub grid: bool,
    /// the kind of launcher this opening asked for (the search, with a Launchpad in the theme), instead of the theme's
    pub forced: Option<LauncherKind>,
    /// where a start menu sits along its bar, and whether the Plasma menu shows favourites as a grid (asked by the start button)
    pub align: MenuAlign,
    pub fav_grid: bool,
    page: Spring,
    page_to: Cell<usize>,
    pages: Cell<usize>,
    dots: RefCell<Vec<[f32; 4]>>,
    /// Launchpad's apps and folders; loaded (or made from the categories) on first use
    layout: Layout,
    layout_ready: bool,
    apps_seen: usize,
    /// the open folder (a slot of `layout`), the last one that was open (it fades out while closing) and how open it is
    folder: Option<usize>,
    folder_last: usize,
    folder_k: Spring,
    folder_pivot: Cell<[f32; 2]>,
    folder_rect: Cell<[f32; 4]>,
    /// the folder's name is being typed
    name_edit: bool,
    /// clickable things of the last Launchpad frame
    pad_hits: RefCell<Vec<([f32; 4], Hit)>>,
    /// where the pointer went down and on what; `dragging` once it has moved far enough
    pressed: Option<(Hit, [f32; 2])>,
    dragging: bool,
    /// the pointer went down while the Launchpad was up (a press that opened it, coming up over it, is not a click on it)
    down: bool,
}

/// Where a start menu sits along its bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MenuAlign {
    /// its edge on the start button's edge (the menu opens where the button is)
    #[default]
    Button,
    /// centred on the start button
    Center,
    /// centred on the screen
    Screen,
}

impl MenuAlign {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => MenuAlign::Center,
            "screen" => MenuAlign::Screen,
            _ => MenuAlign::Button,
        }
    }
}

/// What a Launchpad rectangle stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// an app of the search results
    App(usize),
    /// a slot of the layout: an app or a folder
    Slot(usize),
    /// an app inside the open folder
    InFolder(usize),
    /// the open folder's name
    Title,
}

/// One cell of the Launchpad grid.
#[derive(Clone, Copy)]
enum Entry {
    /// app index, and its slot in the layout (None: a search result)
    App(usize, Option<usize>),
    Folder(usize),
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher {
            open: false,
            query: String::new(),
            sel: 0,
            results: Vec::new(),
            items: Vec::new(),
            figlet: false,
            menu_mode: false,
            menu_cmds: Vec::new(),
            menu_program: String::new(),
            files: None,
            seen_index: 0,
            sel_y: Spring::new(0.0, 620.0, 0.9),
            list_h: Spring::new(0.0, 520.0, 0.95),
            tgt: Cell::new([0.0; 2]),
            fresh: Cell::new(true),
            k: Spring::new(0.0, 300.0, 0.55),
            t: 0.0,
            mouse: [-1000.0, -1000.0],
            hits: RefCell::new(Vec::new()),
            menu_rect: Cell::new([0.0; 4]),
            power_rect: Cell::new([0.0; 4]),
            grid: false,
            forced: None,
            align: MenuAlign::Button,
            fav_grid: false,
            page: Spring::new(0.0, 190.0, 0.92),
            page_to: Cell::new(0),
            pages: Cell::new(1),
            dots: RefCell::new(Vec::new()),
            layout: Layout::default(),
            layout_ready: false,
            apps_seen: 0,
            folder: None,
            folder_last: 0,
            folder_k: Spring::new(0.0, 380.0, 0.8),
            folder_pivot: Cell::new([0.0; 2]),
            folder_rect: Cell::new([0.0; 4]),
            name_edit: false,
            pad_hits: RefCell::new(Vec::new()),
            pressed: None,
            dragging: false,
            down: false,
        }
    }
}

/// What a click on the launcher meant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LauncherClick {
    App(usize),
    /// a Figlet row (index into the rows; fetch with `Launcher::item`)
    Item(usize),
    /// the power button of a start menu: (its rectangle, for placing a menu)
    Power([f32; 4]),
    /// an app dragged out of the grid and let go over the desktop behind it: make a launcher there
    Shortcut(usize),
    Inside,
    Outside,
}

/// Everything the launcher needs to draw one frame.
pub struct DrawCtx<'a> {
    pub screen: [f32; 2],
    pub apps: &'a [AppEntry],
    /// favourite apps (indices), shown first in the start menus
    pub pinned: &'a [usize],
    pub theme: &'a Theme,
    /// the bar the start menu sits on: its edge and rectangle (None = the bottom of the screen)
    pub bar: Option<(Edge, [f32; 4])>,
    /// the start button's rectangle: a start menu opens where it is
    pub anchor: Option<[f32; 4]>,
    pub icon_cell: &'a dyn Fn(&AppEntry) -> Option<u32>,
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

fn mixc2(a: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (1.0 - a[0]) * t, a[1] + (1.0 - a[1]) * t, a[2] + (1.0 - a[2]) * t]
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

impl Launcher {
    pub fn visible(&self) -> bool {
        self.open || self.k.x > 0.02
    }

    /// What is shown now: the kind this opening asked for, else the theme's (`style`).
    pub fn kind(&self, style: LauncherKind) -> LauncherKind {
        self.forced.unwrap_or(style)
    }

    /// Open (or close) the launcher as a given kind: the search is one thing and the grid of apps another, and a theme can
    /// have both. `None` is the theme's own.
    pub fn toggle_as(&mut self, kind: Option<LauncherKind>) {
        if !self.open {
            self.forced = kind;
        }
        self.toggle();
    }

    /// Open (or close) the menu the start button asks for: its kind (`None`: the theme's), where it sits and its favourites view.
    pub fn open_from_start(&mut self, kind: Option<LauncherKind>, align: MenuAlign, grid: bool) {
        let was_open = self.open;
        self.toggle_as(kind);
        if !was_open {
            self.align = align;
            self.fav_grid = grid;
        }
    }

    /// Open the search over the commands of the front program's menu (`program`: its name, for the hint).
    pub fn open_menu_search(&mut self, program: &str, cmds: Vec<crate::figlet::MenuCmd>) {
        if self.open {
            self.close();
        }
        self.forced = Some(LauncherKind::Figlet);
        self.toggle();
        self.menu_mode = true;
        self.menu_cmds = cmds;
        self.menu_program = program.to_string();
        self.items = self.build_items();
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            // (a key opens the menu at the button; the start button asks for more after this)
            self.align = MenuAlign::Button;
            self.fav_grid = false;
            self.menu_mode = false;
            self.menu_cmds.clear();
            self.query.clear();
            self.results.clear();
            self.items.clear();
            self.sel = 0;
            self.page_to.set(0);
            self.page.x = 0.0;
            self.page.v = 0.0;
            self.folder = None;
            self.folder_k.x = 0.0;
            self.name_edit = false;
            self.pressed = None;
            self.dragging = false;
            self.down = false;
            self.k.target = 1.0;
            self.fresh.set(true);
            if let Some(f) = &self.files {
                f.refresh_if_stale();
            }
        }
    }

    pub fn close(&mut self) {
        self.end_edit();
        self.open = false;
        self.k.target = 0.0;
    }

    pub fn set_mouse(&mut self, p: [f32; 2]) {
        self.mouse = p;
        if let Some((_, from)) = self.pressed {
            if (p[0] - from[0]).hypot(p[1] - from[1]) > 6.0 {
                self.dragging = true;
            }
        }
    }

    /// Launchpad: load the layout the first time, and keep it in step with the installed apps.
    pub fn prepare(&mut self, list: &[AppEntry]) {
        if !self.grid || list.is_empty() {
            return;
        }
        if !self.layout_ready {
            self.layout = Layout::load().unwrap_or_else(|| Layout::auto(list));
            self.layout_ready = true;
            self.apps_seen = 0;
        }
        if self.apps_seen != list.len() {
            self.apps_seen = list.len();
            if self.layout.reconcile(list) {
                self.layout.save();
            }
        }
    }

    /// Is a folder's name being typed?
    pub fn editing(&self) -> bool {
        self.name_edit
    }

    /// Finish typing a folder's name (keeps it).
    pub fn end_edit(&mut self) {
        if self.name_edit {
            self.name_edit = false;
            self.layout.save();
        }
    }

    /// Escape: finish the name, else close the folder, else the launcher.
    pub fn escape(&mut self) {
        if self.name_edit {
            self.end_edit();
        } else if self.folder.is_some() {
            self.folder = None;
        } else {
            self.close();
        }
    }

    fn open_folder(&mut self, slot: usize) {
        self.folder = Some(slot);
        self.folder_last = slot;
    }

    fn pad_hit(&self, p: [f32; 2]) -> Option<([f32; 4], Hit)> {
        self.pad_hits.borrow().iter().find(|(r, _)| inside(p, *r)).copied()
    }

    /// Launchpad: the pointer went down.
    pub fn press(&mut self, p: [f32; 2]) {
        let hit = self.pad_hit(p).map(|h| h.1);
        if self.name_edit && hit != Some(Hit::Title) {
            self.end_edit();
        }
        self.pressed = hit.map(|h| (h, p));
        self.dragging = false;
        self.down = true;
    }

    /// Launchpad: the pointer came up. A drag drops what was lifted (an app onto another makes a folder, onto a folder
    /// joins it, between two moves it, out of an open folder takes it out); a click launches, opens a folder or
    /// closes what is open. `over_window`: a file manager window is under the pointer, which also is a place to drop an app
    /// (between the cells) to leave a launcher.
    pub fn release(&mut self, p: [f32; 2], list: &[AppEntry], over_window: bool) -> LauncherClick {
        let (pressed, dragged) = (self.pressed.take(), std::mem::take(&mut self.dragging));
        if !std::mem::take(&mut self.down) {
            return LauncherClick::Inside;
        }
        let Some((hit, _)) = pressed else {
            if self.folder.is_some() {
                if !inside(p, self.folder_rect.get()) {
                    self.end_edit();
                    self.folder = None;
                }
                return LauncherClick::Inside;
            }
            return self.click(p);
        };
        let app_of = |id: &str| list.iter().position(|a| a.id == id);
        if dragged {
            if let (Hit::Slot(s), None) = (hit, self.folder) {
                if let (Some(Slot::App { id }), true) = (self.layout.slots.get(s), self.off_grid(p) || (over_window && self.pad_hit(p).is_none())) {
                    if let Some(i) = app_of(id) {
                        return LauncherClick::Shortcut(i);
                    }
                }
            }
            self.drop_at(hit, p, list);
            return LauncherClick::Inside;
        }
        match hit {
            Hit::App(i) | Hit::InFolder(i) => LauncherClick::App(i),
            Hit::Slot(s) => match self.layout.slots.get(s) {
                Some(Slot::App { id }) => app_of(id).map_or(LauncherClick::Inside, LauncherClick::App),
                Some(Slot::Folder { .. }) => {
                    self.open_folder(s);
                    LauncherClick::Inside
                }
                None => LauncherClick::Inside,
            },
            Hit::Title => {
                self.name_edit = true;
                LauncherClick::Inside
            }
        }
    }

    /// Is `p` clear of the grid (a good way beyond its outermost cells)? Letting go there means "onto the desktop".
    fn off_grid(&self, p: [f32; 2]) -> bool {
        let hits = self.pad_hits.borrow();
        let mut cells = hits.iter().filter(|(_, h)| matches!(h, Hit::Slot(_))).map(|(r, _)| *r);
        let Some(first) = cells.next() else { return false };
        let b = cells.fold(first, |b, r| {
            let (x0, y0) = (b[0].min(r[0]), b[1].min(r[1]));
            let (x1, y1) = ((b[0] + b[2]).max(r[0] + r[2]), (b[1] + b[3]).max(r[1] + r[3]));
            [x0, y0, x1 - x0, y1 - y0]
        });
        let m = 40.0;
        !inside(p, [b[0] - m, b[1] - m, b[2] + 2.0 * m, b[3] + 2.0 * m])
    }

    fn drop_at(&mut self, from: Hit, p: [f32; 2], list: &[AppEntry]) {
        match from {
            Hit::InFolder(i) => {
                let (Some(f), Some(app)) = (self.folder, list.get(i)) else { return };
                if !inside(p, self.folder_rect.get()) {
                    self.layout.take_out(f, &app.id);
                    self.layout.save();
                    self.folder = None;
                }
            }
            Hit::Slot(s) if self.folder.is_none() => {
                let Some((r, Hit::Slot(t))) = self.pad_hit(p) else { return };
                if t == s {
                    return;
                }
                let centre = (p[0] - (r[0] + r[2] / 2.0)).abs() < r[2] * 0.22 && (p[1] - (r[1] + r[3] / 2.0)).abs() < r[3] * 0.3;
                let merged = centre && self.layout.merge(s, t, list).is_some();
                if !merged {
                    self.layout.move_slot(s, if p[0] < r[0] + r[2] / 2.0 { t } else { t + 1 });
                }
                self.layout.save();
            }
            _ => {}
        }
    }

    pub fn update(&mut self, dt: f32, an: &crate::theme::AnimStyle) {
        let [ty, th] = self.tgt.get();
        self.sel_y.target = ty;
        self.list_h.target = th;
        if self.fresh.get() {
            self.sel_y.x = ty;
            self.sel_y.v = 0.0;
            self.list_h.x = th;
            self.list_h.v = 0.0;
            self.fresh.set(false);
        }
        an.step("launcher", &mut self.sel_y, dt);
        an.step("launcher", &mut self.list_h, dt);
        an.step("launcher", &mut self.k, dt);
        self.t += dt;
        self.page.target = self.page_to.get() as f32;
        an.step("launcher", &mut self.page, dt);
        self.folder_k.target = self.folder.is_some() as u8 as f32;
        an.step("launcher", &mut self.folder_k, dt);
        // once it is gone the next opening is the theme's again
        if !self.visible() {
            self.forced = None;
        }
    }

    /// Launchpad: show the next (`1`) or previous (`-1`) page.
    pub fn page_by(&self, d: i32) {
        let last = self.pages.get().saturating_sub(1) as i32;
        self.page_to.set((self.page_to.get() as i32 + d).clamp(0, last) as usize);
    }

    /// Launchpad: the wheel turns pages. True when the launcher took it.
    pub fn scroll(&self, dy: f64) -> bool {
        if !(self.open && self.grid) {
            return false;
        }
        if dy.abs() > 0.5 && self.query.is_empty() {
            self.page_by(if dy > 0.0 { 1 } else { -1 });
        }
        true
    }

    fn refresh(&mut self, list: &[AppEntry]) {
        self.results = apps::search(list, &self.query, if self.grid { 60 } else { MAX_RESULTS }).into_iter().filter_map(|r| list.iter().position(|a| std::ptr::eq(a, r))).collect();
        self.items = self.build_items();
        self.sel = 0;
    }

    fn build_items(&self) -> Vec<Item> {
        if self.menu_mode {
            return figlet::menu_rank(&self.menu_cmds, &self.query, MAX_MENU_ROWS).into_iter().map(|i| figlet::menu_item(&self.menu_cmds[i])).collect();
        }
        let q = self.query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        if let Some(cmd) = q.strip_prefix('>') {
            return if cmd.trim().is_empty() { Vec::new() } else { vec![figlet::run_item(cmd)] };
        }
        let mut v = Vec::new();
        if let Some(ans) = figlet::calc(q) {
            v.push(figlet::calc_item(ans, q));
        }
        for &i in self.results.iter().take(5) {
            v.push(Item { target: Target::App(i), title: String::new(), sub: String::new(), tag: "Application", glyph: "app-window", tint: [0.5; 3], key: String::new(), cell: None });
        }
        v.extend(figlet::actions(q, 3));
        if let Some(f) = &self.files {
            v.extend(f.search(q, 5));
        }
        v.truncate(8);
        v.push(figlet::web_item(q));
        v
    }

    /// Redo the search when the file index has finished a scan since the last one.
    pub fn refresh_if_indexed(&mut self, list: &[AppEntry]) {
        let Some(f) = &self.files else { return };
        let g = f.generation();
        if g != self.seen_index {
            self.seen_index = g;
            if self.open && !self.query.is_empty() {
                let sel = self.sel;
                self.refresh(list);
                self.sel = sel.min(self.items.len().saturating_sub(1));
            }
        }
    }

    /// Replace the query (after picking a calculator answer).
    pub fn set_query(&mut self, text: &str, list: &[AppEntry]) {
        self.query = text.to_string();
        self.refresh(list);
    }

    /// The highlighted Figlet row.
    pub fn accept_item(&self) -> Option<Item> {
        self.items.get(self.sel).cloned()
    }

    pub fn item(&self, i: usize) -> Option<Item> {
        self.items.get(i).cloned()
    }

    pub fn type_char(&mut self, c: char, list: &[AppEntry]) {
        if self.name_edit {
            if let (Some(Slot::Folder { name, .. }), false) = (self.layout.slots.get(self.folder_last), c.is_control()) {
                let mut n = name.clone();
                if n.chars().count() < 28 {
                    n.push(c);
                    self.layout.rename(self.folder_last, &n);
                }
            }
            return;
        }
        if self.folder.is_some() {
            self.folder = None;
        }
        if !c.is_control() {
            self.query.push(c);
            self.refresh(list);
        }
    }

    pub fn backspace(&mut self, list: &[AppEntry]) {
        if self.name_edit {
            if let Some(Slot::Folder { name, .. }) = self.layout.slots.get(self.folder_last) {
                let mut n = name.clone();
                n.pop();
                self.layout.rename(self.folder_last, &n);
            }
            return;
        }
        self.query.pop();
        self.refresh(list);
    }

    pub fn move_sel(&mut self, d: i32) {
        let len = if self.figlet { self.items.len() } else { self.results.len() };
        if len == 0 {
            return;
        }
        let n = len as i32;
        self.sel = (self.sel as i32 + d).rem_euclid(n) as usize;
    }

    /// The highlighted application (index into the app list).
    pub fn accept(&self) -> Option<usize> {
        self.results.get(self.sel).copied()
    }

    /// Apps whose icons should be loaded before drawing (results + the start-menu grids).
    pub fn wanted_icons(&self, pinned: &[usize], list: &[AppEntry]) -> Vec<usize> {
        let mut v: Vec<usize> = self.results.clone();
        v.extend(self.items.iter().filter_map(|i| if let Target::App(a) = i.target { Some(a) } else { None }));
        v.extend(pinned.iter().copied().take(18));
        if self.grid {
            // what the first pages show (a folder shows its first nine), and all of the open folder
            let at = |id: &String| list.iter().position(|a| &a.id == id);
            for (n, s) in self.layout.slots.iter().enumerate() {
                match s {
                    Slot::App { id } => v.extend(at(id)),
                    Slot::Folder { ids, .. } => v.extend(ids.iter().take(if self.folder_last == n { usize::MAX } else { 9 }).filter_map(at)),
                }
                if v.len() > 120 {
                    break;
                }
            }
        } else {
            v.extend((0..list.len()).take(24));
        }
        v
    }

    /// Is the pointer over an app of the last drawn frame?
    pub fn over_app(&self, p: [f32; 2]) -> bool {
        self.hits.borrow().iter().any(|(r, _)| inside(p, *r))
    }

    /// Classify a click (logical px) against the last drawn frame.
    pub fn click(&self, p: [f32; 2]) -> LauncherClick {
        if let Some((_, i)) = self.hits.borrow().iter().find(|(r, _)| inside(p, *r)) {
            return if self.figlet { LauncherClick::Item(*i) } else { LauncherClick::App(*i) };
        }
        for (i, d) in self.dots.borrow().iter().enumerate() {
            if inside(p, *d) {
                self.page_to.set(i);
                return LauncherClick::Inside;
            }
        }
        if inside(p, self.power_rect.get()) {
            return LauncherClick::Power(self.power_rect.get());
        }
        if inside(p, self.menu_rect.get()) {
            LauncherClick::Inside
        } else {
            LauncherClick::Outside
        }
    }

    pub fn draw(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        self.hits.borrow_mut().clear();
        self.pad_hits.borrow_mut().clear();
        self.power_rect.set([0.0; 4]);
        self.dots.borrow_mut().clear();
        if !self.visible() {
            return (Vec::new(), Vec::new());
        }
        let (mut p, mut l) = match self.kind(ctx.theme.launcher.style) {
            LauncherKind::Figlet => self.draw_spotlight(ctx),
            LauncherKind::StartMenuWin11 => self.draw_win11(ctx),
            LauncherKind::StartMenuWin7 => self.draw_win7(ctx),
            LauncherKind::Plasma => self.draw_plasma(ctx),
            LauncherKind::Launchpad => self.draw_launchpad(ctx),
        };
        // one fade for the whole launcher: glass, fills, glyphs and text all follow the open animation
        let f = self.k.x.clamp(0.0, 1.0);
        if f < 0.999 {
            p.iter_mut().for_each(|x| x.fade_by(f));
            l.iter_mut().for_each(|x| x.opacity *= f);
        }
        (p, l)
    }

    // ------------------------------------------------------------ Figlet

    fn draw_spotlight(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        const W: f32 = 640.0;
        const BAR_H: f32 = 58.0;
        const ROW_H: f32 = 52.0;
        const INK: [f32; 3] = [0.10, 0.12, 0.19];
        let (screen, list) = (ctx.screen, ctx.apps);
        let mut p = Vec::new();
        let mut l = Vec::new();
        let k = (0.94 + 0.06 * self.k.x).clamp(0.9, 1.05);
        let top = (screen[1] * 0.18).max(60.0);
        let pivot = [screen[0] / 2.0, top + BAR_H / 2.0];
        let map = |x: f32, y: f32| [pivot[0] + (x - pivot[0]) * k, pivot[1] + (y - pivot[1]) * k];
        let left = screen[0] / 2.0 - W / 2.0;
        let listed = !self.query.is_empty() || self.menu_mode;
        self.menu_rect.set([left, top, W, BAR_H + 10.0 + if !listed { 0.0 } else { self.items.len().max(1) as f32 * ROW_H + 16.0 }]);

        let c = map(screen[0] / 2.0, top + BAR_H / 2.0);
        let mut bar = Panel::glass(c[0], c[1], W * k, BAR_H * k, BAR_H / 2.0 * k).tinted([0.92, 0.95, 1.0, 0.34]).shadow(0.30, 44.0, 18.0);
        bar.fx[0] = 14.0;
        p.push(bar);
        let g = map(left + 34.0, top + BAR_H / 2.0);
        p.push(Panel::glyph(g[0], g[1], 24.0 * k, "search", INK, 0.7));

        let text_x = left + 62.0;
        let baseline = top + BAR_H / 2.0 + 7.5;
        let tp = map(text_x, baseline);
        if self.query.is_empty() {
            let hint = if self.menu_mode { crate::i18n::trf("Search {} menus", &[self.menu_program.clone()]) } else { "Figlet Search".to_string() };
            l.push(Label::new(hint, tp[0], tp[1], 22.0, false, INK, 0.38, Align::Left).verbatim());
        } else {
            l.push(Label::new(self.query.clone(), tp[0], tp[1], 22.0, false, INK, 0.95, Align::Left));
            if (self.t * 1.6).fract() < 0.6 {
                let cx = text_x + self.query.chars().count() as f32 * 11.6 + 2.0;
                let cc = map(cx, top + BAR_H / 2.0);
                p.push(Panel::solid(cc[0], cc[1], 2.0 * k, 26.0 * k, 1.0, INK, INK).flat().alpha(0.8));
            }
        }

        if listed {
            let rows = self.items.len().max(1) as f32;
            let rh_target = rows * ROW_H + 16.0;
            let fresh = self.fresh.get();
            let rh = if fresh { rh_target } else { self.list_h.x.clamp(60.0, 700.0) };
            let sel_cy = top + BAR_H + 10.0 + 8.0 + self.sel as f32 * ROW_H + ROW_H / 2.0;
            self.tgt.set([sel_cy, rh_target]);
            let rtop = top + BAR_H + 10.0;
            let rc = map(screen[0] / 2.0, rtop + rh / 2.0);
            let mut panel = Panel::glass(rc[0], rc[1], W * k, rh * k, 26.0 * k).tinted([0.92, 0.95, 1.0, 0.34]).shadow(0.28, 40.0, 16.0);
            panel.fx[0] = 14.0;
            p.push(panel);
            if self.items.is_empty() {
                let t = map(screen[0] / 2.0, rtop + rh / 2.0 + 5.0);
                l.push(Label::new("No results", t[0], t[1], 14.0, false, INK, 0.5, Align::Center));
            }
            if !self.items.is_empty() {
                let hy = if fresh { sel_cy } else { self.sel_y.x };
                let hc = map(screen[0] / 2.0, hy);
                p.push(Panel::glass(hc[0], hc[1], (W - 16.0) * k, (ROW_H - 4.0) * k, 16.0 * k).tinted([0.15, 0.45, 1.0, 0.42]).shadow(0.0, 0.0, 0.0));
            }
            for (row, it) in self.items.iter().enumerate() {
                let cy = rtop + 8.0 + row as f32 * ROW_H + ROW_H / 2.0;
                self.hits.borrow_mut().push(([left + 8.0, cy - ROW_H / 2.0, W - 16.0, ROW_H], row));
                let hy = if fresh { sel_cy } else { self.sel_y.x };
                // how much of the sliding highlight sits on this row: the ink fades with it
                let cover = (1.0 - (hy - cy).abs() / ROW_H).clamp(0.0, 1.0);
                let cover = cover * cover * (3.0 - 2.0 * cover);
                let mixc = |c: [f32; 3]| [INK[0] + (c[0] - INK[0]) * cover, INK[1] + (c[1] - INK[1]) * cover, INK[2] + (c[2] - INK[2]) * cover];
                let ic = map(left + 38.0, cy);
                let (title, sub, tag) = match it.target {
                    Target::App(ai) => {
                        let app = &list[ai];
                        push_icon(&mut p, ctx, app, ic, 36.0 * k, 8.0 * k);
                        let sub = if app.generic_name.is_empty() { String::new() } else { app.generic_name.clone() };
                        (app.name.clone(), sub, it.tag)
                    }
                    _ => {
                        if let Some(cell) = it.cell {
                            // a menu entry's own icon
                            p.push(Panel::app_icon(ic[0], ic[1], 26.0 * k, cell, 1.0));
                        } else {
                            p.push(Panel::solid(ic[0], ic[1], 34.0 * k, 34.0 * k, 9.0 * k, it.tint, it.tint).flat().alpha(0.22));
                            p.push(Panel::glyph(ic[0], ic[1], 20.0 * k, it.glyph, mixc2(it.tint, cover), 0.95));
                        }
                        (it.title.clone(), it.sub.clone(), it.tag)
                    }
                };
                let col = mixc([1.0; 3]);
                let two_line = !sub.is_empty();
                let n = map(left + 68.0, if two_line { cy - 2.0 } else { cy + 5.0 });
                l.push(Label::new(trunc(&title, 46), n[0], n[1], 15.0, true, col, 0.97, Align::Left));
                if two_line {
                    let s = map(left + 68.0, cy + 14.0);
                    l.push(Label::new(trunc(&sub, 60), s[0], s[1], 11.5, false, col, 0.55 + 0.25 * cover, Align::Left));
                }
                let t = map(left + W - 22.0, cy + 4.5);
                if it.key.is_empty() {
                    l.push(Label::new(tag, t[0], t[1], 11.5, false, col, 0.45 + 0.35 * cover, Align::Right));
                } else {
                    l.push(Label::new(crate::keys::display(&it.key), t[0], t[1], 12.0, false, col, 0.6 + 0.3 * cover, Align::Right).verbatim());
                }
            }
        }
        (p, l)
    }

    // ------------------------------------------------------------ Fluent start menu

    /// (edge, y of the bar's side that faces the screen): the menu opens away from it.
    fn bar_side(ctx: &DrawCtx) -> (Edge, f32) {
        match ctx.bar {
            Some((Edge::Top, r)) => (Edge::Top, r[1] + r[3]),
            Some((Edge::Bottom, r)) => (Edge::Bottom, r[1]),
            // a bar on the side of the screen: the menu opens from the bottom, as with no bar
            Some((Edge::Left | Edge::Right, _)) | None => (Edge::Bottom, ctx.screen[1]),
        }
    }

    /// Top left corner of a start menu `w` x `h` with `gap` to its bar: it opens away from the bar, where the start
    /// button is (or centred, as asked); on a bar at the side of the screen it opens beside it.
    fn place(&self, ctx: &DrawCtx, w: f32, h: f32, gap: f32) -> (f32, f32) {
        let m = 8.0;
        let [sw, sh] = ctx.screen;
        let clamp = |v: f32, len: f32, total: f32| v.max(m).min((total - len - m).max(m));
        match ctx.bar {
            Some((edge @ (Edge::Left | Edge::Right), r)) => {
                let x = if edge == Edge::Left { r[0] + r[2] + gap } else { r[0] - gap - w };
                let y = match (self.align, ctx.anchor) {
                    (MenuAlign::Screen, _) | (_, None) => (sh - h) / 2.0,
                    (MenuAlign::Center, Some(a)) => a[1] + a[3] / 2.0 - h / 2.0,
                    // the menu grows from the button towards the middle of the screen
                    (MenuAlign::Button, Some(a)) => if a[1] + a[3] / 2.0 > sh / 2.0 { a[1] + a[3] - h } else { a[1] },
                };
                (clamp(x, w, sw), clamp(y, h, sh))
            }
            _ => {
                let (edge, bar_y) = Self::bar_side(ctx);
                let y = if edge == Edge::Top { bar_y + gap } else { bar_y - gap - h };
                let x = match (self.align, ctx.anchor) {
                    (MenuAlign::Screen, _) | (_, None) => (sw - w) / 2.0,
                    (MenuAlign::Center, Some(a)) => a[0] + a[2] / 2.0 - w / 2.0,
                    (MenuAlign::Button, Some(a)) => a[0],
                };
                (clamp(x, w, sw), y)
            }
        }
    }

    fn draw_win11(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        const W: f32 = 640.0;
        const H: f32 = 690.0;
        let theme = ctx.theme;
        let ink: [f32; 3] = if theme.dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.17] };
        let faint = if theme.dark { [1.0, 1.0, 1.0, 0.10] } else { [1.0, 1.0, 1.0, 0.62] };
        let list = ctx.apps;
        let mut p = Vec::new();
        let mut l = Vec::new();

        let k = (0.96 + 0.04 * self.k.x).clamp(0.9, 1.04);
        let (x, y) = self.place(ctx, W, H, 12.0);
        let pivot = [x + W / 2.0, y + H];
        let map = |px: f32, py: f32| [pivot[0] + (px - pivot[0]) * k, pivot[1] + (py - pivot[1]) * k];
        self.menu_rect.set([x, y, W, H]);

        let c = map(x + W / 2.0, y + H / 2.0);
        let mut panel = theme.glass.panel(c[0], c[1], W * k, H * k, 12.0 * k);
        panel.fx[0] = panel.fx[0].max(26.0);
        p.push(panel);

        // search box
        let sb = map(x + W / 2.0, y + 42.0);
        p.push(Panel::solid(sb[0], sb[1], (W - 48.0) * k, 40.0 * k, 20.0 * k, [faint[0], faint[1], faint[2]], [faint[0], faint[1], faint[2]]).flat().alpha(faint[3]));
        let sg = map(x + 46.0, y + 42.0);
        p.push(Panel::glyph(sg[0], sg[1], 16.0 * k, "search", ink, 0.65));
        let st = map(x + 66.0, y + 47.0);
        if self.query.is_empty() {
            l.push(Label::new("Type here to search", st[0], st[1], 13.5, false, ink, 0.55, Align::Left));
        } else {
            l.push(Label::new(self.query.clone(), st[0], st[1], 14.0, false, ink, 0.95, Align::Left));
            if (self.t * 1.6).fract() < 0.6 {
                let cx = x + 66.0 + self.query.chars().count() as f32 * 7.4 + 2.0;
                let cc = map(cx, y + 42.0);
                p.push(Panel::solid(cc[0], cc[1], 1.5 * k, 18.0 * k, 0.7, ink, ink).flat().alpha(0.8));
            }
        }

        let grid_icon = |p: &mut Vec<Panel>, app: &AppEntry, cx: f32, cy: f32, size: f32| {
            let ic = map(cx, cy);
            push_icon(p, ctx, app, ic, size * k, size * 0.2 * k);
        };
        let hover_bg = |p: &mut Vec<Panel>, r: [f32; 4]| {
            if inside(self.mouse, r) {
                let c = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                p.push(Panel::solid(c[0], c[1], r[2] * k, r[3] * k, 6.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(if theme.dark { 0.12 } else { 0.55 }));
            }
        };

        if self.query.is_empty() {
            // Pinned
            let h = map(x + 36.0, y + 100.0);
            l.push(Label::new("Pinned", h[0], h[1], 14.0, true, ink, 0.95, Align::Left));
            let all = [x + W - 36.0 - 86.0, y + 84.0, 86.0, 28.0];
            let ac = map(all[0] + all[2] / 2.0, all[1] + all[3] / 2.0);
            p.push(Panel::solid(ac[0], ac[1], all[2] * k, all[3] * k, 6.0 * k, [faint[0], faint[1], faint[2]], [faint[0], faint[1], faint[2]]).flat().alpha(faint[3]));
            let at = map(all[0] + all[2] / 2.0, all[1] + 19.0);
            l.push(Label::new("All apps  ›", at[0], at[1], 12.0, false, ink, 0.9, Align::Center));

            let pinned: Vec<usize> = if ctx.pinned.is_empty() { (0..list.len()).take(18).collect() } else { ctx.pinned.iter().copied().take(18).collect() };
            let cw = (W - 48.0) / 6.0;
            for (i, &ai) in pinned.iter().enumerate() {
                let (col, row) = ((i % 6) as f32, (i / 6) as f32);
                let r = [x + 24.0 + col * cw, y + 120.0 + row * 88.0, cw, 84.0];
                self.hits.borrow_mut().push((r, ai));
                hover_bg(&mut p, r);
                grid_icon(&mut p, &list[ai], r[0] + cw / 2.0, r[1] + 32.0, 36.0);
                let t = map(r[0] + cw / 2.0, r[1] + 68.0);
                l.push(Label::new(trunc(&list[ai].name, 13), t[0], t[1], 11.5, false, ink, 0.95, Align::Center));
            }

            // Recommended
            let rh = map(x + 36.0, y + 410.0);
            l.push(Label::new("Recommended", rh[0], rh[1], 14.0, true, ink, 0.95, Align::Left));
            let rest: Vec<usize> = (0..list.len()).filter(|i| !pinned.contains(i)).take(6).collect();
            for (i, &ai) in rest.iter().enumerate() {
                let (col, row) = ((i % 2) as f32, (i / 2) as f32);
                let r = [x + 28.0 + col * 292.0, y + 428.0 + row * 58.0, 284.0, 54.0];
                self.hits.borrow_mut().push((r, ai));
                hover_bg(&mut p, r);
                grid_icon(&mut p, &list[ai], r[0] + 28.0, r[1] + 27.0, 30.0);
                let n = map(r[0] + 56.0, r[1] + 24.0);
                l.push(Label::new(trunc(&list[ai].name, 26), n[0], n[1], 12.5, false, ink, 0.95, Align::Left));
                let s = map(r[0] + 56.0, r[1] + 40.0);
                l.push(Label::new("Recently added", s[0], s[1], 11.0, false, ink, 0.55, Align::Left));
            }
        } else {
            let h = map(x + 36.0, y + 100.0);
            l.push(Label::new("Best match", h[0], h[1], 14.0, true, ink, 0.95, Align::Left));
            if self.results.is_empty() {
                let t = map(x + W / 2.0, y + 160.0);
                l.push(Label::new("No results", t[0], t[1], 13.0, false, ink, 0.55, Align::Center));
            }
            for (row, &ai) in self.results.iter().enumerate() {
                let r = [x + 24.0, y + 116.0 + row as f32 * 60.0, W - 48.0, 56.0];
                self.hits.borrow_mut().push((r, ai));
                if row == self.sel {
                    let c = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                    p.push(Panel::solid(c[0], c[1], r[2] * k, r[3] * k, 6.0 * k, theme.accent, theme.accent).flat().alpha(0.22));
                } else {
                    hover_bg(&mut p, r);
                }
                grid_icon(&mut p, &list[ai], r[0] + 30.0, r[1] + 28.0, 32.0);
                let n = map(r[0] + 60.0, r[1] + 25.0);
                l.push(Label::new(list[ai].name.clone(), n[0], n[1], 13.5, false, ink, 0.97, Align::Left));
                let s = map(r[0] + 60.0, r[1] + 43.0);
                let sub = if list[ai].generic_name.is_empty() { "App".to_string() } else { list[ai].generic_name.clone() };
                l.push(Label::new(sub, s[0], s[1], 11.0, false, ink, 0.55, Align::Left));
            }
        }

        // bottom bar: user + power
        let by = y + H - 62.0;
        let hl = map(x + W / 2.0, by);
        p.push(Panel::solid(hl[0], hl[1], (W - 48.0) * k, 1.0, 0.5, ink, ink).flat().alpha(0.10));
        let av = map(x + 48.0, by + 32.0);
        p.push(Panel::solid(av[0], av[1], 32.0 * k, 32.0 * k, 16.0 * k, theme.accent, theme.accent).flat());
        p.push(Panel::glyph(av[0], av[1], 18.0 * k, "user", [1.0; 3], 0.95));
        let un = map(x + 72.0, by + 37.0);
        l.push(Label::new(sysinfo::user_name(), un[0], un[1], 13.0, false, ink, 0.95, Align::Left));
        let pw = [x + W - 70.0, by + 12.0, 40.0, 40.0];
        self.power_rect.set(pw);
        hover_bg(&mut p, pw);
        let pc = map(pw[0] + 20.0, pw[1] + 20.0);
        p.push(Panel::glyph(pc[0], pc[1], 18.0 * k, "power", ink, 0.85));
        (p, l)
    }

    // ------------------------------------------------------------ Plasma

    /// The Plasma "Kickoff" look: who is logged in and a search field on top, favourites as a list (or a grid), and the
    /// leave button at the bottom. Search results replace the favourites.
    fn draw_plasma(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        const W: f32 = 600.0;
        const H: f32 = 640.0;
        let theme = ctx.theme;
        let ink: [f32; 3] = if theme.dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.17] };
        let faint = if theme.dark { [1.0, 1.0, 1.0, 0.10] } else { [1.0, 1.0, 1.0, 0.62] };
        let list = ctx.apps;
        let mut p = Vec::new();
        let mut l = Vec::new();

        let k = (0.96 + 0.04 * self.k.x).clamp(0.9, 1.04);
        let (x, y) = self.place(ctx, W, H, 8.0);
        let (edge, _) = Self::bar_side(ctx);
        // it grows out of the bar it sits on
        let pivot = [x + W / 2.0, if edge == Edge::Top { y } else { y + H }];
        let map = |px: f32, py: f32| [pivot[0] + (px - pivot[0]) * k, pivot[1] + (py - pivot[1]) * k];
        self.menu_rect.set([x, y, W, H]);

        let c = map(x + W / 2.0, y + H / 2.0);
        let mut panel = theme.glass.panel(c[0], c[1], W * k, H * k, 10.0 * k);
        panel.fx[0] = panel.fx[0].max(24.0);
        p.push(panel);

        let grid_icon = |p: &mut Vec<Panel>, app: &AppEntry, cx: f32, cy: f32, size: f32| {
            let ic = map(cx, cy);
            push_icon(p, ctx, app, ic, size * k, size * 0.2 * k);
        };
        let hover_bg = |p: &mut Vec<Panel>, r: [f32; 4]| {
            if inside(self.mouse, r) {
                let c = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                p.push(Panel::solid(c[0], c[1], r[2] * k, r[3] * k, 5.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(if theme.dark { 0.12 } else { 0.55 }));
            }
        };

        // header: the user, and the search field
        let av = map(x + 44.0, y + 42.0);
        p.push(Panel::solid(av[0], av[1], 44.0 * k, 44.0 * k, 22.0 * k, theme.accent, theme.accent).flat());
        p.push(Panel::glyph(av[0], av[1], 24.0 * k, "user", [1.0; 3], 0.95));
        let un = map(x + 74.0, y + 47.0);
        l.push(Label::new(sysinfo::user_name(), un[0], un[1], 15.0, true, ink, 0.95, Align::Left));
        let sf = [x + W - 24.0 - 250.0, y + 22.0, 250.0, 38.0];
        let sc = map(sf[0] + sf[2] / 2.0, sf[1] + sf[3] / 2.0);
        p.push(Panel::solid(sc[0], sc[1], sf[2] * k, sf[3] * k, 6.0 * k, [faint[0], faint[1], faint[2]], [faint[0], faint[1], faint[2]]).flat().alpha(faint[3]));
        let sg = map(sf[0] + 20.0, sf[1] + sf[3] / 2.0);
        p.push(Panel::glyph(sg[0], sg[1], 15.0 * k, "search", ink, 0.65));
        let st = map(sf[0] + 38.0, sf[1] + sf[3] / 2.0 + 5.0);
        if self.query.is_empty() {
            l.push(Label::new("Search…", st[0], st[1], 13.5, false, ink, 0.55, Align::Left));
        } else {
            l.push(Label::new(trunc(&self.query, 24), st[0], st[1], 13.5, false, ink, 0.95, Align::Left));
            if (self.t * 1.6).fract() < 0.6 {
                let cx = sf[0] + 38.0 + self.query.chars().count().min(24) as f32 * 7.2 + 2.0;
                let cc = map(cx, sf[1] + sf[3] / 2.0);
                p.push(Panel::solid(cc[0], cc[1], 1.5 * k, 18.0 * k, 0.7, ink, ink).flat().alpha(0.8));
            }
        }
        let hl = map(x + W / 2.0, y + 82.0);
        p.push(Panel::solid(hl[0], hl[1], (W - 32.0) * k, 1.0, 0.5, ink, ink).flat().alpha(0.10));

        // body
        let body_top = y + 94.0;
        let body_bottom = y + H - 70.0;
        if self.query.is_empty() {
            let h = map(x + 28.0, body_top + 8.0);
            l.push(Label::new("Favorites", h[0], h[1], 12.5, true, theme.accent, 0.95, Align::Left));
            let favs: Vec<usize> = if ctx.pinned.is_empty() { (0..list.len()).take(16).collect() } else { ctx.pinned.iter().copied().take(16).collect() };
            if self.fav_grid {
                let cols = 5usize;
                let cw = (W - 48.0) / cols as f32;
                for (i, &ai) in favs.iter().enumerate() {
                    let (col, row) = ((i % cols) as f32, (i / cols) as f32);
                    let r = [x + 24.0 + col * cw, body_top + 22.0 + row * 100.0, cw, 94.0];
                    if r[1] + r[3] > body_bottom {
                        break;
                    }
                    self.hits.borrow_mut().push((r, ai));
                    hover_bg(&mut p, r);
                    grid_icon(&mut p, &list[ai], r[0] + cw / 2.0, r[1] + 34.0, 44.0);
                    let t = map(r[0] + cw / 2.0, r[1] + 76.0);
                    l.push(Label::new(trunc(&list[ai].name, 14), t[0], t[1], 11.5, false, ink, 0.95, Align::Center));
                }
            } else {
                let rows = (((body_bottom - body_top - 24.0) / 50.0) as usize).max(1);
                for (i, &ai) in favs.iter().take(rows).enumerate() {
                    let r = [x + 16.0, body_top + 24.0 + i as f32 * 50.0, W - 32.0, 48.0];
                    self.hits.borrow_mut().push((r, ai));
                    hover_bg(&mut p, r);
                    grid_icon(&mut p, &list[ai], r[0] + 28.0, r[1] + 24.0, 32.0);
                    let n = map(r[0] + 58.0, r[1] + 21.0);
                    l.push(Label::new(trunc(&list[ai].name, 40), n[0], n[1], 13.5, false, ink, 0.97, Align::Left));
                    if !list[ai].generic_name.is_empty() {
                        let g = map(r[0] + 58.0, r[1] + 38.0);
                        l.push(Label::new(trunc(&list[ai].generic_name, 48), g[0], g[1], 11.0, false, ink, 0.55, Align::Left));
                    }
                }
            }
        } else {
            let h = map(x + 28.0, body_top + 8.0);
            l.push(Label::new("Results", h[0], h[1], 12.5, true, theme.accent, 0.95, Align::Left));
            if self.results.is_empty() {
                let t = map(x + W / 2.0, body_top + 80.0);
                l.push(Label::new("No matches", t[0], t[1], 13.0, false, ink, 0.55, Align::Center));
            }
            let rows = (((body_bottom - body_top - 24.0) / 50.0) as usize).max(1);
            for (row, &ai) in self.results.iter().take(rows).enumerate() {
                let r = [x + 16.0, body_top + 24.0 + row as f32 * 50.0, W - 32.0, 48.0];
                self.hits.borrow_mut().push((r, ai));
                if row == self.sel {
                    let c = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                    p.push(Panel::solid(c[0], c[1], r[2] * k, r[3] * k, 5.0 * k, theme.accent, theme.accent).flat().alpha(0.22));
                } else {
                    hover_bg(&mut p, r);
                }
                grid_icon(&mut p, &list[ai], r[0] + 28.0, r[1] + 24.0, 32.0);
                let n = map(r[0] + 58.0, r[1] + 21.0);
                l.push(Label::new(trunc(&list[ai].name, 40), n[0], n[1], 13.5, false, ink, 0.97, Align::Left));
                let sub = if list[ai].generic_name.is_empty() { "Application".to_string() } else { list[ai].generic_name.clone() };
                let g = map(r[0] + 58.0, r[1] + 38.0);
                l.push(Label::new(trunc(&sub, 48), g[0], g[1], 11.0, false, ink, 0.55, Align::Left));
            }
        }

        // footer: leave
        let fy = y + H - 62.0;
        let fl = map(x + W / 2.0, fy);
        p.push(Panel::solid(fl[0], fl[1], (W - 32.0) * k, 1.0, 0.5, ink, ink).flat().alpha(0.10));
        let pw = [x + W - 24.0 - 96.0, fy + 10.0, 96.0, 40.0];
        self.power_rect.set(pw);
        hover_bg(&mut p, pw);
        let pc = map(pw[0] + 22.0, pw[1] + 20.0);
        p.push(Panel::glyph(pc[0], pc[1], 17.0 * k, "power", ink, 0.85));
        let pt = map(pw[0] + 40.0, pw[1] + 25.0);
        l.push(Label::new("Leave", pt[0], pt[1], 13.0, false, ink, 0.9, Align::Left));
        (p, l)
    }

    // ------------------------------------------------------------ Launchpad

    fn draw_launchpad(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        let (screen, list) = (ctx.screen, ctx.apps);
        let (sw, sh) = (screen[0], screen[1]);
        let mut p = Vec::new();
        let mut l = Vec::new();
        let ink = [1.0, 1.0, 1.0];
        // the whole screen zooms a little as it opens
        let k = (0.94 + 0.06 * self.k.x).clamp(0.9, 1.04);
        let pivot = [sw / 2.0, sh / 2.0];
        let map = |x: f32, y: f32| [pivot[0] + (x - pivot[0]) * k, pivot[1] + (y - pivot[1]) * k];

        let mut bg = Panel::glass(sw / 2.0, sh / 2.0, sw, sh, 0.0).tinted([0.03, 0.04, 0.09, 0.40]);
        bg.fx[0] = 34.0;
        bg.params[2] = 0.0;
        bg.params[3] = 0.0;
        p.push(bg);

        // search pill
        let sb = [sw / 2.0 - 150.0, 58.0, 300.0, 36.0];
        self.menu_rect.set(sb);
        let c = map(sb[0] + sb[2] / 2.0, sb[1] + sb[3] / 2.0);
        p.push(Panel::solid(c[0], c[1], sb[2] * k, sb[3] * k, 18.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(0.16));
        let g = map(sb[0] + 20.0, sb[1] + 18.0);
        p.push(Panel::glyph(g[0], g[1], 15.0 * k, "search", ink, 0.75));
        let tx = map(sb[0] + 40.0, sb[1] + 23.0);
        if self.query.is_empty() {
            l.push(Label::new("Search", tx[0], tx[1], 14.0, false, ink, 0.6, Align::Left));
        } else {
            l.push(Label::new(self.query.clone(), tx[0], tx[1], 14.5, false, ink, 0.97, Align::Left));
            if (self.t * 1.6).fract() < 0.6 {
                let cx = map(sb[0] + 40.0 + self.query.chars().count() as f32 * 7.6 + 2.0, sb[1] + 18.0);
                p.push(Panel::solid(cx[0], cx[1], 1.5 * k, 18.0 * k, 0.7, ink, ink).flat().alpha(0.85));
            }
        }

        // what is shown: the layout's apps and folders, or the search results
        let search = !self.query.is_empty();
        let entries: Vec<Entry> = if search {
            self.results.iter().map(|&i| Entry::App(i, None)).collect()
        } else {
            let at: std::collections::HashMap<&str, usize> = list.iter().enumerate().map(|(i, a)| (a.id.as_str(), i)).collect();
            self.layout
                .slots
                .iter()
                .enumerate()
                .filter_map(|(s, slot)| match slot {
                    Slot::App { id } => at.get(id.as_str()).map(|&i| Entry::App(i, Some(s))),
                    Slot::Folder { .. } => Some(Entry::Folder(s)),
                })
                .collect()
        };
        let cols = if sw >= 1500.0 { 8 } else if sw >= 1100.0 { 7 } else if sw >= 760.0 { 5 } else { 3 };
        let rows = if sh >= 900.0 { 5 } else if sh >= 640.0 { 4 } else { 3 };
        let per = cols * rows;
        let pages = entries.len().div_ceil(per).max(1);
        self.pages.set(pages);
        if self.page_to.get() >= pages {
            self.page_to.set(pages - 1);
        }
        let area = [sw * 0.08, 128.0, sw * 0.84, (sh - 128.0 - 96.0).max(200.0)];
        let (cw, ch) = (area[2] / cols as f32, area[3] / rows as f32);
        let icon = (cw * 0.62).min(ch * 0.62).clamp(40.0, 92.0);
        let pos = self.page.x;
        // while a folder is open the grid behind it is only to look at
        let live = self.folder.is_none();
        let lifted = if self.dragging { self.pressed.map(|(h, _)| h) } else { None };
        let mut hover_drop: Option<usize> = None;
        let grid_labels = l.len();
        for pg in (pos.floor().max(0.0) as usize)..=((pos.ceil().max(0.0)) as usize).min(pages - 1) {
            let shift = (pg as f32 - pos) * sw;
            for slot in 0..per {
                let Some(&entry) = entries.get(pg * per + slot) else { break };
                let (col, row) = ((slot % cols) as f32, (slot / cols) as f32);
                let r = [area[0] + col * cw + shift, area[1] + row * ch, cw, ch];
                if r[0] + r[2] < 0.0 || r[0] > sw {
                    continue;
                }
                let here = (pos - pg as f32).abs() < 0.5;
                let my_slot = match entry {
                    Entry::App(_, s) => s,
                    Entry::Folder(s) => Some(s),
                };
                if here && live {
                    let hit = match entry {
                        Entry::App(i, None) => Hit::App(i),
                        _ => Hit::Slot(my_slot.unwrap_or(0)),
                    };
                    self.pad_hits.borrow_mut().push((r, hit));
                }
                let mine = pg * per + slot;
                let hot = live && here && inside(self.mouse, r);
                let selected = search && mine == self.sel;
                let is_lifted = lifted.is_some() && lifted == my_slot.map(Hit::Slot);
                // dropping an app on this one (the middle of the cell) would make a folder or fill one
                let drop_here = hot && !is_lifted && matches!(lifted, Some(Hit::Slot(_))) && (self.mouse[0] - (r[0] + r[2] / 2.0)).abs() < r[2] * 0.22 && (self.mouse[1] - (r[1] + r[3] / 2.0)).abs() < r[3] * 0.3;
                if drop_here {
                    hover_drop = my_slot;
                }
                if (hot && lifted.is_none()) || selected || drop_here {
                    let rc = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 - 4.0);
                    p.push(Panel::solid(rc[0], rc[1], (cw - 12.0) * k, (ch - 10.0) * k, 16.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(if selected || drop_here { 0.26 } else { 0.13 }));
                }
                if is_lifted {
                    continue;
                }
                let ic = map(r[0] + cw / 2.0, r[1] + ch / 2.0 - 14.0);
                let t = map(r[0] + cw / 2.0, r[1] + ch / 2.0 + icon / 2.0 - 2.0);
                match entry {
                    Entry::App(ai, _) => {
                        push_icon(&mut p, ctx, &list[ai], ic, icon * k, icon * 0.22 * k);
                        l.push(Label::new(trunc(&list[ai].name, (cw / 7.2) as usize), t[0], t[1], 12.5, false, ink, 0.96, Align::Center));
                    }
                    Entry::Folder(s) => {
                        if self.folder == Some(s) || (self.folder_k.x > 0.02 && self.folder_last == s) {
                            self.folder_pivot.set([r[0] + cw / 2.0, r[1] + ch / 2.0 - 14.0]);
                        }
                        let Some(Slot::Folder { name, ids }) = self.layout.slots.get(s) else { continue };
                        self.folder_tile(&mut p, ctx, ids, ic, icon * k);
                        l.push(Label::new(trunc(name, (cw / 7.2) as usize), t[0], t[1], 12.5, false, ink, 0.96, Align::Center).verbatim());
                    }
                }
            }
        }
        if entries.is_empty() {
            let t = map(sw / 2.0, sh / 2.0);
            l.push(Label::new("No results", t[0], t[1], 15.0, false, ink, 0.6, Align::Center));
        }
        // page dots
        if pages > 1 {
            let y = sh - 56.0;
            let x0 = sw / 2.0 - (pages as f32 - 1.0) * 9.0;
            for i in 0..pages {
                let near = (1.0 - (pos - i as f32).abs()).clamp(0.0, 1.0);
                let d = map(x0 + i as f32 * 18.0, y);
                p.push(Panel::solid(d[0], d[1], (7.0 + 2.0 * near) * k, (7.0 + 2.0 * near) * k, 4.5 * k, ink, ink).flat().alpha(0.35 + 0.6 * near));
                self.dots.borrow_mut().push([x0 + i as f32 * 18.0 - 9.0, y - 12.0, 18.0, 24.0]);
            }
        }
        let _ = hover_drop;
        // the open folder, growing out of its tile
        let fk = self.folder_k.x.clamp(0.0, 1.0);
        // (text is drawn over every panel, so the grid's names must go before the folder's glass comes over them)
        l[grid_labels..].iter_mut().for_each(|x| x.opacity *= 1.0 - fk);
        if fk > 0.02 {
            if let Some(Slot::Folder { name, ids }) = self.layout.slots.get(self.folder_last) {
                let apps_in: Vec<usize> = ids.iter().filter_map(|id| list.iter().position(|a| &a.id == id)).collect();
                let n = apps_in.len().max(1);
                let fcols = n.min(4);
                let frows = n.div_ceil(fcols);
                let (cell_w, cell_h) = (118.0, 112.0);
                let (fw, fh) = (fcols as f32 * cell_w + 72.0, frows as f32 * cell_h + 116.0);
                let fr = [(sw - fw) / 2.0, (sh - fh) / 2.0, fw, fh];
                if self.folder.is_some() {
                    self.folder_rect.set(fr);
                }
                let pv = self.folder_pivot.get();
                let sc = 0.55 + 0.45 * fk;
                // from the tile's centre out to the real place, then the zoom of the whole screen
                let m2 = |x: f32, y: f32| map(pv[0] + (x - pv[0]) * sc + (sw / 2.0 - pv[0]) * (1.0 - sc), pv[1] + (y - pv[1]) * sc + (sh / 2.0 - pv[1]) * (1.0 - sc));
                let mut op: Vec<Panel> = Vec::new();
                let mut ol: Vec<Label> = Vec::new();
                op.push(Panel::solid(sw / 2.0, sh / 2.0, sw, sh, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(0.30));
                let c = m2(fr[0] + fw / 2.0, fr[1] + fh / 2.0);
                let mut frame = Panel::glass(c[0], c[1], fw * sc * k, fh * sc * k, 30.0 * sc * k).tinted([0.10, 0.13, 0.22, 0.62]);
                frame.fx[0] = 30.0;
                op.push(frame);
                // the name, which can be typed over
                let title = [fr[0] + 40.0, fr[1] + 22.0, fw - 80.0, 40.0];
                if self.folder.is_some() {
                    self.pad_hits.borrow_mut().push((title, Hit::Title));
                }
                let tc = m2(fr[0] + fw / 2.0, fr[1] + 49.0);
                let caret = if self.name_edit && (self.t * 1.6).fract() < 0.6 { "|" } else { "" };
                if self.name_edit {
                    let bc = m2(fr[0] + fw / 2.0, fr[1] + 42.0);
                    op.push(Panel::solid(bc[0], bc[1], (fw - 120.0) * sc * k, 34.0 * sc * k, 10.0 * sc * k, [1.0; 3], [1.0; 3]).flat().alpha(0.14));
                }
                ol.push(Label::new(format!("{name}{caret}"), tc[0], tc[1], 20.0 * sc, true, ink, 0.97, Align::Center).verbatim());
                for (i, &ai) in apps_in.iter().enumerate() {
                    let (col, row) = ((i % fcols) as f32, (i / fcols) as f32);
                    let cell = [fr[0] + 36.0 + col * cell_w, fr[1] + 84.0 + row * cell_h, cell_w, cell_h];
                    let lifted_here = lifted == Some(Hit::InFolder(ai));
                    if self.folder.is_some() {
                        self.pad_hits.borrow_mut().push((cell, Hit::InFolder(ai)));
                        if inside(self.mouse, cell) && lifted.is_none() {
                            let rc = m2(cell[0] + cell_w / 2.0, cell[1] + cell_h / 2.0);
                            op.push(Panel::solid(rc[0], rc[1], (cell_w - 10.0) * sc * k, (cell_h - 8.0) * sc * k, 14.0 * sc * k, [1.0; 3], [1.0; 3]).flat().alpha(0.14));
                        }
                    }
                    if lifted_here {
                        continue;
                    }
                    let ic = m2(cell[0] + cell_w / 2.0, cell[1] + 38.0);
                    push_icon(&mut op, ctx, &list[ai], ic, 64.0 * sc * k, 14.0 * sc * k);
                    let t = m2(cell[0] + cell_w / 2.0, cell[1] + 88.0);
                    ol.push(Label::new(trunc(&list[ai].name, 15), t[0], t[1], 12.5 * sc, false, ink, 0.96, Align::Center));
                }
                op.iter_mut().for_each(|x| x.fade_by(fk));
                ol.iter_mut().for_each(|x| x.opacity *= fk);
                p.extend(op);
                l.extend(ol);
            }
        }
        // what is being dragged follows the pointer
        if let (Some(h), true) = (lifted, self.dragging) {
            let m = self.mouse;
            let from = p.len();
            match h {
                Hit::Slot(s) => match self.layout.slots.get(s) {
                    Some(Slot::App { id }) => {
                        if let Some(a) = list.iter().find(|a| &a.id == id) {
                            push_icon(&mut p, ctx, a, m, icon * 1.08 * k, icon * 0.24 * k);
                        }
                    }
                    Some(Slot::Folder { ids, .. }) => self.folder_tile(&mut p, ctx, ids, m, icon * 1.08 * k),
                    None => {}
                },
                Hit::InFolder(ai) | Hit::App(ai) => push_icon(&mut p, ctx, &list[ai], m, icon * 1.08 * k, icon * 0.24 * k),
                Hit::Title => {}
            }
            p[from..].iter_mut().for_each(|x| x.fade_by(0.85));
        }
        (p, l)
    }

    /// A folder's tile: a rounded plate with the first nine of its apps on it, three by three.
    fn folder_tile(&self, p: &mut Vec<Panel>, ctx: &DrawCtx, ids: &[String], c: [f32; 2], size: f32) {
        p.push(Panel::solid(c[0], c[1], size, size, size * 0.24, [1.0; 3], [1.0; 3]).flat().alpha(0.22));
        let cell = size * 0.84 / 3.0;
        for (n, id) in ids.iter().take(9).enumerate() {
            let Some(a) = ctx.apps.iter().find(|a| &a.id == id) else { continue };
            let (col, row) = ((n % 3) as f32, (n / 3) as f32);
            let at = [c[0] - size * 0.42 + cell * (col + 0.5), c[1] - size * 0.42 + cell * (row + 0.5)];
            push_icon(p, ctx, a, at, cell * 0.84, cell * 0.2);
        }
    }

    // ------------------------------------------------------------ Aero start menu

    fn draw_win7(&self, ctx: &DrawCtx) -> (Vec<Panel>, Vec<Label>) {
        const W: f32 = 410.0;
        const H: f32 = 540.0;
        let theme = ctx.theme;
        let list = ctx.apps;
        let mut p = Vec::new();
        let mut l = Vec::new();
        let k = (0.96 + 0.04 * self.k.x).clamp(0.9, 1.04);
        let (x, y) = self.place(ctx, W, H, 2.0);
        let pivot = [x, y + H];
        let map = |px: f32, py: f32| [pivot[0] + (px - pivot[0]) * k, pivot[1] + (py - pivot[1]) * k];
        self.menu_rect.set([x, y, W, H]);

        let c = map(x + W / 2.0, y + H / 2.0);
        let mut frame = theme.glass.panel(c[0], c[1], W * k, H * k, 8.0 * k);
        frame.fx[0] = frame.fx[0].max(16.0);
        // the right column is white text on glass: the menu is deep blue whatever the theme's glass is
        frame.tint = [0.14, 0.32, 0.56, 0.74];
        p.push(frame);

        // left: white programs list
        let lw = 236.0;
        let lh = H - 60.0;
        let lc = map(x + 8.0 + lw / 2.0, y + 8.0 + lh / 2.0);
        p.push(Panel::solid(lc[0], lc[1], lw * k, lh * k, 4.0 * k, [0.98, 0.99, 1.0], [0.94, 0.96, 0.99]).flat().alpha(0.97));
        let row_item = |p: &mut Vec<Panel>, l: &mut Vec<Label>, ai: usize, r: [f32; 4], selected: bool| {
            let hovered = inside(self.mouse, r);
            if selected || hovered {
                let rc = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                p.push(Panel::solid(rc[0], rc[1], r[2] * k, r[3] * k, 3.0 * k, [0.80, 0.91, 1.0], [0.66, 0.82, 0.98]).flat().alpha(0.85));
                p.push(Panel::solid(rc[0], rc[1], (r[2] + 2.0) * k, (r[3] + 2.0) * k, 3.5 * k, [0.45, 0.65, 0.90], [0.45, 0.65, 0.90]).flat().alpha(0.35));
            }
            let ic = map(r[0] + 24.0, r[1] + r[3] / 2.0);
            push_icon(p, ctx, &list[ai], ic, 28.0 * k, 6.0 * k);
            let t = map(r[0] + 48.0, r[1] + r[3] / 2.0 + 4.5);
            l.push(Label::new(trunc(&list[ai].name, 24), t[0], t[1], 12.5, false, [0.08, 0.10, 0.16], 0.95, Align::Left));
        };
        let shown: Vec<usize> = if self.query.is_empty() {
            if ctx.pinned.is_empty() { (0..list.len()).take(9).collect() } else { ctx.pinned.iter().copied().take(9).collect() }
        } else {
            self.results.clone()
        };
        for (i, &ai) in shown.iter().enumerate() {
            let r = [x + 12.0, y + 14.0 + i as f32 * 40.0, lw - 8.0, 38.0];
            self.hits.borrow_mut().push((r, ai));
            row_item(&mut p, &mut l, ai, r, !self.query.is_empty() && i == self.sel);
        }
        if !self.query.is_empty() && shown.is_empty() {
            let t = map(x + 8.0 + lw / 2.0, y + 60.0);
            l.push(Label::new("No matches found", t[0], t[1], 12.5, false, [0.3, 0.33, 0.4], 0.8, Align::Center));
        }
        // "All Programs" and the search field
        let sep = map(x + 8.0 + lw / 2.0, y + 8.0 + lh - 46.0);
        p.push(Panel::solid(sep[0], sep[1], (lw - 20.0) * k, 1.0, 0.5, [0.6, 0.66, 0.76], [0.6, 0.66, 0.76]).flat().alpha(0.6));
        let ap = map(x + 22.0, y + 8.0 + lh - 24.0);
        l.push(Label::new("All Programs  ›", ap[0], ap[1], 12.5, true, [0.08, 0.10, 0.16], 0.9, Align::Left));
        let sf = map(x + 8.0 + lw / 2.0, y + H - 28.0);
        p.push(Panel::solid(sf[0], sf[1], (lw - 8.0) * k, 28.0 * k, 3.0 * k, [1.0; 3], [0.97, 0.98, 1.0]).flat().alpha(0.97));
        p.push(Panel::solid(sf[0], sf[1], (lw - 6.0) * k, 30.0 * k, 4.0 * k, [0.45, 0.58, 0.78], [0.45, 0.58, 0.78]).flat().alpha(0.35));
        let sg = map(x + 8.0 + lw - 22.0, y + H - 28.0);
        p.push(Panel::glyph(sg[0], sg[1], 15.0 * k, "search", [0.3, 0.4, 0.6], 0.8));
        let stx = map(x + 20.0, y + H - 23.5);
        if self.query.is_empty() {
            l.push(Label::new("Search programs and files", stx[0], stx[1], 12.0, false, [0.4, 0.45, 0.55], 0.75, Align::Left));
        } else {
            l.push(Label::new(self.query.clone(), stx[0], stx[1], 12.5, false, [0.06, 0.08, 0.14], 0.95, Align::Left));
        }

        // right: avatar, links, shut down (white text on the glass)
        let rx = x + 8.0 + lw + 14.0;
        let av = map(rx + 26.0, y + 38.0);
        p.push(Panel::solid(av[0], av[1], 52.0 * k, 52.0 * k, 4.0 * k, [0.95, 0.97, 1.0], [0.70, 0.80, 0.95]).flat().alpha(0.95));
        p.push(Panel::solid(av[0], av[1], 54.0 * k, 54.0 * k, 5.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(0.35));
        p.push(Panel::glyph(av[0], av[1], 30.0 * k, "user", [0.20, 0.35, 0.60], 0.9));
        let un = map(rx + 62.0, y + 44.0);
        l.push(Label::new(sysinfo::user_name(), un[0], un[1], 14.0, true, [1.0; 3], 0.97, Align::Left));
        let links = ["Documents", "Pictures", "Music", "Games", "Computer", "Control Panel", "Devices and Printers", "Default Programs", "Help and Support"];
        for (i, name) in links.iter().enumerate() {
            let r = [rx, y + 92.0 + i as f32 * 31.0, W - (rx - x) - 10.0, 29.0];
            if inside(self.mouse, r) {
                let rc = map(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                p.push(Panel::solid(rc[0], rc[1], r[2] * k, r[3] * k, 3.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(0.28));
            }
            let t = map(r[0] + 12.0, r[1] + 19.5);
            l.push(Label::new(*name, t[0], t[1], 12.5, false, [1.0; 3], 0.95, Align::Left));
        }
        let sd = [rx + 2.0, y + H - 42.0, 118.0, 28.0];
        self.power_rect.set(sd);
        let sc = map(sd[0] + sd[2] / 2.0, sd[1] + sd[3] / 2.0);
        p.push(Panel::solid(sc[0], sc[1], sd[2] * k, sd[3] * k, 4.0 * k, [0.22, 0.34, 0.52], [0.06, 0.12, 0.26]).flat().alpha(0.9));
        p.push(Panel::solid(sc[0], sc[1], (sd[2] + 2.0) * k, (sd[3] + 2.0) * k, 5.0 * k, [1.0; 3], [1.0; 3]).flat().alpha(0.30));
        let st = map(sd[0] + 34.0, sd[1] + 19.0);
        l.push(Label::new("Shut down", st[0], st[1], 12.5, false, [1.0; 3], 0.97, Align::Left));
        let pg = map(sd[0] + 18.0, sd[1] + 14.0);
        p.push(Panel::glyph(pg[0], pg[1], 14.0 * k, "power", [1.0; 3], 0.9));
        (p, l)
    }
}

/// An application icon (or a generic tile) centred at `c`, in the theme's icon style.
fn push_icon(p: &mut Vec<Panel>, ctx: &DrawCtx, app: &AppEntry, c: [f32; 2], size: f32, radius: f32) {
    match (ctx.icon_cell)(app) {
        Some(cell) => crate::theme::push_app_icon(ctx.theme, p, c[0], c[1], size, cell, (0.16, 5.0, 2.0)),
        None => {
            p.push(Panel::solid(c[0], c[1], size * 0.94, size * 0.94, radius, [0.80, 0.84, 0.92], [0.62, 0.68, 0.80]).flat());
            p.push(Panel::glyph(c[0], c[1], size * 0.58, "app-window", [1.0; 3], 0.95));
        }
    }
}
