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

fn box_line(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut sum: f32 = src[..r.min(n)].iter().sum();
    for i in 0..n {
        if i + r < n {
            sum += src[i + r];
        }
        dst[i] = sum * norm;
        if i >= r {
            sum -= src[i - r];
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
    }
}

/// How much of each pixel the shape of half size `half` centred on the canvas covers (4 x 4 samples).
fn shape_mask(shape: Shape, n: usize, half: f32, corner: f32) -> Vec<f32> {
    let c = n as f32 / 2.0;
    let mut out = vec![0.0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (qx, qy) = ((x as f32 + (sx as f32 + 0.5) / 4.0 - c) / half, (y as f32 + (sy as f32 + 0.5) / 4.0 - c) / half);
                    if inside(shape, qx, qy, corner) {
                        hit += 1;
                    }
                }
            }
            out[y * n + x] = hit as f32 / 16.0;
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
}

impl Default for GlassTune {
    fn default() -> GlassTune {
        GlassTune { specular: 1.0, blur: 1.0, refraction: 1.0, translucency: 1.0, shadow: 1.0, light: 231.5, shadow_dist: 1.0, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, colour_clear: false }
    }
}

/// Ready-made looks of the glass, for the buttons of the editor.
pub const GLASS_PRESETS: [(&str, GlassTune); 5] = [
    ("Liquid", GlassTune { specular: 1.0, blur: 1.0, refraction: 1.0, translucency: 1.0, shadow: 1.0, light: 231.5, shadow_dist: 1.0, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, colour_clear: false }),
    ("Frosted", GlassTune { specular: 0.8, blur: 1.9, refraction: 0.5, translucency: 1.4, shadow: 0.9, light: 231.5, shadow_dist: 1.0, shadow_soft: 1.5, corner: 1.0, sharp: 0.6, colour_clear: false }),
    ("Crystal", GlassTune { specular: 1.6, blur: 0.2, refraction: 1.8, translucency: 1.6, shadow: 0.8, light: 231.5, shadow_dist: 0.8, shadow_soft: 0.8, corner: 1.0, sharp: 1.5, colour_clear: false }),
    ("Matte", GlassTune { specular: 0.3, blur: 1.0, refraction: 0.2, translucency: 0.5, shadow: 1.2, light: 231.5, shadow_dist: 1.2, shadow_soft: 1.4, corner: 1.0, sharp: 0.5, colour_clear: false }),
    ("Flat", GlassTune { specular: 0.15, blur: 0.0, refraction: 0.0, translucency: 0.0, shadow: 0.4, light: 231.5, shadow_dist: 0.6, shadow_soft: 1.0, corner: 1.0, sharp: 1.0, colour_clear: false }),
];

