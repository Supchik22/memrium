//! Icons on the desktop (the contents of ~/Desktop): an auto-arranged grid under the windows, with
//! selection (click, Ctrl/Shift, rubber band), rename, open, cut / copy / paste, Trash and drag & drop.

use crate::files::{cmd, FilesEvent, IconState, Key, KeyMods};
use crate::fsops::{self, Entry, Fm, SortBy};
use crate::menu::MenuItem;
use crate::renderer::{Align, Label, Panel};
use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopPrefs {
    pub show: bool,
    pub icon_size: f32,
    pub sort: SortBy,
    pub show_hidden: bool,
}

impl Default for DesktopPrefs {
    fn default() -> Self {
        DesktopPrefs { show: true, icon_size: 64.0, sort: SortBy::Name, show_hidden: false }
    }
}

pub struct DesktopCtx<'a> {
    pub theme: &'a Theme,
    pub measure: &'a dyn Fn(&str, f32, bool) -> f32,
    pub time: f32,
    pub fm: &'a Fm,
    pub icon: &'a dyn Fn(&Entry, bool) -> IconState,
    /// free area (x, y, w, h): below the menu bar, above the Dock / taskbar
    pub area: [f32; 4],
}

struct Edit {
    target: PathBuf,
    text: String,
}

pub struct Desktop {
    pub prefs: DesktopPrefs,
    dir: PathBuf,
    entries: Vec<Entry>,
    sel: Vec<PathBuf>,
    anchor: Option<PathBuf>,
    edit: Option<Edit>,
    mouse: [f32; 2],
    rects: Vec<[f32; 4]>,
    rows: usize,
    /// the desktop has keyboard focus (it was the last thing clicked)
    pub focused: bool,
    last_click: Option<(Instant, PathBuf)>,
    band: Option<[f32; 2]>,
    pub wanted: Vec<Entry>,
    pub drop_hover: Option<PathBuf>,
    seen_gen: u64,
    last_poll: f32,
    stamp: Option<std::time::SystemTime>,
    pending_rename: Option<PathBuf>,
    /// a picture renamed to another format's ending, waiting for the answer (see `Files`)
    pending_convert: Option<(PathBuf, String)>,
    loaded: bool,
    /// the cells icons were dragged to (the others follow the sort order into the free cells)
    cells: HashMap<String, Cell>,
    /// the cell of each entry as last drawn, and the grid: (origin, cell width, cell height, columns, rows)
    placed: Vec<Cell>,
    grid: Option<([f32; 2], f32, f32, i32, i32)>,
    /// icons being carried over the empty desktop: (their paths, where they were picked up, where the pointer is now)
    pub drag: Option<(Vec<PathBuf>, [f32; 2], [f32; 2])>,
}

/// A cell of the icon grid: (column, row).
type Cell = (i32, i32);

/// Where each icon sits: the cell it was left in (kept while it still fits on the screen), the rest filling the free cells
/// from the top left, down the columns. `names` and the result are in the same order.
fn arrange(names: &[&str], saved: &HashMap<String, Cell>, cols: i32, rows: i32) -> Vec<Cell> {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let mut taken = HashSet::new();
    let mut out: Vec<Option<Cell>> = vec![None; names.len()];
    for (i, n) in names.iter().enumerate() {
        if let Some(&(c, r)) = saved.get(*n) {
            let cell = (c.clamp(0, cols - 1), r.clamp(0, rows - 1));
            if taken.insert(cell) {
                out[i] = Some(cell);
            }
        }
    }
    let mut next = 0;
    for slot in out.iter_mut().filter(|c| c.is_none()) {
        loop {
            let cell = (next / rows, next % rows);
            next += 1;
            if taken.insert(cell) {
                *slot = Some(cell);
                break;
            }
        }
    }
    out.into_iter().flatten().collect()
}

/// The new cells of the icons `moving` (indexes into `placed`) when the group is carried by `delta`: each keeps its place
/// in the group, and one that would land on an icon that stays (or on the edge) takes the nearest free cell.
fn carry(placed: &[Cell], moving: &[usize], delta: Cell, cols: i32, rows: i32) -> Vec<(usize, Cell)> {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let mut taken: HashSet<Cell> = placed.iter().enumerate().filter(|(i, _)| !moving.contains(i)).map(|(_, c)| *c).collect();
    let mut order = moving.to_vec();
    order.sort_by_key(|&i| placed[i]);
    let mut out = Vec::new();
    for i in order {
        let want = ((placed[i].0 + delta.0).clamp(0, cols - 1), (placed[i].1 + delta.1).clamp(0, rows - 1));
        let cell = if taken.contains(&want) {
            let mut best: Option<(i32, Cell)> = None;
            for c in 0..cols {
                for r in 0..rows {
                    if !taken.contains(&(c, r)) {
                        let d = (c - want.0).pow(2) + (r - want.1).pow(2);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, (c, r)));
                        }
                    }
                }
            }
            best.map_or(placed[i], |b| b.1)
        } else {
            want
        };
        taken.insert(cell);
        out.push((i, cell));
    }
    out
}

