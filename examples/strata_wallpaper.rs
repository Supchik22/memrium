//! Layered-paper wallpapers: diagonal bands of colour stacked like cut card, each edge stepping up in terraces, lit along
//! its top rim and casting a soft shadow on the band behind it, over a purple -> peach -> crimson gradient with a few faint
//! low-poly lines. `cargo run --release --example strata_wallpaper -- <width> <height> <seed> <out.png> [system | r,g,b]`
//! The last argument tones the sheets around a colour: `system` takes the desktop's accent from appearance.toml.
//! A theme may come before it: `strata` (the default), `mountains`, `waves` or `dunes` (the same paper look, a landscape
//! under a sky with a sun).

use std::f32::consts::PI;

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

struct Rng(u32);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = hash(self.0.wrapping_add(0x9e37_79b9));
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

/// A step of an edge: where it starts (share of the width), how wide the riser is and how far it lifts (share of the height).
struct Step {
    x: f32,
    w: f32,
    h: f32,
}

/// One edge between two bands: y (share of the height) at the left, its slope, and its terraces.
struct Edge {
    y0: f32,
    slope: f32,
    steps: Vec<Step>,
    tint: f32,
    /// 0 terraces, 1 mountains, 2 waves, 3 dunes
    kind: u8,
    /// mountains: (x, half width, height); waves and dunes: (amplitude, frequency, phase)
    parts: Vec<[f32; 3]>,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Edge {
    /// y of the edge at u (both shares of the screen); it rises (y falls) to the right and at every riser.
    fn y(&self, u: f32) -> f32 {
        match self.kind {
            1 => {
                // peaks with softly rounded tips, joined by a smooth maximum (no hard notches where slopes meet)
                let k = 70.0;
                let mut sum = 1.0;
                for p in &self.parts {
                    let d = ((u - p[0]).powi(2) + 0.012f32.powi(2)).sqrt();
                    let hgt = p[2] * (1.0 - d / p[1]).max(0.0);
                    sum += (k * hgt).exp();
                }
                self.y0 - sum.ln() / k - 0.004 * (u * 41.0 + self.tint * 7.0).sin() * (u * 13.0).cos()
            }
            2 => self.y0 + self.parts.iter().map(|p| p[0] * (2.0 * PI * p[1] * u + p[2]).sin()).sum::<f32>(),
            3 => {
                // a dune: a gentle windward slope and a steeper lee side
                self.y0
                    - self
                        .parts
                        .iter()
                        .map(|p| {
                            let t = 2.0 * PI * p[1] * u + p[2];
                            p[0] * (0.5 + 0.5 * (t.sin() + 0.32 * (2.0 * t).sin()) / 1.2)
                        })
                        .sum::<f32>()
            }
            _ => {
                let mut y = self.y0 + self.slope * u;
                for s in &self.steps {
                    y -= s.h * smooth((u - s.x) / s.w);
                }
                y
            }
        }
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn glow(u: f32, v: f32, cu: f32, cv: f32, su: f32, sv: f32) -> f32 {
    (-((u - cu) * (u - cu) / su + (v - cv) * (v - cv) / sv)).exp()
}

/// Each sheet's colours along its length (left, middle, right), from the furthest (top) to the nearest (bottom): deep
/// violet, purple, orchid, the peach band, coral, rose, crimson.
type Palette = [[[f32; 3]; 3]; 7];

const SHEETS: Palette = [
    [[0.25, 0.09, 0.40], [0.36, 0.12, 0.48], [0.46, 0.13, 0.42]],
    [[0.40, 0.14, 0.52], [0.52, 0.20, 0.58], [0.60, 0.20, 0.48]],
    [[0.58, 0.24, 0.60], [0.72, 0.36, 0.62], [0.80, 0.38, 0.48]],
    [[0.86, 0.56, 0.62], [0.96, 0.72, 0.62], [0.97, 0.62, 0.44]],
    [[0.84, 0.44, 0.58], [0.93, 0.54, 0.52], [0.90, 0.40, 0.36]],
    [[0.70, 0.24, 0.52], [0.82, 0.34, 0.50], [0.76, 0.22, 0.36]],
    [[0.52, 0.12, 0.42], [0.66, 0.16, 0.40], [0.60, 0.09, 0.28]],
];

/// The colour of sheet `i` at (u, v); `down` is how far below its own edge (0 at the edge, 1 deep in): lighter at the top
/// where the light falls, a little deeper further down.
fn sheet_colour(pal: &Palette, i: usize, u: f32, v: f32, down: f32) -> [f32; 3] {
    let p = &pal[i.min(pal.len() - 1)];
    // along the sheet: left -> middle -> right, with a slight drift by height so it is not a plain stripe
    let t = (u + (v - 0.5) * 0.15).clamp(0.0, 1.0);
    let mut c = if t < 0.5 { mix(p[0], p[1], smooth(t * 2.0)) } else { mix(p[1], p[2], smooth((t - 0.5) * 2.0)) };
    // half way to a gradient shared by all the sheets: neighbours differ, but softly
    let f = (v - 0.30 * (0.5 - u)).clamp(0.0, 1.0) * (pal.len() - 1) as f32;
    let (k0, k1) = (f.floor() as usize, (f.floor() as usize + 1).min(pal.len() - 1));
    let shared_at = |k: usize| {
        let q = &pal[k];
        if t < 0.5 { mix(q[0], q[1], smooth(t * 2.0)) } else { mix(q[1], q[2], smooth((t - 0.5) * 2.0)) }
    };
    let shared = mix(shared_at(k0), shared_at(k1), smooth(f - f.floor()));
    c = mix(c, shared, 0.45);
    let top = 1.0 - down;
    c = mix(c, [c[0] * 1.06 + 0.03, c[1] * 1.06 + 0.03, c[2] * 1.05 + 0.03], 0.4 * top * top);
    mix(c, [c[0] * 0.92, c[1] * 0.88, c[2] * 0.94], 0.45 * down)
}

/// A low-poly facet: the plane cut into triangles on a jittered grid; each triangle gets a small brightness offset.
fn facet(x: f32, y: f32, seed: u32) -> f32 {
    let cell = 0.16;
    let point = |i: i32, j: i32| -> [f32; 2] {
        let hsh = hash((i as u32).wrapping_mul(73_856_093) ^ (j as u32).wrapping_mul(19_349_663) ^ seed);
        let jx = ((hsh & 0xffff) as f32 / 65535.0 - 0.5) * 0.7;
        let jy = ((hsh >> 16) as f32 / 65535.0 - 0.5) * 0.7;
        [(i as f32 + 0.5 + jx) * cell, (j as f32 + 0.5 + jy) * cell]
    };
    let inside = |p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> bool {
        let s = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| (p[0] - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p[1] - r[1]);
        let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    };
    let (ci, cj) = ((x / cell - 0.5).floor() as i32, (y / cell - 0.5).floor() as i32);
    for dj in -1..=1 {
        for di in -1..=1 {
            let (i, j) = (ci + di, cj + dj);
            let (a, b, c, d) = (point(i, j), point(i + 1, j), point(i, j + 1), point(i + 1, j + 1));
            for (k, tri) in [[a, b, d], [a, d, c]].iter().enumerate() {
                if inside([x, y], tri[0], tri[1], tri[2]) {
                    let hsh = hash((i as u32).wrapping_mul(2_654_435_761) ^ (j as u32).wrapping_mul(40_503) ^ (k as u32 * 977) ^ seed);
                    return ((hsh & 0xffff) as f32 / 65535.0 - 0.5) * 0.03;
                }
            }
        }
    }
    0.0
}

fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

fn hue_of(c: [f32; 3]) -> f32 {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    if d < 1e-5 {
        return 0.0;
    }
    let h = if mx == c[0] { (c[1] - c[2]) / d % 6.0 } else if mx == c[1] { (c[2] - c[0]) / d + 2.0 } else { (c[0] - c[1]) / d + 4.0 };
    (h * 60.0).rem_euclid(360.0)
}

/// Sheets toned around one colour: from a deep berry at the back through the colour itself to a pale, warmer band in
/// the middle and deeper again in front; each sheet a little cooler on the left and warmer on the right.
fn palette_around(accent: [f32; 3]) -> (Palette, [f32; 3]) {
    let h = hue_of(accent);
    // (hue shift, saturation, lightness) per sheet, back to front
    let sheets = [(-28.0, 0.50, 0.26), (-18.0, 0.55, 0.38), (-8.0, 0.62, 0.55), (14.0, 0.78, 0.82), (6.0, 0.72, 0.70), (0.0, 0.66, 0.56), (-6.0, 0.60, 0.40)];
    let mut pal = [[[0.0; 3]; 3]; 7];
    for (i, (dh, s, l)) in sheets.iter().enumerate() {
        pal[i] = [hsl(h + dh - 10.0, *s, l - 0.04), hsl(h + dh, *s, *l), hsl(h + dh + 12.0, s * 1.05, l + 0.02)];
    }
    (pal, hsl(h - 5.0, 0.88, 0.80))
}

/// A landscape's colours: the sky (top, horizon), the sun, the furthest and the nearest sheet, and the cool and warm
/// ends each sheet leans to from left to right.
struct Land {
    sky: [[f32; 3]; 2],
    sun: [f32; 3],
    sun_at: [f32; 3],
    far: [f32; 3],
    near: [f32; 3],
    cool: [f32; 3],
    warm: [f32; 3],
}

fn land(theme: &str) -> Land {
    match theme {
        "waves" => Land { sky: [[0.72, 0.87, 0.97], [1.0, 0.92, 0.86]], sun: [1.0, 0.97, 0.90], sun_at: [0.30, 0.33, 0.060], far: [0.62, 0.85, 0.93], near: [0.05, 0.20, 0.40], cool: [0.42, 0.55, 0.95], warm: [0.55, 0.95, 0.85] },
        "dunes" => Land { sky: [[0.99, 0.79, 0.62], [1.0, 0.93, 0.80]], sun: [1.0, 0.97, 0.88], sun_at: [0.72, 0.30, 0.080], far: [0.97, 0.75, 0.54], near: [0.52, 0.20, 0.17], cool: [0.95, 0.55, 0.58], warm: [1.0, 0.82, 0.45] },
        _ => Land { sky: [[0.96, 0.76, 0.82], [1.0, 0.90, 0.82]], sun: [1.0, 0.96, 0.88], sun_at: [0.66, 0.34, 0.075], far: [0.92, 0.70, 0.80], near: [0.30, 0.12, 0.38], cool: [0.70, 0.60, 0.95], warm: [1.0, 0.70, 0.60] },
    }
}

/// Sheets from far (hazy, light) to near (deep): the paper look of a landscape.
fn palette_land(l: &Land) -> Palette {
    let mut pal = [[[0.0; 3]; 3]; 7];
    for (i, sheet) in pal.iter_mut().enumerate() {
        let base = mix(l.far, l.near, (i as f32 / 6.0).powf(0.9));
        *sheet = [mix(base, l.cool, 0.10), base, mix(base, l.warm, 0.12)];
    }
    pal
}

/// The sky behind a landscape, with the sun and its glow.
fn sky(l: &Land, u: f32, v: f32, aspect: f32) -> [f32; 3] {
    let mut c = mix(l.sky[0], l.sky[1], smooth(v / 0.62));
    let d = (((u - l.sun_at[0]) * aspect).powi(2) + (v - l.sun_at[1]).powi(2)).sqrt();
    c = mix(c, l.sun, (-(d - l.sun_at[2]).max(0.0) / 0.12).exp() * 0.45);
    let disc = ((l.sun_at[2] - d) * 1080.0 + 0.5).clamp(0.0, 1.0);
    mix(c, l.sun, disc)
}

/// The desktop's accent colour (appearance.toml, written by the shell).
fn system_accent() -> Option<[f32; 3]> {
    let home = std::env::var("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    let text = std::fs::read_to_string(home.join("memrium/appearance.toml")).ok()?;
    let line = text.lines().find(|l| l.trim_start().starts_with("accent"))?;
    let nums: Vec<f32> = line.split(['[', ']', ',', '='].as_ref()).filter_map(|t| t.trim().parse().ok()).collect();
    (nums.len() >= 3).then(|| [nums[0], nums[1], nums[2]])
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let w: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(1920);
    let h: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1080);
    let seed: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(1);
    let out = args.get(4).cloned().unwrap_or_else(|| format!("strata_{seed}.png"));
    let mut rng = Rng(hash(seed.wrapping_mul(7919)));
    let themes = ["strata", "mountains", "waves", "dunes"];
    let theme = args.get(5).filter(|t| themes.contains(&t.as_str())).cloned().unwrap_or_else(|| "strata".into());
    let tone_arg = if args.get(5).is_some_and(|t| themes.contains(&t.as_str())) { args.get(6) } else { args.get(5) };
    let tone = match tone_arg.map(String::as_str) {
        Some("system") => system_accent(),
        Some(t) => {
            let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        }
        None => None,
    };
    let landscape = theme != "strata";
    let lnd = land(&theme);
    let (pal, haze) = match tone {
        _ if landscape => (palette_land(&lnd), lnd.sun),
        Some(a) => palette_around(a),
        None => (SHEETS, [0.86, 0.70, 0.96]),
    };
    // where the light comes from: the upper left over terraces, the sun over a landscape
    let light = if landscape { [lnd.sun_at[0], lnd.sun_at[1], 0.10, 0.07, 0.30] } else { [0.30, -0.05, 0.05, 0.05, 0.55] };

    // the edges, top (furthest back) to bottom (closest); the risers of neighbouring edges sit roughly above one another,
    // like terraces cut through several sheets
    let slope = -0.30;
    let columns: Vec<f32> = (0..5).map(|i| 0.08 + i as f32 * 0.19 + rng.range(-0.04, 0.04)).collect();
    let n = 6;
    let mut edges = Vec::new();
    let mut y = 0.0;
    for _ in 0..n {
        let mut steps = Vec::new();
        for &c in &columns {
            if rng.f() < 0.65 {
                // mostly up (to the right), now and then a step down; the risers lean, they are not walls
                let up = rng.f() < 0.88;
                let hgt = rng.range(0.04, 0.10) * if up { 1.0 } else { -0.8 };
                steps.push(Step { x: c + rng.range(-0.05, 0.05), w: rng.range(0.055, 0.095), h: hgt });
            }
        }
        let lift: f32 = steps.iter().map(|s| s.h).sum::<f32>();
        y += rng.range(0.14, 0.24);
        edges.push(Edge { y0: y + lift * 0.55 - 0.08, slope, steps, tint: rng.range(-1.0, 1.0), kind: 0, parts: Vec::new() });
    }
    if landscape {
        // a landscape replaces the terraces: horizontal sheets, the nearer ones lower and further apart
        edges.clear();
        let count = if theme == "waves" { 7 } else { 6 };
        for i in 0..count {
            let f = i as f32;
            let (kind, y0, parts) = match theme.as_str() {
                "mountains" => {
                    let n = 3 + (rng.f() * 3.0) as usize;
                    let parts = (0..n).map(|_| [rng.range(-0.1, 1.1), rng.range(0.15, 0.38), rng.range(0.10, 0.28) * (1.0 - f * 0.10)]).collect();
                    (1, 0.44 + f * 0.105 + rng.range(-0.02, 0.02), parts)
                }
                "waves" => {
                    let k = 1.0 + f * 0.25;
                    let parts = vec![[0.020 * k * rng.range(0.7, 1.3), rng.range(1.5, 3.0), rng.range(0.0, 6.3)], [0.010 * k * rng.range(0.7, 1.3), rng.range(4.0, 7.0), rng.range(0.0, 6.3)], [0.005 * k * rng.range(0.7, 1.3), rng.range(9.0, 14.0), rng.range(0.0, 6.3)]];
                    (2, 0.50 + f * 0.075, parts)
                }
                _ => {
                    let parts = vec![[rng.range(0.05, 0.12) * (1.0 + f * 0.1), rng.range(0.6, 1.6), rng.range(0.0, 6.3)], [rng.range(0.015, 0.035), rng.range(2.0, 3.5), rng.range(0.0, 6.3)]];
                    (3, 0.48 + f * 0.10, parts)
                }
            };
            edges.push(Edge { y0, slope: 0.0, steps: Vec::new(), tint: rng.range(-1.0, 1.0), kind, parts });
        }
    }
    // a few faint straight lines (the low-poly cuts), mostly on the right
    let aspect = w as f32 / h as f32;
    let pts: Vec<[f32; 2]> = (0..16).map(|_| [rng.range(0.40, 1.10), rng.range(-0.10, 1.10)]).collect();
    let mut lines: Vec<[f32; 4]> = Vec::new();
    let wants_lines = !landscape;
    for (i, a) in pts.iter().enumerate() {
        let mut near: Vec<(f32, usize)> = pts.iter().enumerate().filter(|(j, _)| *j != i).map(|(j, b)| ((((a[0] - b[0]) * aspect).powi(2) + (a[1] - b[1]).powi(2)), j)).collect();
        near.sort_by(|x, y| x.0.total_cmp(&y.0));
        for &(_, j) in near.iter().take(3) {
            if j > i || !lines.iter().any(|l| l == &[pts[j][0], pts[j][1], a[0], a[1]]) {
                lines.push([a[0], a[1], pts[j][0], pts[j][1]]);
            }
        }
    }
    if !wants_lines {
        lines.clear();
    }

    let (fw, fh) = (w as f32, h as f32);
    let px = 1.0 / fh; // one pixel in shares of the height
    // the shadow's reach, as a share of the height (about 26 px at 1080)
    let shadow_r = 42.0 / 1080.0;
    let mut img = vec![0u8; w * h * 3];
    let threads = std::thread::available_parallelism().map_or(8, |n| n.get());
    let rows = h.div_ceil(threads);
    let jitter = seed;
    std::thread::scope(|sc| {
        for (t, chunk) in img.chunks_mut(rows * w * 3).enumerate() {
            let (edges, lines, pal, lnd) = (&edges, &lines, &pal, &lnd);
            sc.spawn(move || {
                for (r, row) in chunk.chunks_mut(w * 3).enumerate() {
                    let y = t * rows + r;
                    let v = (y as f32 + 0.5) / fh;
                    for x in 0..w {
                        let u = (x as f32 + 0.5) / fw;
                        // the furthest sheet, then each nearer one over it
                        let mut c = if landscape { sky(lnd, u, v, aspect) } else { sheet_colour(pal, 0, u, v, 1.0) };
                        // how much of this pixel is a sheet rather than the sky
                        let mut solid: f32 = if landscape { 0.0 } else { 1.0 };
                        for (i, e) in edges.iter().enumerate() {
                            let ey = e.y(u);
                            let d = 0.0015;
                            let dy = (e.y(u + d) - e.y(u - d)) / (2.0 * d) * (fh / fw);
                            // the distance to the edge (shares of the height), signed: below it is this sheet. Near the
                            // edge it is the true distance to the curve (a riser is steep: straight down is too far)
                            let below = v > ey;
                            let mut dist = (v - ey).abs();
                            if dist < 4.0 * shadow_r {
                                let (x0, y0) = (x as f32 + 0.5, v * fh);
                                let mut best = f32::MAX;
                                let reach = 4.0 * shadow_r * fh;
                                let n = 24;
                                for k in -n..=n {
                                    let xs = x0 + k as f32 * reach / n as f32;
                                    let ys = e.y(xs / fw) * fh;
                                    best = best.min((x0 - xs).powi(2) + (y0 - ys).powi(2));
                                }
                                dist = dist.min(best.sqrt() / fh);
                            }
                            let across = if below { dist } else { -dist };
                            let me = sheet_colour(pal, i + 1, u, v, (across / 0.20).clamp(0.0, 1.0));
                            if across < 0.0 && !(landscape && i == 0) {
                                // above the edge (a landscape's furthest sheet casts nothing on the sky): this sheet's shadow on the one behind, in a deep shade of this sheet's
                                // own colour (light through coloured card), not grey
                                let s = 0.40 * (across / shadow_r).exp();
                                let tint = [0.45 + me[0] * 0.40, 0.35 + me[1] * 0.35, 0.50 + me[2] * 0.40];
                                c = mix(c, [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]], s);
                            }
                            let cover = ((across / px) + 0.5).clamp(0.0, 1.0);
                            if cover <= 0.0 {
                                continue;
                            }
                            let depth = (across / px).max(0.0);
                            // the rim catches the light: a thin bright line and a soft glow under it
                            let rim = 0.14 * (-depth / 1.6).exp() + 0.07 * (-depth / 26.0).exp();
                            let lit = rim * (1.0 - 0.5 * (dy.abs() / (1.0 + dy.abs())));
                            let s = mix(me, [1.0, 0.93, 0.93], lit);
                            c = mix(c, s, cover);
                            solid = solid.max(cover);
                        }
                        // low-poly facets on the right: each triangle a little lighter or darker
                        let f = facet(u * aspect, v, jitter) * smooth((u - 0.30) / 0.45) * solid;
                        c = [c[0] * (1.0 + f), c[1] * (1.0 + f), c[2] * (1.0 + f)];
                        for l in lines {
                            let (ax, ay, bx, by) = (l[0] * fw, l[1] * fh, l[2] * fw, l[3] * fh);
                            let (pxx, pyy) = (x as f32 + 0.5, y as f32 + 0.5);
                            let (dx, dy) = (bx - ax, by - ay);
                            let t = (((pxx - ax) * dx + (pyy - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                            let dist = ((pxx - ax - dx * t).powi(2) + (pyy - ay - dy * t).powi(2)).sqrt();
                            let a = (1.0 - dist / (0.9 * fh / 1080.0).max(0.7)).clamp(0.0, 1.0) * 0.035 * smooth((u - 0.35) / 0.4);
                            if a > 0.0 {
                                c = mix(c, [1.0, 0.85, 0.85], a);
                            }
                        }
                        // the light from the upper left: a lilac haze over everything
                        let g = glow(u, v, light[0], light[1], light[2], light[3]);
                        c = mix(c, haze, g * light[4]);
                        // a breath of grain, so the long gradients do not band
                        let n = (hash((y * w + x) as u32 ^ seed) & 0xff) as f32 / 255.0 - 0.5;
                        for k in 0..3 {
                            row[x * 3 + k] = ((c[k].clamp(0.0, 1.0) * 255.0) + n * 1.6).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            });
        }
    });
    image::save_buffer(&out, &img, w as u32, h as u32, image::ColorType::Rgb8).expect("save");
    eprintln!("saved {out}");
}
