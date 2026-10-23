//! Input device preferences (pointer speed and acceleration, scrolling, tapping). Stored in the config,
//! and imported per device from KDE's `~/.config/kcminputrc` so a new session feels like the old one.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Accel {
    /// libinput's default curve (speeds up with pointer velocity)
    #[default]
    Adaptive,
    /// constant ratio, no acceleration
    Flat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputPrefs {
    /// take the settings of each device from KDE's kcminputrc when it has any
    pub import_kde: bool,
    /// -1.0 (slowest) .. 1.0 (fastest)
    pub pointer_speed: f32,
    pub accel: Accel,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    pub left_handed: bool,
    /// keyboard repeat: delay (ms) and rate (keys per second)
    pub repeat_delay: i32,
    pub repeat_rate: i32,
    /// keyboard layouts, e.g. "us,ua"; empty = take them from the system
    pub xkb_layout: String,
    pub xkb_variant: String,
    /// e.g. "grp:win_space_toggle"
    pub xkb_options: String,
    /// how the cursor looks
    pub cursor: crate::cursor::CursorPrefs,
}

impl Default for InputPrefs {
    fn default() -> Self {
        InputPrefs { import_kde: true, pointer_speed: 0.0, accel: Accel::Adaptive, natural_scroll: false, tap_to_click: true, left_handed: false, repeat_delay: 300, repeat_rate: 30, xkb_layout: String::new(), xkb_variant: String::new(), xkb_options: String::new(), cursor: Default::default() }
    }
}

/// Settings found in kcminputrc for one device (only what is present).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceCfg {
    pub speed: Option<f32>,
    pub accel: Option<Accel>,
    pub natural_scroll: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub left_handed: Option<bool>,
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Look up `[Libinput][vendor][product][name]` in kcminputrc text.
pub fn kde_device(text: &str, vendor: u32, product: u32) -> Option<DeviceCfg> {
    let head = format!("[Libinput][{vendor}][{product}][");
    let mut inside = false;
    let mut found = false;
    let mut cfg = DeviceCfg::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.starts_with(&head);
            found |= inside;
            continue;
        }
        if !inside {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "PointerAcceleration" => cfg.speed = v.trim().parse().ok(),
            // libinput_config_accel_profile: 1 = flat, 2 = adaptive
            "PointerAccelerationProfile" => {
                cfg.accel = match v.trim() {
                    "1" => Some(Accel::Flat),
                    "2" => Some(Accel::Adaptive),
                    _ => None,
                }
            }
            "PointerAccelerationProfileFlat" => cfg.accel = parse_bool(v).map(|f| if f { Accel::Flat } else { Accel::Adaptive }),
            "NaturalScroll" => cfg.natural_scroll = parse_bool(v),
            "TapToClick" => cfg.tap_to_click = parse_bool(v),
            "LeftHanded" => cfg.left_handed = parse_bool(v),
            _ => {}
        }
    }
    found.then_some(cfg)
}

/// The effective settings for a device: the config, overridden by KDE's per-device values when importing.
pub fn effective(prefs: &InputPrefs, kde: Option<&DeviceCfg>) -> (f32, Accel, bool, bool, bool) {
    let mut speed = prefs.pointer_speed;
    let mut accel = prefs.accel;
    let mut natural = prefs.natural_scroll;
    let mut tap = prefs.tap_to_click;
    let mut left = prefs.left_handed;
    if prefs.import_kde {
        if let Some(k) = kde {
            speed = k.speed.unwrap_or(speed);
            accel = k.accel.unwrap_or(accel);
            natural = k.natural_scroll.unwrap_or(natural);
            tap = k.tap_to_click.unwrap_or(tap);
            left = k.left_handed.unwrap_or(left);
        }
    }
    (speed.clamp(-1.0, 1.0), accel, natural, tap, left)
}

pub fn kcminputrc() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_to_string(format!("{home}/.config/kcminputrc")).unwrap_or_default()
}

/// What a layout code is called in a list: `ua` is Ukrainian. Unknown codes are shown as they are.
pub fn layout_name(code: &str) -> String {
    let n = match code {
        "us" => "English (US)",
        "gb" => "English (UK)",
        "ua" => "Українська",
        "ru" => "Русский",
        "pl" => "Polski",
        "de" => "Deutsch",
        "fr" => "Français",
        "es" => "Español",
        "it" => "Italiano",
        "cz" => "Čeština",
        "pt" => "Português",
        "tr" => "Türkçe",
        _ => return code.to_uppercase(),
    };
    n.to_string()
}

/// The two or three letters the bar shows for a layout: `us` is `EN`, `ua` is `UA`.
pub fn layout_short(code: &str) -> String {
    match code {
        "us" | "gb" => "EN".into(),
        other => other.to_uppercase(),
    }
}

