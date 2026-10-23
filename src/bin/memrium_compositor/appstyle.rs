//! Other programs (GTK, Qt) follow the desktop's look: light or dark, the accent and the icon theme (and the interface
//! font, when it is installed) are written where they read them, as `gsettings` (what the GTK portal serves to
//! libadwaita and browsers), the `settings.ini` of GTK 3 and 4, and KDE's `kdeglobals`. Only in a real session (a nested
//! window must not change the settings of the desktop it runs in), and not while the shell itself follows the GTK theme.

use crate::state::State;
use memrium_de::gtk::{accent_name, config_home, set_ini_key};

#[derive(Clone, Debug, PartialEq)]
pub struct AppStyle {
    pub dark: bool,
    pub accent: &'static str,
    pub icons: Option<String>,
}

fn run(cmd: &str, args: &[&str]) {
    let _ = std::process::Command::new(cmd).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

/// The interface font programs get when it is installed ("Inter 10").
fn font() -> Option<&'static str> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{family}", "Inter"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split(',').any(|f| f.trim() == "Inter").then_some("Inter 10")
}

fn apply(s: &AppStyle) {
    let gs = |key: &str, value: &str| run("gsettings", &["set", "org.gnome.desktop.interface", key, value]);
    gs("color-scheme", if s.dark { "prefer-dark" } else { "default" });
    gs("accent-color", s.accent);
    let font = font();
    if let Some(f) = font {
        gs("font-name", f);
    }
    if let Some(icons) = &s.icons {
        gs("icon-theme", icons);
        run("kwriteconfig6", &["--file", "kdeglobals", "--group", "Icons", "--key", "Theme", icons]);
    }
    for dir in ["gtk-3.0", "gtk-4.0"] {
        let ini = config_home().join(dir).join("settings.ini");
        set_ini_key(&ini, "gtk-application-prefer-dark-theme", if s.dark { "true" } else { "false" });
        if let Some(f) = font {
            set_ini_key(&ini, "gtk-font-name", f);
        }
        if let Some(icons) = &s.icons {
            set_ini_key(&ini, "gtk-icon-theme-name", icons);
        }
        if dir == "gtk-3.0" {
            // the shell shows the menu bar (`gtkmenu.rs`), so GTK3 programs keep it out of their windows
            set_ini_key(&ini, "gtk-shell-shows-menubar", "true");
        }
    }
}

impl State {
    /// Hand the desktop's look to other programs when it changed (called every frame; cheap until it does).
    pub fn sync_app_style(&mut self) {
        if !self.export_env || !self.theme.ui.style_apps || self.theme.ui.follow_gtk {
            self.appstyle_last = None;
            return;
        }
        let want = AppStyle { dark: self.theme.dark, accent: accent_name(self.theme.accent), icons: self.theme.ui.icon_theme.clone() };
        if self.appstyle_last.as_ref() == Some(&want) {
            return;
        }
        self.appstyle_last = Some(want.clone());
        std::thread::Builder::new().name("app-style".into()).spawn(move || apply(&want)).ok();
    }
}
