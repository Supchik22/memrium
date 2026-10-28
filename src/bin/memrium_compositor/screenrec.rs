//! Screen recording: the compositor draws the picked area as it does for a screenshot and pipes the frames to ffmpeg, which
//! encodes an H.264 MP4. The pipe is fed from a worker thread, so a slow encoder drops frames instead of stalling the desktop.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Sink {
    tx: mpsc::SyncSender<Vec<u8>>,
    worker: JoinHandle<Result<(), String>>,
}

pub struct Recording {
    /// the recorded area, logical desktop coordinates
    pub rect: [f32; 4],
    pub path: PathBuf,
    pub started: Instant,
    next: Instant,
    interval: Duration,
    fps: u32,
    /// ffmpeg is started with the first frame, when the picture size is known
    sink: Option<Sink>,
}

impl Recording {
    pub fn new(rect: [f32; 4], path: PathBuf, fps: u32) -> Recording {
        let fps = fps.clamp(5, 120);
        let now = Instant::now();
        Recording { rect, path, started: now, next: now, interval: Duration::from_secs_f32(1.0 / fps as f32), fps, sink: None }
    }

    pub fn due(&self) -> bool {
        Instant::now() >= self.next
    }

    /// Hand over the next frame (RGBA, `w` x `h`).
    pub fn push(&mut self, rgba: Vec<u8>, w: u32, h: u32) -> Result<(), String> {
        // a late frame does not make the next ones come in a burst
        self.next = (self.next + self.interval).max(Instant::now());
        if self.sink.is_none() {
            self.sink = Some(self.spawn(w, h)?);
        }
        if let Some(sink) = &self.sink {
            match sink.tx.try_send(rgba) {
                // the encoder is behind: this frame is dropped (ffmpeg stamps frames by the clock and fills the gap)
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => return Err("ffmpeg stopped".into()),
            }
        }
        Ok(())
    }

    fn spawn(&self, w: u32, h: u32) -> Result<Sink, String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let mut child = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-use_wallclock_as_timestamps", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-video_size"])
            .arg(format!("{w}x{h}"))
            .args(["-i", "-", "-vf", "crop=trunc(iw/2)*2:trunc(ih/2)*2", "-fps_mode", "cfr", "-r"])
            .arg(self.fps.to_string())
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "23", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "ffmpeg is not installed".to_string() } else { format!("ffmpeg: {e}") })?;
        let mut stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
        let worker = std::thread::spawn(move || {
            while let Ok(frame) = rx.recv() {
                if stdin.write_all(&frame).is_err() {
                    break;
                }
            }
            // closing the pipe lets ffmpeg write the end of the file
            drop(stdin);
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ffmpeg failed").to_string())
            }
        });
        Ok(Sink { tx, worker })
    }

    /// End the recording; the file is finished in the background and the result arrives on the returned channel.
    pub fn finish(self) -> mpsc::Receiver<Result<PathBuf, String>> {
        let (done_tx, done_rx) = mpsc::channel();
        let path = self.path;
        let Some(sink) = self.sink else {
            let _ = done_tx.send(Err("nothing was recorded".into()));
            return done_rx;
        };
        std::thread::spawn(move || {
            drop(sink.tx);
            let r = sink.worker.join().unwrap_or_else(|_| Err("the encoder thread died".into()));
            let _ = done_tx.send(r.map(|()| path));
        });
        done_rx
    }
}
