//! The file manager window (Dolphin-like), drawn by the shell. Presets (Dolphin, Finder, Explorer,
//! Minimal) are just different `FilesPrefs`; every field can be changed from the window's menu and is
//! stored in the config.

use crate::fsops::{self, Entry, Fm, Kind, SortBy};
use crate::menu::MenuItem;
use crate::renderer::{Align, Label, Panel};
use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const WIN_W: f32 = 980.0;
pub const WIN_H: f32 = 620.0;
const TB: f32 = 48.0;
const CMD: f32 = 40.0;
const STATUS: f32 = 30.0;
const HEADER: f32 = 28.0;

/// Menu command ids (shared by the file manager and the desktop menu).
pub mod cmd {
    pub const OPEN: u32 = 100;
    pub const CUT: u32 = 101;
    pub const COPY: u32 = 102;
    pub const PASTE: u32 = 103;
    pub const RENAME: u32 = 104;
    pub const TRASH: u32 = 105;
    pub const NEW_FOLDER: u32 = 106;
    pub const NEW_FILE: u32 = 107;
    pub const UNDO: u32 = 108;
    pub const REFRESH: u32 = 109;
    pub const SELECT_ALL: u32 = 110;
    pub const TERMINAL: u32 = 111;
    pub const COPY_PATH: u32 = 112;
    pub const PROPERTIES: u32 = 113;
    pub const EMPTY_TRASH: u32 = 114;
    pub const DELETE_FOREVER: u32 = 115;
    pub const VIEW_ICONS: u32 = 120;
    pub const VIEW_DETAILS: u32 = 121;
    pub const VIEW_COMPACT: u32 = 122;
    pub const SORT_NAME: u32 = 130;
    pub const SORT_SIZE: u32 = 131;
    pub const SORT_DATE: u32 = 132;
    pub const SORT_KIND: u32 = 133;
    pub const SORT_DESC: u32 = 134;
    pub const DIRS_FIRST: u32 = 135;
    pub const HIDDEN: u32 = 140;
    pub const SIDEBAR: u32 = 141;
    pub const STATUSBAR: u32 = 142;
    pub const LABELS: u32 = 143;
    pub const PATH_CRUMBS: u32 = 144;
    pub const SEL_ROUNDED: u32 = 145;
    pub const COL_SIZE: u32 = 146;
    pub const COL_DATE: u32 = 147;
    pub const COL_KIND: u32 = 148;
    pub const ICON_SMALL: u32 = 150;
    pub const ICON_MEDIUM: u32 = 151;
    pub const ICON_LARGE: u32 = 152;
    pub const PRESET_BASE: u32 = 200;
    /// "Convert to" a picture format: this plus the index in `fsops::CONVERT_TO`
    pub const CONVERT_BASE: u32 = 400;
    /// "More…" in the Convert to menu: the converter window (right after the formats)
    pub const CONVERT_MORE: u32 = CONVERT_BASE + 7;
    pub const EXTRACT_HERE: u32 = 410;
    pub const COMPRESS: u32 = 411;
    /// the answer to "convert it, or only rename it?" after a picture was given another format's ending
    pub const RENAME_CONVERT: u32 = 412;
    pub const RENAME_ONLY: u32 = 413;
    /// let go of a server (in the Computer view), and ask for one to connect to
    pub const DISCONNECT: u32 = 414;
    pub const CONNECT_SERVER: u32 = 415;
    pub const G_BACK: u32 = 160;
    pub const G_FWD: u32 = 161;
    pub const G_UP: u32 = 162;
    pub const G_HOME: u32 = 163;
    pub const G_DESKTOP: u32 = 164;
    pub const G_DOCS: u32 = 165;
    pub const G_DOWNLOADS: u32 = 166;
    // desktop-only commands
    pub const D_EDIT: u32 = 300;
    pub const D_WALL: u32 = 301;
    pub const D_DISPLAY: u32 = 302;
    pub const D_FILES: u32 = 303;
    pub const D_TERMINAL: u32 = 304;
    pub const D_DESKTOP_DIR: u32 = 305;
    pub const D_SHOW: u32 = 306;
    pub const D_ARRANGE: u32 = 307;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    Icons,
    #[default]
    Details,
    Compact,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PathStyle {
    #[default]
    Breadcrumb,
    Address,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesPrefs {
    /// name of the preset this started from (display only)
    pub preset: String,
    pub view: ViewMode,
    pub sidebar: bool,
    pub sidebar_w: f32,
    pub sidebar_headers: bool,
    pub status_bar: bool,
    pub icon_size: f32,
    pub row_h: f32,
    pub show_hidden: bool,
    pub sort: SortBy,
    pub sort_desc: bool,
    pub dirs_first: bool,
    pub path_style: PathStyle,
    /// a second row of labelled actions (Explorer style)
    pub toolbar_labels: bool,
    /// selection fills the whole row instead of a rounded pill
    pub selection_row: bool,
    pub col_size: bool,
    pub col_date: bool,
    pub col_kind: bool,
}

pub const PRESETS: [&str; 4] = ["Dolphin", "Icons", "Details", "Minimal"];

impl Default for FilesPrefs {
    fn default() -> Self {
        FilesPrefs::preset("Dolphin")
    }
}

impl FilesPrefs {
    pub fn preset(name: &str) -> FilesPrefs {
        let base = FilesPrefs {
            preset: name.to_string(),
            view: ViewMode::Details,
            sidebar: true,
            sidebar_w: 190.0,
            sidebar_headers: true,
            status_bar: true,
            icon_size: 64.0,
            row_h: 30.0,
            show_hidden: false,
            sort: SortBy::Name,
            sort_desc: false,
            dirs_first: true,
            path_style: PathStyle::Breadcrumb,
            toolbar_labels: false,
            selection_row: false,
            col_size: true,
            col_date: true,
            col_kind: true,
        };
        match name {
            // (the first two names are what these presets were called before they were renamed)
            "Icons" | "Finder" => FilesPrefs { view: ViewMode::Icons, sidebar_w: 200.0, icon_size: 72.0, status_bar: false, col_kind: false, ..base },
            "Details" | "Explorer" => FilesPrefs { toolbar_labels: true, path_style: PathStyle::Address, selection_row: true, row_h: 32.0, sidebar_headers: false, ..base },
            "Minimal" => FilesPrefs { view: ViewMode::Compact, sidebar: false, status_bar: false, row_h: 28.0, ..base },
            _ => base,
        }
    }
}

pub use chebrium::input::{Key, KeyMods};

/// What the shell knows about the icon of a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IconState {
    /// ready: a cell of the app-icon atlas
    Cell(u32),
    /// being loaded (draw the glyph meanwhile, and the shell will fetch it)
    Pending,
    /// no icon available: use the Lucide glyph
    None,
}

pub enum FilesEvent {
    /// the pointer went down on these files; dragging from here moves them
    DragStart(Vec<PathBuf>),
    Menu(Vec<MenuItem>, [f32; 2]),
    /// open with the default application
    Open(PathBuf),
    /// show a folder in the file manager
    OpenFolder(PathBuf),
    Terminal(PathBuf),
    /// start one of our programs with these arguments (the archive manager for "Extract here" and "Compress")
    Own(&'static str, Vec<String>),
    /// preferences changed; save the config
    Prefs,
}

#[derive(Clone, Debug)]
enum Hit {
    Back,
    Forward,
    Up,
    Refresh,
    Crumb(PathBuf),
    PathField,
    Search,
    ViewToggle,
    MenuBtn,
    Place(PathBuf),
    Entry(usize),
    Sort(SortBy),
    ZoomOut,
    ZoomIn,
    Action(u32),
    Content,
    Track,
}

struct Edit {
    text: String,
    /// Some(path) while renaming that entry
    target: Option<PathBuf>,
}

pub struct FilesCtx<'a> {
    pub theme: &'a Theme,
    /// client area size (WIN_W x WIN_H, or larger when maximised)
    pub size: [f32; 2],
    pub measure: &'a dyn Fn(&str, f32, bool) -> f32,
    pub time: f32,
    pub fm: &'a Fm,
    /// icon for an entry (the bool asks for an image thumbnail when there is one)
    pub icon: &'a dyn Fn(&Entry, bool) -> IconState,
}

pub struct Files {
    pub open: bool,
    pub prefs: FilesPrefs,
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    view: Vec<usize>,
    error: Option<String>,
    sel: Vec<PathBuf>,
    anchor: Option<PathBuf>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    scroll: f32,
    /// scrollbar bookkeeping: when it last moved, where the track and thumb are
    scroll_seen: f32,
    scroll_prev: f32,
    track: [f32; 4],
    thumb: [f32; 2],
    /// what a click in the scroll track does (kept from the theme at draw time)
    click_style: crate::theme::ScrollClick,
    filter: String,
    filter_focus: bool,
    edit: Option<Edit>,
    path_edit: Option<String>,
    /// a server being connected to: its address and the answer (where its files are) when it comes
    mounting: Option<(String, std::sync::mpsc::Receiver<Result<PathBuf, String>>)>,
    mouse: [f32; 2],
    hits: Vec<([f32; 4], Hit)>,
    content: [f32; 4],
    rects: Vec<[f32; 4]>,
    content_h: f32,
    cols: usize,
    last_click: Option<(Instant, PathBuf)>,
    typeahead: (String, Instant),
    seen_gen: u64,
    dir_stamp: Option<std::time::SystemTime>,
    last_poll: f32,
    /// set when a freshly created entry should start renaming once it is listed
    pending_rename: Option<PathBuf>,
    /// a picture renamed to another format's ending, waiting for the answer: (the file, the name typed)
    pending_convert: Option<(PathBuf, String)>,
    /// entries whose icons the shell should load (filled by `draw`)
    pub wanted: Vec<Entry>,
    /// folder under a file being dragged (highlighted)
    pub drop_hover: Option<PathBuf>,
}

impl Default for Files {
    fn default() -> Self {
        Files::new(FilesPrefs::default())
    }
}

fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

fn fits(r: [f32; 4], view: [f32; 4]) -> bool {
    r[1] >= view[1] - 0.5 && r[1] + r[3] <= view[1] + view[3] + 0.5
}

fn ellipsize(text: &str, size: f32, max_w: f32, measure: &dyn Fn(&str, f32, bool) -> f32) -> String {
    if measure(text, size, false) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&s, size, false) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// Places in the sidebar: (label, icon, path).
fn places() -> Vec<(&'static str, &'static str, PathBuf)> {
    let h = fsops::home();
    let mut v = vec![("Home", "house", h.clone())];
    for (name, icon, sub) in [
        ("Desktop", "monitor", "Desktop"),
        ("Documents", "file-text", "Documents"),
        ("Downloads", "download", "Downloads"),
        ("Pictures", "image", "Pictures"),
        ("Music", "music", "Music"),
        ("Videos", "video", "Videos"),
    ] {
        let p = h.join(sub);
        if p.is_dir() {
            v.push((name, icon, p));
        }
    }
    v
}

fn system_places() -> Vec<(&'static str, &'static str, PathBuf)> {
    vec![("Computer", "laptop", PathBuf::from(fsops::COMPUTER)), ("Root", "hard-drive", PathBuf::from("/")), ("Trash", "trash-2", fsops::trash_path())]
}

impl Files {
    pub fn new(prefs: FilesPrefs) -> Files {
        Files {
            open: false,
            prefs,
            cwd: fsops::home(),
            entries: Vec::new(),
            view: Vec::new(),
            error: None,
            sel: Vec::new(),
            anchor: None,
            back: Vec::new(),
            fwd: Vec::new(),
            scroll: 0.0,
            scroll_seen: -10.0,
            scroll_prev: 0.0,
            track: [0.0; 4],
            thumb: [0.0; 2],
            click_style: Default::default(),
            filter: String::new(),
            filter_focus: false,
            edit: None,
            path_edit: None,
            mounting: None,
            mouse: [-1000.0; 2],
            hits: Vec::new(),
            content: [0.0; 4],
            rects: Vec::new(),
            content_h: 0.0,
            cols: 1,
            last_click: None,
            typeahead: (String::new(), Instant::now()),
            seen_gen: 0,
            dir_stamp: None,
            last_poll: 0.0,
            pending_rename: None,
            pending_convert: None,
            wanted: Vec::new(),
            drop_hover: None,
        }
    }

