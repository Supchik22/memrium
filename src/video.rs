//! Video wallpapers: `ffmpeg` decodes the file to raw RGBA frames at the size of the screen, a thread keeps only the newest
//! one for the renderer. The file loops, plays without sound and stops decoding while it is paused.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const EXTS: [&str; 7] = ["mp4", "mkv", "webm", "mov", "m4v", "avi", "gif"];
const FPS: u32 = 30;

/// Is `ffmpeg` installed? Video wallpapers are an extra: without it they are left out of the picker and nothing else changes.
pub fn available() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("ffmpeg").is_file())))
}

pub fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| EXTS.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

/// Video files the user keeps for the desktop: `~/Videos/Wallpapers` and the wallpapers folder of the config.
pub fn scan() -> Vec<(String, PathBuf)> {
    if !available() {
        return Vec::new();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let mut v = Vec::new();
    for d in [PathBuf::from(&home).join("Videos/Wallpapers"), crate::live::user_dir()] {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if is_video(&p) {
                    let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    v.push((name, p));
                }
            }
        }
    }
    v.sort();
    v
}

/// The size to decode at for a screen of `pw` x `ph` pixels: at most 1920 wide, even numbers.
pub fn decode_size(pw: u32, ph: u32) -> (u32, u32) {
    let w = pw.clamp(2, 1920) & !1;
    let h = ((w as u64 * ph.max(1) as u64 / pw.max(1) as u64) as u32).max(2) & !1;
    (w, h)
}

/// A decoded frame: tightly packed RGBA.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

pub struct VideoWall {
    latest: Arc<Mutex<Option<Frame>>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    child: Child,
    pub path: PathBuf,
}

impl VideoWall {
    /// Start decoding `path` (covering a `w` x `h` screen). Err with a reason when ffmpeg is missing or the file is not a video.
    pub fn start(path: &Path, w: u32, h: u32) -> Result<VideoWall, String> {
        if !available() {
            return Err("ffmpeg is not installed".into());
        }
        let filter = format!("fps={FPS},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-re", "-stream_loop", "-1", "-i"])
            .arg(path)
            .args(["-an", "-sn", "-vf", &filter, "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut out = child.stdout.take().ok_or("ffmpeg gave no output")?;
        let latest = Arc::new(Mutex::new(None));
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (l, p, s) = (latest.clone(), paused.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut buf = vec![0u8; (w * h * 4) as usize];
            while !s.load(Ordering::Relaxed) {
                // not reading makes ffmpeg wait on the pipe, so a paused wallpaper costs next to nothing
                if p.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                *l.lock().unwrap() = Some(Frame { rgba: buf.clone(), w, h });
            }
        });
        Ok(VideoWall { latest, paused, stop, child, path: path.to_path_buf() })
    }

    /// The newest frame not yet taken.
    pub fn take_frame(&self) -> Option<Frame> {
        self.latest.lock().unwrap().take()
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Has the decoder died (a broken file, or ffmpeg killed)?
    pub fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for VideoWall {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A still from the middle-ish of the video, for the picker (squashed to 128 x 128 like the image thumbnails).
pub fn thumbnail(path: &Path) -> Option<image::RgbaImage> {
    if !available() {
        return None;
    }
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-ss", "1", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale=512:-2", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let img = image::load_from_memory(&out.stdout).ok().or_else(|| {
        // a clip shorter than a second has nothing at 1 s: take its first frame
        let out = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-i"]).arg(path).args(["-frames:v", "1", "-vf", "scale=512:-2", "-f", "image2pipe", "-vcodec", "png", "-"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        image::load_from_memory(&out.stdout).ok()
    })?;
    Some(image::imageops::resize(&img.to_rgba8(), 128, 128, image::imageops::FilterType::Triangle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_size_keeps_the_screen_shape_and_stays_even() {
        assert_eq!(decode_size(3840, 2160), (1920, 1080));
        assert_eq!(decode_size(1366, 768), (1366, 768));
        let (w, h) = decode_size(2560, 1080);
        assert!(w % 2 == 0 && h % 2 == 0 && w == 1920);
    }

    #[test]
    fn plays_a_generated_clip() {
        let dir = std::env::temp_dir().join(format!("memrium-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("t.mp4");
        let ok = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=30:duration=1", "-pix_fmt", "yuv420p", "-y"]).arg(&clip).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return; // no ffmpeg here
        }
        let mut v = VideoWall::start(&clip, 320, 180).unwrap();
        let t0 = std::time::Instant::now();
        let mut got = None;
        while t0.elapsed().as_secs() < 5 && got.is_none() {
            got = v.take_frame();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let f = got.expect("a frame");
        assert_eq!((f.w, f.h, f.rgba.len()), (320, 180, 320 * 180 * 4));
        assert!(f.rgba.iter().any(|&b| b != 0));
        assert!(thumbnail(&clip).is_some());
        assert!(!v.is_dead());
        let _ = std::fs::remove_dir_all(dir);
    }
}
