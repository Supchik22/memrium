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
        "rs" | "py" | "js" | "mjs" | "jsx" | "ts" | "tsx" | "c" | "h" | "cpp" | "cc" | "hpp" | "go" | "java" | "kt" | "cs" | "php" | "rb" | "pl" | "swift" | "zig" | "sh" | "bash" | "fish" | "nix" | "toml"
        | "json" | "yaml" | "yml" | "html" | "htm" | "css" | "scss" | "wgsl" | "glsl" | "lua" | "xml" | "sql" | "lock" | "cmake" | "mk" | "diff" | "patch" | "ron" | "vue" | "svelte" | "hs" | "dart" | "kts" | "pm" | "jl" | "ex" | "exs" | "erl" | "scala" | "clj" => Kind::Code,
        "txt" | "md" | "markdown" | "rst" | "adoc" | "org" | "tex" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "log" | "csv" | "epub" | "ini" | "conf" | "cfg" | "nfo" => Kind::Text,
        // files known by their whole name: the build files of a project are code, the notes beside it are text
        _ => match whole_name_icon(name) {
            Some("text-x-readme" | "text-x-copying" | "text-x-changelog" | "text-x-authors" | "text-x-install" | "text-x-todo") => Kind::Text,
            Some(_) => Kind::Code,
            None => Kind::Other,
        },
    }
}

/// The icon of a file that its whole name tells (the name is a kind of file in itself): `README`, `LICENSE`, `Makefile`,
/// `Cargo.lock`, `.gitignore` ... The notes of a project (README, LICENSE, CHANGELOG, AUTHORS, INSTALL) count with or
/// without an extension of prose (`README.md`, `LICENSE-MIT`, `CHANGELOG.txt`); `install.sh` is a script, not an INSTALL.
fn whole_name_icon(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let known = match lower.as_str() {
        // a project's own files carry the logo of the tool they belong to
        "cargo.toml" | "cargo.lock" => Some("application-x-cargo"),
        "flake.lock" => Some("text-x-nix"),
        "package.json" | "package-lock.json" | ".npmrc" => Some("application-x-npm"),
        "yarn.lock" | ".yarnrc.yml" => Some("application-x-yarn"),
        "pnpm-lock.yaml" | "pnpm-workspace.yaml" => Some("application-x-pnpm"),
        "poetry.lock" | "uv.lock" | "pipfile.lock" => Some("text-x-python"),
        "composer.lock" => Some("application-x-php"),
        "gemfile.lock" => Some("text-x-ruby"),
        "go.sum" | "go.mod" => Some("text-x-go"),
        "bun.lock" | "deno.lock" => Some("application-x-lock"),
        "makefile" | "gnumakefile" | "bsdmakefile" => Some("text-x-makefile"),
        "cmakelists.txt" => Some("text-x-cmake"),
        "dockerfile" | "containerfile" | "compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml" => Some("text-x-dockerfile"),
        "justfile" => Some("text-x-justfile"),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".dockerignore" | ".ignore" | ".npmignore" => Some("text-x-gitignore"),
        "flake.nix" | "default.nix" | "shell.nix" => Some("text-x-nix"),
        _ => None,
    };
    if known.is_some() {
        return known;
    }
    // the stem of a note: `README`, `README.md`, `LICENSE-MIT`, `CHANGELOG.txt`
    let stem = lower.split(['.', '-', '_']).next().unwrap_or("");
    let prose = match lower.rsplit_once('.') {
        None => true,
        Some((_, ext)) => matches!(ext, "md" | "markdown" | "txt" | "rst" | "adoc" | "org" | "text"),
    } || lower.starts_with("license-")
        || lower.starts_with("licence-")
        || lower.starts_with("copying-");
    if !prose {
        return None;
    }
    Some(match stem {
        "readme" => "text-x-readme",
        "license" | "licence" | "copying" | "unlicense" | "notice" | "copyright" => "text-x-copying",
        "changelog" | "changes" | "news" | "history" | "releases" => "text-x-changelog",
        "authors" | "contributors" | "maintainers" | "credits" | "thanks" => "text-x-authors",
        "install" | "installing" => "text-x-install",
        "todo" => "text-x-todo",
        "contributing" | "security" | "conduct" => "text-x-readme",
        _ => return None,
    })
}

