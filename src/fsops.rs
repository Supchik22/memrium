//! File-system helpers shared by the file manager and the desktop: listing, file kinds, copy / move,
//! the freedesktop Trash, unique names and a small undo stack.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Text,
    /// a `.desktop` launcher
    App,
    Other,
}

impl Kind {
    /// Lucide icon for the kind.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Dir => "folder",
            Kind::Image => "file-image",
            Kind::Audio => "file-music",
            Kind::Video => "video",
            Kind::Archive => "file-archive",
            Kind::Code => "file-code",
            Kind::Text => "file-text",
            Kind::App => "app-window",
            Kind::Other => "file",
        }
    }

    pub fn tint(self, accent: [f32; 3]) -> [f32; 3] {
        match self {
            Kind::Dir => accent,
            Kind::Image => [0.30, 0.75, 0.45],
            Kind::Audio => [0.90, 0.45, 0.70],
            Kind::Video => [0.95, 0.60, 0.25],
            Kind::Archive => [0.80, 0.65, 0.30],
            Kind::Code => [0.55, 0.50, 0.95],
            Kind::Text => [0.45, 0.65, 0.95],
            Kind::App => [0.35, 0.80, 0.85],
            Kind::Other => [0.62, 0.66, 0.74],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Dir => "Folder",
            Kind::Image => "Image",
            Kind::Audio => "Audio",
            Kind::Video => "Video",
            Kind::Archive => "Archive",
            Kind::Code => "Source code",
            Kind::Text => "Document",
            Kind::App => "Application",
            Kind::Other => "File",
        }
    }
}

pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Dir;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "desktop" => Kind::App,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "tif" | "tiff" | "avif" | "ico" => Kind::Image,
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" => Kind::Audio,
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" => Kind::Video,
        "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "iso" => Kind::Archive,
        "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "hpp" | "go" | "java" | "sh" | "fish" | "nix" | "toml" | "json" | "yaml" | "yml" | "html" | "css" | "wgsl" | "lua" | "xml" => Kind::Code,
        "txt" | "md" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "log" | "csv" | "epub" | "ini" | "conf" => Kind::Text,
        _ => Kind::Other,
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub hidden: bool,
    pub kind: Kind,
    /// a `.desktop` launcher: the name and icon it declares
    pub app: Option<(String, String)>,
}

impl Entry {
    /// What to show as the name: a launcher shows the name it declares, the rest their file name.
    pub fn title(&self) -> &str {
        self.app.as_ref().map(|(n, _)| n.as_str()).unwrap_or(&self.name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Modified,
    Kind,
}

/// List a directory (folders first when `dirs_first`).
pub fn list_dir(dir: &Path, show_hidden: bool, sort: SortBy, desc: bool, dirs_first: bool) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        let hidden = name.starts_with('.');
        if hidden && !show_hidden {
            continue;
        }
        let path = e.path();
        let md = fs::metadata(&path).or_else(|_| e.metadata());
        let (is_dir, size, modified) = match md {
            Ok(m) => (m.is_dir(), m.len(), m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)),
            Err(_) => (false, 0, 0),
        };
        let kind = kind_of(&name, is_dir);
        let app = if kind == Kind::App { crate::apps::load_desktop(&path).map(|a| (a.name, a.icon)) } else { None };
        out.push(Entry { name, path, is_dir, size, modified, hidden, kind, app });
    }
    out.sort_by(|a, b| {
        if dirs_first && a.is_dir != b.is_dir {
            return b.is_dir.cmp(&a.is_dir);
        }
        let o = match sort {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Size => a.size.cmp(&b.size).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortBy::Modified => a.modified.cmp(&b.modified),
            SortBy::Kind => (a.kind as u8).cmp(&(b.kind as u8)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        };
        if desc {
            o.reverse()
        } else {
            o
        }
    });
    Ok(out)
}

/// Icon-theme names to try for an entry, best first (the first one that exists wins).
pub fn icon_names(e: &Entry) -> Vec<&'static str> {
    if e.is_dir {
        let home = home();
        if e.path.parent() == Some(home.as_path()) {
            let special = match e.name.as_str() {
                "Desktop" => Some("user-desktop"),
                "Documents" => Some("folder-documents"),
                "Downloads" => Some("folder-download"),
                "Pictures" => Some("folder-pictures"),
                "Music" => Some("folder-music"),
                "Videos" => Some("folder-videos"),
                _ => None,
            };
            if let Some(s) = special {
                return vec![s, "folder"];
            }
        }
        return vec!["folder"];
    }
    let ext = e.name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
    let mut v: Vec<&'static str> = match ext.as_str() {
        "pdf" => vec!["application-pdf"],
        "html" | "htm" => vec!["text-html"],
        "json" => vec!["application-json", "text-x-script"],
        "md" => vec!["text-markdown", "text-x-generic"],
        "sh" | "fish" | "bash" => vec!["text-x-script", "application-x-shellscript"],
        "rs" => vec!["text-rust", "text-x-script"],
        "py" => vec!["text-x-python", "text-x-script"],
        "iso" => vec!["application-x-cd-image", "media-optical"],
        "zip" | "7z" | "rar" => vec!["application-zip", "application-x-archive", "package-x-generic"],
        "tar" | "gz" | "xz" | "bz2" | "zst" => vec!["application-x-compressed-tar", "application-x-archive", "package-x-generic"],
        "doc" | "docx" | "odt" => vec!["application-msword", "x-office-document"],
        "xls" | "xlsx" | "ods" | "csv" => vec!["x-office-spreadsheet"],
        "flatpak" => vec!["package-x-generic"],
        _ => vec![],
    };
    v.push(match e.kind {
        Kind::Image => "image-x-generic",
        Kind::Audio => "audio-x-generic",
        Kind::Video => "video-x-generic",
        Kind::Archive => "package-x-generic",
        Kind::Code => "text-x-script",
        Kind::Text => "text-x-generic",
        _ => "application-octet-stream",
    });
    v.push("text-x-generic");
    v
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

