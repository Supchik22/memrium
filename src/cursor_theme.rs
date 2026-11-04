//! The shell's cursor as an Xcursor theme (`~/.local/share/icons/Memrium`), so that every program shows the same pointer:
//! those that name a shape (`cursor-shape-v1`) get the shell's drawing anyway, but Xwayland, GTK 3, Electron and games load
//! a theme themselves, and without one agreeing on a name each of them picks its own.
//!
//! The theme is drawn from [`crate::cursor::render_with`] (the arrow, the text beam, the hand, the busy ring...) in the look
//! and size chosen in Settings, and the session and every program it starts get `XCURSOR_THEME` / `XCURSOR_SIZE`.
//!
//! An installed Xcursor theme (Breeze, Adwaita, one in `~/.icons`...) can be picked instead: then the shell reads its
//! pictures for its own cursor ([`picture`]) and points the programs at it.

use crate::cursor::{render_with, ClientCursor, CursorImage, CursorPrefs, Shape};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const THEME: &str = "Memrium";

/// The sizes the files hold: a program takes the nearest to `XCURSOR_SIZE` (times its scale).
const SIZES: [u32; 4] = [24, 32, 48, 64];
/// frames of the busy cursors (each shows for a twelfth of a loop at the busy cursor's pace)
const FRAMES: u32 = 12;

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
fn xcursor_file(shape: Shape, prefs: &CursorPrefs, accent: [f32; 3]) -> Vec<u8> {
    // (nominal size, width, height, hot x, hot y, delay, pixels)
    let mut images: Vec<(u32, u32, u32, u32, u32, u32, Vec<u32>)> = Vec::new();
    // the frames span one loop at the pace the session turns the busy cursor
    let delay = ((1000.0 / (prefs.busy_rate() * FRAMES as f32)).round() as u32).max(10);
    for &s in &SIZES {
        let frames = if shape.animated() { FRAMES } else { 1 };
        for f in 0..frames {
            let img = render_with(shape, s, prefs, accent, f as f32 / frames as f32);
            let delay = if shape.animated() { delay } else { 0 };
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

/// Everything that changes the drawn pictures (the size does not: every size is in the files).
fn stamp(prefs: &CursorPrefs, accent: [f32; 3]) -> String {
    let p = prefs;
    format!(
        "v2 {:?} {:.0} {:.2} {:.2} {:.1} {:?} {:?} {:?} {:.2} {:.2} {:?} {:?} {:.3} {:.3} {:.3}",
        p.look, p.hue, p.sat, p.bright, p.outline, p.edge, p.arrow, p.busy, p.busy_speed, p.shadow, p.hand, p.beam, accent[0], accent[1], accent[2]
    )
}

/// Write the theme (only when the look or the accent changed since it was written). The files are made in a folder of
/// their own and swapped in at the end, so a program that loads a cursor meanwhile does not find half a theme.
pub fn write(prefs: &CursorPrefs, accent: [f32; 3]) -> std::io::Result<PathBuf> {
    let dir = icons_dir().join(THEME);
    let mark = dir.join(".stamp");
    let want = stamp(prefs, accent);
    if std::fs::read_to_string(&mark).is_ok_and(|s| s == want) {
        return Ok(dir);
    }
    let cursors = dir.join("cursors");
    let fresh = dir.join(".cursors-new");
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh)?;
    for shape in SHAPES {
        let names = names(shape);
        std::fs::write(fresh.join(names[0]), xcursor_file(shape, prefs, accent))?;
        for alias in &names[1..] {
            let _ = std::os::unix::fs::symlink(names[0], fresh.join(alias));
        }
    }
    let _ = std::fs::remove_dir_all(&cursors);
    std::fs::rename(&fresh, &cursors)?;
    std::fs::write(dir.join("index.theme"), format!("[Icon Theme]\nName={THEME}\nComment=The shell's own pointer\n"))?;
    std::fs::write(mark, want)?;
    Ok(dir)
}

/// What `XCURSOR_SIZE` is: the size in logical pixels.
pub fn size(prefs: &CursorPrefs) -> u32 {
    prefs.size.round().clamp(12.0, 64.0) as u32
}

/// The theme programs are pointed at: the installed one picked in Settings (while it is there), or the shell's own.
pub fn theme_name(prefs: &CursorPrefs) -> String {
    if !prefs.theme.is_empty() && !theme_dirs(&prefs.theme).is_empty() {
        prefs.theme.clone()
    } else {
        THEME.to_string()
    }
}

/// The environment that makes a program use the theme.
pub fn env(prefs: &CursorPrefs) -> Vec<(String, String)> {
    vec![("XCURSOR_THEME".into(), theme_name(prefs)), ("XCURSOR_SIZE".into(), size(prefs).to_string())]
}

/// The X resources for Xwayland programs.
pub fn x_resources(prefs: &CursorPrefs) -> String {
    format!("Xcursor.theme:\t{}\nXcursor.size:\t{}\n", theme_name(prefs), size(prefs))
}

// ---- installed themes

/// The folders cursor themes are looked for in, in libXcursor's order: `~/.icons`, then the XDG data folders.
fn search_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut v = vec![home.join(".icons"), icons_dir()];
    let data = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    v.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("icons")));
    v.extend(["/run/current-system/sw/share/icons", "/usr/share/icons", "/usr/share/pixmaps"].map(PathBuf::from));
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

