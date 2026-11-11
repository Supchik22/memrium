//! Which program opens which file, the freedesktop way: the type of a file from the shared MIME database (`mime/globs2`,
//! by name; a peek at the bytes tells text from binary when no name matches), the programs that say they open a type (the
//! `MimeType=` of their desktop files, with the desktop's own viewers added), and the user's choices in `mimeapps.list`
//! ("Open With…" can make one the default). Used by the file manager's Open With menu and by `fsops::open_external`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A program that can open files.
#[derive(Clone, Debug, PartialEq)]
pub struct Handler {
    /// desktop file id without ".desktop" (`org.kde.gwenview`), or one of the desktop's own (`memrium-view`)
    pub id: String,
    pub name: String,
    pub icon: String,
    /// the command line with its field codes (`%f %U ...`) still in it
    pub exec: String,
    pub terminal: bool,
    /// the types it declares (may hold `image/*`)
    pub mimes: Vec<String>,
    /// listed only for files (NoDisplay): kept out of menus of all programs, fine for Open With
    pub hidden: bool,
}

// ---------------------------------------------------------------- the type of a file

struct Globs {
    /// "*.ext" patterns (lower case) -> (weight, type), the most common kind
    suffix: HashMap<String, (u32, String)>,
    /// case-sensitive "*.ext" patterns (`*.C`, `*.H`)
    suffix_cs: HashMap<String, (u32, String)>,
    /// whole names (`Makefile`, `README`)
    literal: HashMap<String, (u32, String)>,
    /// anything else with * or ? in it: (pattern lower case, weight, type)
    other: Vec<(String, u32, String)>,
    /// child type -> parent types
    parents: HashMap<String, Vec<String>>,
    /// alias -> the canonical type
    aliases: HashMap<String, String>,
}

fn mime_dirs() -> Vec<PathBuf> {
    crate::apps::data_dirs().into_iter().map(|d| d.join("mime")).filter(|d| d.join("globs2").exists()).collect()
}

