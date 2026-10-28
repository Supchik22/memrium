//! Widgets that sit on the desktop itself (clock, calendar, ...): which ones, and where.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeskWidget {
    pub kind: String,
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
        DeskWidget { kind: String::new(), scale: 1.0, style: 0, h24: true, date: true, x: 0.0, y: 0.0 }
    }
}

/// Names of the glass clock's looks.
pub const CLOCK_STYLES: &[&str] = &["Jelly Glass", "Crystal", "Frosted"];

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
    KINDS.iter().find(|(k, ..)| *k == kind).map(|(.., s)| *s).unwrap_or([200.0, 100.0])
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
