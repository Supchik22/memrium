//! Listening to what the computer plays, for wallpapers that dance to it: `pw-record` captures the default output
//! (its monitor) as raw samples, a thread turns them into 64 pitch bands.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::renderer::AUDIO_BANDS;

const RATE: f32 = 44_100.0;
const N: usize = 1024;

/// The spectrum so far: bands 0..1 from low to high pitch, and the levels overall, bass, mids, treble.
#[derive(Clone, Copy)]
pub struct Spectrum {
    pub bands: [f32; AUDIO_BANDS],
    pub levels: [f32; 4],
}

impl Default for Spectrum {
    fn default() -> Self {
        Spectrum { bands: [0.0; AUDIO_BANDS], levels: [0.0; 4] }
    }
}

pub struct AudioTap {
    shared: Arc<Mutex<Spectrum>>,
    stop: Arc<AtomicBool>,
    child: Option<Child>,
}

impl AudioTap {
    /// Start listening. `None` when `pw-record` is not installed (the wallpaper then just sits at silence).
    pub fn start() -> Option<AudioTap> {
        // `MEMRIUM_FAKE_AUDIO=1`: a made-up piece of music instead of the real sound (for demos and screenshots)
        if std::env::var_os("MEMRIUM_FAKE_AUDIO").is_some() {
            return Some(AudioTap::fake());
        }
        let mut child = Command::new("pw-record")
            .args(["-P", "{ stream.capture.sink=true }", "--rate", "44100", "--channels", "1", "--format", "s16", "--latency", "512", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let shared = Arc::new(Mutex::new(Spectrum::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut an = Analyzer::new();
            let mut raw = [0u8; N * 2];
            let mut hop = vec![0f32; N / 2];
            // `pw-record` to a pipe starts with a 24 byte `.snd` header, then plain samples
            let mut head = [0u8; 24];
            if out.read_exact(&mut head).is_err() {
                return;
            }
            while !st.load(Ordering::Relaxed) {
                if out.read_exact(&mut raw[..N]).is_err() {
                    break;
                }
                for (i, c) in raw[..N].chunks_exact(2).enumerate() {
                    hop[i] = i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0;
                }
                an.push(&hop);
                *sh.lock().unwrap() = an.spectrum();
            }
        });
        Some(AudioTap { shared, stop, child: Some(child) })
    }

    fn fake() -> AudioTap {
        let shared = Arc::new(Mutex::new(Spectrum::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let mut bands = [0f32; AUDIO_BANDS];
            while !st.load(Ordering::Relaxed) {
                let t = t0.elapsed().as_secs_f32();
                // a beat at 2 Hz that hits the lows, a melody that wanders over the mids, a shimmer on top
                let beat = (1.0 - (t * 2.0).fract()).powi(3);
                let mel = 0.5 + 0.5 * (t * 0.7).sin();
                for (i, b) in bands.iter_mut().enumerate() {
                    let x = i as f32 / (AUDIO_BANDS - 1) as f32;
                    let low = beat * (1.0 - x * 2.2).max(0.0);
                    let mid = (-(x - (0.3 + 0.4 * mel)).powi(2) * 60.0).exp() * (0.55 + 0.45 * (t * 5.0 + i as f32).sin().abs());
                    let hi = 0.25 * x * (0.5 + 0.5 * (t * 13.0 + i as f32 * 1.7).sin()).abs();
                    let v = (low * 0.95 + mid * 0.8 + hi).clamp(0.0, 1.0);
                    *b = if v > *b { v } else { *b * 0.82 + v * 0.18 };
                }
                let avg = |a: usize, z: usize| bands[a..z].iter().sum::<f32>() / (z - a) as f32;
                *sh.lock().unwrap() = Spectrum { bands, levels: [avg(0, AUDIO_BANDS), avg(0, 14), avg(14, 40), avg(40, AUDIO_BANDS)] };
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        });
        AudioTap { shared, stop, child: None }
    }

    pub fn spectrum(&self) -> Spectrum {
        *self.shared.lock().unwrap()
    }
}

impl Drop for AudioTap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Windowed FFT of the last `N` samples, folded into log-spaced bands with a little smoothing and a slow auto-gain.
struct Analyzer {
    ring: Vec<f32>,
    window: Vec<f32>,
    edges: Vec<usize>,
    bands: [f32; AUDIO_BANDS],
    gain: f32,
}

impl Analyzer {
    fn new() -> Self {
        let window = (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
        // band edges in FFT bins, 40 Hz .. 16 kHz, log spaced and strictly growing
        let (lo, hi) = (40.0f32, 16_000.0f32);
        let mut edges = Vec::with_capacity(AUDIO_BANDS + 1);
        let mut prev = 0usize;
        for b in 0..=AUDIO_BANDS {
            let f = lo * (hi / lo).powf(b as f32 / AUDIO_BANDS as f32);
            let bin = ((f / RATE * N as f32).round() as usize).max(prev + if b == 0 { 1 } else { 0 }).min(N / 2 - 1);
            edges.push(bin.max(prev));
            prev = bin.max(prev);
        }
        Analyzer { ring: vec![0.0; N], window, edges, bands: [0.0; AUDIO_BANDS], gain: 0.02 }
    }

    fn push(&mut self, hop: &[f32]) {
        self.ring.drain(..hop.len());
        self.ring.extend_from_slice(hop);
        let mut re: Vec<f32> = self.ring.iter().zip(&self.window).map(|(a, w)| a * w).collect();
        let mut im = vec![0f32; N];
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..N / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt() / (N as f32 / 4.0)).collect();
        let mut raw = [0f32; AUDIO_BANDS];
        let mut peak = 0f32;
        for b in 0..AUDIO_BANDS {
            let (a, z) = (self.edges[b], self.edges[b + 1].max(self.edges[b] + 1).min(N / 2));
            let m = mag[a.min(N / 2 - 1)..z].iter().cloned().fold(0.0f32, f32::max);
            // loudness is felt on a log scale; high pitches carry less energy, so lift them a little
            let tilt = 1.0 + 3.0 * b as f32 / AUDIO_BANDS as f32;
            raw[b] = m * tilt;
            peak = peak.max(raw[b]);
        }
        // auto-gain follows the loudest recent band: quickly down, slowly back up, never amplifying silence
        self.gain = if peak > self.gain { peak } else { (self.gain * 0.995).max(0.02) };
        for b in 0..AUDIO_BANDS {
            let v = (raw[b] / self.gain).clamp(0.0, 1.0).powf(1.4);
            // fast attack, slower release
            self.bands[b] = if v > self.bands[b] { v } else { self.bands[b] * 0.86 + v * 0.14 };
        }
    }

    fn spectrum(&self) -> Spectrum {
        let avg = |a: usize, z: usize| self.bands[a..z].iter().sum::<f32>() / (z - a) as f32;
        Spectrum { bands: self.bands, levels: [avg(0, AUDIO_BANDS), avg(0, 14), avg(14, 40), avg(40, AUDIO_BANDS)] }
    }
}

/// In-place radix-2 FFT (`re.len()` is a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_the_band_it_belongs_to() {
        let mut an = Analyzer::new();
        for round in 0..40 {
            let hop: Vec<f32> = (0..N / 2).map(|i| (std::f32::consts::TAU * 1000.0 * ((round * N / 2 + i) as f32) / RATE).sin() * 0.5).collect();
            an.push(&hop);
        }
        let s = an.spectrum();
        let top = s.bands.iter().enumerate().fold((0, 0.0), |m, (i, &v)| if v > m.1 { (i, v) } else { m }).0;
        let expect = ((1000.0f32 / 40.0).ln() / (16_000.0f32 / 40.0).ln() * AUDIO_BANDS as f32) as usize;
        assert!((top as i32 - expect as i32).abs() <= 2, "peak in band {top}, expected near {expect}");
        assert!(s.bands[2] < 0.3 && s.bands[60] < 0.3);
    }
}