fn load_globs() -> Globs {
    let mut g = Globs { suffix: HashMap::new(), suffix_cs: HashMap::new(), literal: HashMap::new(), other: Vec::new(), parents: HashMap::new(), aliases: HashMap::new() };
    // the first directory wins (the user's own database before the system's), so read them in reverse and let later ones
    // overwrite
    for dir in mime_dirs().into_iter().rev() {
        if let Ok(text) = std::fs::read_to_string(dir.join("globs2")) {
            for line in text.lines() {
                if line.starts_with('#') {
                    continue;
                }
                let mut parts = line.splitn(4, ':');
                let (Some(w), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
                let cs = parts.next().is_some_and(|f| f.split(',').any(|f| f == "cs"));
                let weight: u32 = w.parse().unwrap_or(50);
                let entry = (weight, mime.to_string());
                let keep = |m: &mut HashMap<String, (u32, String)>, k: String| {
                    // for the same pattern the heavier one wins; equal weights: the first line
                    if m.get(&k).is_none_or(|(old, _)| weight > *old) {
                        m.insert(k, entry.clone());
                    }
                };
                if let Some(ext) = glob.strip_prefix("*.").filter(|e| !e.contains(['*', '?', '['])) {
                    if cs {
                        keep(&mut g.suffix_cs, ext.to_string());
                    } else {
                        keep(&mut g.suffix, ext.to_lowercase());
                    }
                } else if !glob.contains(['*', '?', '[']) {
                    keep(&mut g.literal, glob.to_string());
                } else if !glob.contains('[') {
                    g.other.push((glob.to_lowercase(), weight, mime.to_string()));
                }
            }
        }
        for (file, aliases) in [("subclasses", false), ("aliases", true)] {
            if let Ok(text) = std::fs::read_to_string(dir.join(file)) {
                for line in text.lines() {
                    if let Some((a, b)) = line.split_once(' ') {
                        if aliases {
                            g.aliases.insert(a.to_string(), b.to_string());
                        } else {
                            let v = g.parents.entry(a.to_string()).or_default();
                            if !v.iter().any(|p| p == b) {
                                v.push(b.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    g
}

fn globs() -> &'static Globs {
    static G: OnceLock<Globs> = OnceLock::new();
    G.get_or_init(load_globs)
}

/// `*` and `?` only (the patterns with `[...]` are left out when the database is read).
fn wild(p: &[u8], s: &[u8]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(b'*') => (0..=s.len()).any(|i| wild(&p[1..], &s[i..])),
        Some(b'?') => !s.is_empty() && wild(&p[1..], &s[1..]),
        Some(c) => s.first() == Some(c) && wild(&p[1..], &s[1..]),
    }
}

/// The type a file name says (None when nothing in the database matches it).
pub fn mime_of_name(name: &str) -> Option<String> {
    let g = globs();
    if let Some((_, m)) = g.literal.get(name) {
        return Some(m.clone());
    }
    // the longest ending that is known wins: `x.tar.gz` is a compressed tar, not just gzip
    let lower = name.to_lowercase();
    let mut best: Option<(usize, u32, &String)> = None;
    for (i, _) in name.match_indices('.') {
        let ext = &name[i + 1..];
        let hit = g.suffix_cs.get(ext).or_else(|| g.suffix.get(&lower[i + 1..]));
        if let Some((w, m)) = hit {
            let len = ext.len();
            if best.is_none_or(|(l, bw, _)| len > l || (len == l && *w > bw)) {
                best = Some((len, *w, m));
            }
        }
    }
    if let Some((_, _, m)) = best {
        return Some(m.clone());
    }
    g.other.iter().filter(|(p, ..)| wild(p.as_bytes(), lower.as_bytes())).max_by_key(|(p, w, _)| (*w, p.len())).map(|(_, _, m)| m.clone())
}

/// The type of a file: a folder, what its name says, else text or binary by its first bytes.
pub fn mime_of(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".into();
    }
    if let Some(m) = path.file_name().and_then(|n| mime_of_name(&n.to_string_lossy())) {
        return canonical(&m);
    }
    let mut head = [0u8; 1024];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"\x7fELF") {
        return "application/x-executable".into();
    }
    if head.starts_with(b"%PDF") {
        return "application/pdf".into();
    }
    // text when there is no zero byte and it reads as UTF-8 (a character cut at the end of the peek is fine)
    let text = !head.contains(&0) && match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    if text { "text/plain".into() } else { "application/octet-stream".into() }
}

/// The type an alias stands for (`application/x-pdf` -> `application/pdf`).
pub fn canonical(mime: &str) -> String {
    globs().aliases.get(mime).cloned().unwrap_or_else(|| mime.to_string())
}

/// The type and the types it is a kind of, nearest first: `text/rust`, `text/plain`, `application/octet-stream`.
pub fn lineage(mime: &str) -> Vec<String> {
    let g = globs();
    let mut out = vec![canonical(mime)];
    let mut i = 0;
    while i < out.len() && out.len() < 12 {
        let m = out[i].clone();
        let mut ups: Vec<String> = g.parents.get(&m).cloned().unwrap_or_default();
        // every text is plain text, everything is a stream of bytes (the spec's implicit parents)
        if m.starts_with("text/") && m != "text/plain" {
            ups.push("text/plain".into());
        }
        if m != "application/octet-stream" && m != "inode/directory" {
            ups.push("application/octet-stream".into());
        }
        for u in ups {
            if !out.contains(&u) {
                out.push(u);
            }
        }
        i += 1;
    }
    // the stream of bytes is the farthest of all
    if let Some(at) = out.iter().position(|m| m == "application/octet-stream") {
        let o = out.remove(at);
        out.push(o);
    }
    out
}

/// A short name of a type for people: "PNG image", "Rust source code"... from the database's comment, else the type.
pub fn describe(mime: &str) -> String {
    for dir in mime_dirs() {
        let Some((major, minor)) = mime.split_once('/') else { break };
        if let Ok(xml) = std::fs::read_to_string(dir.join(major).join(format!("{minor}.xml"))) {
            // the first <comment> without xml:lang is the English one
            for part in xml.split("<comment").skip(1) {
                if part.starts_with('>') {
                    if let Some(end) = part.find("</comment>") {
                        return unescape(&part[1..end]);
                    }
                }
            }
        }
    }
    mime.to_string()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// ---------------------------------------------------------------- the programs

/// The desktop's own programs and the types they open (they ship without desktop files).
const OWN: [(&str, &str, &str, &str, &[&str]); 4] = [
    ("memrium_view", "memrium-view", "Memrium Pictures", "image-viewer", &["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/tiff", "image/vnd.microsoft.icon", "image/svg+xml"]),
    ("memrium_player", "memrium-player", "Memrium Player", "multimedia-video-player", &["audio/*", "video/*"]),
    ("memrium_archive", "memrium-archive", "Memrium Archive", "utilities-file-archiver", &["application/zip", "application/x-tar", "application/x-compressed-tar", "application/x-xz-compressed-tar", "application/x-bzip-compressed-tar", "application/x-zstd-compressed-tar", "application/gzip", "application/x-7z-compressed", "application/vnd.rar"]),
    ("memrium_paint", "memrium-paint", "Memrium Paint", "applications-graphics", &["image/png", "image/jpeg", "image/bmp", "image/webp"]),
];

fn parse(path: &Path, id: &str) -> Option<Handler> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut f: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                if !k.contains('[') {
                    f.entry(k.trim()).or_insert(v.trim());
                }
            }
        }
    }
    if f.get("Type").copied() != Some("Application") || f.get("Hidden").copied() == Some("true") {
        return None;
    }
    let mimes: Vec<String> = f.get("MimeType")?.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect();
    Some(Handler {
        id: id.to_string(),
        name: f.get("Name")?.to_string(),
        icon: f.get("Icon").unwrap_or(&"").to_string(),
        exec: f.get("Exec")?.to_string(),
        terminal: f.get("Terminal").copied() == Some("true"),
        mimes,
        hidden: f.get("NoDisplay").copied() == Some("true"),
    })
}

fn scan_dir(dir: &Path, prefix: &str, out: &mut Vec<Handler>, seen: &mut std::collections::HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            scan_dir(&path, &format!("{prefix}{name}-"), out, seen);
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            let id = format!("{prefix}{stem}");
            // the first directory that has an id wins, even when that copy opens nothing
            if seen.insert(id.clone()) {
                if let Some(h) = parse(&path, &id) {
                    out.push(h);
                }
            }
        }
    }
}

