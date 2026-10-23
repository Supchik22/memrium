//! Several monitors: which one is the main one, where the others sit, how big the interface is on each.
//!
//! The shell lays its interface (menu bar, Dock, panels, notifications) out on the *main* monitor, whose top-left
//! corner is the origin of the desktop; the other monitors extend the desktop to the left or right of it with
//! negative or larger x. Windows can be dragged across all of them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// the other monitors are left of the main one
    #[default]
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayPrefs {
    /// connector name of the main monitor ("eDP-1"); empty = the laptop panel, else the first one
    pub primary: String,
    pub side: Side,
    /// user interface scale for every monitor; 0 = choose from the main monitor's size
    pub scale: f32,
    /// connectors that are switched off
    pub disabled: Vec<String>,
    /// chosen mode per connector, "1920x1080@60"; missing = the monitor's preferred mode
    pub modes: BTreeMap<String, String>,
    /// connector names, left to right, of the monitors beside the main one (missing ones follow in name order)
    pub order: Vec<String>,
}

impl Default for DisplayPrefs {
    fn default() -> Self {
        DisplayPrefs { primary: String::new(), side: Side::Left, scale: 0.0, disabled: Vec::new(), modes: BTreeMap::new(), order: Vec::new() }
    }
}

/// What the machine reports about one connected monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    /// modes as (width, height, refresh Hz), preferred first
    pub modes: Vec<(u32, u32, u32)>,
    pub mm: (u32, u32),
}

impl MonitorInfo {
    pub fn internal(&self) -> bool {
        let n = self.name.to_lowercase();
        n.starts_with("edp") || n.starts_with("lvds") || n.starts_with("dsi")
    }
}

/// "1920x1080@60"
pub fn mode_text(m: (u32, u32, u32)) -> String {
    format!("{}x{}@{}", m.0, m.1, m.2)
}

fn parse_mode(text: &str) -> Option<(u32, u32, u32)> {
    let (size, hz) = text.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, hz.parse().ok()?))
}

/// A monitor placed on the desktop; positions and sizes are device pixels, x / y of the main monitor are 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub hz: u32,
    pub primary: bool,
}

/// The mode a monitor runs in with these preferences.
pub fn chosen_mode(m: &MonitorInfo, prefs: &DisplayPrefs) -> (u32, u32, u32) {
    prefs.modes.get(&m.name).and_then(|t| parse_mode(t)).filter(|c| m.modes.contains(c)).or_else(|| m.modes.first().copied()).unwrap_or((1920, 1080, 60))
}

/// Which monitor is the main one.
pub fn primary_index(monitors: &[MonitorInfo], prefs: &DisplayPrefs) -> usize {
    let enabled = |m: &MonitorInfo| !prefs.disabled.contains(&m.name);
    monitors
        .iter()
        .position(|m| m.name == prefs.primary && enabled(m))
        .or_else(|| monitors.iter().position(|m| m.internal() && enabled(m)))
        .or_else(|| monitors.iter().position(enabled))
        .unwrap_or(0)
}

/// Place the enabled monitors. Tops are aligned (y = 0), the main monitor starts at x = 0.
pub fn place(monitors: &[MonitorInfo], prefs: &DisplayPrefs) -> Vec<Placed> {
    if monitors.is_empty() {
        return Vec::new();
    }
    let main = primary_index(monitors, prefs);
    let mode = |m: &MonitorInfo| chosen_mode(m, prefs);
    let mut others: Vec<&MonitorInfo> = monitors.iter().enumerate().filter(|(i, m)| *i != main && !prefs.disabled.contains(&m.name)).map(|(_, m)| m).collect();
    // the user's order first, the rest by name
    others.sort_by_key(|m| (prefs.order.iter().position(|n| *n == m.name).unwrap_or(usize::MAX), m.name.clone()));
    let pm = &monitors[main];
    let (pw, ph, phz) = mode(pm);
    let mut out = vec![Placed { name: pm.name.clone(), x: 0, y: 0, w: pw, h: ph, hz: phz, primary: true }];
    match prefs.side {
        Side::Left => {
            // the last one in the order is next to the main monitor
            let mut x = 0;
            for m in others.iter().rev() {
                let (w, h, hz) = mode(m);
                x -= w as i32;
                out.push(Placed { name: m.name.clone(), x, y: 0, w, h, hz, primary: false });
            }
        }
        Side::Right => {
            let mut x = pw as i32;
            for m in &others {
                let (w, h, hz) = mode(m);
                out.push(Placed { name: m.name.clone(), x, y: 0, w, h, hz, primary: false });
                x += w as i32;
            }
        }
    }
    out
}

/// The bounding box of the placed monitors: (x0, y0, x1, y1).
pub fn bounds(placed: &[Placed]) -> (i32, i32, i32, i32) {
    let x0 = placed.iter().map(|p| p.x).min().unwrap_or(0);
    let y0 = placed.iter().map(|p| p.y).min().unwrap_or(0);
    let x1 = placed.iter().map(|p| p.x + p.w as i32).max().unwrap_or(0);
    let y1 = placed.iter().map(|p| p.y + p.h as i32).max().unwrap_or(0);
    (x0, y0, x1, y1)
}

