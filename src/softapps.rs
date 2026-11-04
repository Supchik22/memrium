//! Programs that come out soft at a fractional scale, and what the shell does about them without being asked.
//!
//! A program that draws for 100% on a 125% desktop is stretched by the compositor and blurs. Like KWin and GNOME, the
//! shell does not leave it at that: a program with a known switch gets it set before it starts (`prepare`), and any other
//! that was seen stretched is remembered (`learn`) and starts on X11 next time, where the desktop's Xft.dpi makes it draw
//! at the screen's own pixels. A program's explicit `x11 = false` rule in app-rules.toml still wins.

use crate::theme::config_dir;
use std::path::PathBuf;

fn path() -> PathBuf {
    config_dir().join("soft-apps.txt")
}

static SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// The desktop's scale, set by the compositor (programs are started from it).
pub fn set_scale(s: f32) {
    SCALE.store(s.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn scale() -> f32 {
    f32::from_bits(SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

fn norm(id: &str) -> String {
    id.trim().trim_end_matches(".desktop").to_lowercase()
}

/// Programs seen stretched, one name per line.
pub fn learned() -> Vec<String> {
    std::fs::read_to_string(path()).map(|t| t.lines().map(norm).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()).unwrap_or_default()
}

pub fn is_soft(names: &[&str]) -> bool {
    let l = learned();
    names.iter().map(|n| norm(n)).any(|n| !n.is_empty() && l.contains(&n))
}

/// Remember a program as soft. True if it was new.
pub fn learn(app_id: &str) -> bool {
    let id = norm(app_id);
    if id.is_empty() || learned().contains(&id) {
        return false;
    }
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut text = std::fs::read_to_string(&p).unwrap_or_else(|_| "# programs the shell saw stretched at a fractional scale: they start on X11 (delete a line to undo)\n".into());
    text.push_str(&id);
    text.push('\n');
    std::fs::write(&p, text).is_ok()
}

/// Programs with a setting of their own for a sharp picture on a HiDPI screen: set it before they start (a JSON key in their config).
const KNOWN: [(&str, &str, &str); 1] = [("sober", ".var/app/org.vinegarhq.Sober/config/sober/config.json", "enable_hidpi")];

/// The program is about to start on a desktop scaled by `scale`: switch on its own HiDPI mode if it has one.
/// Returns true when it has a switch of its own (so it needs no X11 fallback).
pub fn prepare(names: &[&str], scale: f32) -> bool {
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    for (key, rel, option) in KNOWN {
        if !lower.iter().any(|n| n.contains(key)) {
            continue;
        }
        if scale > 1.01 {
            let p = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rel);
            if let Ok(text) = std::fs::read_to_string(&p) {
                let off = format!("\"{option}\": false");
                if text.contains(&off) {
                    let _ = std::fs::write(&p, text.replace(&off, &format!("\"{option}\": true")));
                }
            }
        }
        return true;
    }
    false
}
