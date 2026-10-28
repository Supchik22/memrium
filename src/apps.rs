//! Installed applications: .desktop entries, a freedesktop icon-theme index, launching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AppEntry {
    /// desktop file id without ".desktop" (e.g. "org.kde.dolphin")
    pub id: String,
    pub name: String,
    pub generic_name: String,
    /// command line with the %-field codes removed
    pub exec: String,
    pub icon: String,
    pub keywords: String,
    pub terminal: bool,
    /// StartupWMClass, used to match windows to apps
    pub wm_class: String,
    /// the Categories= of the desktop file (`Network`, `AudioVideo`, ...)
    pub categories: Vec<String>,
}

/// Id of the virtual entry that opens the shell's Settings.
pub const SETTINGS_ID: &str = "memrium-settings";
/// Id of the virtual entry that opens the shell's file manager.
pub const FILES_ID: &str = "memrium-files";

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    dirs.push(std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(&home).join(".local/share")));
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    dirs.extend(xdg.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    // NixOS / Nix profiles and Flatpak may not be listed in XDG_DATA_DIRS for a bare session
    for extra in [
        "/run/current-system/sw/share".to_string(),
        format!("{home}/.nix-profile/share"),
        format!("/etc/profiles/per-user/{}/share", std::env::var("USER").unwrap_or_default()),
        "/var/lib/flatpak/exports/share".to_string(),
        format!("{home}/.local/share/flatpak/exports/share"),
    ] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

/// Remove field codes like %u %F from an Exec line (keeping a literal %%).
fn clean_exec(exec: &str) -> String {
    let mut out = String::new();
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => out.push('%'),
                Some(_) | None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_desktop(path: &Path, id: &str) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut f: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            // keep unlocalised keys only
            if !k.contains('[') {
                f.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
            }
        }
    }
    if f.get("Type").map(String::as_str) != Some("Application") || f.get("NoDisplay").map(String::as_str) == Some("true") || f.get("Hidden").map(String::as_str) == Some("true") {
        return None;
    }
    Some(AppEntry {
        id: id.to_string(),
        name: f.get("Name")?.clone(),
        generic_name: f.get("GenericName").cloned().unwrap_or_default(),
        exec: clean_exec(f.get("Exec")?),
        icon: f.get("Icon").cloned().unwrap_or_default(),
        keywords: f.get("Keywords").cloned().unwrap_or_default().replace(';', " "),
        terminal: f.get("Terminal").map(String::as_str) == Some("true"),
        wm_class: f.get("StartupWMClass").cloned().unwrap_or_default(),
        categories: f.get("Categories").map(|c| c.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
    })
}

/// One launcher file anywhere on disk (a copy on the desktop, say), by its path.
pub fn load_desktop(path: &Path) -> Option<AppEntry> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    parse_desktop(path, &id)
}

fn scan_dir(dir: &Path, prefix: &str, out: &mut Vec<AppEntry>, seen: &mut std::collections::HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            scan_dir(&path, &format!("{prefix}{name}-"), out, seen);
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            let id = format!("{prefix}{stem}");
            if seen.insert(id.clone()) {
                if let Some(app) = parse_desktop(&path, &id) {
                    out.push(app);
                }
            }
        }
    }
}