/// A name kept for good: the icon names of file types come out of the MIME database (a few hundred at most) and the callers
/// want `&'static str`.
fn intern(s: String) -> &'static str {
    static NAMES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static str>>> = std::sync::OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap();
    if let Some(n) = names.get(&s) {
        return n;
    }
    let kept: &'static str = Box::leak(s.clone().into_boxed_str());
    names.insert(s, kept);
    kept
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
                "Public" => Some("folder-publicshare"),
                "Templates" => Some("folder-templates"),
                _ => None,
            };
            if let Some(s) = special {
                return vec![s, "folder"];
            }
        }
        if e.path == home {
            return vec!["user-home", "folder"];
        }
        return vec!["folder"];
    }
    let ext = e.name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
    // what the whole name says comes first (README, LICENSE, Makefile, Cargo.lock ...), then the extension; the names are the
    // ones our own icons carry (the pictures of `kind_icons`), then what the icon packs call them
    let mut v: Vec<&'static str> = Vec::new();
    v.extend(whole_name_icon(&e.name));
    v.extend(match ext.as_str() {
        "pdf" => vec!["application-pdf"],
        "html" | "htm" => vec!["text-html"],
        "json" => vec!["application-json", "text-x-script"],
        "toml" => vec!["application-toml"],
        "yaml" | "yml" => vec!["application-yaml"],
        "nix" => vec!["text-x-nix"],
        "lock" => vec!["application-x-lock"],
        "md" | "markdown" => vec!["text-markdown", "text-x-generic"],
        "sh" | "fish" | "bash" | "zsh" => vec!["application-x-shellscript", "text-x-script"],
        "rs" => vec!["text-rust", "text-x-script"],
        "py" => vec!["text-x-python", "text-x-script"],
        "css" => vec!["text-css"],
        "scss" | "sass" => vec!["text-x-scss", "text-css"],
        "xml" => vec!["text-xml", "application-xml"],
        "c" => vec!["text-x-csrc"],
        "h" => vec!["text-x-chdr"],
        "cpp" | "cc" | "cxx" => vec!["text-x-c++src"],
        "hpp" | "hh" | "hxx" => vec!["text-x-c++hdr"],
        "js" | "mjs" | "cjs" | "jsx" => vec!["text-javascript", "application-javascript"],
        "ts" | "tsx" | "mts" => vec!["text-x-typescript"],
        "go" => vec!["text-x-go"],
        "php" => vec!["application-x-php", "text-x-php"],
        "rb" => vec!["text-x-ruby", "application-x-ruby"],
        "pl" | "pm" => vec!["text-x-perl"],
        "swift" => vec!["text-x-swift"],
        "kt" | "kts" => vec!["text-x-kotlin"],
        "zig" => vec!["text-x-zig"],
        "dart" => vec!["text-x-dart"],
        "hs" => vec!["text-x-haskell"],
        "ipynb" => vec!["application-x-ipynb+json"],
        "wasm" => vec!["application-wasm"],
        "jar" => vec!["application-x-java-archive"],
        "apk" => vec!["application-vnd.android.package-archive"],
        "deb" => vec!["application-vnd.debian.binary-package"],
        "rpm" => vec!["application-x-rpm"],
        "java" => vec!["text-x-java", "text-x-java-source"],
        "lua" => vec!["text-x-lua"],
        "sql" => vec!["text-x-sql", "application-sql"],
        "log" => vec!["text-x-log"],
        "diff" | "patch" => vec!["text-x-patch", "text-x-diff"],
        "tex" => vec!["text-x-tex"],
        "ini" | "conf" | "cfg" => vec!["text-x-ini"],
        "mk" => vec!["text-x-makefile"],
        "cmake" => vec!["text-x-cmake"],
        "iso" => vec!["application-x-cd-image", "media-optical"],
        "zip" | "7z" | "rar" => vec!["application-zip", "application-x-archive", "package-x-generic"],
        "tar" | "gz" | "xz" | "bz2" | "zst" => vec!["application-x-compressed-tar", "application-x-archive", "package-x-generic"],
        // office: the logo of the program the file belongs to (Word, Excel and PowerPoint by their letters, LibreOffice by its icons)
        "doc" | "docx" | "dotx" => vec!["application-msword", "x-office-document"],
        "odt" | "ott" => vec!["application-vnd.oasis.opendocument.text", "x-office-document"],
        "xls" | "xlsx" => vec!["application-vnd.ms-excel", "x-office-spreadsheet"],
        "ods" | "ots" => vec!["application-vnd.oasis.opendocument.spreadsheet", "x-office-spreadsheet"],
        "ppt" | "pptx" => vec!["application-vnd.ms-powerpoint", "x-office-presentation"],
        "odp" | "otp" => vec!["application-vnd.oasis.opendocument.presentation", "x-office-presentation"],
        "csv" => vec!["text-csv", "x-office-spreadsheet"],
        "flatpak" => vec!["application-vnd.flatpak", "package-x-generic"],
        // a download that is still coming (Firefox, Chrome, Safari, qBittorrent, wget2)
        "part" | "crdownload" | "download" | "partial" | "!qb" | "tmpdownload" => vec!["application-x-partial-download"],
        _ => vec![],
    });
    // then the type the shared MIME database gives the name (`text/x-readme`, `application/toml` ...) and the types that is a
    // kind of: that is how the icon packs name their pictures, and they have one for far more kinds of files than the list above.
    // Pictures, songs and films keep their own icons (and thumbnails): the database's `.ts` is a film.
    if !matches!(e.kind, Kind::Image | Kind::Audio | Kind::Video) {
        if let Some(mime) = crate::mimeapps::mime_of_name(&e.name) {
            for t in crate::mimeapps::lineage(&mime) {
                // (the generic ones are the end of the list, below)
                if t == "text/plain" || t == "application/octet-stream" {
                    continue;
                }
                let name = intern(t.replace('/', "-"));
                let moving = name.starts_with("video-") || name.starts_with("audio-") || name.starts_with("image-");
                if !(moving && matches!(e.kind, Kind::Code | Kind::Text)) && !v.contains(&name) {
                    v.push(name);
                }
            }
        }
    }
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

/// The colours a folder can be given (as Dolphin does: `Icon=folder-<colour>` in the folder's `.directory`): the end of
/// the icon's name, its name in the menu and the colour of its spot there.
pub const FOLDER_COLOURS: [(&str, &str, [f32; 3]); 8] = [
    ("red", "Red", [0.90, 0.28, 0.30]),
    ("orange", "Orange", [0.96, 0.52, 0.16]),
    ("yellow", "Yellow", [0.96, 0.75, 0.16]),
    ("green", "Green", [0.20, 0.72, 0.35]),
    ("teal", "Teal", [0.12, 0.69, 0.69]),
    ("violet", "Violet", [0.54, 0.36, 0.94]),
    ("pink", "Pink", [0.93, 0.30, 0.61]),
    ("grey", "Grey", [0.56, 0.58, 0.62]),
];