/// Every folder of the theme `name` (a theme can be spread over several, e.g. one in `~/.icons` and one in /usr/share).
fn theme_dirs(name: &str) -> Vec<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Vec::new();
    }
    search_dirs().into_iter().map(|d| d.join(name)).filter(|d| d.is_dir()).collect()
}

fn scan_installed() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for dir in search_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // "default" only says which theme is the default, and our own is the drawn cursor
            if name == THEME || name == "default" || name.starts_with('.') || names.contains(&name) {
                continue;
            }
            if e.path().join("cursors").is_dir() {
                names.push(name);
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// The cursor themes installed (each has a `cursors` folder), sorted, without the shell's own. Looked for again at most
/// every few seconds (Settings asks every frame it draws).
pub fn installed() -> Vec<String> {
    static CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match &*c {
        Some((at, v)) if at.elapsed() < Duration::from_secs(4) => v.clone(),
        _ => {
            let v = scan_installed();
            *c = Some((Instant::now(), v.clone()));
            v
        }
    }
}

/// The themes `name` builds on (`Inherits=` of its index.theme).
fn inherits(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in theme_dirs(name) {
        let Ok(text) = std::fs::read_to_string(d.join("index.theme")) else { continue };
        for l in text.lines() {
            if let Some(v) = l.trim().strip_prefix("Inherits") {
                if let Some(v) = v.trim_start().strip_prefix('=') {
                    out.extend(v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
                }
            }
        }
    }
    out
}

/// The file of the first of `names` in the theme, or in the themes it builds on.
fn find_cursor(theme: &str, names: &[&str]) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([theme.to_string()]);
    let mut seen = Vec::new();
    while let Some(t) = queue.pop_front() {
        if seen.contains(&t) || seen.len() > 8 {
            continue;
        }
        for d in theme_dirs(&t) {
            for n in names {
                let f = d.join("cursors").join(n);
                if f.is_file() {
                    return Some(f);
                }
            }
        }
        queue.extend(inherits(&t));
        seen.push(t);
    }
    None
}

/// One picture of an Xcursor file.
struct Frame {
    nominal: u32,
    delay: u32,
    pic: ClientCursor,
}

/// The pictures of an Xcursor file (all sizes, all frames, in file order). Empty when it is not one.
fn parse_xcursor(b: &[u8]) -> Vec<Frame> {
    let u = |at: usize| b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let mut out = Vec::new();
    if b.get(..4) != Some(b"Xcur") {
        return out;
    }
    let (Some(head), Some(ntoc)) = (u(4), u(12)) else { return out };
    for i in 0..ntoc.min(1024) as usize {
        let e = head as usize + i * 12;
        let (Some(kind), Some(pos)) = (u(e), u(e + 8)) else { break };
        if kind != 0xfffd_0002 {
            continue;
        }
        let pos = pos as usize;
        let (Some(chead), Some(nominal), Some(w), Some(h), Some(hx), Some(hy), Some(delay)) = (u(pos), u(pos + 8), u(pos + 16), u(pos + 20), u(pos + 24), u(pos + 28), u(pos + 32)) else { continue };
        // (Xcursor allows up to 0x7fff; no cursor needs more than this)
        if w == 0 || h == 0 || w > 512 || h > 512 {
            continue;
        }
        let start = pos + chead as usize;
        let Some(px) = b.get(start..start + 4 * (w * h) as usize) else { continue };
        let argb = px.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]])).collect();
        out.push(Frame { nominal, delay, pic: ClientCursor { w, h, scale: 1, hot: (hx.min(w - 1) as i32, hy.min(h - 1) as i32), argb } });
    }
    out
}