/// "15 Oct 2026, 17:12" in local time.
pub fn format_time(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {}, {:02}:{:02}", tm.tm_mday, months[(tm.tm_mon as usize).min(11)], tm.tm_year + 1900, tm.tm_hour, tm.tm_min)
}

fn iso_time() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The folder shown on the desktop (~/Desktop, created if missing).
pub fn desktop_dir() -> PathBuf {
    let d = home().join("Desktop");
    let _ = fs::create_dir_all(&d);
    d
}

/// `dir/name`, or `dir/name (2)`, `dir/name (3)`… when taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for i in 2.. {
        let p = dir.join(format!("{stem} ({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
    } else if md.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dst)?;
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn copy_to(src: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "copy".into());
    let dst = unique_path(dir, &name);
    if src.is_dir() && dst.starts_with(src) {
        return Err(std::io::Error::other("cannot copy a folder into itself"));
    }
    copy_recursive(src, &dst)?;
    Ok(dst)
}

pub fn move_to(src: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "moved".into());
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf());
    }
    let dst = unique_path(dir, &name);
    if src.is_dir() && dst.starts_with(src) {
        return Err(std::io::Error::other("cannot move a folder into itself"));
    }
    if fs::rename(src, &dst).is_err() {
        copy_recursive(src, &dst)?;
        if src.is_dir() {
            fs::remove_dir_all(src)?;
        } else {
            fs::remove_file(src)?;
        }
    }
    Ok(dst)
}

pub fn rename(path: &Path, new_name: &str) -> std::io::Result<PathBuf> {
    if new_name.is_empty() || new_name.contains('/') {
        return Err(std::io::Error::other("invalid name"));
    }
    let dst = path.with_file_name(new_name);
    if dst.exists() {
        return Err(std::io::Error::other("a file with that name already exists"));
    }
    fs::rename(path, &dst)?;
    Ok(dst)
}

pub fn make_dir(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let p = unique_path(dir, name);
    fs::create_dir(&p)?;
    Ok(p)
}

pub fn make_file(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let p = unique_path(dir, name);
    fs::File::create(&p)?;
    Ok(p)
}

fn trash_dirs() -> (PathBuf, PathBuf) {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("Trash");
    (base.join("files"), base.join("info"))
}

