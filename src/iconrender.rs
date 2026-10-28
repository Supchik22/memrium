//! Drawing an `IconDoc`: the layers are rasterised from their SVG, placed, recoloured for the clear and tinted looks and put
//! together with their blend modes. A layer with Liquid Glass also gets a shadow, frosts and bends what is behind it, lets
//! some of it through and catches light on its edges. `import_svg` makes a document out of an existing SVG icon, one layer
//! for each of its shapes.

use crate::icondoc::*;
use crate::svgedit::{self, ElementEdit, Outline, SvgDoc};
use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod gpu;

// ----------------------------------------------------------------------------------------------- pixel buffers

/// A square picture of floats, premultiplied: planes r, g, b, a.
#[derive(Clone)]
pub struct Buf {
    pub n: usize,
    pub c: [Vec<f32>; 4],
}

impl Buf {
    pub fn new(n: usize) -> Buf {
        Buf { n, c: [vec![0.0; n * n], vec![0.0; n * n], vec![0.0; n * n], vec![0.0; n * n]] }
    }

    pub fn from_image(img: &RgbaImage) -> Buf {
        let n = img.width() as usize;
        let mut b = Buf::new(n);
        for (i, p) in img.pixels().enumerate() {
            let a = p[3] as f32 / 255.0;
            b.c[0][i] = p[0] as f32 / 255.0 * a;
            b.c[1][i] = p[1] as f32 / 255.0 * a;
            b.c[2][i] = p[2] as f32 / 255.0 * a;
            b.c[3][i] = a;
        }
        b
    }

    pub fn to_image(&self) -> RgbaImage {
        let mut img = RgbaImage::new(self.n as u32, self.n as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            let a = self.c[3][i].clamp(0.0, 1.0);
            let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
            *p = Rgba([un(self.c[0][i]), un(self.c[1][i]), un(self.c[2][i]), (a * 255.0).round() as u8]);
        }
        img
    }

    fn alpha(&self) -> &[f32] {
        &self.c[3]
    }

    /// Bilinear sample of the four planes at (x, y) in pixels.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let n = self.n;
        let (x, y) = (x.clamp(0.0, n as f32 - 1.001), y.clamp(0.0, n as f32 - 1.001));
        let (x0, y0) = (x as usize, y as usize);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |xx: usize, yy: usize, k: usize| self.c[k][yy * n + xx];
        let mut out = [0.0; 4];
        for k in 0..4 {
            out[k] = at(x0, y0, k) * (1.0 - fx) * (1.0 - fy) + at(x0 + 1, y0, k) * fx * (1.0 - fy) + at(x0, y0 + 1, k) * (1.0 - fx) * fy + at(x0 + 1, y0 + 1, k) * fx * fy;
        }
        out
    }
}

/// A blur close to a Gaussian of deviation `sigma` (three box passes), in place, on an `n` x `n` plane.
fn blur(plane: &mut [f32], n: usize, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    let bw = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().round() as usize).max(1) | 1;
    let r = bw / 2;
    let mut tmp = vec![0.0f32; plane.len()];
    let (mut col, mut out) = (vec![0.0f32; n], vec![0.0f32; n]);
    for _ in 0..3 {
        for y in 0..n {
            box_line(&plane[y * n..(y + 1) * n], &mut tmp[y * n..(y + 1) * n], r);
        }
        for x in 0..n {
            for y in 0..n {
                col[y] = tmp[y * n + x];
            }
            box_line(&col, &mut out, r);
            for y in 0..n {
                plane[y * n + x] = out[y];
            }
        }
    }
}

/// One line of a box blur by a running sum. The sum is kept in f64: in f32 what is added and taken off again leaves a
/// residue that stays on the rest of the line (and on the rows after the shapes), which shows as streaks in the shadows.
fn box_line(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    let norm = 1.0 / (2 * r + 1) as f64;
    let mut sum: f64 = src[..r.min(n)].iter().map(|&v| v as f64).sum();
    for i in 0..n {
        if i + r < n {
            sum += src[i + r] as f64;
        }
        dst[i] = (sum * norm).max(0.0) as f32;
        if i >= r {
            sum -= src[i - r] as f64;
        }
    }
}

fn blur_buf(b: &Buf, sigma: f32) -> Buf {
    let mut out = b.clone();
    for k in 0..4 {
        blur(&mut out.c[k], b.n, sigma);
    }
    out
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `src` scaled by `s` about the centre and moved by `off` pixels.
fn transform(src: &Buf, s: f32, off: [f32; 2]) -> Buf {
    if (s - 1.0).abs() < 1e-4 && off[0].abs() < 1e-3 && off[1].abs() < 1e-3 {
        return src.clone();
    }
    let n = src.n;
    let c = n as f32 / 2.0;
    let mut out = Buf::new(n);
    let s = s.max(0.01);
    for y in 0..n {
        for x in 0..n {
            let sx = (x as f32 + 0.5 - c - off[0]) / s + c - 0.5;
            let sy = (y as f32 + 0.5 - c - off[1]) / s + c - 0.5;
            if sx < -1.0 || sy < -1.0 || sx > n as f32 || sy > n as f32 {
                continue;
            }
            let v = src.sample(sx, sy);
            for k in 0..4 {
                out.c[k][y * n + x] = v[k];
            }
        }
    }
    out
}

/// `src` (any size) laid on an empty `n` x `n` picture with its top left corner at `at`.
fn place_at(src: &Buf, n: usize, at: [i32; 2]) -> Buf {
    let mut out = Buf::new(n);
    let m = src.n as i32;
    for y in 0..n as i32 {
        let sy = y - at[1];
        if sy < 0 || sy >= m {
            continue;
        }
        for x in 0..n as i32 {
            let sx = x - at[0];
            if sx < 0 || sx >= m {
                continue;
            }
            for k in 0..4 {
                out.c[k][y as usize * n + x as usize] = src.c[k][(sy * m + sx) as usize];
            }
        }
    }
    out
}

// ----------------------------------------------------------------------------------------------- blending

fn blend_channel(mode: Blend, cb: f32, cs: f32) -> f32 {
    match mode {
        Blend::Normal => cs,
        Blend::Multiply => cb * cs,
        Blend::Screen => cb + cs - cb * cs,
        Blend::Overlay => {
            if cb <= 0.5 {
                2.0 * cb * cs
            } else {
                1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
            }
        }
        Blend::Darken => cb.min(cs),
        Blend::Lighten => cb.max(cs),
        Blend::Plus => (cb + cs).min(1.0),
    }
}

/// Put colour `cs` of alpha `a` over pixel `i` of `dst`.
fn put(dst: &mut Buf, i: usize, cs: [f32; 3], a: f32, mode: Blend) {
    if a <= 0.0 {
        return;
    }
    let ab = dst.c[3][i];
    for k in 0..3 {
        let pm = dst.c[k][i];
        let cb = if ab > 0.0 { (pm / ab).clamp(0.0, 1.0) } else { 0.0 };
        dst.c[k][i] = (1.0 - ab) * a * cs[k] + (1.0 - a) * pm + ab * a * blend_channel(mode, cb, cs[k]);
    }
    dst.c[3][i] = a + ab - a * ab;
}

// ----------------------------------------------------------------------------------------------- shapes

fn inside(shape: Shape, x: f32, y: f32, corner: f32) -> bool {
    match shape {
        Shape::None => true,
        Shape::Circle => x * x + y * y <= 1.0,
        Shape::Rounded => {
            let r = (0.44 * corner).clamp(0.04, 1.0);
            let (qx, qy) = (x.abs() - (1.0 - r), y.abs() - (1.0 - r));
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) <= r
        }
        Shape::Squircle => {
            let k = (2.0 + 3.0 / corner.max(0.2)).min(14.0);
            x.abs().powf(k) + y.abs().powf(k) <= 1.0
        }
        // a true superellipse, the curve Apple's icons are close to: the corner flows into the sides with no seam
        Shape::Super => {
            let k = (5.0 / corner.max(0.4)).clamp(2.5, 12.0);
            x.abs().powf(k) + y.abs().powf(k) <= 1.0
        }
    }
}

/// How much of each pixel the shape of half size `half` centred on the canvas covers (4 x 4 samples).
fn shape_mask(shape: Shape, n: usize, half: f32, corner: f32) -> Vec<f32> {
    let c = n as f32 / 2.0;
    let mut out = vec![0.0f32; n * n];
    let at = |x: f32, y: f32| inside(shape, (x - c) / half, (y - c) / half, corner);
    // the pixels well inside or outside need one look; only those on the edge are sampled finely (a coarse coverage steps
    // along a straight edge, and the glass lights the steps as a comb)
    const S: usize = 16;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32, y as f32);
            let corners = [at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0), at(fx + 0.5, fy + 0.5)];
            if corners.iter().all(|&v| v == corners[0]) {
                out[y * n + x] = if corners[0] { 1.0 } else { 0.0 };
                continue;
            }
            let mut hit = 0;
            for sy in 0..S {
                for sx in 0..S {
                    if at(fx + (sx as f32 + 0.5) / S as f32, fy + (sy as f32 + 0.5) / S as f32) {
                        hit += 1;
                    }
                }
            }
            out[y * n + x] = hit as f32 / (S * S) as f32;
        }
    }
    out
}

// ----------------------------------------------------------------------------------------------- the renderer

fn hash_of(parts: &[&dyn Fn(&mut std::collections::hash_map::DefaultHasher)]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p(&mut h);
    }
    h.finish()
}

/// Draws documents. It keeps the rasters of layers, so that changing a setting does not draw the SVGs again.
#[derive(Default)]
pub struct Renderer {
    rasters: HashMap<(u64, u32), Arc<RgbaImage>>,
    /// the flat version: every glass is replaced by a very plain one (a faint edge light and a small shadow)
    pub flat: bool,
    /// the global glass strength, applied on top of every glass setting
    pub tune: GlassTune,
    /// what is behind the icon (opaque): the glass blurs and bends it, and the picture then holds it
    pub backdrop: Option<Arc<RgbaImage>>,
    /// the layers only, cut to the plate but without it (on nothing): the shell draws the plate itself, as live glass over
    /// what is really behind the icon (the clear looks, see `LIVE_SUFFIX`)
    pub bare: bool,
    /// draw on the processor even when there is a graphics card
    pub cpu: bool,
    /// draw the tinted looks in this colour instead of the shell's accent (the look that tints each icon by its own colour)
    pub tint_override: Option<[f32; 3]>,
    /// the tinted looks take the colour of each icon instead of the accent (drawn by `render`, which finds that colour)
    pub tint_by_icon: bool,
    /// the buffers on the graphics card, for each size drawn there
    gpu: HashMap<u32, gpu::Ctx>,
    /// the coverage of the plate's shape: (shape, size, corner)
    masks: HashMap<(u8, u32, u32), Arc<Vec<f32>>>,
}