    /// Where files dropped at `p` would go: a place in the sidebar, a folder entry, or the open folder.
    pub fn drop_target(&self, p: [f32; 2]) -> Option<PathBuf> {
        for (r, h) in self.hits.iter().rev() {
            if !inside(p, *r) {
                continue;
            }
            match h {
                Hit::Place(path) => return Some(path.clone()),
                Hit::Entry(vi) => {
                    let e = &self.entries[self.view[*vi]];
                    return Some(if e.is_dir { e.path.clone() } else { self.cwd.clone() });
                }
                Hit::Content => return Some(self.cwd.clone()),
                _ => {}
            }
        }
        None
    }

    pub fn open_at(&mut self, dir: &Path) {
        if !self.open || self.cwd != dir {
            self.open = true;
            if self.entries.is_empty() || self.cwd != dir {
                self.goto(dir.to_path_buf(), true);
            }
        }
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.edit = None;
        self.path_edit = None;
        self.filter_focus = false;
    }

    pub fn set_mouse(&mut self, p: [f32; 2]) {
        self.mouse = p;
    }

    /// Is a text field taking keyboard input?
    pub fn typing(&self) -> bool {
        self.edit.is_some() || self.path_edit.is_some() || self.filter_focus
    }

    // ---------------------------------------------------------------- navigation

    pub fn goto(&mut self, dir: PathBuf, push: bool) {
        let dir = dir.canonicalize().unwrap_or(dir);
        if push && dir != self.cwd {
            self.back.push(self.cwd.clone());
            self.fwd.clear();
        }
        self.cwd = dir;
        self.scroll = 0.0;
        self.sel.clear();
        self.anchor = None;
        self.filter.clear();
        self.filter_focus = false;
        self.edit = None;
        self.path_edit = None;
        self.reload();
    }

