//! Widgets that sit on the desktop itself (clock, calendar, ...): which ones, and where.
//!
//! A widget is one of two things. The built-in ones (`KINDS`) are drawn by the shell. An *external* one is a program of
//! its own (any chebrium app) that the shell starts, puts on the desktop under every window and moves for it: its
//! `kind` is `ext:<name>` and its window's app id is `memrium.widget.<name>`. What the shell needs to know about one is
//! in a small manifest, `<name>.toml`, found in `$MEMRIUM_WIDGETS`, `~/.local/share/memrium/widgets`, next to the shell
//! (`../share/memrium/widgets`, or `widgets/manifests` of a checkout) or in `<data dir>/memrium/widgets`:
//!
//! ```toml
//! label = "Disk"                  # shown when the widget is added (default: the name)
//! exec  = "memrium_widget_disk"   # program: next to the shell, or on PATH (default: memrium_widget_<name>)
//! size  = [270, 84]               # logical px, until the window has told its own
//! card  = true                    # the shell draws the glass card behind it (default); false: the program is all of it
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Start of the `kind` of a widget that is a program of its own.
pub const EXT: &str = "ext:";
/// Start of the app id of such a program's window.
pub const APP_ID: &str = "memrium.widget.";

/// An external widget, as its manifest describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ext {
    /// the manifest's file name without `.toml`: `kind` is `ext:<name>`
    pub name: String,
    pub label: String,
    pub exec: String,
    pub size: [f32; 2],
    pub card: bool,
}

impl Ext {
    pub fn kind(&self) -> String {
        format!("{EXT}{}", self.name)
    }

    pub fn app_id(&self) -> String {
        format!("{APP_ID}{}", self.name)
    }
}

#[derive(Deserialize)]
struct ExtFile {
    label: Option<String>,
    exec: Option<String>,
    size: Option<[f32; 2]>,
    card: Option<bool>,
}

/// A manifest's text for the widget `name` (`None`: not valid, or the name is not a plain word).
pub fn parse_ext(name: &str, text: &str) -> Option<Ext> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let f: ExtFile = toml::from_str(text).ok()?;
    let size = f.size.filter(|s| s[0] >= 40.0 && s[1] >= 20.0 && s[0] <= 4000.0 && s[1] <= 4000.0).unwrap_or([240.0, 120.0]);
    Some(Ext {
        name: name.to_string(),
        label: f.label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| name.to_string()),
        exec: f.exec.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| format!("memrium_widget_{name}")),
        size,
        card: f.card.unwrap_or(true),
    })
}

/// Where manifests are looked for, the first place that has a name winning.
pub fn ext_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(list) = std::env::var_os("MEMRIUM_WIDGETS") {
        dirs.extend(std::env::split_paths(&list));
    }
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    dirs.extend(home.map(|h| h.join("memrium/widgets")));
    // installed with the shell (`<prefix>/bin/..` has `share/memrium/widgets`), or a checkout run from `target/<profile>`
    if let Some(bin) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(bin.join("../share/memrium/widgets"));
        dirs.push(bin.join("../../widgets/manifests"));
    }
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(system.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("memrium/widgets")));
    dirs
}

/// The manifests in `dirs`, by name.
pub fn discover_in(dirs: &[PathBuf]) -> Vec<Ext> {
    let mut out: Vec<Ext> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<Ext> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?.strip_suffix(".toml")?.to_string();
                parse_ext(&name, &std::fs::read_to_string(&path).ok()?)
            })
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        for ext in found {
            if !out.iter().any(|o| o.name == ext.name) {
                out.push(ext);
            }
        }
    }
    out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    out
}

/// Every external widget installed.
pub fn discover() -> Vec<Ext> {
    discover_in(&ext_dirs())
}

static REGISTRY: RwLock<Vec<Ext>> = RwLock::new(Vec::new());

/// Tell the rest of the library which external widgets exist (the shell does it once at start): `size` knows their sizes then.
pub fn set_ext(list: Vec<Ext>) {
    if let Ok(mut r) = REGISTRY.write() {
        *r = list;
    }
}

