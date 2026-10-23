//! The icon "squircle-izer": any application icon (a Papirus circle, a bare glyph, a folder) is set onto an opaque
//! tile in the shape of the theme's icons, the way macOS draws app icons. The tile colour is chosen from the icon:
//! a light tile for coloured icons, a dark one for icons that are mostly white, or the icon's own colour on request.
//! The result is baked into the icon atlas, so drawing it costs nothing extra.

use crate::theme::{IconShape, TileMode};
use image::{imageops, Rgba, RgbaImage};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileStyle {
    pub shape: IconShape,
    /// space around the icon as a fraction of the tile
    pub padding: f32,
    pub mode: TileMode,
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Is the point `(x, y)` (in -1..1 across the tile) inside the shape?
fn inside(shape: IconShape, x: f32, y: f32) -> bool {
    match shape {
        IconShape::Circle => x * x + y * y <= 1.0,
        IconShape::Rounded => {
            // a rounded rectangle whose corners are 0.44 of the half size
            let r = 0.44;
            let (qx, qy) = (x.abs() - (1.0 - r), y.abs() - (1.0 - r));
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            outside + qx.max(qy).min(0.0) <= r
        }
        // the macOS icon outline is a superellipse, flatter than a rounded square
        _ => x.abs().powi(5) + y.abs().powi(5) <= 1.0,
    }
}

/// Fraction of the pixel `(px, py)` covered by the shape scaled by `grow` (4 x 4 samples).
fn coverage(shape: IconShape, px: u32, py: u32, n: u32, grow: f32) -> f32 {
    let half = n as f32 / 2.0 - 0.5;
    let mut hit = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let x = px as f32 + (sx as f32 + 0.5) / 4.0 - n as f32 / 2.0;
            let y = py as f32 + (sy as f32 + 0.5) / 4.0 - n as f32 / 2.0;
            if inside(shape, x / half * grow, y / half * grow) {
                hit += 1;
            }
        }
    }
    hit as f32 / 16.0
}

/// What the icon looks like on average: its main colour and how light it is.
fn analyse(src: &RgbaImage) -> ([f32; 3], f32) {
    let (mut w, mut rgb, mut lum, mut lw) = (0.0f32, [0.0f32; 3], 0.0f32, 0.0f32);
    for p in src.pixels() {
        let a = p[3] as f32 / 255.0;
        if a < 0.5 {
            continue;
        }
        let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        // colourful pixels decide the hue; grey ones barely count
        let weight = a * (0.1 + sat);
        for i in 0..3 {
            rgb[i] += c[i] * weight;
        }
        w += weight;
        lum += a * (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]);
        lw += a;
    }
    let hue = if w > 0.0 { [rgb[0] / w, rgb[1] / w, rgb[2] / w] } else { [0.5; 3] };
    (hue, if lw > 0.0 { lum / lw } else { 0.5 })
}