    /// Connect to a server in the background; the folder opens when it is there.
    fn connect(&mut self, url: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let u = url.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fsops::mount_server(&u));
        });
        self.mounting = Some((url.clone(), rx));
        if !fsops::is_computer(&self.cwd) {
            self.back.push(self.cwd.clone());
        }
        self.cwd = PathBuf::from(fsops::COMPUTER);
        self.entries.clear();
        self.view.clear();
        self.error = Some(format!("Connecting to {url}…"));
    }

    /// The answer of a connection that was started, once it has come.
    fn poll_mount(&mut self) {
        let Some((url, rx)) = &self.mounting else { return };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                self.mounting = None;
                self.goto(path, true);
            }
            Ok(Err(e)) => {
                let msg = format!("Could not connect to {url}: {e}");
                self.mounting = None;
                self.reload();
                self.error = Some(msg);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(_) => self.mounting = None,
        }
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.fwd.push(self.cwd.clone());
            self.goto_raw(p);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(p) = self.fwd.pop() {
            self.back.push(self.cwd.clone());
            self.goto_raw(p);
        }
    }

    fn goto_raw(&mut self, p: PathBuf) {
        let (b, f) = (std::mem::take(&mut self.back), std::mem::take(&mut self.fwd));
        self.goto(p, false);
        self.back = b;
        self.fwd = f;
    }

    pub fn go_up(&mut self) {
        if fsops::is_computer(&self.cwd) {
            return;
        }
        if let Some(parent) = self.cwd.parent() {
            let from = self.cwd.clone();
            self.goto(parent.to_path_buf(), true);
            self.sel = vec![from.clone()];
            self.anchor = Some(from);
        }
    }

    pub fn reload(&mut self) {
        if fsops::is_computer(&self.cwd) {
            self.entries = fsops::drives();
            self.error = None;
            self.dir_stamp = None;
            self.sel.retain(|p| self.entries.iter().any(|e| &e.path == p));
            self.apply_filter();
            return;
        }
        match fsops::list_dir(&self.cwd, self.prefs.show_hidden, self.prefs.sort, self.prefs.sort_desc, self.prefs.dirs_first) {
            Ok(e) => {
                self.entries = e;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(if e.kind() == std::io::ErrorKind::PermissionDenied { "Permission denied".into() } else { e.to_string() });
            }
        }
        self.dir_stamp = std::fs::metadata(&self.cwd).and_then(|m| m.modified()).ok();
        self.sel.retain(|p| self.entries.iter().any(|e| &e.path == p));
        self.apply_filter();
        if let Some(p) = self.pending_rename.take() {
            if self.entries.iter().any(|e| e.path == p) {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.sel = vec![p.clone()];
                self.anchor = Some(p.clone());
                self.edit = Some(Edit { text: name, target: Some(p) });
            }
        }
    }

    fn apply_filter(&mut self) {
        let f = self.filter.to_lowercase();
        self.view = (0..self.entries.len()).filter(|&i| f.is_empty() || self.entries[i].name.to_lowercase().contains(&f)).collect();
    }

    fn selected_entries(&self) -> Vec<&Entry> {
        self.view.iter().map(|&i| &self.entries[i]).filter(|e| self.sel.contains(&e.path)).collect()
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries().iter().map(|e| e.path.clone()).collect()
    }

    fn view_pos(&self, path: &Path) -> Option<usize> {
        self.view.iter().position(|&i| self.entries[i].path == path)
    }

    fn select_only(&mut self, vi: usize) {
        let p = self.entries[self.view[vi]].path.clone();
        self.sel = vec![p.clone()];
        self.anchor = Some(p);
        self.ensure_visible(vi);
    }

    fn ensure_visible(&mut self, vi: usize) {
        if let Some(r) = self.rects.get(vi) {
            let top = r[1] - self.content[1] - self.header_h();
            let h = self.content[3] - self.header_h();
            if top < self.scroll {
                self.scroll = top - 4.0;
            } else if top + r[3] > self.scroll + h {
                self.scroll = top + r[3] - h + 4.0;
            }
            self.scroll = self.scroll.max(0.0);
        }
    }

    fn header_h(&self) -> f32 {
        if self.prefs.view == ViewMode::Details {
            HEADER
        } else {
            0.0
        }
    }

    // ---------------------------------------------------------------- opening things

    fn activate(&mut self, vi: usize, ev: &mut Vec<FilesEvent>) {
        let e = self.entries[self.view[vi]].clone();
        if e.is_dir {
            self.goto(e.path, true);
        } else {
            ev.push(FilesEvent::Open(e.path));
        }
    }

    fn start_rename(&mut self) {
        if let Some(e) = self.selected_entries().first() {
            self.edit = Some(Edit { text: e.name.clone(), target: Some(e.path.clone()) });
        }
    }

    fn commit_edit(&mut self, fm: &mut Fm, ev: &mut Vec<FilesEvent>) {
        if let Some(ed) = self.edit.take() {
            if let Some(target) = ed.target {
                let name = ed.text.trim().to_string();
                if let Some(label) = fsops::rename_converts_to(&target, &name) {
                    // a picture given another format's ending: ask whether it should become that format
                    ev.push(FilesEvent::Menu(fsops::rename_prompt(&target, &name, label), [-1.0, -1.0]));
                    self.pending_convert = Some((target, name));
                    return;
                }
                if let Some(to) = fm.rename(&target, &name) {
                    self.reload();
                    self.sel = vec![to.clone()];
                    self.anchor = Some(to);
                }
            }
        }
    }

    // ---------------------------------------------------------------- menus & commands

    pub fn view_menu(&self) -> Vec<MenuItem> {
        use cmd::*;
        let p = &self.prefs;
        vec![
            MenuItem::new(0, "View mode").icon("layout-grid").sub(vec![
                MenuItem::new(VIEW_ICONS, "Icons").checked(p.view == ViewMode::Icons),
                MenuItem::new(VIEW_DETAILS, "Details").checked(p.view == ViewMode::Details),
                MenuItem::new(VIEW_COMPACT, "Compact").checked(p.view == ViewMode::Compact),
            ]),
            MenuItem::new(0, "Sort by").icon("arrow-up-down").sub(vec![
                MenuItem::new(SORT_NAME, "Name").checked(p.sort == SortBy::Name),
                MenuItem::new(SORT_SIZE, "Size").checked(p.sort == SortBy::Size),
                MenuItem::new(SORT_DATE, "Date modified").checked(p.sort == SortBy::Modified),
                MenuItem::new(SORT_KIND, "Type").checked(p.sort == SortBy::Kind),
                MenuItem::sep(),
                MenuItem::new(SORT_DESC, "Descending").checked(p.sort_desc),
                MenuItem::new(DIRS_FIRST, "Folders first").checked(p.dirs_first),
            ]),
            MenuItem::new(0, "Icon size").icon("image").sub(vec![
                MenuItem::new(ICON_SMALL, "Small").checked(p.icon_size < 56.0),
                MenuItem::new(ICON_MEDIUM, "Medium").checked((56.0..80.0).contains(&p.icon_size)),
                MenuItem::new(ICON_LARGE, "Large").checked(p.icon_size >= 80.0),
            ]),
            MenuItem::new(0, "Columns").icon("list").sub(vec![
                MenuItem::new(COL_SIZE, "Size").checked(p.col_size),
                MenuItem::new(COL_DATE, "Date modified").checked(p.col_date),
                MenuItem::new(COL_KIND, "Type").checked(p.col_kind),
            ]),
            MenuItem::sep(),
            MenuItem::new(HIDDEN, "Show hidden files").key("Ctrl+H").icon("eye").checked(p.show_hidden),
            MenuItem::new(SIDEBAR, "Sidebar").icon("panel-left").checked(p.sidebar),
            MenuItem::new(STATUSBAR, "Status bar").checked(p.status_bar),
            MenuItem::new(LABELS, "Labelled toolbar").checked(p.toolbar_labels),
            MenuItem::new(PATH_CRUMBS, "Breadcrumb path").checked(p.path_style == PathStyle::Breadcrumb),
            MenuItem::new(SEL_ROUNDED, "Rounded selection").checked(!p.selection_row),
            MenuItem::sep(),
            MenuItem::new(0, "Preset").icon("sliders-horizontal").sub(
                PRESETS.iter().enumerate().map(|(i, n)| MenuItem::new(PRESET_BASE + i as u32, *n).checked(p.preset == *n)).collect(),
            ),
        ]
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.fwd.is_empty()
    }

    pub fn has_selection(&self) -> bool {
        !self.sel.is_empty()
    }

    fn background_menu(&self, fm: &Fm) -> Vec<MenuItem> {
        use cmd::*;
        let mut v = vec![
            MenuItem::new(0, "Create New").icon("file-plus").sub(vec![MenuItem::new(NEW_FOLDER, "Folder").icon("folder-plus").key("Ctrl+Shift+N"), MenuItem::new(NEW_FILE, "Text file").icon("file-text")]),
            MenuItem::new(PASTE, "Paste").icon("clipboard-paste").key("Ctrl+V").enabled(fm.can_paste()),
            MenuItem::new(UNDO, "Undo").icon("undo-2").key("Ctrl+Z").enabled(fm.can_undo()),
            MenuItem::sep(),
            MenuItem::new(SELECT_ALL, "Select all").key("Ctrl+A"),
            MenuItem::new(REFRESH, "Refresh").icon("refresh-cw").key("F5"),
            MenuItem::sep(),
        ];
        if fsops::is_computer(&self.cwd) {
            v.insert(0, MenuItem::sep());
            v.insert(0, MenuItem::new(CONNECT_SERVER, "Connect to Server…").icon("server"));
        }
        v.extend(self.view_menu());
        v.push(MenuItem::sep());
        v.push(MenuItem::new(TERMINAL, "Open Terminal here").icon("terminal"));
        v
    }

    fn entry_menu(&self, fm: &Fm) -> Vec<MenuItem> {
        use cmd::*;
        let sel = self.selected_entries();
        let one = sel.len() == 1;
        let in_trash = self.cwd.starts_with(fsops::trash_path());
        let mut v = vec![MenuItem::new(OPEN, "Open").icon("external-link").key("Enter").enabled(one), MenuItem::sep()];
        v.push(MenuItem::new(CUT, "Cut").icon("scissors").key("Ctrl+X"));
        v.push(MenuItem::new(COPY, "Copy").icon("copy").key("Ctrl+C"));
        v.push(MenuItem::new(PASTE, "Paste").icon("clipboard-paste").key("Ctrl+V").enabled(fm.can_paste() && one && sel[0].is_dir));
        v.push(MenuItem::sep());
        v.push(MenuItem::new(RENAME, "Rename…").icon("pencil").key("F2").enabled(one));
        if in_trash {
            v.push(MenuItem::new(DELETE_FOREVER, "Delete permanently").icon("trash-2"));
        } else {
            v.push(MenuItem::new(TRASH, "Move to Trash").icon("trash-2").key("Delete"));
        }
        if let Some(m) = fsops::convert_menu(&self.selected_paths(), CONVERT_BASE) {
            v.push(m);
        }
        if one && crate::archive::is_archive(&sel[0].path) {
            v.push(MenuItem::new(EXTRACT_HERE, "Extract here").icon("file-archive"));
        }
        if one && fsops::is_computer(&self.cwd) && fsops::is_network_path(&sel[0].path) {
            v.push(MenuItem::sep());
            v.push(MenuItem::new(DISCONNECT, "Disconnect").icon("x"));
        }
        if !in_trash {
            v.push(MenuItem::new(COMPRESS, "Compress…").icon("file-archive"));
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new(COPY_PATH, "Copy path").enabled(one));
        v.push(MenuItem::new(PROPERTIES, "Properties").icon("info").enabled(one));
        v
    }

    /// Run a menu command. Commands that change preferences ask the caller to save.
    pub fn command(&mut self, id: u32, fm: &mut Fm, ev: &mut Vec<FilesEvent>) {
        use cmd::*;
        let mut prefs_changed = true;
        match id {
            OPEN => {
                if let Some(vi) = self.sel.first().and_then(|p| self.view_pos(p)) {
                    self.activate(vi, ev);
                }
                prefs_changed = false;
            }
            CUT => {
                fm.set_clip(self.selected_paths(), true);
                prefs_changed = false;
            }
            COPY => {
                fm.set_clip(self.selected_paths(), false);
                prefs_changed = false;
            }
            PASTE => {
                let dir = match self.selected_entries().as_slice() {
                    [e] if e.is_dir => e.path.clone(),
                    _ => self.cwd.clone(),
                };
                fm.paste_into(&dir);
                self.reload();
                prefs_changed = false;
            }
            RENAME => {
                self.start_rename();
                prefs_changed = false;
            }
            TRASH => {
                let paths = self.selected_paths();
                fm.trash_all(&paths);
                self.reload();
                prefs_changed = false;
            }
            DELETE_FOREVER => {
                for p in self.selected_paths() {
                    if let Err(e) = fsops::delete_forever(&p) {
                        fm.note = format!("Could not delete: {e}");
                    }
                }
                self.reload();
                prefs_changed = false;
            }
            NEW_FOLDER => {
                if let Some(p) = fm.new_folder(&self.cwd.clone()) {
                    self.pending_rename = Some(p);
                    self.reload();
                }
                prefs_changed = false;
            }
            NEW_FILE => {
                if let Some(p) = fm.new_file(&self.cwd.clone()) {
                    self.pending_rename = Some(p);
                    self.reload();
                }
                prefs_changed = false;
            }
            UNDO => {
                fm.undo_last();
                self.reload();
                prefs_changed = false;
            }
            REFRESH => {
                self.reload();
                prefs_changed = false;
            }
            SELECT_ALL => {
                self.sel = self.view.iter().map(|&i| self.entries[i].path.clone()).collect();
                prefs_changed = false;
            }
            id if (CONVERT_BASE..CONVERT_BASE + fsops::CONVERT_TO.len() as u32).contains(&id) => {
                fm.note = fsops::convert_all(&self.selected_paths(), fsops::CONVERT_TO[(id - CONVERT_BASE) as usize].1);
                self.reload();
                prefs_changed = false;
            }
            RENAME_CONVERT => {
                if let Some((from, name)) = self.pending_convert.take() {
                    fm.note = match fsops::convert_image_to(&from, &name) {
                        Ok(to) => {
                            self.reload();
                            self.sel = vec![to.clone()];
                            self.anchor = Some(to);
                            format!("Converted to {name}; the original is kept")
                        }
                        Err(e) => format!("Could not convert: {e}"),
                    };
                }
                prefs_changed = false;
            }
            RENAME_ONLY => {
                if let Some((from, name)) = self.pending_convert.take() {
                    if let Some(to) = fm.rename(&from, &name) {
                        self.reload();
                        self.sel = vec![to.clone()];
                        self.anchor = Some(to);
                    }
                }
                prefs_changed = false;
            }
            CONNECT_SERVER => {
                // the location field, with the usual start of an address in it
                self.path_edit = Some("smb://".into());
                prefs_changed = false;
            }
            DISCONNECT => {
                if let Some(p) = self.selected_paths().first() {
                    fm.note = match fsops::disconnect(p) {
                        Ok(()) => format!("Disconnected {}", p.display()),
                        Err(e) => format!("Could not disconnect: {e}"),
                    };
                    self.reload();
                }
                prefs_changed = false;
            }
            CONVERT_MORE => {
                let mut args: Vec<String> = Vec::new();
                args.extend(self.selected_paths().iter().map(|p| p.to_string_lossy().to_string()));
                ev.push(FilesEvent::Own("memrium_convert", args));
                prefs_changed = false;
            }
            EXTRACT_HERE | COMPRESS => {
                let mut args = vec![if id == EXTRACT_HERE { "--extract" } else { "--compress" }.to_string()];
                args.extend(self.selected_paths().iter().map(|p| p.to_string_lossy().to_string()));
                ev.push(FilesEvent::Own("memrium_archive", args));
                prefs_changed = false;
            }
            TERMINAL => {
                ev.push(FilesEvent::Terminal(self.cwd.clone()));
                prefs_changed = false;
            }
            COPY_PATH => {
                if let Some(p) = self.selected_paths().first() {
                    fm.note = format!("Path: {}", p.display());
                }
                prefs_changed = false;
            }
            PROPERTIES => {
                if let Some(e) = self.selected_entries().first() {
                    fm.note = format!("{} · {} · {} · modified {}", e.name, e.kind.label(), if e.is_dir { "folder".to_string() } else { fsops::human_size(e.size) }, fsops::format_time(e.modified));
                }
                prefs_changed = false;
            }
            EMPTY_TRASH => {
                fsops::empty_trash();
                self.reload();
                prefs_changed = false;
            }
            G_BACK => {
                self.go_back();
                prefs_changed = false;
            }
            G_FWD => {
                self.go_forward();
                prefs_changed = false;
            }
            G_UP => {
                self.go_up();
                prefs_changed = false;
            }
            G_HOME | G_DESKTOP | G_DOCS | G_DOWNLOADS => {
                let h = fsops::home();
                let p = match id {
                    G_HOME => h,
                    G_DESKTOP => h.join("Desktop"),
                    G_DOCS => h.join("Documents"),
                    _ => h.join("Downloads"),
                };
                if p.is_dir() {
                    self.goto(p, true);
                }
                prefs_changed = false;
            }
            VIEW_ICONS => self.prefs.view = ViewMode::Icons,
            VIEW_DETAILS => self.prefs.view = ViewMode::Details,
            VIEW_COMPACT => self.prefs.view = ViewMode::Compact,
            SORT_NAME | SORT_SIZE | SORT_DATE | SORT_KIND => {
                let s = match id {
                    SORT_NAME => SortBy::Name,
                    SORT_SIZE => SortBy::Size,
                    SORT_DATE => SortBy::Modified,
                    _ => SortBy::Kind,
                };
                self.set_sort(s);
            }
            SORT_DESC => {
                self.prefs.sort_desc = !self.prefs.sort_desc;
                self.reload();
            }
            DIRS_FIRST => {
                self.prefs.dirs_first = !self.prefs.dirs_first;
                self.reload();
            }
            HIDDEN => {
                self.prefs.show_hidden = !self.prefs.show_hidden;
                self.reload();
            }
            SIDEBAR => self.prefs.sidebar = !self.prefs.sidebar,
            STATUSBAR => self.prefs.status_bar = !self.prefs.status_bar,
            LABELS => self.prefs.toolbar_labels = !self.prefs.toolbar_labels,
            PATH_CRUMBS => {
                self.prefs.path_style = if self.prefs.path_style == PathStyle::Breadcrumb { PathStyle::Address } else { PathStyle::Breadcrumb };
            }
            SEL_ROUNDED => self.prefs.selection_row = !self.prefs.selection_row,
            COL_SIZE => self.prefs.col_size = !self.prefs.col_size,
            COL_DATE => self.prefs.col_date = !self.prefs.col_date,
            COL_KIND => self.prefs.col_kind = !self.prefs.col_kind,
            ICON_SMALL => self.prefs.icon_size = 48.0,
            ICON_MEDIUM => self.prefs.icon_size = 64.0,
            ICON_LARGE => self.prefs.icon_size = 96.0,
            n if (PRESET_BASE..PRESET_BASE + PRESETS.len() as u32).contains(&n) => {
                let hidden = self.prefs.show_hidden;
                self.prefs = FilesPrefs::preset(PRESETS[(n - PRESET_BASE) as usize]);
                self.prefs.show_hidden = hidden;
                self.reload();
            }
            _ => prefs_changed = false,
        }
        if prefs_changed {
            ev.push(FilesEvent::Prefs);
        }
        self.scroll = self.scroll.max(0.0);
    }

    fn set_sort(&mut self, s: SortBy) {
        if self.prefs.sort == s {
            self.prefs.sort_desc = !self.prefs.sort_desc;
        } else {
            self.prefs.sort = s;
            self.prefs.sort_desc = false;
        }
        self.reload();
    }

    // ---------------------------------------------------------------- input

    pub fn mouse_down(&mut self, p: [f32; 2], right: bool, mods: KeyMods, fm: &mut Fm) -> Vec<FilesEvent> {
        let mut ev = Vec::new();
        // a click anywhere else ends text editing
        if self.edit.is_some() {
            let on_edit = self.hits.iter().any(|(r, h)| matches!(h, Hit::Entry(_)) && inside(p, *r)) && false;
            if !on_edit {
                self.commit_edit(fm, &mut ev);
            }
        }
        let was_path_edit = self.path_edit.is_some();
        let was_filter = self.filter_focus;
        self.path_edit = None;
        self.filter_focus = false;

        let hit = self.hits.iter().rev().find(|(r, _)| inside(p, *r)).map(|(_, h)| h.clone());
        let Some(hit) = hit else { return ev };
        // entries only count inside the visible content area
        if let Hit::Entry(_) = hit {
            if !inside(p, [self.content[0], self.content[1] + self.header_h(), self.content[2], self.content[3] - self.header_h()]) {
                return ev;
            }
        }
        match hit {
            Hit::Back if !right => self.go_back(),
            Hit::Forward if !right => self.go_forward(),
            Hit::Up if !right => self.go_up(),
            Hit::Refresh if !right => self.reload(),
            Hit::Crumb(path) if !right => self.goto(path, true),
            Hit::PathField if !right => {
                self.path_edit = Some(self.cwd.to_string_lossy().to_string());
                let _ = was_path_edit;
            }
            Hit::Search if !right => {
                self.filter_focus = true;
                let _ = was_filter;
            }
            Hit::ViewToggle if !right => {
                self.prefs.view = match self.prefs.view {
                    ViewMode::Icons => ViewMode::Details,
                    ViewMode::Details => ViewMode::Compact,
                    ViewMode::Compact => ViewMode::Icons,
                };
                ev.push(FilesEvent::Prefs);
            }
            Hit::MenuBtn if !right => {
                ev.push(FilesEvent::Menu(self.view_menu(), [p[0] - 120.0, p[1] + 18.0]));
            }
            Hit::Place(path) if !right => {
                if path == fsops::trash_path() {
                    let _ = std::fs::create_dir_all(&path);
                }
                self.goto(path, true);
            }
            Hit::Sort(s) if !right => {
                self.set_sort(s);
                ev.push(FilesEvent::Prefs);
            }
            Hit::Track if !right => {
                let [ty, th] = self.thumb;
                let max = (self.content_h - self.track[3]).max(0.0);
                if self.click_style == crate::theme::ScrollClick::Spot {
                    // the thumb's middle goes to the click
                    let f = ((p[1] - self.track[1] - th / 2.0) / (self.track[3] - th).max(1.0)).clamp(0.0, 1.0);
                    self.scroll = f * max;
                } else if p[1] < ty {
                    self.scroll = (self.scroll - self.track[3]).max(0.0);
                } else if p[1] > ty + th {
                    self.scroll = (self.scroll + self.track[3]).min(max);
                }
            }
            Hit::ZoomOut if !right => {
                self.prefs.icon_size = (self.prefs.icon_size - 8.0).max(40.0);
                ev.push(FilesEvent::Prefs);
            }
            Hit::ZoomIn if !right => {
                self.prefs.icon_size = (self.prefs.icon_size + 8.0).min(128.0);
                ev.push(FilesEvent::Prefs);
            }
            Hit::Action(id) if !right => {
                self.command(id, fm, &mut ev);
            }
            Hit::Entry(vi) => {
                let path = self.entries[self.view[vi]].path.clone();
                if right {
                    if !self.sel.contains(&path) {
                        self.select_only(vi);
                    }
                    ev.push(FilesEvent::Menu(self.entry_menu(fm), p));
                } else if mods.ctrl {
                    if let Some(i) = self.sel.iter().position(|s| *s == path) {
                        self.sel.remove(i);
                    } else {
                        self.sel.push(path.clone());
                    }
                    self.anchor = Some(path);
                } else if mods.shift && self.anchor.is_some() {
                    let a = self.anchor.as_ref().and_then(|a| self.view_pos(a)).unwrap_or(vi);
                    let (lo, hi) = (a.min(vi), a.max(vi));
                    self.sel = (lo..=hi).map(|i| self.entries[self.view[i]].path.clone()).collect();
                } else {
                    let double = self.last_click.as_ref().is_some_and(|(t, last)| *last == path && t.elapsed().as_millis() < 400);
                    if double {
                        self.last_click = None;
                        self.select_only(vi);
                        self.activate(vi, &mut ev);
                    } else {
                        self.last_click = Some((Instant::now(), path.clone()));
                        // pressing on one of several selected files keeps the selection (so it can be dragged together)
                        if !(self.sel.len() > 1 && self.sel.contains(&path)) {
                            self.select_only(vi);
                        }
                        ev.push(FilesEvent::DragStart(self.selected_paths()));
                    }
                }
            }
            Hit::Content => {
                if right {
                    self.sel.clear();
                    ev.push(FilesEvent::Menu(self.background_menu(fm), p));
                } else if !mods.ctrl {
                    self.sel.clear();
                }
            }
            _ => {}
        }
        ev
    }

    pub fn scroll(&mut self, dy: f64) {
        let max = (self.content_h - (self.content[3] - self.header_h())).max(0.0);
        self.scroll = (self.scroll - dy as f32 * 0.6).clamp(0.0, max);
    }

    /// Keyboard input. Returns true when the key was used.
    pub fn key(&mut self, k: Key, mods: KeyMods, fm: &mut Fm) -> (bool, Vec<FilesEvent>) {
        let mut ev = Vec::new();
        // text fields
        if let Some(ed) = self.edit.as_mut() {
            match k {
                Key::Char(c) if !mods.ctrl && !mods.alt => ed.text.push(c),
                Key::Backspace => {
                    ed.text.pop();
                }
                Key::Enter => self.commit_edit(fm, &mut ev),
                Key::Esc => self.edit = None,
                _ => {}
            }
            return (true, ev);
        }
        if let Some(t) = self.path_edit.as_mut() {
            match k {
                Key::Char(c) if !mods.ctrl && !mods.alt => t.push(c),
                Key::Backspace => {
                    t.pop();
                }
                Key::Enter => {
                    let typed = self.path_edit.take().unwrap_or_default();
                    if let Some(url) = fsops::server_url(&typed) {
                        self.connect(url);
                        return (true, ev);
                    }
                    let path = PathBuf::from(typed);
                    if path.is_dir() {
                        self.goto(path, true);
                    } else {
                        fm.note = format!("No such folder: {}", path.display());
                    }
                }
                Key::Esc => self.path_edit = None,
                _ => {}
            }
            return (true, ev);
        }
        if self.filter_focus {
            match k {
                Key::Char(c) if !mods.ctrl && !mods.alt => {
                    self.filter.push(c);
                    self.apply_filter();
                    self.scroll = 0.0;
                }
                Key::Backspace => {
                    self.filter.pop();
                    self.apply_filter();
                }
                Key::Enter | Key::Down => self.filter_focus = false,
                Key::Esc => {
                    self.filter.clear();
                    self.filter_focus = false;
                    self.apply_filter();
                }
                _ => {}
            }
            return (true, ev);
        }

        let cur = self.sel.last().and_then(|p| self.view_pos(p));
        let n = self.view.len();
        let cols = self.cols.max(1);
        let mv = |this: &mut Files, to: Option<usize>| {
            if let Some(t) = to.filter(|&t| t < n) {
                if mods.shift && this.anchor.is_some() {
                    let a = this.anchor.as_ref().and_then(|a| this.view_pos(a)).unwrap_or(t);
                    let (lo, hi) = (a.min(t), a.max(t));
                    this.sel = (lo..=hi).map(|i| this.entries[this.view[i]].path.clone()).collect();
                    this.ensure_visible(t);
                } else {
                    this.select_only(t);
                }
            }
        };
        let grid = self.prefs.view != ViewMode::Details;
        match k {
            Key::Left if mods.alt => self.go_back(),
            Key::Right if mods.alt => self.go_forward(),
            Key::Up if mods.alt => self.go_up(),
            Key::Down => {
                let to = match cur {
                    Some(c) => Some(c + if grid { cols } else { 1 }),
                    None => Some(0),
                };
                mv(self, to.map(|t| t.min(n.saturating_sub(1))));
            }
            Key::Up => {
                let to = match cur {
                    Some(c) => c.checked_sub(if grid { cols } else { 1 }),
                    None => Some(0),
                };
                mv(self, to);
            }
            Key::Right if grid => mv(self, cur.map(|c| c + 1).or(Some(0))),
            Key::Left if grid => mv(self, cur.and_then(|c| c.checked_sub(1)).or(Some(0))),
            Key::Right => {
                if let Some(vi) = cur {
                    if self.entries[self.view[vi]].is_dir {
                        self.activate(vi, &mut ev);
                    }
                }
            }
            Key::Left => self.go_up(),
            Key::Home => mv(self, Some(0)),
            Key::End => mv(self, n.checked_sub(1)),
            Key::PageDown => mv(self, cur.map(|c| (c + 10 * if grid { cols } else { 1 }).min(n.saturating_sub(1)))),
            Key::PageUp => mv(self, cur.map(|c| c.saturating_sub(10 * if grid { cols } else { 1 }))),
            Key::Enter => {
                if let Some(vi) = cur {
                    self.activate(vi, &mut ev);
                }
            }
            Key::Backspace => self.go_up(),
            Key::F2 => self.start_rename(),
            Key::F5 => self.reload(),
            Key::Delete => {
                if mods.shift {
                    self.command(cmd::DELETE_FOREVER, fm, &mut ev);
                } else {
                    self.command(cmd::TRASH, fm, &mut ev);
                }
            }
            Key::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.apply_filter();
                } else if !self.sel.is_empty() {
                    self.sel.clear();
                } else {
                    return (false, ev);
                }
            }
            Key::Char(c) if mods.ctrl => match c.to_ascii_lowercase() {
                'c' => self.command(cmd::COPY, fm, &mut ev),
                'x' => self.command(cmd::CUT, fm, &mut ev),
                'v' => self.command(cmd::PASTE, fm, &mut ev),
                'z' => self.command(cmd::UNDO, fm, &mut ev),
                'a' => self.command(cmd::SELECT_ALL, fm, &mut ev),
                'h' => self.command(cmd::HIDDEN, fm, &mut ev),
                'n' if mods.shift => self.command(cmd::NEW_FOLDER, fm, &mut ev),
                'f' => self.filter_focus = true,
                'l' => self.path_edit = Some(self.cwd.to_string_lossy().to_string()),
                _ => return (false, ev),
            },
            Key::Char(c) if !mods.alt && !c.is_control() => {
                // type-ahead: jump to the first entry starting with what was typed
                if self.typeahead.1.elapsed().as_millis() > 900 {
                    self.typeahead.0.clear();
                }
                self.typeahead = ({ let mut s = std::mem::take(&mut self.typeahead.0); s.push(c); s }, Instant::now());
                let q = self.typeahead.0.to_lowercase();
                let to = self.view.iter().position(|&i| self.entries[i].name.to_lowercase().starts_with(&q));
                mv(self, to);
            }
            _ => return (false, ev),
        }
        (true, ev)
    }

    // ---------------------------------------------------------------- drawing

    pub fn draw(&mut self, cx: &FilesCtx, origin: [f32; 2]) -> (Vec<Panel>, Vec<Label>) {
        let mut p: Vec<Panel> = Vec::new();
        let mut l: Vec<Label> = Vec::new();
        self.hits.clear();
        if !self.open {
            return (p, l);
        }
        self.poll_mount();
        // follow changes made outside (and by our own operations)
        if cx.fm.gen != self.seen_gen {
            self.seen_gen = cx.fm.gen;
            self.reload();
        } else if cx.time - self.last_poll > 1.0 {
            self.last_poll = cx.time;
            let stamp = std::fs::metadata(&self.cwd).and_then(|m| m.modified()).ok();
            if stamp != self.dir_stamp && self.edit.is_none() {
                self.reload();
            }
        }
        let theme = cx.theme;
        let dark = theme.dark;
        let ink = if dark { [0.95, 0.96, 0.99] } else { [0.10, 0.12, 0.18] };
        let accent = theme.accent;
        // selections follow the chosen highlight colour
        let hl = theme.ui.highlight.unwrap_or(accent);
        let (x, y) = (origin[0], origin[1]);
        let (w, h) = (cx.size[0], cx.size[1]);
        let mouse = self.mouse;
        let prefs = self.prefs.clone();

        // frosted background
        let mut bg = theme.glass.panel(x + w / 2.0, y + h / 2.0, w, h, 0.0);
        bg.fx[0] = bg.fx[0].max(18.0);
        bg.extra[0] = 0.0;
        bg.params[3] = 0.0;
        let op = theme.window.client_opacity;
        bg.tint = if dark { [0.11, 0.11, 0.11, (op - 0.06).max(0.0)] } else { [0.97, 0.98, 1.0, op] };
        bg.color2 = [0.0; 4];
        p.push(bg);
        // everything but the sidebar is opaque (the panels go in once the rows above the sidebar are known)
        let opaque_at = p.len();

        macro_rules! fill {
            ($r:expr, $rad:expr, $c:expr, $a:expr) => {{
                let r: [f32; 4] = $r;
                p.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], $rad, $c, $c).flat().alpha($a));
            }};
        }
        macro_rules! glyph {
            ($cx:expr, $cy:expr, $s:expr, $n:expr, $c:expr, $a:expr) => {
                p.push(Panel::glyph($cx, $cy, $s, $n, $c, $a))
            };
        }
        macro_rules! text {
            ($x:expr, $cy:expr, $t:expr, $s:expr, $bold:expr, $c:expr, $a:expr, $al:expr) => {
                l.push(Label::new($t, $x, $cy + $s * 0.36, $s, $bold, $c, $a, $al))
            };
        }
        // like `text!` for names and paths from the disk (and what the user types): shown exactly as they are
        macro_rules! data {
            ($x:expr, $cy:expr, $t:expr, $s:expr, $bold:expr, $c:expr, $a:expr, $al:expr) => {
                l.push(Label::new($t, $x, $cy + $s * 0.36, $s, $bold, $c, $a, $al).verbatim())
            };
        }
        let meas = cx.measure;
        let mut wanted: Vec<Entry> = Vec::new();
        let drop_hover = self.drop_hover.clone();

        // ---- toolbar ------------------------------------------------------
        let tb = [x, y, w, TB];
        fill!([x, y + TB - 1.0, w, 1.0], 0.0, ink, 0.08);
        let nav = [("arrow-left", Hit::Back, !self.back.is_empty()), ("arrow-right", Hit::Forward, !self.fwd.is_empty()), ("arrow-up", Hit::Up, self.cwd.parent().is_some()), ("refresh-cw", Hit::Refresh, true)];
        let mut bx = x + 10.0;
        for (icon, hit, enabled) in nav {
            let r = [bx, y + 8.0, 32.0, 32.0];
            if enabled && inside(mouse, r) {
                fill!(r, 8.0, ink, 0.10);
            }
            glyph!(r[0] + 16.0, r[1] + 16.0, 17.0, icon, ink, if enabled { 0.9 } else { 0.28 });
            if enabled {
                self.hits.push((r, hit));
            }
            bx += 34.0;
        }
        // right side: search, view toggle, menu
        let menu_r = [x + w - 42.0, y + 8.0, 32.0, 32.0];
        let view_r = [menu_r[0] - 36.0, y + 8.0, 32.0, 32.0];
        let search_r = [view_r[0] - 214.0, y + 9.0, 206.0, 30.0];
        for (r, icon, hit) in [(menu_r, "sliders-horizontal", Hit::MenuBtn), (view_r, match prefs.view { ViewMode::Icons => "layout-grid", ViewMode::Details => "list", ViewMode::Compact => "columns-2" }, Hit::ViewToggle)] {
            if inside(mouse, r) {
                fill!(r, 8.0, ink, 0.10);
            }
            glyph!(r[0] + 16.0, r[1] + 16.0, 17.0, icon, ink, 0.9);
            self.hits.push((r, hit));
        }
        fill!(search_r, 9.0, ink, if self.filter_focus { 0.14 } else { 0.08 });
        if self.filter_focus {
            fill!([search_r[0], search_r[1] + search_r[3] - 2.0, search_r[2], 2.0], 1.0, accent, 0.9);
        }
        glyph!(search_r[0] + 17.0, search_r[1] + 15.0, 14.0, "search", ink, 0.6);
        if self.filter.is_empty() && !self.filter_focus {
            text!(search_r[0] + 34.0, search_r[1] + 15.0, "Filter", 13.0, false, ink, 0.45, Align::Left);
        } else {
            let t = ellipsize(&self.filter, 13.0, search_r[2] - 62.0, meas);
            let caret = if self.filter_focus && ((cx.time * 2.0) as i32 % 2 == 0) { "|" } else { "" };
            data!(search_r[0] + 34.0, search_r[1] + 15.0, format!("{t}{caret}"), 13.0, false, ink, 0.95, Align::Left);
        }
        self.hits.push((search_r, Hit::Search));

        // path bar
        let path_x = bx + 6.0;
        let path_r = [path_x, y + 9.0, search_r[0] - path_x - 10.0, 30.0];
        fill!(path_r, 9.0, ink, if self.path_edit.is_some() { 0.14 } else { 0.07 });
        if let Some(t) = &self.path_edit {
            let caret = if (cx.time * 2.0) as i32 % 2 == 0 { "|" } else { "" };
            let s = ellipsize(&format!("{t}{caret}"), 13.0, path_r[2] - 24.0, meas);
            data!(path_r[0] + 12.0, path_r[1] + 15.0, s, 13.0, false, ink, 0.95, Align::Left);
            fill!([path_r[0], path_r[1] + path_r[3] - 2.0, path_r[2], 2.0], 1.0, accent, 0.9);
        } else if prefs.path_style == PathStyle::Address {
            let s = ellipsize(&self.cwd.to_string_lossy(), 13.0, path_r[2] - 24.0, meas);
            data!(path_r[0] + 12.0, path_r[1] + 15.0, s, 13.0, false, ink, 0.9, Align::Left);
        } else {
            // breadcrumbs: Home › Documents › …, dropping the leftmost parts when too long
            let home = fsops::home();
            let mut parts: Vec<(String, PathBuf)> = Vec::new();
            let (base, rest) = match self.cwd.strip_prefix(&home) {
                Ok(r) => (home.clone(), r.to_path_buf()),
                Err(_) => (PathBuf::from("/"), self.cwd.strip_prefix("/").unwrap_or(&self.cwd).to_path_buf()),
            };
            parts.push((if base == home { "Home".into() } else { "/".into() }, base.clone()));
            let mut acc = base;
            for c in rest.components() {
                acc.push(c);
                parts.push((c.as_os_str().to_string_lossy().to_string(), acc.clone()));
            }
            let widths: Vec<f32> = parts.iter().map(|(n, _)| meas(n, 13.0, false) + 16.0).collect();
            let mut start = 0;
            while start + 1 < parts.len() && widths[start..].iter().sum::<f32>() + 18.0 * (parts.len() - start) as f32 > path_r[2] - 16.0 {
                start += 1;
            }
            let mut px = path_r[0] + 6.0;
            if start > 0 {
                text!(px + 4.0, path_r[1] + 15.0, "…", 13.0, false, ink, 0.6, Align::Left);
                px += 22.0;
            }
            for (i, (name, path)) in parts.iter().enumerate().skip(start) {
                let r = [px, path_r[1] + 3.0, widths[i], 24.0];
                let last = i + 1 == parts.len();
                if inside(mouse, r) {
                    fill!(r, 7.0, ink, 0.12);
                }
                text!(px + 8.0, path_r[1] + 15.0, name.clone(), 13.0, last, ink, if last { 1.0 } else { 0.8 }, Align::Left);
                self.hits.push((r, Hit::Crumb(path.clone())));
                px += widths[i];
                if !last {
                    glyph!(px + 3.0, path_r[1] + 15.0, 12.0, "chevron-right", ink, 0.45);
                    px += 12.0;
                }
            }
        }
        self.hits.push((path_r, Hit::PathField));
        let _ = tb;

        // ---- labelled command row (Explorer style) ----------------------------
        let mut top = y + TB;
        if prefs.toolbar_labels {
            fill!([x, top + CMD - 1.0, w, 1.0], 0.0, ink, 0.06);
            let sel_n = self.sel.len();
            let acts: [(&str, &str, u32, bool); 6] = [
                ("folder-plus", "New", cmd::NEW_FOLDER, true),
                ("scissors", "Cut", cmd::CUT, sel_n > 0),
                ("copy", "Copy", cmd::COPY, sel_n > 0),
                ("clipboard-paste", "Paste", cmd::PASTE, cx.fm.can_paste()),
                ("pencil", "Rename", cmd::RENAME, sel_n == 1),
                ("trash-2", "Delete", cmd::TRASH, sel_n > 0),
            ];
            let mut ax = x + 12.0;
            for (icon, label, id, enabled) in acts {
                let tw = meas(label, 13.0, false);
                let r = [ax, top + 5.0, 40.0 + tw, 30.0];
                if enabled && inside(mouse, r) {
                    fill!(r, 8.0, ink, 0.10);
                }
                let a = if enabled { 0.92 } else { 0.32 };
                glyph!(r[0] + 18.0, r[1] + 15.0, 16.0, icon, if enabled { accent } else { ink }, a);
                text!(r[0] + 33.0, r[1] + 15.0, label, 13.0, false, ink, a, Align::Left);
                if enabled {
                    self.hits.push((r, Hit::Action(id)));
                }
                ax += r[2] + 4.0;
            }
            top += CMD;
        }

        // ---- sidebar ------------------------------------------------------
        let bottom = y + h - if prefs.status_bar { STATUS } else { 0.0 };
        let mut cx0 = x;
        {
            let ground = if dark { [0.11, 0.11, 0.11] } else { [0.97, 0.98, 1.0] };
            let side = if prefs.sidebar { prefs.sidebar_w } else { 0.0 };
            let parts = [[x, y, w, top - y], [x + side, top, w - side, bottom - top], [x, bottom, w, y + h - bottom]];
            let panels: Vec<Panel> = parts.iter().filter(|r| r[2] > 0.0 && r[3] > 0.0).map(|r| Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 0.0, ground, ground).flat()).collect();
            p.splice(opaque_at..opaque_at, panels);
        }
        if prefs.sidebar {
            let sw = prefs.sidebar_w;
            fill!([x, top, sw, bottom - top], 0.0, if dark { [0.0, 0.0, 0.0] } else { [0.5, 0.55, 0.65] }, if dark { 0.14 } else { 0.07 });
            fill!([x + sw - 1.0, top, 1.0, bottom - top], 0.0, ink, 0.07);
            let mut sy = top + 10.0;
            let section = |title: &str, items: Vec<(&str, &str, PathBuf)>, sy: &mut f32, p: &mut Vec<Panel>, l: &mut Vec<Label>, hits: &mut Vec<([f32; 4], Hit)>, cwd: &Path| {
                if prefs.sidebar_headers {
                    l.push(Label::new(title, x + 18.0, *sy + 14.0, 11.0, true, ink, 0.45, Align::Left));
                    *sy += 22.0;
                }
                for (name, icon, path) in items {
                    let r = [x + 8.0, *sy, sw - 16.0, 32.0];
                    let selected = cwd == path;
                    let hov = inside(mouse, r);
                    if drop_hover.as_ref() == Some(&path) {
                        p.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 8.0, accent, accent).flat().alpha(0.45));
                    } else if selected {
                        p.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 8.0, hl, hl).flat().alpha(0.22));
                    } else if hov {
                        p.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 8.0, ink, ink).flat().alpha(0.07));
                    }
                    p.push(Panel::glyph(r[0] + 20.0, r[1] + 16.0, 16.0, icon, if selected { accent } else { ink }, if selected { 1.0 } else { 0.78 }));
                    l.push(Label::new(name, r[0] + 40.0, r[1] + 16.0 + 4.8, 13.5, selected, ink, 0.95, Align::Left));
                    hits.push((r, Hit::Place(path)));
                    *sy += 34.0;
                }
                *sy += 8.0;
            };
            section("PLACES", places(), &mut sy, &mut p, &mut l, &mut self.hits, &self.cwd);
            section("SYSTEM", system_places(), &mut sy, &mut p, &mut l, &mut self.hits, &self.cwd);
            cx0 = x + sw;
        }

        // ---- content ------------------------------------------------------
        let content = [cx0, top, x + w - cx0, bottom - top];
        self.content = content;
        let in_trash = self.cwd.starts_with(fsops::trash_path());
        // header (details)
        let hh = self.header_h();
        let items_view = [content[0], content[1] + hh, content[2], content[3] - hh];
        // column layout for details
        let mut col_x = [0.0f32; 3];
        let mut col_w = [0.0f32; 3];
        if prefs.view == ViewMode::Details {
            let mut right = content[0] + content[2] - 16.0;
            let defs = [(prefs.col_date, 158.0f32, 2usize), (prefs.col_size, 84.0, 1), (prefs.col_kind, 100.0, 0)];
            for (on, wd, i) in defs {
                if on && content[2] > 520.0 {
                    col_w[i] = wd;
                    right -= wd;
                    col_x[i] = right;
                }
            }
            fill!([content[0], content[1] + hh - 1.0, content[2], 1.0], 0.0, ink, 0.07);
            let name_x = content[0] + 16.0;
            let arrow = if prefs.sort_desc { "chevron-down" } else { "chevron-up" };
            let heads: [(&str, SortBy, f32, f32); 4] = [("Name", SortBy::Name, name_x, 200.0), ("Type", SortBy::Kind, col_x[0], col_w[0]), ("Size", SortBy::Size, col_x[1], col_w[1]), ("Modified", SortBy::Modified, col_x[2], col_w[2])];
            for (title, sort, hx, hw) in heads {
                if hw <= 0.0 {
                    continue;
                }
                let r = [hx - 6.0, content[1], hw, hh];
                if inside(mouse, r) {
                    fill!([r[0], r[1] + 3.0, r[2] - 4.0, hh - 6.0], 6.0, ink, 0.08);
                }
                text!(hx, content[1] + hh / 2.0, title, 12.0, true, ink, if prefs.sort == sort { 0.95 } else { 0.55 }, Align::Left);
                if prefs.sort == sort {
                    let tw = meas(title, 12.0, true);
                    glyph!(hx + tw + 12.0, content[1] + hh / 2.0, 12.0, arrow, ink, 0.7);
                }
                self.hits.push((r, Hit::Sort(sort)));
            }
        }

        // item rects (unscrolled), relative to the items view
        let n = self.view.len();
        let (cell_w, cell_h) = match prefs.view {
            ViewMode::Details => (items_view[2], prefs.row_h),
            ViewMode::Icons => (prefs.icon_size + 44.0, prefs.icon_size + 46.0),
            ViewMode::Compact => (236.0, prefs.row_h - 2.0),
        };
        let pad = if prefs.view == ViewMode::Details { 0.0 } else { 8.0 };
        let cols = if prefs.view == ViewMode::Details { 1 } else { (((items_view[2] - pad * 2.0) / cell_w).floor() as usize).max(1) };
        self.cols = cols;
        let rows = (n + cols - 1) / cols;
        self.content_h = rows as f32 * cell_h + pad * 2.0;
        let max_scroll = (self.content_h - items_view[3]).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        self.rects.clear();
        for i in 0..n {
            let (c, r) = (i % cols, i / cols);
            let rx = items_view[0] + pad + c as f32 * cell_w;
            let ry = items_view[1] + pad + r as f32 * cell_h - self.scroll;
            self.rects.push([rx, ry, cell_w, cell_h]);
        }
        // content area is the fallback target for clicks (registered before entries, so entries win)
        self.hits.push((items_view, Hit::Content));

        let t = cx.time;
        for vi in 0..n {
            let r = self.rects[vi];
            if r[1] + r[3] < items_view[1] || r[1] > items_view[1] + items_view[3] {
                continue;
            }
            let e = &self.entries[self.view[vi]];
            let selected = self.sel.contains(&e.path);
            let hover = inside(mouse, r) && inside(mouse, items_view);
            let cut = cx.fm.cut && cx.fm.clip.contains(&e.path);
            let full = fits(r, items_view);
            let sel_rect = if prefs.selection_row && prefs.view == ViewMode::Details { [r[0] + 4.0, r[1] + 1.0, r[2] - 8.0, r[3] - 2.0] } else { [r[0] + 2.0, r[1] + 1.0, r[2] - 4.0, r[3] - 2.0] };
            if selected || hover {
                if let Some(c) = intersect(sel_rect, items_view) {
                    let rad = if prefs.selection_row { 3.0 } else { 9.0 };
                    if selected {
                        fill!(c, rad, hl, 0.24);
                        if prefs.selection_row {
                            if full {
                                fill!([c[0], c[1] + 6.0, 3.0, c[3] - 12.0], 1.5, hl, 1.0);
                            }
                        }
                    } else {
                        fill!(c, rad, ink, 0.07);
                    }
                }
            }
            if e.is_dir && drop_hover.as_ref() == Some(&e.path) {
                if let Some(c) = intersect(sel_rect, items_view) {
                    fill!(c, 9.0, accent, 0.40);
                }
            }
            if !full {
                continue;
            }
            self.hits.push((r, Hit::Entry(vi)));
            let a = if cut { 0.45 } else { 1.0 };
            let tint = e.kind.tint(accent);
            let editing = self.edit.as_ref().is_some_and(|ed| ed.target.as_ref() == Some(&e.path));
            let name_for = |maxw: f32, size: f32| -> String { ellipsize(&e.name, size, maxw, meas) };
            match prefs.view {
                ViewMode::Icons => {
                    let (icx, icy) = (r[0] + r[2] / 2.0, r[1] + 10.0 + prefs.icon_size / 2.0);
                    match (cx.icon)(e, true) {
                        IconState::Cell(c) => p.push(Panel::app_icon(icx, icy, prefs.icon_size * 0.92, c, a)),
                        st => {
                            if st == IconState::Pending {
                                wanted.push(e.clone());
                            }
                            glyph!(icx, icy, (prefs.icon_size * 0.62).max(26.0), e.kind.icon(), tint, a);
                        }
                    }
                    let ty = r[1] + prefs.icon_size + 28.0;
                    if editing {
                        let ed = self.edit.as_ref().unwrap();
                        let caret = if (t * 2.0) as i32 % 2 == 0 { "|" } else { "" };
                        fill!([r[0] + 2.0, ty - 12.0, r[2] - 4.0, 22.0], 6.0, ink, 0.16);
                        data!(r[0] + r[2] / 2.0, ty - 1.0, ellipsize(&format!("{}{caret}", ed.text), 13.0, r[2] - 12.0, meas), 13.0, false, ink, 1.0, Align::Center);
                    } else {
                        data!(r[0] + r[2] / 2.0, ty - 1.0, name_for(r[2] - 8.0, 13.0), 13.0, selected, ink, 0.95 * a, Align::Center);
                    }
                }
                ViewMode::Compact | ViewMode::Details => {
                    let cy = r[1] + r[3] / 2.0;
                    let ix = r[0] + 22.0;
                    match (cx.icon)(e, false) {
                        IconState::Cell(c) => p.push(Panel::app_icon(ix, cy, 22.0, c, a)),
                        st => {
                            if st == IconState::Pending {
                                wanted.push(e.clone());
                            }
                            glyph!(ix, cy, 17.0, e.kind.icon(), tint, a);
                        }
                    }
                    let name_w = if prefs.view == ViewMode::Details {
                        let first_col = col_x.iter().zip(col_w.iter()).filter(|(_, w)| **w > 0.0).map(|(x, _)| *x).fold(f32::MAX, f32::min);
                        (if first_col == f32::MAX { r[0] + r[2] - 16.0 } else { first_col - 8.0 }) - (r[0] + 42.0)
                    } else {
                        r[2] - 52.0
                    };
                    if editing {
                        let ed = self.edit.as_ref().unwrap();
                        let caret = if (t * 2.0) as i32 % 2 == 0 { "|" } else { "" };
                        fill!([r[0] + 36.0, cy - 12.0, name_w.min(280.0), 24.0], 6.0, ink, 0.16);
                        data!(r[0] + 42.0, cy, ellipsize(&format!("{}{caret}", ed.text), 13.5, name_w.min(280.0) - 12.0, meas), 13.5, false, ink, 1.0, Align::Left);
                    } else {
                        data!(r[0] + 42.0, cy, name_for(name_w, 13.5), 13.5, selected, ink, 0.95 * a, Align::Left);
                    }
                    if prefs.view == ViewMode::Details {
                        if col_w[0] > 0.0 {
                            text!(col_x[0], cy, e.kind.label(), 12.5, false, ink, 0.6 * a, Align::Left);
                        }
                        if col_w[1] > 0.0 && !e.is_dir {
                            text!(col_x[1], cy, fsops::human_size(e.size), 12.5, false, ink, 0.6 * a, Align::Left);
                        }
                        if col_w[2] > 0.0 {
                            text!(col_x[2], cy, fsops::format_time(e.modified), 12.5, false, ink, 0.6 * a, Align::Left);
                        }
                    }
                }
            }
        }

        // empty / error state
        if n == 0 {
            let msg = if let Some(e) = &self.error {
                e.clone()
            } else if !self.filter.is_empty() {
                format!("Nothing matches “{}”", self.filter)
            } else if in_trash {
                "The Trash is empty".to_string()
            } else {
                "This folder is empty".to_string()
            };
            glyph!(items_view[0] + items_view[2] / 2.0, items_view[1] + items_view[3] / 2.0 - 18.0, 40.0, if self.error.is_some() { "info" } else { "folder-open" }, ink, 0.25);
            text!(items_view[0] + items_view[2] / 2.0, items_view[1] + items_view[3] / 2.0 + 24.0, msg, 14.0, false, ink, 0.6, Align::Center);
        }

        // scrollbar
        self.click_style = theme.ui.scroll_click;
        if (self.scroll - self.scroll_prev).abs() > 0.01 {
            self.scroll_seen = cx.time;
            self.scroll_prev = self.scroll;
        }
        if max_scroll > 0.0 {
            let th = (items_view[3] * items_view[3] / self.content_h).max(30.0);
            let ty = items_view[1] + (items_view[3] - th) * (self.scroll / max_scroll);
            // "when scrolling" fades the bar out a moment after the last movement
            let show = match theme.ui.scrollbars {
                crate::theme::ScrollShow::Scrolling => (1.0 - (cx.time - self.scroll_seen - 0.9) / 0.4).clamp(0.0, 1.0),
                _ => 1.0,
            };
            if show > 0.0 {
                fill!([items_view[0] + items_view[2] - 7.0, ty, 4.0, th], 2.0, ink, 0.28 * show);
            }
            self.track = [items_view[0] + items_view[2] - 14.0, items_view[1], 14.0, items_view[3]];
            self.thumb = [ty, th];
            self.hits.push((self.track, Hit::Track));
        } else {
            self.track = [0.0; 4];
        }

        // ---- status bar ---------------------------------------------------
        if prefs.status_bar {
            let sr = [x, bottom, w, STATUS];
            fill!([x, bottom, w, 1.0], 0.0, ink, 0.07);
            let sel = self.selected_entries();
            let mut s = if sel.is_empty() {
                let dirs = self.view.iter().filter(|&&i| self.entries[i].is_dir).count();
                let files = n - dirs;
                format!("{} folder{}, {} file{}", dirs, if dirs == 1 { "" } else { "s" }, files, if files == 1 { "" } else { "s" })
            } else {
                let bytes: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                format!("{} selected · {}", sel.len(), fsops::human_size(bytes))
            };
            if !cx.fm.note.is_empty() {
                s = cx.fm.note.clone();
            }
            text!(sr[0] + 16.0, sr[1] + STATUS / 2.0, ellipsize(&s, 12.0, w - 200.0, meas), 12.0, false, ink, 0.65, Align::Left);
            if prefs.view == ViewMode::Icons {
                for (i, (icon, hit)) in [("minus", Hit::ZoomOut), ("plus", Hit::ZoomIn)].into_iter().enumerate() {
                    let r = [x + w - 74.0 + i as f32 * 32.0, bottom + 3.0, 26.0, 24.0];
                    if inside(mouse, r) {
                        fill!(r, 6.0, ink, 0.10);
                    }
                    glyph!(r[0] + 13.0, r[1] + 12.0, 13.0, icon, ink, 0.7);
                    self.hits.push((r, hit));
                }
            }
        }
        self.wanted = wanted;
        (p, l)
    }
}

