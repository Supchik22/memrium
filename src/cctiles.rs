//! The tiles of the Quick Settings as data: each one has a kind, a place and a size on a four-column grid, and
//! options. The layout lives in `config.toml` (`[[cc_tiles]]`); an empty list means the built-in default.
//!
//! ```toml
//! [[cc_tiles]]
//! kind = "media"
//! col = 2
//! row = 0
//! look = "half-record"     # player | record | half-record
//!
//! [[cc_tiles]]
//! kind = "toggle"
//! col = 0
//! row = 3
//! icon = "terminal"
//! action = "terminal"      # a shell action, or "exec:some-program --arg"
//! ```
//!
//! Kinds: `wifi`, `bluetooth` (2 x 1), `media` (2 x 2), `slider` (4 x 1, `target = "display" | "sound"`),
//! `pill` (2 x 1: `icon`, `title`, `subtitle`, `action`, `style = "dark" | "active"`) and `toggle` (1 x 1: `icon`,
//! `action`). This module knows nothing about drawing, so the layout rules can be tested on their own.

use serde::{Deserialize, Serialize};

pub const COLS: u8 = 4;
pub const MAX_ROWS: u8 = 9;
/// Size of a cell and the gap between cells, in logical px.
pub const CELL_W: f32 = 73.0;
pub const CELL_H: f32 = 70.0;
pub const GAP: f32 = 14.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CcTile {
    pub kind: String,
    pub col: u8,
    pub row: u8,
    /// size in cells; 0 = the kind's own size
    #[serde(default)]
    pub w: u8,
    #[serde(default)]
    pub h: u8,
    /// everything else the kind understands
    #[serde(flatten)]
    pub opts: toml::Table,
}

impl CcTile {
    pub fn new(kind: &str, col: u8, row: u8) -> CcTile {
        CcTile { kind: kind.into(), col, row, w: 0, h: 0, opts: toml::Table::new() }
    }

    pub fn with(mut self, key: &str, value: &str) -> CcTile {
        self.opts.insert(key.into(), toml::Value::String(value.into()));
        self
    }

    pub fn opt(&self, key: &str) -> Option<&str> {
        self.opts.get(key).and_then(|v| v.as_str())
    }

    /// Size in cells.
    pub fn span(&self) -> (u8, u8) {
        let (kw, kh) = info(&self.kind).map_or((1, 1), |k| k.span);
        (if self.w == 0 { kw } else { self.w }.clamp(1, COLS), if self.h == 0 { kh } else { self.h }.clamp(1, MAX_ROWS))
    }

    /// The tile's rectangle `[x, y, w, h]` relative to the top-left of the grid.
    pub fn rect(&self) -> [f32; 4] {
        let (w, h) = self.span();
        [self.col as f32 * (CELL_W + GAP), self.row as f32 * (CELL_H + GAP), w as f32 * CELL_W + (w - 1) as f32 * GAP, h as f32 * CELL_H + (h - 1) as f32 * GAP]
    }
}