/// All launchable applications, earlier data dirs winning; sorted by name. Includes our own terminal.
pub fn scan() -> Vec<AppEntry> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in data_dirs() {
        scan_dir(&d.join("applications"), "", &mut out, &mut seen);
    }
    if let Some(term) = memrium_term() {
        out.retain(|a| a.id != term.id);
        out.push(term);
    }
    if let Some(paint) = memrium_paint() {
        out.retain(|a| a.id != paint.id);
        out.push(paint);
    }
    // the rest of the shell's own programs, found next to the running executable like the two above
    for app in own_apps() {
        out.retain(|a| a.id != app.id);
        out.push(app);
    }
    // the shell's own settings window (handled specially: it is not a process)
    out.push(AppEntry {
        id: SETTINGS_ID.into(),
        name: "Settings".into(),
        generic_name: "Desktop Settings".into(),
        exec: String::new(),
        icon: "preferences-system".into(),
        keywords: "preferences appearance theme wallpaper layout windows 11 7 mac".into(),
        terminal: false,
        wm_class: String::new(),
        categories: vec!["Settings".into()],
    });
    out.push(AppEntry {
        id: FILES_ID.into(),
        name: "Files".into(),
        generic_name: "File Manager".into(),
        exec: String::new(),
        icon: "system-file-manager".into(),
        keywords: "folder browse explorer finder dolphin documents downloads".into(),
        terminal: false,
        wm_class: String::new(),
        categories: vec!["Utility".into(), "FileManager".into()],
    });
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// The programs of the desktop that ship beside it and have no desktop file: (executable, id, name, kind of program,
/// icon, search words, categories). Each is listed when its executable is there.
const OWN: [(&str, &str, &str, &str, &str, &str, &[&str]); 8] = [
    ("memrium_calc", "memrium-calc", "Memrium Calculator", "Calculator", "accessories-calculator", "calc math sum arithmetic numbers", &["Utility", "Calculator"]),
    ("memrium_player", "memrium-player", "Memrium Player", "Media Player", "multimedia-video-player", "video audio music movie mpv film", &["AudioVideo", "Player"]),
    ("memrium_snake", "memrium-snake", "Memrium Snake", "Game", "applications-games", "snake game arcade retro nokia", &["Game", "ArcadeGame"]),
    ("memrium_archive", "memrium-archive", "Memrium Archive", "Archive Manager", "utilities-file-archiver", "zip tar gz unzip extract compress pack archive 7z rar xz zstd bz2", &["Utility", "Archiving", "Compression"]),
    ("memrium_convert", "memrium-convert", "Memrium Converter", "Image Converter", "applications-graphics", "convert picture image jpg jpeg png webp bmp gif tiff ico resize quality format", &["Graphics", "Utility"]),
    ("memrium_monitor", "memrium-monitor", "Memrium Monitor", "System Monitor", "utilities-system-monitor", "processes tasks cpu memory ram kill task manager top htop resources network", &["System", "Monitor"]),
    ("memrium_icons", "memrium-icons", "Memrium Icons", "Icon Composer", "preferences-desktop-icons", "icon icons composer glass vector draw svg shapes editor inkscape design theme", &["Graphics", "Utility"]),
    ("memrium_view", "memrium-view", "Memrium Pictures", "Image Viewer", "image-viewer", "photo image picture gallery view png jpeg", &["Graphics", "Viewer"]),
];

fn own_apps() -> Vec<AppEntry> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) else { return Vec::new() };
    OWN.iter()
        .filter(|(exe, ..)| dir.join(exe).exists())
        .map(|(exe, id, name, generic, icon, keywords, cats)| AppEntry {
            id: (*id).into(),
            name: (*name).into(),
            generic_name: (*generic).into(),
            exec: dir.join(exe).to_string_lossy().to_string(),
            icon: (*icon).into(),
            keywords: (*keywords).into(),
            terminal: false,
            wm_class: (*id).into(),
            categories: cats.iter().map(|c| c.to_string()).collect(),
        })
        .collect()
}

/// Our own bitmap editor, found next to the running executable.
fn memrium_paint() -> Option<AppEntry> {
    let exe = std::env::current_exe().ok()?.parent()?.join("memrium_paint");
    if !exe.exists() {
        return None;
    }
    Some(AppEntry {
        id: "memrium-paint".into(),
        name: "Memrium Paint".into(),
        generic_name: "Image Editor".into(),
        exec: exe.to_string_lossy().to_string(),
        icon: "applications-graphics".into(),
        keywords: "draw drawing picture image bitmap brush mspaint kolourpaint".into(),
        terminal: false,
        wm_class: "memrium-paint".into(),
        categories: vec!["Graphics".into()],
    })
}