/// The spot of a folder that has no colour given.
pub const FOLDER_BLUE: [f32; 3] = [0.30, 0.62, 0.95];

/// The icon of a folder by what is in it, as far as the first few hundred names tell: the language of a project (a
/// `Cargo.toml`, a `flake.nix` ...), a git repository, or the kind most of its files are of (photos, music, films, code,
/// documents, archives). None: a mixed or empty folder, or one that is not looked into (on the network).
pub fn folder_emblem(dir: &Path) -> Option<&'static str> {
    if is_network_path(dir) {
        return None;
    }
    let mut names: Vec<(String, bool)> = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten().take(400) {
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        names.push((e.file_name().to_string_lossy().to_string(), is_dir));
    }
    let has = |n: &str| names.iter().any(|(m, _)| m == n);
    const PROJECTS: [(&[&str], &str); 5] = [
        (&["Cargo.toml"], "folder-rust"),
        (&["package.json", "deno.json"], "folder-node"),
        (&["pyproject.toml", "setup.py", "requirements.txt", "Pipfile"], "folder-python"),
        (&["flake.nix", "default.nix", "shell.nix"], "folder-nix"),
        (&["go.mod", "CMakeLists.txt", "meson.build", "Makefile", "build.gradle", "pom.xml", "build.zig"], "folder-code"),
    ];
    for (marks, icon) in PROJECTS {
        if marks.iter().any(|m| has(m)) {
            return Some(icon);
        }
    }
    if has(".git") {
        return Some("folder-git");
    }
    // the kind most files are of: at least three of them, and three in four of the files
    let mut count: std::collections::HashMap<u8, usize> = Default::default();
    let mut files = 0;
    for (n, is_dir) in &names {
        if *is_dir || n.starts_with('.') {
            continue;
        }
        files += 1;
        let k = match kind_of(n, false) {
            Kind::Image => 0,
            Kind::Audio => 1,
            Kind::Video => 2,
            Kind::Code => 3,
            Kind::Text => 4,
            Kind::Archive => 5,
            _ => continue,
        };
        *count.entry(k).or_default() += 1;
    }
    let (&k, &n) = count.iter().max_by_key(|(_, n)| **n)?;
    if n < 3 || n * 4 < files * 3 {
        return None;
    }
    Some(["folder-pictures", "folder-music", "folder-videos", "folder-code", "folder-documents", "folder-archive"][k as usize])
}

/// The colour tags a file can be given (as Finder has them), with the colour of their spot. A tag is kept in the file's
/// `user.xdg.tags` attribute, where Dolphin keeps its tags too, so both see the same.
pub const TAG_COLOURS: [(&str, [f32; 3]); 7] = [
    ("Red", [0.93, 0.27, 0.27]),
    ("Orange", [0.97, 0.56, 0.15]),
    ("Yellow", [0.98, 0.80, 0.18]),
    ("Green", [0.25, 0.75, 0.35]),
    ("Blue", [0.22, 0.52, 0.96]),
    ("Purple", [0.66, 0.36, 0.90]),
    ("Grey", [0.58, 0.60, 0.64]),
];

/// The colour of tag `name`: one of the colour tags (in any letter case), else a quiet grey for a tag of words.
pub fn tag_colour(name: &str) -> [f32; 3] {
    TAG_COLOURS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).map_or([0.72, 0.74, 0.78], |(_, c)| *c)
}

const TAGS_ATTR: &str = "user.xdg.tags";

/// The tags of a file (`user.xdg.tags`, comma-separated); none when the file system keeps no attributes.
pub fn tags_of(path: &Path) -> Vec<String> {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(p), Ok(name)) = (std::ffi::CString::new(path.as_os_str().as_bytes()), std::ffi::CString::new(TAGS_ATTR)) else { return Vec::new() };
    // (the most an attribute holds: a list cut short would be written back without the rest)
    let mut buf = vec![0u8; 64 * 1024];
    let n = unsafe { libc::getxattr(p.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n <= 0 {
        return Vec::new();
    }
    buf.truncate(n as usize);
    String::from_utf8_lossy(&buf).split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// Give a file these tags (none: the attribute goes), and note them in the list of tagged files (`tagged`).
pub fn set_tags(path: &Path, tags: &[String]) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let name = std::ffi::CString::new(TAGS_ATTR)?;
    let r = if tags.is_empty() {
        let r = unsafe { libc::removexattr(p.as_ptr(), name.as_ptr()) };
        if r != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA) { 0 } else { r }
    } else {
        let v = tags.join(",");
        unsafe { libc::setxattr(p.as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0) }
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    note_tagged(path, !tags.is_empty());
    Ok(())
}

/// The list of files that have tags (the attribute is the truth; this only says where to look, so that a tag can be found
/// without walking the disk): `~/.local/share/memrium/tagged`, a path a line.
fn tagged_list() -> PathBuf {
    crate::apps::data_dirs().into_iter().next().unwrap_or_else(|| home().join(".local/share")).join("memrium/tagged")
}

fn note_tagged(path: &Path, has: bool) {
    let file = tagged_list();
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let line = path.to_string_lossy().to_string();
    let mut lines: Vec<&str> = text.lines().filter(|l| *l != line).collect();
    if has {
        lines.push(&line);
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&file, lines.join("\n") + if lines.is_empty() { "" } else { "\n" });
}

/// Every file that has tag `tag` now (from the list of tagged files, each checked again), and the tags in use at all.
pub fn tagged(tag: Option<&str>) -> (Vec<PathBuf>, Vec<String>) {
    let text = std::fs::read_to_string(tagged_list()).unwrap_or_default();
    let (mut files, mut used) = (Vec::new(), Vec::<String>::new());
    for l in text.lines().filter(|l| !l.is_empty()) {
        let p = PathBuf::from(l);
        let tags = tags_of(&p);
        for t in &tags {
            if !used.iter().any(|u| u.eq_ignore_ascii_case(t)) {
                used.push(t.clone());
            }
        }
        if tag.is_some_and(|want| tags.iter().any(|t| t.eq_ignore_ascii_case(want))) {
            files.push(p);
        }
    }
    // the colour tags in their own order, then the others by name
    used.sort_by_key(|t| (TAG_COLOURS.iter().position(|(n, _)| n.eq_ignore_ascii_case(t)).unwrap_or(99), t.to_lowercase()));
    (files, used)
}

/// The icon a folder asks for in its `.directory` (`[Desktop Entry] Icon=`), as Dolphin writes it.
pub fn folder_icon(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".directory")).ok()?;
    let mut inside = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == "[Desktop Entry]";
        } else if inside {
            if let Some(v) = l.strip_prefix("Icon=") {
                let v = v.trim();
                return (!v.is_empty()).then(|| v.to_string());
            }
        }
    }
    None
}