fn url_encode(p: &str) -> String {
    let mut s = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Move to the Trash (freedesktop layout). Returns where it went, so it can be restored.
pub fn trash(path: &Path) -> std::io::Result<PathBuf> {
    let (files, info) = trash_dirs();
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let mut dst = unique_path(&files, &name);
    let final_name = dst.file_name().unwrap().to_string_lossy().to_string();
    if fs::rename(path, &dst).is_err() {
        copy_recursive(path, &dst)?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let _ = fs::write(info.join(format!("{final_name}.trashinfo")), format!("[Trash Info]\nPath={}\nDeletionDate={}\n", url_encode(&abs.to_string_lossy()), iso_time()));
    dst = files.join(final_name);
    Ok(dst)
}

pub fn trash_path() -> PathBuf {
    trash_dirs().0
}

pub fn delete_forever(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Empty the Trash.
pub fn empty_trash() {
    let (files, info) = trash_dirs();
    for d in [files, info] {
        if let Ok(rd) = fs::read_dir(&d) {
            for e in rd.flatten() {
                let _ = delete_forever(&e.path());
            }
        }
    }
}

/// Things the file manager can take back.
#[derive(Clone, Debug)]
pub enum Undo {
    /// created at `path`
    Created(PathBuf),
    /// renamed `from` -> `to`
    Renamed { from: PathBuf, to: PathBuf },
    /// moved `from` -> `to` (also used for Trash: `to` is inside the Trash)
    Moved { from: PathBuf, to: PathBuf },
    /// copied to `to`
    Copied(PathBuf),
}

pub fn undo(op: &Undo) -> std::io::Result<()> {
    match op {
        Undo::Created(p) | Undo::Copied(p) => {
            // a created / copied item goes to the Trash instead of being destroyed
            trash(p).map(|_| ())
        }
        Undo::Renamed { from, to } | Undo::Moved { from, to } => {
            if from.exists() {
                return Err(std::io::Error::other("the original location is occupied"));
            }
            fs::rename(to, from)?;
            // drop the stale .trashinfo when restoring from the Trash
            let (files, info) = trash_dirs();
            if to.starts_with(&files) {
                if let Some(n) = to.file_name() {
                    let _ = fs::remove_file(info.join(format!("{}.trashinfo", n.to_string_lossy())));
                }
            }
            Ok(())
        }
    }
}

/// Our own program for a file, if there is one: pictures open in Memrium View, music and video in Memrium Player.
/// (Only the formats the viewer can decode: anything else is better left to the system's default.)
pub fn own_opener(path: &Path) -> Option<&'static str> {
    if path.is_dir() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_string();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match kind_of(&name, false) {
        Kind::Image if ["png", "jpg", "jpeg", "jfif", "webp", "gif", "bmp", "tif", "tiff", "ico", "svg"].contains(&ext.as_str()) => Some("memrium_view"),
        Kind::Audio | Kind::Video => Some("memrium_player"),
        _ if crate::archive::is_archive(path) => Some("memrium_archive"),
        _ => None,
    }
}

/// Where a program of ours is: next to the running executable (a build or an install), else left to `PATH`.
pub fn own_program(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

/// Start one of our programs with arguments, on the display of this desktop.
pub fn spawn_own(program: &str, args: &[String], socket: &str) {
    let mut c = std::process::Command::new(own_program(program));
    c.args(args).env("WAYLAND_DISPLAY", socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Err(e) = c.spawn() {
        eprintln!("{program} did not start: {e}");
    }
}

/// Open a file or folder: with one of our programs when there is one for it, otherwise with the default application.
pub fn open_external(path: &Path, socket: &str) {
    // a launcher starts its program (with the rules of that program) instead of opening as text
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop")) {
        if let Some(app) = crate::apps::load_desktop(path) {
            match crate::apps::launch(&app, socket) {
                Ok(()) => return,
                Err(e) => eprintln!("{} did not start: {e}", app.name),
            }
        }
    }
    let quiet = |c: &mut std::process::Command| {
        c.env("WAYLAND_DISPLAY", socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    };
    if let Some(program) = own_opener(path) {
        let mut c = std::process::Command::new(own_program(program));
        c.arg(path);
        quiet(&mut c);
        match c.spawn() {
            Ok(_) => return,
            // not installed: the system's default will do
            Err(e) => eprintln!("{program} did not start ({e}); using xdg-open"),
        }
    }
    let mut c = std::process::Command::new("xdg-open");
    c.arg(path);
    quiet(&mut c);
    if let Err(e) = c.spawn() {
        eprintln!("xdg-open failed: {e}");
    }
}

/// Clipboard + undo history shared by every file view (the file manager and the desktop).
#[derive(Default)]
pub struct Fm {
    pub clip: Vec<PathBuf>,
    pub cut: bool,
    pub undo: Vec<Vec<Undo>>,
    /// last error or hint, shown in the status bar
    pub note: String,
    /// bumped on every change on disk, so views know to reload
    pub gen: u64,
}

impl Fm {
    pub fn set_clip(&mut self, paths: Vec<PathBuf>, cut: bool) {
        if paths.is_empty() {
            return;
        }
        self.note = format!("{} item{} {}", paths.len(), if paths.len() == 1 { "" } else { "s" }, if cut { "cut" } else { "copied" });
        self.clip = paths;
        self.cut = cut;
    }

    pub fn can_paste(&self) -> bool {
        !self.clip.is_empty()
    }

    fn record(&mut self, group: Vec<Undo>) {
        if !group.is_empty() {
            self.undo.push(group);
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
        }
        self.gen += 1;
    }

    /// Paste the clipboard into `dir`; returns the new paths.
    pub fn paste_into(&mut self, dir: &Path) -> Vec<PathBuf> {
        let mut group = Vec::new();
        let mut out = Vec::new();
        for src in self.clip.clone() {
            let r = if self.cut { move_to(&src, dir) } else { copy_to(&src, dir) };
            match r {
                Ok(dst) => {
                    if self.cut {
                        if dst != src {
                            group.push(Undo::Moved { from: src.clone(), to: dst.clone() });
                        }
                    } else {
                        group.push(Undo::Copied(dst.clone()));
                    }
                    out.push(dst);
                }
                Err(e) => self.note = format!("Could not paste {}: {e}", src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            }
        }
        if self.cut {
            self.clip.clear();
            self.cut = false;
        }
        self.record(group);
        out
    }

    /// Drop files onto `dir`: move them, or copy when `copy` is set. Returns how many were handled.
    pub fn drop_into(&mut self, paths: &[PathBuf], dir: &Path, copy: bool) -> usize {
        let mut group = Vec::new();
        for src in paths {
            // dropping a folder onto itself or a file onto its own folder is a no-op
            if src == dir || (src.is_dir() && dir.starts_with(src)) || (!copy && src.parent() == Some(dir)) {
                continue;
            }
            let r = if copy { copy_to(src, dir) } else { move_to(src, dir) };
            match r {
                Ok(dst) => group.push(if copy { Undo::Copied(dst) } else { Undo::Moved { from: src.clone(), to: dst } }),
                Err(e) => self.note = format!("Could not {}: {e}", if copy { "copy" } else { "move" }),
            }
        }
        let n = group.len();
        self.record(group);
        n
    }

    pub fn trash_all(&mut self, paths: &[PathBuf]) {
        let mut group = Vec::new();
        for p in paths {
            match trash(p) {
                Ok(to) => group.push(Undo::Moved { from: p.clone(), to }),
                Err(e) => self.note = format!("Could not move to Trash: {e}"),
            }
        }
        self.record(group);
    }

    pub fn new_folder(&mut self, dir: &Path) -> Option<PathBuf> {
        match make_dir(dir, "New Folder") {
            Ok(p) => {
                self.record(vec![Undo::Created(p.clone())]);
                Some(p)
            }
            Err(e) => {
                self.note = format!("Could not create folder: {e}");
                None
            }
        }
    }

    pub fn new_file(&mut self, dir: &Path) -> Option<PathBuf> {
        match make_file(dir, "New File.txt") {
            Ok(p) => {
                self.record(vec![Undo::Created(p.clone())]);
                Some(p)
            }
            Err(e) => {
                self.note = format!("Could not create file: {e}");
                None
            }
        }
    }

    pub fn rename(&mut self, path: &Path, new_name: &str) -> Option<PathBuf> {
        if path.file_name().is_some_and(|n| n.to_string_lossy() == new_name) {
            return Some(path.to_path_buf());
        }
        match rename(path, new_name) {
            Ok(to) => {
                self.record(vec![Undo::Renamed { from: path.to_path_buf(), to: to.clone() }]);
                Some(to)
            }
            Err(e) => {
                self.note = format!("Could not rename: {e}");
                None
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo_last(&mut self) -> bool {
        let Some(group) = self.undo.pop() else { return false };
        for op in group.iter().rev() {
            if let Err(e) = undo(op) {
                self.note = format!("Undo failed: {e}");
            }
        }
        self.gen += 1;
        true
    }
}

/// The pseudo folder that lists the machine's drives.
pub const COMPUTER: &str = "computer:";

pub fn is_computer(p: &Path) -> bool {
    p.as_os_str() == COMPUTER
}

/// Mounted disks (real file systems only) as entries: "Name (/mount) - 120 GB free of 450 GB".
pub fn drives() -> Vec<Entry> {
    const REAL: [&str; 11] = ["ext4", "ext3", "ext2", "btrfs", "xfs", "vfat", "exfat", "ntfs", "ntfs3", "f2fs", "fuseblk"];
    let text = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(dev), Some(mp), Some(fs)) = (it.next(), it.next(), it.next()) else { continue };
        let mp = mp.replace("\\040", " ");
        if !REAL.contains(&fs) || !dev.starts_with('/') || mp.starts_with("/nix/store") || mp.starts_with("/var/lib/docker") || !seen.insert(dev.to_string()) {
            continue;
        }
        let Ok(c) = std::ffi::CString::new(mp.clone()) else { continue };
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            continue;
        }
        let (total, free) = (st.f_blocks as u64 * st.f_frsize as u64, st.f_bavail as u64 * st.f_frsize as u64);
        let label = if mp == "/" {
            "Local Disk".to_string()
        } else {
            Path::new(&mp).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| mp.clone())
        };
        out.push(Entry {
            name: format!("{label} ({mp}) - {} free of {}", human_size(free), human_size(total)),
            path: PathBuf::from(&mp),
            is_dir: true,
            size: total,
            modified: 0,
            hidden: false,
            kind: Kind::Dir,
            app: None,
        });
    }
    // servers: file systems of the network, and the shares that GVfs has mounted. Their free space is not asked (a server that
    // has gone away would hold the whole window up)
    let mut net = network_mounts(&text);
    net.extend(gvfs_mounts());
    for (label, mp) in net {
        if out.iter().any(|e| e.path == mp) {
            continue;
        }
        out.push(Entry { name: format!("{label} ({}) - network", mp.display()), path: mp, is_dir: true, size: 0, modified: 0, hidden: false, kind: Kind::Dir, app: None });
    }
    out
}

/// File systems that live on another machine.
pub fn is_network_fs(fs: &str) -> bool {
    matches!(fs, "cifs" | "smb3" | "smbfs" | "nfs" | "nfs4" | "fuse.sshfs" | "fuse.rclone" | "davfs" | "fuse.davfs2" | "9p" | "afs" | "ceph")
}

/// What to call a network mount: `//server/share` is "share on server", `server:/export` "export on server", `user@host:/dir`
/// "dir on host".
pub fn network_label(dev: &str, mp: &str) -> String {
    let last = |s: &str| s.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    let named = |share: String, host: &str| if share.is_empty() { host.to_string() } else { format!("{share} on {host}") };
    if let Some(rest) = dev.strip_prefix("//") {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        return named(last(path), host.rsplit('@').next().unwrap_or(host));
    }
    if let Some((host, path)) = dev.split_once(':') {
        if !host.is_empty() && !host.contains('/') {
            return named(last(path), host.rsplit('@').next().unwrap_or(host));
        }
    }
    Path::new(mp).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dev.to_string())
}

/// (name, mount point) of the network file systems in `/proc/mounts`.
pub fn network_mounts(proc_mounts: &str) -> Vec<(String, PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    proc_mounts
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let (dev, mp, fs) = (it.next()?, it.next()?.replace("\\040", " "), it.next()?);
            (is_network_fs(fs) && seen.insert(mp.clone())).then(|| (network_label(&dev.replace("\\040", " "), &mp), PathBuf::from(mp)))
        })
        .collect()
}

/// A GVfs mount's folder name (`smb-share:server=nas,share=photos`, `sftp:host=box,user=me`) as a name.
pub fn gvfs_label(dir: &str) -> String {
    let (scheme, rest) = dir.split_once(':').unwrap_or((dir, ""));
    let field = |k: &str| rest.split(',').find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(|v| v.to_string()));
    match scheme {
        "smb-share" => match (field("share"), field("server")) {
            (Some(share), Some(server)) => format!("{share} on {server}"),
            _ => dir.to_string(),
        },
        "sftp" | "ftp" | "dav" | "davs" | "nfs" => {
            let host = field("host").or_else(|| field("server")).unwrap_or_default();
            match field("user") {
                Some(u) => format!("{u}@{host}"),
                None => host,
            }
        }
        _ => dir.to_string(),
    }
}

