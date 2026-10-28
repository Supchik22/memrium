//! Live (animated) wallpapers: WGSL snippets defining
//!
//! ```text
//! fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32>
//! ```
//!
//! `uv` runs 0..1 (y down), `t` is seconds since start. Built-ins below; users can drop their own
//! `.wgsl` files into `~/.config/memrium/wallpapers/`. Helper functions should carry a unique prefix.
//!
//! A shader that calls `lw_band(x)` (loudness 0..1 at pitch x, 0 = bass, 1 = treble) or `lw_level(i)` (0 overall, 1 bass,
//! 2 mids, 3 treble) dances to whatever the computer is playing; the compositor listens to the sound only while such a
//! wallpaper is on.

use std::path::PathBuf;

const AURORA: &str = r#"
fn lw_hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
fn lw_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(lw_hash(i), lw_hash(i + vec2(1.0, 0.0)), u.x), mix(lw_hash(i + vec2(0.0, 1.0)), lw_hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
fn lw_fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * lw_noise(q);
        q = q * 2.0 + vec2(1.7, 9.2);
        a = a * 0.5;
    }
    return v;
}
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let p = vec2(uv.x * aspect, uv.y);
    var col = mix(vec3(0.01, 0.03, 0.10), vec3(0.03, 0.11, 0.24), uv.y);
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let w = lw_fbm(vec2(p.x * 1.6 + fi * 3.1, t * 0.12 + fi));
        let centre = 0.30 + 0.09 * fi + 0.12 * sin(p.x * 2.0 + t * 0.3 + fi * 2.0) + 0.16 * (w - 0.5);
        let band = smoothstep(0.20, 0.0, abs(uv.y - centre));
        let c = mix(vec3(0.10, 0.95, 0.60), vec3(0.50, 0.30, 0.95), clamp(fi / 2.0 + 0.3 * sin(t * 0.2 + p.x), 0.0, 1.0));
        col = col + c * band * (0.55 - 0.12 * fi);
    }
    let sp = floor(p * 190.0);
    let star = step(0.9965, lw_hash(sp)) * (0.5 + 0.5 * sin(t * 2.0 + lw_hash(sp) * 30.0));
    col = col + vec3(star) * smoothstep(0.75, 0.2, uv.y);
    return col;
}
"#;

const LIQUID_GRADIENT: &str = r#"
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let p = vec2(uv.x * aspect, uv.y);
    var col = vec3(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        let c = vec2(aspect * (0.5 + 0.42 * sin(t * (0.21 + 0.07 * fi) + fi * 1.7)), 0.5 + 0.38 * cos(t * (0.17 + 0.05 * fi) + fi * 2.3));
        let d = length(p - c);
        let w = exp(-d * d * 4.2);
        let hue = fi / 5.0 + 0.1 * sin(t * 0.1);
        let tint = 0.5 + 0.5 * cos(6.2831 * (vec3(hue) + vec3(0.0, 0.33, 0.67)));
        col = col + tint * w;
        wsum = wsum + w;
    }
    col = col / max(wsum, 0.0001);
    let glow = smoothstep(1.2, 0.0, length(p - vec2(aspect * 0.5, 0.5)));
    return mix(col * 0.75, col * 1.05, glow);
}
"#;

const OCEAN: &str = r#"
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let p = vec2(uv.x * aspect, uv.y);
    let horizon = 0.42;
    if (uv.y < horizon) {
        let k = uv.y / horizon;
        var sky = mix(vec3(0.35, 0.65, 0.95), vec3(0.85, 0.93, 1.0), k * k);
        let sun = smoothstep(0.35, 0.0, length(vec2(p.x - aspect * 0.68, uv.y - horizon + 0.1)));
        sky = sky + vec3(1.0, 0.85, 0.55) * sun * 0.7;
        return sky;
    }
    let d = (uv.y - horizon) / (1.0 - horizon);
    let persp = 1.0 / (0.15 + d);
    var h = 0.0;
    h = h + sin(p.x * 14.0 * persp * 0.25 + t * 0.9 + sin(p.y * 40.0 * persp * 0.05 + t * 0.4) * 2.0);
    h = h + 0.5 * sin(p.x * 31.0 * persp * 0.2 - t * 1.3 + p.y * 9.0 * persp * 0.1);
    h = h * 0.5 + 0.5;
    var col = mix(vec3(0.02, 0.22, 0.42), vec3(0.08, 0.50, 0.72), h * (1.0 - 0.5 * d));
    let glint = smoothstep(0.85, 1.0, h) * exp(-abs(p.x - aspect * 0.68) * 1.6) * (1.0 - d);
    col = col + vec3(1.0, 0.9, 0.7) * glint * 0.8;
    col = mix(vec3(0.75, 0.88, 0.98), col, smoothstep(0.0, 0.18, d));
    return col;
}
"#;