/// Icon used for a file in other places (desktop).
pub fn kind_icon(k: Kind) -> &'static str {
    k.icon()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type `name` over the selected file's name and press Enter.
    fn rename_to(f: &mut Files, fm: &mut Fm, name: &str) -> Vec<FilesEvent> {
        let mut ev = Vec::new();
        f.command(cmd::RENAME, fm, &mut ev);
        for _ in 0..40 {
            f.key(Key::Backspace, KeyMods::default(), fm);
        }
        for c in name.chars() {
            f.key(Key::Char(c), KeyMods::default(), fm);
        }
        let (_, ev2) = f.key(Key::Enter, KeyMods::default(), fm);
        ev.extend(ev2);
        ev
    }

    fn setup(tag: &str) -> (PathBuf, Files, Fm) {
        let dir = std::env::temp_dir().join(format!("memrium-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255])).save(dir.join("pic.png")).unwrap();
        std::fs::write(dir.join("note.txt"), "x").unwrap();
        let mut f = Files::new(FilesPrefs::default());
        f.goto(dir.clone(), false);
        f.reload();
        (dir.canonicalize().unwrap(), f, Fm::default())
    }

    #[test]
    fn giving_a_picture_another_format_s_ending_asks_what_to_do() {
        let (dir, mut f, mut fm) = setup("ask");
        f.sel = vec![dir.join("pic.png")];
        let ev = rename_to(&mut f, &mut fm, "pic.jpg");
        let Some(FilesEvent::Menu(items, _)) = ev.iter().find(|e| matches!(e, FilesEvent::Menu(..))) else { panic!("no question was asked: {}", ev.len()) };
        assert!(items.iter().any(|i| i.id == cmd::RENAME_CONVERT && i.label == "Convert to JPEG"));
        assert!(items.iter().any(|i| i.id == cmd::RENAME_ONLY));
        // nothing has happened to the file yet
        assert!(dir.join("pic.png").exists() && !dir.join("pic.jpg").exists());
        // convert: a real JPEG appears under the typed name, and the original stays
        f.command(cmd::RENAME_CONVERT, &mut fm, &mut Vec::new());
        assert!(dir.join("pic.png").exists());
        assert_eq!(image::ImageReader::open(dir.join("pic.jpg")).unwrap().with_guessed_format().unwrap().format(), Some(image::ImageFormat::Jpeg));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_renaming_keeps_the_pictures_content() {
        let (dir, mut f, mut fm) = setup("only");
        f.sel = vec![dir.join("pic.png")];
        rename_to(&mut f, &mut fm, "pic.jpg");
        f.command(cmd::RENAME_ONLY, &mut fm, &mut Vec::new());
        assert!(!dir.join("pic.png").exists());
        // the name changed, the bytes are still a PNG
        assert_eq!(image::ImageReader::open(dir.join("pic.jpg")).unwrap().with_guessed_format().unwrap().format(), Some(image::ImageFormat::Png));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_renames_are_not_questioned() {
        let (dir, mut f, mut fm) = setup("plain");
        f.sel = vec![dir.join("pic.png")];
        let ev = rename_to(&mut f, &mut fm, "renamed.png");
        assert!(!ev.iter().any(|e| matches!(e, FilesEvent::Menu(..))));
        assert!(dir.join("renamed.png").exists());
        // a text file given a picture's ending is just renamed too
        f.reload();
        f.sel = vec![dir.join("note.txt")];
        let ev = rename_to(&mut f, &mut fm, "note.png");
        assert!(!ev.iter().any(|e| matches!(e, FilesEvent::Menu(..))));
        assert!(dir.join("note.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