fn gvfs_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    std::env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(format!("/run/user/{uid}"))).join("gvfs")
}

fn gvfs_mounts() -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(gvfs_dir()) else { return Vec::new() };
    rd.flatten().map(|e| (gvfs_label(&e.file_name().to_string_lossy()), e.path())).collect()
}

/// Is `p` on a server (a network mount, or a share of GVfs)?
pub fn is_network_path(p: &Path) -> bool {
    p.starts_with(gvfs_dir()) || network_mounts(&std::fs::read_to_string("/proc/mounts").unwrap_or_default()).iter().any(|(_, mp)| p == mp.as_path())
}

/// An address typed in the location field as the URL that `gio mount` wants: `smb://nas/photos`, `sftp://me@box/`,
/// `\\nas\photos`, `ssh://box`. None when it is an ordinary path.
pub fn server_url(input: &str) -> Option<String> {
    let t = input.trim();
    if let Some(rest) = t.strip_prefix("\\\\") {
        return Some(format!("smb://{}", rest.replace('\\', "/")));
    }
    let (scheme, rest) = t.split_once("://")?;
    if rest.is_empty() {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "smb" | "sftp" | "ftp" | "ftps" | "dav" | "davs" | "nfs" | "afp" => Some(format!("{scheme}://{rest}")),
        "ssh" => Some(format!("sftp://{rest}")),
        _ => None,
    }
}