const STARFIELD: &str = r#"
fn lw_h(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let p = vec2(uv.x * aspect, uv.y);
    var col = mix(vec3(0.005, 0.01, 0.04), vec3(0.03, 0.04, 0.13), uv.y);
    let neb = 0.5 + 0.5 * sin(p.x * 3.0 + sin(p.y * 4.0 + t * 0.05) * 2.0 + t * 0.04);
    col = col + vec3(0.10, 0.04, 0.20) * neb * smoothstep(0.2, 0.9, uv.y) * 0.6;
    for (var layer = 0; layer < 3; layer = layer + 1) {
        let fl = f32(layer);
        let scale = 70.0 + 55.0 * fl;
        let q = p * scale + vec2(t * (0.8 + 0.7 * fl), 0.0);
        let cell = floor(q);
        let f = fract(q) - vec2(0.5);
        let r = lw_h(cell + fl * 17.0);
        let size = 0.05 + 0.12 * lw_h(cell + 3.0);
        let d = length(f - (vec2(lw_h(cell + 7.0), lw_h(cell + 11.0)) - vec2(0.5)) * 0.6);
        let s = smoothstep(size, 0.0, d) * step(0.78, r) * (0.55 + 0.45 * sin(t * 1.5 + r * 60.0));
        col = col + vec3(0.8, 0.9, 1.0) * s * (1.0 - 0.25 * fl);
    }
    return col;
}
"#;


const SPECTRUM: &str = r#"
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let bass = lw_level(1);
    var col = mix(vec3(0.02, 0.01, 0.07), vec3(0.07, 0.03, 0.16), uv.y);
    col = col + vec3(0.25, 0.08, 0.35) * bass * 0.35 * smoothstep(1.0, 0.3, length(uv - vec2(0.5, 0.62)));
    let n = 48.0;
    let x = uv.x * n;
    let i = floor(x);
    let cell = fract(x);
    // the idle sway keeps the bars alive in silence
    let idle = 0.025 + 0.02 * sin(t * 1.3 + i * 0.7);
    let h = 0.04 + 0.52 * lw_band((i + 0.5) / n) + idle;
    let base = 0.80;
    let y = base - uv.y;
    let w = smoothstep(0.0, 0.14, cell) * smoothstep(1.0, 0.86, cell);
    let bar = w * step(0.0, y) * step(y, h);
    let k = clamp(y / max(h, 0.001), 0.0, 1.0);
    let hue = (i / n) * 0.8 + 0.55 + 0.05 * sin(t * 0.3);
    let tint = 0.5 + 0.5 * cos(6.2831 * (vec3(hue) + vec3(0.0, 0.33, 0.67)));
    col = mix(col, mix(tint * 0.55, tint * 1.15, k), bar);
    // glow above the bars and a faded mirror below the floor
    col = col + tint * w * exp(-max(y - h, 0.0) * 18.0) * step(0.0, y) * 0.22 * (0.4 + lw_band((i + 0.5) / n));
    let m = -y;
    let mh = h * 0.45;
    let mirror = w * step(0.0, m) * step(m, mh) * (1.0 - m / max(mh, 0.001));
    col = col + tint * mirror * 0.28;
    return col;
}
"#;

const PULSE: &str = r#"
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    let p = vec2((uv.x - 0.5) * aspect, uv.y - 0.5);
    let r = length(p);
    let a = atan2(p.y, p.x);
    let bass = lw_level(1);
    var col = mix(vec3(0.01, 0.02, 0.06), vec3(0.04, 0.02, 0.12), smoothstep(0.0, 0.9, r));
    // the ring breathes with the bass; its rim is made of the spectrum, mirrored left and right
    let side = abs(a) / 3.14159;
    let band = lw_band(side);
    let radius = 0.22 + 0.04 * bass + 0.012 * sin(t * 0.8);
    let edge = radius + 0.20 * band * band + 0.01;
    let ring = smoothstep(0.006, 0.0, abs(r - radius));
    let spikes = smoothstep(0.004, 0.0, r - edge) * step(radius, r) * smoothstep(0.012, 0.0, abs(fract(side * 64.0) - 0.5) - 0.28);
    let hue = side * 0.7 + 0.55 + 0.04 * t;
    let tint = 0.5 + 0.5 * cos(6.2831 * (vec3(hue) + vec3(0.0, 0.33, 0.67)));
    col = col + tint * ring * 0.9;
    col = col + tint * spikes * 1.1;
    col = col + tint * exp(-abs(r - radius) * 16.0) * (0.10 + 0.5 * bass);
    col = col + vec3(1.0, 0.8, 0.9) * exp(-r * r * 90.0) * (0.15 + 0.7 * lw_level(0));
    // sparks thrown out by the treble
    let sp = floor((p + vec2(t * 0.02, 0.0)) * 90.0);
    let tw = step(0.992, fract(sin(dot(sp, vec2(12.9898, 78.233))) * 43758.5453));
    col = col + vec3(0.7, 0.8, 1.0) * tw * (0.2 + lw_level(3) * 2.0) * smoothstep(0.25, 0.7, r);
    return col;
}
"#;

/// WGSL for a built-in live wallpaper id.
pub fn builtin(id: &str) -> Option<&'static str> {
    Some(match id {
        "aurora" => AURORA,
        "liquid-gradient" => LIQUID_GRADIENT,
        "ocean" => OCEAN,
        "starfield" => STARFIELD,
        "spectrum" => SPECTRUM,
        "pulse" => PULSE,
        _ => return None,
    })
}

/// Is `id` one of the built-in shaders (a user's file path is not)?
pub fn is_builtin(id: &str) -> bool {
    builtin(id).is_some()
}

pub fn user_dir() -> PathBuf {
    crate::theme::config_dir().join("wallpapers")
}

/// User-provided live wallpapers: (name, path).
pub fn user_items() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(user_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("wgsl") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    v.push((stem.to_string(), p));
                }
            }
        }
    }
    v.sort();
    v
}

/// Shader source for a wallpaper spec path: a built-in id, or a .wgsl file path.
pub fn source_for(path: &str) -> Result<String, String> {
    if let Some(b) = builtin(path) {
        return Ok(b.to_string());
    }
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}
