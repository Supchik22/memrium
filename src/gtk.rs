//! What the GTK theme says about the look: light or dark and the accent colour. Read from the GTK settings files, the
//! theme's own stylesheet (`@define-color accent_bg_color ...`) and, for GNOME's accent choice, `gsettings`.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct GtkLook {
    /// the theme's name as the settings give it ("Adwaita-dark", "Breeze")
    pub name: String,
    pub dark: bool,
    pub accent: Option<[f32; 3]>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

/// The value of `key=` in an ini file's text.
pub fn ini_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// `#rrggbb`, `#rgb`, `rgb(r, g, b)` or `rgba(r, g, b, a)` as straight RGB 0..1.
pub fn parse_color(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_end_matches(';').trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
        return match hex.len() {
            6 | 8 => Some([v(0, 2)? as f32 / 255.0, v(2, 2)? as f32 / 255.0, v(4, 2)? as f32 / 255.0]),
            3 => Some([v(0, 1)? as f32 * 17.0 / 255.0, v(1, 1)? as f32 * 17.0 / 255.0, v(2, 1)? as f32 * 17.0 / 255.0]),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<f32> = inner.split(',').filter_map(|p| p.trim().trim_end_matches('%').parse().ok()).collect();
    (parts.len() >= 3).then(|| [parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0])
}

/// The `@define-color name value;` lines of a stylesheet.
pub fn define_colors(css: &str) -> HashMap<String, String> {
    css.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("@define-color")?;
            let (name, value) = rest.trim().split_once(char::is_whitespace)?;
            Some((name.to_string(), value.trim().trim_end_matches(';').trim().to_string()))
        })
        .collect()
}

/// A colour by name, following `@other` references.
pub fn resolve(name: &str, colors: &HashMap<String, String>) -> Option<[f32; 3]> {
    let mut value = colors.get(name)?.clone();
    for _ in 0..8 {
        match value.strip_prefix('@') {
            Some(next) => value = colors.get(next.trim())?.clone(),
            None => return parse_color(&value),
        }
    }
    None
}

fn theme_dirs() -> Vec<PathBuf> {
    let mut v = vec![home().join(".themes"), home().join(".local/share/themes"), PathBuf::from("/usr/share/themes"), PathBuf::from("/run/current-system/sw/share/themes")];
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        v.extend(std::env::split_paths(&dirs).map(|d| d.join("themes")));
    }
    v
}

/// The accent a theme's stylesheet names, if it ships one.
fn accent_from_theme(name: &str) -> Option<[f32; 3]> {
    for dir in theme_dirs() {
        for sub in ["gtk-3.0", "gtk-4.0"] {
            let base = dir.join(name).join(sub);
            for file in ["gtk.css", "gtk-dark.css"] {
                let Ok(css) = std::fs::read_to_string(base.join(file)) else { continue };
                let colors = define_colors(&css);
                for key in ["accent_bg_color", "theme_selected_bg_color", "accent_color", "selected_bg_color"] {
                    if let Some(c) = resolve(key, &colors) {
                        return Some(c);
                    }
                }
            }
        }
    }
    None
}

/// What some well-known themes use when they have no stylesheet of their own on disk (they are built in).
fn accent_by_name(name: &str) -> Option<[f32; 3]> {
    let n = name.to_lowercase();
    // themes that come in colours say which in their name (Colloid-Teal, Orchis-Purple-Dark)
    for (word, hex) in [("teal", "#2190a4"), ("blue", "#3584e4"), ("green", "#3a944a"), ("yellow", "#c88800"), ("orange", "#ed5b00"), ("red", "#e62d42"), ("pink", "#d56199"), ("purple", "#9141ac"), ("grey", "#6f8396"), ("gray", "#6f8396")] {
        if n.split(|c: char| !c.is_alphanumeric()).any(|w| w == word) {
            return parse_color(hex);
        }
    }
    let hex = if n.starts_with("adwaita") || n.starts_with("default") { "#3584e4" } else if n.starts_with("breeze") { "#3daee9" } else if n.starts_with("yaru") { "#e95420" } else if n.starts_with("nordic") || n.starts_with("nord") { "#88c0d0" } else if n.starts_with("dracula") { "#bd93f9" } else if n.starts_with("catppuccin") { "#cba6f7" } else if n.starts_with("gruvbox") { "#d79921" } else { return None };
    parse_color(hex)
}