/// Mount a server with GVfs (`gio mount`) and return where its files are. Needs the `gio` program; a share that wants a
/// password asks for it in the terminal that is not there, so it must be one that lets you in, or the user name is in the
/// address.
pub fn mount_server(url: &str) -> Result<PathBuf, String> {
    let out = std::process::Command::new("gio").args(["mount", url]).stdin(std::process::Stdio::null()).output().map_err(|_| "Connecting to a server needs GVfs (the gio program)".to_string())?;
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() && !err.contains("already mounted") {
        let first = err.lines().next().unwrap_or("could not connect").trim().trim_start_matches("gio: ").to_string();
        return Err(first);
    }
    let info = std::process::Command::new("gio").args(["info", url]).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&info.stdout).lines().find_map(|l| l.trim().strip_prefix("local path:").map(|p| PathBuf::from(p.trim()))).ok_or_else(|| "Connected, but the files are not reachable from here".to_string())
}

/// Let go of a server (a GVfs share or a FUSE mount of the user's).
pub fn disconnect(p: &Path) -> Result<(), String> {
    let run = |prog: &str, args: &[&str]| std::process::Command::new(prog).args(args).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string());
    let path = p.to_string_lossy().to_string();
    let out = if p.starts_with(gvfs_dir()) { run("gio", &["mount", "-u", &path])? } else { run("fusermount3", &["-u", &path]).or_else(|_| run("fusermount", &["-u", &path]))? };
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("could not disconnect").trim().to_string())
    }
}

// ---------------------------------------------------------------- converting pictures

/// What a picture can be converted to: (menu label, file extension).
pub const CONVERT_TO: [(&str, &str); 7] = [("PNG", "png"), ("JPEG", "jpg"), ("WebP", "webp"), ("BMP", "bmp"), ("GIF", "gif"), ("TIFF", "tiff"), ("ICO", "ico")];

fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// A picture the converter can read (a format of the `image` crate, or SVG).
pub fn can_convert(path: &Path) -> bool {
    !path.is_dir() && ["png", "jpg", "jpeg", "jfif", "webp", "gif", "bmp", "tif", "tiff", "ico", "svg"].contains(&extension_of(path).as_str())
}

fn load_picture(path: &Path) -> Result<image::DynamicImage, String> {
    if extension_of(path) == "svg" {
        let data = fs::read(path).map_err(|e| e.to_string())?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
        let size = tree.size();
        // a vector picture has no pixels of its own: 2048 on the long side stays sharp
        let k = (2048.0 / size.width().max(size.height())).clamp(0.01, 16.0);
        let (w, h) = ((size.width() * k).ceil().max(1.0) as u32, (size.height() * k).ceil().max(1.0) as u32);
        let mut pix = resvg::tiny_skia::Pixmap::new(w, h).ok_or("empty picture")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pix.as_mut());
        // tiny-skia's pixels are premultiplied; the file wants straight alpha
        let mut raw = pix.take();
        for p in raw.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a > 0 && a < 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        return image::RgbaImage::from_raw(w, h, raw).map(image::DynamicImage::ImageRgba8).ok_or_else(|| "empty picture".to_string());
    }
    use image::ImageDecoder;
    let mut dec = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?.into_decoder().map_err(|e| e.to_string())?;
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Write `path` as another picture format (`to` is an extension from `CONVERT_TO`) beside it, under a name that is free
/// ("photo.png" -> "photo.jpg", or "photo (2).jpg"). Formats without transparency get the picture over white; an icon
/// is at most 256 px. An animation keeps its first frame. Returns the new file.
pub fn convert_image(path: &Path, to: &str) -> Result<PathBuf, String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "picture".to_string());
    convert_image_to(path, &format!("{stem}.{to}"))
}

