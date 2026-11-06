//! Dictation into any text field (Super+H): the microphone is recorded (PipeWire, `pw-record`) while a glow in the
//! accent colour rises from the bottom of the screen and breathes with the voice; once the speaker stops (or presses the
//! key again) whisper.cpp reads the recording on this computer (nothing leaves it) and the words are pasted into the
//! program that has the keyboard. Esc cancels.
//!
//! The model (whisper "small", 466 MB, many languages) is fetched on first use into
//! `~/.local/share/memrium/models/whisper`; `MEMRIUM_WHISPER` names the `whisper-cli` program when it is not on PATH.

use crate::state::State;
use memrium_de::renderer::{Align, Label, Panel};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MODEL: &str = "ggml-small.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
/// Loudness (0..1 RMS) above which the microphone hears speech.
const VOICE: f32 = 0.035;
/// How long a pause after speech ends the recording, and the longest recording.
const PAUSE: Duration = Duration::from_millis(1400);
const LONGEST: Duration = Duration::from_secs(45);

fn model_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".local/share/memrium/models/whisper").join(MODEL)
}

fn whisper() -> String {
    if let Some(w) = std::env::var("MEMRIUM_WHISPER").ok().filter(|w| !w.is_empty()) {
        return w;
    }
    // a copy kept for Memrium (a Nix store link at ~/.local/share/memrium/whisper-cpp) when it is not on PATH
    let kept = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".local/share/memrium/whisper-cpp/bin/whisper-cli");
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("whisper-cli").exists()));
    if !on_path && kept.exists() {
        return kept.to_string_lossy().into_owned();
    }
    "whisper-cli".into()
}

#[derive(Clone, PartialEq)]
enum Stage {
    Downloading,
    Listening,
    Recognizing,
    /// the words that went in, shown a moment
    Done(String),
    Failed(String),
}

enum Msg {
    Downloaded(Result<(), String>),
    /// the speaker went quiet (or never spoke, or spoke too long)
    Quiet,
    Text(Result<String, String>),
}

pub struct Dictation {
    stage: Stage,
    since: Instant,
    rec: Option<Child>,
    wav: PathBuf,
    /// the microphone's loudness now (f32 bits), and a smoothed copy for drawing
    level: Arc<AtomicU32>,
    shown: f32,
    stop: Arc<AtomicBool>,
    tx: std::sync::mpsc::Sender<Msg>,
    rx: Receiver<Msg>,
    /// how far the glow is in (0..1)
    k: f32,
    /// the language whisper is told to expect (telling it is faster and surer than letting it guess)
    lang: String,
}

impl Dictation {
    fn start_recording(&mut self) {
        let _ = std::fs::remove_file(&self.wav);
        let child = Command::new("pw-record").args(["--rate", "16000", "--channels", "1", "--format", "s16"]).arg(&self.wav).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(c) => self.rec = Some(c),
            Err(e) => {
                self.stage = Stage::Failed(format!("no microphone recorder ({e})"));
                return;
            }
        }
        self.stage = Stage::Listening;
        self.since = Instant::now();
        // the loudness of the last quarter second, and the pause that ends the recording
        let (wav, level, stop, tx) = (self.wav.clone(), self.level.clone(), self.stop.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let (mut spoke, mut last_voice) = (false, Instant::now());
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(80));
                let Ok(mut f) = std::fs::File::open(&wav) else { continue };
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                if len <= 44 + 8000 {
                    continue;
                }
                let mut buf = vec![0u8; 8000];
                if f.seek(SeekFrom::Start(len - 8000)).is_err() || f.read_exact(&mut buf).is_err() {
                    continue;
                }
                let n = buf.len() / 2;
                let sum: f64 = buf.chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as f64 / 32768.0).powi(2)).sum();
                let rms = (sum / n as f64).sqrt() as f32;
                level.store(rms.to_bits(), Ordering::Relaxed);
                if rms > VOICE {
                    spoke = true;
                    last_voice = Instant::now();
                }
                let quiet = (spoke && last_voice.elapsed() > PAUSE) || (!spoke && started.elapsed() > Duration::from_secs(8)) || started.elapsed() > LONGEST;
                if quiet {
                    let _ = tx.send(Msg::Quiet);
                    return;
                }
            }
        });
    }

    /// End the recording and read it.
    fn recognize(&mut self) {
        let lang = self.lang.clone();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.rec.take() {
            // SIGINT lets pw-record write the end of the file
            // SAFETY: a signal to our own child process
            unsafe {
                libc::kill(c.id() as i32, libc::SIGINT);
            }
            let _ = c.wait();
        }
        self.stage = Stage::Recognizing;
        self.since = Instant::now();
        let (wav, tx) = (self.wav.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let len = std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0);
            let secs = len.saturating_sub(44) as f32 / 32000.0;
            if secs < 0.3 {
                let _ = tx.send(Msg::Text(Ok(String::new())));
                return;
            }
            // whisper looks at 30 s at a time; a short phrase needs only as much context as it is long (much faster)
            let ctx = (((secs + 1.0) * 50.0).ceil() as u32).clamp(128, 1500);
            let t0 = Instant::now();
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 10);
            let out = Command::new(whisper())
                .args(["-m"])
                .arg(model_path())
                .args(["-l", &lang, "-nt", "-np", "-t", &threads.to_string(), "-ac", &ctx.to_string(), "-f"])
                .arg(&wav)
                .stdin(Stdio::null())
                .output();
            let r = match out {
                Ok(o) if o.status.success() => Ok(clean(&String::from_utf8_lossy(&o.stdout))),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("whisper failed").to_string()),
                Err(e) => Err(format!("whisper-cli is not installed ({e})")),
            };
            eprintln!("dictation: {secs:.1} s of speech read in {:.1} s: {:?}", t0.elapsed().as_secs_f32(), r);
            let _ = std::fs::remove_file(&wav);
            let _ = tx.send(Msg::Text(r));
        });
    }
}