/// The box (x0, y0, x1, y1, inclusive) around the pixels that are mostly opaque.
fn opaque_box(src: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in src.enumerate_pixels() {
        if p[3] > 128 {
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    b
}

/// Is this icon already a rounded square (or squircle) that fills its canvas? Such an icon is a tile itself: putting
/// it on another one would show a plate behind a plate. A circle fills only 78% of its box, a bare glyph far less.
fn is_ready_tile(src: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0, x1, y1) = opaque_box(src)?;
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let (sw, sh) = src.dimensions();
    if w * 100 < sw * 85 || h * 100 < sh * 85 || w.min(h) * 100 < w.max(h) * 90 || w < 16 {
        return None;
    }
    let solid = |x: u32, y: u32| src.get_pixel(x, y)[3] > 200;
    // the middle of each side is solid, a little inside the box
    let (mx, my, dx, dy) = ((x0 + x1) / 2, (y0 + y1) / 2, (w / 25).max(1), (h / 25).max(1));
    if !(solid(mx, y0 + dy) && solid(mx, y1 - dy) && solid(x0 + dx, my) && solid(x1 - dx, my)) {
        return None;
    }
    // the corners are cut away (a square with its corners is a picture, not an icon plate)
    if [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].iter().any(|&(x, y)| src.get_pixel(x, y)[3] > 128) {
        return None;
    }
    let area = src.pixels().filter(|p| p[3] > 128).count() as f32;
    (area / (w * h) as f32 > 0.84).then_some((x0, y0, x1, y1))
}

/// The icon on its tile, `n` x `n` pixels, straight (not premultiplied) alpha. An icon that is a rounded square
/// already is shown as it is, cropped to its shape and scaled to the tile.
pub fn bake(src: &RgbaImage, st: &TileStyle, n: u32) -> RgbaImage {
    if let Some((x0, y0, x1, y1)) = is_ready_tile(src) {
        // the icon is the tile: cut to the theme's shape and lit like the other tiles
        let crop = imageops::crop_imm(src, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
        let icon = imageops::resize(&crop, n, n, imageops::FilterType::Lanczos3);
        let shape = if st.shape == IconShape::None { IconShape::Squircle } else { st.shape };
        let ring = 1.0 + 3.0 / n as f32;
        let mut out = RgbaImage::new(n, n);
        for y in 0..n {
            let t = (y as f32 + 0.5) / n as f32;
            for x in 0..n {
                let cov = coverage(shape, x, y, n, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let p = icon.get_pixel(x, y).0;
                let mut c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
                let rim = (cov - coverage(shape, x, y, n, ring)).max(0.0);
                c = mix(c, [1.0; 3], rim * 0.55 * (1.0 - t));
                c = mix(c, [0.0; 3], rim * 0.12 * t);
                // a pixel the icon leaves clear (its own corner) shows the icon's colour at its edge, so it stays opaque
                out.put_pixel(x, y, Rgba([(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, (cov * 255.0).round() as u8]));
            }
        }
        return out;
    }
    let (hue, mean_luma) = analyse(src);
    let dark = match st.mode {
        TileMode::Light | TileMode::Vivid => false,
        TileMode::Dark => true,
        TileMode::Auto => mean_luma > 0.80,
    };
    let (top, bottom) = match (st.mode, dark) {
        (TileMode::Vivid, _) => (mix(hue, [1.0; 3], 0.30), mix(hue, [0.0; 3], 0.30)),
        (_, false) => (mix([1.0; 3], hue, 0.05), mix([0.92, 0.93, 0.96], hue, 0.20)),
        (_, true) => (mix([0.36, 0.38, 0.46], hue, 0.20), mix([0.15, 0.16, 0.21], hue, 0.20)),
    };
    let shape = if st.shape == IconShape::None { IconShape::Squircle } else { st.shape };
    let inner = ((n as f32 * (1.0 - 2.0 * st.padding.clamp(0.04, 0.4))).round() as u32).max(8);
    let icon = imageops::resize(src, inner, inner, imageops::FilterType::Lanczos3);
    let off = (n - inner) / 2;
    let ring = 1.0 + 3.0 / n as f32;
    let mut out = RgbaImage::new(n, n);
    for y in 0..n {
        let t = (y as f32 + 0.5) / n as f32;
        for x in 0..n {
            let cov = coverage(shape, x, y, n, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let mut c = mix(top, bottom, t);
            // a thin light edge on the upper half and a soft dark one below, like a lit slab
            let rim = (cov - coverage(shape, x, y, n, ring)).max(0.0);
            c = mix(c, [1.0; 3], rim * 0.55 * (1.0 - t));
            c = mix(c, [0.0; 3], rim * 0.12 * t);
            let (mut r, mut g, mut b, mut a) = (c[0], c[1], c[2], cov);
            // the icon over the tile
            if x >= off && y >= off && x < off + inner && y < off + inner {
                let p = icon.get_pixel(x - off, y - off).0;
                let ia = p[3] as f32 / 255.0;
                if ia > 0.0 {
                    let (ir, ig, ib) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
                    let oa = ia + a * (1.0 - ia);
                    r = (ir * ia + r * a * (1.0 - ia)) / oa;
                    g = (ig * ia + g * a * (1.0 - ia)) / oa;
                    b = (ib * ia + b * a * (1.0 - ia)) / oa;
                    a = oa;
                }
            }
            out.put_pixel(x, y, Rgba([(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8, (a * 255.0).round() as u8]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: [u8; 4], n: u32) -> RgbaImage {
        RgbaImage::from_pixel(n, n, Rgba(c))
    }

    #[test]
    fn the_tile_is_opaque_inside_and_clear_outside() {
        let t = bake(&square([200, 30, 30, 255], 64), &TileStyle { shape: IconShape::Squircle, padding: 0.16, mode: TileMode::Auto }, 128);
        assert_eq!(t.get_pixel(64, 64)[3], 255, "centre");
        assert_eq!(t.get_pixel(30, 4)[3], 255, "edge midpoint is part of the tile");
        assert_eq!(t.get_pixel(0, 0)[3], 0, "the corner is cut away");
    }

    /// a rounded square of `c` filling `n` x `n`, corner radius `r`
    fn rounded(c: [u8; 4], n: u32, r: f32) -> RgbaImage {
        RgbaImage::from_fn(n, n, |x, y| {
            let (qx, qy) = ((x as f32 + 0.5 - n as f32 / 2.0).abs() - (n as f32 / 2.0 - r), (y as f32 + 0.5 - n as f32 / 2.0).abs() - (n as f32 / 2.0 - r));
            if qx > 0.0 && qy > 0.0 && qx.hypot(qy) > r { Rgba([0, 0, 0, 0]) } else { Rgba(c) }
        })
    }

    #[test]
    fn a_rounded_square_icon_is_not_put_on_a_tile() {
        let style = TileStyle { shape: IconShape::Squircle, padding: 0.16, mode: TileMode::Light };
        let t = bake(&rounded([20, 30, 40, 255], 128, 28.0), &style, 128);
        // no light plate between the edge and the middle: the icon's own colour reaches the edge
        assert_eq!(t.get_pixel(64, 4).0, [20, 30, 40, 255]);
        // a circle or a plain square is still tiled
        let circle = RgbaImage::from_fn(128, 128, |x, y| if (x as f32 - 63.5).hypot(y as f32 - 63.5) < 64.0 { Rgba([20, 30, 40, 255]) } else { Rgba([0; 4]) });
        assert!(is_ready_tile(&circle).is_none());
        assert!(is_ready_tile(&square([20, 30, 40, 255], 128)).is_none());
    }

    #[test]
    fn light_icons_get_a_dark_tile_and_coloured_ones_a_light_one() {
        let style = TileStyle { shape: IconShape::Squircle, padding: 0.3, mode: TileMode::Auto };
        let on_white = bake(&square([255, 255, 255, 255], 32), &style, 128);
        let on_red = bake(&square([200, 30, 30, 255], 32), &style, 128);
        // sample the plate between the icon and the edge
        assert!(on_white.get_pixel(14, 64)[0] < 140, "a white icon sits on a dark tile");
        assert!(on_red.get_pixel(14, 64)[0] > 180, "a red icon sits on a light tile");
    }

    #[test]
    fn a_circle_tile_clips_the_corners_harder() {
        let circle = bake(&square([10, 120, 220, 255], 32), &TileStyle { shape: IconShape::Circle, padding: 0.2, mode: TileMode::Light }, 128);
        let squircle = bake(&square([10, 120, 220, 255], 32), &TileStyle { shape: IconShape::Squircle, padding: 0.2, mode: TileMode::Light }, 128);
        assert_eq!(circle.get_pixel(14, 14)[3], 0);
        assert!(squircle.get_pixel(14, 14)[3] > 0);
    }
}
