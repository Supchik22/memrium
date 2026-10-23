//! The shell's colours for KDE programs. They take their colours from a scheme in `~/.config/kdeglobals` (`[Colors:*]`),
//! not from the Qt palette, so the Qt style alone leaves half of Dolphin dark in a light desktop. In a Memrium session the
//! shell writes its scheme there, and a copy as `~/.local/share/color-schemes/Memrium.colors`.
//!
//! `kdeglobals` is shared with Plasma, so it is handled with care:
//! - nothing is written unless this is a Memrium session (`XDG_CURRENT_DESKTOP=Memrium`; a shell started inside another
//!   desktop to try it leaves the files alone), or `MEMRIUM_NO_KDE_COLORS` is set;
//! - the first time, the file is copied to `memrium/kdeglobals.orig`; only the colour sections and `ColorScheme` are
//!   replaced, the rest stays;
//! - when the shell ends (`Guard` is dropped) the copy is put back. After a crash `tools/restore-kdeglobals.sh` does it.

use crate::theme::{config_dir, Theme};
use std::path::PathBuf;

type Rgb = [f32; 3];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn rgb(c: Rgb) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{},{},{}", b(c[0]), b(c[1]), b(c[2]))
}

/// This is a Memrium session: the colours may be written.
pub fn active() -> bool {
    std::env::var_os("MEMRIUM_NO_KDE_COLORS").is_none() && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.eq_ignore_ascii_case("Memrium"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn kdeglobals() -> PathBuf {
    crate::gtk::config_home().join("kdeglobals")
}

fn backup() -> PathBuf {
    config_dir().join("kdeglobals.orig")
}

fn scheme_file() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("color-schemes").join("Memrium.colors")
}

type Section = (String, Vec<(String, String)>);

/// The colour sections of a scheme for a theme (the same colours as the Qt style and the GTK stylesheet).
pub fn sections(theme: &Theme) -> Vec<Section> {
    let c = |r: u8, g: u8, b: u8| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    let dark = theme.dark;
    let (window, view, fg) = if dark { (c(33, 33, 33), c(27, 27, 27), c(238, 238, 238)) } else { (c(247, 249, 255), c(255, 255, 255), c(28, 30, 36)) };
    let button = if dark { c(48, 48, 48) } else { c(229, 232, 240) };
    let tooltip = if dark { c(43, 43, 43) } else { c(255, 255, 255) };
    let accent = theme.accent;
    let faint = mix(window, fg, 0.55);
    let set = |bg: Rgb, alt: Rgb, fgn: Rgb| -> Vec<(String, String)> {
        let kv = |k: &str, v: String| (k.to_string(), v);
        vec![
            kv("BackgroundAlternate", rgb(alt)),
            kv("BackgroundNormal", rgb(bg)),
            kv("DecorationFocus", rgb(accent)),
            kv("DecorationHover", rgb(accent)),
            kv("ForegroundActive", rgb(accent)),
            kv("ForegroundInactive", rgb(faint)),
            kv("ForegroundLink", rgb(accent)),
            kv("ForegroundNegative", rgb(c(218, 68, 83))),
            kv("ForegroundNeutral", rgb(c(246, 116, 0))),
            kv("ForegroundNormal", rgb(fgn)),
            kv("ForegroundPositive", rgb(c(39, 174, 96))),
            kv("ForegroundVisited", rgb(c(155, 89, 182))),
        ]
    };
    let mut selection = set(accent, mix(accent, [0.0; 3], 0.15), [1.0; 3]);
    for (k, v) in selection.iter_mut() {
        // on the selection the decoration and the active text show as the selection's own text
        if k == "DecorationFocus" || k == "DecorationHover" {
            *v = rgb([1.0; 3]);
        }
    }
    let disabled = ["ChangeSelectionColor=", "Color=56,56,56", "ColorAmount=0", "ColorEffect=0", "ContrastAmount=0.65", "ContrastEffect=1", "Enable=", "IntensityAmount=0.1", "IntensityEffect=2"];
    let inactive = ["ChangeSelectionColor=true", "Color=112,111,110", "ColorAmount=0.025", "ColorEffect=2", "ContrastAmount=0.1", "ContrastEffect=2", "Enable=false", "IntensityAmount=0", "IntensityEffect=0"];
    let kv = |l: &[&str]| l.iter().map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap_or_default()).collect::<Vec<_>>();
    vec![
        ("ColorEffects:Disabled".into(), kv(&disabled)),
        ("ColorEffects:Inactive".into(), kv(&inactive)),
        ("Colors:Button".into(), set(button, mix(button, fg, 0.06), fg)),
        ("Colors:Complementary".into(), set(c(26, 33, 51), c(34, 41, 62), c(238, 238, 238))),
        ("Colors:Header".into(), set(window, mix(window, fg, 0.04), fg)),
        ("Colors:Selection".into(), selection),
        ("Colors:Tooltip".into(), set(tooltip, mix(tooltip, fg, 0.05), fg)),
        ("Colors:View".into(), set(view, mix(view, fg, 0.04), fg)),
        ("Colors:Window".into(), set(window, mix(window, fg, 0.04), fg)),
    ]
}