/// The words, without the marks whisper puts for silence or music.
fn clean(s: &str) -> String {
    let mut t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    for mark in ["[BLANK_AUDIO]", "[MUSIC]", "(music)", "(Музика)", "(музика)", "[Музика]"] {
        t = t.replace(mark, "");
    }
    t.trim().to_string()
}

impl State {
    /// Super+H: start listening; pressed while listening, stop and write what was said.
    pub fn toggle_dictation(&mut self) {
        if let Some(d) = self.voice.as_mut() {
            if d.stage == Stage::Listening {
                d.recognize();
            } else if !matches!(d.stage, Stage::Recognizing | Stage::Downloading) {
                self.voice = None;
            }
            return;
        }
        let (tx, rx) = channel();
        let wav = std::env::temp_dir().join(format!("memrium-dictation-{}.wav", std::process::id()));
        // the language: the one the keyboard is set up for besides English (Ukrainian here), else English
        let layouts = self.config.input.xkb_layout.to_lowercase();
        let lang = std::env::var("MEMRIUM_DICTATE_LANG").ok().unwrap_or_else(|| {
            [("ua", "uk"), ("ru", "ru"), ("pl", "pl"), ("de", "de"), ("fr", "fr"), ("es", "es"), ("it", "it"), ("cz", "cs")].iter().find(|(k, _)| layouts.split(',').any(|l| l.trim() == *k)).map_or("en", |(_, l)| l).to_string()
        });
        let mut d = Dictation { stage: Stage::Listening, since: Instant::now(), rec: None, wav, level: Arc::new(AtomicU32::new(0)), shown: 0.0, stop: Arc::new(AtomicBool::new(false)), tx, rx, k: 0.0, lang };
        if let Some(test) = std::env::var_os("MEMRIUM_TEST_DICTATE_WAV") {
            // test hook: a recording from a file instead of the microphone
            let _ = std::fs::copy(test, &d.wav);
            d.recognize();
        } else if model_path().exists() {
            d.start_recording();
        } else {
            d.stage = Stage::Downloading;
            let tx = d.tx.clone();
            std::thread::spawn(move || {
                let path = model_path();
                let r = (|| -> Result<(), String> {
                    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
                    let part = path.with_extension("part");
                    let ok = Command::new("curl").args(["-sL", "--fail", "-o"]).arg(&part).arg(MODEL_URL).status().map_err(|e| e.to_string())?.success();
                    if !ok {
                        return Err("the download failed".into());
                    }
                    std::fs::rename(&part, &path).map_err(|e| e.to_string())
                })();
                let _ = tx.send(Msg::Downloaded(r));
            });
        }
        self.voice = Some(d);
    }

