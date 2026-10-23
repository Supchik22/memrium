//! Screenshots: where they are saved and how they are named. Capturing itself lives in the compositor.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotPrefs {
    /// target folder; `~` is the home directory
    pub dir: String,
    /// file name prefix, followed by `_YYYYMMDD_HHMMSS.png`
    pub prefix: String,
    /// short white flash when a shot is taken
    pub flash: bool,
    /// small "saved" notice in the corner
    pub notice: bool,
    /// camera shutter sound
    pub sound: bool,
    /// loudness, 0 .. 2 (full-screen shots use the long shutter, region and window shots a short click)
    pub volume: f32,
}

impl Default for ScreenshotPrefs {
    fn default() -> Self {
        ScreenshotPrefs { dir: "~/Pictures/Screenshots".into(), prefix: "Screenshot".into(), flash: true, notice: true, sound: true, volume: 1.0 }
    }
}

/// Local time as "YYYYMMDD_HHMMSS".
pub fn timestamp() -> String {
    // SAFETY: time/localtime_r are given valid pointers to initialised values
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}{:02}{:02}_{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

impl ScreenshotPrefs {
    pub fn folder(&self) -> PathBuf {
        match self.dir.strip_prefix('~') {
            Some(rest) => {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(rest.trim_start_matches('/'))
            }
            None => PathBuf::from(&self.dir),
        }
    }

    /// Write RGBA pixels as a PNG; returns the new path.
    pub fn save(&self, rgba: Vec<u8>, w: u32, h: u32) -> Result<PathBuf, String> {
        let dir = self.folder();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut path = dir.join(format!("{}_{}.png", self.prefix, timestamp()));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{}_{}_{n}.png", self.prefix, timestamp()));
            n += 1;
        }
        let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad image size")?;
        img.save(&path).map_err(|e| e.to_string())?;
        Ok(path)
    }
}