/// How strong all the glass of every icon is: a factor for each part of the glass, 1 leaves the settings as they are.
/// It is kept in the user's config, so the editor, the theme build and the next session draw the same.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassTune {
    pub specular: f32,
    pub blur: f32,
    pub refraction: f32,
    pub translucency: f32,
    pub shadow: f32,
    /// where the light comes from, in degrees (231 is the top left)
    pub light: f32,
    /// factors for how far the shadow falls and how soft it is
    pub shadow_dist: f32,
    pub shadow_soft: f32,
    /// factor for how round the corners of the plate are
    pub corner: f32,
    /// factor for how crisp the edges of the glass are: the bevel that bends the light and the line of light on the edge
    pub sharp: f32,
    /// the clear looks keep the colours of the layers (the plate is clear glass all the same), as Apple's do not
    pub colour_clear: bool,
    /// bevel, inner shadow and glow added to the glass of every layer (0 adds nothing), and how wide the bevel is (a factor)
    pub bevel: f32,
    pub bevel_width: f32,
    pub inner_shadow: f32,
    pub glow: f32,
    /// factors: the bright rim all round, the sheen across the glass, the fine line of light on the edge, how wide the lens
    /// that bends what is behind is, how far the colours split at its edge, how much the glass bends its own picture
    pub rim: f32,
    pub sheen: f32,
    pub edge_line: f32,
    /// factor for how thick the fine line of light on the edge is
    pub edge_width: f32,
    /// factor for how hard the edge of that line is (its width stays): 1 as drawn, more is a crisper step, less a softer fade
    pub edge_sharp: f32,
    pub lens_width: f32,
    pub dispersion: f32,
    pub inner_bend: f32,
    /// the soft, wide light that lies along the edge turned to the light, under the crisp line (a factor)
    pub soft_light: f32,
    /// the shape of the plate of every icon in every look (None: no plate at all); only how the icons are drawn, never kept in a document
    pub plate: Shape,
}

impl GlassTune {
    /// the settings that change nothing: every factor 1, nothing added
    const NEUTRAL: GlassTune = GlassTune {
        specular: 1.0, blur: 1.0, refraction: 1.0, translucency: 1.0, shadow: 1.0, light: 231.5, shadow_dist: 1.0, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, colour_clear: false,
        bevel: 0.0, bevel_width: 1.0, inner_shadow: 0.0, glow: 0.0, rim: 1.0, sheen: 1.0, edge_line: 1.0, edge_width: 1.0, edge_sharp: 1.0, lens_width: 1.0, dispersion: 1.0, inner_bend: 1.0, soft_light: 1.0, plate: Shape::Squircle,
    };
}

/// The reference glass: the look of the shell's own glass (src/glass.wgsl) on the icons, every factor at 1.
impl Default for GlassTune {
    fn default() -> GlassTune {
        GLASS_PRESETS[0].1
    }
}

/// The reference glass, the one the renders are made with unless the user tunes it (chosen by eye by the user, 2026-10-26):
/// little specular, but a strong fine edge line and soft light along the edge, crisp, much frosting and see-through.
pub const REFERENCE_GLASS: GlassTune = GlassTune {
    specular: 0.32, edge_line: 2.0, soft_light: 3.0, rim: 0.85, sheen: 1.32, sharp: 1.83,
    refraction: 0.5, lens_width: 1.0, dispersion: 1.0, inner_bend: 1.0, blur: 1.9, translucency: 1.4,
    bevel: 0.0, bevel_width: 1.0, inner_shadow: 0.0, glow: 0.0,
    shadow: 0.9, shadow_dist: 1.0, shadow_soft: 1.5,
    ..GlassTune::NEUTRAL
};

/// Ready-made looks of the glass, for the buttons of the editor (the first one is the reference).
pub const GLASS_PRESETS: [(&str, GlassTune); 5] = [
    ("Liquid", REFERENCE_GLASS),
    ("Frosted", GlassTune { specular: 0.8, blur: 1.9, refraction: 0.5, translucency: 1.4, shadow: 0.9, shadow_soft: 1.5, sharp: 0.6, ..GlassTune::NEUTRAL }),
    ("Crystal", GlassTune { specular: 1.6, blur: 0.2, refraction: 1.8, translucency: 1.6, shadow: 0.8, shadow_dist: 0.8, shadow_soft: 0.8, sharp: 1.5, ..GlassTune::NEUTRAL }),
    ("Matte", GlassTune { specular: 0.3, refraction: 0.2, translucency: 0.5, shadow: 1.2, shadow_dist: 1.2, shadow_soft: 1.4, sharp: 0.5, ..GlassTune::NEUTRAL }),
    ("Flat", GlassTune { specular: 0.15, blur: 0.0, refraction: 0.0, translucency: 0.0, shadow: 0.4, shadow_dist: 0.6, sharp: 1.0, ..GlassTune::NEUTRAL }),
];

impl GlassTune {
    fn path() -> std::path::PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
        base.join("memrium").join("icon-glass")
    }

    /// The saved tuning: numbers in a fixed order; an older file has fewer, and what it lacks stays as in the reference.
    pub fn load() -> GlassTune {
        let text = std::fs::read_to_string(GlassTune::path()).unwrap_or_default();
        let v: Vec<f32> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        let mut t = GlassTune::default();
        if v.len() < 5 {
            return t;
        }
        // (a file from before the light and corner settings has only the first five, and is read with the others neutral)
        if v.len() < 9 {
            t = GlassTune { light: GlassTune::NEUTRAL.light, shadow_dist: 1.0, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, ..t };
        }
        let at = |i: usize, d: f32| v.get(i).copied().unwrap_or(d);
        let n = GlassTune::NEUTRAL;
        GlassTune {
            specular: at(0, t.specular), blur: at(1, t.blur), refraction: at(2, t.refraction), translucency: at(3, t.translucency), shadow: at(4, t.shadow),
            light: at(5, t.light), shadow_dist: at(6, t.shadow_dist), shadow_soft: at(7, t.shadow_soft), corner: at(8, t.corner), sharp: at(9, t.sharp),
            colour_clear: at(10, 0.0) > 0.5,
            plate: Shape::from_code(at(11, t.plate.code() as f32) as u8).unwrap_or(t.plate),
            bevel: at(12, n.bevel), bevel_width: at(13, n.bevel_width), inner_shadow: at(14, n.inner_shadow), glow: at(15, n.glow),
            rim: at(16, n.rim), sheen: at(17, n.sheen), edge_line: at(18, n.edge_line), lens_width: at(19, n.lens_width), dispersion: at(20, n.dispersion), inner_bend: at(21, n.inner_bend), soft_light: at(22, n.soft_light), edge_width: at(23, n.edge_width), edge_sharp: at(24, n.edge_sharp),
        }
    }

    pub fn save(&self) {
        let path = GlassTune::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let v = [
            self.specular, self.blur, self.refraction, self.translucency, self.shadow, self.light, self.shadow_dist, self.shadow_soft, self.corner, self.sharp,
            self.colour_clear as u8 as f32, self.plate.code() as f32, self.bevel, self.bevel_width, self.inner_shadow, self.glow, self.rim, self.sheen,
            self.edge_line, self.lens_width, self.dispersion, self.inner_bend, self.soft_light, self.edge_width, self.edge_sharp,
        ];
        let _ = std::fs::write(path, v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ") + "\n");
    }

    fn apply(&self, g: Glass) -> Glass {
        if !g.enabled {
            return g;
        }
        Glass {
            specular: (g.specular * self.specular).clamp(0.0, 2.0),
            blur: (g.blur * self.blur).clamp(0.0, 1.0),
            refraction: (g.refraction * self.refraction).clamp(0.0, 2.0),
            translucency: (g.translucency * self.translucency).clamp(0.0, 1.0),
            shadow_amount: (g.shadow_amount * self.shadow).clamp(0.0, 1.0),
            bevel: (g.bevel + self.bevel).clamp(0.0, 1.0),
            bevel_width: (g.bevel_width * self.bevel_width).clamp(0.0, 1.0),
            inner_shadow: (g.inner_shadow + self.inner_shadow).clamp(0.0, 1.0),
            glow: (g.glow + self.glow).clamp(0.0, 1.0),
            ..g
        }
    }

    /// The direction the light travels in (pointing from the light to the glass).
    fn light_dir(&self) -> (f32, f32) {
        let a = self.light.to_radians();
        (a.cos(), a.sin())
    }
}

/// The glass of the flat version: no bending, frosting or see-through, only a faint light on the edge and a small shadow.
fn primitive_glass(g: Glass) -> Glass {
    if !g.enabled {
        return g;
    }
    Glass { enabled: true, specular: 0.2, blur: 0.0, refraction: 0.0, translucency: 0.0, shadow: ShadowKind::Neutral, shadow_amount: 0.15, ..Glass::default() }
}