/// The external widgets known to the shell.
pub fn ext_list() -> Vec<Ext> {
    REGISTRY.read().map(|r| r.clone()).unwrap_or_default()
}

pub fn is_ext(kind: &str) -> bool {
    kind.starts_with(EXT)
}

/// The external widget a `kind` names, if it is installed.
pub fn ext_of(kind: &str) -> Option<Ext> {
    let name = kind.strip_prefix(EXT)?;
    REGISTRY.read().ok()?.iter().find(|e| e.name == name).cloned()
}

/// The widget a window's app id belongs to (`memrium.widget.disk` -> `disk`).
pub fn name_of_app_id(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(APP_ID).filter(|n| !n.is_empty())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeskWidget {
    pub kind: String,
    /// which window is this one's (external widgets only: they are told apart by it, not by their place in the list)
    pub uid: u64,
    /// size factor (the glass clock can be made bigger or smaller)
    pub scale: f32,
    /// look of the glass clock: 0 Jelly Glass, 1 Crystal, 2 Frosted
    pub style: u8,
    /// 24-hour clock, and whether the date is shown (clocks)
    pub h24: bool,
    pub date: bool,
    /// top-left on the desktop, logical px
    pub x: f32,
    pub y: f32,
}

impl Default for DeskWidget {
    fn default() -> Self {
        DeskWidget { kind: String::new(), uid: 0, scale: 1.0, style: 0, h24: true, date: true, x: 0.0, y: 0.0 }
    }
}

/// Names of the glass clock's looks.
pub const CLOCK_STYLES: &[&str] = &["Jelly Glass", "Crystal", "Frosted"];

/// Names of the calendar's looks: the month with today as a square, the date large beside a small month, and the old one.
pub const CAL_STYLES: &[&str] = &["Modern", "Date", "Classic"];

/// Names of the visualizer's looks.
pub const VIZ_STYLES: &[&str] = &["Bars", "Mirror", "Dots"];

/// The glass clock and the visualizer are resizable.
pub fn scalable(kind: &str) -> bool {
    kind == "glassclock" || kind == "visualizer"
}

/// (kind, label, size) of every widget that can be added.
pub const KINDS: &[(&str, &str, [f32; 2])] = &[
    ("clock", "Clock", [230.0, 120.0]),
    ("glassclock", "Glass clock", [400.0, 230.0]),
    ("calendar", "Calendar", [270.0, 262.0]),
    ("system", "System", [270.0, 128.0]),
    ("disk", "Disk", [270.0, 84.0]),
    ("weather", "Weather", [230.0, 84.0]),
    ("music", "Now playing", [300.0, 112.0]),
    ("vinyl", "Vinyl record", [300.0, 352.0]),
    ("visualizer", "Music visualizer", [340.0, 140.0]),
];

pub fn size(kind: &str) -> [f32; 2] {
    KINDS.iter().find(|(k, ..)| *k == kind).map(|(.., s)| *s).or_else(|| ext_of(kind).map(|e| e.size)).unwrap_or([200.0, 100.0])
}

/// Where a new widget goes: centred on `at`, kept inside `area` (x0, y0, x1, y1).
pub fn place_at(kind: &str, at: [f32; 2], area: [f32; 4]) -> [f32; 2] {
    let s = size(kind);
    [(at[0] - s[0] / 2.0).clamp(area[0], (area[2] - s[0]).max(area[0])), (at[1] - 20.0).clamp(area[1], (area[3] - s[1]).max(area[1]))]
}

/// How a dragged widget is pulled to its surroundings. The modifier keys pick the mode while dragging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// no key: edges and centres line up with the other widgets and the screen (smart guides, as in a spreadsheet or a design tool)
    Smart,
    /// Ctrl: a 20 px grid
    Grid,
    /// Shift: moves along one line only (the one the pointer went along), and still lines up on it
    Axis,
    /// Ctrl+Shift: sits next to the others with an even gap, and keeps a margin from the screen edge
    Gap,
    /// Alt: nothing pulls
    Free,
}