/// Interface scale: the configured one, or from the main monitor's pixel density (1.0, 1.25, 1.5 or 2.0).
pub fn ui_scale(main: &MonitorInfo, mode: (u32, u32, u32), prefs: &DisplayPrefs) -> f32 {
    if prefs.scale > 0.0 {
        return prefs.scale.clamp(0.75, 3.0);
    }
    if main.mm.0 == 0 {
        return 1.0;
    }
    let dpi = mode.0 as f32 / (main.mm.0 as f32 / 25.4);
    match dpi {
        d if d >= 200.0 => 2.0,
        d if d >= 150.0 => 1.5,
        d if d >= 125.0 => 1.25,
        _ => 1.0,
    }
}

/// The monitor under a point of the desktop (device px), or the nearest one.
pub fn monitor_at(placed: &[Placed], x: f32, y: f32) -> Option<usize> {
    let inside = |p: &Placed| x >= p.x as f32 && x < (p.x + p.w as i32) as f32 && y >= p.y as f32 && y < (p.y + p.h as i32) as f32;
    placed.iter().position(inside).or_else(|| {
        placed
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let d = |p: &Placed| {
                    let cx = x.clamp(p.x as f32, (p.x + p.w as i32) as f32);
                    let cy = y.clamp(p.y as f32, (p.y + p.h as i32) as f32);
                    (cx - x).hypot(cy - y)
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, w: u32, h: u32) -> MonitorInfo {
        MonitorInfo { name: name.into(), modes: vec![(w, h, 60), (1280, 720, 60)], mm: (344, 194) }
    }

    #[test]
    fn the_laptop_panel_is_the_main_monitor_and_others_go_left() {
        let ms = vec![mon("DP-3", 2560, 1440), mon("eDP-1", 1920, 1080)];
        let p = place(&ms, &DisplayPrefs::default());
        let main = p.iter().find(|m| m.primary).unwrap();
        assert_eq!((main.name.as_str(), main.x), ("eDP-1", 0));
        let ext = p.iter().find(|m| m.name == "DP-3").unwrap();
        assert_eq!(ext.x, -2560);
        assert_eq!(bounds(&p), (-2560, 0, 1920, 1440));
    }

    #[test]
    fn right_side_and_order() {
        let ms = vec![mon("eDP-1", 1920, 1080), mon("DP-1", 1000, 800), mon("DP-2", 1200, 800)];
        let prefs = DisplayPrefs { side: Side::Right, order: vec!["DP-2".into(), "DP-1".into()], ..Default::default() };
        let p = place(&ms, &prefs);
        assert_eq!(p.iter().find(|m| m.name == "DP-2").unwrap().x, 1920);
        assert_eq!(p.iter().find(|m| m.name == "DP-1").unwrap().x, 1920 + 1200);
    }

    #[test]
    fn a_disabled_monitor_is_left_out_and_the_main_one_can_change() {
        let ms = vec![mon("eDP-1", 1920, 1080), mon("DP-3", 2560, 1440)];
        let prefs = DisplayPrefs { disabled: vec!["DP-3".into()], ..Default::default() };
        assert_eq!(place(&ms, &prefs).len(), 1);
        let prefs = DisplayPrefs { primary: "DP-3".into(), ..Default::default() };
        let p = place(&ms, &prefs);
        assert_eq!(p[0].name, "DP-3");
        assert_eq!(p[1].x, -1920);
    }

    #[test]
    fn modes_and_scale() {
        let m = mon("eDP-1", 1920, 1080);
        let mut prefs = DisplayPrefs::default();
        assert_eq!(chosen_mode(&m, &prefs), (1920, 1080, 60));
        prefs.modes.insert("eDP-1".into(), "1280x720@60".into());
        assert_eq!(chosen_mode(&m, &prefs), (1280, 720, 60));
        prefs.modes.insert("eDP-1".into(), "999x1@60".into());
        assert_eq!(chosen_mode(&m, &prefs), (1920, 1080, 60), "an unknown mode falls back");
        // 1920 px over 344 mm is 142 dpi
        assert_eq!(ui_scale(&m, (1920, 1080, 60), &DisplayPrefs::default()), 1.25);
        assert_eq!(ui_scale(&m, (1920, 1080, 60), &DisplayPrefs { scale: 2.0, ..Default::default() }), 2.0);
    }

    #[test]
    fn finding_the_monitor_under_the_pointer() {
        let ms = vec![mon("eDP-1", 1920, 1080), mon("DP-3", 2560, 1440)];
        let p = place(&ms, &DisplayPrefs::default());
        assert_eq!(p[monitor_at(&p, 100.0, 100.0).unwrap()].name, "eDP-1");
        assert_eq!(p[monitor_at(&p, -100.0, 100.0).unwrap()].name, "DP-3");
        // beyond the bottom of the shorter monitor: the nearest one
        assert_eq!(p[monitor_at(&p, 100.0, 1100.0).unwrap()].name, "eDP-1");
    }
}
