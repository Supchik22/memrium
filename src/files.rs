//! What the file views share: the command ids of their menus (the desktop's icons and the Files program use the same
//! ones), the events a view hands to the shell, and the file manager's preferences with their presets (Dolphin, Icons,
//! Details, Minimal). The file manager itself is the program `memrium_files`; its preferences live in
//! `~/.config/memrium/files.toml`, which it watches, so the shell can change them (a theme template brings its own look).

use crate::fsops::SortBy;
use crate::menu::MenuItem;
use crate::renderer::Panel;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub const ROTATE_LEFT: u32 = 416;
    pub const ROTATE_RIGHT: u32 = 417;
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
            // (the names in the middle are what these presets were called before they were renamed)
            "Icons" | "Finder" => FilesPrefs { view: ViewMode::Icons, sidebar_w: 200.0, icon_size: 72.0, status_bar: false, col_kind: false, ..base },
            "Details" | "Explorer" => FilesPrefs { toolbar_labels: true, path_style: PathStyle::Address, selection_row: true, row_h: 32.0, sidebar_headers: false, ..base },
            "Minimal" => FilesPrefs { view: ViewMode::Compact, sidebar: false, status_bar: false, row_h: 28.0, ..base },
            _ => base,
        }
    }

    /// Where the preferences are kept.
    pub fn path() -> PathBuf {
        crate::theme::config_dir().join("files.toml")
    }

    /// The saved preferences (the default preset when there are none or they cannot be read).
    pub fn load() -> FilesPrefs {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        let _ = std::fs::create_dir_all(crate::theme::config_dir());
        match toml::to_string_pretty(self) {
            Ok(text) => {
                // written whole, then put in place: the program watching it never reads half a file
                let tmp = path.with_extension("toml.new");
                if std::fs::write(&tmp, text).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => eprintln!("files.toml: {e}"),
        }
    }

    /// Switch to preset `name` (a theme template asked for it), keeping whether hidden files are shown, and save it.
    pub fn apply_preset(name: &str) {
        let hidden = Self::load().show_hidden;
        let mut p = FilesPrefs::preset(name);
        p.show_hidden = hidden;
        p.save();
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
    /// show a folder in the file manager (it starts as a program of its own)
    OpenFolder(PathBuf),
    Terminal(PathBuf),
    /// start one of our programs with these arguments (the archive manager for "Extract here" and "Compress")
    Own(&'static str, Vec<String>),
    /// preferences changed; save the config
    Prefs,
}

/// A fill of `rc` that is rounded (by `radius`) only at those of its corners that are corners of the window `win`, so a
/// window's content keeps the window's rounded shape: a rounded box reaching past every side that is not on the window's
/// edge, cut back to `rc`.
pub fn window_fill(rc: [f32; 4], win: [f32; 4], radius: f32, color: [f32; 3], alpha: f32) -> Panel {
    let on = |a: f32, b: f32| (a - b).abs() < 1.0;
    let g = if radius > 0.0 { 2.0 * radius } else { 0.0 };
    let x0 = if on(rc[0], win[0]) { rc[0] } else { rc[0] - g };
    let y0 = if on(rc[1], win[1]) { rc[1] } else { rc[1] - g };
    let x1 = if on(rc[0] + rc[2], win[0] + win[2]) { rc[0] + rc[2] } else { rc[0] + rc[2] + g };
    let y1 = if on(rc[1] + rc[3], win[1] + win[3]) { rc[1] + rc[3] } else { rc[1] + rc[3] + g };
    let p = Panel::solid((x0 + x1) / 2.0, (y0 + y1) / 2.0, x1 - x0, y1 - y0, radius, color, color).flat().alpha(alpha);
    if g > 0.0 { p.cut_to(rc) } else { p }
}
