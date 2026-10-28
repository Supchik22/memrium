//! The shell's cursor as an Xcursor theme (`~/.local/share/icons/Memrium`), so that every program shows the same pointer:
//! those that name a shape (`cursor-shape-v1`) get the shell's drawing anyway, but Xwayland, GTK 3, Electron and games load
//! a theme themselves, and without one agreeing on a name each of them picks its own.
//!
//! The theme is drawn from [`crate::cursor::render`] (the arrow, the text beam, the hand, the busy ring...) in the look and
//! size chosen in Settings, and the session and every program it starts get `XCURSOR_THEME` / `XCURSOR_SIZE`.

use crate::cursor::{render, CursorLook, CursorPrefs, Shape};
use std::path::PathBuf;

pub const THEME: &str = "Memrium";

/// The sizes the files hold: a program takes the nearest to `XCURSOR_SIZE` (times its scale).
const SIZES: [u32; 4] = [24, 32, 48, 64];
/// frames of the busy cursors, and how long each shows (ms)
const FRAMES: u32 = 12;
const DELAY: u32 = 80;

/// The names the toolkits know each shape by (the first is the file, the rest link to it).
fn names(shape: Shape) -> &'static [&'static str] {
    match shape {
        Shape::Default => &["default", "left_ptr", "arrow", "top_left_arrow", "context-menu", "help", "question_arrow", "dnd-none"],
        Shape::Text => &["text", "xterm", "ibeam", "vertical-text"],
        Shape::Pointer => &["pointer", "hand2", "hand1", "hand", "pointing_hand", "alias"],
        Shape::Grab => &["grab", "openhand", "fleur_open"],
        Shape::Grabbing => &["grabbing", "closedhand", "dnd-move"],
        Shape::Move => &["move", "fleur", "all-scroll", "size_all"],
        Shape::ResizeEw => &["ew-resize", "sb_h_double_arrow", "h_double_arrow", "size_hor", "col-resize", "split_h", "e-resize", "w-resize", "left_side", "right_side"],
        Shape::ResizeNs => &["ns-resize", "sb_v_double_arrow", "v_double_arrow", "size_ver", "row-resize", "split_v", "n-resize", "s-resize", "top_side", "bottom_side"],
        Shape::ResizeNesw => &["nesw-resize", "size_bdiag", "fd_double_arrow", "ne-resize", "sw-resize", "top_right_corner", "bottom_left_corner"],
        Shape::ResizeNwse => &["nwse-resize", "size_fdiag", "bd_double_arrow", "nw-resize", "se-resize", "top_left_corner", "bottom_right_corner"],
        Shape::Wait => &["wait", "watch"],
        Shape::Progress => &["progress", "left_ptr_watch", "half-busy"],
        Shape::NotAllowed => &["not-allowed", "crossed_circle", "no-drop", "forbidden", "circle"],
        Shape::Crosshair => &["crosshair", "cross", "tcross", "cell", "plus"],
        Shape::Copy => &["copy", "dnd-copy"],
        Shape::Hidden | Shape::Client => &[],
    }
}

const SHAPES: [Shape; 15] = [
    Shape::Default,
    Shape::Text,
    Shape::Pointer,
    Shape::Grab,
    Shape::Grabbing,
    Shape::Move,
    Shape::ResizeEw,
    Shape::ResizeNs,
    Shape::ResizeNesw,
    Shape::ResizeNwse,
    Shape::Wait,
    Shape::Progress,
    Shape::NotAllowed,
    Shape::Crosshair,
    Shape::Copy,
];

/// One Xcursor file: every size, and every frame of it for a busy cursor. Premultiplied ARGB, little endian.
fn xcursor_file(shape: Shape, look: CursorLook, accent: [f32; 3]) -> Vec<u8> {
    // (nominal size, width, height, hot x, hot y, delay, pixels)
    let mut images: Vec<(u32, u32, u32, u32, u32, u32, Vec<u32>)> = Vec::new();
    for &s in &SIZES {
        let frames = if shape.animated() { FRAMES } else { 1 };
        for f in 0..frames {
            let img = render(shape, s, look, accent, f as f32 / frames as f32);
            let delay = if shape.animated() { DELAY } else { 0 };
            images.push((s, img.size, img.size, img.hot.0.max(0) as u32, img.hot.1.max(0) as u32, delay, img.argb));
        }
    }
    let mut out = Vec::new();
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"Xcur");
    u32le(&mut out, 16);
    u32le(&mut out, 0x10000);
    u32le(&mut out, images.len() as u32);
    // the table of contents: kind, subtype (the nominal size), where the chunk starts
    let mut pos = 16 + 12 * images.len() as u32;
    for im in &images {
        u32le(&mut out, 0xfffd_0002);
        u32le(&mut out, im.0);
        u32le(&mut out, pos);
        pos += 36 + 4 * im.1 * im.2;
    }
    for im in &images {
        for v in [36, 0xfffd_0002, im.0, 1, im.1, im.2, im.3, im.4, im.5] {
            u32le(&mut out, v);
        }
        for p in &im.6 {
            u32le(&mut out, *p);
        }
    }
    out
}