/// What a layer or group contributes at the top of the drawing: scale and shift (in pixels) of everything below it.
#[derive(Clone, Copy)]
struct Place {
    scale: f32,
    off: [f32; 2],
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer::default()
    }

    /// The colour the tinted looks are drawn in now.
    fn tint(&self) -> [f32; 3] {
        self.tint_override.unwrap_or_else(tint)
    }

    /// The one colour of an icon: its layers' colours averaged, the more saturated the more weight, made bright and rich enough
    /// to tint a plate with. A grey icon gets a graphite.
    pub fn icon_colour(&mut self, doc: &IconDoc) -> [f32; 3] {
        if let Some(c) = doc.icon_tint {
            return c;
        }
        self.found_colour(doc)
    }

    /// The colour found from the layers, whatever was set by hand.
    pub fn found_colour(&mut self, doc: &IconDoc) -> [f32; 3] {
        let (bare, over) = (self.bare, self.tint_override.take());
        self.bare = true;
        let img = self.render(doc, Appearance::Default, 64);
        self.bare = bare;
        self.tint_override = over;
        let (mut sum, mut w_sum) = ([0.0f32; 3], 0.0f32);
        for p in img.pixels() {
            let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            let sat = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
            let w = p[3] as f32 / 255.0 * (0.02 + sat * sat);
            for k in 0..3 {
                sum[k] += c[k] * w;
            }
            w_sum += w;
        }
        if w_sum < 1e-3 {
            return [0.42, 0.44, 0.50];
        }
        let c = [sum[0] / w_sum, sum[1] / w_sum, sum[2] / w_sum];
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if hi - lo < 0.06 {
            return [0.42, 0.44, 0.50];
        }
        // brightest channel to 0.92, the colour pushed away from grey
        let k = 0.92 / hi.max(0.05);
        let c = [c[0] * k, c[1] * k, c[2] * k];
        let grey = (c[0] + c[1] + c[2]) / 3.0;
        [0, 1, 2].map(|i| (grey + (c[i] - grey) * 1.2).clamp(0.0, 1.0))
    }

    /// The drawn layer (straight alpha, `n` x `n`), recoloured when its settings or the look ask for it.
    pub fn raster(&mut self, layer: &Layer, props: &Props, doc: &IconDoc, app: Appearance, n: u32) -> Arc<RgbaImage> {
        let treated = (app.is_clear() && !self.tune.colour_clear) || app.is_tinted();
        let key = hash_of(&[
            &|h| layer.svg.hash(h),
            &|h| props.fill.map(|c| c.map(|v| v.to_bits())).hash(h),
            &|h| props.gradient.map(|g| (g.a.map(f32::to_bits), g.b.map(f32::to_bits), g.angle.to_bits(), g.radial, g.blobs.used().iter().map(|b| (b.at.map(f32::to_bits), b.size.to_bits(), b.colour.map(f32::to_bits))).collect::<Vec<_>>())).hash(h),
            &|h| (if treated { Some(app.label()) } else { None }, if app.is_tinted() { self.tint().map(|v| v.to_bits()) } else { [0; 3] }).hash(h),
        ]);
        if let Some(img) = self.rasters.get(&(key, n)) {
            return img.clone();
        }
        let mut text = layer.svg.clone();
        if let Some(fill) = props.fill {
            if let Some(d) = SvgDoc::from_text(&layer.svg) {
                let el = d.elements();
                let (has_fill, has_stroke) = el.first().map_or((true, false), |e| (e.fill.is_some(), e.stroke.is_some()));
                text = d.render_text(&[ElementEdit { index: 0, hidden: false, fill: has_fill.then_some(fill), stroke: has_stroke.then_some(fill) }], None);
            }
        }
        let mut img = svgedit::rasterize(&text, n).unwrap_or_else(|| RgbaImage::new(n, n));
        if let Some(g) = props.gradient {
            paint_gradient(&mut img, &g);
        }
        if treated {
            // (read once: it is behind a lock, and every thread drawing at once would queue on it for each pixel)
            let tint = self.tint();
            for p in img.pixels_mut() {
                let c = treat([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0], app, tint);
                *p = Rgba([(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, p[3]]);
            }
        }
        if self.rasters.len() > 600 {
            self.rasters.clear();
        }
        let img = Arc::new(img);
        self.rasters.insert((key, n), img.clone());
        img
    }

    /// The plate as it is drawn: the document's own, except that the shape is the global one, and that the default look of an
    /// icon without a plate gets a light one (so that it is not the only look left bare).
    pub fn plate_for(&self, doc: &IconDoc, app: Appearance) -> Background {
        let mut b = doc.background.resolve(app, &auto_background(self.tint()));
        if self.tune.plate == Shape::None {
            return Background { shape: Shape::None, fill: FillKind::None, glass: Glass { enabled: false, ..b.glass }, ..b };
        }
        if app == Appearance::Default && b.fill == FillKind::None {
            b = Background { fill: FillKind::System, glass: Glass { enabled: true, specular: 0.5, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() }, ..b };
        }
        Background { shape: self.tune.plate, ..b }
    }

    /// The document as a picture of `n` x `n` pixels in the look `app`.
    pub fn render(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        if self.tint_by_icon && app.is_tinted() && self.tint_override.is_none() {
            let c = self.icon_colour(doc);
            self.tint_override = Some(c);
            let img = self.render(doc, app, n);
            self.tint_override = None;
            return img;
        }
        // (the tests check the sums of the processor's version)
        if !self.cpu && !cfg!(test) {
            if let Some(img) = gpu::render(self, doc, app, n) {
                return img;
            }
        }
        self.render_cpu(doc, app, n)
    }

    /// `render` on the processor.
    pub fn render_cpu(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        let nn = n as usize;
        let fit = plate_fit();
        let bg = self.plate_for(doc, app);
        let mask = (bg.shape != Shape::None).then(|| shape_mask(bg.shape, nn, n as f32 * fit / 2.0, self.tune.corner));
        let mut accum = match &self.backdrop {
            Some(_) if self.bare => Buf::new(nn),
            Some(bd) => {
                let img = if bd.width() == n { (**bd).clone() } else { image::imageops::resize(&**bd, n, n, image::imageops::FilterType::Triangle) };
                Buf::from_image(&img)
            }
            None => Buf::new(nn),
        };

        // the plate
        let mut plate = Buf::new(nn);
        let (ptop, pbottom) = match bg.fill {
            FillKind::Solid => (bg.top, bg.top),
            FillKind::System if app.is_dark() => (DARK_BASE_TOP, DARK_BASE_BOTTOM),
            FillKind::System => ([1.0, 1.0, 1.0], [0.90, 0.91, 0.94]),
            FillKind::Auto => (bg.top.map(|v| v + (1.0 - v) * 0.22), bg.top.map(|v| v * 0.92)),
            _ => (bg.top, bg.bottom),
        };
        if bg.fill != FillKind::None && !self.bare {
            let top = n as f32 * (1.0 - fit) / 2.0;
            for y in 0..nn {
                let t = ((y as f32 + 0.5 - top) / (n as f32 * fit)).clamp(0.0, 1.0);
                let c = [mix(ptop[0], pbottom[0], t), mix(ptop[1], pbottom[1], t), mix(ptop[2], pbottom[2], t)];
                for x in 0..nn {
                    let a = mask.as_ref().map_or(1.0, |m| m[y * nn + x]);
                    let i = y * nn + x;
                    plate.c[0][i] = c[0] * a;
                    plate.c[1][i] = c[1] * a;
                    plate.c[2][i] = c[2] * a;
                    plate.c[3][i] = a;
                }
            }
        }
        // the plate is glass like any layer, but nothing clips its shadow
        let plate_glass = if self.flat { primitive_glass(bg.glass) } else { self.tune.apply(bg.glass) };
        if !self.bare {
            composite(&mut accum, &plate, &Props { glass: plate_glass, ..Props::default() }, None, &self.tune);
        }

        for node in &doc.nodes {
            self.draw_node(node, doc, app, &mut accum, Place { scale: fit, off: [0.0, 0.0] }, mask.as_deref());
        }
        accum.to_image()
    }

    fn draw_node(&mut self, node: &Node, doc: &IconDoc, app: Appearance, accum: &mut Buf, place: Place, mask: Option<&[f32]>) {
        let mut p = effective_props(node, app);
        if !p.visible {
            return;
        }
        p.glass = if self.flat { primitive_glass(p.glass) } else { self.tune.apply(p.glass) };
        let n = accum.n;
        let nf = n as f32;
        match &node.kind {
            Kind::Layer(layer) => {
                // the drawing is made at the size it is shown, and put in place by whole pixels, so that scaling it up
                // does not blur it
                let k = p.scale * place.scale;
                let m = (nf * k).round().clamp(8.0, 2.0 * nf.max(512.0)) as u32;
                let img = self.raster(layer, &p, doc, app, m);
                let src = Buf::from_image(&img);
                let off = [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]];
                let moved = place_at(&src, n, [(nf / 2.0 + off[0] - m as f32 / 2.0).round() as i32, (nf / 2.0 + off[1] - m as f32 / 2.0).round() as i32]);
                composite(accum, &moved, &p, mask, &self.tune);
            }
            Kind::Group(g) => {
                let plain = (p.opacity - 1.0).abs() < 1e-4 && p.blend == Blend::Normal && (p.scale - 1.0).abs() < 1e-4 && p.offset == [0.0, 0.0];
                let combined = p.glass_mode == GlassMode::Combined && p.glass.enabled;
                if plain && !combined {
                    for c in &g.children {
                        self.draw_node(c, doc, app, accum, place, mask);
                    }
                    return;
                }
                // the group is drawn on its own and then put on as one piece
                let mut inner = Buf::new(n);
                for c in &g.children {
                    self.draw_node(c, doc, app, &mut inner, Place { scale: 1.0, off: [0.0, 0.0] }, None);
                }
                let moved = transform(&inner, p.scale * place.scale, [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]]);
                let own = if combined { p } else { Props { glass: Glass { enabled: false, ..p.glass }, ..p } };
                composite(accum, &moved, &own, mask, &self.tune);
            }
        }
    }

    /// The scale and shift (as fractions of the icon) that put layer `id` where it is drawn, and whether it is shown.
    pub fn placement(doc: &IconDoc, app: Appearance, id: u32) -> Option<(f32, [f32; 2], bool)> {
        fn path<'a>(nodes: &'a [Node], id: u32, out: &mut Vec<&'a Node>) -> bool {
            for n in nodes {
                out.push(n);
                if n.id == id || path(n.children(), id, out) {
                    return true;
                }
                out.pop();
            }
            false
        }
        let mut chain = Vec::new();
        if !path(&doc.nodes, id, &mut chain) {
            return None;
        }
        let fit = plate_fit();
        // from the layer up: the layer's own placement first, then each group's
        let (mut s, mut o, mut shown) = (1.0f32, [0.0f32, 0.0f32], true);
        for (k, n) in chain.iter().rev().enumerate() {
            let p = effective_props(n, app);
            shown &= p.visible;
            if k == 0 {
                s = p.scale;
                o = p.offset;
            } else {
                // a plain group is not drawn on its own, so it moves nothing
                o = [o[0] * p.scale + p.offset[0], o[1] * p.scale + p.offset[1]];
                s *= p.scale;
            }
        }
        Some((s * fit, [o[0] * fit, o[1] * fit], shown))
    }

    /// The topmost visible layer under the point (x, y) of a picture of `n` pixels.
    pub fn hit(&mut self, doc: &IconDoc, app: Appearance, n: u32, x: f32, y: f32) -> Option<u32> {
        let layers: Vec<Node> = doc.layers().into_iter().cloned().collect();
        for node in layers.iter().rev() {
            let Kind::Layer(layer) = &node.kind else { continue };
            let Some((s, o, shown)) = Self::placement(doc, app, node.id) else { continue };
            if !shown {
                continue;
            }
            let img = self.raster(layer, &effective_props(node, app), doc, app, n);
            let c = n as f32 / 2.0;
            let sx = (x - c - o[0] * n as f32) / s + c;
            let sy = (y - c - o[1] * n as f32) / s + c;
            if sx < 0.0 || sy < 0.0 || sx >= n as f32 || sy >= n as f32 {
                continue;
            }
            if img.get_pixel(sx as u32, sy as u32)[3] > 40 {
                return Some(node.id);
            }
        }
        None
    }

    /// The shape of node `id` (a layer, or all the layers of a group) as the silhouette drawn on the icon, for outlining it.
    pub fn silhouette(&mut self, doc: &IconDoc, app: Appearance, id: u32, n: u32) -> Option<RgbaImage> {
        fn gather<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
            match &node.kind {
                Kind::Layer(_) => out.push(node),
                Kind::Group(g) => g.children.iter().for_each(|c| gather(c, out)),
            }
        }
        let mut layers = Vec::new();
        gather(doc.find(id)?, &mut layers);
        let layers: Vec<Node> = layers.into_iter().cloned().collect();
        let mut all = Buf::new(n as usize);
        for node in &layers {
            let Kind::Layer(layer) = &node.kind else { continue };
            let (s, o, _) = Self::placement(doc, app, node.id)?;
            let img = self.raster(layer, &effective_props(node, app), doc, app, n);
            let moved = transform(&Buf::from_image(&img), s, [o[0] * n as f32, o[1] * n as f32]);
            for i in 0..all.c[3].len() {
                all.c[3][i] = all.c[3][i].max(moved.c[3][i]);
            }
        }
        let mut out = RgbaImage::new(n, n);
        for (i, p) in out.pixels_mut().enumerate() {
            *p = Rgba([255, 255, 255, (all.c[3][i].clamp(0.0, 1.0) * 255.0) as u8]);
        }
        Some(out)
    }
}