/// GNOME's own accent choice (`blue`, `teal`, ...) as the colours of its palette.
pub fn gnome_accent(name: &str) -> Option<[f32; 3]> {
    let hex = match name.trim().trim_matches('\'') {
        "blue" => "#3584e4",
        "teal" => "#2190a4",
        "green" => "#3a944a",
        "yellow" => "#c88800",
        "orange" => "#ed5b00",
        "red" => "#e62d42",
        "pink" => "#d56199",
        "purple" => "#9141ac",
        "slate" => "#6f8396",
        _ => return None,
    };
    parse_color(hex)
}

/// GNOME's accent choice nearest to a colour (`blue`, `teal`, ...).
pub fn accent_name(c: [f32; 3]) -> &'static str {
    const NAMES: [&str; 9] = ["blue", "teal", "green", "yellow", "orange", "red", "pink", "purple", "slate"];
    // a colour with hardly any saturation is the grey one, whatever its nearest hue is
    let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    if hi <= 0.0 || (hi - lo) / hi < 0.18 {
        return "slate";
    }
    let dist = |n: &&str| gnome_accent(n).map_or(f32::MAX, |g| (0..3).map(|i| (g[i] - c[i]).powi(2)).sum());
    NAMES.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).copied().unwrap_or("blue")
}

/// Put `key=value` into the `[Settings]` section of a GTK settings file, keeping the rest. A file that is a link (kept by
/// a configuration manager) is left alone. True when the file was written.
pub fn set_ini_key(path: &std::path::Path, key: &str, value: &str) -> bool {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let line = format!("{key}={value}");
    let mut out: Vec<String> = Vec::new();
    let (mut in_settings, mut done, mut seen_section) = (false, false, false);
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_settings && !done {
                out.push(line.clone());
                done = true;
            }
            in_settings = t == "[Settings]";
            seen_section |= in_settings;
        } else if in_settings && l.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !seen_section {
            out.push("[Settings]".to_string());
        }
        out.push(line);
    }
    let new = out.join("\n") + "\n";
    if new == text {
        return false;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, new).is_ok()
}