/// `~/.local/share/icons`: where toolkits look for a cursor theme without any further setting.
fn icons_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("icons")
}

fn stamp(prefs: &CursorPrefs, accent: [f32; 3]) -> String {
    format!("v1 {:?} {:.3} {:.3} {:.3}", prefs.look, accent[0], accent[1], accent[2])
}

/// Write the theme (only when the look or the accent changed since it was written).
pub fn write(prefs: &CursorPrefs, accent: [f32; 3]) -> std::io::Result<PathBuf> {
    let dir = icons_dir().join(THEME);
    let mark = dir.join(".stamp");
    let want = stamp(prefs, accent);
    if std::fs::read_to_string(&mark).is_ok_and(|s| s == want) {
        return Ok(dir);
    }
    let cursors = dir.join("cursors");
    let _ = std::fs::remove_dir_all(&cursors);
    std::fs::create_dir_all(&cursors)?;
    for shape in SHAPES {
        let names = names(shape);
        std::fs::write(cursors.join(names[0]), xcursor_file(shape, prefs.look, accent))?;
        for alias in &names[1..] {
            let _ = std::os::unix::fs::symlink(names[0], cursors.join(alias));
        }
    }
    std::fs::write(dir.join("index.theme"), format!("[Icon Theme]\nName={THEME}\nComment=The shell's own pointer\n"))?;
    std::fs::write(mark, want)?;
    Ok(dir)
}

/// What `XCURSOR_SIZE` is: the size in logical pixels.
pub fn size(prefs: &CursorPrefs) -> u32 {
    prefs.size.round().clamp(12.0, 64.0) as u32
}

/// The environment that makes a program use the theme.
pub fn env(prefs: &CursorPrefs) -> Vec<(String, String)> {
    vec![("XCURSOR_THEME".into(), THEME.into()), ("XCURSOR_SIZE".into(), size(prefs).to_string())]
}

/// The X resources for Xwayland programs.
pub fn x_resources(prefs: &CursorPrefs) -> String {
    format!("Xcursor.theme:\t{THEME}\nXcursor.size:\t{}\n", size(prefs))
}

const MARK: &str = "# memrium";

/// Make GTK 3 and 4 use the theme: their `settings.ini` beats the environment. What was there is kept in a comment line
/// and put back by [`restore_gtk`].
pub fn write_gtk(prefs: &CursorPrefs) {
    let home = crate::gtk::config_home();
    let want = [("gtk-cursor-theme-name", THEME.to_string()), ("gtk-cursor-theme-size", size(prefs).to_string())];
    for v in ["gtk-3.0", "gtk-4.0"] {
        let file = home.join(v).join("settings.ini");
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        let text = merge_gtk(&old, &want);
        if text != old {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&file, text);
        }
    }
}

fn merge_gtk(old: &str, want: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut done = vec![false; want.len()];
    let mut in_settings = false;
    let mut flush = |lines: &mut Vec<String>, done: &mut Vec<bool>| {
        for (i, (k, v)) in want.iter().enumerate() {
            if !done[i] {
                lines.push(format!("{k}={v} {MARK}"));
                done[i] = true;
            }
        }
    };
    for l in old.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_settings {
                flush(&mut lines, &mut done);
            }
            in_settings = t == "[Settings]";
        }
        if in_settings {
            if let Some(i) = want.iter().position(|(k, _)| t.starts_with(k) && t[k.len()..].trim_start().starts_with('=')) {
                let ours = t.ends_with(MARK);
                if !ours {
                    lines.push(format!("# memrium-was {t}"));
                }
                lines.push(format!("{}={} {MARK}", want[i].0, want[i].1));
                done[i] = true;
                continue;
            }
        }
        lines.push(l.to_string());
    }
    if !in_settings && done.iter().any(|d| !d) {
        lines.push("[Settings]".into());
        in_settings = true;
    }
    if in_settings {
        flush(&mut lines, &mut done);
    }
    lines.join("\n") + "\n"
}