/// The frames of one size of a theme's cursor, ready to be scaled.
struct Loaded {
    nominal: u32,
    frames: Vec<Frame>,
    /// one loop of the animation, ms
    total: u32,
}

/// The pictures of `shape` in `theme`, at the size in the file nearest `px`. Files are read once and kept (a missing
/// shape too, so that it is not looked for on every turn).
fn load(theme: &str, shape: Shape, px: u32) -> Option<Arc<Loaded>> {
    type Cache = HashMap<(String, Shape, u32), Option<Arc<Loaded>>>;
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    let key = (theme.to_string(), shape, px);
    if let Some(hit) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(&key)) {
        return hit.clone();
    }
    let names = names(shape);
    let found = if names.is_empty() { None } else { find_cursor(theme, names) };
    let loaded = found.and_then(|f| std::fs::read(f).ok()).and_then(|bytes| {
        let all = parse_xcursor(&bytes);
        // the nearest size (a bigger one when two are as near: scaling down looks better)
        let nominal = all.iter().map(|f| f.nominal).min_by_key(|&n| ((n as i64 - px as i64).abs() * 2 - (n > px) as i64, n))?;
        let frames: Vec<Frame> = all.into_iter().filter(|f| f.nominal == nominal).collect();
        let total = frames.iter().map(|f| f.delay.max(1)).sum();
        Some(Arc::new(Loaded { nominal, frames, total }))
    });
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let c = c.get_or_insert_with(HashMap::new);
    if c.len() > 256 {
        c.clear();
    }
    c.insert(key, loaded.clone());
    loaded
}

/// The picture of `shape` from the installed theme the preferences name, `px` device pixels big, at `phase` (0 .. 1) of
/// its animation. None when no theme is picked or it has no such shape: then the drawn cursor stands in.
pub fn theme_picture(shape: Shape, px: u32, prefs: &CursorPrefs, phase: f32) -> Option<CursorImage> {
    if prefs.theme.is_empty() {
        return None;
    }
    let l = load(&prefs.theme, shape, px)?;
    let mut t = (phase.rem_euclid(1.0) * l.total as f32) as u32;
    let mut frame = &l.frames[0];
    for f in &l.frames {
        frame = f;
        if t < f.delay.max(1) {
            break;
        }
        t -= f.delay.max(1);
    }
    Some(frame.pic.image(px as f32 / l.nominal.max(1) as f32))
}

/// The cursor picture the shell shows for `shape`, `px` device pixels big: from the picked theme, else drawn.
pub fn picture(shape: Shape, px: u32, prefs: &CursorPrefs, accent: [f32; 3], phase: f32) -> CursorImage {
    theme_picture(shape, px, prefs, phase).unwrap_or_else(|| render_with(shape, px, prefs, accent, phase))
}

/// How the cursor for `shape` moves: loops per second and the number of distinct pictures in a loop. None for a still one.
pub fn animation(shape: Shape, prefs: &CursorPrefs) -> Option<(f32, u32)> {
    if !prefs.theme.is_empty() {
        if let Some(l) = load(&prefs.theme, shape, size(prefs)) {
            if l.frames.len() < 2 {
                return None;
            }
            return Some((1000.0 / l.total.max(1) as f32 * prefs.busy_speed.clamp(0.25, 3.0), (l.frames.len() as u32).min(120)));
        }
    }
    shape.animated().then(|| (prefs.busy_rate(), 24))
}

const MARK: &str = "# memrium";