fn render(sections: &[Section]) -> String {
    sections.iter().map(|(n, kv)| format!("[{n}]\n{}\n", kv.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>())).collect::<Vec<_>>().join("\n")
}

/// A scheme file for `color-schemes`.
pub fn scheme_text(theme: &Theme) -> String {
    let mut s = sections(theme);
    s.push(("General".into(), vec![("ColorScheme".into(), "Memrium".into()), ("Name".into(), "Memrium".into())]));
    render(&s)
}

/// `kdeglobals` with the colour sections and `ColorScheme` replaced and everything else kept.
pub fn merge(existing: &str, theme: &Theme) -> String {
    let mut kept: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for line in existing.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            kept.push((t[1..t.len() - 1].to_string(), Vec::new()));
        } else {
            kept.last_mut().unwrap().1.push(line.to_string());
        }
    }
    kept.retain(|(n, _)| !(n.starts_with("Colors:") || n.starts_with("ColorEffects:")));
    // the scheme is named in `[General]` (older KDE) and in `[UiSettings]` (newer: what a program applies at start)
    for section in ["General", "UiSettings"] {
        if !kept.iter().any(|(n, _)| n == section) {
            kept.push((section.to_string(), Vec::new()));
        }
        for (name, lines) in kept.iter_mut().filter(|(n, _)| n == section) {
            let _ = name;
            lines.retain(|l| !(l.starts_with("ColorScheme=") || l.starts_with("ColorSchemeHash=")));
            lines.insert(0, "ColorScheme=Memrium".into());
        }
    }
    let mut out = String::new();
    for (name, lines) in &kept {
        if !name.is_empty() {
            out += &format!("[{name}]\n");
        }
        for l in lines.iter().filter(|l| !l.trim().is_empty()) {
            out += l;
            out.push('\n');
        }
        out.push('\n');
    }
    out.trim_start().to_string() + &render(&sections(theme))
}