    /// Esc while listening: stop and throw the recording away.
    pub fn cancel_dictation(&mut self) -> bool {
        let Some(d) = self.voice.as_mut() else { return false };
        d.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = d.rec.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&d.wav);
        self.voice = None;
        true
    }

    pub fn voice_tick(&mut self, dt: f32) {
        let Some(d) = self.voice.as_mut() else { return };
        let mut text = None;
        while let Ok(m) = d.rx.try_recv() {
            match m {
                Msg::Downloaded(Ok(())) => d.start_recording(),
                Msg::Downloaded(Err(e)) => d.stage = Stage::Failed(format!("could not fetch the speech model: {e}")),
                Msg::Quiet if d.stage == Stage::Listening => d.recognize(),
                Msg::Quiet => {}
                Msg::Text(Ok(t)) if t.is_empty() => {
                    d.stage = Stage::Failed("didn't catch that".into());
                    d.since = Instant::now();
                }
                Msg::Text(Ok(t)) => {
                    d.stage = Stage::Done(t.clone());
                    d.since = Instant::now();
                    text = Some(t);
                }
                Msg::Text(Err(e)) => {
                    d.stage = Stage::Failed(e);
                    d.since = Instant::now();
                }
            }
        }
        let lv = f32::from_bits(d.level.load(Ordering::Relaxed));
        let target = if d.stage == Stage::Listening { (lv / 0.12).clamp(0.0, 1.0) } else { 0.0 };
        d.shown += (target - d.shown) * (dt * 12.0).min(1.0);
        let leaving = matches!(d.stage, Stage::Done(_) | Stage::Failed(_)) && d.since.elapsed() > Duration::from_millis(1600);
        d.k += ((if leaving { 0.0 } else { 1.0 }) - d.k) * (dt * 8.0).min(1.0);
        if leaving && d.k < 0.02 {
            self.voice = None;
        }
        if let Some(t) = text {
            // into the program that has the keyboard: through the clipboard, as a paste
            self.clip_type_text(t);
        }
    }

    /// The glow at the bottom of the main monitor and what it is doing.
    pub fn voice_panels(&mut self, shell: &mut Vec<Panel>) {
        let Some(d) = &self.voice else { return };
        let o = self.mon_origin(self.main_mon);
        let [mw, mh] = self.size;
        let (bottom, cx) = (o[1] + mh, o[0] + mw / 2.0);
        let accent = self.theme.accent;
        let t = d.since.elapsed().as_secs_f32();
        let breathe = 0.5 + 0.5 * (t * 2.6).sin();
        let lift = match d.stage {
            Stage::Listening => 0.55 + 0.25 * breathe + 0.9 * d.shown,
            Stage::Recognizing | Stage::Downloading => 0.45 + 0.20 * (0.5 + 0.5 * (t * 6.0).sin()),
            _ => 0.4,
        } * d.k;
        // the glow: wide soft layers of the accent, taller and fainter outwards, so they read as one gradient
        for i in 0..20 {
            let f = i as f32 / 19.0;
            let h = (70.0 + 300.0 * f) * lift + 10.0;
            let w = mw * (0.55 + 0.5 * f);
            let warm = [accent[0] * (1.0 - 0.3 * f) + 0.3 * f, accent[1] * (1.0 - 0.3 * f) + 0.15 * f, accent[2] * (1.0 - 0.3 * f) + 0.35 * f];
            shell.push(Panel::solid(cx, bottom, w, h * 2.0, h, warm, warm).flat().alpha(0.065 * (1.0 - 0.55 * f) * d.k));
        }
        shell.push(Panel::solid(cx, bottom, mw * 0.9, 8.0 + 10.0 * lift, 6.0, [1.0; 3], accent).flat().alpha(0.55 * d.k));
        // what it is doing
        let (icon, text) = match &d.stage {
            Stage::Downloading => ("audio-lines", "Fetching the speech model (466 MB, once)…".to_string()),
            Stage::Listening => ("mic", "Listening…   Esc cancels".to_string()),
            Stage::Recognizing => ("audio-lines", "Writing it down…".to_string()),
            Stage::Done(s) => ("mic", if s.chars().count() > 60 { format!("“{}…”", s.chars().take(60).collect::<String>()) } else { format!("“{s}”") }),
            Stage::Failed(e) => ("mic", e.clone()),
        };
        let w = self.renderer.text_width(&text, 13.0, true) + 64.0;
        // above the Dock (or a taskbar)
        let py = bottom - 150.0 - 30.0 * lift;
        let mut g = self.theme.glass.panel(cx, py, w, 40.0, 20.0).tinted([0.08, 0.08, 0.10, 0.70 * d.k]);
        g.fade_by(d.k);
        shell.push(g);
        shell.push(Panel::glyph(cx - w / 2.0 + 24.0, py, 16.0, icon, [1.0; 3], d.k));
        let labels = vec![Label::new(text, cx - w / 2.0 + 42.0, py + 4.8, 13.0, true, [1.0; 3], d.k, Align::Left)];
        shell.extend(self.renderer.label_panels(&labels));
    }
}