/// A folder's colour of the user's own ("Other…") beside the icon, in its `.directory`.
const COLOUR_KEY: &str = "X-Memrium-Colour=";

/// The colour of its own a folder was given (`set_folder_colour`).
pub fn folder_colour(dir: &Path) -> Option<[f32; 3]> {
    let text = std::fs::read_to_string(dir.join(".directory")).ok()?;
    let mut inside = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == "[Desktop Entry]";
        } else if inside {
            if let Some(v) = l.strip_prefix(COLOUR_KEY) {
                return crate::theme::from_hex(v.trim());
            }
        }
    }
    None
}

/// The icon of another kind a coloured folder had (Dolphin's `folder-music`, say), kept while `Icon=` names its colour
/// for the programs that know only that: we draw both, the icon in the colour.
const SHAPE_KEY: &str = "X-Memrium-Icon=";

/// The colour a colour icon (`folder-red` ... `folder-blue`) stands for; None for any other icon.
pub fn colour_of_icon(icon: &str) -> Option<[f32; 3]> {
    let name = icon.strip_prefix("folder-")?;
    if name == "blue" {
        return Some(FOLDER_BLUE);
    }
    FOLDER_COLOURS.iter().find(|f| f.0 == name).map(|f| f.2)
}

/// A line of the folder's `[Desktop Entry]` that starts with `key`, without it.
fn entry_line(dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".directory")).ok()?;
    let mut inside = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == "[Desktop Entry]";
        } else if inside {
            if let Some(v) = l.strip_prefix(key) {
                let v = v.trim();
                return (!v.is_empty()).then(|| v.to_string());
            }
        }
    }
    None
}

/// The icon a folder has besides its colour: the one kept beside the colour, else its `Icon=` when that is no colour.
pub fn folder_shape(dir: &Path) -> Option<String> {
    entry_line(dir, SHAPE_KEY).or_else(|| folder_icon(dir).filter(|i| colour_of_icon(i).is_none()))
}

/// The colour a folder is drawn in: its own one, else the named one of its icon.
pub fn folder_tint(dir: &Path) -> Option<[f32; 3]> {
    folder_colour(dir).or_else(|| folder_icon(dir).and_then(|i| colour_of_icon(&i)))
}

/// Give a folder a named colour (`folder-red` ...), or none: the icon it had besides keeps (as `X-Memrium-Icon`, and
/// back in `Icon=` when the colour goes).
pub fn set_folder_paint(dir: &Path, colour_icon: Option<&str>) -> std::io::Result<()> {
    let shape = folder_shape(dir);
    match colour_icon {
        Some(c) => write_entry(dir, Some(c), &shape.map(|s| format!("{SHAPE_KEY}{s}")).into_iter().collect::<Vec<_>>()),
        None => write_entry(dir, shape.as_deref(), &[]),
    }
}

/// Give a folder any colour. Dolphin and the others, which know only the named ones, get the nearest of those as the icon;
/// the icon it had besides keeps.
pub fn set_folder_colour(dir: &Path, c: [f32; 3]) -> std::io::Result<()> {
    let far = |o: &[f32; 3]| (0..3).map(|i| (o[i] - c[i]).powi(2)).sum::<f32>();
    let blue = ("blue", "Blue", FOLDER_BLUE);
    let near = FOLDER_COLOURS.iter().chain([&blue]).min_by(|a, b| far(&a.2).total_cmp(&far(&b.2))).map_or("blue", |f| f.0);
    let mut extra = vec![format!("{COLOUR_KEY}{}", crate::theme::to_hex(c))];
    extra.extend(folder_shape(dir).map(|s| format!("{SHAPE_KEY}{s}")));
    write_entry(dir, Some(&format!("folder-{near}")), &extra)
}

