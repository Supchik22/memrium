//! Makes an icon in the style of the favourites (gvim, kwalletmanager, discord, Prism Launcher) from a plain SVG icon:
//! the shape that is the icon's own background is taken out and becomes the colour of the plate (a gradient, lighter at the
//! top), the rest, the symbol, is drawn bigger with a little glass on it.

use crate::icondoc::{Appearance, Background, FillKind, Glass, GlassMode, IconDoc, Kind, Props, ShadowKind, Shape};
use crate::iconrender::{effective_props, set_background, set_node_props, Renderer};

/// What one layer of the icon covers: its share of the canvas, its mean colour and how solid it is.
struct Cover {
    id: u32,
    share: f32,
    colour: [f32; 3],
    solid: f32,
    /// how much the colour varies over the shape (0 is one flat colour)
    spread: f32,
    bounds: [u32; 4],
    /// is the shape solid in the middle of its box (a ring is not)
    filled_middle: bool,
}

const N: u32 = 64;

fn measure(doc: &IconDoc, r: &mut Renderer) -> Vec<Cover> {
    let mut out = Vec::new();
    for l in doc.layers() {
        let Some(node) = doc.find(l.id) else { continue };
        let Kind::Layer(layer) = &node.kind else { continue };
        let p = effective_props(node, Appearance::Default);
        let img = r.raster(layer, &p, doc, Appearance::Default, N);
        let (mut area, mut solid, mut sum, mut sq) = (0.0f32, 0.0f32, [0.0f32; 3], [0.0f32; 3]);
        let mut bounds = [N, N, 0u32, 0u32];
        for (x, y, px) in img.enumerate_pixels() {
            let a = px[3] as f32 / 255.0;
            if a > 0.08 {
                area += 1.0;
                solid += a;
                for k in 0..3 {
                    let v = px[k] as f32 / 255.0;
                    sum[k] += v;
                    sq[k] += v * v;
                }
                bounds = [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)];
            }
        }
        if area < 1.0 {
            continue;
        }
        let mean = sum.map(|v| v / area);
        let spread = (0..3).map(|k| (sq[k] / area - mean[k] * mean[k]).max(0.0)).sum::<f32>().sqrt();
        let (mx, my) = ((bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2);
        let filled_middle = img.get_pixel(mx.min(N - 1), my.min(N - 1))[3] > 128;
        out.push(Cover { id: l.id, share: area / (N * N) as f32, colour: mean, solid: solid / area, spread, bounds, filled_middle });
    }
    out
}

fn mix(c: [f32; 3], to: f32, k: f32) -> [f32; 3] {
    c.map(|v| v + (to - v) * k)
}

/// How an icon was treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    /// its background became the plate and a white mark is left on it (Telegram, Discord, Steam)
    WhiteMark,
    /// a bright symbol on a dark plate with its glow (NVIDIA)
    DarkPlate,
    /// the icon is a tile already and keeps its own colour
    Tile,
    /// the picture is kept on a light plate
    Art,
}

/// The document of `svg` in the style of the favourites. `None` when the icon has no shapes.
pub fn favourite_style(name: &str, svg: &str) -> Option<IconDoc> {
    favourite_style_kind(name, svg).map(|(d, _)| d)
}

