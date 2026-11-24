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

/// Where the time of drawing goes, in nanoseconds: the layers' SVGs, getting a render ready for the card, the card's work,
/// the processor's `composite`. Read by examples/icon_bench.
pub static PROF: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

fn prof(k: usize, since: std::time::Instant) {
    PROF[k].fetch_add(since.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
}

// ----------------------------------------------------------------------------------------------- pixel buffers

thread_local! {
    /// Planes that were let go on this thread, for the next that are asked for.
    static SPARE: std::cell::RefCell<Vec<Vec<f32>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// How much the spare planes of a thread may hold, in numbers (24 MB).
const SPARE_ROOM: usize = 6 << 20;

/// One plane of a picture: numbers row by row. A drawing takes and lets go of dozens of these for every layer, and room
/// of this size asked from the system each time comes as fresh pages, which costs more than the sums done in them (and
/// the threads of a theme build wait on each other for it): a plane let go is kept for the next one of its size.
#[derive(Default)]
pub struct Plane(Vec<f32>);

impl Plane {
    /// A plane of `len` numbers holding whatever was left there: for one that is written before it is read.
    fn any(len: usize) -> Plane {
        let kept = SPARE.with(|s| {
            let mut s = s.borrow_mut();
            s.iter().position(|v| v.len() == len).map(|k| s.swap_remove(k))
        });
        Plane(kept.unwrap_or_else(|| vec![0.0; len]))
    }

    fn filled(len: usize, v: f32) -> Plane {
        let mut p = Plane::any(len);
        p.0.fill(v);
        p
    }

    fn of(src: &[f32]) -> Plane {
        let mut p = Plane::any(src.len());
        p.0.copy_from_slice(src);
        p
    }
}

impl Clone for Plane {
    fn clone(&self) -> Plane {
        Plane::of(&self.0)
    }
}

impl std::ops::Deref for Plane {
    type Target = [f32];
    fn deref(&self) -> &[f32] {
        &self.0
    }
}

impl std::ops::DerefMut for Plane {
    fn deref_mut(&mut self) -> &mut [f32] {
        &mut self.0
    }
}

impl std::ops::Drop for Plane {
    fn drop(&mut self) {
        let v = std::mem::take(&mut self.0);
        if v.is_empty() || v.len() > SPARE_ROOM {
            return;
        }
        // (a thread that is ending has no room left to keep it in)
        let _ = SPARE.try_with(|s| {
            let mut s = s.borrow_mut();
            // room is made by letting planes of other sizes go first: the size being drawn now is the one wanted next
            let mut held: usize = s.iter().map(|o| o.len()).sum();
            while held + v.len() > SPARE_ROOM {
                let Some(k) = s.iter().position(|o| o.len() != v.len()) else { return };
                held -= s.swap_remove(k).len();
            }
            s.push(v);
        });
    }
}

/// A square picture of floats, premultiplied: planes r, g, b, a.
#[derive(Clone)]
pub struct Buf {
    pub n: usize,
    pub c: [Plane; 4],
}

impl Buf {
    pub fn new(n: usize) -> Buf {
        Buf { n, c: [0; 4].map(|_| Plane::filled(n * n, 0.0)) }
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
            // (nothing here is below zero, so adding a half and cutting rounds)
            let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 };
            *p = Rgba([un(self.c[0][i]), un(self.c[1][i]), un(self.c[2][i]), (a * 255.0 + 0.5) as u8]);
        }
        img
    }

    fn alpha(&self) -> &[f32] {
        &self.c[3]
    }

    /// `sample` of one plane.
    fn sample_plane(&self, k: usize, x: f32, y: f32) -> f32 {
        let n = self.n;
        let (x, y) = (x.clamp(0.0, n as f32 - 1.001), y.clamp(0.0, n as f32 - 1.001));
        let (x0, y0) = (x as usize, y as usize);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let (top, bottom) = (&self.c[k][y0 * n + x0..y0 * n + x0 + 2], &self.c[k][(y0 + 1) * n + x0..(y0 + 1) * n + x0 + 2]);
        top[0] * (1.0 - fx) * (1.0 - fy) + top[1] * fx * (1.0 - fy) + bottom[0] * (1.0 - fx) * fy + bottom[1] * fx * fy
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

/// A part of a picture: columns `x0..x1` of rows `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Rect {
    /// Wider by `d` pixels on every side, inside a picture of `n` pixels.
    fn grow(self, d: usize, n: usize) -> Rect {
        Rect { x0: self.x0.saturating_sub(d), y0: self.y0.saturating_sub(d), x1: (self.x1 + d).min(n), y1: (self.y1 + d).min(n) }
    }
}

/// The part of an `n` x `n` plane that holds everything above zero in it; None when nothing is.
fn bounds(plane: &[f32], n: usize) -> Option<Rect> {
    let filled = |y: usize| plane[y * n..(y + 1) * n].iter().any(|&v| v > 0.0);
    let y0 = (0..n).find(|&y| filled(y))?;
    let y1 = (y0..n).rev().find(|&y| filled(y))? + 1;
    let (mut x0, mut x1) = (n, 0);
    for y in y0..y1 {
        let row = &plane[y * n..(y + 1) * n];
        if let Some(a) = row[..x0].iter().position(|&v| v > 0.0) {
            x0 = a;
        }
        if let Some(b) = row[x1..].iter().rposition(|&v| v > 0.0) {
            x1 += b + 1;
        }
    }
    Some(Rect { x0, y0, x1, y1 })
}

/// A blur close to a Gaussian of deviation `sigma` is three passes of a box, across and down. This is the radius of that
/// box; None when there is nothing to blur.
fn box_radius(sigma: f32) -> Option<usize> {
    if sigma < 0.3 {
        return None;
    }
    let r = (((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().round() as usize).max(1) | 1) / 2;
    (r > 0).then_some(r)
}

/// How many lines the blur runs side by side.
const LANES: usize = 8;

/// The three box passes of a blur over `LANES` lines of `len` pixels at once. The lines lie side by side in `a` (pixel k of
/// line l at `(r + k) * LANES + l`), with `r` pixels of nothing before and after them, which stand for what is outside the
/// picture; `b` is as big, and the result is left there. The mean of a box is from a running sum, kept in f64: in f32 what
/// is added and taken off again leaves a residue that stays on the rest of the line (and on the rows after the shapes),
/// which shows as streaks in the shadows. A running sum waits for itself at every pixel, which is why several lines are
/// run at once; and all three passes are done here, on a few lines that stay in the processor's cache, instead of each
/// pass going over the whole plane.
#[inline(always)]
fn box3(a: &mut [f64], b: &mut [f64], len: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f64;
    let (mut src, mut dst) = (a, b);
    for pass in 0..3 {
        let mut sum = [0.0f64; LANES];
        for head in src[..2 * r * LANES].chunks_exact(LANES) {
            for l in 0..LANES {
                sum[l] += head[l];
            }
        }
        for i in 0..len {
            let add = &src[(i + 2 * r) * LANES..(i + 2 * r + 1) * LANES];
            for l in 0..LANES {
                sum[l] += add[l];
            }
            let out = &mut dst[(i + r) * LANES..(i + r + 1) * LANES];
            for l in 0..LANES {
                out[l] = (sum[l] * norm).max(0.0);
            }
            let sub = &src[i * LANES..(i + 1) * LANES];
            for l in 0..LANES {
                sum[l] -= sub[l];
            }
        }
        if pass < 2 {
            std::mem::swap(&mut src, &mut dst);
        }
    }
}

/// The blur of box radius `r` of the part `w` of a plane, in place, as if there were nothing round that part: all the
/// passes across first, then all the passes down (they are sums along one axis each, so the order between the axes does
/// not matter), the passes down only for the columns `cols`. `pad` is room to work in.
#[inline(always)]
fn blur_part(plane: &mut [f32], n: usize, r: usize, w: Rect, cols: (usize, usize), pad: &mut Vec<f64>) {
    let len = w.x1 - w.x0;
    let size = (len + 2 * r) * LANES;
    pad.clear();
    pad.resize(2 * size, 0.0);
    let (a, b) = pad.split_at_mut(size);
    let mut y = w.y0;
    while y < w.y1 {
        let rows = (w.y1 - y).min(LANES);
        for l in 0..rows {
            for (k, &v) in plane[(y + l) * n + w.x0..(y + l) * n + w.x1].iter().enumerate() {
                a[(r + k) * LANES + l] = v as f64;
            }
        }
        box3(a, b, len, r);
        for l in 0..rows {
            for (k, v) in plane[(y + l) * n + w.x0..(y + l) * n + w.x1].iter_mut().enumerate() {
                *v = b[(r + k) * LANES + l] as f32;
            }
        }
        y += rows;
    }
    let len = w.y1 - w.y0;
    let size = (len + 2 * r) * LANES;
    pad.clear();
    pad.resize(2 * size, 0.0);
    let (a, b) = pad.split_at_mut(size);
    let mut x = cols.0;
    while x < cols.1 {
        let lanes = (cols.1 - x).min(LANES);
        for k in 0..len {
            let (from, to) = (&plane[(w.y0 + k) * n + x..(w.y0 + k) * n + x + lanes], &mut a[(r + k) * LANES..(r + k) * LANES + lanes]);
            for l in 0..lanes {
                to[l] = from[l] as f64;
            }
        }
        box3(a, b, len, r);
        for k in 0..len {
            let (from, to) = (&b[(r + k) * LANES..(r + k) * LANES + lanes], &mut plane[(w.y0 + k) * n + x..(w.y0 + k) * n + x + lanes]);
            for l in 0..lanes {
                to[l] = from[l] as f32;
            }
        }
        x += lanes;
    }
}

/// The blur of a plane that is empty outside `filled`: only the part the blur reaches is worked on (the rest stays empty, as
/// the blur of nothing is). Returns that part.
#[inline(always)]
fn blur_grow(plane: &mut [f32], n: usize, sigma: f32, filled: Rect, room: &mut Vec<f64>) -> Rect {
    let Some(r) = box_radius(sigma) else { return filled };
    let reach = filled.grow(3 * r, n);
    blur_part(plane, n, r, reach, (reach.x0, reach.x1), room);
    reach
}

/// The blur of a plane of which only the part `need` is wanted afterwards: what is further from it than the three passes
/// reach is left out. That makes the edge of what is worked on wrong, a box further in with each pass, which is as far
/// as `need` begins; outside `need` the plane is left half blurred.
#[inline(always)]
fn blur_need(plane: &mut [f32], n: usize, sigma: f32, need: Rect, room: &mut Vec<f64>) {
    let Some(r) = box_radius(sigma) else { return };
    blur_part(plane, n, r, need.grow(3 * r, n), (need.x0, need.x1), room);
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

/// The picture `src` (any size, straight alpha) laid premultiplied on an empty `n` x `n` picture with its top left corner
/// at `at`.
fn place_at(src: &RgbaImage, n: usize, at: [i32; 2]) -> Buf {
    let mut out = Buf::new(n);
    let m = src.width() as i32;
    let raw = src.as_raw();
    let (x0, x1) = (at[0].max(0), (at[0] + m).min(n as i32));
    for y in at[1].max(0)..(at[1] + m).min(n as i32) {
        for x in x0..x1 {
            let p = &raw[((y - at[1]) * m + x - at[0]) as usize * 4..][..4];
            if p[3] == 0 {
                continue;
            }
            let (i, a) = (y as usize * n + x as usize, p[3] as f32 / 255.0);
            out.c[0][i] = p[0] as f32 / 255.0 * a;
            out.c[1][i] = p[1] as f32 / 255.0 * a;
            out.c[2][i] = p[2] as f32 / 255.0 * a;
            out.c[3][i] = a;
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
    /// draw on the processor whenever the graphics card is busy with another thread's picture, instead of waiting for it
    /// (the threads of a theme build: the card draws one picture at a time, and the cores would stand idle meanwhile)
    pub share_card: bool,
    /// draw the tinted looks in this colour instead of the shell's accent (the look that tints each icon by its own colour)
    pub tint_override: Option<[f32; 3]>,
    /// the tinted looks take the colour of each icon instead of the accent (drawn by `render`, which finds that colour)
    pub tint_by_icon: bool,
    /// the buffers on the graphics card, for each size drawn there
    gpu: HashMap<u32, gpu::Ctx>,
    /// the coverage of the plate's shape: (shape, size, corner)
    masks: HashMap<(u8, u32, u32), Arc<Vec<f32>>>,
    /// pictures of the plate alone (on nothing), as the processor draws them
    plates: Vec<(PlateKey, Arc<Buf>)>,
    /// the colour of the icon being drawn, when its plate is to be of that colour (`GlassTune::dress`)
    own: Option<[f32; 3]>,
    /// the icon being drawn is a folder or a file (`IconContext::is_object`): its layers are not dressed
    object: bool,
    /// the colours found of icons: by all that the layers of a document are
    colours: HashMap<u64, [f32; 3]>,
    /// the sizes a theme is written in now, when not all of them (`theme_size_steps`)
    sizes: Option<&'static [u32]>,
    /// the SVGs of layers as they were read (None: not an SVG that can be drawn)
    trees: HashMap<u64, Option<Arc<resvg::usvg::Tree>>>,
}

/// All that the picture of a plate on nothing comes from.
#[derive(Clone, Copy, PartialEq)]
struct PlateKey {
    n: u32,
    shape: Shape,
    corner: f32,
    filled: bool,
    top: [f32; 3],
    bottom: [f32; 3],
    glass: Glass,
    flat: bool,
    tune: GlassTune,
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
    /// how the symbols of the clear looks stand out of their glass (see `ClearStyle`)
    pub clear: ClearStyle,
    /// the default look dresses a plain icon as the tinted looks do: its layers that are not glass become glass
    /// (light on their edges, a shadow under them), and an icon without a plate of its own gets one in its own colour
    /// instead of the white or the grey one that is the same for every icon
    pub dress: bool,
    /// the glass edge of the phone's launcher (`edge_glow`), over the finished picture: a light rim along the plate's
    /// outline, strongest where it faces the light, and a faint bevel inside it (0 adds nothing)
    pub edge_glow: f32,
}

/// How a clear icon is made: its symbol and its plate. The colours of the artwork go in all but `Colour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClearStyle {
    /// pale frosted symbols on pale frost (the first clear look: soft, but it washes out on a light picture)
    #[default]
    Frost,
    /// bright white symbols with a shadow under them, on a smoky plate: reads on any picture (Apple's clear look)
    Contrast,
    /// dark ink symbols on a milky plate (the monochrome look of a light desktop)
    Ink,
    /// the symbols keep their colours on clear glass
    Colour,
    /// Frost over dark and coloured pictures, Contrast over light ones: as the picture behind an icon gets bright, its plate turns
    /// from pale frost to smoky glass and its symbol from a shade of the backdrop to white (decided by the shell's shader, see
    /// `auto_amount`). Drawn as Frost; it comes last so that the codes of the others, which the tuning file keeps, stay.
    Auto,
}

impl ClearStyle {
    pub const ALL: [ClearStyle; 5] = [ClearStyle::Frost, ClearStyle::Contrast, ClearStyle::Ink, ClearStyle::Colour, ClearStyle::Auto];

    pub fn label(self) -> &'static str {
        match self {
            ClearStyle::Frost => "Frost",
            ClearStyle::Contrast => "Contrast",
            ClearStyle::Ink => "Ink",
            ClearStyle::Colour => "Colour",
            ClearStyle::Auto => "Auto",
        }
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(c: u8) -> ClearStyle {
        ClearStyle::ALL.get(c as usize).copied().unwrap_or_default()
    }

    /// The symbols are coloured by what is behind the icon (by the shell's shader; `frost_ink` is the same on the processor).
    pub fn follows_backdrop(self) -> bool {
        matches!(self, ClearStyle::Frost | ClearStyle::Auto)
    }

    /// The plate tint over a backdrop of colour `back`: the style's own, and for Auto the one between Frost's and Contrast's that
    /// the backdrop's lightness asks for (the shader does the same on the screen).
    pub fn plate_tint_over(self, back: [f32; 3], dark: bool) -> [f32; 4] {
        if self != ClearStyle::Auto {
            return self.plate_tint(dark);
        }
        let (frost, smoky) = (ClearStyle::Frost.plate_tint(dark), ClearStyle::Contrast.plate_tint(dark));
        let k = auto_amount(back);
        [0, 1, 2, 3].map(|i| frost[i] + (smoky[i] - frost[i]) * k)
    }

    /// The tint of the live plate the shell draws under a clear icon (rgb + strength), light or dark look.
    pub fn plate_tint(self, dark: bool) -> [f32; 4] {
        match (self, dark) {
            (ClearStyle::Contrast, false) => [0.26, 0.27, 0.32, 0.34],
            (ClearStyle::Contrast, true) => [0.06, 0.06, 0.08, 0.55],
            (ClearStyle::Ink, false) => [1.0, 1.0, 1.0, 0.62],
            (ClearStyle::Ink, true) => [0.92, 0.93, 0.96, 0.50],
            // Frost: the backdrop shows through in its own colour, a little deeper, so the light symbol of that colour reads
            // (Auto starts from Frost's; over a light picture it moves to Contrast's, `plate_tint_over`)
            (ClearStyle::Frost | ClearStyle::Auto, false) => [0.40, 0.42, 0.48, 0.20],
            (ClearStyle::Frost | ClearStyle::Auto, true) => [0.06, 0.06, 0.08, 0.30],
            (_, false) => [0.97, 0.97, 1.0, 0.24],
            (_, true) => [0.10, 0.10, 0.12, 0.42],
        }
    }
}

/// The colour of a Frost symbol of lightness `lum` over a backdrop of colour `back` (as `glass.wgsl` draws it): a light
/// shade of the backdrop's hue, or a deep one in the dark look.
pub fn frost_ink(back: [f32; 3], lum: f32, dark: bool) -> [f32; 3] {
    let m = back[0].max(back[1]).max(back[2]).max(0.05);
    let n = back.map(|v| v / m);
    let g = 0.299 * n[0] + 0.587 * n[1] + 0.114 * n[2];
    let hue = n.map(|v| (g + (v - g) * 1.25).clamp(0.0, 1.0));
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    // (the symbol comes in the grey of the clear look, 0.5..1 light, 0.32..0.94 dark: its shades are spread out again)
    if dark {
        let l = ((lum - 0.32) / 0.62).clamp(0.0, 1.0);
        hue.map(|h| mix(h * 0.22, mix(h, 1.0, 0.45), l))
    } else {
        let l = ((lum - 0.5) / 0.5).clamp(0.0, 1.0);
        hue.map(|h| mix(h * 0.62, 1.0, 0.15 + 0.85 * l))
    }
}

/// How far the Auto style has gone from Frost (0) to Contrast (1) over a backdrop of colour `back`: by how light the picture is,
/// nothing below 0.55, all of it from 0.80 (`auto_amount` in src/glass.wgsl is the same).
pub fn auto_amount(back: [f32; 3]) -> f32 {
    let l = 0.299 * back[0] + 0.587 * back[1] + 0.114 * back[2];
    let t = ((l - 0.55) / 0.25).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The colour of an Auto symbol of lightness `lum` over a backdrop of colour `back` (as `glass.wgsl` draws it; `frost_ink` is the
/// Frost end of it): the Contrast end is white (0.80 to 1 by the artwork's own lightness), and what the picture holds darker than any
/// symbol, the shadow under it, stays dark.
pub fn auto_ink(back: [f32; 3], lum: f32, dark: bool) -> [f32; 3] {
    let frost = frost_ink(back, lum, dark);
    let k = auto_amount(back);
    if k <= 0.0 {
        return frost;
    }
    let (l, lo, hi) = if dark { (((lum - 0.32) / 0.62).clamp(0.0, 1.0), 0.12, 0.26) } else { (((lum - 0.5) / 0.5).clamp(0.0, 1.0), 0.30, 0.46) };
    let s = ((lum - lo) / (hi - lo)).clamp(0.0, 1.0);
    let s = s * s * (3.0 - 2.0 * s);
    let contrast = lum + (0.80 + 0.20 * l - lum) * s;
    frost.map(|f| f + (contrast - f) * k)
}

/// The clear style the looks are drawn in now (the property functions are plain functions the documents call, so the
/// tuning they follow is kept here; `Renderer` and `GlassTune::load` set it).
static CLEAR_STYLE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn clear_style() -> ClearStyle {
    ClearStyle::from_code(CLEAR_STYLE.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_clear_style(c: ClearStyle) {
    CLEAR_STYLE.store(c.code(), std::sync::atomic::Ordering::Relaxed);
}

impl GlassTune {
    /// the settings that change nothing: every factor 1, nothing added
    const NEUTRAL: GlassTune = GlassTune {
        specular: 1.0, blur: 1.0, refraction: 1.0, translucency: 1.0, shadow: 1.0, light: 231.5, shadow_dist: 1.0, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, colour_clear: false,
        bevel: 0.0, bevel_width: 1.0, inner_shadow: 0.0, glow: 0.0, rim: 1.0, sheen: 1.0, edge_line: 1.0, edge_width: 1.0, edge_sharp: 1.0, lens_width: 1.0, dispersion: 1.0, inner_bend: 1.0, soft_light: 1.0, plate: Shape::Squircle, clear: ClearStyle::Frost, dress: true, edge_glow: 0.0,
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
    // the user liked the phone's glass edge better than the edge line alone (2026-11-24)
    edge_glow: 1.0,
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
        let colour_clear = at(10, 0.0) > 0.5;
        // (a file from before the clear styles has only "keeps colours": that is the Colour style)
        let clear = match v.get(25) {
            Some(c) => ClearStyle::from_code(*c as u8),
            None if colour_clear => ClearStyle::Colour,
            None => ClearStyle::Frost,
        };
        set_clear_style(clear);
        GlassTune {
            clear,
            dress: at(26, 1.0) > 0.5,
            edge_glow: at(27, t.edge_glow),
            specular: at(0, t.specular), blur: at(1, t.blur), refraction: at(2, t.refraction), translucency: at(3, t.translucency), shadow: at(4, t.shadow),
            light: at(5, t.light), shadow_dist: at(6, t.shadow_dist), shadow_soft: at(7, t.shadow_soft), corner: at(8, t.corner), sharp: at(9, t.sharp),
            colour_clear: clear == ClearStyle::Colour,
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
            self.clear.code() as f32, self.dress as u8 as f32, self.edge_glow,
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

    /// The direction from the glass towards the light.
    fn light_toward(&self) -> (f32, f32) {
        let (x, y) = self.light_dir();
        (-x, -y)
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
    /// The clear style drawn: the tuning's ("keeps colours" set in an older way is the Colour style).
    fn clear(&self) -> ClearStyle {
        if self.tune.colour_clear {
            ClearStyle::Colour
        } else {
            self.tune.clear
        }
    }

    fn tint(&self) -> [f32; 3] {
        self.tint_override.unwrap_or_else(tint)
    }

    /// The coverage of the plate's shape in a picture of `n` pixels, worked out once for each size.
    fn mask(&mut self, shape: Shape, n: u32) -> Arc<Vec<f32>> {
        let corner = self.tune.corner;
        self.masks.entry((shape.code(), n, corner.to_bits())).or_insert_with(|| Arc::new(shape_mask(shape, n as usize, n as f32 * plate_fit() / 2.0, corner))).clone()
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
        let key = hash_of(&[&|h| format!("{:?}", doc.nodes).hash(h)]);
        if let Some(c) = self.colours.get(&key) {
            return *c;
        }
        let c = self.find_colour(doc);
        if self.colours.len() > 64 {
            self.colours.clear();
        }
        self.colours.insert(key, c);
        c
    }

    fn find_colour(&mut self, doc: &IconDoc) -> [f32; 3] {
        let (bare, over, own) = (self.bare, self.tint_override.take(), self.own.take());
        self.bare = true;
        let img = self.render_as(doc, Appearance::Default, 64);
        self.bare = bare;
        self.tint_override = over;
        self.own = own;
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
        let style = self.clear();
        set_clear_style(style);
        let treated = (app.is_clear() && style != ClearStyle::Colour) || app.is_tinted();
        let key_in = |look: Option<Appearance>| {
            hash_of(&[
                &|h| layer.svg.hash(h),
                &|h| props.fill.map(|c| c.map(|v| v.to_bits())).hash(h),
                &|h| gradient_bits(&props.gradient).hash(h),
                &|h| (look.map(|a| a.label()), if look.is_some_and(|a| a.is_tinted()) { self.tint().map(|v| v.to_bits()) } else { [0; 3] }, if look.is_some_and(|a| a.is_clear()) { style.code() } else { 0 }).hash(h),
            ])
        };
        let key = key_in(treated.then_some(app));
        if let Some(img) = self.rasters.get(&(key, n)) {
            return img.clone();
        }
        let since = std::time::Instant::now();
        // the drawing before it is recoloured is the same in every look: it is drawn once for them all
        let plain_key = key_in(None);
        let plain = self.rasters.get(&(plain_key, n)).cloned().ok_or(plain_key);
        let plain = match plain {
            Ok(img) => img,
            Err(plain_key) => {
                let mut img = self.tree(layer, props).and_then(|t| svgedit::rasterize_tree(&t, n)).unwrap_or_else(|| RgbaImage::new(n, n));
                if let Some(g) = props.gradient {
                    paint_gradient(&mut img, &g);
                }
                let img = Arc::new(img);
                self.keep(plain_key, n, img.clone());
                img
            }
        };
        if !treated {
            prof(0, since);
            return plain;
        }
        let mut img = (*plain).clone();
        // (read once: it is behind a lock, and every thread drawing at once would queue on it for each pixel)
        let tint = self.tint();
        for p in img.pixels_mut().filter(|p| p[3] != 0) {
            let c = treat([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0], app, tint, style);
            *p = Rgba([(c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8, p[3]]);
        }
        let img = Arc::new(img);
        self.keep(key, n, img.clone());
        prof(0, since);
        img
    }

    fn keep(&mut self, key: u64, n: u32, img: Arc<RgbaImage>) {
        if self.rasters.len() > 900 {
            self.rasters.clear();
        }
        self.rasters.insert((key, n), img);
    }

    /// The layer's SVG read (with the layer's one colour put in), kept for the other sizes and looks it is drawn in.
    fn tree(&mut self, layer: &Layer, props: &Props) -> Option<Arc<resvg::usvg::Tree>> {
        let key = hash_of(&[&|h| layer.svg.hash(h), &|h| props.fill.map(|c| c.map(|v| v.to_bits())).hash(h)]);
        if let Some(t) = self.trees.get(&key) {
            return t.clone();
        }
        let mut text = std::borrow::Cow::Borrowed(layer.svg.as_str());
        if let Some(fill) = props.fill {
            if let Some(d) = SvgDoc::from_text(&layer.svg) {
                let el = d.elements();
                let (has_fill, has_stroke) = el.first().map_or((true, false), |e| (e.fill.is_some(), e.stroke.is_some()));
                text = d.render_text(&[ElementEdit { index: 0, hidden: false, fill: has_fill.then_some(fill), stroke: has_stroke.then_some(fill) }], None).into();
            }
        }
        let tree = svgedit::parse_tree(&text).map(Arc::new);
        if self.trees.len() > 300 {
            self.trees.clear();
        }
        self.trees.insert(key, tree.clone());
        tree
    }

    /// Forget the drawings kept of the layers (those of a document that will not be drawn again).
    pub fn forget_layers(&mut self) {
        self.colours.clear();
        self.rasters.clear();
        self.trees.clear();
    }

    /// The plate as it is drawn: the document's own, except that the shape is the global one, and that the default look of an
    /// icon without a plate gets a light one (so that it is not the only look left bare).
    pub fn plate_for(&self, doc: &IconDoc, app: Appearance) -> Background {
        set_clear_style(self.clear());
        let mut b = doc.background.resolve(app, &auto_background(self.tint()));
        // (a folder or a file is the object itself: no plate in any look)
        if self.tune.plate == Shape::None || doc.context.is_object() {
            return Background { shape: Shape::None, fill: FillKind::None, glass: Glass { enabled: false, ..b.glass }, ..b };
        }
        if app == Appearance::Default && b.fill == FillKind::None {
            b = Background { fill: FillKind::System, glass: Glass { enabled: true, specular: 0.5, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() }, ..b };
        }
        // a plate nobody set, dressed: the icon's own colour, pale (the icon's colours are of that hue, and would be lost on
        // it at full strength)
        if let Some(c) = self.own.filter(|_| Self::plate_is_plain(doc, app)) {
            let (top, bottom) = (c.map(|v| mix(v, 1.0, 0.80)), c.map(|v| mix(v, 1.0, 0.62)));
            b = Background { fill: FillKind::Gradient, top, bottom, glass: Glass { specular: 0.6, ..b.glass }, ..b };
        }
        Background { shape: self.tune.plate, ..b }
    }

    /// Whether the plate of `doc` in look `app` is the one every icon gets (so `GlassTune::dress` may colour it). The dark
    /// look is left as it was: its one dark base for every icon is liked as it is.
    fn plate_is_plain(doc: &IconDoc, app: Appearance) -> bool {
        app == Appearance::Default && doc.background.default.fill == FillKind::None && !doc.context.is_object()
    }

    /// The glass of a layer as it is drawn: one that is not glass is made glass in the looks that dress plain icons.
    fn dressed(&self, g: Glass, app: Appearance) -> Glass {
        if self.tune.dress && !self.bare && !self.object && !g.enabled && app == Appearance::Default {
            Glass { enabled: true, specular: 0.6, refraction: 0.4, shadow: ShadowKind::Neutral, shadow_amount: 0.3, ..Glass::default() }
        } else {
            g
        }
    }

    /// The document as a picture of `n` x `n` pixels in the look `app`.
    pub fn render(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        // a look drawn from another look is that look's picture (the tinted-by-icon look asked for by the caller is not)
        if !self.tint_by_icon {
            let (from, by_icon) = doc.source(app);
            if by_icon || from != app {
                self.tint_by_icon = by_icon;
                let img = self.render_as(doc, from, n);
                self.tint_by_icon = false;
                return img;
            }
        }
        self.render_as(doc, app, n)
    }

    /// The plate and the settings of every layer and group as look `app` really draws them (`GlassTune::dress` and the
    /// automatic glass of the clear and tinted looks included), for putting into another look so that it draws the same
    /// without following this one. What the look does to the colours of the layers when it draws them (the clear and tinted
    /// looks recolour) is not a setting and does not come along.
    pub fn drawn_settings(&mut self, doc: &IconDoc, app: Appearance) -> (Background, Vec<(u32, Props)>) {
        self.own = None;
        self.object = doc.context.is_object();
        if self.tune.dress && !self.bare && Self::plate_is_plain(doc, app) {
            self.own = Some(self.icon_colour(doc));
        }
        let mut bg = self.plate_for(doc, app);
        self.own = None;
        // (the plate's colours as they are drawn: the system's and the automatic fills are made of the look)
        let (top, bottom) = match bg.fill {
            FillKind::Solid => (bg.top, bg.top),
            FillKind::System if app.is_dark() => (DARK_BASE_TOP, DARK_BASE_BOTTOM),
            FillKind::System => ([1.0, 1.0, 1.0], [0.90, 0.91, 0.94]),
            FillKind::Auto => (bg.top.map(|v| v + (1.0 - v) * 0.22), bg.top.map(|v| v * 0.92)),
            _ => (bg.top, bg.bottom),
        };
        let fill = match bg.fill {
            FillKind::System | FillKind::Auto => FillKind::Gradient,
            f => f,
        };
        // (the shape is the global one, which is never kept in a document: the document's own stays)
        bg = Background { shape: background_for(doc, app).shape, fill, top, bottom, ..bg };
        let mut props = Vec::new();
        fn walk(r: &Renderer, nodes: &[Node], app: Appearance, out: &mut Vec<(u32, Props)>) {
            for n in nodes {
                let mut p = effective_props(n, app);
                if let Kind::Layer(_) = n.kind {
                    p.glass = r.dressed(p.glass, app);
                }
                out.push((n.id, p));
                walk(r, n.children(), app, out);
            }
        }
        walk(self, &doc.nodes, app, &mut props);
        (bg, props)
    }

    /// `render` of the look itself, whatever it is drawn from.
    fn render_as(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        self.own = None;
        self.object = doc.context.is_object();
        if self.tune.dress && !self.bare && Self::plate_is_plain(doc, app) {
            self.own = Some(self.icon_colour(doc));
        }
        if self.tint_by_icon && app.is_tinted() && self.tint_override.is_none() {
            let c = self.icon_colour(doc);
            self.tint_override = Some(c);
            let img = self.render_as(doc, app, n);
            self.tint_override = None;
            return img;
        }
        // (the tests check the sums of the processor's version)
        let mut img = None;
        if !self.cpu && !cfg!(test) {
            img = gpu::render(self, doc, app, n);
        }
        let mut img = match img {
            Some(img) => img,
            None => self.render_cpu(doc, app, n),
        };
        // the glass edge goes round the plate (not round the picture's alpha: the shadow under the plate would get it too)
        if self.tune.edge_glow > 0.0 && !self.bare && !self.flat {
            let bg = self.plate_for(doc, app);
            if bg.shape != Shape::None && bg.fill != FillKind::None {
                let mask = self.mask(bg.shape, n);
                edge_glow(&mut img, &mask, self.tune.edge_glow, self.tune.light_toward());
            }
        }
        img
    }

    /// `render` on the processor.
    pub fn render_cpu(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        let nn = n as usize;
        let fit = plate_fit();
        let bg = self.plate_for(doc, app);
        let mask = (bg.shape != Shape::None).then(|| self.mask(bg.shape, n));
        let mask = mask.as_deref().map(|m| &m[..]);
        let mut accum = match &self.backdrop {
            Some(_) if self.bare => Buf::new(nn),
            Some(bd) => {
                let img = if bd.width() == n { (**bd).clone() } else { image::imageops::resize(&**bd, n, n, image::imageops::FilterType::Triangle) };
                Buf::from_image(&img)
            }
            None => Buf::new(nn),
        };

        // the plate
        let (ptop, pbottom) = match bg.fill {
            FillKind::Solid => (bg.top, bg.top),
            FillKind::System if app.is_dark() => (DARK_BASE_TOP, DARK_BASE_BOTTOM),
            FillKind::System => ([1.0, 1.0, 1.0], [0.90, 0.91, 0.94]),
            FillKind::Auto => (bg.top.map(|v| v + (1.0 - v) * 0.22), bg.top.map(|v| v * 0.92)),
            _ => (bg.top, bg.bottom),
        };
        // the plate on nothing is the same picture for every icon of a look (but for the plates set by hand): it is kept
        let plate_key = (self.backdrop.is_none() && !self.bare).then(|| PlateKey { n, shape: bg.shape, corner: self.tune.corner, filled: bg.fill != FillKind::None, top: ptop, bottom: pbottom, glass: bg.glass, flat: self.flat, tune: self.tune });
        if let Some(kept) = plate_key.as_ref().and_then(|k| self.plates.iter().find(|(have, _)| have == k)) {
            let accum = (*kept.1).clone();
            return self.draw_nodes(doc, app, accum, mask);
        }
        let mut plate = Buf::new(nn);
        if bg.fill != FillKind::None && !self.bare {
            let top = n as f32 * (1.0 - fit) / 2.0;
            for y in 0..nn {
                let t = ((y as f32 + 0.5 - top) / (n as f32 * fit)).clamp(0.0, 1.0);
                let c = [mix(ptop[0], pbottom[0], t), mix(ptop[1], pbottom[1], t), mix(ptop[2], pbottom[2], t)];
                for x in 0..nn {
                    let a = mask.map_or(1.0, |m| m[y * nn + x]);
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
            composite(&mut accum, plate, &Props { glass: plate_glass, ..Props::default() }, None, &self.tune);
        }
        if let Some(key) = plate_key {
            if self.plates.len() >= 40 {
                self.plates.clear();
            }
            self.plates.push((key, Arc::new(accum.clone())));
        }
        self.draw_nodes(doc, app, accum, mask)
    }

    fn draw_nodes(&mut self, doc: &IconDoc, app: Appearance, mut accum: Buf, mask: Option<&[f32]>) -> RgbaImage {
        let fit = plate_fit();

        for node in &doc.nodes {
            self.draw_node(node, doc, app, &mut accum, Place { scale: fit, off: [0.0, 0.0] }, mask);
        }
        accum.to_image()
    }

    fn draw_node(&mut self, node: &Node, doc: &IconDoc, app: Appearance, accum: &mut Buf, place: Place, mask: Option<&[f32]>) {
        let mut p = effective_props(node, app);
        if !p.visible {
            return;
        }
        if let Kind::Layer(_) = node.kind {
            p.glass = self.dressed(p.glass, app);
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
                let off = [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]];
                let moved = place_at(&img, n, [(nf / 2.0 + off[0] - m as f32 / 2.0).round() as i32, (nf / 2.0 + off[1] - m as f32 / 2.0).round() as i32]);
                composite(accum, moved, &p, mask, &self.tune);
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
                composite(accum, moved, &own, mask, &self.tune);
            }
        }
    }

    /// The scale and shift (as fractions of the icon) that put layer `id` where it is drawn, and whether it is shown.
    pub fn placement(doc: &IconDoc, app: Appearance, id: u32) -> Option<(f32, [f32; 2], bool)> {
        let app = doc.source(app).0;
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
        let app = doc.source(app).0;
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
        let app = doc.source(app).0;
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

/// A gradient as something to hash.
fn gradient_bits(g: &Option<Gradient>) -> Option<([u32; 3], [u32; 3], u32, bool, Vec<([u32; 2], u32, [u32; 3])>)> {
    g.map(|g| (g.a.map(f32::to_bits), g.b.map(f32::to_bits), g.angle.to_bits(), g.radial, g.blobs.used().iter().map(|b| (b.at.map(f32::to_bits), b.size.to_bits(), b.colour.map(f32::to_bits))).collect::<Vec<_>>()))
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

/// How far the translucency of a layer fades it down its height: at 1.0 the bottom row is fully see-through (1.6: the
/// average layer is about as see-through as it was when the translucency was one even amount, 0.85 of the setting).
pub const TRANSLUCENT_FADE: f32 = 1.6;

/// Below this luminance the light tinted look makes the artwork darker than the plate (see `treat`).
const SHARP_BELOW: f32 = 0.3;

/// The sharper dark parts of the light tinted look (on; the example that compares the two switches it).
pub static SHARP_TINT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// How a colour of the artwork is made in the clear and tinted looks: light and shade are kept, the colour is not.
fn treat(c: [f32; 3], app: Appearance, tint: [f32; 3], style: ClearStyle) -> [f32; 3] {
    let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    match (app, style) {
        // white symbols, the artwork's shade kept as a light grey modelling
        (Appearance::ClearLight | Appearance::ClearDark, ClearStyle::Contrast) => [0.80 + 0.20 * l; 3],
        // ink: dark symbols (light parts of the artwork a little lighter)
        (Appearance::ClearLight, ClearStyle::Ink) => [0.10 + 0.32 * l; 3],
        (Appearance::ClearDark, ClearStyle::Ink) => [0.14 + 0.30 * l; 3],
        (Appearance::ClearLight, _) => [0.50 + 0.50 * l; 3],
        (Appearance::ClearDark, _) => [0.32 + 0.62 * l; 3],
        _ => match app {
        Appearance::TintedLight => {
            let f = 0.35 + 0.65 * l;
            let lit = [mix(tint[0], 1.0, f), mix(tint[1], 1.0, f), mix(tint[2], 1.0, f)];
            // the darkest parts of the artwork (outlines, ink) go deeper than the plate instead of staying pale on it, so a
            // picture with detail keeps its drawing; white and light symbols are not touched
            if l < SHARP_BELOW && SHARP_TINT.load(std::sync::atomic::Ordering::Relaxed) {
                let at = 0.35 + 0.65 * SHARP_BELOW;
                let edge = [mix(tint[0], 1.0, at), mix(tint[1], 1.0, at), mix(tint[2], 1.0, at)];
                let deep = [tint[0] * 0.42, tint[1] * 0.42, tint[2] * 0.42];
                let k = l / SHARP_BELOW;
                return [0, 1, 2].map(|i| mix(deep[i], edge[i], k));
            }
            lit
        }
        Appearance::TintedDark => [tint[0] * (0.42 + 0.55 * l), tint[1] * (0.42 + 0.55 * l), tint[2] * (0.42 + 0.55 * l)],
        _ => c,
        },
    }
}

/// The glass a layer gets in the clear and tinted looks when it was not set there.
fn auto_glass(app: Appearance) -> Glass {
    let base = Glass { enabled: true, specular: 0.75, blur: if app.is_clear() { 0.4 } else { 0.0 }, refraction: 0.5, translucency: if app.is_clear() { 0.3 } else { 0.2 }, shadow: ShadowKind::Neutral, shadow_amount: 0.3, ..Glass::default() };
    if !app.is_clear() {
        return base;
    }
    match clear_style() {
        // solid enough to read: little see-through, a firmer shadow lifting it off the plate
        ClearStyle::Contrast => Glass { specular: 0.9, blur: 0.0, translucency: 0.05, shadow_amount: 0.6, ..base },
        ClearStyle::Ink => Glass { specular: 0.35, blur: 0.0, translucency: 0.0, shadow_amount: 0.12, ..base },
        ClearStyle::Colour => Glass { translucency: 0.1, blur: 0.1, ..base },
        ClearStyle::Frost | ClearStyle::Auto => base,
    }
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
            Appearance::ClearLight | Appearance::ClearDark => {
                let dark = app == Appearance::ClearDark;
                let (top, bottom) = match (clear_style(), dark) {
                    (ClearStyle::Contrast, false) => ([0.40; 3], [0.30; 3]),
                    (ClearStyle::Contrast, true) => ([0.16; 3], [0.08; 3]),
                    (ClearStyle::Ink, false) => ([0.97; 3], [0.88; 3]),
                    (ClearStyle::Ink, true) => ([0.86; 3], [0.76; 3]),
                    (_, false) => ([0.66; 3], [0.52; 3]),
                    (_, true) => ([0.24; 3], [0.15; 3]),
                };
                Background { shape, fill: FillKind::Gradient, top, bottom, glass: clear(if dark { 0.6 } else { 0.75 }) }
            }
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
fn composite(accum: &mut Buf, layer: Buf, p: &Props, mask: Option<&[f32]>, tune: &GlassTune) {
    let since = std::time::Instant::now();
    // the same sums, in wider registers where the processor has them (the build is for any x86-64)
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        #[target_feature(enable = "avx2")]
        unsafe fn wide(accum: &mut Buf, layer: Buf, p: &Props, mask: Option<&[f32]>, tune: &GlassTune) {
            composite_any(accum, layer, p, mask, tune)
        }
        // (safe: the processor was just asked for what `wide` is built for)
        unsafe { wide(accum, layer, p, mask, tune) };
        return prof(3, since);
    }
    composite_any(accum, layer, p, mask, tune);
    prof(3, since);
}

#[inline(always)]
fn composite_any(accum: &mut Buf, mut layer: Buf, p: &Props, mask: Option<&[f32]>, tune: &GlassTune) {
    let n = accum.n;
    // everything below is worked out only where the layer is, or as far from it as its blurs reach: a layer is seldom the
    // size of the icon, and all the rest of the picture stays as it is (an empty layer changes nothing at all)
    let Some(bb) = bounds(&layer.c[3], n) else { return };
    if let Some(m) = mask {
        for y in bb.y0..bb.y1 {
            for i in y * n + bb.x0..y * n + bb.x1 {
                for k in 0..4 {
                    layer.c[k][i] *= m[i];
                }
            }
        }
    }
    let room = &mut Vec::new();
    let g = p.glass;
    let nf = n as f32;
    let alpha = layer.c[3].clone();
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
    // (these are written wherever they are read: within a pixel of what the blurred shape reaches)
    let (mut nrm_x, mut nrm_y, mut slope) = (Plane::default(), Plane::default(), Plane::default());
    if glass && (g.specular > 0.0 || g.refraction > 0.0 || g.inner_shadow > 0.0) {
        let mut hill = alpha.clone();
        let reach = blur_grow(&mut hill, n, bezel * 0.45, bb, room).grow(1, n);
        (nrm_x, nrm_y, slope) = (Plane::any(n * n), Plane::any(n * n), Plane::any(n * n));
        let at = |x: usize, y: usize| hill[y.min(n - 1) * n + x.min(n - 1)];
        let mut peak = 1e-6f32;
        for y in reach.y0..reach.y1 {
            for x in reach.x0..reach.x1 {
                // (the hill rises inwards: the outward normal is minus its gradient)
                let (gx, gy) = (at(x + 1, y) - at(x.saturating_sub(1), y), at(x, y + 1) - at(x, y.saturating_sub(1)));
                // (the plain root, as the card takes it: `hypot` is a slow call that guards against sizes no slope has)
                let l = (gx * gx + gy * gy).sqrt();
                (nrm_x[y * n + x], nrm_y[y * n + x]) = if l > 1e-7 { (-gx / l, -gy / l) } else { (0.0, -1.0) };
                slope[y * n + x] = l;
                peak = peak.max(l);
            }
        }
        for y in reach.y0..reach.y1 {
            for v in &mut slope[y * n + reach.x0..y * n + reach.x1] {
                *v /= peak;
            }
        }
    }
    // the rounded edge of a bevel: as wide as it is asked to be
    let (mut bev_x, mut bev_y, mut bev_peak) = (Plane::default(), Plane::default(), 1e-4f32);
    if glass && g.bevel > 0.0 {
        let mut height = alpha.clone();
        let reach = blur_grow(&mut height, n, nf * (0.008 + 0.06 * g.bevel_width.clamp(0.0, 1.0)) / sharp, bb, room).grow(1, n);
        (bev_x, bev_y) = (Plane::any(n * n), Plane::any(n * n));
        let at = |x: usize, y: usize| height[y.min(n - 1) * n + x.min(n - 1)];
        for y in reach.y0..reach.y1 {
            for x in reach.x0..reach.x1 {
                let gx = (at(x + 1, y) - at(x.saturating_sub(1), y)) * 0.5;
                let gy = (at(x, y + 1) - at(x, y.saturating_sub(1))) * 0.5;
                bev_peak = bev_peak.max((gx * gx + gy * gy).sqrt());
                (bev_x[y * n + x], bev_y[y * n + x]) = (gx, gy);
            }
        }
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
        let reach = blur_grow(&mut s, n, nf * 0.05, bb, room);
        let mut colour = [layer.c[0].clone(), layer.c[1].clone(), layer.c[2].clone()];
        for k in 0..3 {
            blur_grow(&mut colour[k], n, nf * 0.05, bb, room);
        }
        for i in (reach.y0..reach.y1).flat_map(|y| y * n + reach.x0..y * n + reach.x1) {
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
        let reach = blur_grow(&mut s, n, nf * 0.03 * tune.shadow_soft, bb, room);
        let (tint, mut colour) = (g.shadow == ShadowKind::Chromatic, [Plane::default(), Plane::default(), Plane::default()]);
        if tint {
            for k in 0..3 {
                colour[k] = layer.c[k].clone();
                blur_grow(&mut colour[k], n, nf * 0.03 * tune.shadow_soft, bb, room);
            }
        }
        let dy = (nf * 0.025 * tune.shadow_dist).round() as usize;
        for y in reach.y0 + dy..(reach.y1 + dy).min(n) {
            for x in reach.x0..reach.x1 {
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
    // (there may be nothing behind yet: the first layer of a picture without a plate, the plate itself)
    let behind_empty = glass && (g.blur > 0.0 || g.refraction > 0.0) && bounds(&accum.c[3], n).is_none() && accum.c[..3].iter().all(|c| c.iter().all(|&v| v == 0.0));
    if glass && g.blur > 0.0 && !behind_empty {
        let mut frosted = accum.clone();
        for k in 0..4 {
            blur_need(&mut frosted.c[k], n, nf * 0.045 * g.blur, bb, room);
        }
        for i in (bb.y0..bb.y1).flat_map(|y| y * n + bb.x0..y * n + bb.x1) {
            let w = alpha[i].clamp(0.0, 1.0);
            if w > 0.0 {
                for k in 0..4 {
                    accum.c[k][i] = mix(accum.c[k][i], frosted.c[k][i], w);
                }
            }
        }
    }
    if glass && g.refraction > 0.0 && !slope.is_empty() && !behind_empty {
        let behind = accum.clone();
        let bend = nf * 0.11 * g.refraction;
        for y in bb.y0..bb.y1 {
            for x in bb.x0..bb.x1 {
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
                let (nx, ny) = (-nrm_x[i], -nrm_y[i]);
                // (each colour is read only from its own plane)
                let at = |k: usize, f: f32| behind.sample_plane(k, x as f32 + nx * bend * k2 * f, y as f32 + ny * bend * k2 * f);
                let v = [at(0, 1.0 + 0.1 * tune.dispersion), at(1, 1.0), at(2, 1.0 - 0.1 * tune.dispersion), at(3, 1.0)];
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
        for y in bb.y0..bb.y1 {
            for x in bb.x0..bb.x1 {
                let i = y * n + x;
                let la = own.c[3][i];
                if la <= 0.0 {
                    continue;
                }
                let k2 = bend_at(i);
                if k2 <= 0.0 {
                    continue;
                }
                let v = own.sample(x as f32 - nrm_x[i] * bend * k2, y as f32 - nrm_y[i] * bend * k2);
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
    // (the translucency is a gradient down the layer, as in Apple's icons: opaque at the top, see-through at the bottom)
    let fade = if glass { TRANSLUCENT_FADE * g.translucency } else { 0.0 };
    let (top, span) = (bb.y0 as f32, ((bb.y1 - bb.y0) as f32).max(1.0));
    for y in bb.y0..bb.y1 {
        let see_through = (1.0 - fade * ((y as f32 + 0.5 - top) / span).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        for i in y * n + bb.x0..y * n + bb.x1 {
            let la = layer.c[3][i];
            if la <= 0.0 {
                continue;
            }
            let cs = [layer.c[0][i] / la, layer.c[1][i] / la, layer.c[2][i] / la];
            put(accum, i, cs, (la * p.opacity * see_through).clamp(0.0, 1.0), p.blend);
        }
    }

    // the bevel: the edge turned to the light is lighter, the one turned away darker; and a shade inside along the edge
    if glass && (g.bevel > 0.0 || g.inner_shadow > 0.0) {
        let (lx, ly) = tune.light_dir();
        // (the shade along the edge from the blurred shape too, so that it is round in the corners)
        let mut inner = Plane::default();
        if g.inner_shadow > 0.0 {
            inner = alpha.clone();
            blur_grow(&mut inner, n, 0.04 * nf * tune.lens_width.clamp(0.2, 3.0), bb, room);
        }
        for i in (bb.y0..bb.y1).flat_map(|y| y * n + bb.x0..y * n + bb.x1) {
            let a = alpha[i];
            if a <= 0.0 {
                continue;
            }
            let da = accum.c[3][i];
            if g.bevel > 0.0 && !bev_x.is_empty() {
                let (nx, ny) = (bev_x[i] / bev_peak, bev_y[i] / bev_peak);
                let edge = (nx * nx + ny * ny).sqrt().clamp(0.0, 1.0);
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
        blur_grow(&mut near, n, 10.0 * unit.max(0.25), bb, room);
        // the fine line on the edge from the shape blurred a little, so that it is smooth along a curve (antialiased)
        let mut line = alpha.clone();
        blur_grow(&mut line, n, ((1.6 + (0.55 - 1.6) * sharp01) * unit * tune.edge_width).max(0.6), bb, room);
        for i in (bb.y0..bb.y1).flat_map(|y| y * n + bb.x0..y * n + bb.x1) {
            let a = alpha[i];
            if a <= 0.0 {
                continue;
            }
            // (no pixel is skipped for being deep inside: the sheen lies over the whole glass, and a cut by the distance to
            // the edge showed as a square in the middle of the plate)
            let (nx, ny) = (nrm_x[i], nrm_y[i]);
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

/// The sizes in the two steps of a redrawing the shell waits for (a new accent, another clear style): first those the
/// shell shows nearly everywhere (its icons are 128 pixels and smaller ones made of that), which is a quarter of the work,
/// then the big one, which only icons drawn larger than that use. The shell is told after each step.
fn theme_size_steps() -> [&'static [u32]; 2] {
    let (quick, big) = THEME_SIZES.split_at(3);
    [quick, big]
}

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
    let each = || IconContext::ALL.iter().flat_map(|c| THEME_SIZES.iter().map(move |s| (*c, *s)));
    let dirs = each().map(|(c, s)| format!("{s}x{s}/{}", c.dir())).collect::<Vec<_>>().join(",");
    let sections: String = each().map(|(c, s)| format!("[{s}x{s}/{}]\nContext={}\nSize={s}\nType=Fixed\n\n", c.dir(), c.theme_context())).collect();
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
    for &s in renderer.sizes.unwrap_or(&THEME_SIZES) {
        let path = out.join(format!("{s}x{s}/{}/{}.png", doc.context.dir(), doc.name));
        // (an icon that was another name for one of the others is a link: written to, it would change the one it points to)
        if path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            let _ = std::fs::remove_file(&path);
        }
        renderer.render(doc, look, s).save(path).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

/// The documents to draw, and the icons that are only another name for one of those (a document that is the same as an
/// earlier one but for its name: `(name, name of the one it is a copy of)`). Each picture is drawn once, and the other
/// names are links to it in the theme, as Papirus has them.
fn split_aliases(docs: &[IconDoc]) -> (Vec<IconDoc>, Vec<Alias>) {
    let mut seen: HashMap<Vec<u8>, String> = HashMap::new();
    let (mut unique, mut aliases) = (Vec::new(), Vec::new());
    for d in docs {
        let mut same = d.clone();
        same.name.clear();
        let key = crate::icondoc::encode(&same);
        match seen.get(&key) {
            // (the context is a part of the document, so a copy is in the same folder of the theme)
            Some(first) => aliases.push((d.name.clone(), first.clone(), d.context)),
            None => {
                seen.insert(key, d.name.clone());
                unique.push(d.clone());
            }
        }
    }
    (unique, aliases)
}

/// Make each alias a link, in the theme folder `dir`, to the picture it is another name for (the same folder, so the link is
/// the bare file name and the theme can be moved).
fn link_aliases(aliases: &[Alias], dir: &std::path::Path) {
    for &size in &THEME_SIZES {
        for (alias, of, context) in aliases {
            let link = dir.join(format!("{size}x{size}/{}/{alias}.png", context.dir()));
            if link.symlink_metadata().is_ok() {
                let _ = std::fs::remove_file(&link);
            }
            if let Err(e) = std::os::unix::fs::symlink(format!("{of}.png"), &link) {
                eprintln!("[icons] linking {alias}: {e}");
            }
        }
    }
}

/// An icon that is another name for a picture of the theme: (its name, the name of that picture, the folder of both).
type Alias = (String, String, IconContext);

/// Make the folders of every size and context in the theme folder `out`.
fn make_dirs(out: &std::path::Path) -> Result<(), String> {
    for &size in &THEME_SIZES {
        for c in IconContext::ALL {
            std::fs::create_dir_all(out.join(format!("{size}x{size}/{}", c.dir()))).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn prepare(out: &std::path::Path, source: &str) -> Result<(), String> {
    make_dirs(out)?;
    if !out.join("index.theme").exists() {
        write_index(out, source)?;
    }
    Ok(())
}

/// Do `work` for every document on all the cores but one, each thread with a renderer of its own (on the processor only
/// when `cpu`, in the sizes `sizes` only when some are given), and add up what it returns. `cancel` and `progress` are called
/// on this thread only, while the others draw.
fn for_each_doc(docs: &[IconDoc], tune: GlassTune, cpu: bool, sizes: Option<&'static [u32]>, cancel: &dyn Fn() -> bool, progress: &dyn Fn(usize, usize), work: impl Fn(&mut Renderer, &IconDoc) -> Result<usize, String> + Sync) -> Result<usize, String> {
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
                renderer.share_card = true;
                renderer.sizes = sizes;
                while !stop.load(Relaxed) {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(doc) = docs.get(i) else { break };
                    let worked = work(&mut renderer, doc);
                    renderer.forget_layers();
                    match worked {
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
                    renderer.share_card = true;
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
    let (unique, aliases) = split_aliases(docs);
    let written = for_each_doc(&unique, GlassTune::load(), false, None, cancel, progress, |renderer, doc| {
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
        link_aliases(&aliases, &look_dir(out, suffix));
        if is_live_look(look) {
            link_aliases(&aliases, &live_dir(&look_dir(out, suffix)));
        }
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

/// Draw the clear looks in `style` from now on: the tuning keeps it, and the clear themes (and their live versions) are
/// drawn again in the background (the processor's work, a few seconds); the shell is told when they are there. A newer
/// choice while drawing starts the work over.
pub fn set_clear(style: ClearStyle) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GEN: AtomicU64 = AtomicU64::new(0);
    let mut t = GlassTune::load();
    t.clear = style;
    t.colour_clear = style == ClearStyle::Colour;
    t.save();
    set_clear_style(style);
    let out = theme_dir();
    if !THEME_LOOKS.iter().filter(|(look, _)| is_live_look(*look)).all(|(_, suffix)| look_dir(&out, suffix).join("index.theme").is_file()) {
        return;
    }
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let cancel = || GEN.load(Ordering::SeqCst) != gen;
        if cancel() {
            return;
        }
        let (docs, aliases) = split_aliases(&saved_docs());
        for sizes in theme_size_steps() {
            let done = for_each_doc(&docs, GlassTune::load(), true, Some(sizes), &cancel, &|_, _| {}, |renderer, doc| {
                let mut written = 0;
                for (look, suffix) in THEME_LOOKS {
                    if is_live_look(look) {
                        let dir = look_dir(&out, suffix);
                        written += write_look(renderer, doc, look, &dir)?;
                        written += write_live(renderer, doc, look, &dir)?;
                    }
                }
                Ok(written)
            });
            match done {
                Ok(_) if !cancel() => {
                    for (look, suffix) in THEME_LOOKS.iter().filter(|(look, _)| is_live_look(*look)) {
                        let _ = look;
                        link_aliases(&aliases, &look_dir(&out, suffix));
                        link_aliases(&aliases, &live_dir(&look_dir(&out, suffix)));
                    }
                    touch_marker()
                }
                Ok(_) => return,
                Err(e) => return eprintln!("[icons] redrawing the clear looks: {e}"),
            }
        }
    });
}

/// Draw the tinted looks of every saved document again in `accent`, into the themes the editor wrote.
fn retint(accent: [f32; 3], cancel: &dyn Fn() -> bool) -> Result<(), String> {
    set_tint(accent);
    let out = theme_dir();
    let all = saved_docs();
    let (docs, aliases) = split_aliases(&all);
    for (look, suffix) in THEME_LOOKS {
        if follows_accent(look, suffix) {
            make_dirs(&look_dir(&out, suffix))?;
        }
    }
    // (the shell runs this: a core is left for it, and the processor draws, so that the shell never holds the dedicated card
    // open and keeps it awake)
    let steps = theme_size_steps();
    for (k, sizes) in steps.into_iter().enumerate() {
        for_each_doc(&docs, GlassTune::load(), true, Some(sizes), cancel, &|_, _| {}, |renderer, doc| {
            let mut written = 0;
            for (look, suffix) in THEME_LOOKS {
                if follows_accent(look, suffix) {
                    written += write_look(renderer, doc, look, &look_dir(&out, suffix))?;
                }
            }
            Ok(written)
        })?;
        for (look, suffix) in THEME_LOOKS {
            if follows_accent(look, suffix) {
                link_aliases(&aliases, &look_dir(&out, suffix));
            }
        }
        // (the last step tells the shell below, once the stamp is written)
        if k + 1 < steps.len() {
            touch_marker();
        }
    }
    prune(&out, &all);
    stamp_tint(&out, accent);
    touch_marker();
    Ok(())
}

/// Remove from the themes of every look (and their live versions) the pictures of icons that no longer have a document:
/// left behind, they would stay in the colours and the accent they were drawn in, instead of the pack's own icon.
fn prune(out: &std::path::Path, docs: &[IconDoc]) {
    let names: std::collections::HashSet<(&str, &str)> = docs.iter().map(|d| (d.context.dir(), d.name.as_str())).collect();
    for (look, suffix) in THEME_LOOKS {
        let dir = look_dir(out, suffix);
        let mut dirs = vec![dir.clone()];
        if is_live_look(look) {
            dirs.push(live_dir(&dir));
        }
        for d in dirs {
            for (&size, c) in THEME_SIZES.iter().flat_map(|s| IconContext::ALL.iter().map(move |c| (s, c))) {
                let Ok(rd) = std::fs::read_dir(d.join(format!("{size}x{size}/{}", c.dir()))) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "png") && p.file_stem().is_some_and(|n| !names.contains(&(c.dir(), n.to_string_lossy().as_ref()))) {
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
    /// A renderer that draws the layers as the document has them: no plate, and plain layers left plain.
    fn bare() -> Renderer {
        let mut r = Renderer::new();
        r.tune.plate = Shape::None;
        r.tune.dress = false;
        r
    }

    #[test]
    fn a_look_can_be_drawn_from_another_look_or_from_tinted_by_icon() {
        let mut doc = import_svg("t", ICON).unwrap();
        let mut r = Renderer::new();
        let (default, dark) = (r.render(&doc, Appearance::Default, 96), r.render(&doc, Appearance::Dark, 96));
        assert_ne!(default.as_raw(), dark.as_raw());
        // dark drawn as the default look
        doc.bases[1] = Base::Look(Appearance::Default);
        assert_eq!(r.render(&doc, Appearance::Dark, 96).as_raw(), default.as_raw(), "the same picture");
        assert_eq!(r.render(&doc, Appearance::Default, 96).as_raw(), default.as_raw(), "the default look is not changed by that");
        // the default look drawn as tinted by icon: what a caller asking for that look gets
        doc.bases = [Base::Own; 6];
        let mut by_icon = Renderer::new();
        by_icon.tint_by_icon = true;
        let tinted = by_icon.render(&doc, Appearance::TintedLight, 96);
        doc.bases[0] = Base::TintedByIcon;
        assert_eq!(r.render(&doc, Appearance::Default, 96).as_raw(), tinted.as_raw());
        // and the dark one after the default one, which is after the tinted one
        doc.bases[1] = Base::Look(Appearance::Default);
        assert_eq!(r.render(&doc, Appearance::Dark, 96).as_raw(), tinted.as_raw());
        assert!(!r.tint_by_icon, "the renderer is left as it was");
        // two looks drawn from each other end up drawn as themselves
        doc.bases = [Base::Own; 6];
        doc.bases[0] = Base::Look(Appearance::Dark);
        doc.bases[1] = Base::Look(Appearance::Default);
        assert_eq!(doc.source(Appearance::Default), (Appearance::Default, false));
        r.render(&doc, Appearance::Default, 64);
    }

    #[test]
    fn a_look_taken_from_another_can_be_made_its_own_and_draws_the_same() {
        let mut doc = import_svg("t", ICON).unwrap();
        let mut r = Renderer::new();
        let shown = r.render(&doc, Appearance::Default, 96);
        // dark drawn as the default look, then made unique: its own settings are those, and nothing is followed any more
        doc.bases[1] = Base::Look(Appearance::Default);
        let (bg, props) = r.drawn_settings(&doc, Appearance::Default);
        doc.background.pin(Appearance::Dark, bg);
        for (id, p) in props {
            doc.find_mut(id).unwrap().props_mut().pin(Appearance::Dark, p);
        }
        doc.bases[1] = Base::Own;
        assert_eq!(r.render(&doc, Appearance::Dark, 96).as_raw(), shown.as_raw(), "dark now draws what default draws, by itself");
        // and it is its own: a change in the default look does not reach it
        let id = doc.layers()[0].id;
        let mut p = effective_props(doc.find(id).unwrap(), Appearance::Default);
        p.opacity = 0.4;
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        assert_eq!(r.render(&doc, Appearance::Dark, 96).as_raw(), shown.as_raw(), "dark is not moved by it");
        assert_ne!(r.render(&doc, Appearance::Default, 96).as_raw(), shown.as_raw());
    }

    #[test]
    fn copies_of_a_document_are_drawn_once_and_the_other_names_are_links() {
        let one = import_svg("one", ICON).unwrap();
        let other = IconDoc { name: "other".into(), ..one.clone() };
        let mut different = one.clone();
        different.name = "different".into();
        different.bases[1] = Base::Look(Appearance::Default);
        let docs = vec![one.clone(), other, different];
        let (unique, aliases) = split_aliases(&docs);
        assert_eq!(unique.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["one", "different"]);
        assert_eq!(aliases, [("other".to_string(), "one".to_string(), IconContext::App)]);
        // the links: a bare file name beside the picture; and drawing a name that was a link does not write through it
        let dir = std::env::temp_dir().join(format!("memrium-alias-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for &s in &THEME_SIZES {
            std::fs::create_dir_all(dir.join(format!("{s}x{s}/apps"))).unwrap();
            std::fs::write(dir.join(format!("{s}x{s}/apps/one.png")), b"one").unwrap();
            std::fs::write(dir.join(format!("{s}x{s}/apps/other.png")), b"an old copy").unwrap();
        }
        link_aliases(&aliases, &dir);
        let link = dir.join("64x64/apps/other.png");
        assert_eq!(std::fs::read_link(&link).unwrap(), std::path::PathBuf::from("one.png"));
        assert_eq!(std::fs::read(&link).unwrap(), b"one");
        let mut r = bare();
        write_sizes(&mut r, &docs[1], Appearance::Default, &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("64x64/apps/one.png")).unwrap(), b"one", "the picture it was a link to is untouched");
        assert!(!link.symlink_metadata().unwrap().file_type().is_symlink(), "and it is a picture of its own now");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_icon_is_dressed_in_the_default_look_only() {
        let doc = import_svg("t", r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><circle cx="32" cy="32" r="16" fill="#2060ff"/></svg>"##).unwrap();
        let mut plain = Renderer::new();
        plain.tune.dress = false;
        let mut dressed = Renderer::new();
        assert!(dressed.tune.dress, "icons are dressed unless it is turned off");
        // (10, 32): on the plate, left of the circle
        let (white, pale) = (px(&plain.render(&doc, Appearance::Default, 128), 10, 64), px(&dressed.render(&doc, Appearance::Default, 128), 10, 64));
        assert!(white[2].abs_diff(white[0]) < 12, "the plate every icon gets is white: {white:?}");
        assert!(pale[2] > pale[0] + 20 && pale[0] > 150, "dressed, it is a pale blue, the colour of the icon: {pale:?}");
        // the layer is glass: light on its edge and a shadow under it change the picture round it
        let (a, b) = (plain.render(&doc, Appearance::Default, 128), dressed.render(&doc, Appearance::Default, 128));
        assert_ne!(px(&a, 64, 98), px(&b, 64, 98), "the shadow under the circle");
        // the dark look is the same either way
        assert_eq!(plain.render(&doc, Appearance::Dark, 128).as_raw(), dressed.render(&doc, Appearance::Dark, 128).as_raw(), "dark is left as it was");
        // a plate set by hand is kept
        let mut own = doc.clone();
        own.background.default = Background { fill: FillKind::Solid, top: [0.0, 1.0, 0.0], ..own.background.default };
        let green = px(&dressed.render(&own, Appearance::Default, 128), 10, 64);
        assert!(green[1] > 200 && green[0] < 60, "{green:?}");
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
        r.tune.dress = false;
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
    fn the_auto_style_is_frost_over_a_dark_picture_and_contrast_over_a_light_one() {
        let (dark_back, light_back) = ([0.20, 0.10, 0.30], [0.97, 0.98, 1.0]);
        // the codes the tuning file keeps did not move
        assert_eq!(ClearStyle::from_code(0), ClearStyle::Frost);
        assert_eq!(ClearStyle::from_code(3), ClearStyle::Colour);
        assert_eq!(ClearStyle::from_code(4), ClearStyle::Auto);
        assert!(ClearStyle::Auto.follows_backdrop());
        // dark picture: Frost exactly
        assert_eq!(auto_ink(dark_back, 0.9, false), frost_ink(dark_back, 0.9, false));
        assert_eq!(ClearStyle::Auto.plate_tint_over(dark_back, false), ClearStyle::Frost.plate_tint(false));
        // light picture: a white symbol on smoky glass, the shadow under it still dark
        let white = auto_ink(light_back, 1.0, false);
        assert!(white.iter().all(|v| *v > 0.97), "a white symbol: {white:?}");
        assert!(auto_ink(light_back, 0.06, false)[0] < 0.2, "the shadow stays dark");
        assert_eq!(ClearStyle::Auto.plate_tint_over(light_back, false), ClearStyle::Contrast.plate_tint(false));
        assert_eq!(ClearStyle::Auto.plate_tint_over(light_back, true), ClearStyle::Contrast.plate_tint(true));
        // in between it is in between, and never jumps
        let mut last = 0.0;
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            let k = auto_amount([v; 3]);
            assert!(k >= last && (k - last) < 0.08, "{v}: {k}");
            last = k;
        }
        assert!(last == 1.0 && auto_amount([0.0; 3]) == 0.0);
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
            r.tune = GlassTune { sharp, ..r.tune };
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

/// A box blur with radius `r`, three passes each way (close to a gaussian); outside the picture is 0.
fn blur_zero(src: &[f32], n: usize, r: usize) -> Vec<f32> {
    let mut a = src.to_vec();
    let mut b = vec![0f32; a.len()];
    let k = (2 * r + 1) as f32;
    for _ in 0..3 {
        for y in 0..n {
            let row = &a[y * n..(y + 1) * n];
            let at = |x: isize| if x < 0 || x >= n as isize { 0.0 } else { row[x as usize] };
            let mut acc: f32 = (-(r as isize)..=r as isize).map(at).sum();
            for x in 0..n {
                b[y * n + x] = acc / k;
                acc += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
            }
        }
        for x in 0..n {
            let at = |y: isize| if y < 0 || y >= n as isize { 0.0 } else { b[y as usize * n + x] };
            let mut acc: f32 = (-(r as isize)..=r as isize).map(at).sum();
            for y in 0..n {
                a[y * n + x] = acc / k;
                acc += at(y as isize + r as isize + 1) - at(y as isize - r as isize);
            }
        }
    }
    a
}

/// The glass edge of the phone's launcher (`android/src/apps.rs`, `glass_edges`) on a finished icon: along the outline
/// of `shape` (the plate's coverage, n × n) a thin bright rim, strongest on the side turned to the light (`toward`, a
/// unit vector from the glass to the light) and weaker on the opposite one, and a faint bevel inside it, lighter towards
/// the light and darker away from it. `amount` scales it all (1 is the phone's). The picture has straight alpha.
pub fn edge_glow(img: &mut RgbaImage, shape: &[f32], amount: f32, toward: (f32, f32)) {
    let n = img.width() as usize;
    if n < 8 || shape.len() != n * n || img.height() as usize != n {
        return;
    }
    let side = n as f32 * plate_fit();
    let rim = blur_zero(shape, n, ((side * 0.02).round() as usize).max(1));
    let bevel = blur_zero(shape, n, ((side * 0.045).round() as usize).max(2));
    let (lx, ly) = toward;
    let at = |v: &[f32], x: usize, y: usize| v[y.min(n - 1) * n + x.min(n - 1)];
    let normal = |v: &[f32], x: usize, y: usize| {
        let gx = at(v, x + 1, y) - at(v, x.saturating_sub(1), y);
        let gy = at(v, x, y + 1) - at(v, x, y.saturating_sub(1));
        let l = (gx * gx + gy * gy).sqrt();
        if l < 1e-5 {
            (0.0, 0.0)
        } else {
            (-gx / l, -gy / l)
        }
    };
    let px = img.as_mut();
    for y in 0..n {
        for x in 0..n {
            let a = shape[y * n + x];
            if a <= 0.0 {
                continue;
            }
            let edge = ((0.97 - rim[y * n + x]) / 0.42).clamp(0.0, 1.0).powf(1.3);
            let (nx, ny) = normal(&rim, x, y);
            let facing = nx * lx + ny * ly;
            let light = 0.2 + 0.8 * facing.max(0.0).powf(1.2) + 0.6 * (-facing).max(0.0).powf(1.5);
            let t_rim = (edge * light * 0.8 * amount).min(0.85);
            let band = ((1.0 - bevel[y * n + x]).clamp(0.0, 1.0) * 1.6).min(1.0);
            let (bx, by) = normal(&bevel, x, y);
            let shade = (bx * lx + by * ly) * band * amount;
            let p = &mut px[(y * n + x) * 4..][..4];
            for c in 0..3 {
                let mut v = p[c] as f32;
                if shade > 0.0 {
                    v += (255.0 - v) * (shade * 0.22).min(1.0);
                } else {
                    v *= (1.0 + shade * 0.18).max(0.0);
                }
                v += (255.0 - v) * t_rim;
                p[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}