/// The shape of `img` (its alpha) painted with the gradient: along its angle across the box of the shape, from the middle
/// of that box out, or with spots of colour (blobs) over the first colour.
fn paint_gradient(img: &mut RgbaImage, g: &Gradient) {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0.0f32, 0.0f32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 8 {
            x0 = x0.min(x as f32);
            y0 = y0.min(y as f32);
            x1 = x1.max(x as f32 + 1.0);
            y1 = y1.max(y as f32 + 1.0);
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh, cx, cy) = (x1 - x0, y1 - y0, (x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let rad = g.angle.to_radians();
    let (dx, dy) = (rad.sin(), rad.cos());
    let span = (bw * dx.abs() + bh * dy.abs()).max(1.0);
    for (x, y, p) in img.enumerate_pixels_mut() {
        if p[3] == 0 {
            continue;
        }
        let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        if g.blobs.count > 0 {
            // blobs live in the box of the shape (stretched with it, as an SVG radial gradient is)
            let c = g.blobs.colour_at(g.a, (px + bw / 2.0) / bw, (py + bh / 2.0) / bh);
            for k in 0..3 {
                p[k] = (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            continue;
        }
        let t = if g.radial { (px.hypot(py) / (bw.max(bh) / 2.0)).clamp(0.0, 1.0) } else { ((px * dx + py * dy) / span + 0.5).clamp(0.0, 1.0) };
        for k in 0..3 {
            p[k] = ((g.a[k] + (g.b[k] - g.a[k]) * t).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// The starting point of a layer's settings in look `app`: the clear and tinted looks are made by the system, so a layer
/// is glass there unless a look says otherwise.
fn auto_props(app: Appearance, base: &Props) -> Props {
    let mut p = *base;
    if app.is_clear() || app.is_tinted() {
        p.glass = auto_glass(app);
        if app.is_clear() {
            p.opacity *= 0.92;
        }
    }
    p
}

/// The settings a node is drawn with in look `app`: those of the default look, what the system makes of them in the clear
/// and tinted looks, and what the looks on the way (the parent looks, then this one) changed.
pub fn effective_props(node: &Node, app: Appearance) -> Props {
    match node.kind {
        Kind::Layer(_) => node.props().resolve(app, &auto_props),
        Kind::Group(_) => node.props().get(app),
    }
}

/// Make the settings of `node` in look `app` be `p`: the look keeps only what differs from what it inherits.
pub fn set_node_props(node: &mut Node, app: Appearance, p: Props) {
    match node.kind {
        Kind::Layer(_) => node.props_mut().set(app, p, &auto_props),
        Kind::Group(_) => node.props_mut().set(app, p, &|_, d| *d),
    }
}

/// How a colour of the artwork is made in the clear and tinted looks: light and shade are kept, the colour is not.
fn treat(c: [f32; 3], app: Appearance, tint: [f32; 3]) -> [f32; 3] {
    let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    match app {
        Appearance::ClearLight => [0.50 + 0.50 * l; 3],
        Appearance::ClearDark => [0.32 + 0.62 * l; 3],
        Appearance::TintedLight => [mix(tint[0], 1.0, 0.35 + 0.65 * l), mix(tint[1], 1.0, 0.35 + 0.65 * l), mix(tint[2], 1.0, 0.35 + 0.65 * l)],
        Appearance::TintedDark => [tint[0] * (0.42 + 0.55 * l), tint[1] * (0.42 + 0.55 * l), tint[2] * (0.42 + 0.55 * l)],
        _ => c,
    }
}

/// The glass a layer gets in the clear and tinted looks when it was not set there.
fn auto_glass(app: Appearance) -> Glass {
    Glass { enabled: true, specular: 0.75, blur: if app.is_clear() { 0.4 } else { 0.0 }, refraction: 0.5, translucency: if app.is_clear() { 0.3 } else { 0.2 }, shadow: ShadowKind::Neutral, shadow_amount: 0.3, ..Glass::default() }
}

static TINT: std::sync::Mutex<Option<[f32; 3]>> = std::sync::Mutex::new(None);

/// The colour of the tinted looks: the accent of the shell's theme, the same for every icon (nothing in a document keeps it).
pub fn tint() -> [f32; 3] {
    let mut t = TINT.lock().unwrap();
    *t.get_or_insert_with(|| chebrium::theme::Appearance::load().accent)
}

/// Draw the tinted looks in `c` from now on (the editor follows the theme with this).
pub fn set_tint(c: [f32; 3]) {
    *TINT.lock().unwrap() = Some(c);
}

/// The empty border round the plate, a fraction of the icon: the same for every icon, so that all the plates are one size
/// (what a document holds as its margin is not used).
pub const PLATE_MARGIN: f32 = 0.04;

pub fn plate_fit() -> f32 {
    1.0 - 2.0 * PLATE_MARGIN
}

/// The plate of every icon in the dark look.
pub const DARK_BASE_TOP: [f32; 3] = [0.20, 0.20, 0.23];
pub const DARK_BASE_BOTTOM: [f32; 3] = [0.10, 0.10, 0.12];

/// The starting point of the plate in look `app`, made of the default one: darker for the dark look, grey glass for the clear
/// looks, the tint colour for the tinted ones.
fn auto_background(tint: [f32; 3]) -> impl Fn(Appearance, &Background) -> Background {
    move |app, base| {
        let shape = if base.shape == Shape::None { Shape::Squircle } else { base.shape };
        let glass = |specular: f32| Glass { enabled: true, specular, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() };
        // the clear plate is frosted glass: what is under the icon shows through it
        let clear = |specular: f32| Glass { translucency: 0.7, blur: 0.6, ..glass(specular) };
        let scaled = |k: f32| [tint[0] * k, tint[1] * k, tint[2] * k];
        match app {
            Appearance::Default => *base,
            // the base of every dark icon is the same, whatever the light one is
            Appearance::Dark => Background { shape, fill: FillKind::Gradient, top: DARK_BASE_TOP, bottom: DARK_BASE_BOTTOM, glass: glass(0.5) },
            Appearance::ClearLight => Background { shape, fill: FillKind::Gradient, top: [0.66; 3], bottom: [0.52; 3], glass: clear(0.75) },
            Appearance::ClearDark => Background { shape, fill: FillKind::Gradient, top: [0.24; 3], bottom: [0.15; 3], glass: clear(0.6) },
            Appearance::TintedLight => Background { shape, fill: FillKind::Gradient, top: scaled(1.05), bottom: scaled(0.82), glass: glass(0.7) },
            Appearance::TintedDark => Background { shape, fill: FillKind::Gradient, top: scaled(0.2), bottom: scaled(0.12), glass: glass(0.9) },
        }
    }
}

/// The plate in look `app`: what the system makes of the default one, with what the looks on the way changed.
pub fn background_for(doc: &IconDoc, app: Appearance) -> Background {
    doc.background.resolve(app, &auto_background(tint()))
}

/// Make the plate in look `app` be `b`: the look keeps only what differs from what it inherits.
pub fn set_background(doc: &mut IconDoc, app: Appearance, b: Background) {
    let auto = auto_background(tint());
    doc.background.set(app, b, &auto);
}

// ----------------------------------------------------------------------------------------------- putting a layer on

/// Put `layer` (already placed) on `accum`, with the layer's opacity, blend mode and Liquid Glass.
fn composite(accum: &mut Buf, layer: &Buf, p: &Props, mask: Option<&[f32]>, tune: &GlassTune) {
    let n = accum.n;
    let mut layer = layer.clone();
    if let Some(m) = mask {
        for i in 0..n * n {
            for k in 0..4 {
                layer.c[k][i] *= m[i];
            }
        }
    }
    let g = p.glass;
    let nf = n as f32;
    let alpha: Vec<f32> = layer.c[3].clone();
    let glass = g.enabled;

    // the shape of the glass as the shell's glass shader sees it (src/glass.wgsl): how deep inside the silhouette a pixel is
    // (in pixels, drawn for a 128 pixel icon: everything scales with the picture) and which way the edge faces
    let unit = nf / 128.0;
    let sharp = tune.sharp.clamp(0.25, 2.5);
    let sharp01 = ((sharp - 0.25) / 2.25).clamp(0.0, 1.0);
    // the glass is a surface made the way the bevel is: the shape blurred into a smooth hill. Its slope says which way the
    // edge faces (the normal, round in every corner) and how much the glass bends there: steep at the edge, flat inside.
    // Nothing is drawn from the exact distance to the edge: that is measured to the pixels inside, so it follows their
    // steps (no smoothing on a curved edge), and its lines of equal distance turn into sharp boxes inside a corner
    let bezel = (0.09 * nf * tune.lens_width.clamp(0.2, 3.0)).max(2.0);
    let (mut nrm, mut slope): (Vec<(f32, f32)>, Vec<f32>) = (Vec::new(), Vec::new());
    if glass && (g.specular > 0.0 || g.refraction > 0.0 || g.inner_shadow > 0.0) {
        let mut hill = alpha.clone();
        blur(&mut hill, n, bezel * 0.45);
        nrm = vec![(0.0, -1.0); n * n];
        slope = vec![0.0; n * n];
        let at = |x: usize, y: usize| hill[y.min(n - 1) * n + x.min(n - 1)];
        let mut peak = 1e-6f32;
        for y in 0..n {
            for x in 0..n {
                // (the hill rises inwards: the outward normal is minus its gradient)
                let (gx, gy) = (at(x + 1, y) - at(x.saturating_sub(1), y), at(x, y + 1) - at(x, y.saturating_sub(1)));
                let l = gx.hypot(gy);
                if l > 1e-7 {
                    nrm[y * n + x] = (-gx / l, -gy / l);
                }
                slope[y * n + x] = l;
                peak = peak.max(l);
            }
        }
        for v in slope.iter_mut() {
            *v /= peak;
        }
    }
    // the rounded edge of a bevel: as wide as it is asked to be
    let (mut bev, mut bev_peak) = (Vec::new(), 1e-4f32);
    if glass && g.bevel > 0.0 {
        let mut height = alpha.clone();
        blur(&mut height, n, nf * (0.008 + 0.06 * g.bevel_width.clamp(0.0, 1.0)) / sharp);
        let mut slope = vec![(0.0f32, 0.0f32); n * n];
        let at = |x: usize, y: usize| height[y.min(n - 1) * n + x.min(n - 1)];
        for y in 0..n {
            for x in 0..n {
                let gx = (at(x + 1, y) - at(x.saturating_sub(1), y)) * 0.5;
                let gy = (at(x, y + 1) - at(x, y.saturating_sub(1))) * 0.5;
                bev_peak = bev_peak.max(gx.hypot(gy));
                slope[y * n + x] = (gx, gy);
            }
        }
        bev = slope;
    }
    // how far the glass bends what is behind it at a pixel: as steep as the hill is there
    let bend_at = |i: usize| -> f32 {
        if slope.is_empty() {
            return 0.0;
        }
        let k = slope[i];
        k * k
    };

    // the layer's own colour glowing round it
    if glass && g.glow > 0.0 {
        let mut s = alpha.clone();
        blur(&mut s, n, nf * 0.05);
        let mut colour = [layer.c[0].clone(), layer.c[1].clone(), layer.c[2].clone()];
        for k in 0..3 {
            blur(&mut colour[k], n, nf * 0.05);
        }
        for i in 0..n * n {
            let sv = s[i];
            let mut a = (sv * g.glow * 0.9).clamp(0.0, 1.0);
            if let Some(m) = mask {
                a *= m[i];
            }
            if a <= 0.0 {
                continue;
            }
            let sa = sv.max(1e-4);
            put(accum, i, [(colour[0][i] / sa).clamp(0.0, 1.0), (colour[1][i] / sa).clamp(0.0, 1.0), (colour[2][i] / sa).clamp(0.0, 1.0)], a, Blend::Normal);
        }
    }

    // a shadow under the layer
    if glass && g.shadow != ShadowKind::None && g.shadow_amount > 0.0 {
        let mut s = alpha.clone();
        blur(&mut s, n, nf * 0.03 * tune.shadow_soft);
        let (tint, mut colour) = (g.shadow == ShadowKind::Chromatic, [Vec::new(), Vec::new(), Vec::new()]);
        if tint {
            for k in 0..3 {
                colour[k] = layer.c[k].clone();
                blur(&mut colour[k], n, nf * 0.03 * tune.shadow_soft);
            }
        }
        let dy = (nf * 0.025 * tune.shadow_dist).round() as usize;
        for y in dy..n {
            for x in 0..n {
                let i = y * n + x;
                let sv = s[(y - dy) * n + x];
                // the far tail of the blur is cut off, or the box it reaches shows as a faint square round the shape
                let mut a = ((sv - 0.01) * 1.0101).max(0.0) * g.shadow_amount * 0.5;
                // a shadow that reaches the edge of the picture is cut there; it fades out before it, or the picture shows as a pale square
                a *= smooth(x.min(n - 1 - x).min(y.min(n - 1 - y)) as f32 / (nf * 0.035));
                if let Some(m) = mask {
                    a *= m[i];
                }
                if a <= 0.0 {
                    continue;
                }
                let c = if tint {
                    let sa = sv.max(1e-4);
                    let base = [colour[0][(y - dy) * n + x] / sa, colour[1][(y - dy) * n + x] / sa, colour[2][(y - dy) * n + x] / sa];
                    let l = 0.3 * base[0] + 0.59 * base[1] + 0.11 * base[2];
                    [(mix(l, base[0], 1.4) * 0.7).clamp(0.0, 1.0), (mix(l, base[1], 1.4) * 0.7).clamp(0.0, 1.0), (mix(l, base[2], 1.4) * 0.7).clamp(0.0, 1.0)]
                } else {
                    [0.03, 0.06, 0.14]
                };
                put(accum, i, c, a, Blend::Normal);
            }
        }
    }

    // what is behind the glass: frosted and bent
    if glass && g.blur > 0.0 {
        let frosted = blur_buf(accum, nf * 0.045 * g.blur);
        for i in 0..n * n {
            let w = alpha[i].clamp(0.0, 1.0);
            if w > 0.0 {
                for k in 0..4 {
                    accum.c[k][i] = mix(accum.c[k][i], frosted.c[k][i], w);
                }
            }
        }
    }
    if glass && g.refraction > 0.0 && !slope.is_empty() {
        let behind = accum.clone();
        let bend = nf * 0.11 * g.refraction;
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                let w = alpha[i].clamp(0.0, 1.0);
                if w <= 0.0 {
                    continue;
                }
                let k2 = bend_at(i);
                if k2 <= 0.0 {
                    continue;
                }
                // the edge bends the scene inwards; the colours by a little different amounts, so that the lens splits the light
                let (nx, ny) = (-nrm[i].0, -nrm[i].1);
                let at = |f: f32| behind.sample(x as f32 + nx * bend * k2 * f, y as f32 + ny * bend * k2 * f);
                let (vr, vg, vb) = (at(1.0 + 0.1 * tune.dispersion), at(1.0), at(1.0 - 0.1 * tune.dispersion));
                let v = [vr[0], vg[1], vb[2], vg[3]];
                for k in 0..4 {
                    accum.c[k][i] = mix(accum.c[k][i], v[k], w);
                }
            }
        }
    }

    // the glass bends its own picture at the edges too, so the bending shows even when nothing is let through
    if glass && g.refraction > 0.0 && !slope.is_empty() {
        let own = layer.clone();
        let bend = nf * 0.05 * g.refraction * tune.inner_bend;
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                let la = own.c[3][i];
                if la <= 0.0 {
                    continue;
                }
                let k2 = bend_at(i);
                if k2 <= 0.0 {
                    continue;
                }
                let v = own.sample(x as f32 - nrm[i].0 * bend * k2, y as f32 - nrm[i].1 * bend * k2);
                // only the colour is bent, and only in from what is inside the shape: the silhouette and its soft edge stay as they were
                let w = smooth(((v[3] / la.max(1e-4)) - 0.3) / 0.6);
                if v[3] > 1e-4 && w > 0.0 {
                    for k in 0..3 {
                        let bent = v[k] / v[3] * la;
                        layer.c[k][i] = mix(own.c[k][i], bent, w);
                    }
                }
            }
        }
    }


    // the layer itself
    let see_through = if glass { 1.0 - 0.85 * g.translucency } else { 1.0 };
    for i in 0..n * n {
        let la = layer.c[3][i];
        if la <= 0.0 {
            continue;
        }
        let cs = [layer.c[0][i] / la, layer.c[1][i] / la, layer.c[2][i] / la];
        put(accum, i, cs, (la * p.opacity * see_through).clamp(0.0, 1.0), p.blend);
    }

    // the bevel: the edge turned to the light is lighter, the one turned away darker; and a shade inside along the edge
    if glass && (g.bevel > 0.0 || g.inner_shadow > 0.0) {
        let (lx, ly) = tune.light_dir();
        // (the shade along the edge from the blurred shape too, so that it is round in the corners)
        let mut inner = Vec::new();
        if g.inner_shadow > 0.0 {
            inner = alpha.clone();
            blur(&mut inner, n, 0.04 * nf * tune.lens_width.clamp(0.2, 3.0));
        }
        for i in 0..n * n {
            let a = alpha[i];
            if a <= 0.0 {
                continue;
            }
            let da = accum.c[3][i];
            if g.bevel > 0.0 && !bev.is_empty() {
                let (nx, ny) = (bev[i].0 / bev_peak, bev[i].1 / bev_peak);
                let edge = nx.hypot(ny).clamp(0.0, 1.0);
                let facing = -(nx * lx + ny * ly);
                let v = facing.clamp(-1.0, 1.0) * edge.powf(0.7) * g.bevel * smooth(a) * p.opacity;
                for k in 0..3 {
                    let c = accum.c[k][i];
                    accum.c[k][i] = if v > 0.0 { mix(c, da, (v * 0.6).min(0.8)) } else { (c * (1.0 + v * 0.7)).max(0.0) };
                }
            }
            if g.inner_shadow > 0.0 && !inner.is_empty() {
                let edge = (2.0 * (1.0 - inner[i])).clamp(0.0, 1.0).powf(1.5);
                let v = (edge * g.inner_shadow * smooth(a) * p.opacity * 0.55).clamp(0.0, 0.8);
                for k in 0..3 {
                    accum.c[k][i] *= 1.0 - v;
                }
            }
        }
    }

    // light on the edges, as the shell's glass puts it (src/glass.wgsl): a thin line and a soft inner glow, both brightest
    // where the edge faces the light, with a weaker answer on the far side, and a darker inner edge on the unlit sides
    if glass && g.specular > 0.0 && !slope.is_empty() {
        let (lx, ly) = tune.light_dir();
        let rim = g.specular * tune.rim * 1.3 * p.opacity;
        // how near the edge a pixel is, for the soft light: from the blurred shape and not from the distance to the edge, whose
        // lines of equal distance turn into sharp boxes inside a corner
        let mut near = alpha.clone();
        blur(&mut near, n, 10.0 * unit.max(0.25));
        // the fine line on the edge from the shape blurred a little, so that it is smooth along a curve (antialiased)
        let mut line = alpha.clone();
        blur(&mut line, n, ((1.6 + (0.55 - 1.6) * sharp01) * unit * tune.edge_width).max(0.6));
        for i in 0..n * n {
            let a = alpha[i];
            if a <= 0.0 {
                continue;
            }
            // (no pixel is skipped for being deep inside: the sheen lies over the whole glass, and a cut by the distance to
            // the edge showed as a square in the middle of the plate)
            let (nx, ny) = nrm[i];
            let nd = nx * lx + ny * ly;
            let edge = ((2.0 * (1.0 - line[i]) - 0.5) * tune.edge_sharp + 0.5).clamp(0.0, 1.0);
            let glow = (2.0 * (1.0 - near[i])).clamp(0.0, 1.0) * (1.0 - 0.7 * sharp01);
            let spec = nd.max(0.0).powf(1.6) + (-nd).max(0.0).powf(2.4) * 0.65;
            let da = accum.c[3][i];
            // (the rim fades where the glass is already bright, so that it never blows out to white)
            let lum = if da > 1e-4 { (0.299 * accum.c[0][i] + 0.587 * accum.c[1][i] + 0.114 * accum.c[2][i]) / da } else { 0.0 };
            let rim_k = 1.0 + (0.3 - 1.0) * smooth((lum - 0.35) / 0.45);
            let light = rim_k * (edge * (0.15 + 0.55 * spec) * (0.55 + 1.3 * sharp01) * tune.edge_line + glow * (0.055 + 0.075 * spec) * tune.soft_light) * rim;
            // (capped: with the rim turned up it would leave dark clouds where the edge turns from the light, in the corners)
            let dark = (0.05 * glow * (1.0 - spec.min(1.0)) * rim).min(0.12);
            // a soft sheen lying across the glass towards the light
            let (px, py) = ((i % n) as f32 / nf - 0.5, (i / n) as f32 / nf - 0.5);
            let sheen = (0.5 - (px * lx + py * ly) * 0.9).clamp(0.0, 1.0).powi(2) * g.specular * 0.05 * tune.sheen * p.opacity;
            // the glass covers `a` of the pixel, and the light is on the glass: it is lit (up to white) and then put on by
            // the coverage, so that the edge stays smooth (lit first and cut by the coverage after, an edge pixel the shape
            // barely touches was as white as one inside, and a curved edge showed its pixel steps)
            for k in 0..3 {
                let c = accum.c[k][i];
                let lit = (c + da * (light + sheen) - da * dark).clamp(0.0, da);
                accum.c[k][i] = mix(c, lit, a.min(1.0));
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------- importing an SVG

/// A document made from an SVG icon: a layer for each shape, groups as the drawing has them. The plate is empty, so the
/// icon looks as it did until something is changed.
pub fn import_svg(name: &str, svg_text: &str) -> Result<IconDoc, String> {
    let doc = SvgDoc::from_text(svg_text).ok_or("this is not an SVG picture")?;
    let elements = doc.elements();
    if elements.is_empty() {
        return Err("this picture has no shapes".into());
    }
    let mut out = IconDoc::new(name);
    fn build(doc: &SvgDoc, items: &[Outline], elements: &[svgedit::Element], out: &mut IconDoc) -> Vec<Node> {
        let mut nodes = Vec::new();
        for item in items {
            match item {
                Outline::Shape(i) => {
                    let e = &elements[*i];
                    let label = if e.id.is_empty() { format!("{} {}", e.tag, i + 1) } else { e.id.clone() };
                    let id = out.fresh_id();
                    nodes.push(Node { id, name: label, kind: Kind::Layer(Layer { svg: doc.render_text(&[], Some(*i)), props: Variants::new(Props::default()) }) });
                }
                Outline::Group { name, children } => {
                    let kids = build(doc, children, elements, out);
                    let id = out.fresh_id();
                    let label = if name.is_empty() { format!("Group {id}") } else { name.clone() };
                    nodes.push(Node { id, name: label, kind: Kind::Group(Group { children: kids, props: Variants::new(Props::default()) }) });
                }
            }
        }
        nodes
    }
    let nodes = build(&doc, &doc.outline(), &elements, &mut out);
    out.nodes = nodes;
    // an icon's own picture fills its canvas: no empty border until a shadow needs one
    out.margin = 0.0;
    Ok(out)
}

// ----------------------------------------------------------------------------------------------- the icon theme

pub const THEME_NAME: &str = "Memrium-Icons";
const THEME_SIZES: [u32; 4] = [48, 64, 128, 256];

/// Where the theme is written: `~/.local/share/icons/Memrium-Icons`.
pub fn theme_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("icons").join(THEME_NAME)
}

/// The file the shell watches: it is touched whenever the theme was written, so that the shell reads the icons again.
pub fn marker_path() -> std::path::PathBuf {
    theme_dir().join(".rebuilt")
}

pub fn touch_marker() {
    let _ = std::fs::create_dir_all(theme_dir());
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let _ = std::fs::write(marker_path(), at.to_string());
}

/// The looks the theme is written in, each as a theme of its own: the look and the end of the theme's name. The default
/// look is `Memrium-Icons` itself.
pub const THEME_LOOKS: [(Appearance, &str); 7] = [
    (Appearance::Default, ""),
    (Appearance::Dark, "-Dark"),
    (Appearance::ClearLight, "-Clear-Light"),
    (Appearance::ClearDark, "-Clear-Dark"),
    (Appearance::TintedLight, "-Tinted-Light"),
    (Appearance::TintedDark, "-Tinted-Dark"),
    (Appearance::TintedLight, ICON_TINT_SUFFIX),
];

/// The theme of the tinted look in which every icon has a colour of its own (its own), not the shell's accent.
pub const ICON_TINT_SUFFIX: &str = "-Tinted-Icon";

/// Tinted looks that follow the shell's accent (the one tinted by the icons does not).
fn follows_accent(look: Appearance, suffix: &str) -> bool {
    look.is_tinted() && suffix != ICON_TINT_SUFFIX
}

/// The folder of the theme of one look, next to `base`.
fn look_dir(base: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let name = base.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string());
    base.with_file_name(format!("{name}{suffix}"))
}

/// Every folder the editor writes themes into, for removing them.
pub fn theme_dirs() -> Vec<std::path::PathBuf> {
    THEME_LOOKS.iter().map(|(_, suffix)| look_dir(&theme_dir(), suffix)).collect()
}

fn write_index(out: &std::path::Path, source: &str) -> Result<(), String> {
    let name = out.file_name().map_or(THEME_NAME.to_string(), |n| n.to_string_lossy().to_string());
    let dirs = THEME_SIZES.iter().map(|s| format!("{s}x{s}/apps")).collect::<Vec<_>>().join(",");
    let sections: String = THEME_SIZES.iter().map(|s| format!("[{s}x{s}/apps]\nContext=Applications\nSize={s}\nType=Fixed\n\n")).collect();
    let index = format!("[Icon Theme]\nName={name}\nComment=Application icons made with Memrium Icons\nInherits={source},hicolor\n\nDirectories={dirs}\n\n{sections}");
    std::fs::write(out.join("index.theme"), index).map_err(|e| e.to_string())
}

/// The end of the name of the theme that holds the clear looks without their plate (`Memrium-Icons-Clear-Light-Live`): the
/// shell draws the plate of those icons itself, as live glass that bends and frosts what is really behind them. It inherits
/// the clear theme, so an icon without a document comes from there as it is.
pub const LIVE_SUFFIX: &str = "-Live";

/// The looks whose plate the shell draws live.
pub fn is_live_look(look: Appearance) -> bool {
    matches!(look, Appearance::ClearLight | Appearance::ClearDark)
}

/// The theme of the live version of a look, next to its theme `look_out`.
fn live_dir(look_out: &std::path::Path) -> std::path::PathBuf {
    let name = look_out.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string());
    look_out.with_file_name(format!("{name}{LIVE_SUFFIX}"))
}

/// Write the live (plateless) pictures of `doc` in `look` next to the look's theme `look_out`.
fn write_live(renderer: &mut Renderer, doc: &IconDoc, look: Appearance, look_out: &std::path::Path) -> Result<usize, String> {
    let out = live_dir(look_out);
    let inherits = look_out.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string());
    prepare(&out, &inherits)?;
    renderer.bare = true;
    let written = write_look(renderer, doc, look, &out);
    renderer.bare = false;
    written
}

/// How the shell draws the plate of a live icon of `size` px: the side of the plate and its corner radius, negative for the
/// continuous corners of a squircle (as `glass.wgsl`'s `sd_shape` takes it). None: the icons have no plate.
pub fn live_plate(tune: &GlassTune, size: f32) -> Option<(f32, f32)> {
    let side = size * plate_fit();
    let half = side / 2.0;
    let r = match tune.plate {
        Shape::None => return None,
        Shape::Circle => half,
        Shape::Rounded => (0.44 * tune.corner).clamp(0.04, 1.0) * half,
        // a superellipse of exponent k reaches 0.5^(1/k) of its half along the diagonal; the shell's continuous corner (a
        // cube-root curve of radius r) reaches 1 - (1 - 0.5^(1/3)) r there: the radius that meets the same point
        Shape::Squircle => -squircle_radius((2.0 + 3.0 / tune.corner.max(0.2)).min(14.0), half),
        Shape::Super => -squircle_radius((5.0 / tune.corner.max(0.4)).clamp(2.5, 12.0), half),
    };
    Some((side, r))
}

fn squircle_radius(k: f32, half: f32) -> f32 {
    let diag = 0.5f32.powf(1.0 / k);
    ((1.0 - diag) / (1.0 - 0.5f32.powf(1.0 / 3.0)) * half).clamp(0.05 * half, half)
}

/// One icon drawn in a look, written into the theme folder `out` in every size of the theme. Each size is drawn on its own
/// pixels (a smaller copy shrunk from a big picture is soft: its edges fall between the pixels).
fn write_look(renderer: &mut Renderer, doc: &IconDoc, look: Appearance, out: &std::path::Path) -> Result<usize, String> {
    let by_icon = out.file_name().is_some_and(|n| n.to_string_lossy().ends_with(ICON_TINT_SUFFIX));
    if by_icon {
        renderer.tint_override = Some(renderer.icon_colour(doc));
    }
    let result = write_sizes(renderer, doc, look, out);
    renderer.tint_override = None;
    result
}

fn write_sizes(renderer: &mut Renderer, doc: &IconDoc, look: Appearance, out: &std::path::Path) -> Result<usize, String> {
    let mut written = 0;
    for &s in &THEME_SIZES {
        renderer.render(doc, look, s).save(out.join(format!("{s}x{s}/apps/{}.png", doc.name))).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

fn prepare(out: &std::path::Path, source: &str) -> Result<(), String> {
    for &size in &THEME_SIZES {
        std::fs::create_dir_all(out.join(format!("{size}x{size}/apps"))).map_err(|e| e.to_string())?;
    }
    if !out.join("index.theme").exists() {
        write_index(out, source)?;
    }
    Ok(())
}

/// Do `work` for every document on all the cores but one, each thread with a renderer of its own (on the processor only
/// when `cpu`), and add up what it returns. `cancel` and `progress` are called on this thread only, while the others draw.
fn for_each_doc(docs: &[IconDoc], tune: GlassTune, cpu: bool, cancel: &dyn Fn() -> bool, progress: &dyn Fn(usize, usize), work: impl Fn(&mut Renderer, &IconDoc) -> Result<usize, String> + Sync) -> Result<usize, String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(1).clamp(1, docs.len().max(1));
    let (next, done, alive, written) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(threads), AtomicUsize::new(0));
    let stop = AtomicBool::new(false);
    let failed = std::sync::Mutex::new(None::<String>);
    let mut cancelled = false;
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut renderer = Renderer::new();
                renderer.tune = tune;
                renderer.cpu = cpu;
                while !stop.load(Relaxed) {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(doc) = docs.get(i) else { break };
                    match work(&mut renderer, doc) {
                        Ok(n) => {
                            written.fetch_add(n, Relaxed);
                        }
                        Err(e) => {
                            failed.lock().unwrap().get_or_insert(e);
                            stop.store(true, Relaxed);
                        }
                    }
                    done.fetch_add(1, Relaxed);
                }
                alive.fetch_sub(1, Relaxed);
            });
        }
        let mut shown = 0;
        loop {
            let d = done.load(Relaxed);
            if d != shown {
                shown = d;
                progress(d, docs.len());
            }
            if alive.load(Relaxed) == 0 {
                break;
            }
            if !cancelled && cancel() {
                cancelled = true;
                stop.store(true, Relaxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    });
    if cancelled {
        return Err("cancelled".into());
    }
    match failed.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(written.into_inner()),
    }
}

/// Write the pictures of one icon into the themes of all the looks (a quick version of `build_theme`, for the live preview
/// while editing), and tell the shell.
pub fn build_one(doc: &IconDoc, tune: GlassTune, source: &str, notify: bool) -> Result<(), String> {
    // each look on a thread of its own
    let results: Vec<Result<usize, String>> = std::thread::scope(|s| {
        let jobs: Vec<_> = THEME_LOOKS
            .iter()
            .map(|&(look, suffix)| {
                s.spawn(move || {
                    let mut renderer = Renderer::new();
                    renderer.tune = tune;
                    let out = look_dir(&theme_dir(), suffix);
                    prepare(&out, source)?;
                    let mut written = write_look(&mut renderer, doc, look, &out)?;
                    if is_live_look(look) {
                        written += write_live(&mut renderer, doc, look, &out)?;
                    }
                    Ok(written)
                })
            })
            .collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Err("drawing a look failed".into()))).collect()
    });
    for r in results {
        r?;
    }
    if notify {
        touch_marker();
    }
    Ok(())
}