fn write_atomic(path: &PathBuf, text: &str) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = path.with_extension("memrium-tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Write the scheme for the current theme (in a Memrium session only).
pub fn export(theme: &Theme) {
    if !active() || !theme.ui.style_apps {
        // not a Memrium session, or the user does not want it: leave Plasma's colours as they are (or give them back)
        if active() {
            restore();
        }
        return;
    }
    let scheme = scheme_text(theme);
    if std::fs::read_to_string(scheme_file()).map_or(true, |old| old != scheme) {
        write_atomic(&scheme_file(), &scheme);
    }
    write_editor_themes(theme.accent);
    let path = kdeglobals();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = merge(&old, theme);
    if new == old {
        return;
    }
    // the first time, keep what Plasma (or the user) had; after a crash the copy is still there and stays the original
    if !backup().exists() {
        let _ = std::fs::create_dir_all(config_dir());
        let _ = std::fs::write(backup(), &old);
    }
    write_atomic(&path, &new);
}

// ---------------------------------------------------------------------------------------------------------------- editor themes

fn hex(c: Rgb) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// A text editor theme (KSyntaxHighlighting JSON, used by Kate, KWrite, KDevelop...) in the colours of the shell: the
/// background and selection of the desktop, a soft palette for the syntax.
pub fn editor_theme(dark: bool, accent: Rgb) -> String {
    let c = |r: u8, g: u8, b: u8| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    let (bg, fg) = if dark { (c(27, 27, 27), c(232, 232, 232)) } else { (c(255, 255, 255), c(28, 30, 36)) };
    // (name, colour, bold, italic)
    let syn: [(&str, Rgb, bool, bool); 28] = if dark {
        [
            ("Keyword", c(187, 154, 247), true, false), ("Function", c(122, 162, 247), false, false), ("Variable", c(192, 202, 245), false, false),
            ("ControlFlow", c(187, 154, 247), true, false), ("Operator", c(137, 221, 255), false, false), ("BuiltIn", c(125, 207, 255), false, false),
            ("Extension", c(122, 162, 247), false, false), ("Preprocessor", c(224, 175, 104), false, false), ("Attribute", c(224, 175, 104), false, false),
            ("Char", c(158, 206, 106), false, false), ("SpecialChar", c(255, 158, 100), false, false), ("String", c(158, 206, 106), false, false),
            ("VerbatimString", c(158, 206, 106), false, false), ("SpecialString", c(255, 158, 100), false, false), ("Import", c(224, 175, 104), false, false),
            ("DataType", c(42, 195, 222), false, false), ("DecVal", c(255, 158, 100), false, false), ("BaseN", c(255, 158, 100), false, false),
            ("Float", c(255, 158, 100), false, false), ("Constant", c(255, 158, 100), false, false), ("Comment", c(107, 119, 148), false, true),
            ("Documentation", c(122, 136, 184), false, true), ("Annotation", c(122, 136, 184), false, true), ("CommentVar", c(122, 136, 184), false, true),
            ("RegionMarker", c(122, 162, 247), false, false), ("Information", c(125, 207, 255), false, false), ("Warning", c(224, 175, 104), false, false),
            ("Error", c(247, 118, 142), false, false),
        ]
    } else {
        [
            ("Keyword", c(130, 80, 223), true, false), ("Function", c(5, 80, 174), false, false), ("Variable", c(36, 41, 47), false, false),
            ("ControlFlow", c(130, 80, 223), true, false), ("Operator", c(9, 105, 218), false, false), ("BuiltIn", c(10, 122, 138), false, false),
            ("Extension", c(5, 80, 174), false, false), ("Preprocessor", c(154, 103, 0), false, false), ("Attribute", c(154, 103, 0), false, false),
            ("Char", c(26, 127, 55), false, false), ("SpecialChar", c(179, 89, 0), false, false), ("String", c(26, 127, 55), false, false),
            ("VerbatimString", c(26, 127, 55), false, false), ("SpecialString", c(179, 89, 0), false, false), ("Import", c(154, 103, 0), false, false),
            ("DataType", c(10, 122, 138), false, false), ("DecVal", c(179, 89, 0), false, false), ("BaseN", c(179, 89, 0), false, false),
            ("Float", c(179, 89, 0), false, false), ("Constant", c(179, 89, 0), false, false), ("Comment", c(110, 119, 129), false, true),
            ("Documentation", c(110, 119, 129), false, true), ("Annotation", c(110, 119, 129), false, true), ("CommentVar", c(110, 119, 129), false, true),
            ("RegionMarker", c(5, 80, 174), false, false), ("Information", c(10, 122, 138), false, false), ("Warning", c(154, 103, 0), false, false),
            ("Error", c(207, 34, 46), false, false),
        ]
    };
    let style = |name: &str, col: Rgb, bold: bool, italic: bool| format!("        \"{name}\": {{ \"text-color\": \"{}\", \"selected-text-color\": \"#ffffff\", \"bold\": {bold}, \"italic\": {italic}, \"underline\": false, \"strike-through\": false }}", hex(col));
    let mut styles = vec![style("Normal", fg, false, false), style("Others", fg, false, false), style("Alert", c(247, 118, 142), true, false)];
    styles.extend(syn.iter().map(|(n, col, b, i)| style(n, *col, *b, *i)));
    let faint = mix(bg, fg, 0.45);
    let line = mix(bg, fg, if dark { 0.05 } else { 0.04 });
    let sel = mix(bg, accent, if dark { 0.45 } else { 0.30 });
    let find = if dark { c(120, 100, 30) } else { c(255, 232, 140) };
    let colors: [(&str, Rgb); 27] = [
        ("BackgroundColor", bg), ("CodeFolding", mix(bg, accent, 0.3)), ("BracketMatching", mix(bg, accent, 0.5)), ("CurrentLine", line), ("IconBorder", mix(bg, fg, 0.03)),
        ("IndentationLine", mix(bg, fg, 0.12)), ("LineNumbers", faint), ("CurrentLineNumber", accent), ("MarkBookmark", c(66, 133, 244)), ("MarkBreakpointActive", c(220, 60, 60)),
        ("MarkBreakpointReached", c(224, 175, 104)), ("MarkBreakpointDisabled", c(150, 150, 150)), ("MarkExecution", accent), ("MarkWarning", c(224, 175, 104)), ("MarkError", c(247, 118, 142)),
        ("ModifiedLines", c(224, 175, 104)), ("ReplaceHighlight", mix(bg, c(60, 190, 110), 0.4)), ("SavedLines", c(60, 190, 110)), ("SearchHighlight", find), ("TextSelection", sel),
        ("Separator", mix(bg, fg, 0.15)), ("SpellChecking", c(247, 118, 142)), ("TabMarker", mix(bg, fg, 0.15)), ("TemplateBackground", mix(bg, fg, 0.04)),
        ("TemplatePlaceholder", mix(bg, accent, 0.25)), ("TemplateFocusedPlaceholder", mix(bg, accent, 0.45)), ("WordWrapMarker", mix(bg, fg, 0.12)),
    ];
    let editor: Vec<String> = colors.iter().map(|(n, col)| format!("        \"{n}\": \"{}\"", hex(*col))).collect();
    let name = if dark { "Memrium Dark" } else { "Memrium Light" };
    format!("{{\n    \"metadata\": {{\n        \"name\": \"{name}\",\n        \"revision\": 1\n    }},\n    \"text-styles\": {{\n{}\n    }},\n    \"editor-colors\": {{\n{}\n    }}\n}}\n", styles.join(",\n"), editor.join(",\n"))
}

fn themes_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("org.kde.syntax-highlighting").join("themes")
}