/// What a conversion can be told besides the format (the converter window sets them; the menu uses the defaults).
#[derive(Clone, Debug, PartialEq)]
pub struct ConvertOptions {
    /// JPEG quality, 1 .. 100
    pub quality: u8,
    /// PNG compression: 0 fast, 1 balanced, 2 smallest
    pub png_level: u8,
    /// the longest side the result may have (a bigger picture is made smaller, never a smaller one bigger)
    pub max_side: Option<u32>,
    /// what a see-through picture is put over for the formats that cannot be see-through (JPEG, BMP)
    pub background: [u8; 3],
    /// the folder the result goes in (None: beside the original)
    pub out_dir: Option<PathBuf>,
}

impl Default for ConvertOptions {
    fn default() -> ConvertOptions {
        ConvertOptions { quality: 90, png_level: 1, max_side: None, background: [255; 3], out_dir: None }
    }
}

/// Write the picture as the file `out_name` beside it (the format is the one the name's ending says), under a free name
/// when that one is taken. The original stays.
pub fn convert_image_to(path: &Path, out_name: &str) -> Result<PathBuf, String> {
    convert_image_with(path, out_name, &ConvertOptions::default())
}

/// `convert_image_to` with the options.
pub fn convert_image_with(path: &Path, out_name: &str, opts: &ConvertOptions) -> Result<PathBuf, String> {
    use image::codecs::{jpeg::JpegEncoder, png::{CompressionType, FilterType as PngFilter, PngEncoder}};
    let to = out_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).ok_or("the name has no ending")?;
    let format = image::ImageFormat::from_extension(&to).ok_or("unknown format")?;
    let mut img = load_picture(path)?;
    if let Some(n) = opts.max_side.filter(|n| *n >= 1) {
        if img.width().max(img.height()) > n {
            img = img.resize(n, n, image::imageops::FilterType::Lanczos3);
        }
    }
    match format {
        image::ImageFormat::Jpeg | image::ImageFormat::Bmp => {
            // over the background (white by default), not over black
            let bg = opts.background.map(|c| c as u32);
            let mut rgba = img.to_rgba8();
            for p in rgba.pixels_mut() {
                let a = p[3] as u32;
                for (i, c) in p.0[..3].iter_mut().enumerate() {
                    *c = ((*c as u32 * a + bg[i] * (255 - a)) / 255) as u8;
                }
                p[3] = 255;
            }
            img = image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(rgba).to_rgb8());
        }
        image::ImageFormat::Ico if img.width().max(img.height()) > 256 => img = img.resize(256, 256, image::imageops::FilterType::Lanczos3),
        _ => {}
    }
    let dir = opts.out_dir.clone().unwrap_or_else(|| path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf());
    let out = unique_path(&dir, out_name);
    let write = || -> Result<(), String> {
        let file = std::io::BufWriter::new(fs::File::create(&out).map_err(|e| e.to_string())?);
        match format {
            image::ImageFormat::Jpeg => img.write_with_encoder(JpegEncoder::new_with_quality(file, opts.quality.clamp(1, 100))).map_err(|e| e.to_string()),
            image::ImageFormat::Png => {
                let level = match opts.png_level {
                    0 => CompressionType::Fast,
                    2 => CompressionType::Best,
                    _ => CompressionType::Default,
                };
                img.write_with_encoder(PngEncoder::new_with_quality(file, level, PngFilter::Adaptive)).map_err(|e| e.to_string())
            }
            _ => {
                drop(file);
                img.save_with_format(&out, format).map_err(|e| e.to_string())
            }
        }
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(&out);
        return Err(e);
    }
    Ok(out)
}

/// An ending as the format list spells it ("jpeg" is "jpg").
fn canonical_ext(e: &str) -> String {
    match e.to_ascii_lowercase().as_str() {
        "jpeg" | "jfif" => "jpg".into(),
        "tif" => "tiff".into(),
        other => other.to_string(),
    }
}

/// Renaming a picture so that it ends in the name of another picture format: the label of that format
/// (`Some("JPEG")`) when the picture could be converted to it, which is probably what was meant.
pub fn rename_converts_to(path: &Path, new_name: &str) -> Option<&'static str> {
    if !can_convert(path) {
        return None;
    }
    let new = canonical_ext(new_name.rsplit_once('.').map(|(s, e)| if s.is_empty() { "" } else { e }).unwrap_or(""));
    if new.is_empty() || new == canonical_ext(&extension_of(path)) {
        return None;
    }
    CONVERT_TO.iter().find(|(_, ext)| *ext == new).map(|(label, _)| *label)
}