/// Layouts the Keyboard page offers to add: (xkb code, name).
pub const COMMON_LAYOUTS: [(&str, &str); 18] = [
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("ua", "Українська"),
    ("ru", "Русский"),
    ("pl", "Polski"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("cz", "Čeština"),
    ("sk", "Slovenčina"),
    ("hu", "Magyar"),
    ("ro", "Română"),
    ("tr", "Türkçe"),
    ("se", "Svenska"),
    ("no", "Norsk"),
    ("fi", "Suomi"),
];

/// The ways to switch layouts: (xkb option, what the keys are called). "" is no switch at all.
pub const SWITCH_KEYS: [(&str, &str); 6] = [
    ("grp:win_space_toggle", "Win+Space"),
    ("grp:alt_shift_toggle", "Alt+Shift"),
    ("grp:ctrl_shift_toggle", "Ctrl+Shift"),
    ("grp:alt_space_toggle", "Alt+Space"),
    ("grp:caps_toggle", "Caps Lock"),
    ("", "None"),
];

/// `layout` ("us,ua") and `variant` (",") as pairs.
pub fn layout_entries(layout: &str, variant: &str) -> Vec<(String, String)> {
    let vars: Vec<&str> = variant.split(',').collect();
    layout.split(',').map(str::trim).enumerate().filter(|(_, c)| !c.is_empty()).map(|(i, c)| (c.to_string(), vars.get(i).map(|v| v.trim().to_string()).unwrap_or_default())).collect()
}

/// The inverse: (layout, variant) strings; the variant is empty when no layout has one.
pub fn join_layouts(entries: &[(String, String)]) -> (String, String) {
    let layout = entries.iter().map(|e| e.0.as_str()).collect::<Vec<_>>().join(",");
    let variant = if entries.iter().any(|e| !e.1.is_empty()) { entries.iter().map(|e| e.1.as_str()).collect::<Vec<_>>().join(",") } else { String::new() };
    (layout, variant)
}

/// The layout-switching option among `options`, as an index into `SWITCH_KEYS` (None when it is not one of them).
pub fn switch_index(options: &str) -> usize {
    let has = |o: &str| options.split(',').any(|x| x.trim() == o);
    SWITCH_KEYS.iter().position(|(o, _)| !o.is_empty() && has(o)).unwrap_or(SWITCH_KEYS.len() - 1)
}

/// `options` with the layout switch replaced by choice `i` of `SWITCH_KEYS`; the other options stay.
pub fn with_switch(options: &str, i: usize) -> String {
    let mut out: Vec<String> = options.split(',').map(|s| s.trim().to_string()).filter(|o| !o.is_empty() && !o.starts_with("grp:")).collect();
    if let Some((o, _)) = SWITCH_KEYS.get(i).filter(|(o, _)| !o.is_empty()) {
        out.push(o.to_string());
    }
    out.join(",")
}

/// The keymap to use: the config, else XKB_DEFAULT_* in the environment, else the system's X11
/// keyboard configuration (/etc/X11/xorg.conf.d), else plain "us". Returns (layout, variant, options).
pub fn xkb_settings(prefs: &InputPrefs) -> (String, String, String) {
    if !prefs.xkb_layout.trim().is_empty() {
        return (prefs.xkb_layout.clone(), prefs.xkb_variant.clone(), prefs.xkb_options.clone());
    }
    if let Ok(l) = std::env::var("XKB_DEFAULT_LAYOUT") {
        return (l, std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default(), std::env::var("XKB_DEFAULT_OPTIONS").unwrap_or_default());
    }
    let (mut layout, mut variant, mut options) = (String::new(), String::new(), String::new());
    if let Ok(rd) = std::fs::read_dir("/etc/X11/xorg.conf.d") {
        for e in rd.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            for line in text.lines() {
                let t = line.trim();
                let Some(rest) = t.strip_prefix("Option") else { continue };
                let mut parts = rest.split('"').filter(|p| !p.trim().is_empty());
                let (Some(k), Some(v)) = (parts.next(), parts.next()) else { continue };
                match k {
                    "XkbLayout" => layout = v.to_string(),
                    "XkbVariant" => variant = v.to_string(),
                    "XkbOptions" => options = v.to_string(),
                    _ => {}
                }
            }
        }
    }
    if layout.is_empty() {
        layout = "us".into();
    }
    (layout, variant, options)
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn layouts_round_trip_and_the_switch_is_replaced() {
        let e = layout_entries("us,ua", ",");
        assert_eq!(e, vec![("us".to_string(), String::new()), ("ua".to_string(), String::new())]);
        assert_eq!(join_layouts(&e), ("us,ua".to_string(), String::new()));
        let v = layout_entries("us,ua", ",winkeys");
        assert_eq!(join_layouts(&v), ("us,ua".to_string(), ",winkeys".to_string()));
        assert_eq!(switch_index("grp:alt_shift_toggle,caps:escape"), 1);
        assert_eq!(switch_index("caps:escape"), SWITCH_KEYS.len() - 1);
        assert_eq!(with_switch("grp:alt_shift_toggle,caps:escape", 0), "caps:escape,grp:win_space_toggle");
        assert_eq!(with_switch("grp:alt_shift_toggle", SWITCH_KEYS.len() - 1), "");
    }
}