/// Our own terminal emulator, found next to the running executable.
fn memrium_term() -> Option<AppEntry> {
    let exe = std::env::current_exe().ok()?.parent()?.join("memrium_term");
    if !exe.exists() {
        return None;
    }
    Some(AppEntry {
        id: "memrium-term".into(),
        name: "Memrium Terminal".into(),
        generic_name: "Terminal Emulator".into(),
        exec: exe.to_string_lossy().to_string(),
        icon: "utilities-terminal".into(),
        keywords: "shell console command line cli".into(),
        terminal: false,
        wm_class: "memrium-term".into(),
        categories: vec!["System".into(), "TerminalEmulator".into()],
    })
}

/// Write a launcher for `app` into `dir` (the desktop), under a name that is free. None when the entry is not a program
/// (the shell's own Settings and Files windows have no command).
pub fn write_shortcut(app: &AppEntry, dir: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if app.exec.is_empty() {
        return Err(std::io::Error::other("this entry has no command to put on the desktop"));
    }
    let stem: String = app.id.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    let mut path = dir.join(format!("{stem}.desktop"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).desktop"));
        n += 1;
    }
    let mut text = format!("[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal={}\n", app.name, app.exec.replace('%', "%%"), app.terminal);
    for (k, v) in [("GenericName", &app.generic_name), ("Icon", &app.icon), ("StartupWMClass", &app.wm_class)] {
        if !v.is_empty() {
            text.push_str(&format!("{k}={v}\n"));
        }
    }
    if !app.categories.is_empty() {
        text.push_str(&format!("Categories={};\n", app.categories.join(";")));
    }
    std::fs::write(&path, text)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// Start an application. `socket` is the Wayland display the child should connect to.
pub fn launch(app: &AppEntry, socket: &str) -> std::io::Result<()> {
    launch_in(app, socket, None)
}

/// Like `launch`, in a given working directory.
pub fn launch_in(app: &AppEntry, socket: &str, dir: Option<&Path>) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("sh");
    let line = if app.terminal {
        match memrium_term() {
            Some(t) => format!("{} -e {}", t.exec, app.exec),
            None => app.exec.clone(),
        }
    } else {
        app.exec.clone()
    };
    // the program's own rule (app-rules.toml): a dark look, an accent, a font, the flag its engine takes
    let rules = crate::apprules::Rules::load();
    let exe = line.split_whitespace().next().and_then(|c| Path::new(c).file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let rule = rules.find(&[app.id.as_str(), app.wm_class.as_str(), exe.as_str()]);
    let mut launch = rule.map(|r| r.launch(&line)).unwrap_or_default();
    // the graphics card: the preset in Settings > Power & Battery, and the program's own rule
    launch.env.extend(crate::gpu::launch_env(rule.and_then(|r| r.gpu)));
    // one pointer for every program (a Flatpak does not inherit the environment: `with_launch` turns these into `--env=`)
    launch.env.extend(crate::cursor_theme::env(&crate::theme::Config::load().input.cursor));
    // the window system: GTK 3 programs cannot draw for a fractional scale on Wayland (soft), on X11 they can
    if rule.and_then(|r| r.x11).unwrap_or_else(|| crate::apprules::gtk3_program(&[app.id.as_str(), app.wm_class.as_str(), exe.as_str()])) {
        launch.env.push(("GDK_BACKEND".into(), "x11".into()));
        launch.env.push(("QT_QPA_PLATFORM".into(), "xcb".into()));
    }
    let (line, rule_env) = with_launch(&line, launch);
    cmd.arg("-c").arg(format!("exec {line}"));
    cmd.env("WAYLAND_DISPLAY", socket).env("XDG_SESSION_TYPE", "wayland").env("MOZ_ENABLE_WAYLAND", "1").env("QT_QPA_PLATFORM", "wayland;xcb").env("GDK_BACKEND", "wayland,x11");
    cmd.envs(rule_env);
    // new session so closing the compositor does not take the apps with it
    cmd.process_group(0);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn()?;
    Ok(())
}

/// A command line with a rule's flags and environment put in. A Flatpak does not pass the environment into its sandbox: it
/// gets `--env=` options instead, and its own flags go before the `@@` marks of the file arguments. Returns the line and the
/// environment left for the process itself.
fn with_launch(line: &str, l: crate::apprules::Launch) -> (String, Vec<(String, String)>) {
    let words: Vec<&str> = line.split_whitespace().collect();
    let flatpak = words.first().is_some_and(|w| w.ends_with("flatpak")) && words.get(1) == Some(&"run");
    let mut out = line.to_string();
    let mut env = l.env.clone();
    if flatpak {
        let opts: Vec<String> = l.env.iter().map(|(k, v)| format!("--env={k}={v}")).collect();
        let at = line.find(" run").map_or(0, |i| i + 4);
        out.insert_str(at, &format!(" {}", opts.join(" ")));
        env.clear();
    }
    if !l.args.is_empty() {
        let extra = l.args.join(" ");
        match out.find(" @@") {
            Some(i) if flatpak => out.insert_str(i, &format!(" {extra}")),
            _ => out = format!("{out} {extra}"),
        }
    }
    (out, env)
}

/// Find the application a window belongs to (by app_id / StartupWMClass / desktop id).
pub fn find_for_app_id<'a>(apps: &'a [AppEntry], app_id: &str) -> Option<&'a AppEntry> {
    if app_id.is_empty() {
        return None;
    }
    let a = app_id.to_lowercase();
    let exe_name = |e: &AppEntry| e.exec.split_whitespace().next().and_then(|c| Path::new(c).file_name()).map(|f| f.to_string_lossy().to_lowercase());
    apps.iter()
        .find(|e| e.id.to_lowercase() == a)
        .or_else(|| apps.iter().find(|e| !e.wm_class.is_empty() && e.wm_class.to_lowercase() == a))
        .or_else(|| apps.iter().find(|e| e.id.to_lowercase().ends_with(&format!(".{a}"))))
        .or_else(|| apps.iter().find(|e| exe_name(e).as_deref() == Some(a.as_str())))
        // Steam games run as `steam_app_<id>` while their launcher is `steam steam://rungameid/<id>`.
        .or_else(|| {
            let id = a.strip_prefix("steam_app_")?;
            apps.iter().find(|e| e.exec.contains(&format!("rungameid/{id}")))
        })
        // Last resort: the window class is the first word of the app name ("Godot" -> "Godot Engine").
        .or_else(|| {
            let class = a.split('_').next().filter(|c| c.len() >= 3)?;
            apps.iter().find(|e| e.name.split_whitespace().next().map(|w| w.to_lowercase()).as_deref() == Some(class))
        })
}