/// The question to ask when `rename_converts_to` says so: convert, or only change the name.
pub fn rename_prompt(path: &Path, new_name: &str, label: &str) -> Vec<crate::menu::MenuItem> {
    use crate::files::cmd::{RENAME_CONVERT, RENAME_ONLY};
    use crate::menu::MenuItem;
    let old = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    vec![MenuItem::new(0, format!("{old} → {new_name}")).enabled(false), MenuItem::sep(), MenuItem::new(RENAME_CONVERT, format!("Convert to {label}")).icon("image"), MenuItem::new(RENAME_ONLY, "Only rename").icon("pencil")]
}

/// Convert each picture; the status line text says how it went.
pub fn convert_all(paths: &[PathBuf], to: &str) -> String {
    let (mut done, mut last_err) = (0, None);
    for p in paths.iter().filter(|p| can_convert(p)) {
        match convert_image(p, to) {
            Ok(_) => done += 1,
            Err(e) => last_err = Some(format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
        }
    }
    match last_err {
        Some(e) => format!("Could not convert ({done} done): {e}"),
        None if done == 1 => "Converted 1 picture".to_string(),
        None => format!("Converted {done} pictures"),
    }
}

/// The "Convert to" submenu of the context menu: when every selected file is a picture. A single picture is not offered
/// its own format. The ids are `base` plus the index in `CONVERT_TO`.
pub fn convert_menu(paths: &[PathBuf], base: u32) -> Option<crate::menu::MenuItem> {
    use crate::menu::MenuItem;
    if paths.is_empty() || !paths.iter().all(|p| can_convert(p)) {
        return None;
    }
    let own = if let [p] = paths { Some(canonical_ext(&extension_of(p))) } else { None };
    // only the formats most are after; "More…" opens the converter with the rest and its settings
    let mut items: Vec<MenuItem> = QUICK_FORMATS.iter().filter(|&&i| own.as_deref() != Some(CONVERT_TO[i].1)).map(|&i| MenuItem::new(base + i as u32, CONVERT_TO[i].0)).collect();
    items.push(MenuItem::sep());
    items.push(MenuItem::new(base + CONVERT_TO.len() as u32, "More…").icon("sliders-horizontal"));
    Some(MenuItem::new(0, "Convert to").icon("image").sub(items))
}

/// The formats the context menu lists itself (indices into `CONVERT_TO`): PNG, JPEG, WebP.
const QUICK_FORMATS: [usize; 3] = [0, 1, 2];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_and_media_open_in_our_own_programs() {
        for (file, program) in [("a.png", Some("memrium_view")), ("B.JPG", Some("memrium_view")), ("logo.svg", Some("memrium_view")), ("song.flac", Some("memrium_player")), ("movie.mkv", Some("memrium_player")), ("notes.txt", None), ("report.pdf", None), ("raw.dng", None)] {
            assert_eq!(own_opener(Path::new(file)), program, "{file}");
        }
    }

    #[test]
    fn folders_and_links_are_left_to_the_system() {
        assert_eq!(own_opener(&std::env::temp_dir()), None);
        assert_eq!(own_opener(Path::new("https://example.com/watch?v=1")), None);
    }

    #[test]
    fn pictures_convert_beside_the_original() {
        let dir = std::env::temp_dir().join(format!("memrium-conv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // a 300 x 200 picture with a see-through half
        let mut img = image::RgbaImage::from_pixel(300, 200, image::Rgba([200, 30, 30, 255]));
        for x in 150..300 {
            for y in 0..200 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        let src = dir.join("pic.png");
        img.save(&src).unwrap();
        assert!(can_convert(&src));
        assert!(!can_convert(&dir));
        for (_, ext) in CONVERT_TO {
            let out = convert_image(&src, ext).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(out.exists(), "{ext}");
            let back = image::open(&out).unwrap_or_else(|e| panic!("{ext} unreadable: {e}"));
            assert_eq!((back.width(), back.height()), if ext == "ico" { (256, 171) } else { (300, 200) }, "{ext}");
        }
        // a second conversion does not overwrite the first
        let again = convert_image(&src, "jpg").unwrap();
        assert_eq!(again.file_name().unwrap().to_string_lossy(), "pic (2).jpg");
        // JPEG has no transparency: the see-through half is white
        let jpg = image::open(dir.join("pic.jpg")).unwrap().to_rgb8();
        assert!(jpg.get_pixel(250, 100).0.iter().all(|&c| c > 240));
        // a single picture is not offered its own format; two different ones are offered all
        let menu = convert_menu(&[src.clone()], 400).unwrap();
        // a few formats (not its own), a separator, and "More…" for the rest and the settings
        let labels: Vec<&str> = menu.sub.iter().filter(|i| !i.separator).map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["JPEG", "WebP", "More…"]);
        assert_eq!(menu.sub.last().unwrap().id, 400 + CONVERT_TO.len() as u32);
        assert!(convert_menu(&[dir.clone()], 400).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_ending_on_a_picture_offers_the_conversion() {
        let p = Path::new("/x/photo.png");
        assert_eq!(rename_converts_to(p, "photo.jpg"), Some("JPEG"));
        assert_eq!(rename_converts_to(p, "holiday.JPEG"), Some("JPEG"));
        assert_eq!(rename_converts_to(p, "photo.tif"), Some("TIFF"));
        // not when the format stays, the new ending is no picture format, or the file is no picture
        assert_eq!(rename_converts_to(p, "other.png"), None);
        assert_eq!(rename_converts_to(p, "photo.txt"), None);
        assert_eq!(rename_converts_to(p, "photo"), None);
        assert_eq!(rename_converts_to(Path::new("/x/photo.jpeg"), "photo.jpg"), None);
        assert_eq!(rename_converts_to(Path::new("/x/notes.txt"), "notes.png"), None);
        // the converted file gets the name that was typed, and the original stays
        let dir = std::env::temp_dir().join(format!("memrium-renconv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("pic.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255])).save(&src).unwrap();
        let out = convert_image_to(&src, "holiday.jpg").unwrap();
        assert_eq!(out.file_name().unwrap().to_string_lossy(), "holiday.jpg");
        assert!(src.exists() && image::open(&out).is_ok());
        assert_eq!(convert_image_to(&src, "holiday.jpg").unwrap().file_name().unwrap().to_string_lossy(), "holiday (2).jpg");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn conversion_options_change_the_result() {
        let dir = std::env::temp_dir().join(format!("memrium-copts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("out")).unwrap();
        // a noisy 400 x 200 picture with a see-through left half
        let mut img = image::RgbaImage::new(400, 200);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let n = ((x * 7919 + y * 104729) % 251) as u8;
            *p = if x < 200 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([n, 255 - n, n / 2, 255]) };
        }
        let src = dir.join("pic.png");
        img.save(&src).unwrap();
        // the quality makes a smaller or bigger file
        let low = convert_image_with(&src, "low.jpg", &ConvertOptions { quality: 20, ..Default::default() }).unwrap();
        let high = convert_image_with(&src, "high.jpg", &ConvertOptions { quality: 98, ..Default::default() }).unwrap();
        assert!(fs::metadata(&low).unwrap().len() < fs::metadata(&high).unwrap().len());
        // a see-through part takes the chosen background
        let black = convert_image_with(&src, "black.jpg", &ConvertOptions { background: [0, 0, 0], ..Default::default() }).unwrap();
        let (b, w) = (image::open(&black).unwrap().to_rgb8(), image::open(dir.join("low.jpg")).unwrap().to_rgb8());
        assert!(b.get_pixel(50, 100).0.iter().all(|&c| c < 20) && w.get_pixel(50, 100).0.iter().all(|&c| c > 235));
        // a size limit makes a big picture smaller, in proportion, and leaves a small one alone
        let small = convert_image_with(&src, "small.png", &ConvertOptions { max_side: Some(100), ..Default::default() }).unwrap();
        assert_eq!(image::image_dimensions(&small).unwrap(), (100, 50));
        let same = convert_image_with(&src, "same.webp", &ConvertOptions { max_side: Some(1000), ..Default::default() }).unwrap();
        assert_eq!(image::image_dimensions(&same).unwrap(), (400, 200));
        // the best PNG is not bigger than the fast one
        let fast = convert_image_with(&src, "fast.png", &ConvertOptions { png_level: 0, ..Default::default() }).unwrap();
        let best = convert_image_with(&src, "best.png", &ConvertOptions { png_level: 2, ..Default::default() }).unwrap();
        assert!(fs::metadata(&best).unwrap().len() <= fs::metadata(&fast).unwrap().len());
        // another folder
        let elsewhere = convert_image_with(&src, "x.bmp", &ConvertOptions { out_dir: Some(dir.join("out")), ..Default::default() }).unwrap();
        assert_eq!(elsewhere.parent().unwrap(), dir.join("out"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    #[test]
    fn servers_are_found_in_the_mount_table_and_named() {
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n//nas.local/photos /mnt/nas cifs rw 0 0\nserver:/export/media /mnt/media nfs4 rw 0 0\nme@box:/home/me /mnt/box fuse.sshfs rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n";
        let m = network_mounts(table);
        let names: Vec<&str> = m.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["photos on nas.local", "media on server", "me on box"]);
        assert_eq!(m[0].1, PathBuf::from("/mnt/nas"));
    }

    #[test]
    fn gvfs_folders_read_as_names() {
        assert_eq!(gvfs_label("smb-share:server=nas,share=photos"), "photos on nas");
        assert_eq!(gvfs_label("sftp:host=box,user=me"), "me@box");
        assert_eq!(gvfs_label("mtp:host=phone"), "mtp:host=phone");
    }

    #[test]
    fn addresses_become_urls_and_paths_do_not() {
        assert_eq!(server_url("smb://nas/photos").as_deref(), Some("smb://nas/photos"));
        assert_eq!(server_url("  SFTP://me@box/ ").as_deref(), Some("sftp://me@box/"));
        assert_eq!(server_url("ssh://box").as_deref(), Some("sftp://box"));
        assert_eq!(server_url("\\\\nas\\photos\\2024").as_deref(), Some("smb://nas/photos/2024"));
        assert_eq!(server_url("/home/me"), None);
        assert_eq!(server_url("smb://"), None);
        assert_eq!(server_url("https://example.com"), None);
    }
}