/// Write the light and dark editor themes where KTextEditor looks for them. Returns the files.
pub fn write_editor_themes(accent: Rgb) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for (dark, file) in [(false, "Memrium-Light.theme"), (true, "Memrium-Dark.theme")] {
        let path = themes_dir().join(file);
        let text = editor_theme(dark, accent);
        if std::fs::read_to_string(&path).map_or(true, |old| old != text) {
            write_atomic(&path, &text);
        }
        out.push(path);
    }
    out
}

/// Put back the `kdeglobals` that was there before the shell wrote its colours.
pub fn restore() {
    if !active() {
        return;
    }
    if let Ok(orig) = std::fs::read_to_string(backup()) {
        write_atomic(&kdeglobals(), &orig);
        let _ = std::fs::remove_file(backup());
    }
}

/// Held while the shell runs: when it ends the colours of Plasma come back.
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
        crate::gtkcss::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        let mut t = crate::theme::preset("macos").unwrap();
        t.dark = false;
        t.accent = [0.0, 0.5, 1.0];
        t
    }

    #[test]
    fn colour_sections_are_replaced_and_the_rest_is_kept() {
        let old = "[ColorEffects:Disabled]\nColor=1,1,1\n\n[Colors:View]\nBackgroundNormal=0,0,0\n\n[Colors:Header][Inactive]\nBackgroundNormal=5,5,5\n\n[General]\nColorSchemeHash=abc\nfont=Noto Sans,10\n\n[KDE]\nwidgetStyle=Breeze\n";
        let new = merge(old, &theme());
        assert!(new.contains("[Colors:View]\nBackgroundAlternate=") && new.contains("BackgroundNormal=255,255,255"));
        assert!(!new.contains("BackgroundNormal=0,0,0") && !new.contains("[Colors:Header][Inactive]"));
        assert!(new.contains("[General]\nColorScheme=Memrium\nfont=Noto Sans,10\n"), "{new}");
        assert!(new.contains("[UiSettings]\nColorScheme=Memrium\n"));
        assert!(!new.contains("ColorSchemeHash"));
        assert!(new.contains("[KDE]\nwidgetStyle=Breeze\n"));
        // doing it again changes nothing
        assert_eq!(merge(&new, &theme()), new);
    }

    #[test]
    fn a_file_without_general_gets_one() {
        assert!(merge("", &theme()).starts_with("[General]\nColorScheme=Memrium\n"), "{}", merge("", &theme()));
        assert!(merge("[KDE]\nx=1\n", &theme()).contains("[General]\nColorScheme=Memrium\n"));
    }

    #[test]
    fn the_editor_theme_has_what_ktexteditor_reads() {
        for dark in [false, true] {
            let t = editor_theme(dark, [0.0, 0.5, 1.0]);
            assert_eq!(t.matches('{').count(), t.matches('}').count());
            for key in ["\"metadata\"", "\"text-styles\"", "\"editor-colors\"", "\"Normal\"", "\"Keyword\"", "\"BackgroundColor\"", "\"TextSelection\"", "\"CurrentLineNumber\": \"#0080ff\""] {
                assert!(t.contains(key), "{key} in {t}");
            }
            assert!(t.contains(if dark { "Memrium Dark" } else { "Memrium Light" }));
            assert!(!t.contains(",\n    }"), "no trailing comma");
        }
    }

    #[test]
    fn the_selection_is_the_accent() {
        let s = sections(&theme());
        let sel = s.iter().find(|(n, _)| n == "Colors:Selection").unwrap();
        assert!(sel.1.contains(&("BackgroundNormal".into(), "0,128,255".into())));
    }
}