/// The same, and how the icon was treated.
pub fn favourite_style_kind(name: &str, svg: &str) -> Option<(IconDoc, StyleKind)> {
    let mut doc = crate::iconrender::import_svg(name, svg).ok()?;
    let mut r = Renderer::new();
    let covers = measure(&doc, &mut r);
    // the icon's own background: the biggest shape, when it fills much of the canvas
    let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    // (a plain shape: it fills most of its own box, as a circle or a rounded square does; a head with ears or a shield does not)
    let plain = |c: &Cover| {
        let (w, h) = ((c.bounds[2] - c.bounds[0] + 1) as f32, (c.bounds[3] - c.bounds[1] + 1) as f32);
        c.share * (N * N) as f32 / (w * h) > 0.74
    };
    let base = covers.iter().filter(|c| c.share > 0.45 && c.solid > 0.7 && c.spread < 0.22 && plain(c) && c.filled_middle).max_by(|a, b| a.share.partial_cmp(&b.share).unwrap());
    // the symbol on it has to stand out from its colour, and be big enough to carry the icon, or taking the background away
    // leaves little to see; a grey plate is not wanted either
    let base = base.filter(|b| {
        let rest: Vec<&Cover> = covers.iter().filter(|c| c.id != b.id && c.solid > 0.45).collect();
        let weight: f32 = rest.iter().map(|c| c.share).sum();
        let sat = b.colour.iter().cloned().fold(0.0f32, f32::max) - b.colour.iter().cloned().fold(1.0f32, f32::min);
        let l = lum(b.colour);
        weight > 0.08 && (sat > 0.2 || l < 0.3) && {
            let mean = rest.iter().map(|c| lum(c.colour) * c.share).sum::<f32>() / weight;
            // the background goes only for a white symbol on a colour (Telegram, Steam, Discord) or a bright one on a dark
            // plate (NVIDIA): a grey or coloured symbol on a colour loses too much without its own background
            if l < 0.3 {
                (mean - l).abs() > 0.2
            } else {
                // (and a mark, not a picture that covers most of the background: that one needs it)
                mean > 0.85 && (mean - l).abs() > 0.35 && weight < 0.2
            }
        }
    });
    // shadows drawn in the icon: thin, dark and faint
    let faint: Vec<u32> = covers.iter().filter(|c| c.solid < 0.45 && c.colour.iter().sum::<f32>() / 3.0 < 0.3).map(|c| c.id).collect();
    // an icon that is a tile already (a dark rounded square with a symbol on it, as a monitor or a terminal has) stays one:
    // its plate is the tile's colour, so that no light ring is put round it
    let tile = if base.is_some() { None } else { covers.iter().filter(|c| c.share > 0.5 && c.solid > 0.7 && plain(c)).max_by(|a, b| a.share.partial_cmp(&b.share).unwrap()).map(|c| c.colour) };
    let mut plate = None;
    if let Some(b) = base {
        // a plate in the colour of the background, but not a flat grey one, nor an almost white one
        plate = Some(b.colour);
        doc.remove(b.id);
    }
    for id in faint {
        doc.remove(id);
    }
    let rest: Vec<&Cover> = covers.iter().filter(|c| doc.find(c.id).is_some()).collect();
    // how much of the canvas the symbol takes, to be brought to about 0.62
    let (mut x0, mut y0, mut x1, mut y1) = (N, N, 0, 0);
    for c in &rest {
        x0 = x0.min(c.bounds[0]);
        y0 = y0.min(c.bounds[1]);
        x1 = x1.max(c.bounds[2]);
        y1 = y1.max(c.bounds[3]);
    }
    let extent = if x1 > x0 && y1 > y0 { ((x1 - x0) as f32).max((y1 - y0) as f32) / N as f32 } else { 1.0 };
    let scale = if plate.is_some() { (0.7 / extent.max(0.1)).clamp(0.9, 1.5) } else if tile.is_some() { 1.0 } else { 0.9 };

    // the strongest colour of the symbol (the green of a logo on a dark plate), and whether the symbol is white
    let sat = |c: [f32; 3]| c.iter().cloned().fold(0.0f32, f32::max) - c.iter().cloned().fold(1.0f32, f32::min);
    let accent = rest.iter().filter(|c| sat(c.colour) > 0.35).max_by(|a, b| (a.share * sat(a.colour)).partial_cmp(&(b.share * sat(b.colour))).unwrap()).map(|c| c.colour);
    let symbol_weight: f32 = rest.iter().map(|c| c.share).sum::<f32>().max(1e-3);
    let symbol_lum: f32 = rest.iter().map(|c| lum(c.colour) * c.share).sum::<f32>() / symbol_weight;
    let white_symbol = symbol_lum > 0.85 && accent.is_none();
    let base_plate = match plate {
        // a dark plate gets the glow of the symbol's colour along the bottom, as the logos of dark apps have
        Some(c) if lum(c) < 0.3 && accent.is_some() => {
            let a = accent.unwrap();
            Background {
                shape: Shape::Squircle,
                fill: FillKind::Gradient,
                top: [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85],
                bottom: [c[0] * 0.7 + a[0] * 0.2, c[1] * 0.7 + a[1] * 0.2, c[2] * 0.7 + a[2] * 0.2].map(|v| v.min(1.0)),
                glass: Glass { enabled: true, specular: 0.45, shadow: ShadowKind::Neutral, shadow_amount: 0.35, bevel: 0.4, inner_shadow: 0.35, ..Glass::default() },
            }
        }
        Some(c) => {
            let lum = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
            // light at the top, deeper at the bottom; a very light colour is kept as it is
            let (top, bottom) = if lum > 0.9 { ([0.97, 0.97, 0.99], [0.86, 0.87, 0.92]) } else { (mix(c, 1.0, 0.16), c.map(|v| v * 0.86)) };
            Background { shape: Shape::Squircle, fill: FillKind::Gradient, top, bottom, glass: Glass { enabled: true, specular: 0.7, shadow: ShadowKind::Neutral, shadow_amount: 0.35, bevel: 0.3, inner_shadow: 0.15, ..Glass::default() } }
        }
        None if tile.is_some() => {
            let c = tile.unwrap();
            Background { shape: Shape::Squircle, fill: FillKind::Gradient, top: mix(c, 1.0, 0.05), bottom: c.map(|v| v * 0.95), glass: Glass { enabled: true, specular: 0.5, shadow: ShadowKind::Neutral, shadow_amount: 0.35, bevel: 0.25, inner_shadow: 0.15, ..Glass::default() } }
        }
        None => Background { shape: Shape::Squircle, fill: FillKind::Gradient, top: [0.97, 0.97, 0.99], bottom: [0.86, 0.87, 0.92], glass: Glass { enabled: true, specular: 0.5, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() } },
    };
    set_background(&mut doc, Appearance::Default, base_plate);
    doc.margin = 0.0;

    // the symbol: bigger, with glass on each layer
    let ids: Vec<u32> = rest.iter().map(|c| c.id).collect();
    let rest_count = ids.len();
    for id in ids {
        if let Some(node) = doc.find_mut(id) {
            let mut p: Props = effective_props(node, Appearance::Default);
            p.scale = scale;
            // a white symbol is frosted glass with a soft bevel, a coloured one only has light on its edges
            // glass on every little shape of a busy picture makes it look like clay: only a logo of a few shapes gets it
            p.glass = if (plate.is_none() && rest_count > 5) || tile.is_some() {
                Glass { enabled: false, ..Glass::default() }
            } else if white_symbol {
                Glass { enabled: true, specular: 0.9, blur: 0.0, refraction: 0.6, translucency: 0.0, shadow: ShadowKind::Neutral, shadow_amount: 0.35, bevel: 0.3, bevel_width: 0.35, inner_shadow: 0.0, ..Glass::default() }
            } else {
                Glass { enabled: true, specular: 0.7, blur: 0.0, refraction: 0.5, translucency: 0.0, shadow: ShadowKind::Neutral, shadow_amount: 0.3, bevel: 0.3, ..Glass::default() }
            };
            p.glass_mode = GlassMode::Individual;
            set_node_props(node, Appearance::Default, p);
        }
    }
    let kind = match (plate, white_symbol) {
        (Some(_), true) => StyleKind::WhiteMark,
        (Some(_), false) => StyleKind::DarkPlate,
        (None, _) if tile.is_some() => StyleKind::Tile,
        _ => StyleKind::Art,
    };
    Some((doc, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISC: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><circle cx="32" cy="32" r="30" fill="#5865f2"/><rect x="18" y="24" width="28" height="16" fill="#ffffff"/></svg>"##;

    #[test]
    fn the_background_shape_becomes_the_plate_and_the_symbol_grows() {
        let doc = favourite_style("t", DISC).unwrap();
        assert_eq!(doc.layers().len(), 1, "only the symbol is left");
        let plate = crate::iconrender::background_for(&doc, Appearance::Default);
        assert_eq!(plate.shape, Shape::Squircle);
        assert!(plate.bottom[2] > plate.bottom[0], "the plate is the blue of the circle: {:?}", plate.bottom);
        let id = doc.layers()[0].id;
        assert!(effective_props(doc.find(id).unwrap(), Appearance::Default).scale > 1.2, "the symbol is bigger");
        let img = Renderer::new().render(&doc, Appearance::Default, 128);
        assert!(img.get_pixel(64, 64)[0] > 200, "white symbol in the middle");
    }
}