/// What a kind of tile is.
pub struct KindInfo {
    pub kind: &'static str,
    pub span: (u8, u8),
    /// an option that tapping the tile in edit mode steps through: (key, values)
    pub cycle: Option<(&'static str, &'static [&'static str])>,
}

pub const KINDS: &[KindInfo] = &[
    KindInfo { kind: "wifi", span: (2, 1), cycle: None },
    KindInfo { kind: "bluetooth", span: (2, 1), cycle: None },
    KindInfo { kind: "media", span: (2, 2), cycle: Some(("look", &["player", "record", "half-record", "cover"])) },
    KindInfo { kind: "slider", span: (4, 1), cycle: Some(("target", &["display", "sound"])) },
    KindInfo { kind: "pill", span: (2, 1), cycle: None },
    KindInfo { kind: "toggle", span: (1, 1), cycle: None },
];

pub fn info(kind: &str) -> Option<&'static KindInfo> {
    KINDS.iter().find(|k| k.kind == kind)
}

/// A control that can be added from the gallery.
pub struct Preset {
    pub label: &'static str,
    pub icon: &'static str,
    pub kind: &'static str,
    pub opts: &'static [(&'static str, &'static str)],
}

pub const PRESETS: &[Preset] = &[
    Preset { label: "Wi-Fi", icon: "wifi", kind: "wifi", opts: &[] },
    Preset { label: "Bluetooth", icon: "bluetooth", kind: "bluetooth", opts: &[] },
    Preset { label: "Now Playing", icon: "audio-lines", kind: "media", opts: &[("look", "player")] },
    Preset { label: "Now Playing (record)", icon: "audio-waveform", kind: "media", opts: &[("look", "record")] },
    Preset { label: "Now Playing (half record)", icon: "audio-waveform", kind: "media", opts: &[("look", "half-record")] },
    Preset { label: "Display", icon: "sun", kind: "slider", opts: &[("target", "display")] },
    Preset { label: "Sound", icon: "volume-2", kind: "slider", opts: &[("target", "sound")] },
    Preset { label: "Focus", icon: "moon", kind: "pill", opts: &[("icon", "moon"), ("title", "Focus"), ("style", "active")] },
    Preset { label: "Dark mode", icon: "contrast", kind: "toggle", opts: &[("icon", "contrast"), ("action", "dark-mode")] },
    Preset { label: "Screenshot", icon: "camera", kind: "toggle", opts: &[("icon", "camera"), ("action", "screenshot")] },
    Preset { label: "Lock Screen", icon: "lock", kind: "toggle", opts: &[("icon", "lock"), ("action", "lock")] },
    Preset { label: "Settings", icon: "settings", kind: "toggle", opts: &[("icon", "settings"), ("action", "settings")] },
    Preset { label: "Files", icon: "folder", kind: "toggle", opts: &[("icon", "folder"), ("action", "files")] },
    Preset { label: "Terminal", icon: "terminal", kind: "toggle", opts: &[("icon", "terminal"), ("action", "terminal")] },
    Preset { label: "Show Desktop", icon: "monitor", kind: "toggle", opts: &[("icon", "monitor"), ("action", "show-desktop")] },
    Preset { label: "Launcher", icon: "search", kind: "toggle", opts: &[("icon", "search"), ("action", "launcher")] },
    Preset { label: "Now Playing (cover)", icon: "image", kind: "media", opts: &[("look", "cover")] },
];

impl Preset {
    pub fn tile(&self, col: u8, row: u8) -> CcTile {
        let mut t = CcTile::new(self.kind, col, row);
        for (k, v) in self.opts {
            t = t.with(k, v);
        }
        t
    }

