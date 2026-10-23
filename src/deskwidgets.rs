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

/// Only the glass clock is resizable.
pub fn scalable(kind: &str) -> bool {
    kind == "glassclock"
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
];

pub fn size(kind: &str) -> [f32; 2] {
    KINDS.iter().find(|(k, ..)| *k == kind).map(|(.., s)| *s).unwrap_or([200.0, 100.0])
}

/// Where a new widget goes: centred on `at`, kept inside `area` (x0, y0, x1, y1).
pub fn place_at(kind: &str, at: [f32; 2], area: [f32; 4]) -> [f32; 2] {
    let s = size(kind);
    [(at[0] - s[0] / 2.0).clamp(area[0], (area[2] - s[0]).max(area[0])), (at[1] - 20.0).clamp(area[1], (area[3] - s[1]).max(area[1]))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_widgets_stay_on_the_desktop() {
        let p = place_at("calendar", [10.0, 10.0], [0.0, 0.0, 1000.0, 800.0]);
        assert_eq!(p[0], 0.0);
        let p = place_at("calendar", [990.0, 790.0], [0.0, 0.0, 1000.0, 800.0]);
        assert_eq!((p[0], p[1]), (1000.0 - 270.0, 800.0 - 262.0));
    }
}