/// Rank apps for a launcher query (case-insensitive; prefix beats substring beats keywords).
pub fn search<'a>(apps: &'a [AppEntry], query: &str, limit: usize) -> Vec<&'a AppEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i32, &AppEntry)> = apps
        .iter()
        .filter_map(|a| {
            let name = a.name.to_lowercase();
            let score = if name == q {
                100
            } else if name.starts_with(&q) {
                80
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                70
            } else if name.contains(&q) {
                50
            } else if a.generic_name.to_lowercase().contains(&q) {
                30
            } else if a.keywords.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q) {
                20
            } else {
                return None;
            };
            Some((score, a))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.len().cmp(&b.1.name.len())));
    scored.into_iter().take(limit).map(|(_, a)| a).collect()
}

// ---------------------------------------------------------------- icons

/// An icon index that can be swapped for a new one while it is in use (the icon theme changed): the readers take a
/// handle to the index of the moment, and `get` is `None` until the first one is built.
#[derive(Default)]
pub struct IconSlot(std::sync::RwLock<Option<std::sync::Arc<IconIndex>>>);

impl IconSlot {
    pub fn get(&self) -> Option<std::sync::Arc<IconIndex>> {
        self.0.read().ok().and_then(|g| g.clone())
    }

    pub fn set(&self, index: IconIndex) {
        self.set_arc(std::sync::Arc::new(index));
    }