fn gsettings(key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Read the GTK look of this session. None when there is no sign of a GTK theme at all.
pub fn read() -> Option<GtkLook> {
    let mut name = std::env::var("GTK_THEME").ok();
    let mut dark = name.as_deref().is_some_and(|n| n.ends_with(":dark"));
    name = name.map(|n| n.split(':').next().unwrap_or("").to_string());
    for ver in ["gtk-4.0", "gtk-3.0"] {
        if let Ok(text) = std::fs::read_to_string(config_home().join(ver).join("settings.ini")) {
            if name.is_none() {
                name = ini_value(&text, "gtk-theme-name");
            }
            if ini_value(&text, "gtk-application-prefer-dark-theme").is_some_and(|v| v == "true" || v == "1") {
                dark = true;
            }
        }
    }
    // GNOME keeps the same things in gsettings (and the accent, which a stylesheet does not)
    let scheme = gsettings("color-scheme");
    if let Some(s) = &scheme {
        dark |= s.contains("prefer-dark");
    }
    if name.is_none() {
        name = gsettings("gtk-theme").map(|n| n.trim_matches('\'').to_string());
    }
    let name = name.filter(|n| !n.is_empty())?;
    dark |= name.to_lowercase().contains("dark");
    let accent = gsettings("accent-color").and_then(|a| gnome_accent(&a)).or_else(|| accent_from_theme(&name)).or_else(|| accent_by_name(&name));
    Some(GtkLook { name, dark, accent })
}

/// Put a GTK look on a theme: light or dark, and the accent when the GTK theme has one. True when something changed.
pub fn apply(theme: &mut crate::theme::Theme, look: &GtkLook) -> bool {
    let before = (theme.dark, theme.auto_dark, theme.accent);
    theme.auto_dark = false;
    theme.dark = look.dark;
    if let Some(a) = look.accent {
        theme.accent = a;
    }
    before != (theme.dark, theme.auto_dark, theme.accent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_read_in_their_usual_spellings() {
        assert_eq!(parse_color("#ff0000"), Some([1.0, 0.0, 0.0]));
        assert_eq!(parse_color("#0f0"), Some([0.0, 1.0, 0.0]));
        assert_eq!(parse_color("rgb(0, 0, 255);"), Some([0.0, 0.0, 1.0]));
        assert_eq!(parse_color("rgba(255, 255, 255, 0.5)"), Some([1.0, 1.0, 1.0]));
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn references_between_colours_are_followed() {
        let css = "@define-color blue_3 #3584e4;\n@define-color accent_bg_color @blue_3;\n@define-color loop_a @loop_b;\n@define-color loop_b @loop_a;\n";
        let colors = define_colors(css);
        let c = resolve("accent_bg_color", &colors).unwrap();
        assert!((c[0] - 0.208).abs() < 0.01 && (c[2] - 0.894).abs() < 0.01);
        assert_eq!(resolve("loop_a", &colors), None);
        assert_eq!(resolve("missing", &colors), None);
    }

    #[test]
    fn the_theme_name_comes_from_the_ini() {
        let ini = "[Settings]\ngtk-theme-name = Adwaita-dark\ngtk-application-prefer-dark-theme=1\n";
        assert_eq!(ini_value(ini, "gtk-theme-name").as_deref(), Some("Adwaita-dark"));
        assert_eq!(ini_value(ini, "gtk-application-prefer-dark-theme").as_deref(), Some("1"));
        assert_eq!(ini_value(ini, "nothing"), None);
    }

    #[test]
    fn a_colour_in_the_theme_name_is_the_accent() {
        let teal = accent_by_name("Colloid-Teal-Dark").unwrap();
        assert!(teal[1] > teal[0]);
        assert!(accent_by_name("Adwaita-dark").is_some());
        assert!(accent_by_name("Mystery").is_none());
    }

    #[test]
    fn accent_names_are_the_nearest_palette_colour() {
        assert_eq!(accent_name(parse_color("#3584e4").unwrap()), "blue");
        assert_eq!(accent_name(parse_color("#e8334a").unwrap()), "red");
        assert_eq!(accent_name(parse_color("#c0c0c0").unwrap()), "slate");
    }

    #[test]
    fn ini_keys_are_set_in_place() {
        let dir = std::env::temp_dir().join(format!("memrium-ini-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("settings.ini");
        assert!(set_ini_key(&f, "gtk-application-prefer-dark-theme", "true"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "[Settings]\ngtk-application-prefer-dark-theme=true\n");
        std::fs::write(&f, "[Other]\na=1\n[Settings]\ngtk-font-name=Foo 10\ngtk-application-prefer-dark-theme=false\n").unwrap();
        assert!(set_ini_key(&f, "gtk-application-prefer-dark-theme", "true"));
        assert!(!set_ini_key(&f, "gtk-application-prefer-dark-theme", "true"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "[Other]\na=1\n[Settings]\ngtk-font-name=Foo 10\ngtk-application-prefer-dark-theme=true\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gnome_accents_have_colours() {
        assert!(gnome_accent("'orange'").is_some());
        assert!(gnome_accent("'plaid'").is_none());
    }
}