fn layout_file() -> PathBuf {
    crate::theme::config_dir().join("desktop-layout.toml")
}

/// The cells icons were left in, by file name: `name = [column, row]` in `desktop-layout.toml`.
fn load_cells() -> HashMap<String, Cell> {
    let Ok(text) = std::fs::read_to_string(layout_file()) else { return HashMap::new() };
    let Ok(t) = text.parse::<toml::Table>() else { return HashMap::new() };
    t.iter()
        .filter_map(|(k, v)| {
            let a = v.as_array()?;
            Some((k.clone(), (a.first()?.as_integer()? as i32, a.get(1)?.as_integer()? as i32)))
        })
        .collect()
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]
}

/// Split a name into at most two centred lines of at most `max_w` px.
fn wrap2(text: &str, size: f32, max_w: f32, measure: &dyn Fn(&str, f32, bool) -> f32) -> Vec<String> {
    if measure(text, size, false) <= max_w {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    // longest prefix that fits, preferring a break after a separator
    let mut cut = 0;
    for i in 1..=chars.len() {
        let s: String = chars[..i].iter().collect();
        if measure(&s, size, false) > max_w {
            break;
        }
        cut = i;
    }
    if cut == 0 {
        cut = 1;
    }
    if let Some(sep) = chars[..cut].iter().rposition(|c| matches!(c, ' ' | '_' | '-' | '.')) {
        if sep > cut / 2 {
            cut = sep + 1;
        }
    }
    let first: String = chars[..cut].iter().collect::<String>().trim_end().to_string();
    let rest: String = chars[cut..].iter().collect();
    let mut second = rest.trim_start().to_string();
    if measure(&second, size, false) > max_w {
        let c2: Vec<char> = second.chars().collect();
        let (mut lo, mut hi) = (0usize, c2.len());
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let s: String = c2[..mid].iter().collect::<String>() + "…";
            if measure(&s, size, false) <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        second = c2[..lo].iter().collect::<String>() + "…";
    }
    if second.is_empty() {
        vec![first]
    } else {
        vec![first, second]
    }
}

impl Desktop {
    pub fn new(prefs: DesktopPrefs) -> Desktop {
        Desktop {
            prefs,
            dir: fsops::desktop_dir(),
            entries: Vec::new(),
            sel: Vec::new(),
            anchor: None,
            edit: None,
            mouse: [-1000.0; 2],
            rects: Vec::new(),
            rows: 1,
            focused: false,
            last_click: None,
            band: None,
            wanted: Vec::new(),
            drop_hover: None,
            seen_gen: 0,
            last_poll: 0.0,
            stamp: None,
            pending_rename: None,
            pending_convert: None,
            loaded: false,
            cells: load_cells(),
            placed: Vec::new(),
            grid: None,
            drag: None,
        }
    }

    fn save_cells(&mut self) {
        // names that are gone (deleted, moved away) take their cell with them
        let dir = self.dir.clone();
        self.cells.retain(|n, _| dir.join(n).symlink_metadata().is_ok());
        let mut t = toml::Table::new();
        for (n, (c, r)) in &self.cells {
            t.insert(n.clone(), toml::Value::Array(vec![toml::Value::Integer(*c as i64), toml::Value::Integer(*r as i64)]));
        }
        let path = layout_file();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(path, toml::to_string(&t).unwrap_or_default());
    }

    /// A renamed file keeps its place.
    fn renamed(&mut self, from: &Path, to: &Path) {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_string());
        if let (Some(a), Some(b)) = (name(from), name(to)) {
            if let Some(c) = self.cells.remove(&a) {
                self.cells.insert(b, c);
                self.save_cells();
            }
        }
    }

    /// Are `paths` icons of this desktop (so that dropping them on the empty desktop moves them rather than files)?
    pub fn owns(&self, paths: &[PathBuf]) -> bool {
        !paths.is_empty() && paths.iter().all(|p| p.parent() == Some(self.dir.as_path()))
    }

    /// Where icons picked up at `from` and let go at `to` would land: (entry, cell) for each of `paths`. They move by the
    /// distance between the two cells, as a group. None when this is not a move on the grid.
    fn plan_move(&self, paths: &[PathBuf], from: [f32; 2], to: [f32; 2]) -> Option<Vec<(usize, Cell)>> {
        let ([ox, oy], cw, ch, cols, rows) = self.grid?;
        let anchor = self.entry_at(from).filter(|&i| i < self.placed.len())?;
        let t = (((to[0] - ox) / cw).floor() as i32, ((to[1] - oy) / ch).floor() as i32);
        let a = self.placed[anchor];
        let moving: Vec<usize> = self.entries.iter().enumerate().filter(|(_, e)| paths.contains(&e.path)).map(|(i, _)| i).collect();
        if moving.is_empty() {
            return None;
        }
        let delta = (t.0.clamp(0, cols - 1) - a.0, t.1.clamp(0, rows - 1) - a.1);
        Some(carry(&self.placed, &moving, delta, cols, rows))
    }

    /// Icons picked up at `from` and let go at `to`: they stay where they are put, and everything else keeps its place too
    /// (so nothing slides about). False when this was not a move on the grid.
    pub fn drop_move(&mut self, paths: &[PathBuf], from: [f32; 2], to: [f32; 2]) -> bool {
        let Some(moved) = self.plan_move(paths, from, to) else { return false };
        if moved.iter().all(|(i, c)| self.placed[*i] == *c) {
            return true;
        }
        for (e, c) in self.entries.iter().zip(&self.placed) {
            self.cells.insert(e.name.clone(), *c);
        }
        for (i, c) in moved {
            self.cells.insert(self.entries[i].name.clone(), c);
        }
        self.save_cells();
        true
    }

    /// Forget where icons were put: they fall back into the order of the sort.
    pub fn arrange_icons(&mut self) {
        self.cells.clear();
        self.save_cells();
    }

    /// The icon next to entry `cur` in a direction (`dc` columns, `dr` rows: one of them is 1 or -1), by where they are drawn.
    fn neighbour(&self, cur: usize, dc: i32, dr: i32) -> Option<usize> {
        let c = *self.placed.get(cur)?;
        self.placed
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != cur && i < &self.entries.len())
            .filter_map(|(i, p)| {
                let (x, y) = (p.0 - c.0, p.1 - c.1);
                let along = x * dc + y * dr;
                (along > 0).then(|| (along + 3 * (x * dr + y * dc).abs(), i))
            })
            .min()
            .map(|(_, i)| i)
    }

    /// Select just this file.
    pub fn select_only(&mut self, path: PathBuf) {
        self.sel = vec![path];
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn set_mouse(&mut self, p: [f32; 2]) {
        self.mouse = p;
        if let Some(start) = self.band {
            let r = [start[0].min(p[0]), start[1].min(p[1]), (start[0] - p[0]).abs(), (start[1] - p[1]).abs()];
            self.sel = self.entries.iter().zip(&self.rects).filter(|(_, c)| overlaps(r, **c)).map(|(e, _)| e.path.clone()).collect();
        }
    }

    pub fn typing(&self) -> bool {
        self.edit.is_some()
    }

    pub fn reload(&mut self) {
        self.dir = fsops::desktop_dir();
        self.entries = fsops::list_dir(&self.dir, self.prefs.show_hidden, self.prefs.sort, false, true).unwrap_or_default();
        self.entries.truncate(300);
        self.stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        self.sel.retain(|p| self.entries.iter().any(|e| &e.path == p));
        self.loaded = true;
        if let Some(p) = self.pending_rename.take() {
            if let Some(e) = self.entries.iter().find(|e| e.path == p) {
                self.sel = vec![p.clone()];
                self.anchor = Some(p.clone());
                self.edit = Some(Edit { target: p, text: e.name.clone() });
            }
        }
    }

    fn selected(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| self.sel.contains(&e.path)).collect()
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected().iter().map(|e| e.path.clone()).collect()
    }

    pub fn entry_at(&self, p: [f32; 2]) -> Option<usize> {
        self.rects.iter().position(|r| inside(p, *r))
    }

    /// Where files dropped at `p` go: a folder icon under the pointer, or the desktop folder.
    pub fn drop_target(&self, p: [f32; 2]) -> PathBuf {
        match self.entry_at(p) {
            Some(i) if self.entries[i].is_dir => self.entries[i].path.clone(),
            _ => self.dir.clone(),
        }
    }

    pub fn background_menu_items(&self, fm: &Fm) -> Vec<MenuItem> {
        use cmd::*;
        let p = &self.prefs;
        vec![
            MenuItem::new(0, "Create New").icon("file-plus").sub(vec![MenuItem::new(NEW_FOLDER, "Folder").icon("folder-plus"), MenuItem::new(NEW_FILE, "Text file").icon("file-text")]),
            MenuItem::new(PASTE, "Paste Clipboard Contents…").icon("clipboard-paste").key("Ctrl+V").enabled(fm.can_paste()),
            MenuItem::new(UNDO, "Undo").icon("undo-2").key("Ctrl+Z").enabled(fm.can_undo()),
            MenuItem::sep(),
            MenuItem::new(0, "Icons").icon("layout-grid").sub(vec![
                MenuItem::new(D_SHOW, "Show icons on the desktop").icon("eye").checked(p.show),
                MenuItem::sep(),
                MenuItem::new(ICON_SMALL, "Small").icon("minimize-2").checked(p.icon_size < 56.0),
                MenuItem::new(ICON_MEDIUM, "Medium").icon("square").checked((56.0..80.0).contains(&p.icon_size)),
                MenuItem::new(ICON_LARGE, "Large").icon("maximize-2").checked(p.icon_size >= 80.0),
                MenuItem::sep(),
                MenuItem::new(D_ARRANGE, "Arrange icons").icon("layout-grid"),
                MenuItem::new(SORT_NAME, "Sort by name").icon("arrow-down-a-z").checked(p.sort == SortBy::Name),
                MenuItem::new(SORT_DATE, "Sort by date").icon("calendar").checked(p.sort == SortBy::Modified),
                MenuItem::new(SORT_SIZE, "Sort by size").icon("ruler").checked(p.sort == SortBy::Size),
                MenuItem::new(SORT_KIND, "Sort by type").icon("shapes").checked(p.sort == SortBy::Kind),
                MenuItem::sep(),
                MenuItem::new(HIDDEN, "Show hidden files").icon("eye-off").checked(p.show_hidden),
            ]),
        ]
    }

    fn icon_menu(&self, fm: &Fm) -> Vec<MenuItem> {
        use cmd::*;
        let sel = self.selected();
        let one = sel.len() == 1;
        let in_trash = self.dir.starts_with(fsops::trash_path());
        let mut v = vec![
            MenuItem::new(OPEN, "Open").icon("external-link").key("Enter").enabled(one),
            MenuItem::sep(),
            MenuItem::new(CUT, "Cut").icon("scissors").key("Ctrl+X"),
            MenuItem::new(COPY, "Copy").icon("copy").key("Ctrl+C"),
            MenuItem::new(PASTE, "Paste").icon("clipboard-paste").key("Ctrl+V").enabled(fm.can_paste() && one && sel[0].is_dir),
            MenuItem::sep(),
            MenuItem::new(RENAME, "Rename…").icon("pencil").key("F2").enabled(one),
        ];
        if in_trash {
            v.push(MenuItem::new(DELETE_FOREVER, "Delete permanently").icon("trash-2").danger());
        } else {
            v.push(MenuItem::new(TRASH, "Move to Trash").icon("trash-2").key("Delete").danger());
        }
        if let Some(m) = fsops::convert_menu(&self.selected_paths(), CONVERT_BASE) {
            v.push(m);
        }
        if one && crate::archive::is_archive(&sel[0].path) {
            v.push(MenuItem::new(EXTRACT_HERE, "Extract here").icon("file-archive"));
        }
        if !in_trash {
            v.push(MenuItem::new(COMPRESS, "Compress…").icon("file-archive"));
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new(PROPERTIES, "Properties").icon("info").enabled(one));
        v
    }

    /// Apply a name typed over an icon's label. A picture given another format's ending is first asked about.
    fn commit_rename(&mut self, ed: Edit, fm: &mut Fm, ev: &mut Vec<FilesEvent>) {
        let name = ed.text.trim().to_string();
        if let Some(label) = fsops::rename_converts_to(&ed.target, &name) {
            ev.push(FilesEvent::Menu(fsops::rename_prompt(&ed.target, &name, label), [-1.0, -1.0]));
            self.pending_convert = Some((ed.target, name));
            return;
        }
        if let Some(to) = fm.rename(&ed.target, &name) {
            self.renamed(&ed.target, &to);
            self.reload();
            self.sel = vec![to];
        }
    }

    // ---------------------------------------------------------------- input

    /// A press on the desktop. Returns (consumed, events): consumed when it hit an icon.
    pub fn mouse_down(&mut self, p: [f32; 2], right: bool, mods: KeyMods, fm: &mut Fm) -> (bool, Vec<FilesEvent>) {
        let mut ev = Vec::new();
        if let Some(ed) = self.edit.take() {
            self.commit_rename(ed, fm, &mut ev);
        }
        self.focused = true;
        match self.entry_at(p) {
            Some(i) => {
                let path = self.entries[i].path.clone();
                if right {
                    if !self.sel.contains(&path) {
                        self.sel = vec![path.clone()];
                    }
                    self.anchor = Some(path);
                    ev.push(FilesEvent::Menu(self.icon_menu(fm), p));
                } else if mods.ctrl {
                    if let Some(k) = self.sel.iter().position(|s| *s == path) {
                        self.sel.remove(k);
                    } else {
                        self.sel.push(path.clone());
                    }
                    self.anchor = Some(path);
                } else if mods.shift && self.anchor.is_some() {
                    let a = self.anchor.as_ref().and_then(|a| self.entries.iter().position(|e| &e.path == a)).unwrap_or(i);
                    let (lo, hi) = (a.min(i), a.max(i));
                    self.sel = (lo..=hi).map(|k| self.entries[k].path.clone()).collect();
                } else {
                    let double = self.last_click.as_ref().is_some_and(|(t, last)| *last == path && t.elapsed().as_millis() < 400);
                    if double {
                        self.last_click = None;
                        self.sel = vec![path.clone()];
                        self.activate(i, &mut ev);
                    } else {
                        self.last_click = Some((Instant::now(), path.clone()));
                        if !(self.sel.len() > 1 && self.sel.contains(&path)) {
                            self.sel = vec![path.clone()];
                        }
                        self.anchor = Some(path);
                        ev.push(FilesEvent::DragStart(self.selected_paths()));
                    }
                }
                (true, ev)
            }
            None => {
                if !right {
                    if !mods.ctrl {
                        self.sel.clear();
                    }
                    self.band = Some(p);
                }
                (false, ev)
            }
        }
    }

    pub fn mouse_up(&mut self) {
        self.band = None;
    }

    fn activate(&mut self, i: usize, ev: &mut Vec<FilesEvent>) {
        let e = &self.entries[i];
        if e.is_dir {
            ev.push(FilesEvent::OpenFolder(e.path.clone()));
        } else {
            ev.push(FilesEvent::Open(e.path.clone()));
        }
    }

    pub fn command(&mut self, id: u32, fm: &mut Fm, ev: &mut Vec<FilesEvent>) {
        use cmd::*;
        let mut prefs = false;
        match id {
            OPEN => {
                if let Some(i) = self.sel.first().and_then(|p| self.entries.iter().position(|e| &e.path == p)) {
                    self.activate(i, ev);
                }
            }
            RENAME_CONVERT => {
                if let Some((from, name)) = self.pending_convert.take() {
                    fm.note = match fsops::convert_image_to(&from, &name) {
                        Ok(to) => {
                            self.reload();
                            self.sel = vec![to];
                            format!("Converted to {name}; the original is kept")
                        }
                        Err(e) => format!("Could not convert: {e}"),
                    };
                }
            }
            RENAME_ONLY => {
                if let Some((from, name)) = self.pending_convert.take() {
                    if let Some(to) = fm.rename(&from, &name) {
                        self.renamed(&from, &to);
                        self.reload();
                        self.sel = vec![to];
                    }
                }
            }
            CONVERT_MORE => {
                let mut args: Vec<String> = Vec::new();
                args.extend(self.selected_paths().iter().map(|p| p.to_string_lossy().to_string()));
                ev.push(FilesEvent::Own("memrium_convert", args));
            }
            EXTRACT_HERE | COMPRESS => {
                let mut args = vec![if id == EXTRACT_HERE { "--extract" } else { "--compress" }.to_string()];
                args.extend(self.selected_paths().iter().map(|p| p.to_string_lossy().to_string()));
                ev.push(FilesEvent::Own("memrium_archive", args));
            }
            id if (CONVERT_BASE..CONVERT_BASE + fsops::CONVERT_TO.len() as u32).contains(&id) => {
                fm.note = fsops::convert_all(&self.selected_paths(), fsops::CONVERT_TO[(id - CONVERT_BASE) as usize].1);
                self.reload();
            }
            CUT => fm.set_clip(self.selected_paths(), true),
            COPY => fm.set_clip(self.selected_paths(), false),
            PASTE => {
                let dir = match self.selected().as_slice() {
                    [e] if e.is_dir => e.path.clone(),
                    _ => self.dir.clone(),
                };
                fm.paste_into(&dir);
                self.reload();
            }
            RENAME => {
                if let Some(e) = self.selected().first() {
                    self.edit = Some(Edit { target: e.path.clone(), text: e.name.clone() });
                }
            }
            TRASH => {
                let paths = self.selected_paths();
                fm.trash_all(&paths);
                self.reload();
            }
            DELETE_FOREVER => {
                for p in self.selected_paths() {
                    let _ = fsops::delete_forever(&p);
                }
                self.reload();
            }
            NEW_FOLDER => {
                if let Some(p) = fm.new_folder(&self.dir.clone()) {
                    self.pending_rename = Some(p);
                    self.reload();
                }
            }
            NEW_FILE => {
                if let Some(p) = fm.new_file(&self.dir.clone()) {
                    self.pending_rename = Some(p);
                    self.reload();
                }
            }
            UNDO => {
                fm.undo_last();
                self.reload();
            }
            SELECT_ALL => self.sel = self.entries.iter().map(|e| e.path.clone()).collect(),
            PROPERTIES => {
                if let Some(e) = self.selected().first() {
                    ev.push(FilesEvent::Own("memrium_props", vec![e.path.to_string_lossy().to_string()]));
                }
            }
            D_SHOW => {
                self.prefs.show = !self.prefs.show;
                prefs = true;
            }
            ICON_SMALL => {
                self.prefs.icon_size = 48.0;
                prefs = true;
            }
            ICON_MEDIUM => {
                self.prefs.icon_size = 64.0;
                prefs = true;
            }
            ICON_LARGE => {
                self.prefs.icon_size = 88.0;
                prefs = true;
            }
            SORT_NAME | SORT_SIZE | SORT_DATE | SORT_KIND => {
                self.prefs.sort = match id {
                    SORT_NAME => SortBy::Name,
                    SORT_SIZE => SortBy::Size,
                    SORT_DATE => SortBy::Modified,
                    _ => SortBy::Kind,
                };
                self.arrange_icons();
                self.reload();
                prefs = true;
            }
            D_ARRANGE => self.arrange_icons(),
            HIDDEN => {
                self.prefs.show_hidden = !self.prefs.show_hidden;
                self.reload();
                prefs = true;
            }
            _ => {}
        }
        if prefs {
            ev.push(FilesEvent::Prefs);
        }
    }

    /// Keyboard input while the desktop has focus. Returns (used, events).
    pub fn key(&mut self, k: Key, mods: KeyMods, fm: &mut Fm) -> (bool, Vec<FilesEvent>) {
        let mut ev = Vec::new();
        if let Some(ed) = self.edit.as_mut() {
            match k {
                Key::Char(c) if !mods.ctrl && !mods.alt => ed.text.push(c),
                Key::Backspace => {
                    ed.text.pop();
                }
                Key::Enter => {
                    let ed = self.edit.take().unwrap();
                    self.commit_rename(ed, fm, &mut ev);
                }
                Key::Esc => self.edit = None,
                _ => {}
            }
            return (true, ev);
        }
        let n = self.entries.len();
        let cur = self.sel.last().and_then(|p| self.entries.iter().position(|e| &e.path == p));
        let go = |this: &mut Desktop, to: Option<usize>| {
            if let Some(t) = to.filter(|&t| t < n) {
                let p = this.entries[t].path.clone();
                this.sel = vec![p.clone()];
                this.anchor = Some(p);
            }
        };
        match k {
            Key::Down | Key::Up | Key::Right | Key::Left => {
                let (dc, dr) = match k {
                    Key::Down => (0, 1),
                    Key::Up => (0, -1),
                    Key::Right => (1, 0),
                    _ => (-1, 0),
                };
                // the icon next door as drawn; at the edge, or with nothing selected, the first one
                let to = cur.map(|c| self.neighbour(c, dc, dr).unwrap_or(c)).or(Some(0));
                go(self, to)
            }
            Key::Enter => {
                if let Some(i) = cur {
                    self.activate(i, &mut ev);
                }
            }
            Key::F2 => self.command(cmd::RENAME, fm, &mut ev),
            Key::Delete => self.command(cmd::TRASH, fm, &mut ev),
            Key::Esc => self.sel.clear(),
            Key::Char(c) if mods.ctrl => match c.to_ascii_lowercase() {
                'c' => self.command(cmd::COPY, fm, &mut ev),
                'x' => self.command(cmd::CUT, fm, &mut ev),
                'v' => self.command(cmd::PASTE, fm, &mut ev),
                'z' => self.command(cmd::UNDO, fm, &mut ev),
                'a' => self.command(cmd::SELECT_ALL, fm, &mut ev),
                'n' if mods.shift => self.command(cmd::NEW_FOLDER, fm, &mut ev),
                _ => return (false, ev),
            },
            _ => return (false, ev),
        }
        (true, ev)
    }

    // ---------------------------------------------------------------- drawing

    pub fn draw(&mut self, cx: &DesktopCtx) -> (Vec<Panel>, Vec<Label>) {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        self.wanted.clear();
        if !self.prefs.show {
            self.rects.clear();
            return (p, l);
        }
        if !self.loaded || cx.fm.gen != self.seen_gen {
            self.seen_gen = cx.fm.gen;
            self.reload();
        } else if cx.time - self.last_poll > 1.0 {
            self.last_poll = cx.time;
            let stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
            if stamp != self.stamp && self.edit.is_none() {
                self.reload();
            }
        }
        let theme = cx.theme;
        let accent = theme.accent;
        let area = cx.area;
        let size = self.prefs.icon_size;
        let (cw, ch) = (size + 40.0, size + 46.0);
        let rows = (((area[3] - 20.0) / ch).floor() as i32).max(1);
        let cols = (((area[2] - 20.0) / cw).floor() as i32).max(1);
        self.rows = rows as usize;
        let origin = [area[0] + 10.0, area[1] + 10.0];
        self.grid = Some((origin, cw, ch, cols, rows));
        let names: Vec<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
        self.placed = arrange(&names, &self.cells, cols, rows);
        self.rects.clear();
        let mut wanted = Vec::new();
        let band = self.band.map(|s| [s[0].min(self.mouse[0]), s[1].min(self.mouse[1]), (s[0] - self.mouse[0]).abs(), (s[1] - self.mouse[1]).abs()]);
        for (i, e) in self.entries.iter().enumerate() {
            let (col, row) = self.placed[i];
            let r = [origin[0] + col as f32 * cw, origin[1] + row as f32 * ch, cw, ch];
            // one renderer slot holds 576 panels (icons, shadows and labels all count)
            if r[0] > area[0] + area[2] || p.len() + l.len() * 2 > 560 {
                // not drawn, so not to be hit either
                self.rects.push([-1.0e6, -1.0e6, 0.0, 0.0]);
                continue;
            }
            self.rects.push(r);
            let selected = self.sel.contains(&e.path);
            let hover = inside(self.mouse, r) && self.band.is_none();
            let cut = cx.fm.cut && cx.fm.clip.contains(&e.path);
            let dropping = e.is_dir && self.drop_hover.as_ref() == Some(&e.path);
            if selected || hover || dropping {
                let c = [r[0] + 4.0, r[1] + 2.0, r[2] - 8.0, r[3] - 4.0];
                let col = if selected || dropping { accent } else { [1.0, 1.0, 1.0] };
                let a = if dropping { 0.55 } else if selected { 0.38 } else { 0.14 };
                p.push(Panel::solid(c[0] + c[2] / 2.0, c[1] + c[3] / 2.0, c[2], c[3], 10.0, col, col).flat().alpha(a));
            }
            let a = if cut { 0.45 } else { 1.0 };
            let (icx, icy) = (r[0] + r[2] / 2.0, r[1] + 8.0 + size / 2.0);
            match (cx.icon)(e, true) {
                IconState::Cell(c) => {
                    p.push(Panel::app_icon(icx, icy, size, c, a));
                }
                st => {
                    if st == IconState::Pending {
                        wanted.push(e.clone());
                    }
                    p.push(Panel::glyph(icx, icy, size * 0.7, e.kind.icon(), e.kind.tint(accent), a));
                }
            }
            if e.kind == fsops::Kind::Image {
                wanted.push(e.clone());
            }
            // label: white with a soft shadow, up to two lines
            let editing = self.edit.as_ref().is_some_and(|ed| ed.target == e.path);
            let ty = r[1] + size + 22.0;
            if editing {
                let ed = self.edit.as_ref().unwrap();
                let caret = if (cx.time * 2.0) as i32 % 2 == 0 { "|" } else { "" };
                p.push(Panel::solid(r[0] + r[2] / 2.0, ty - 3.0, r[2] - 6.0, 22.0, 6.0, [1.0; 3], [1.0; 3]).flat().alpha(0.92));
                let t = wrap2(&format!("{}{caret}", ed.text), 12.5, r[2] - 18.0, cx.measure);
                l.push(Label::new(t.last().cloned().unwrap_or_default(), r[0] + r[2] / 2.0, ty + 1.5, 12.5, false, [0.08, 0.1, 0.15], 1.0, Align::Center).verbatim());
            } else {
                for (k, line) in wrap2(e.title(), 12.5, r[2] - 10.0, cx.measure).into_iter().enumerate() {
                    let y = ty + k as f32 * 15.0;
                    // two shadows (a tight one and a wide faint one) keep white names readable on any wallpaper; every extra
                    // label eats into the 576-panel limit of one draw
                    for (dx, dy, al) in [(0.4, 1.2, 0.90), (-0.9, 1.9, 0.30)] {
                        l.push(Label::new(line.clone(), r[0] + r[2] / 2.0 + dx, y + dy, 12.5, false, [0.0; 3], al * a, Align::Center).verbatim());
                    }
                    l.push(Label::new(line, r[0] + r[2] / 2.0, y, 12.5, false, [1.0; 3], 0.98 * a, Align::Center).verbatim());
                }
            }
        }
        // the cells the carried icons would land in
        if let Some((paths, from, to)) = self.drag.clone() {
            if let (Some(moved), Some(([ox, oy], ..))) = (self.plan_move(&paths, from, to), self.grid) {
                for (i, (c, r)) in moved.into_iter().take(24) {
                    let _ = i;
                    let rc = [ox + c as f32 * cw + 4.0, oy + r as f32 * ch + 2.0, cw - 8.0, ch - 4.0];
                    p.push(Panel::solid(rc[0] + rc[2] / 2.0, rc[1] + rc[3] / 2.0, rc[2], rc[3], 10.0, accent, accent).flat().alpha(0.22));
                    p.push(Panel::solid(rc[0] + rc[2] / 2.0, rc[1] + rc[3] / 2.0, rc[2] - 4.0, rc[3] - 4.0, 8.0, [1.0; 3], [1.0; 3]).flat().alpha(0.06));
                }
            }
        }
        // rubber band
        if let Some(b) = band {
            if b[2] > 3.0 || b[3] > 3.0 {
                p.push(Panel::solid(b[0] + b[2] / 2.0, b[1] + b[3] / 2.0, b[2], b[3], 3.0, accent, accent).flat().alpha(0.22));
            }
        }
        self.wanted = wanted;
        (p, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(v: &[(&str, Cell)]) -> HashMap<String, Cell> {
        v.iter().map(|(n, c)| (n.to_string(), *c)).collect()
    }

    #[test]
    fn icons_without_a_cell_fill_down_the_columns() {
        let cells = arrange(&["a", "b", "c", "d", "e"], &HashMap::new(), 4, 2);
        assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0)]);
    }

    #[test]
    fn a_placed_icon_keeps_its_cell_and_the_others_flow_round_it() {
        let cells = arrange(&["a", "b", "c"], &saved(&[("b", (0, 0))]), 4, 3);
        assert_eq!(cells, vec![(0, 1), (0, 0), (0, 2)]);
    }

    #[test]
    fn a_cell_off_the_screen_is_pulled_in_and_two_never_share_one() {
        let cells = arrange(&["a", "b"], &saved(&[("a", (90, 90)), ("b", (3, 2))]), 4, 3);
        assert_eq!(cells[0], (3, 2));
        assert_ne!(cells[1], cells[0]);
    }

    #[test]
    fn more_icons_than_cells_run_on_past_the_last_column() {
        let names: Vec<String> = (0..7).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let cells = arrange(&refs, &HashMap::new(), 2, 2);
        assert_eq!(cells[6], (3, 0));
    }

    #[test]
    fn carrying_a_group_keeps_its_shape_and_steps_round_icons_that_stay() {
        let placed = [(0, 0), (0, 1), (2, 0)];
        let out = carry(&placed, &[0, 1], (2, 0), 4, 3);
        // (2,0) is taken by the icon that stays: the first one takes the nearest free cell
        let get = |i| out.iter().find(|(k, _)| *k == i).unwrap().1;
        assert_ne!(get(0), (2, 0));
        assert_eq!(get(1), (2, 1));
        let all: HashSet<Cell> = out.iter().map(|(_, c)| *c).chain([(2, 0)]).collect();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn carrying_past_the_edge_stops_at_the_edge() {
        let out = carry(&[(1, 1)], &[0], (50, -50), 4, 3);
        assert_eq!(out, vec![(0, (3, 0))]);
    }
}