    pub fn set_arc(&self, index: std::sync::Arc<IconIndex>) {
        if let Ok(mut g) = self.0.write() {
            *g = Some(index);
        }
    }
}

/// Index of icon files by name, built from the installed icon themes.
pub struct IconIndex {
    files: HashMap<String, Vec<(u32, PathBuf)>>,
    /// the names, sorted (for the icon picker), made when first asked for
    names: std::sync::OnceLock<Vec<String>>,
    /// the folders of the themes, the one that counts most first: a file of an earlier theme beats any of a later one
    order: Vec<PathBuf>,
}

fn icon_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = data_dirs().into_iter().map(|d| d.join("icons")).collect();
    if let Ok(home) = std::env::var("HOME") {
        v.insert(0, PathBuf::from(home).join(".icons"));
    }
    v.push(PathBuf::from("/usr/share/pixmaps"));
    v
}

/// The icon themes installed (a folder with an `index.theme` that lists directories), sorted by name.
pub fn installed_icon_themes() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for base in icon_dirs() {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(index) = std::fs::read_to_string(e.path().join("index.theme")) else { continue };
            if index.lines().any(|l| l.starts_with("Directories=")) && !name.to_lowercase().contains("cursor") && !v.contains(&name) {
                v.push(name);
            }
        }
    }
    v.sort_by_key(|n| n.to_lowercase());
    v
}

/// The icon theme the shell was asked for (`icons.theme` of the look): a name, or "auto" / empty for the desktop's own.
static PREFERRED_THEME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Tell the icon indexes which theme to prefer; call it before building them.
pub fn set_icon_theme(name: &str) {
    if let Ok(mut p) = PREFERRED_THEME.lock() {
        *p = name.to_string();
    }
}

/// The themes `Inherits=` in its `index.theme`.
fn inherits(theme: &str) -> Vec<String> {
    for base in icon_dirs() {
        let Ok(index) = std::fs::read_to_string(base.join(theme).join("index.theme")) else { continue };
        if let Some(l) = index.lines().find_map(|l| l.trim().strip_prefix("Inherits=")) {
            return l.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        }
    }
    Vec::new()
}

/// Icon themes to look in, best first: the one chosen for the shell, the desktop's, what each of them inherits, then
/// hicolor. When none is known, every installed theme is tried.
fn theme_chain() -> Vec<String> {
    let chosen = PREFERRED_THEME.lock().map(|p| p.clone()).unwrap_or_default();
    let mut chain: Vec<String> = Vec::new();
    let mut add = |t: String, chain: &mut Vec<String>| {
        let mut todo = vec![t];
        while let Some(t) = todo.pop() {
            if !t.is_empty() && !chain.contains(&t) {
                todo.extend(inherits(&t).into_iter().rev());
                chain.push(t);
            }
        }
    };
    if !chosen.is_empty() && !chosen.eq_ignore_ascii_case("auto") {
        add(chosen, &mut chain);
    }
    if let Some(t) = current_theme() {
        add(t, &mut chain);
    }
    if !chain.iter().any(|t| t == "hicolor") {
        chain.push("hicolor".to_string());
    }
    if chain.len() == 1 {
        for t in installed_icon_themes() {
            if !chain.contains(&t) {
                chain.push(t);
            }
        }
    }
    chain
}

/// The user's icon theme (KDE or GNOME setting), if discoverable.
fn current_theme() -> Option<String> {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let o = std::process::Command::new(cmd).args(args).output().ok()?;
        let s = String::from_utf8(o.stdout).ok()?.trim().trim_matches('\'').to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    run("kreadconfig6", &["--group", "Icons", "--key", "Theme"]).or_else(|| run("gsettings", &["get", "org.gnome.desktop.interface", "icon-theme"]))
}