/// Write the icon themes: for each look a theme with a PNG of every size for each document, named after the icon it
/// replaces. `out` is the theme of the default look; the others are next to it. The themes inherit `source`, so every icon
/// without a document comes from there. Returns the number of pictures.
pub fn build_theme(docs: &[IconDoc], source: &str, out: &std::path::Path, cancel: &dyn Fn() -> bool, progress: &dyn Fn(usize, usize)) -> Result<usize, String> {
    for (_, suffix) in THEME_LOOKS {
        prepare(&look_dir(out, suffix), source)?;
    }
    for (look, suffix) in THEME_LOOKS {
        if is_live_look(look) {
            let base = look_dir(out, suffix);
            prepare(&live_dir(&base), &base.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string()))?;
        }
    }
    let written = for_each_doc(docs, GlassTune::load(), false, cancel, progress, |renderer, doc| {
        let mut written = 0;
        for (look, suffix) in THEME_LOOKS {
            written += write_look(renderer, doc, look, &look_dir(out, suffix))?;
            if is_live_look(look) {
                written += write_live(renderer, doc, look, &look_dir(out, suffix))?;
            }
        }
        Ok(written)
    })?;
    for (look, suffix) in THEME_LOOKS {
        write_index(&look_dir(out, suffix), source)?;
        if is_live_look(look) {
            let base = look_dir(out, suffix);
            write_index(&live_dir(&base), &base.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string()))?;
        }
    }
    prune(out, docs);
    stamp_tint(out, tint());
    touch_marker();
    Ok(written)
}