impl GlassTune {
    fn path() -> std::path::PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
        base.join("memrium").join("icon-glass")
    }

    /// The saved tuning. Files from before the light, shadow and corner settings hold five numbers.
    pub fn load() -> GlassTune {
        let text = std::fs::read_to_string(GlassTune::path()).unwrap_or_default();
        let v: Vec<f32> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        let d = GlassTune::default();
        match v[..] {
            [specular, blur, refraction, translucency, shadow] => GlassTune { specular, blur, refraction, translucency, shadow, ..d },
            [specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner] => GlassTune { specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner, ..d },
            [specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner, sharp] => GlassTune { specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner, sharp, ..d },
            [specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner, sharp, colour] => GlassTune { specular, blur, refraction, translucency, shadow, light, shadow_dist, shadow_soft, corner, sharp, colour_clear: colour > 0.5 },
            _ => d,
        }
    }

    pub fn save(&self) {
        let path = GlassTune::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            path,
            format!("{} {} {} {} {} {} {} {} {} {} {}\n", self.specular, self.blur, self.refraction, self.translucency, self.shadow, self.light, self.shadow_dist, self.shadow_soft, self.corner, self.sharp, self.colour_clear as u8),
        );
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

    /// The drawn layer (straight alpha, `n` x `n`), recoloured when its settings or the look ask for it.
    pub fn raster(&mut self, layer: &Layer, props: &Props, doc: &IconDoc, app: Appearance, n: u32) -> Arc<RgbaImage> {
        let treated = (app.is_clear() && !self.tune.colour_clear) || app.is_tinted();
        let key = hash_of(&[
            &|h| layer.svg.hash(h),
            &|h| props.fill.map(|c| c.map(|v| v.to_bits())).hash(h),
            &|h| props.gradient.map(|g| (g.a.map(f32::to_bits), g.b.map(f32::to_bits), g.angle.to_bits(), g.radial)).hash(h),
            &|h| (if treated { Some(app.label()) } else { None }, if app.is_tinted() { doc.tint.map(|v| v.to_bits()) } else { [0; 3] }).hash(h),
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
            for p in img.pixels_mut() {
                let c = treat([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0], app, doc.tint);
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

    /// The document as a picture of `n` x `n` pixels in the look `app`.
    pub fn render(&mut self, doc: &IconDoc, app: Appearance, n: u32) -> RgbaImage {
        let nn = n as usize;
        let fit = 1.0 - 2.0 * doc.margin.clamp(0.0, 0.3);
        let bg = background_for(doc, app);
        let mask = (bg.shape != Shape::None).then(|| shape_mask(bg.shape, nn, n as f32 * fit / 2.0, self.tune.corner));
        let mut accum = match &self.backdrop {
            Some(bd) => {
                let img = if bd.width() == n { (**bd).clone() } else { image::imageops::resize(&**bd, n, n, image::imageops::FilterType::Triangle) };
                Buf::from_image(&img)
            }
            None => Buf::new(nn),
        };

        // the plate
        let mut plate = Buf::new(nn);
        if bg.fill != FillKind::None {
            let top = n as f32 * (1.0 - fit) / 2.0;
            for y in 0..nn {
                let t = ((y as f32 + 0.5 - top) / (n as f32 * fit)).clamp(0.0, 1.0);
                let c = if bg.fill == FillKind::Solid { bg.top } else { [mix(bg.top[0], bg.bottom[0], t), mix(bg.top[1], bg.bottom[1], t), mix(bg.top[2], bg.bottom[2], t)] };
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
        composite(&mut accum, &plate, &Props { glass: plate_glass, ..Props::default() }, None, &self.tune);

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
                let img = self.raster(layer, &p, doc, app, n as u32);
                let src = Buf::from_image(&img);
                let moved = transform(&src, p.scale * place.scale, [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]]);
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
    fn placement(doc: &IconDoc, app: Appearance, id: u32) -> Option<(f32, [f32; 2], bool)> {
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
        let fit = 1.0 - 2.0 * doc.margin.clamp(0.0, 0.3);
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

/// The shape of `img` (its alpha) painted with the gradient: along its angle across the box of the shape, or from the middle
/// of that box out.
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
            Appearance::Dark => Background { shape, fill: FillKind::Gradient, top: DARK_BASE_TOP, bottom: DARK_BASE_BOTTOM, ..*base },
            Appearance::ClearLight => Background { shape, fill: FillKind::Gradient, top: [0.66; 3], bottom: [0.52; 3], glass: clear(0.75) },
            Appearance::ClearDark => Background { shape, fill: FillKind::Gradient, top: [0.24; 3], bottom: [0.15; 3], glass: clear(0.6) },
            Appearance::TintedLight => Background { shape, fill: FillKind::Gradient, top: scaled(1.05), bottom: scaled(0.82), glass: glass(0.7) },
            Appearance::TintedDark => Background { shape, fill: FillKind::Gradient, top: scaled(0.2), bottom: scaled(0.12), glass: glass(0.9) },
        }
    }
}

/// The plate in look `app`: what the system makes of the default one, with what the looks on the way changed.
pub fn background_for(doc: &IconDoc, app: Appearance) -> Background {
    doc.background.resolve(app, &auto_background(doc.tint))
}

/// Make the plate in look `app` be `b`: the look keeps only what differs from what it inherits.
pub fn set_background(doc: &mut IconDoc, app: Appearance, b: Background) {
    let auto = auto_background(doc.tint);
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

    // the slope of the silhouette: where the edges face, and how far a pixel is from them
    // sharper glass has narrower bevels and a finer line of light on its edge
    let sharp = tune.sharp.clamp(0.25, 2.5);
    // (the lens that bends what is behind has a wide bevel, the light on the edge a narrow one)
    let slope_of = |sigma: f32| -> (Vec<(f32, f32)>, f32) {
        let mut height = alpha.clone();
        blur(&mut height, n, nf * sigma);
        let mut slope = vec![(0.0f32, 0.0f32); n * n];
        let mut peak = 1e-4f32;
        let at = |x: usize, y: usize| height[y.min(n - 1) * n + x.min(n - 1)];
        for y in 0..n {
            for x in 0..n {
                let gx = (at(x + 1, y) - at(x.saturating_sub(1), y)) * 0.5;
                let gy = (at(x, y + 1) - at(x, y.saturating_sub(1))) * 0.5;
                peak = peak.max(gx.hypot(gy));
                slope[y * n + x] = (gx, gy);
            }
        }
        (slope, peak)
    };
    let (mut slope, mut peak, mut hair) = (Vec::new(), 1e-4f32, Vec::new());
    let (mut wide, mut wide_peak) = (Vec::new(), 1e-4f32);
    if glass && (g.specular > 0.0 || g.refraction > 0.0) {
        (slope, peak) = slope_of(0.02 / sharp);
        hair = alpha.clone();
        blur(&mut hair, n, nf * 0.008 / sharp);
    }
    if glass && (g.refraction > 0.0 || g.inner_shadow > 0.0) {
        (wide, wide_peak) = slope_of(0.045 / sharp);
    }
    // the rounded edge of a bevel: as wide as it is asked to be
    let (mut bev, mut bev_peak) = (Vec::new(), 1e-4f32);
    if glass && g.bevel > 0.0 {
        (bev, bev_peak) = slope_of((0.008 + 0.06 * g.bevel_width.clamp(0.0, 1.0)) / sharp);
    }

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
                let mut a = sv * g.shadow_amount * 0.6;
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
    if glass && g.refraction > 0.0 && !wide.is_empty() {
        let behind = accum.clone();
        let bend = nf * 0.11 * g.refraction;
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                let w = alpha[i].clamp(0.0, 1.0);
                if w <= 0.0 {
                    continue;
                }
                let (nx, ny) = (wide[i].0 / wide_peak, wide[i].1 / wide_peak);
                // the colours are bent by a little different amounts, so that the edges of the lens split the light
                let at = |f: f32| behind.sample(x as f32 + nx * bend * f, y as f32 + ny * bend * f);
                let (vr, vg, vb) = (at(0.9), at(1.0), at(1.1));
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
        let bend = nf * 0.05 * g.refraction;
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                let la = own.c[3][i];
                if la <= 0.0 {
                    continue;
                }
                let (nx, ny) = (slope[i].0 / peak, slope[i].1 / peak);
                let v = own.sample(x as f32 + nx * bend, y as f32 + ny * bend);
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
            if g.inner_shadow > 0.0 && !wide.is_empty() {
                let edge = (wide[i].0.hypot(wide[i].1) / wide_peak).clamp(0.0, 1.0);
                let v = (edge * g.inner_shadow * smooth(a) * p.opacity * 0.55).clamp(0.0, 0.8);
                for k in 0..3 {
                    accum.c[k][i] *= 1.0 - v;
                }
            }
        }
    }

    // light caught by the edges
    if glass && g.specular > 0.0 && !slope.is_empty() {
        let (lx, ly) = tune.light_dir();
        for i in 0..n * n {
            let a = alpha[i];
            if a <= 0.0 {
                continue;
            }
            let (nx, ny) = (slope[i].0 / peak, slope[i].1 / peak);
            let edge = nx.hypot(ny).clamp(0.0, 1.0);
            let facing = -(nx * lx + ny * ly);
            let lit = facing.clamp(0.0, 1.0).powf(1.4) * edge.powf(0.6);
            let back = (-facing).clamp(0.0, 1.0).powi(2) * edge * 0.35;
            let line = (a - hair[i]).clamp(0.0, 1.0) * 2.0;
            let v = ((lit * 0.95 + back + line * sharp.min(2.0) * (0.3 + 0.4 * facing.max(0.0))) * g.specular * smooth(a)).clamp(0.0, 1.0) * p.opacity;
            // the light multiplies the colour instead of being mixed in as white, so the glass stays as saturated as it was
            let da = accum.c[3][i];
            // a bright rim all round (more where the edge is steep), and a soft sheen that lies across the glass towards the light
            let rim = edge.powf(1.6) * smooth(a) * g.specular * 0.10 * p.opacity;
            let (px, py) = ((i % n) as f32 / nf - 0.5, (i / n) as f32 / nf - 0.5);
            let sheen = (0.5 - (px * lx + py * ly) * 0.9).clamp(0.0, 1.0).powi(2) * smooth(a) * g.specular * 0.05 * p.opacity;
            for k in 0..3 {
                let c = accum.c[k][i];
                let lit = (c * (1.0 + 1.1 * v) + da * v * 0.12).min(da);
                accum.c[k][i] = mix(lit, da, (rim + sheen).clamp(0.0, 0.6));
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
pub const THEME_LOOKS: [(Appearance, &str); 6] = [
    (Appearance::Default, ""),
    (Appearance::Dark, "-Dark"),
    (Appearance::ClearLight, "-Clear-Light"),
    (Appearance::ClearDark, "-Clear-Dark"),
    (Appearance::TintedLight, "-Tinted-Light"),
    (Appearance::TintedDark, "-Tinted-Dark"),
];

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

/// One icon drawn in a look at `size` x `size`, written into the theme folder `out` in every size of the theme.
fn write_look(renderer: &mut Renderer, doc: &IconDoc, look: Appearance, size: u32, out: &std::path::Path) -> Result<usize, String> {
    // drawn once large and shrunk, so that the small sizes keep clean edges
    let big = renderer.render(doc, look, size);
    let mut written = 0;
    for &s in &THEME_SIZES {
        let img = if s == size { big.clone() } else { image::imageops::resize(&big, s, s, image::imageops::FilterType::Lanczos3) };
        img.save(out.join(format!("{s}x{s}/apps/{}.png", doc.name))).map_err(|e| e.to_string())?;
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

/// Write the pictures of one icon into the themes of all the looks (a quick version of `build_theme`, for the live preview
/// while editing), and tell the shell.
pub fn build_one(doc: &IconDoc, tune: GlassTune, source: &str, notify: bool) -> Result<(), String> {
    let mut renderer = Renderer::new();
    renderer.tune = tune;
    for (look, suffix) in THEME_LOOKS {
        let out = look_dir(&theme_dir(), suffix);
        prepare(&out, source)?;
        write_look(&mut renderer, doc, look, 256, &out)?;
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
    let mut renderer = Renderer::new();
    renderer.tune = GlassTune::load();
    let mut written = 0;
    for (_, suffix) in THEME_LOOKS {
        prepare(&look_dir(out, suffix), source)?;
    }
    for (i, doc) in docs.iter().enumerate() {
        if cancel() {
            return Err("cancelled".into());
        }
        for (look, suffix) in THEME_LOOKS {
            written += write_look(&mut renderer, doc, look, 512, &look_dir(out, suffix))?;
        }
        progress(i + 1, docs.len());
    }
    for (_, suffix) in THEME_LOOKS {
        write_index(&look_dir(out, suffix), source)?;
    }
    touch_marker();
    Ok(written)
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
    use super::*;

    const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
 <rect id="back" x="8" y="8" width="48" height="48" fill="#ff0000"/>
 <g id="pair"><circle cx="32" cy="32" r="14" fill="#0000ff"/><circle cx="46" cy="18" r="6" fill="#00ff00"/></g>
</svg>"##;

    fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        img.get_pixel(x, y).0
    }

    #[test]
    fn an_import_has_a_layer_per_shape_and_looks_like_the_svg() {
        let doc = import_svg("t", ICON).unwrap();
        assert_eq!(doc.layers().len(), 3);
        assert!(matches!(doc.nodes[1].kind, Kind::Group(_)), "the <g> became a group");
        let img = Renderer::new().render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 32, 32), [0, 0, 255, 255], "the blue circle on the red square");
        assert_eq!(px(&img, 12, 50), [255, 0, 0, 255]);
        assert_eq!(px(&img, 2, 2)[3], 0);
    }

    #[test]
    fn opacity_hiding_and_blending_change_the_picture() {
        let mut doc = import_svg("t", ICON).unwrap();
        let circle = doc.layers()[1].id;
        let mut r = Renderer::new();
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
        let mut r = Renderer::new();
        {
            let p = &mut doc.find_mut(square).unwrap().props_mut().default;
            p.scale = 0.5;
            p.fill = Some([1.0, 1.0, 0.0]);
        }
        let img = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 21, 43), [255, 255, 0, 255], "yellow now, and smaller");
        assert_eq!(px(&img, 12, 50)[3], 0, "no longer reaches the corner");
        doc.find_mut(square).unwrap().props_mut().default.offset = [0.25, 0.0];
        let moved = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&moved, 58, 43), [255, 255, 0, 255], "moved right");
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
        // the tinted looks paint everything in the tint colour (a purple by default): plate and layers
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
        let mut r = Renderer::new();
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
        p.gradient = Some(Gradient { a: [0.0, 0.0, 1.0], b: [1.0, 0.0, 0.0], angle: 0.0, radial: false });
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let img = Renderer::new().render(&doc, Appearance::Default, 64);
        let (top, bottom) = (px(&img, 20, 10), px(&img, 20, 54));
        assert!(top[2] > 150 && top[0] < 100, "blue at the top: {top:?}");
        assert!(bottom[0] > 150 && bottom[2] < 100, "red at the bottom: {bottom:?}");
        // a radial one is the first colour in the middle of the shape
        p.gradient = Some(Gradient { radial: true, ..p.gradient.unwrap() });
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let plain = Renderer::new().render(&doc, Appearance::Default, 64);
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
            let with = Renderer::new().render(&doc, Appearance::Default, 64);
            assert_ne!(with.as_raw(), plain.as_raw(), "{name} changes the picture");
        }
    }

    #[test]
    fn flipping_and_turning_a_layer_moves_its_drawing() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect x="0" y="0" width="16" height="16" fill="#ff0000"/></svg>"##;
        let at = |s: &str, x: u32, y: u32| px(&Renderer::new().render(&import_svg("t", s).unwrap(), Appearance::Default, 64), x, y);
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
            let mut r = Renderer::new();
            r.tune = GlassTune { colour_clear: colour, ..GlassTune::default() };
            px(&r.render(&doc, Appearance::ClearLight, 64), 12, 50)
        };
        let grey = red_at(false);
        let coloured = red_at(true);
        assert!((grey[0] as i32 - grey[2] as i32).abs() < 12, "the clear look is grey: {grey:?}");
        assert!(coloured[0] as i32 > coloured[1] as i32 + 60, "kept red: {coloured:?}");
    }

    #[test]
    fn sharpness_changes_how_crisp_the_edges_of_the_glass_are() {
        let mut doc = import_svg("t", ICON).unwrap();
        let id = doc.layers()[0].id;
        let mut p = effective_props(doc.find(id).unwrap(), Appearance::Default);
        p.glass = Glass { enabled: true, specular: 1.0, refraction: 1.0, ..Glass::default() };
        set_node_props(doc.find_mut(id).unwrap(), Appearance::Default, p);
        let draw = |sharp: f32| {
            let mut r = Renderer::new();
            r.tune = GlassTune { sharp, ..GlassTune::default() };
            r.render(&doc, Appearance::Default, 128)
        };
        assert_ne!(draw(0.3).as_raw(), draw(2.0).as_raw(), "soft and crisp glass are not the same picture");
        assert_eq!(draw(1.0).as_raw(), Renderer::new().render(&doc, Appearance::Default, 128).as_raw(), "100% is what it was");
    }

    #[test]
    fn a_picture_behind_the_glass_is_blurred_and_kept() {
        let doc = import_svg("t", ICON).unwrap();
        let mut r = Renderer::new();
        r.backdrop = Some(Arc::new(RgbaImage::from_pixel(64, 64, Rgba([10, 200, 30, 255]))));
        let img = r.render(&doc, Appearance::Default, 64);
        assert_eq!(px(&img, 2, 2), [10, 200, 30, 255], "the picture holds the backdrop where nothing is drawn");
        assert_eq!(px(&img, 12, 50), [255, 0, 0, 255], "and the layers are on it");
    }
}