fn size_of_path(path: &Path) -> u32 {
    let mut size = 48;
    for comp in path.components() {
        let c = comp.as_os_str().to_string_lossy();
        if c == "scalable" {
            return 512;
        }
        if let Some((a, _)) = c.split_once('x') {
            if let Ok(n) = a.parse::<u32>() {
                size = n;
            }
        } else if let Ok(n) = c.parse::<u32>() {
            if n >= 8 && n <= 1024 {
                size = n;
            }
        }
    }
    size
}

fn walk_icons(dir: &Path, depth: u32, out: &mut HashMap<String, Vec<(u32, PathBuf)>>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            walk_icons(&path, depth + 1, out);
        } else if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
            if ext == "svg" || ext == "png" {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.entry(stem.to_string()).or_default().push((size_of_path(&path), path.clone()));
                }
            }
        }
    }
}

impl IconIndex {
    /// Scan the preferred theme, then hicolor and a few common fallbacks (only "apps"-like folders).
    pub fn build() -> IconIndex {
        Self::build_in(&theme_chain())
    }

    /// The application icons of one icon pack (and what it inherits), for its preview in Settings.
    pub fn build_pack(pack: &str) -> IconIndex {
        let mut themes = vec![pack.to_string()];
        let mut at = 0;
        while at < themes.len() {
            for t in inherits(&themes[at]) {
                if !themes.contains(&t) {
                    themes.push(t);
                }
            }
            at += 1;
        }
        Self::build_in(&themes)
    }

    fn build_in(themes: &[String]) -> IconIndex {
        let mut files: HashMap<String, Vec<(u32, PathBuf)>> = HashMap::new();
        let order: Vec<PathBuf> = themes.iter().flat_map(|t| icon_dirs().into_iter().map(move |b| b.join(t))).collect();
        for base in icon_dirs() {
            if base.ends_with("pixmaps") {
                walk_icons(&base, 5, &mut files);
                continue;
            }
            for t in themes {
                let theme = base.join(t);
                // only application icons: <theme>/<size>/apps, <theme>/apps/<size>, scalable/apps ...
                walk_filtered(&theme, 0, &mut files);
            }
        }
        IconIndex { files, order, names: Default::default() }
    }

    /// The small icons of actions and states ("actions", "status": document-open, edit-copy...) that menus show beside their
    /// entries, from the installed icon themes.
    pub fn build_actions() -> IconIndex {
        let themes = theme_chain();
        let mut files: HashMap<String, Vec<(u32, PathBuf)>> = HashMap::new();
        for base in icon_dirs() {
            if base.ends_with("pixmaps") {
                continue;
            }
            for t in &themes {
                walk_cats(&base.join(t), 0, false, &["actions", "status"], &mut files);
            }
        }
        IconIndex { files, order: Vec::new(), names: Default::default() }
    }

    /// Folder and file-type icons ("places" / "mimetypes") from the installed icon themes.
    pub fn build_mime() -> IconIndex {
        let themes = theme_chain();
        let mut files: HashMap<String, Vec<(u32, PathBuf)>> = HashMap::new();
        for base in icon_dirs() {
            if base.ends_with("pixmaps") {
                continue;
            }
            for t in &themes {
                walk_mime(&base.join(t), 0, false, &mut files);
            }
        }
        IconIndex { files, order: Vec::new(), names: Default::default() }
    }

    /// Every icon name of the index, sorted (the picker for a widget's icon lists them).
    pub fn names(&self) -> &[String] {
        self.names.get_or_init(|| {
            let mut v: Vec<String> = self.files.keys().filter(|k| !k.starts_with('/') && !k.ends_with("-symbolic")).cloned().collect();
            v.sort_unstable();
            v
        })
    }