/// The file in the theme of a tinted look that keeps the accent its pictures were drawn in.
fn tint_stamp(look_out: &std::path::Path) -> std::path::PathBuf {
    look_out.join(".tint")
}

fn tint_text(c: [f32; 3]) -> String {
    format!("{:.4} {:.4} {:.4}", c[0], c[1], c[2])
}

fn stamp_tint(out: &std::path::Path, c: [f32; 3]) {
    for (look, suffix) in THEME_LOOKS {
        if follows_accent(look, suffix) {
            let _ = std::fs::write(tint_stamp(&look_dir(out, suffix)), tint_text(c));
        }
    }
}

/// The shell's accent is now `accent`: when the tinted looks of the theme were drawn in another one, draw them again in the
/// background and tell the shell. The colour is baked into their pictures, so without this they keep the accent of the
/// last build. A new accent while drawing (the picker sends one on every step of a drag) starts the work over.
pub fn follow_accent(accent: [f32; 3]) {
    use std::sync::mpsc::{channel, Sender};
    static WORKER: std::sync::OnceLock<std::sync::Mutex<Sender<[f32; 3]>>> = std::sync::OnceLock::new();
    let out = theme_dir();
    let built = THEME_LOOKS.iter().filter(|(look, s)| follows_accent(*look, s)).all(|(_, suffix)| look_dir(&out, suffix).join("index.theme").is_file());
    let current = THEME_LOOKS.iter().filter(|(look, s)| follows_accent(*look, s)).all(|(_, suffix)| std::fs::read_to_string(tint_stamp(&look_dir(&out, suffix))).is_ok_and(|t| t == tint_text(accent)));
    if !built || current {
        return;
    }
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = channel::<[f32; 3]>();
        std::thread::spawn(move || {
            let Ok(mut want) = rx.recv() else { return };
            loop {
                // wait until the accent has stayed still for a moment
                while let Ok(c) = rx.recv_timeout(std::time::Duration::from_millis(800)) {
                    want = c;
                }
                let newer = std::cell::Cell::new(None);
                let cancel = || {
                    while let Ok(c) = rx.try_recv() {
                        newer.set(Some(c));
                    }
                    newer.get().is_some()
                };
                let done = retint(want, &cancel);
                if let Some(c) = newer.get() {
                    want = c;
                    continue;
                }
                if let Err(e) = done {
                    eprintln!("[icons] redrawing the tinted looks: {e}");
                }
                match rx.recv() {
                    Ok(c) => want = c,
                    Err(_) => return,
                }
            }
        });
        std::sync::Mutex::new(tx)
    });
    let _ = worker.lock().unwrap().send(accent);
}

