//! Graphics cards of a hybrid laptop: which there are, which one a program is started on, and the preset that decides it.
//!
//! The shell itself always stays on the integrated card (it drives the screen); a program started on the dedicated one
//! draws there (PRIME render offload) and the compositor imports what it drew as a DMA-BUF.
//!
//! `~/.config/memrium/gpu.toml` holds the preset: `mode = "auto" | "dedicated" | "integrated" | "plugged"`. A program's own
//! `gpu = true|false` in app-rules.toml (see apprules.rs) beats the preset, except under `integrated`.

use crate::theme::config_dir;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// the integrated card, unless a program's rule asks for the dedicated one
    Auto,
    /// every program started from the shell, unless its rule says no
    Dedicated,
    /// never the dedicated card (it can power down: the battery lasts)
    Integrated,
    /// like Dedicated on mains power, like Auto on battery (the default: nothing wakes the dedicated card by itself on battery)
    #[default]
    Plugged,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Auto, Mode::Dedicated, Mode::Integrated, Mode::Plugged];

    pub fn key(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Dedicated => "dedicated",
            Mode::Integrated => "integrated",
            Mode::Plugged => "plugged",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Auto => "Auto",
            Mode::Dedicated => "Dedicated",
            Mode::Integrated => "Integrated",
            Mode::Plugged => "Plugged in",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Mode::Auto => "Programs use the integrated card; those with their own rule (Settings > Programs) use the dedicated one.",
            Mode::Dedicated => "Every program started from the shell uses the dedicated card. Fastest, and the battery pays for it.",
            Mode::Integrated => "The dedicated card is never used, so it can power down. Longest battery life.",
            Mode::Plugged => "Dedicated card on mains power, Auto on battery.",
        }
    }

    fn from_key(k: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == k)
    }
}

fn path() -> PathBuf {
    config_dir().join("gpu.toml")
}

pub fn load_mode() -> Mode {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("mode").and_then(|v| v.as_str()).and_then(Mode::from_key))
        .unwrap_or_default()
}

pub fn save_mode(m: Mode) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, format!("# written by the shell (Settings > Power & Battery)\nmode = \"{}\"\n", m.key()));
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub driver: String,
    /// sits on the processor's own PCI bus (bus 00): the integrated one
    pub integrated: bool,
    /// `active`, `suspended`...: whether the card is powered down now (runtime power management)
    pub power: Option<String>,
}

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The cards with a screen driver, from sysfs.
pub fn detect() -> Vec<Gpu> {
    let mut cards: Vec<_> = std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit())).collect();
    cards.sort();
    let mut out = Vec::new();
    for c in cards {
        let dev = Path::new("/sys/class/drm").join(&c).join("device");
        let uevent = read(dev.join("uevent"));
        let field = |k: &str| uevent.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
        let (slot, driver) = (field("PCI_SLOT_NAME"), field("DRIVER"));
        if slot.is_empty() {
            continue;
        }
        let vendor = read(dev.join("vendor"));
        let name = match vendor.as_str() {
            "0x10de" => nvidia_model().unwrap_or_else(|| "NVIDIA graphics".into()),
            "0x8086" => "Intel graphics".into(),
            "0x1002" => "AMD graphics".into(),
            _ => format!("Graphics ({driver})"),
        };
        let power = Some(read(dev.join("power/runtime_status"))).filter(|s| !s.is_empty());
        out.push(Gpu { name, driver, integrated: slot.split(':').nth(1) == Some("00"), power });
    }
    out
}

fn nvidia_model() -> Option<String> {
    let dir = std::fs::read_dir("/proc/driver/nvidia/gpus").ok()?.flatten().next()?.path();
    read(dir.join("information")).lines().find_map(|l| l.strip_prefix("Model:").map(|m| m.trim().to_string()))
}

/// Is there a dedicated card next to an integrated one?
pub fn hybrid(gpus: &[Gpu]) -> bool {
    gpus.iter().any(|g| g.integrated) && gpus.iter().any(|g| !g.integrated)
}

/// Is the machine on mains power? (A desktop, with no battery, always is.)
pub fn on_ac() -> bool {
    let mut any = false;
    for e in std::fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten() {
        let p = e.path();
        if matches!(read(p.join("type")).as_str(), "Mains" | "USB" | "USB_C" | "USB_PD") {
            any = true;
            if read(p.join("online")) == "1" {
                return true;
            }
        }
    }
    !any
}

/// Should a program whose rule says `rule` start on the dedicated card under `mode`?
pub fn wants_discrete(mode: Mode, rule: Option<bool>, ac: bool) -> bool {
    match mode {
        Mode::Integrated => false,
        Mode::Dedicated => rule != Some(false),
        Mode::Plugged if ac => rule != Some(false),
        Mode::Auto | Mode::Plugged => rule == Some(true),
    }
}

/// The environment that makes a program render on the dedicated GPU. With the proprietary NVIDIA driver that is PRIME
/// render offload (GLX, EGL and Vulkan each have their switch); without it, Mesa's `DRI_PRIME` picks the other card.
pub fn discrete_env() -> Vec<(String, String)> {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    let nvidia = Path::new("/proc/driver/nvidia").exists() || Path::new("/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.x86_64.json").exists();
    if !nvidia {
        return vec![s("DRI_PRIME", "1")];
    }
    let mut env = vec![s("__NV_PRIME_RENDER_OFFLOAD", "1"), s("__NV_PRIME_RENDER_OFFLOAD_PROVIDER", "NVIDIA-G0"), s("__GLX_VENDOR_LIBRARY_NAME", "nvidia"), s("__VK_LAYER_NV_optimus", "NVIDIA_only"), s("DXVK_FILTER_DEVICE_NAME", "NVIDIA")];
    // glvnd finds the EGL vendor by this file; it is not always on the default search path (NixOS)
    for dir in ["/run/opengl-driver/share/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"] {
        let f = format!("{dir}/10_nvidia.json");
        if Path::new(&f).exists() {
            env.push(("__EGL_VENDOR_LIBRARY_FILENAMES".into(), f));
            break;
        }
    }
    env
}

/// The extra environment for a program started now whose rule says `rule`: empty on a machine with one card.
pub fn launch_env(rule: Option<bool>) -> Vec<(String, String)> {
    if !hybrid(&detect()) || !wants_discrete(load_mode(), rule, on_ac()) {
        return Vec::new();
    }
    discrete_env()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_decide_with_the_rule() {
        use Mode::*;
        // (mode, rule, on mains) -> dedicated?
        for (m, r, ac, want) in [
            (Auto, None, true, false),
            (Auto, Some(true), false, true),
            (Dedicated, None, false, true),
            (Dedicated, Some(false), true, false),
            (Integrated, Some(true), true, false),
            (Plugged, None, true, true),
            (Plugged, None, false, false),
            (Plugged, Some(true), false, true),
            (Plugged, Some(false), true, false),
        ] {
            assert_eq!(wants_discrete(m, r, ac), want, "{m:?} {r:?} ac={ac}");
        }
    }

    #[test]
    fn mode_keys_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_key(m.key()), Some(m));
        }
    }
}
