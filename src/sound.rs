//! Short sound effects, played through whatever the system offers (PipeWire, PulseAudio, ALSA). The sounds are
//! built into the program and written once to the cache directory, because the players want a file.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug)]
pub enum Sound {
    /// whole-screen screenshot
    Shutter,
    /// region or window screenshot
    Click,
    Notify,
    Unlock,
    Error,
    Theme,
    Tick,
}

impl Sound {
    fn file(self) -> (&'static str, &'static [u8]) {
        match self {
            Sound::Shutter => ("shutter.wav", include_bytes!("../assets/sounds/Camera_01.wav")),
            Sound::Click => ("click.wav", include_bytes!("../assets/sounds/Camera_02.wav")),
            Sound::Notify => ("notify.wav", include_bytes!("../assets/sounds/notify.wav")),
            Sound::Unlock => ("unlock.wav", include_bytes!("../assets/sounds/unlock.wav")),
            Sound::Error => ("error.wav", include_bytes!("../assets/sounds/error.wav")),
            Sound::Theme => ("theme.wav", include_bytes!("../assets/sounds/theme.wav")),
            Sound::Tick => ("tick.wav", include_bytes!("../assets/sounds/tick.wav")),
        }
    }
}

fn cache_path(name: &str) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    base.join("memrium").join(name)
}

/// Play without waiting; silently does nothing when no player exists.
pub fn play(sound: Sound, volume: f32) {
    let (name, bytes) = sound.file();
    let path = cache_path(name);
    if std::fs::metadata(&path).map(|m| m.len() as usize != bytes.len()).unwrap_or(true) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&path, bytes).is_err() {
            return;
        }
    }
    let vol = volume.clamp(0.0, 2.0);
    std::thread::spawn(move || {
        let file = path.to_string_lossy().to_string();
        let tries: [(&str, Vec<String>); 3] = [
            ("pw-play", vec![format!("--volume={vol}"), file.clone()]),
            ("paplay", vec![format!("--volume={}", (vol * 65536.0) as u32), file.clone()]),
            ("aplay", vec!["-q".into(), file.clone()]),
        ];
        for (cmd, args) in tries {
            if let Ok(mut c) = Command::new(cmd).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                let _ = c.wait();
                return;
            }
        }
    });
}