    /// Best file for an icon name at roughly `px` pixels (an absolute path also works).
    pub fn find(&self, name: &str, px: u32) -> Option<PathBuf> {
        if name.starts_with('/') {
            let p = PathBuf::from(name);
            return p.exists().then_some(p);
        }
        let list = self.files.get(name)?;
        // the earliest theme that has the icon decides (our own theme is not beaten by the SVG of the pack it inherits)
        let rank = |p: &Path| self.order.iter().position(|t| p.starts_with(t)).unwrap_or(usize::MAX);
        let best = list.iter().map(|(_, p)| rank(p)).min()?;
        // then: prefer sizes >= px (smallest such), else the largest; svg counts as "any size"
        list.iter()
            .filter(|(_, p)| rank(p) == best)
            .min_by_key(|(s, p)| {
                let svg = p.extension().and_then(|e| e.to_str()) == Some("svg");
                if svg {
                    0
                } else if *s >= px {
                    s - px + 1
                } else {
                    10_000 + (px - s)
                }
            })
            .map(|(_, p)| p.clone())
    }

    /// Load an icon as an RGBA image of `px` x `px` (premultiplied is not applied; straight alpha).
    pub fn load(&self, name: &str, px: u32) -> Option<image::RgbaImage> {
        let path = self.find(name, px)?;
        load_icon_file(&path, px)
    }
}

fn walk_filtered(dir: &Path, depth: u32, out: &mut HashMap<String, Vec<(u32, PathBuf)>>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // skip categories that never hold application icons
            if matches!(name.as_str(), "actions" | "status" | "devices" | "mimetypes" | "emblems" | "places" | "panel" | "animations" | "stock" | "intl" | "preferences" | "categories") {
                continue;
            }
            walk_filtered(&path, depth + 1, out);
        } else if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
            if ext == "svg" || ext == "png" {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.entry(stem.to_string()).or_default().push((size_of_path(&path), path.clone()));
                }
            }
        }
    }
}

/// Like `walk_mime`, but for the icon folders called one of `cats`.
fn walk_cats(dir: &Path, depth: u32, wanted: bool, cats: &[&str], out: &mut HashMap<String, Vec<(u32, PathBuf)>>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            walk_cats(&path, depth + 1, wanted || cats.contains(&name.as_str()), cats, out);
        } else if wanted {
            if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
                if ext == "svg" || ext == "png" {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        out.entry(stem.to_string()).or_default().push((size_of_path(&path), path.clone()));
                    }
                }
            }
        }
    }
}

fn walk_mime(dir: &Path, depth: u32, wanted: bool, out: &mut HashMap<String, Vec<(u32, PathBuf)>>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            let w = wanted || matches!(name.as_str(), "places" | "mimetypes" | "mimes");
            walk_mime(&path, depth + 1, w, out);
        } else if wanted {
            if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
                if ext == "svg" || ext == "png" {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        out.entry(stem.to_string()).or_default().push((size_of_path(&path), path.clone()));
                    }
                }
            }
        }
    }
}

/// Is the icon file from the plateless theme of a clear look of our icons (`iconrender::LIVE_SUFFIX`)?
pub fn is_live_icon(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_str().is_some_and(|n| n.starts_with(crate::iconrender::THEME_NAME) && n.ends_with(crate::iconrender::LIVE_SUFFIX)))
}

