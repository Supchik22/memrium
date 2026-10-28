//! Finding wallpaper images on disk and decoding them (off the main thread).

use std::path::{Path, PathBuf};

const EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "avif"];

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| EXTS.contains(&e.to_lowercase().as_str()) && e != "avif").unwrap_or(false)
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect(&p, depth + 1, out);
        } else if is_image(&p) {
            out.push(p);
        }
    }
}

/// Plasma-style wallpaper packs keep several sizes in `contents/images/`; take the largest.
fn pack_image(dir: &Path) -> Option<PathBuf> {
    let imgs = dir.join("contents/images");
    let mut best: Option<(u64, PathBuf)> = None;
    for e in std::fs::read_dir(imgs).ok()?.flatten() {
        let p = e.path();
        if is_image(&p) {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().map_or(true, |(s, _)| size > *s) {
                best = Some((size, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// The wallpaper pack called `name` ("Scarlet Tree" finds `ScarletTree`; case, spaces and dashes do not matter), wherever
/// the packs are installed: (display name, picture).
pub fn named(name: &str) -> Option<(String, PathBuf)> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let want = key(name);
    let home = std::env::var("HOME").unwrap_or_default();
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let bases = xdg.split(':').map(str::to_string).chain(["/run/current-system/sw/share".to_string(), "/usr/share".to_string(), format!("{home}/.local/share")]);
    for base in bases.filter(|b| !b.is_empty()) {
        let Ok(rd) = std::fs::read_dir(Path::new(&base).join("wallpapers")) else { continue };
        for e in rd.flatten() {
            let dir_name = e.file_name().to_string_lossy().to_string();
            if key(&dir_name) == want {
                if let Some(p) = pack_image(&e.path()) {
                    return Some((dir_name, p));
                }
            }
        }
    }
    None
}

/// Candidate wallpapers: (display name, path). Capped so the grid stays manageable.
pub fn scan() -> Vec<(String, PathBuf)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut paths: Vec<PathBuf> = Vec::new();
    // system packs first: <data dir>/wallpapers/<Pack>/contents/images/*
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    for base in xdg.split(':').chain(["/run/current-system/sw/share", "/usr/share"]) {
        if base.is_empty() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(Path::new(base).join("wallpapers")) {
            for e in rd.flatten() {
                if let Some(p) = pack_image(&e.path()) {
                    paths.push(p);
                }
            }
        }
        collect(&Path::new(base).join("backgrounds"), 2, &mut paths);
    }
    // then the user's own
    for d in [format!("{home}/Pictures/Wallpapers"), format!("{home}/.local/share/wallpapers"), format!("{home}/.local/share/backgrounds"), format!("{home}/Pictures")] {
        collect(Path::new(&d), 0, &mut paths);
    }
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .take(40)
        .map(|p| {
            // "Altai/contents/images/5120x2880.png" -> "Altai"
            let name = p.ancestors().find(|a| a.join("contents").exists()).and_then(|a| a.file_name()).or_else(|| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            (name, p)
        })
        .collect()
}

/// Wallpapers drawn by the program itself: `path = "builtin:<name>"` instead of a file.
pub const BUILTIN: &[(&str, &str, [f32; 3], [f32; 3])] = &[("Meadow", "builtin:meadow", [0.45, 0.72, 0.95], [0.40, 0.72, 0.22])];

fn builtin(name: &str, w: u32) -> Option<image::RgbaImage> {
    match name {
        "meadow" => Some(meadow(w, w * 9 / 16)),
        _ => None,
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rolling green hills under a blue sky with a bright sun, clouds and a lake: the look of an Aero-era desktop.
fn meadow(w: u32, h: u32) -> image::RgbaImage {
    let aspect = w as f32 / h as f32;
    let px = 1.0 / h as f32;
    // (base height, amplitudes, colour at the ridge, colour at the bottom)
    let hills: [(f32, [f32; 3], [f32; 3], [f32; 3]); 3] = [
        (0.60, [0.030, 0.018, 0.008], [0.52, 0.78, 0.86], [0.62, 0.84, 0.88]),
        (0.68, [0.048, 0.022, 0.010], [0.50, 0.80, 0.32], [0.34, 0.66, 0.20]),
        (0.80, [0.070, 0.030, 0.012], [0.58, 0.88, 0.26], [0.20, 0.52, 0.10]),
    ];
    let clouds: [(f32, f32, f32, f32); 8] = [(0.28, 0.14, 0.13, 0.035), (0.35, 0.12, 0.10, 0.030), (0.22, 0.12, 0.09, 0.028), (0.62, 0.22, 0.15, 0.040), (0.72, 0.20, 0.11, 0.030), (0.55, 0.24, 0.10, 0.026), (0.90, 0.12, 0.12, 0.030), (0.84, 0.14, 0.09, 0.024)];
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        let v = (y as f32 + 0.5) * px;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let p = (u * aspect, v);
            // sky
            let mut col = mix3([0.14, 0.42, 0.84], [0.80, 0.93, 0.99], (v / 0.68).powf(0.85));
            // sun with a wide glow
            let d = ((p.0 - 0.14 * aspect).powi(2) + (p.1 - 0.17).powi(2)).sqrt();
            let core = smooth(0.082, 0.070, d);
            let glow = (-d * d * 26.0).exp() * 0.55 + (-d * 3.2).exp() * 0.16;
            col = mix3(col, [1.0, 0.94, 0.36], core);
            for i in 0..3 {
                col[i] = (col[i] + [1.0, 0.92, 0.55][i] * glow * (1.0 - core) * 0.55).min(1.0);
            }
            // clouds: soft ellipses
            for &(cx, cy, rx, ry) in &clouds {
                let e = (((u - cx) * aspect) / (rx * 0.9)).powi(2) + ((v - cy) / ry).powi(2);
                col = mix3(col, [1.0, 1.0, 1.0], smooth(1.0, 0.25, e) * 0.92);
            }
            // hills, back to front
            for (i, &(base, a, top, bottom)) in hills.iter().enumerate() {
                let f = [3.1 + i as f32 * 0.7, 7.3 + i as f32, 15.0 + i as f32 * 2.0];
                let ph = i as f32 * 1.9;
                let ridge = base + a[0] * (p.0 * f[0] + ph).sin() + a[1] * (p.0 * f[1] + ph * 1.7).sin() + a[2] * (p.0 * f[2] + ph * 0.6).sin();
                let cover = ((v - ridge) / px + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let c = mix3(top, bottom, (v - ridge) / (1.0 - ridge) * 1.6);
                    col = mix3(col, c, cover);
                }
            }
            // a lake in the lower right, bright where it meets the sky
            let lk = (((u - 0.74) * aspect) / (0.42 * aspect)).powi(2) + ((v - 0.97) / 0.075).powi(2);
            if lk < 1.0 {
                let shore = smooth(1.0, 0.82, lk);
                let water = mix3([0.80, 0.93, 0.98], [0.20, 0.55, 0.85], ((v - 0.90) / 0.10).clamp(0.0, 1.0));
                let glint = 0.10 * (p.0 * 90.0 + (v * 400.0).sin()).sin().abs() * smooth(1.0, 0.6, lk);
                col = mix3(col, [water[0] + glint, water[1] + glint, water[2] + glint], shore);
            }
            // a hair of noise keeps the long gradients from banding
            let n = ((x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)) % 255) as f32 / 255.0 - 0.5;
            let px8 = |c: f32| ((c + n / 255.0).clamp(0.0, 1.0) * 255.0).round() as u8;
            img.put_pixel(x, y, image::Rgba([px8(col[0]), px8(col[1]), px8(col[2]), 255]));
        }
    }
    img
}

/// A small square-ish thumbnail (squashed to 128 x 128; the UI restores the aspect ratio).
pub fn thumbnail(path: &Path) -> Option<image::RgbaImage> {
    if let Some(name) = path.to_str().and_then(|s| s.strip_prefix("builtin:")) {
        let img = builtin(name, 256)?;
        return Some(image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Triangle));
    }
    if crate::video::is_video(path) {
        return crate::video::thumbnail(path);
    }
    let img = image::open(path).ok()?.thumbnail(512, 512).to_rgba8();
    Some(image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Triangle))
}

/// The full wallpaper, downscaled to at most `max_w` wide to keep memory sane.
pub fn load(path: &Path, max_w: u32) -> Option<image::RgbaImage> {
    if let Some(name) = path.to_str().and_then(|s| s.strip_prefix("builtin:")) {
        return builtin(name, max_w.min(2560));
    }
    let img = image::open(path).ok()?;
    let img = if img.width() > max_w { img.resize(max_w, u32::MAX, image::imageops::FilterType::Triangle) } else { img };
    Some(img.to_rgba8())
}

/// A good accent colour for a picture: its main colour, made vivid enough to use for buttons and highlights.
pub fn accent_of(img: &image::RgbaImage) -> [f32; 3] {
    let (w, h) = (img.width(), img.height());
    let step = ((w as usize * h as usize / 20_000) as f64).sqrt().ceil().max(1.0) as u32;
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let p = img.get_pixel(x, y).0;
            let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            let hsv = crate::theme::rgb_to_hsv(c);
            // colourful, not-too-dark pixels decide
            let wgt = hsv[1] * hsv[1] * hsv[2].min(0.9);
            for i in 0..3 {
                sum[i] += c[i] * wgt;
            }
            weight += wgt;
        }
    }
    if weight <= 0.0 {
        return [0.04, 0.52, 1.0];
    }
    let main = [sum[0] / weight, sum[1] / weight, sum[2] / weight];
    let hsv = crate::theme::rgb_to_hsv(main);
    crate::theme::hsv_to_rgb(hsv[0], hsv[1].clamp(0.55, 0.95), hsv[2].clamp(0.70, 0.95))
}

/// Is the top strip of the image dark (so light text is needed over it)?
pub fn is_dark_top(img: &image::RgbaImage) -> bool {
    let (w, h) = (img.width(), img.height());
    let rows = (h / 14).max(1);
    let mut sum = 0.0f64;
    let mut n = 0.0f64;
    for y in 0..rows {
        for x in (0..w).step_by(((w / 96).max(1)) as usize) {
            let p = img.get_pixel(x, y).0;
            sum += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
            n += 1.0;
        }
    }
    sum / n.max(1.0) < 118.0
}