fn scan() -> Vec<Handler> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in crate::apps::data_dirs() {
        scan_dir(&d.join("applications"), "", &mut out, &mut seen);
    }
    let dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    for (exe, id, name, icon, mimes) in OWN {
        let Some(path) = dir.as_ref().map(|d| d.join(exe)).filter(|p| p.exists()) else { continue };
        out.retain(|h| h.id != id);
        out.push(Handler { id: id.into(), name: name.into(), icon: icon.into(), exec: format!("{} %F", sh_quote(&path.to_string_lossy())), terminal: false, mimes: mimes.iter().map(|m| m.to_string()).collect(), hidden: false });
    }
    out
}

/// Every program that opens some type (read again when the last look is older than half a minute: programs get
/// installed while the file manager runs).
pub fn all() -> Arc<Vec<Handler>> {
    static CACHE: Mutex<Option<(Instant, Arc<Vec<Handler>>)>> = Mutex::new(None);
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = g.as_ref() {
        if at.elapsed() < Duration::from_secs(30) {
            return list.clone();
        }
    }
    let list = Arc::new(scan());
    *g = Some((Instant::now(), list.clone()));
    list
}

fn declares(h: &Handler, mime: &str) -> bool {
    h.mimes.iter().any(|m| m == mime || m.strip_suffix("/*").is_some_and(|major| mime.split('/').next() == Some(major)))
}

// ---------------------------------------------------------------- mimeapps.list