/// Make GTK 3 and 4 use the theme: their `settings.ini` beats the environment. What was there is kept in a comment line
/// and put back by [`restore_gtk`].
pub fn write_gtk(prefs: &CursorPrefs) {
    let home = crate::gtk::config_home();
    let want = [("gtk-cursor-theme-name", theme_name(prefs)), ("gtk-cursor-theme-size", size(prefs).to_string())];
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
///
/// Drawing the theme takes a moment (a few hundred pictures), and a slider dragged in Settings asks for it on every
/// step: a thread of its own writes it, and only the newest wish when several came meanwhile.
pub fn export(prefs: &CursorPrefs, accent: [f32; 3], session: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WANT: Mutex<Option<(CursorPrefs, [f32; 3])>> = Mutex::new(None);
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if session {
        write_gtk(prefs);
    }
    *WANT.lock().unwrap_or_else(|e| e.into_inner()) = Some((prefs.clone(), accent));
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new().name("cursor-theme".into()).spawn(|| loop {
        let job = WANT.lock().unwrap_or_else(|e| e.into_inner()).take();
        match job {
            Some((p, a)) => {
                if let Err(e) = write(&p, a) {
                    eprintln!("cursor: could not write the cursor theme: {e}");
                }
            }
            None => {
                RUNNING.store(false, Ordering::Release);
                // a wish that came in just now, after the look but before the flag went down
                if WANT.lock().unwrap_or_else(|e| e.into_inner()).is_some() && !RUNNING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                break;
            }
        }
    });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xcursor_header_and_chunks_add_up() {
        let f = xcursor_file(Shape::Default, &CursorPrefs::default(), [0.3, 0.5, 0.9]);
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
        let f = xcursor_file(Shape::Wait, &CursorPrefs::default(), [0.3, 0.5, 0.9]);
        assert_eq!(u32::from_le_bytes(f[12..16].try_into().unwrap()) as usize, SIZES.len() * FRAMES as usize);
    }

    #[test]
    fn xcursor_files_read_back() {
        let prefs = CursorPrefs::default();
        let frames = parse_xcursor(&xcursor_file(Shape::Wait, &prefs, [0.3, 0.5, 0.9]));
        assert_eq!(frames.len(), SIZES.len() * FRAMES as usize);
        let drawn = render_with(Shape::Wait, 32, &prefs, [0.3, 0.5, 0.9], 0.0);
        let f = frames.iter().find(|f| f.nominal == 32).unwrap();
        assert_eq!((f.pic.w, f.pic.hot, &f.pic.argb), (32, drawn.hot, &drawn.argb));
        assert!(f.delay > 0);
        assert!(parse_xcursor(b"nope").is_empty());
        // a cut-off file keeps the pictures that are whole
        let bytes = xcursor_file(Shape::Default, &prefs, [0.3, 0.5, 0.9]);
        assert_eq!(parse_xcursor(&bytes[..bytes.len() - 10]).len(), SIZES.len() - 1);
    }

    #[test]
    fn a_missing_theme_falls_back_to_the_drawing() {
        let prefs = CursorPrefs { theme: "no-such-cursor-theme".into(), ..CursorPrefs::default() };
        assert_eq!(picture(Shape::Default, 32, &prefs, [0.3, 0.5, 0.9], 0.0).argb, render_with(Shape::Default, 32, &prefs, [0.3, 0.5, 0.9], 0.0).argb);
        assert_eq!(theme_name(&prefs), THEME);
        assert_eq!(animation(Shape::Wait, &prefs).map(|a| a.1), Some(24));
        assert_eq!(animation(Shape::Default, &prefs), None);
        // nothing outside the icon folders is reached through the name
        assert!(theme_dirs("../..").is_empty());
    }

    /// `cargo test installed_themes_load -- --ignored --nocapture`: every installed theme's arrow and busy cursor.
    #[test]
    #[ignore]
    fn installed_themes_load() {
        for t in installed() {
            let prefs = CursorPrefs { theme: t.clone(), ..CursorPrefs::default() };
            let arrow = theme_picture(Shape::Default, 35, &prefs, 0.0);
            let busy = animation(Shape::Wait, &prefs);
            eprintln!("{t}: arrow {:?}, busy {busy:?}", arrow.map(|a| (a.size, a.hot)));
        }
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