/// Draw the tinted looks of every saved document again in `accent`, into the themes the editor wrote.
fn retint(accent: [f32; 3], cancel: &dyn Fn() -> bool) -> Result<(), String> {
    set_tint(accent);
    let out = theme_dir();
    let docs = saved_docs();
    for (look, suffix) in THEME_LOOKS {
        if follows_accent(look, suffix) {
            let dir = look_dir(&out, suffix);
            for &size in &THEME_SIZES {
                std::fs::create_dir_all(dir.join(format!("{size}x{size}/apps"))).map_err(|e| e.to_string())?;
            }
        }
    }
    // (the shell runs this: a core is left for it, and the processor draws, so that the shell never holds the dedicated card
    // open and keeps it awake)
    for_each_doc(&docs, GlassTune::load(), true, cancel, &|_, _| {}, |renderer, doc| {
        let mut written = 0;
        for (look, suffix) in THEME_LOOKS {
            if follows_accent(look, suffix) {
                written += write_look(renderer, doc, look, &look_dir(&out, suffix))?;
            }
        }
        Ok(written)
    })?;
    prune(&out, &docs);
    stamp_tint(&out, accent);
    touch_marker();
    Ok(())
}

/// Remove from the themes of every look (and their live versions) the pictures of icons that no longer have a document:
/// left behind, they would stay in the colours and the accent they were drawn in, instead of the pack's own icon.
fn prune(out: &std::path::Path, docs: &[IconDoc]) {
    let names: std::collections::HashSet<&str> = docs.iter().map(|d| d.name.as_str()).collect();
    for (look, suffix) in THEME_LOOKS {
        let dir = look_dir(out, suffix);
        let mut dirs = vec![dir.clone()];
        if is_live_look(look) {
            dirs.push(live_dir(&dir));
        }
        for d in dirs {
            for &size in &THEME_SIZES {
                let Ok(rd) = std::fs::read_dir(d.join(format!("{size}x{size}/apps"))) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "png") && p.file_stem().is_some_and(|n| !names.contains(n.to_string_lossy().as_ref())) {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------- turning a layer

/// What can be done to the drawing of a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SvgOp {
    FlipH,
    FlipV,
    RotateCw,
    RotateCcw,
}

/// The SVG with its drawing flipped or turned about the centre of the picture: the drawing is wrapped in a group that
/// carries the transform, so that nothing in it is rewritten.
pub fn transform_svg(svg: &str, op: SvgOp) -> Option<String> {
    let open_at = svg.find("<svg")?;
    let open_end = open_at + svg[open_at..].find('>')? + 1;
    let close_at = svg.rfind("</svg>")?;
    if close_at < open_end || svg[..open_end].ends_with("/>") {
        return None;
    }
    let tag = &svg[open_at..open_end];
    let attr = |name: &str| -> Option<String> {
        let key = format!("{name}=\"");
        let from = tag.find(&key)? + key.len();
        Some(tag[from..from + tag[from..].find('"')?].to_string())
    };
    let nums = |s: &str| -> Vec<f32> { s.split(|c: char| c == ' ' || c == ',').filter_map(|t| t.parse().ok()).collect() };
    let (x, y, w, h) = match attr("viewBox").map(|v| nums(&v)) {
        Some(v) if v.len() == 4 => (v[0], v[1], v[2], v[3]),
        _ => (0.0, 0.0, attr("width")?.trim_end_matches("px").parse().ok()?, attr("height")?.trim_end_matches("px").parse().ok()?),
    };
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let t = match op {
        SvgOp::FlipH => format!("translate({} 0) scale(-1 1)", 2.0 * cx),
        SvgOp::FlipV => format!("translate(0 {}) scale(1 -1)", 2.0 * cy),
        SvgOp::RotateCw => format!("rotate(90 {cx} {cy})"),
        SvgOp::RotateCcw => format!("rotate(-90 {cx} {cy})"),
    };
    Some(format!("{}<g transform=\"{t}\">{}</g>{}", &svg[..open_end], &svg[open_end..close_at], &svg[close_at..]))
}

#[cfg(test)]
mod tests {
    /// a renderer that draws no plate of its own, for the tests of the layers alone
    fn bare() -> Renderer {
        let mut r = Renderer::new();
        r.tune.plate = Shape::None;
        r
    }

    use super::*;

    const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
 <rect id="back" x="8" y="8" width="48" height="48" fill="#ff0000"/>
 <g id="pair"><circle cx="32" cy="32" r="14" fill="#0000ff"/><circle cx="46" cy="18" r="6" fill="#00ff00"/></g>
</svg>"##;

    fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        img.get_pixel(x, y).0
    }

    /// The graphics card draws what the processor draws: every part of the glass, a group drawn on its own, every look and
    /// a size of each blur (the one-pass blur up to 512 pixels, the box at a time above). Without a dedicated card there is
    /// nothing to compare.
    #[test]
    fn the_card_draws_what_the_processor_draws() {
        let mut doc = import_svg("t", ICON).unwrap();
        let ids: Vec<u32> = doc.layers().iter().map(|l| l.id).collect();
        for (k, id) in ids.iter().enumerate() {
            let p = &mut doc.find_mut(*id).unwrap().props_mut().default;
            p.glass = Glass { enabled: true, specular: 0.8, blur: 0.6, refraction: 0.7, translucency: 0.3, shadow: if k == 0 { ShadowKind::Chromatic } else { ShadowKind::Neutral }, shadow_amount: 0.5, bevel: 0.4, bevel_width: 0.5, inner_shadow: 0.3, glow: 0.3 };
            p.blend = if k == 2 { Blend::Overlay } else { Blend::Normal };
        }
        let group = doc.nodes[1].id;
        doc.find_mut(group).unwrap().props_mut().default = Props { opacity: 0.8, scale: 0.9, offset: [0.03, -0.02], glass: Glass { enabled: true, ..Glass::default() }, glass_mode: GlassMode::Combined, ..Props::default() };
        let mut cpu = Renderer::new();
        cpu.tune.bevel = 0.2;
        let mut card = Renderer { tune: cpu.tune, ..Renderer::new() };
        for look in Appearance::ALL {
            for n in [64, 700] {
                let Some(b) = gpu::render(&mut card, &doc, look, n) else { return };
                let a = cpu.render_cpu(&doc, look, n);
                let diffs: Vec<u8> = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y)).collect();
                let mean = diffs.iter().map(|&d| d as f64).sum::<f64>() / diffs.len() as f64;
                let worst = diffs.iter().max().copied().unwrap_or(0);
                assert!(mean < 0.05 && worst <= 32, "{look:?} at {n}: mean {mean:.4}, worst {worst}");
            }
        }
    }

    #[test]
    fn an_import_has_a_layer_per_shape_and_looks_like_the_svg() {
        let doc = import_svg("t", ICON).unwrap();
        assert_eq!(doc.layers().len(), 3);
        assert!(matches!(doc.nodes[1].kind, Kind::Group(_)), "the <g> became a group");
        let img = bare().render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 32, 32), [0, 0, 255, 255], "the blue circle on the red square");
        assert_eq!(px(&img, 12, 50), [255, 0, 0, 255]);
        assert_eq!(px(&img, 2, 2)[3], 0);
    }

    #[test]
    fn opacity_hiding_and_blending_change_the_picture() {
        let mut doc = import_svg("t", ICON).unwrap();
        let circle = doc.layers()[1].id;
        let mut r = bare();
        // hidden: the red square shows where the circle was
        {
            let p = &mut doc.find_mut(circle).unwrap().props_mut().default;
            p.visible = false;
        }
        assert_eq!(px(&r.render(&doc, Appearance::Default, 64), 32, 32), [255, 0, 0, 255]);
        // half transparent: red and blue mixed
        {
            let p = &mut doc.find_mut(circle).unwrap().props_mut().default;
            p.visible = true;
            p.opacity = 0.5;
        }
        let mixed = px(&r.render(&doc, Appearance::Default, 64), 32, 32);
        assert!(mixed[0] > 100 && mixed[2] > 100, "red and blue: {mixed:?}");
        // multiplied: blue times red is black
        {
            let p = &mut doc.find_mut(circle).unwrap().props_mut().default;
            p.opacity = 1.0;
            p.blend = Blend::Multiply;
        }
        let black = px(&r.render(&doc, Appearance::Default, 64), 32, 32);
        assert!(black[0] < 20 && black[2] < 20, "multiply: {black:?}");
    }

    #[test]
    fn a_layer_can_be_recoloured_scaled_and_moved() {
        let mut doc = import_svg("t", ICON).unwrap();
        let square = doc.layers()[0].id;
        let mut r = bare();
        {
            let p = &mut doc.find_mut(square).unwrap().props_mut().default;
            p.scale = 0.5;
            p.fill = Some([1.0, 1.0, 0.0]);
        }
        let img = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 22, 42), [255, 255, 0, 255], "yellow now, and smaller");
        assert_eq!(px(&img, 12, 50)[3], 0, "no longer reaches the corner");
        doc.find_mut(square).unwrap().props_mut().default.offset = [0.25, 0.0];
        let moved = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&moved, 56, 42), [255, 255, 0, 255], "moved right");
    }

    #[test]
    fn the_dark_and_tinted_looks_have_their_own_settings() {
        let mut doc = import_svg("t", ICON).unwrap();
        let square = doc.layers()[0].id;
        {
            let n = doc.find_mut(square).unwrap();
            let mut p = effective_props(n, Appearance::Dark);
            p.visible = false;
            set_node_props(n, Appearance::Dark, p);
        }
        let mut r = Renderer::new();
        assert_eq!(px(&r.render(&doc, Appearance::Default, 64), 12, 50), [255, 0, 0, 255]);
        // hidden in the dark look only: what is there is the dark plate, the same for every icon
        let dark = px(&r.render(&doc, Appearance::Dark, 64), 12, 50);
        assert!(dark[0] < 80 && dark[3] == 255, "the layer is gone and the dark base shows: {dark:?}");
        let other = import_svg("other", r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect width="64" height="64" fill="#00ff00"/></svg>"##).unwrap();
        let plate = |d: &IconDoc| background_for(d, Appearance::Dark);
        assert_eq!(plate(&doc).top, plate(&other).top, "the base of the dark icons is always the same");
        // the tinted looks paint everything in the tint colour (a purple here): plate and layers
        set_tint([0.42, 0.34, 0.80]);
        let tinted = px(&r.render(&doc, Appearance::TintedLight, 64), 12, 50);
        assert!(tinted[2] > tinted[1] + 20, "purple-ish plate: {tinted:?}");
        let clear = px(&r.render(&doc, Appearance::ClearLight, 64), 12, 50);
        assert!((clear[0] as i32 - clear[2] as i32).abs() < 12, "the clear look is grey: {clear:?}");
        // a look that was set keeps what it was given
        doc.background.default.fill = FillKind::Solid;
        set_background(&mut doc, Appearance::TintedDark, Background { shape: Shape::Circle, fill: FillKind::Solid, top: [1.0, 0.0, 0.0], bottom: [1.0, 0.0, 0.0], glass: Glass::default() });
        let own = px(&r.render(&doc, Appearance::TintedDark, 64), 32, 4);
        assert!(own[0] > 200 && own[1] < 60, "its own plate colour: {own:?}");
    }

    #[test]
    fn a_look_inherits_from_the_default_look_and_from_its_parent_look() {
        let mut doc = import_svg("t", ICON).unwrap();
        let circle = doc.layers()[1].id;
        let mut r = bare();
        // the circle is hidden in the default look afterwards: the dark look, which set nothing about it, follows
        doc.find_mut(circle).unwrap().props_mut().default.visible = false;
        assert_eq!(px(&r.render(&doc, Appearance::Dark, 64), 32, 32), [255, 0, 0, 255], "hidden in the dark look too");
        // set in the dark look, it stays so when the default look changes
        let n = doc.find_mut(circle).unwrap();
        let mut p = effective_props(n, Appearance::Dark);
        p.visible = true;
        set_node_props(n, Appearance::Dark, p);
        assert_eq!(px(&r.render(&doc, Appearance::Dark, 64), 32, 32), [0, 0, 255, 255], "shown in the dark look");
        assert_eq!(px(&r.render(&doc, Appearance::Default, 64), 32, 32), [255, 0, 0, 255], "still hidden in the default one");
    }

    #[test]
    fn a_layer_can_be_painted_with_a_gradient_and_have_a_bevel_and_a_glow() {
        let mut doc = import_svg("t", ICON).unwrap();
        let id = doc.layers()[0].id;
        let mut p = effective_props(doc.find(id).unwrap(), Appearance::Default);
        p.gradient = Some(Gradient { a: [0.0, 0.0, 1.0], b: [1.0, 0.0, 0.0], ..Gradient::default() });
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let img = bare().render(&doc, Appearance::Default, 64);
        let (top, bottom) = (px(&img, 20, 10), px(&img, 20, 54));
        assert!(top[2] > 150 && top[0] < 100, "blue at the top: {top:?}");
        assert!(bottom[0] > 150 && bottom[2] < 100, "red at the bottom: {bottom:?}");
        // a radial one is the first colour in the middle of the shape
        p.gradient = Some(Gradient { radial: true, ..p.gradient.unwrap() });
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let plain = bare().render(&doc, Appearance::Default, 64);
        assert!(px(&plain, 12, 50)[0] > 100 || px(&plain, 12, 50)[2] > 100, "painted");
        // bevel, inner shadow and glow change the picture
        for (name, glass) in [
            ("bevel", Glass { enabled: true, bevel: 1.0, ..Glass::default() }),
            ("inner shadow", Glass { enabled: true, inner_shadow: 1.0, ..Glass::default() }),
            ("glow", Glass { enabled: true, glow: 1.0, ..Glass::default() }),
        ] {
            let mut q = p;
            q.glass = Glass { specular: 0.0, shadow: ShadowKind::None, refraction: 0.0, ..glass };
            set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, q);
            let with = bare().render(&doc, Appearance::Default, 64);
            assert_ne!(with.as_raw(), plain.as_raw(), "{name} changes the picture");
        }
    }

    #[test]
    fn a_gradient_of_blobs_puts_each_colour_where_its_blob_is() {
        let mut doc = import_svg("t", ICON).unwrap();
        let id = doc.layers()[0].id;
        let mut p = effective_props(doc.find(id).unwrap(), Appearance::Default);
        let mut blobs = Blobs::default();
        blobs.spots[0] = Blob { at: [0.1, 0.5], size: 0.5, colour: [1.0, 0.0, 0.0] };
        blobs.spots[1] = Blob { at: [0.9, 0.5], size: 0.5, colour: [0.0, 1.0, 0.0] };
        blobs.count = 2;
        p.gradient = Some(Gradient { a: [0.0, 0.0, 1.0], blobs, ..Gradient::default() });
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let img = bare().render(&doc, Appearance::Default, 64);
        let (l, r) = (px(&img, 14, 32), px(&img, 50, 32));
        assert!(l[0] > l[1] && l[0] > 120, "red on the left: {l:?}");
        assert!(r[1] > r[0] && r[1] > 120, "green on the right: {r:?}");
    }

    #[test]
    fn flipping_and_turning_a_layer_moves_its_drawing() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect x="0" y="0" width="16" height="16" fill="#ff0000"/></svg>"##;
        let at = |s: &str, x: u32, y: u32| px(&bare().render(&import_svg("t", s).unwrap(), Appearance::Default, 64), x, y);
        assert_eq!(at(svg, 4, 4), [255, 0, 0, 255]);
        assert_eq!(at(&transform_svg(svg, SvgOp::FlipH).unwrap(), 60, 4), [255, 0, 0, 255]);
        assert_eq!(at(&transform_svg(svg, SvgOp::FlipV).unwrap(), 4, 60), [255, 0, 0, 255]);
        assert_eq!(at(&transform_svg(svg, SvgOp::RotateCw).unwrap(), 60, 4), [255, 0, 0, 255]);
        assert_eq!(at(&transform_svg(svg, SvgOp::RotateCcw).unwrap(), 4, 60), [255, 0, 0, 255]);
    }

    #[test]
    fn the_clear_look_can_keep_the_colours_of_the_layers() {
        let doc = import_svg("t", ICON).unwrap();
        let red_at = |colour: bool| {
            let mut r = bare();
            r.tune = GlassTune { colour_clear: colour, ..GlassTune::default() };
            px(&r.render(&doc, Appearance::ClearLight, 64), 12, 50)
        };
        let grey = red_at(false);
        let coloured = red_at(true);
        assert!((grey[0] as i32 - grey[2] as i32).abs() < 12, "the clear look is grey: {grey:?}");
        assert!(coloured[0] as i32 > coloured[1] as i32 + 60, "kept red: {coloured:?}");
    }

    #[test]
    fn the_live_version_of_a_clear_icon_has_no_plate_and_the_shell_knows_its_shape() {
        let doc = import_svg("t", ICON).unwrap();
        let mut r = Renderer::new();
        let plated = r.render(&doc, Appearance::ClearLight, 64);
        r.bare = true;
        let live = r.render(&doc, Appearance::ClearLight, 64);
        // (4, 32): on the plate, left of the layers
        assert!(px(&plated, 4, 32)[3] > 30, "the (see-through) plate is painted in: {:?}", px(&plated, 4, 32));
        assert!(px(&live, 4, 32)[3] < 10, "no plate in the live version (only a breath of the layers' shadow): {:?}", px(&live, 4, 32));
        assert!(px(&live, 32, 32)[3] > 100, "the layers are kept: {:?}", px(&live, 32, 32));
        let (side, radius) = live_plate(&GlassTune::default(), 100.0).unwrap();
        assert!((side - 100.0 * plate_fit()).abs() < 1e-3);
        assert!(radius < 0.0 && -radius > 10.0 && -radius < side / 2.0, "a squircle's continuous corner: {radius}");
        assert!(live_plate(&GlassTune { plate: Shape::None, ..GlassTune::default() }, 100.0).is_none());
    }

    #[test]
    fn sharpness_changes_how_crisp_the_edges_of_the_glass_are() {
        let mut doc = import_svg("t", ICON).unwrap();
        let id = doc.layers()[0].id;
        let mut p = effective_props(doc.find(id).unwrap(), Appearance::Default);
        p.glass = Glass { enabled: true, specular: 1.0, refraction: 1.0, ..Glass::default() };
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let draw = |sharp: f32| {
            let mut r = bare();
            r.tune = GlassTune { sharp, plate: Shape::None, ..GlassTune::default() };
            r.render(&doc, Appearance::Default, 128)
        };
        assert_ne!(draw(0.3).as_raw(), draw(2.0).as_raw(), "soft and crisp glass are not the same picture");
        assert_eq!(draw(GlassTune::default().sharp).as_raw(), bare().render(&doc, Appearance::Default, 128).as_raw(), "the reference sharpness is what the plain renderer uses");
    }

    #[test]
    fn a_picture_behind_the_glass_is_blurred_and_kept() {
        let doc = import_svg("t", ICON).unwrap();
        let mut r = bare();
        r.backdrop = Some(Arc::new(RgbaImage::from_pixel(64, 64, Rgba([10, 200, 30, 255]))));
        let img = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 2, 2), [10, 200, 30, 255], "the picture holds the backdrop where nothing is drawn");
        assert_eq!(px(&img, 12, 50), [255, 0, 0, 255], "and the layers are on it");
    }
}