/// Decode a PNG or rasterise an SVG to `px` x `px`.
pub fn load_icon_file(path: &Path, px: u32) -> Option<image::RgbaImage> {
    use resvg::{tiny_skia, usvg};
    if path.extension().and_then(|e| e.to_str()) == Some("svg") {
        let data = std::fs::read(path).ok()?;
        let mut opt = usvg::Options::default();
        opt.resources_dir = path.parent().map(|p| p.to_path_buf());
        let tree = usvg::Tree::from_data(&data, &opt).ok()?;
        let mut pm = tiny_skia::Pixmap::new(px, px)?;
        let size = tree.size();
        let sc = (px as f32 / size.width()).min(px as f32 / size.height());
        let (ox, oy) = ((px as f32 - size.width() * sc) / 2.0, (px as f32 - size.height() * sc) / 2.0);
        resvg::render(&tree, tiny_skia::Transform::from_scale(sc, sc).post_translate(ox, oy), &mut pm.as_mut());
        // tiny-skia is premultiplied: convert to straight alpha for the atlas pipeline
        let mut img = image::RgbaImage::from_raw(px, px, pm.take())?;
        for p in img.pixels_mut() {
            let a = p.0[3] as f32 / 255.0;
            if a > 0.0 {
                for c in 0..3 {
                    p.0[c] = (p.0[c] as f32 / a).min(255.0) as u8;
                }
            }
        }
        Some(img)
    } else {
        let img = image::open(path).ok()?.to_rgba8();
        Some(image::imageops::resize(&img, px, px, image::imageops::FilterType::Lanczos3))
    }
}

/// The Launchpad group an app falls into by its desktop-file categories (the first match in the order of the list wins).
pub fn group_of(app: &AppEntry) -> &'static str {
    const GROUPS: [(&str, &[&str]); 8] = [
        ("Internet", &["Network", "WebBrowser", "Email", "InstantMessaging", "Chat", "FileTransfer", "P2P"]),
        ("Media", &["AudioVideo", "Audio", "Video", "Player", "Music", "Recorder"]),
        ("Graphics", &["Graphics", "Photography", "2DGraphics", "3DGraphics", "RasterGraphics", "VectorGraphics"]),
        ("Games", &["Game"]),
        ("Development", &["Development", "IDE", "Debugger"]),
        ("Office", &["Office", "Education", "Science", "Spreadsheet", "WordProcessor"]),
        ("Settings", &["Settings", "DesktopSettings", "HardwareSettings"]),
        ("Utilities", &["Utility", "System", "Monitor", "TerminalEmulator", "FileManager", "Accessibility"]),
    ];
    for (name, cats) in GROUPS {
        if app.categories.iter().any(|c| cats.contains(&c.as_str())) {
            return name;
        }
    }
    "Other"
}

#[cfg(test)]
mod launch_tests {
    use super::*;
    use crate::apprules::Launch;

    #[test]
    fn a_flatpak_gets_options_and_a_plain_program_gets_the_environment() {
        let l = Launch { env: vec![("CHEBRIUM_DARK".into(), "1".into())], args: vec!["--force-dark-mode".into()] };
        let (line, env) = with_launch("flatpak run --branch=stable com.google.Chrome @@u @@", l.clone());
        assert_eq!(line, "flatpak run --env=CHEBRIUM_DARK=1 --branch=stable com.google.Chrome --force-dark-mode @@u @@");
        assert!(env.is_empty());
        let (line, env) = with_launch("chromium", l);
        assert_eq!(line, "chromium --force-dark-mode");
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn a_theme_that_comes_first_beats_the_svg_of_one_that_comes_later() {
        let mut files = HashMap::new();
        files.insert("app".to_string(), vec![(128, PathBuf::from("/t/own/128x128/apps/app.png")), (512, PathBuf::from("/t/pack/scalable/apps/app.svg"))]);
        let index = IconIndex { files, order: vec![PathBuf::from("/t/own"), PathBuf::from("/t/pack")], names: Default::default() };
        assert_eq!(index.find("app", 128), Some(PathBuf::from("/t/own/128x128/apps/app.png")), "the chosen theme wins");
        // with no order (the action and file-type indexes) the old rule stays: a vector picture beats a bitmap
        let mut files = HashMap::new();
        files.insert("app".to_string(), vec![(128, PathBuf::from("/a/app.png")), (512, PathBuf::from("/b/app.svg"))]);
        let any = IconIndex { files, order: Vec::new(), names: Default::default() };
        assert_eq!(any.find("app", 128), Some(PathBuf::from("/b/app.svg")));
    }
}