/// A folder picture in colour `to`: what has colour takes it (keeping its shading), paper and white marks stay.
pub fn recolour_folder(img: &image::RgbaImage, to: [f32; 3]) -> image::RgbaImage {
    use crate::theme::{hsv_to_rgb, rgb_to_hsv};
    let hsv = |p: &image::Rgba<u8>| rgb_to_hsv([p.0[0] as f32 / 255.0, p.0[1] as f32 / 255.0, p.0[2] as f32 / 255.0]);
    // the body's hue: the commonest among what is clearly coloured (a mark of another colour on it, a Rust gear or a
    // git logo, is not the body and keeps its colour)
    let mut bins = [0usize; 36];
    for p in img.pixels().filter(|p| p.0[3] > 200) {
        let [h, s, _] = hsv(p);
        if s > 0.25 {
            bins[((h.rem_euclid(1.0) * 36.0) as usize).min(35)] += 1;
        }
    }
    let Some(peak) = (0..36).max_by_key(|&i| (bins[i] + bins[(i + 1) % 36] + bins[(i + 35) % 36], bins[i])).filter(|&i| bins[i] > 0) else {
        return img.clone();
    };
    let body = (peak as f32 + 0.5) / 36.0;
    // how far a hue is from the body's (0 the same, 0.5 across): near is the body, far is a mark
    let near = |h: f32| {
        let d = (h - body).rem_euclid(1.0);
        let d = d.min(1.0 - d);
        (1.0 - (d - 0.08) / 0.06).clamp(0.0, 1.0)
    };
    // the body's own saturation and brightness: the average of its clearly coloured part
    let (mut sat, mut val, mut n) = (0.0f32, 0.0f32, 0.0f32);
    for p in img.pixels().filter(|p| p.0[3] > 200) {
        let [h, s, v] = hsv(p);
        if s > 0.25 && near(h) > 0.5 {
            sat += s;
            val += v;
            n += 1.0;
        }
    }
    let (sat, val) = (sat / n, (val / n).max(0.05));
    let [th, ts, tv] = rgb_to_hsv(to);
    let mut out = img.clone();
    for p in out.pixels_mut().filter(|p| p.0[3] > 0) {
        let [h, s, v] = hsv(p);
        let k = ((s - 0.08) / 0.17).clamp(0.0, 1.0) * near(h);
        if k <= 0.0 {
            continue;
        }
        let new = hsv_to_rgb(th, (ts * s / sat).min(1.0), (tv * v / val).min(1.0));
        for i in 0..3 {
            p.0[i] = (p.0[i] as f32 * (1.0 - k) + new[i] * 255.0 * k).round() as u8;
        }
    }
    out
}

/// Give a folder an icon (None: its own again), in its `.directory`; the other lines of the file are kept, and a file left
/// with nothing in it is removed.
pub fn set_folder_icon(dir: &Path, icon: Option<&str>) -> std::io::Result<()> {
    write_entry(dir, icon, &[])
}