fn unmerge_gtk(old: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for l in old.lines() {
        if l.trim_end().ends_with(MARK) {
            continue;
        }
        match l.trim().strip_prefix("# memrium-was ") {
            Some(orig) => out.push(orig.to_string()),
            None => out.push(l.to_string()),
        }
    }
    out.join("\n") + "\n"
}

/// Put `settings.ini` back as it was.
pub fn restore_gtk() {
    let home = crate::gtk::config_home();
    for v in ["gtk-3.0", "gtk-4.0"] {
        let file = home.join(v).join("settings.ini");
        let Ok(old) = std::fs::read_to_string(&file) else { continue };
        if old.contains(MARK) {
            let _ = std::fs::write(&file, unmerge_gtk(&old));
        }
    }
}

/// Everything the shell does for the cursor when the session starts or the cursor's look changes. `session` is true in a real
/// session (the user's own `settings.ini` is only touched there).
pub fn export(prefs: &CursorPrefs, accent: [f32; 3], session: bool) {
    if write(prefs, accent).is_err() {
        return;
    }
    if session {
        write_gtk(prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xcursor_header_and_chunks_add_up() {
        let f = xcursor_file(Shape::Default, CursorLook::Light, [0.3, 0.5, 0.9]);
        assert_eq!(&f[..4], b"Xcur");
        let n = u32::from_le_bytes(f[12..16].try_into().unwrap()) as usize;
        assert_eq!(n, SIZES.len());
        // the last chunk ends exactly at the end of the file
        let last = 16 + 12 * (n - 1);
        let pos = u32::from_le_bytes(f[last + 8..last + 12].try_into().unwrap()) as usize;
        let (w, h) = (u32::from_le_bytes(f[pos + 16..pos + 20].try_into().unwrap()) as usize, u32::from_le_bytes(f[pos + 20..pos + 24].try_into().unwrap()) as usize);
        assert_eq!(pos + 36 + 4 * w * h, f.len());
    }

    #[test]
    fn busy_cursor_has_frames() {
        let f = xcursor_file(Shape::Wait, CursorLook::Light, [0.3, 0.5, 0.9]);
        assert_eq!(u32::from_le_bytes(f[12..16].try_into().unwrap()) as usize, SIZES.len() * FRAMES as usize);
    }

    #[test]
    fn gtk_settings_round_trip_keeps_the_users_lines() {
        let old = "[Settings]\ngtk-cursor-blink=true\ngtk-cursor-theme-name=breeze_cursors\ngtk-cursor-theme-size=24\n";
        let want = [("gtk-cursor-theme-name", THEME.to_string()), ("gtk-cursor-theme-size", "28".to_string())];
        let merged = merge_gtk(old, &want);
        assert!(merged.contains("gtk-cursor-theme-name=Memrium # memrium"));
        assert_eq!(merge_gtk(&merged, &want), merged, "a second pass changes nothing");
        assert_eq!(unmerge_gtk(&merged), old);
        // no settings file at all
        let fresh = merge_gtk("", &want);
        assert!(fresh.starts_with("[Settings]") && fresh.contains("gtk-cursor-theme-size=28 # memrium"));
        assert_eq!(unmerge_gtk(&fresh).trim(), "[Settings]");
    }

    #[test]
    fn theme_is_written_with_aliases_and_only_once() {
        let dir = std::env::temp_dir().join(format!("memrium-cursor-test-{}", std::process::id()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        let prefs = CursorPrefs::default();
        let out = write(&prefs, [0.3, 0.5, 0.9]).unwrap();
        for n in ["default", "left_ptr", "text", "xterm", "hand2", "sb_h_double_arrow", "left_ptr_watch", "crossed_circle"] {
            assert!(out.join("cursors").join(n).exists(), "{n}");
        }
        assert!(out.join("index.theme").exists());
        let t = std::fs::metadata(out.join(".stamp")).unwrap().modified().unwrap();
        write(&prefs, [0.3, 0.5, 0.9]).unwrap();
        assert_eq!(std::fs::metadata(out.join(".stamp")).unwrap().modified().unwrap(), t);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