    pub fn span(&self) -> (u8, u8) {
        info(self.kind).map_or((1, 1), |k| k.span)
    }
}

/// The layout a fresh install gets.
pub fn default_layout() -> Vec<CcTile> {
    vec![
        CcTile::new("wifi", 0, 0),
        CcTile::new("bluetooth", 0, 1),
        CcTile::new("media", 2, 0).with("look", "player"),
        PRESETS[7].tile(0, 2),
        CcTile::new("toggle", 2, 2).with("icon", "layout-panel-left"),
        CcTile::new("toggle", 3, 2).with("icon", "screen-share"),
        PRESETS[8].tile(0, 3),
        PRESETS[9].tile(1, 3),
        PRESETS[5].tile(0, 4),
        PRESETS[6].tile(0, 5),
    ]
}

/// `layout` itself, or the default when it is empty.
pub fn or_default(layout: &[CcTile]) -> Vec<CcTile> {
    if layout.is_empty() {
        default_layout()
    } else {
        layout.to_vec()
    }
}

fn overlaps(a: &CcTile, b: &CcTile) -> bool {
    let ((aw, ah), (bw, bh)) = (a.span(), b.span());
    a.col < b.col + bw && b.col < a.col + aw && a.row < b.row + bh && b.row < a.row + ah
}

/// Does `t` lie inside the grid and clear of every other tile (`skip` is left out: the tile being moved)?
pub fn fits(layout: &[CcTile], t: &CcTile, skip: Option<usize>) -> bool {
    let (w, h) = t.span();
    t.col + w <= COLS && t.row + h <= MAX_ROWS && layout.iter().enumerate().all(|(i, o)| Some(i) == skip || !overlaps(t, o))
}

/// The first free place (reading order) for a tile of `w` x `h` cells.
pub fn first_free(layout: &[CcTile], kind: &str, w: u8, h: u8) -> Option<(u8, u8)> {
    for row in 0..MAX_ROWS {
        for col in 0..COLS {
            let mut t = CcTile::new(kind, col, row);
            t.w = w;
            t.h = h;
            if fits(layout, &t, None) {
                return Some((col, row));
            }
        }
    }
    None
}

/// Rows the tiles use (at least one).
pub fn rows(layout: &[CcTile]) -> u8 {
    layout.iter().map(|t| t.row + t.span().1).max().unwrap_or(1)
}

/// The cell whose top-left is nearest to `p` (relative to the grid's top-left).
pub fn cell_near(p: [f32; 2]) -> (u8, u8) {
    let col = (p[0] / (CELL_W + GAP)).round().clamp(0.0, (COLS - 1) as f32) as u8;
    let row = (p[1] / (CELL_H + GAP)).round().clamp(0.0, (MAX_ROWS - 1) as f32) as u8;
    (col, row)
}

/// Step the tile's cycling option (a media tile's look, a slider's target) to its next value.
pub fn cycle(t: &mut CcTile) -> bool {
    let Some((key, values)) = info(&t.kind).and_then(|k| k.cycle) else { return false };
    let cur = t.opt(key).and_then(|v| values.iter().position(|x| *x == v)).unwrap_or(0);
    t.opts.insert(key.into(), toml::Value::String(values[(cur + 1) % values.len()].into()));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_layout_fits_without_overlaps() {
        let l = default_layout();
        for (i, t) in l.iter().enumerate() {
            assert!(fits(&l, t, Some(i)), "{} at {},{} collides", t.kind, t.col, t.row);
        }
    }

    #[test]
    fn the_grid_is_as_wide_as_the_old_hand_made_layout() {
        // two cells and a gap = the 160 px pill, four cells = the 334 px slider
        assert_eq!(CcTile::new("wifi", 0, 0).rect()[2], 160.0);
        assert_eq!(CcTile::new("slider", 0, 0).rect()[2], 334.0);
        // the player card was 154 px tall
        assert_eq!(CcTile::new("media", 2, 0).rect()[3], 154.0);
    }

    #[test]
    fn overlapping_or_outside_tiles_do_not_fit() {
        let l = vec![CcTile::new("media", 2, 0)];
        assert!(!fits(&l, &CcTile::new("toggle", 3, 1), None));
        assert!(fits(&l, &CcTile::new("toggle", 1, 1), None));
        assert!(!fits(&l, &CcTile::new("wifi", 3, 5), None));
        assert!(!fits(&l, &CcTile::new("toggle", 0, MAX_ROWS), None));
    }

    #[test]
    fn a_new_tile_goes_into_the_first_free_place() {
        let l = default_layout();
        assert_eq!(first_free(&l, "toggle", 0, 0), Some((2, 3)));
        // a two-cell pill also fits beside it; a full-width slider has to go below everything
        assert_eq!(first_free(&l, "wifi", 0, 0), Some((2, 3)));
        assert_eq!(first_free(&l, "slider", 0, 0), Some((0, 6)));
        let mut full = l.clone();
        for row in 6..MAX_ROWS {
            full.push(CcTile::new("slider", 0, row));
        }
        assert_eq!(first_free(&full, "slider", 0, 0), None);
    }

    #[test]
    fn dropping_snaps_to_the_nearest_cell() {
        assert_eq!(cell_near([95.0, 10.0]), (1, 0));
        assert_eq!(cell_near([-30.0, 400.0]), (0, 5));
        assert_eq!(cell_near([9999.0, 9999.0]), (COLS - 1, MAX_ROWS - 1));
    }

    #[test]
    fn tapping_steps_through_the_options() {
        let mut t = CcTile::new("media", 2, 0).with("look", "record");
        assert!(cycle(&mut t));
        assert_eq!(t.opt("look"), Some("half-record"));
        assert!(cycle(&mut t));
        assert_eq!(t.opt("look"), Some("cover"));
        assert!(cycle(&mut t));
        assert_eq!(t.opt("look"), Some("player"));
        assert!(!cycle(&mut CcTile::new("wifi", 0, 0)));
    }

    #[test]
    fn the_layout_survives_the_config_file() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Cfg {
            cc_tiles: Vec<CcTile>,
        }
        let cfg = Cfg { cc_tiles: default_layout() };
        let text = toml::to_string(&cfg).unwrap();
        let back: Cfg = toml::from_str(&text).unwrap();
        assert_eq!(back, cfg);
        let hand: Cfg = toml::from_str("[[cc_tiles]]\nkind = \"toggle\"\ncol = 1\nrow = 2\nicon = \"terminal\"\n").unwrap();
        assert_eq!(hand.cc_tiles[0].opt("icon"), Some("terminal"));
        assert_eq!(hand.cc_tiles[0].span(), (1, 1));
    }
}