/// Write `Icon=` (or take it out) with `extra` lines after it; the colour and icon of ours that were there go.
fn write_entry(dir: &Path, icon: Option<&str>, extra: &[String]) -> std::io::Result<()> {
    let file = dir.join(".directory");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let (mut inside, mut had_group, mut done) = (false, false, false);
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if inside && !done {
                if let Some(i) = icon {
                    out.push(format!("Icon={i}"));
                    out.extend(extra.iter().cloned());
                }
                done = true;
            }
            inside = t == "[Desktop Entry]";
            had_group |= inside;
        } else if inside && (t.starts_with(COLOUR_KEY) || t.starts_with(SHAPE_KEY)) {
            // (our colour and kept icon go with whatever is given now)
            continue;
        } else if inside && t.starts_with("Icon=") {
            if !done {
                if let Some(i) = icon {
                    out.push(format!("Icon={i}"));
                    out.extend(extra.iter().cloned());
                }
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if let Some(i) = icon {
            if !had_group {
                out.push("[Desktop Entry]".into());
            }
            out.push(format!("Icon={i}"));
            out.extend(extra.iter().cloned());
        }
    }
    // (only an empty group is left: the file says nothing)
    let said = out.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && *l != "[Desktop Entry]").count();
    if said == 0 {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::write(&file, out.join("\n") + "\n")
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
    // a program the user chose for this type ("Open With…", "Always use") beats the desktop's own viewers
    if !path.is_dir() {
        if let Some(h) = crate::mimeapps::user_default(&crate::mimeapps::mime_of(path)) {
            match crate::mimeapps::open_with(&h, &[path.to_path_buf()], socket) {
                Ok(()) => return,
                Err(e) => eprintln!("{} did not start: {e}", h.name),
            }
        }
    }
    if let Some(program) = own_opener(path) {
        crate::recent::add(path, program, program);
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
    crate::recent::add(path, "xdg-open", "xdg-open %u");
    if let Err(e) = c.spawn() {
        eprintln!("xdg-open failed: {e}");
    }
}

/// `file://` address of a path (what a `text/uri-list` line holds).
pub fn uri_of(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The local path of a `file://` address (percent escapes undone).
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.trim().strip_prefix("file://")?;
    let rest = rest.find('/').map_or("", |i| &rest[i..]);
    let b = rest.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        let hex = |c: Option<&u8>| c.and_then(|&c| (c as char).to_digit(16));
        match (b[i], hex(b.get(i + 1)), hex(b.get(i + 2))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// The files on the system clipboard as the last change reported them: `None` until the watcher has reported once (or
/// when there is no `wl-paste`), then `Some(files, cut)` where the list is empty if the clipboard holds something else.
static SYSTEM_CLIP: std::sync::Mutex<Option<(Vec<PathBuf>, bool)>> = std::sync::Mutex::new(None);

/// Follow the Wayland clipboard for files: one `wl-paste --watch` for the whole process, run on every change (no polling).
/// Each change prints the file list (`x-special/gnome-copied-files` says copy or cut; a plain `text/uri-list` is a copy)
/// and a 0x1e byte; a change to anything else prints just the byte.
fn system_clip() -> Option<(Vec<PathBuf>, bool)> {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        std::thread::spawn(|| {
            use std::io::Read;
            const SCRIPT: &str = "t=$(wl-paste -l 2>/dev/null); \
                if echo \"$t\" | grep -qx x-special/gnome-copied-files; then wl-paste -n -t x-special/gnome-copied-files; \
                elif echo \"$t\" | grep -qx text/uri-list; then echo copy; wl-paste -n -t text/uri-list; fi; printf '\\036'";
            let Ok(mut child) = std::process::Command::new("wl-paste").args(["--watch", "sh", "-c", SCRIPT]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() else { return };
            let Some(mut out) = child.stdout.take() else { return };
            let (mut buf, mut chunk) = (Vec::new(), [0u8; 4096]);
            while let Ok(n) = out.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(end) = buf.iter().position(|&b| b == 0x1e) {
                    let record: Vec<u8> = buf.drain(..=end).collect();
                    let text = String::from_utf8_lossy(&record[..record.len() - 1]).into_owned();
                    let mut lines = text.lines();
                    let cut = text.trim_start().starts_with("cut");
                    let first = lines.next();
                    // the first line is the verb; a bare uri list was given the verb "copy" above
                    let files: Vec<PathBuf> = first.into_iter().chain(lines).filter(|l| !l.starts_with('#')).filter_map(path_of_uri).collect();
                    if let Ok(mut g) = SYSTEM_CLIP.lock() {
                        *g = Some((files, cut));
                    }
                }
            }
        });
    });
    SYSTEM_CLIP.lock().ok().and_then(|g| g.clone())
}

/// Offer files to other programs on the Wayland clipboard (`text/uri-list`; wl-copy can serve only one type).
fn offer_files(paths: &[PathBuf]) {
    use std::io::Write;
    let list: String = paths.iter().map(|p| uri_of(p) + "\r\n").collect();
    if let Ok(mut c) = std::process::Command::new("wl-copy").args(["-t", "text/uri-list"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        if let Some(mut i) = c.stdin.take() {
            let _ = i.write_all(list.as_bytes());
        }
        std::thread::spawn(move || {
            let _ = c.wait();
        });
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
        offer_files(&paths);
        self.clip = paths;
        self.cut = cut;
        system_clip();
    }

    /// What a paste would bring: the files on the system clipboard once it has reported (what another program copied, or
    /// our own copy coming back with its cut mark), else what was copied here.
    fn pending(&self) -> (Vec<PathBuf>, bool) {
        match system_clip() {
            Some((files, _)) if files == self.clip => (files, self.cut),
            Some((files, cut)) => (files, cut),
            None => (self.clip.clone(), self.cut),
        }
    }

    pub fn can_paste(&self) -> bool {
        !self.pending().0.is_empty()
    }

    /// Keep a group of changes for Undo (and tell the views to reload).
    pub fn record(&mut self, group: Vec<Undo>) {
        if !group.is_empty() {
            self.undo.push(group);
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
        }
        self.gen += 1;
    }

    /// What a paste brings and whether it was a cut, for a paste done on a thread (`paste_into` is the one done at once).
    /// A cut is taken off the clipboard: its files are about to leave the place it names.
    pub fn take_paste(&mut self) -> (Vec<PathBuf>, bool) {
        let (files, cut) = self.pending();
        if cut {
            self.clip.clear();
            self.cut = false;
            if let Ok(mut g) = SYSTEM_CLIP.lock() {
                if g.is_some() {
                    *g = Some((Vec::new(), false));
                }
            }
        }
        (files, cut)
    }

    /// Paste the clipboard into `dir`; returns the new paths.
    pub fn paste_into(&mut self, dir: &Path) -> Vec<PathBuf> {
        let mut group = Vec::new();
        let mut out = Vec::new();
        let (files, cut) = self.pending();
        self.cut = cut;
        for src in files {
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
            // the moved files are gone from where the clipboard names them
            if let Ok(mut g) = SYSTEM_CLIP.lock() {
                if g.is_some() {
                    *g = Some((Vec::new(), false));
                }
            }
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

/// A mounted disk of this machine, with how full it is.
pub struct Mount {
    pub label: String,
    pub path: PathBuf,
    pub total: u64,
    pub free: u64,
}

/// Mounted disks (real file systems only), without the servers (asking one that has gone away would hold the caller up).
pub fn mounts() -> Vec<Mount> {
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
        out.push(Mount { label, path: PathBuf::from(mp), total, free });
    }
    out
}

/// Mounted disks as entries: "Name (/mount) - 120 GB free of 450 GB".
pub fn drives() -> Vec<Entry> {
    let text = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut out = Vec::new();
    for m in mounts() {
        out.push(Entry {
            name: format!("{} ({}) - {} free of {}", m.label, m.path.display(), human_size(m.free), human_size(m.total)),
            path: m.path,
            is_dir: true,
            size: m.total,
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

/// Turn each picture a quarter turn, in the file itself (clockwise or not); the status line text says how it went.
pub fn rotate_all(paths: &[PathBuf], clockwise: bool) -> String {
    let (mut done, mut last_err) = (0, None);
    for p in paths.iter().filter(|p| can_convert(p) && extension_of(p) != "svg") {
        let r = image::open(p).map_err(|e| e.to_string()).and_then(|img| {
            let turned = if clockwise { img.rotate90() } else { img.rotate270() };
            turned.save(p).map_err(|e| e.to_string())
        });
        match r {
            Ok(()) => done += 1,
            Err(e) => last_err = Some(format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
        }
    }
    match last_err {
        Some(e) => format!("Could not rotate ({done} done): {e}"),
        None if done == 1 => "Rotated 1 picture".to_string(),
        None => format!("Rotated {done} pictures"),
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

/// "Quick Edit": one entry for every quick change to the selected files, whose submenu depends on what they are (only what
/// fits all of them is offered). Pictures get "Convert to"; sound and film will get theirs when there is a converter for them.
/// `None` when nothing applies.
pub fn quick_edit_menu(paths: &[PathBuf], base: u32) -> Option<crate::menu::MenuItem> {
    use crate::menu::MenuItem;
    // the formats go straight under "Quick Edit" ("Convert to PNG"), no menu of their own in between
    let items: Vec<MenuItem> = convert_menu(paths, base)
        .map(|m| m.sub.into_iter().map(|mut i| {
            if !i.separator && i.id != base + CONVERT_TO.len() as u32 {
                i.label = format!("Convert to {}", i.label);
            }
            i
        }).collect())
        .unwrap_or_default();
    let mut items = items;
    // turning a picture round is offered for pixel pictures only (a drawing is a different thing to turn)
    if !items.is_empty() && paths.iter().all(|p| can_convert(p) && extension_of(p) != "svg") {
        items.push(MenuItem::sep());
        items.push(MenuItem::new(crate::files::cmd::ROTATE_LEFT, "Rotate Left").icon("rotate-ccw"));
        items.push(MenuItem::new(crate::files::cmd::ROTATE_RIGHT, "Rotate Right").icon("rotate-cw"));
    }
    (!items.is_empty()).then(|| MenuItem::new(0, "Quick Edit").icon("wand-sparkles").sub(items))
}

/// The formats the context menu lists itself (indices into `CONVERT_TO`): PNG, JPEG, WebP. "More…" opens the converter with the rest.
const QUICK_FORMATS: [usize; 3] = [0, 1, 2];

#[cfg(test)]
mod tests {
    fn file(name: &str) -> Entry {
        Entry { name: name.into(), path: PathBuf::from("/x").join(name), is_dir: false, size: 1, modified: 0, hidden: name.starts_with('.'), kind: kind_of(name, false), app: None }
    }

    #[test]
    fn a_file_is_known_by_its_name_before_its_extension() {
        let first = |n: &str| icon_names(&file(n))[0];
        assert_eq!(first("README.md"), "text-x-readme");
        assert_eq!(first("README"), "text-x-readme");
        assert_eq!(first("LICENSE-MIT"), "text-x-copying");
        assert_eq!(first("CHANGELOG.txt"), "text-x-changelog");
        assert_eq!(first("flake.nix"), "text-x-nix");
        assert_eq!(first("shell.nix"), "text-x-nix");
        assert_eq!(first("Cargo.lock"), "application-x-cargo");
        assert_eq!(first("flake.lock"), "text-x-nix");
        assert_eq!(first("package.json"), "application-x-npm");
        assert_eq!(first("bun.lock"), "application-x-lock");
        assert_eq!(first("report.docx"), "application-msword");
        assert_eq!(first("letter.odt"), "application-vnd.oasis.opendocument.text");
        assert_eq!(first("art.psd"), "image-vnd.adobe.photoshop");
        assert_eq!(first("scene.blend"), "application-x-blender");
        assert_eq!(first("Makefile"), "text-x-makefile");
        assert_eq!(first(".gitignore"), "text-x-gitignore");
        assert_eq!(first("Cargo.toml"), "application-x-cargo");
        assert_eq!(first("config.toml"), "application-toml");
        // an ordinary note and a script that only has the word in its name
        assert_eq!(first("CONCEPT.md"), "text-markdown");
        assert_eq!(first("install.sh"), "application-x-shellscript");
        assert_eq!(first("main.rs"), "text-rust");
        // the build files and notes are read as code and text (so that their lines are drawn on the icon)
        assert_eq!(kind_of("Cargo.lock", false), Kind::Code);
        assert_eq!(kind_of("Makefile", false), Kind::Code);
        assert_eq!(kind_of("LICENSE", false), Kind::Text);
        assert_eq!(kind_of("a.out", false), Kind::Other);
    }

    #[test]
    fn a_source_file_is_never_a_film() {
        // the MIME database says `.ts` is an MPEG stream; for a file read as code that is TypeScript
        let names = icon_names(&file("app.ts"));
        assert_eq!(names[0], "text-x-typescript");
        assert!(!names.iter().any(|n| n.starts_with("video-")), "{names:?}");
    }
    #[test]
    fn a_folder_is_known_by_what_is_in_it() {
        let dir = std::env::temp_dir().join(format!("memrium-emblem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mk = |sub: &str, files: &[&str]| {
            let d = dir.join(sub);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), "").unwrap();
            }
            d
        };
        assert_eq!(folder_emblem(&mk("rust", &["Cargo.toml", "flake.nix", "README.md"])), Some("folder-rust"), "the language wins over nix");
        assert_eq!(folder_emblem(&mk("nix", &["flake.nix"])), Some("folder-nix"));
        assert_eq!(folder_emblem(&mk("songs", &["a.mp3", "b.flac", "c.ogg", "cover.jpg"])), Some("folder-music"));
        assert_eq!(folder_emblem(&mk("mixed", &["a.mp3", "b.png", "c.txt", "d.zip"])), None);
        assert_eq!(folder_emblem(&mk("few", &["a.png", "b.png"])), None, "two pictures are not a photo folder yet");
        std::fs::create_dir_all(dir.join("repo/.git")).unwrap();
        assert_eq!(folder_emblem(&dir.join("repo")), Some("folder-git"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_are_kept_on_the_file_and_found_again() {
        let dir = std::env::temp_dir().join(format!("memrium-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // (the list of tagged files goes to a scratch data folder)
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        let f = dir.join("note.txt");
        std::fs::write(&f, "x").unwrap();
        if set_tags(&f, &["Red".into(), "Work".into()]).is_err() {
            eprintln!("this file system keeps no user attributes: nothing to test");
            return;
        }
        assert_eq!(tags_of(&f), ["Red", "Work"]);
        let (files, used) = tagged(Some("red"));
        assert_eq!(files, [f.clone()]);
        assert_eq!(used, ["Red", "Work"], "the colour tags first");
        set_tags(&f, &[]).unwrap();
        assert!(tags_of(&f).is_empty());
        assert!(tagged(Some("Red")).0.is_empty());
        assert_eq!(tag_colour("blue"), TAG_COLOURS[4].1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_colour_is_written_to_its_directory_file_and_taken_back() {
        let dir = std::env::temp_dir().join(format!("memrium-folder-colour-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(folder_icon(&dir), None);
        set_folder_icon(&dir, Some("folder-red")).unwrap();
        assert_eq!(folder_icon(&dir).as_deref(), Some("folder-red"));
        // what else Dolphin keeps there stays
        std::fs::write(dir.join(".directory"), "[Dolphin]\nViewMode=1\n\n[Desktop Entry]\nIcon=folder-red\n").unwrap();
        set_folder_icon(&dir, Some("folder-green")).unwrap();
        let text = std::fs::read_to_string(dir.join(".directory")).unwrap();
        assert!(text.contains("ViewMode=1") && text.contains("Icon=folder-green") && !text.contains("folder-red"), "{text}");
        set_folder_icon(&dir, None).unwrap();
        assert_eq!(folder_icon(&dir), None);
        assert!(std::fs::read_to_string(dir.join(".directory")).unwrap().contains("ViewMode=1"));
        // a colour of one's own, with the nearest named one for Dolphin; a named one afterwards replaces it
        set_folder_colour(&dir, [0.93, 0.30, 0.30]).unwrap();
        assert_eq!(folder_icon(&dir).as_deref(), Some("folder-red"));
        assert_eq!(folder_colour(&dir).map(crate::theme::to_hex).as_deref(), Some("#ED4D4D"));
        set_folder_icon(&dir, Some("folder-pink")).unwrap();
        assert_eq!(folder_colour(&dir), None);
        set_folder_icon(&dir, None).unwrap();
        // a file with only the icon in it goes
        std::fs::remove_file(dir.join(".directory")).unwrap();
        set_folder_icon(&dir, Some("folder-teal")).unwrap();
        set_folder_icon(&dir, None).unwrap();
        assert!(!dir.join(".directory").exists());
        // an icon of another kind (Dolphin's `folder-music`) and a colour go together: Dolphin gets the colour, we draw both
        set_folder_icon(&dir, Some("folder-music")).unwrap();
        assert_eq!((folder_shape(&dir).as_deref(), folder_tint(&dir)), (Some("folder-music"), None));
        set_folder_paint(&dir, Some("folder-red")).unwrap();
        assert_eq!(folder_icon(&dir).as_deref(), Some("folder-red"));
        assert_eq!((folder_shape(&dir).as_deref(), folder_tint(&dir)), (Some("folder-music"), Some(FOLDER_COLOURS[0].2)));
        set_folder_colour(&dir, [0.2, 0.3, 0.9]).unwrap();
        assert_eq!(folder_shape(&dir).as_deref(), Some("folder-music"), "a colour of one's own keeps the icon too");
        assert_eq!(folder_tint(&dir).map(crate::theme::to_hex).as_deref(), Some("#334DE6"));
        set_folder_paint(&dir, Some("folder-green")).unwrap();
        assert_eq!((folder_colour(&dir), folder_tint(&dir)), (None, Some(FOLDER_COLOURS[3].2)));
        set_folder_paint(&dir, None).unwrap();
        assert_eq!((folder_icon(&dir).as_deref(), folder_tint(&dir)), (Some("folder-music"), None), "no colour: the icon is back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mark_of_another_colour_keeps_its_colour_when_the_folder_is_recoloured() {
        // a blue folder with an orange mark on it
        let mut img = image::RgbaImage::from_pixel(20, 20, image::Rgba([60, 140, 240, 255]));
        for x in 8..12 {
            for y in 8..12 {
                img.put_pixel(x, y, image::Rgba([240, 120, 30, 255]));
            }
        }
        img.put_pixel(0, 0, image::Rgba([250, 250, 250, 255]));
        let out = recolour_folder(&img, [0.9, 0.2, 0.2]);
        let body = out.get_pixel(2, 15).0;
        assert!(body[0] > 150 && body[2] < 120, "the body turns red: {body:?}");
        assert_eq!(out.get_pixel(10, 10).0, [240, 120, 30, 255], "the mark keeps its colour");
        assert_eq!(out.get_pixel(0, 0).0, [250, 250, 250, 255], "white stays");
    }

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