/// The `mimeapps.list` files, the one that wins first: the user's (desktop-specific, then plain), the system's config
/// dirs, then the data dirs' `applications/`.
fn list_files() -> Vec<PathBuf> {
    let home = crate::fsops::home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect();
    let mut dirs = vec![config];
    dirs.extend(std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into()).split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    let mut out = Vec::new();
    for d in &dirs {
        for de in &desktops {
            out.push(d.join(format!("{de}-mimeapps.list")));
        }
        out.push(d.join("mimeapps.list"));
    }
    for d in crate::apps::data_dirs() {
        out.push(d.join("applications").join("mimeapps.list"));
    }
    out
}

/// What the user wrote for types: the defaults, the added and the removed associations of every list, in order.
#[derive(Default, Debug)]
struct Choices {
    defaults: Vec<(String, Vec<String>)>,
    added: Vec<(String, Vec<String>)>,
    removed: Vec<(String, Vec<String>)>,
}

fn read_choices(files: &[PathBuf]) -> Choices {
    let mut c = Choices::default();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line;
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let ids: Vec<String> = v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(|s| s.strip_suffix(".desktop").unwrap_or(s).to_string()).collect();
            let to = match section {
                "[Default Applications]" => &mut c.defaults,
                "[Added Associations]" => &mut c.added,
                "[Removed Associations]" => &mut c.removed,
                _ => continue,
            };
            to.push((k.trim().to_string(), ids));
        }
    }
    c
}

fn ids_for<'a>(list: &'a [(String, Vec<String>)], mime: &str) -> impl Iterator<Item = &'a String> {
    let mime = mime.to_string();
    list.iter().filter(move |(m, _)| *m == mime).flat_map(|(_, ids)| ids.iter())
}

/// The programs for a type, the one that opens it first: the defaults, the added associations, then every program that
/// declares the type (exact types before the ones it is a kind of), without the removed ones.
pub fn handlers_for(mime: &str) -> Vec<Handler> {
    handlers_with(mime, &all(), &read_choices(&list_files()))
}

fn handlers_with(mime: &str, all: &[Handler], ch: &Choices) -> Vec<Handler> {
    let by_id = |id: &str| all.iter().find(|h| h.id == id);
    let mut out: Vec<Handler> = Vec::new();
    let push = |h: &Handler, out: &mut Vec<Handler>| {
        if !out.iter().any(|o| o.id == h.id) {
            out.push(h.clone());
        }
    };
    let lineage = lineage(mime);
    let removed = |id: &str, m: &str| ids_for(&ch.removed, m).any(|r| r == id);
    for m in &lineage {
        for id in ids_for(&ch.defaults, m).chain(ids_for(&ch.added, m)) {
            if let Some(h) = by_id(id).filter(|_| !removed(id, m)) {
                push(h, &mut out);
            }
        }
        // the generic stream of bytes is opened by "anything": only say it when nothing more exact is there
        if m == "application/octet-stream" && !out.is_empty() {
            break;
        }
        let mut declared: Vec<&Handler> = all.iter().filter(|h| declares(h, m) && !removed(&h.id, m)).collect();
        // exact declarations before wildcards, shown programs before hidden ones, then by name
        declared.sort_by_key(|h| (!h.mimes.iter().any(|x| x == m), h.hidden, h.name.to_lowercase()));
        for h in declared {
            push(h, &mut out);
        }
    }
    out
}

/// The program the user chose for this type (or one of the types it is a kind of) in their own `mimeapps.list`. The
/// system's lists are left out: they are the distribution's guesses, and the desktop's own viewers beat those.
pub fn user_default(mime: &str) -> Option<Handler> {
    let home = crate::fsops::home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let ch = read_choices(&[config.join("mimeapps.list")]);
    let all = all();
    lineage(mime).iter().filter(|m| *m != "application/octet-stream").find_map(|m| ids_for(&ch.defaults, m).find_map(|id| all.iter().find(|h| &h.id == id)).cloned())
}

