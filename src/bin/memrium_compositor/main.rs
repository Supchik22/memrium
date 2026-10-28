//! Jelly Glass compositor.
//!
//! smithay does the Wayland protocol; pixels are drawn by our wgpu glass renderer. Two backends:
//!   * nested  - a window inside another desktop session (winit); the default, good for development
//!   * session - a real desktop session on a TTY (DRM/KMS + libinput + libseat)   [--session]

mod damage;
mod dock;
mod pointer;
mod fileicons;
mod headless;
mod record;
mod flyout;
mod popup;
mod appmenu_service;
mod appmenu_wayland;
mod gtk_shell;
mod gtkmenu;
mod icon_loader;
mod appstyle;
mod blur;
mod dbusmenu_bridge;
mod tray_service;
mod tray_ui;
mod power_panel;
mod calendar_panel;
mod audio_panel;
mod network_panel;
mod idle;
mod ime;
mod widgets_panel;
mod desk_widgets;
mod media;
mod handlers;
mod nested;
mod panels;
mod session;
mod lock;
mod notify;
mod bluetooth_panel;
mod clipboard;
mod clip_panel;
mod notif_panel;
mod protocols;
mod role;
mod screencopy;
mod ext_capture;
mod screenrec;
mod shot_edit;
mod xwm;
mod title_menu;
mod focus;
mod state;

/// Command line shared by the backends.
pub struct Args {
    /// run this command (with WAYLAND_DISPLAY set) once the socket is up
    pub exec: Option<String>,
    /// debug: save a screenshot of our own frame after N ms and quit
    pub shot: Option<(String, u128)>,
    /// start with the "Edit Controls" screen open
    pub edit: bool,
    pub wallpaper: Option<String>,
    /// debug: open the launcher with this query
    pub launcher: Option<String>,
    /// debug: open Settings at this page (0 = first)
    pub settings: Option<usize>,
    /// debug: open the file manager at this folder
    pub files: Option<String>,
    /// debug: open the desktop context menu
    pub menu: bool,
    /// debug: open the Quick Settings
    pub cc: bool,
    /// debug: measure the average frame time over N offscreen frames, then quit
    pub bench: Option<u32>,
    /// start with the screen locked (to try the lock screen)
    pub lock: bool,
    /// debug: start with this live wallpaper (aurora, liquid-gradient, ocean, starfield or a .wgsl path)
    pub live: Option<String>,
    /// run as a real desktop session on a TTY (DRM/KMS + libinput + libseat)
    pub session: bool,
    /// draw offscreen without any window
    pub headless: bool,
}

const DEFAULT_WALLPAPER: &str = "/nix/store/1n95gvf26ipr5d6vavyjzam7879h8qps-plasma-workspace-wallpapers-6.5.6/share/wallpapers/Altai/contents/images/5120x2880.png";

pub fn wallpaper_path(args: &Args) -> String {
    args.wallpaper.clone().unwrap_or_else(|| DEFAULT_WALLPAPER.to_string())
}

pub fn load_wallpaper(args: &Args) -> Option<image::RgbaImage> {
    let path = args.wallpaper.clone().unwrap_or_else(|| DEFAULT_WALLPAPER.to_string());
    match image::open(&path) {
        Ok(img) => Some(img.to_rgba8()),
        Err(e) => {
            eprintln!("wallpaper {path}: {e} (using procedural)");
            None
        }
    }
}

fn parse_args() -> Args {
    let a: Vec<String> = std::env::args().collect();
    let val = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    Args {
        exec: val("--exec"),
        shot: val("--shot").map(|p| (p, val("--after-ms").and_then(|v| v.parse().ok()).unwrap_or(4000))),
        edit: a.iter().any(|x| x == "--edit"),
        wallpaper: val("--wallpaper"),
        launcher: val("--launcher"),
        live: val("--live"),
        session: a.iter().any(|x| x == "--session"),
        headless: a.iter().any(|x| x == "--headless"),
        files: val("--files"),
        menu: a.iter().any(|x| x == "--menu"),
        cc: a.iter().any(|x| x == "--cc"),
        lock: a.iter().any(|x| x == "--lock"),
        bench: val("--bench").map(|v| v.parse().unwrap_or(60)),
        settings: val("--settings").map(|v| v.parse().ok().or_else(|| memrium_de::settings::Settings::page_by_name(&v)).unwrap_or(0)),
    }
}

fn main() {
    // few allocator arenas: the threads that decode wallpapers and scan icons must not each keep what they freed
    chebrium::mem::tame_allocator();
    let args = parse_args();
    if args.session {
        session::run(args);
    } else if args.headless {
        headless::run(args);
    } else {
        nested::run(args);
    }
}
