//! Maximum-efficiency mode: square windows, no transparency, no blur, no shadows, no animations.
//!
//! It is an overlay on the theme (`Theme::efficient`), not a theme of its own: the user's theme is kept as it was and comes back when
//! the mode is switched off. The flag lives in `~/.config/memrium/efficiency` (the file exists = on) so it survives logins.

use crate::theme::config_dir;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);
static CHANGED: AtomicBool = AtomicBool::new(false);

fn path() -> std::path::PathBuf {
    config_dir().join("efficiency")
}

/// Read the saved flag (once, when the session starts).
pub fn load() {
    ON.store(path().exists(), Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Switch the mode and remember it; the shell notices through `take_changed` and re-applies its theme.
pub fn set(on: bool) {
    if on == enabled() {
        return;
    }
    ON.store(on, Ordering::Relaxed);
    CHANGED.store(true, Ordering::Relaxed);
    let p = path();
    if on {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, b"");
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

/// True once after each `set` that changed the mode.
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::Relaxed)
}