/// The default program for a type by every list (what `xdg-open` would start), else the first that declares it.
pub fn default_for(mime: &str) -> Option<Handler> {
    handlers_for(mime).into_iter().next()
}

/// Make `id` the program that opens `mime` (the user's `mimeapps.list`, its other lines kept).
pub fn set_default(mime: &str, id: &str) -> std::io::Result<()> {
    let home = crate::fsops::home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let path = config.join("mimeapps.list");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let out = with_default(&text, mime, id);
    std::fs::create_dir_all(&config)?;
    let tmp = path.with_extension("list.new");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &path)
}

/// `text` (a mimeapps.list) with `mime=id.desktop;` as the default and in the added associations.
fn with_default(text: &str, mime: &str, id: &str) -> String {
    let file = format!("{id}.desktop");
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if line.trim().starts_with('[') {
            sections.push((line.trim().to_string(), Vec::new()));
        } else if let Some(last) = sections.last_mut() {
            last.1.push(line.to_string());
        } else if !line.trim().is_empty() {
            sections.push((String::new(), vec![line.to_string()]));
        }
    }
    for (name, first) in [("[Default Applications]", true), ("[Added Associations]", false)] {
        if !sections.iter().any(|(n, _)| n == name) {
            sections.push((name.to_string(), Vec::new()));
        }
        let lines = &mut sections.iter_mut().find(|(n, _)| n == name).unwrap().1;
        let at = lines.iter().position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == mime));
        let line = if first {
            format!("{mime}={file}")
        } else {
            // our choice first, the others kept after it
            let old: Vec<String> = at.map(|i| lines[i].split_once('=').map(|(_, v)| v.split(';').map(str::trim).filter(|s| !s.is_empty() && *s != file).map(str::to_string).collect()).unwrap_or_default()).unwrap_or_default();
            let mut all = vec![file.clone()];
            all.extend(old);
            format!("{mime}={};", all.join(";"))
        };
        match at {
            Some(i) => lines[i] = line,
            None => {
                // before the blank lines that end the section
                let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
                lines.insert(end, line);
            }
        }
    }
    let mut out = String::new();
    for (i, (name, lines)) in sections.iter().enumerate() {
        if !name.is_empty() {
            if i > 0 && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(name);
            out.push('\n');
        }
        let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
        for l in &lines[..end] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------- starting a program with files

/// A word for `sh -c`.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-_.,+=:@%".contains(&b)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The command line of `h` with the files put in for its field codes (`%f` one path, `%F` all, `%u`/`%U` as addresses);
/// a line without one gets the paths at the end. A program that takes one file at a time gets only the first.
pub fn command_line(h: &Handler, files: &[PathBuf]) -> String {
    let paths: Vec<String> = files.iter().map(|p| sh_quote(&p.to_string_lossy())).collect();
    let uris: Vec<String> = files.iter().map(|p| sh_quote(&crate::fsops::uri_of(p))).collect();
    let mut out = String::new();
    let mut used = false;
    let mut chars = h.exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('f') => {
                used = true;
                out.push_str(paths.first().map_or("", String::as_str));
            }
            Some('F') => {
                used = true;
                out.push_str(&paths.join(" "));
            }
            Some('u') => {
                used = true;
                out.push_str(uris.first().map_or("", String::as_str));
            }
            Some('U') => {
                used = true;
                out.push_str(&uris.join(" "));
            }
            Some('i') if !h.icon.is_empty() => out.push_str(&format!("--icon {}", sh_quote(&h.icon))),
            Some('c') => out.push_str(&sh_quote(&h.name)),
            _ => {}
        }
    }
    if !used && !paths.is_empty() {
        out.push(' ');
        out.push_str(&paths.join(" "));
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the program takes several files in one go (`%F` / `%U`); otherwise it is started once per file.
pub fn takes_many(h: &Handler) -> bool {
    h.exec.contains("%F") || h.exec.contains("%U")
}

/// Open `files` with `h` on the desktop `socket` (one start for all when it takes several, else one per file), with the
/// launch rules of that program (its GPU, flags, look).
pub fn open_with(h: &Handler, files: &[PathBuf], socket: &str) -> std::io::Result<()> {
    let groups: Vec<Vec<PathBuf>> = if takes_many(h) { vec![files.to_vec()] } else { files.iter().map(|f| vec![f.clone()]).collect() };
    for g in groups {
        let app = crate::apps::AppEntry { id: h.id.clone(), name: h.name.clone(), generic_name: String::new(), exec: command_line(h, &g), icon: h.icon.clone(), keywords: String::new(), terminal: h.terminal, wm_class: String::new(), categories: Vec::new() };
        crate::apps::launch_in(&app, socket, g.first().and_then(|p| p.parent()))?;
    }
    for f in files {
        crate::recent::add(f, &h.id, &h.exec);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(id: &str, mimes: &[&str]) -> Handler {
        Handler { id: id.into(), name: id.into(), icon: String::new(), exec: format!("{id} %f"), terminal: false, mimes: mimes.iter().map(|m| m.to_string()).collect(), hidden: false }
    }

    #[test]
    fn field_codes_take_the_files_quoted() {
        let mut v = h("gimp", &[]);
        let files = vec![PathBuf::from("/tmp/a b.png"), PathBuf::from("/tmp/it's.png")];
        assert_eq!(command_line(&v, &files), "gimp '/tmp/a b.png'");
        v.exec = "gimp --new %U".into();
        assert_eq!(command_line(&v, &files), "gimp --new file:///tmp/a%20b.png file:///tmp/it%27s.png");
        v.exec = "kate".into();
        assert_eq!(command_line(&v, &files[..1]), "kate '/tmp/a b.png'");
        v.exec = "flatpak run --file-forwarding org.x.Y @@u %u @@".into();
        assert_eq!(command_line(&v, &files[..1]), "flatpak run --file-forwarding org.x.Y @@u file:///tmp/a%20b.png @@");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn defaults_come_first_and_removed_ones_go() {
        let all = vec![h("viewer", &["image/png"]), h("editor", &["image/*"]), h("other", &["image/png"]), h("gone", &["image/png"])];
        let ch = Choices { defaults: vec![("image/png".into(), vec!["editor".into()])], added: vec![], removed: vec![("image/png".into(), vec!["gone".into()])] };
        let ids: Vec<String> = handlers_with("image/png", &all, &ch).into_iter().map(|h| h.id).collect();
        assert_eq!(ids, vec!["editor", "other", "viewer"]);
    }

    #[test]
    fn a_new_default_is_written_and_the_rest_kept() {
        let before = "[Default Applications]\nx-scheme-handler/http=brave.desktop\n\n[Added Associations]\nimage/png=gimp.desktop;\n";
        let after = with_default(before, "image/png", "org.kde.gwenview");
        assert!(after.contains("x-scheme-handler/http=brave.desktop"));
        assert!(after.contains("[Default Applications]\nx-scheme-handler/http=brave.desktop\nimage/png=org.kde.gwenview.desktop\n"));
        assert!(after.contains("image/png=org.kde.gwenview.desktop;gimp.desktop;"));
        let fresh = with_default("", "text/plain", "kate");
        assert_eq!(fresh, "[Default Applications]\ntext/plain=kate.desktop\n\n[Added Associations]\ntext/plain=kate.desktop;\n");
    }

    #[test]
    fn names_tell_the_type() {
        // only meaningful with a MIME database on the machine
        if mime_dirs().is_empty() {
            return;
        }
        assert_eq!(mime_of_name("a.PNG").as_deref(), Some("image/png"));
        assert_eq!(mime_of_name("x.tar.gz").as_deref(), Some("application/x-compressed-tar"));
        assert_eq!(mime_of_name("main.rs").as_deref(), Some("text/rust"));
        assert!(lineage("text/rust").contains(&"text/plain".to_string()));
        assert_eq!(lineage("text/rust").last().map(String::as_str), Some("application/octet-stream"));
    }
}