impl Snap {
    pub fn from_keys(ctrl: bool, shift: bool, alt: bool) -> Snap {
        match (alt, ctrl, shift) {
            (true, ..) => Snap::Free,
            (_, true, true) => Snap::Gap,
            (_, true, false) => Snap::Grid,
            (_, false, true) => Snap::Axis,
            _ => Snap::Smart,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Snap::Smart => "Align",
            Snap::Grid => "Grid",
            Snap::Axis => "One direction",
            Snap::Gap => "Next to each other",
            Snap::Free => "Free",
        }
    }
}

/// A line to draw while a widget is pulled onto it: vertical (`x` fixed) or horizontal (`y` fixed), from `a` to `b` along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub at: f32,
    pub a: f32,
    pub b: f32,
}

pub const GRID: f32 = 20.0;
/// Gap kept between neighbours (and from the screen edge) in `Snap::Gap`.
pub const GAP: f32 = 12.0;
const PULL: f32 = 10.0;

/// The place a widget lands when dragged to `want` (top-left, size `size`): pulled by the others (`rects`, x y w h) and by `area`
/// (x0 y0 x1 y1) according to `mode`. `start` is where the drag began (for `Snap::Axis`). Also the guide lines to show.
pub fn snap(mode: Snap, want: [f32; 2], size: [f32; 2], rects: &[[f32; 4]], area: [f32; 4], start: [f32; 2]) -> ([f32; 2], Vec<Guide>) {
    let mut pos = want;
    let mut guides = Vec::new();
    match mode {
        Snap::Free => {}
        Snap::Grid => {
            pos = [area[0] + ((want[0] - area[0]) / GRID).round() * GRID, area[1] + ((want[1] - area[1]) / GRID).round() * GRID];
        }
        Snap::Smart | Snap::Axis | Snap::Gap => {
            let mut free = [true, true];
            if mode == Snap::Axis {
                // the line the pointer went along
                let horizontal = (want[0] - start[0]).abs() >= (want[1] - start[1]).abs();
                pos = if horizontal { [want[0], start[1]] } else { [start[0], want[1]] };
                free = [horizontal, !horizontal];
            }
            for axis in 0..2 {
                if !free[axis] {
                    continue;
                }
                // (what the widget offers, what it is pulled to, the guide's extent along the other axis)
                let (lo, len) = (pos[axis], size[axis]);
                let mine = [lo, lo + len / 2.0, lo + len];
                let mut best: Option<(f32, f32, Option<[f32; 4]>)> = None; // (distance, shift, the rect it lines up with)
                let mut consider = |offer: f32, target: f32, other: Option<[f32; 4]>| {
                    let d = target - offer;
                    if d.abs() <= PULL && best.map_or(true, |b| d.abs() < b.0.abs()) {
                        best = Some((d, d, other));
                    }
                };
                if mode == Snap::Gap {
                    consider(mine[0], area[axis] + GAP, None);
                    consider(mine[2], area[axis + 2] - GAP, None);
                    for r in rects {
                        let (a, b) = (r[axis], r[axis] + r[axis + 2]);
                        consider(mine[0], b + GAP, Some(*r));
                        consider(mine[2], a - GAP, Some(*r));
                    }
                } else {
                    for (i, &m) in mine.iter().enumerate() {
                        consider(m, [area[axis], (area[axis] + area[axis + 2]) / 2.0, area[axis + 2]][i], None);
                        for r in rects {
                            let (a, w) = (r[axis], r[axis + 2]);
                            // the same part of the other widget, and its centre for the centre
                            consider(m, [a, a + w / 2.0, a + w][i], Some(*r));
                            if i != 1 {
                                consider(m, a + w / 2.0, Some(*r));
                            }
                        }
                    }
                }
                if let Some((_, d, other)) = best {
                    pos[axis] += d;
                    // draw the line the widget landed on: along the whole screen edge, or between the two widgets
                    let other_axis = 1 - axis;
                    let (mut a, mut b) = (pos[other_axis], pos[other_axis] + size[other_axis]);
                    if let Some(r) = other {
                        a = a.min(r[other_axis]);
                        b = b.max(r[other_axis] + r[other_axis + 2]);
                    }
                    // the exact line: the one of the three of ours that landed
                    let at = [pos[axis], pos[axis] + len / 2.0, pos[axis] + len].into_iter().min_by(|x, y| {
                        let t = |v: f32| other.map_or(f32::MAX, |r| [r[axis], r[axis] + r[axis + 2] / 2.0, r[axis] + r[axis + 2], r[axis] - GAP, r[axis] + r[axis + 2] + GAP].into_iter().map(|c| (c - v).abs()).fold(f32::MAX, f32::min)).min([area[axis], (area[axis] + area[axis + 2]) / 2.0, area[axis + 2], area[axis] + GAP, area[axis + 2] - GAP].into_iter().map(|c| (c - v).abs()).fold(f32::MAX, f32::min));
                        t(*x).total_cmp(&t(*y))
                    });
                    if mode != Snap::Gap {
                        if let Some(at) = at {
                            guides.push(Guide { vertical: axis == 0, at, a, b });
                        }
                    }
                }
            }
        }
    }
    // never off the screen
    pos[0] = pos[0].clamp(area[0], (area[2] - size[0]).max(area[0]));
    pos[1] = pos[1].clamp(area[1], (area[3] - size[1]).max(area[1]));
    (pos, guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: [f32; 4] = [0.0, 0.0, 1000.0, 800.0];

    #[test]
    fn a_manifest_gives_defaults_for_what_it_leaves_out() {
        let e = parse_ext("disk", "").unwrap();
        assert_eq!((e.label.as_str(), e.exec.as_str(), e.size, e.card), ("disk", "memrium_widget_disk", [240.0, 120.0], true));
        let e = parse_ext("disk", "label = \"Disk\"\nexec = \"my-disk\"\nsize = [270, 84]\ncard = false").unwrap();
        assert_eq!((e.label.as_str(), e.exec.as_str(), e.size, e.card), ("Disk", "my-disk", [270.0, 84.0], false));
        assert_eq!((e.kind().as_str(), e.app_id().as_str()), ("ext:disk", "memrium.widget.disk"));
        // an absurd size is not taken, a name that is not a word is not a widget, nor is text that is not TOML
        assert_eq!(parse_ext("disk", "size = [1, 1]").unwrap().size, [240.0, 120.0]);
        assert!(parse_ext("../x", "").is_none() && parse_ext("", "").is_none() && parse_ext("a b", "").is_none());
        assert!(parse_ext("disk", "label = ").is_none());
    }

    #[test]
    fn manifests_are_found_by_name_and_the_first_folder_wins() {
        let root = std::env::temp_dir().join(format!("memrium-widgets-test-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("disk.toml"), "label = \"Disk A\"").unwrap();
        std::fs::write(b.join("disk.toml"), "label = \"Disk B\"").unwrap();
        std::fs::write(b.join("clock.toml"), "label = \"Clock\"").unwrap();
        std::fs::write(b.join("notes.txt"), "not a manifest").unwrap();
        std::fs::write(b.join("bad name.toml"), "").unwrap();
        let found = discover_in(&[a, b, root.join("missing")]);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(found.iter().map(|e| (e.name.as_str(), e.label.as_str())).collect::<Vec<_>>(), vec![("clock", "Clock"), ("disk", "Disk A")]);
    }

    #[test]
    fn a_window_is_told_by_its_app_id() {
        assert_eq!(name_of_app_id("memrium.widget.disk"), Some("disk"));
        assert_eq!(name_of_app_id("memrium.widget."), None);
        assert_eq!(name_of_app_id("memrium.files"), None);
        assert!(is_ext("ext:disk") && !is_ext("disk"));
    }

    #[test]
    fn the_keys_pick_the_mode() {
        assert_eq!(Snap::from_keys(false, false, false), Snap::Smart);
        assert_eq!(Snap::from_keys(true, false, false), Snap::Grid);
        assert_eq!(Snap::from_keys(false, true, false), Snap::Axis);
        assert_eq!(Snap::from_keys(true, true, false), Snap::Gap);
        assert_eq!(Snap::from_keys(true, true, true), Snap::Free);
    }

    #[test]
    fn a_widget_lines_up_with_the_edge_of_another() {
        let other = [300.0, 100.0, 200.0, 100.0];
        // 6 px off the other's left edge, a long way from its top
        let (p, g) = snap(Snap::Smart, [306.0, 400.0], [100.0, 50.0], &[other], AREA, [0.0; 2]);
        assert_eq!(p, [300.0, 400.0]);
        assert!(g.iter().any(|g| g.vertical && g.at == 300.0));
        // 30 px off: nothing pulls
        let (p, g) = snap(Snap::Smart, [330.0, 400.0], [100.0, 50.0], &[other], AREA, [0.0; 2]);
        assert_eq!(p, [330.0, 400.0]);
        assert!(g.is_empty());
    }

    #[test]
    fn centres_line_up_too() {
        let other = [300.0, 100.0, 200.0, 100.0];
        // centre of the other is x = 400; ours (width 100) is centred at 404
        let (p, _) = snap(Snap::Smart, [354.0, 400.0], [100.0, 50.0], &[other], AREA, [0.0; 2]);
        assert_eq!(p[0], 350.0);
    }

    #[test]
    fn the_grid_rounds_to_twenty() {
        let (p, g) = snap(Snap::Grid, [113.0, 247.0], [100.0, 50.0], &[], AREA, [0.0; 2]);
        assert_eq!(p, [120.0, 240.0]);
        assert!(g.is_empty());
    }

    #[test]
    fn shift_keeps_to_the_line_the_pointer_went_along() {
        let (p, _) = snap(Snap::Axis, [400.0, 130.0], [100.0, 50.0], &[], AREA, [100.0, 100.0]);
        assert_eq!(p, [400.0, 100.0]);
        let (p, _) = snap(Snap::Axis, [130.0, 500.0], [100.0, 50.0], &[], AREA, [100.0, 100.0]);
        assert_eq!(p, [100.0, 500.0]);
    }

    #[test]
    fn neighbours_sit_with_a_gap_and_the_screen_edge_keeps_a_margin() {
        let other = [300.0, 100.0, 200.0, 100.0];
        // our left edge near the other's right edge (500) + gap
        let (p, _) = snap(Snap::Gap, [505.0, 300.0], [100.0, 50.0], &[other], AREA, [0.0; 2]);
        assert_eq!(p[0], 500.0 + GAP);
        // near the left screen edge
        let (p, _) = snap(Snap::Gap, [4.0, 300.0], [100.0, 50.0], &[], AREA, [0.0; 2]);
        assert_eq!(p[0], GAP);
    }

    #[test]
    fn alt_is_free_and_nothing_leaves_the_screen() {
        let other = [300.0, 100.0, 200.0, 100.0];
        let (p, _) = snap(Snap::Free, [303.0, 400.0], [100.0, 50.0], &[other], AREA, [0.0; 2]);
        assert_eq!(p, [303.0, 400.0]);
        let (p, _) = snap(Snap::Free, [-50.0, 900.0], [100.0, 50.0], &[], AREA, [0.0; 2]);
        assert_eq!(p, [0.0, 750.0]);
    }

    #[test]
    fn new_widgets_stay_on_the_desktop() {
        let p = place_at("calendar", [10.0, 10.0], [0.0, 0.0, 1000.0, 800.0]);
        assert_eq!(p[0], 0.0);
        let p = place_at("calendar", [990.0, 790.0], [0.0, 0.0, 1000.0, 800.0]);
        assert_eq!((p[0], p[1]), (1000.0 - 270.0, 800.0 - 262.0));
    }
}
