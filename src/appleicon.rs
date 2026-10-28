//! Reads Apple's `.icon` format (what Icon Composer saves): a folder with `icon.json` and the layers as SVG files in `Assets`.
//!
//! The JSON lists the groups from the front to the back, each with its layers (front to back too) and the Liquid Glass of
//! the group: specular light, translucency, refractivity, blur and shadow. A setting can have specializations for the
//! looks of the icon ("dark", "tinted"); what has none is for the default look. The plate is the `fill` of the whole icon.
//! Colours come as `display-p3:r,g,b,a`, `srgb:...` or `gray:v,a`.
//!
//! The document made of it keeps what our model can say: layers as they are drawn (SVG), a colour for a layer that has a
//! fill (the middle of a gradient's colours when it is one), each group as one piece of glass, the plate as a gradient made
//! of the automatic-gradient colour, and the settings of the dark and tinted looks.

use crate::icondoc::{Appearance, Background, Blend, FillKind, Glass, GlassMode, Group, IconDoc, Kind, Layer, Node, Props, ShadowKind, Shape, Variants};
use crate::iconrender::{effective_props, set_background, set_node_props};
use serde_json::Value;
use std::path::Path;

/// Is this the folder of an Apple icon?
pub fn is_icon_dir(path: &Path) -> bool {
    path.is_dir() && path.join("icon.json").is_file()
}

/// The `.icon` folders in `dirs` (not looking deeper), by name.
pub fn find_icons(dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "icon") && is_icon_dir(&p) {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

// ----------------------------------------------------------------------------------------------- colours

fn eotf(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn oetf(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Display P3 to sRGB (through linear light and XYZ), clipped to what sRGB can show.
fn p3_to_srgb(c: [f32; 3]) -> [f32; 3] {
    const P3_TO_XYZ: [[f32; 3]; 3] = [[0.4865709, 0.2656677, 0.1982173], [0.2289746, 0.6917385, 0.0792869], [0.0, 0.0451134, 1.0439444]];
    const XYZ_TO_SRGB: [[f32; 3]; 3] = [[3.2404542, -1.5371385, -0.4985314], [-0.969266, 1.8760108, 0.041556], [0.0556434, -0.2040259, 1.0572252]];
    let lin = [eotf(c[0]), eotf(c[1]), eotf(c[2])];
    let xyz: Vec<f32> = P3_TO_XYZ.iter().map(|r| r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2]).collect();
    let rgb: Vec<f32> = XYZ_TO_SRGB.iter().map(|r| r[0] * xyz[0] + r[1] * xyz[1] + r[2] * xyz[2]).collect();
    [oetf(rgb[0]), oetf(rgb[1]), oetf(rgb[2])]
}

/// `display-p3:0.2,0.5,1,1` and the like, as sRGB.
fn colour(text: &str) -> Option<[f32; 3]> {
    let (space, nums) = text.split_once(':')?;
    let v: Vec<f32> = nums.split(',').filter_map(|t| t.trim().parse().ok()).collect();
    match (space, v.as_slice()) {
        ("display-p3", [r, g, b, ..]) => Some(p3_to_srgb([*r, *g, *b])),
        ("srgb" | "extended-srgb", [r, g, b, ..]) => Some([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]),
        ("gray" | "extended-gray", [v, ..]) => Some([v.clamp(0.0, 1.0); 3]),
        _ => None,
    }
}

/// The colour a fill stands for: a solid one, the colour of an automatic gradient, or the middle of a gradient.
fn fill_colour(v: &Value) -> Option<[f32; 3]> {
    // the colours the system gives to a light and a dark icon
    match v.as_str() {
        Some("system-light") => return Some([1.0, 1.0, 1.0]),
        Some("system-dark") => return Some([0.14, 0.14, 0.16]),
        _ => {}
    }
    if let Some(s) = v.get("solid").and_then(Value::as_str) {
        return colour(s);
    }
    if let Some(s) = v.get("automatic-gradient").and_then(Value::as_str) {
        return colour(s);
    }
    let stops: Vec<[f32; 3]> = v.get("linear-gradient")?.as_array()?.iter().filter_map(|s| colour(s.as_str()?)).collect();
    if stops.is_empty() {
        return None;
    }
    let n = stops.len() as f32;
    Some([0, 1, 2].map(|k| stops.iter().map(|c| c[k]).sum::<f32>() / n))
}

/// A linear gradient of a fill: its colours from the start to the stop (as fractions of the shape), when it is one.
struct Gradient {
    stops: Vec<[f32; 3]>,
    start: [f32; 2],
    stop: [f32; 2],
}

fn gradient_of(v: &Value) -> Option<Gradient> {
    let stops: Vec<[f32; 3]> = v.get("linear-gradient")?.as_array()?.iter().filter_map(|s| colour(s.as_str()?)).collect();
    if stops.len() < 2 {
        return None;
    }
    let point = |key: &str, default: [f32; 2]| {
        v.get("orientation").and_then(|o| o.get(key)).map_or(default, |p| [p.get("x").and_then(Value::as_f64).unwrap_or(0.5) as f32, p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32])
    };
    // without an orientation the gradient goes from the top of the shape to its bottom
    Some(Gradient { stops, start: point("start", [0.5, 0.0]), stop: point("stop", [0.5, 1.0]) })
}

fn hex(c: [f32; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

/// The SVG with everything it fills painted with the gradient (laid over the bounds of each shape).
fn paint_with_gradient(svg: &str, g: &Gradient) -> String {
    // the fills the shapes have: attributes and the declarations of a style sheet
    fn replace_fills(text: &str, marker: &str, end: &[char], paint: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find(marker) {
            let value_at = at + marker.len();
            out.push_str(&rest[..value_at]);
            let tail = &rest[value_at..];
            let stop = tail.find(end).unwrap_or(tail.len());
            let value = tail[..stop].trim();
            out.push_str(if value == "none" { &tail[..stop] } else { paint });
            rest = &tail[stop..];
        }
        out.push_str(rest);
        out
    }
    let n = g.stops.len();
    let stops: String = g.stops.iter().enumerate().map(|(i, c)| format!("<stop offset=\"{}\" stop-color=\"{}\"/>", i as f32 / (n - 1) as f32, hex(*c))).collect();
    let def = format!("<defs><linearGradient id=\"mem-gradient\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" gradientUnits=\"objectBoundingBox\">{stops}</linearGradient></defs>", g.start[0], g.start[1], g.stop[0], g.stop[1]);
    let paint = "url(#mem-gradient)";
    let text = replace_fills(svg, "fill=\"", &['"'], paint);
    let text = replace_fills(&text, "fill:", &[';', '}', '\n'], paint);
    match text.find("<svg").and_then(|a| text[a..].find('>').map(|b| a + b + 1)) {
        Some(end) => format!("{}{def}{}", &text[..end], &text[end..]),
        None => text,
    }
}

// ----------------------------------------------------------------------------------------------- the settings

/// The looks that have their own setting in the file, with the look of ours each one stands for.
fn appearance_of(name: &str) -> Option<Appearance> {
    match name {
        "dark" => Some(Appearance::Dark),
        "tinted" => Some(Appearance::TintedLight),
        "clear" => Some(Appearance::ClearLight),
        _ => None,
    }
}

/// The entries of a setting's specializations: (appearance, value), the one without an appearance having `None`.
fn entries<'a>(obj: &'a Value, specs: &str) -> Vec<(Option<&'a str>, &'a Value)> {
    obj.get(specs).and_then(Value::as_array).map(|a| a.iter().filter_map(|s| Some((s.get("appearance").and_then(Value::as_str), s.get("value")?))).collect()).unwrap_or_default()
}

/// The value of a setting in the default look: the plain key, else the entry for "light" (the default look is the light
/// one), else the entry without an appearance.
fn default_of<'a>(obj: &'a Value, plain: &str, specs: &str) -> Option<&'a Value> {
    if let Some(v) = obj.get(plain).filter(|v| !v.is_null()) {
        return Some(v);
    }
    let e = entries(obj, specs);
    e.iter().find(|(a, _)| *a == Some("light")).or_else(|| e.iter().find(|(a, _)| a.is_none())).map(|(_, v)| *v)
}

/// The value of a setting for one look, if the file says something for it: its own entry, or, when the default look has an
/// entry of its own ("light"), the general one, which is what the other looks have.
fn special_of<'a>(obj: &'a Value, specs: &str, look: Appearance) -> Option<&'a Value> {
    let e = entries(obj, specs);
    if let Some((_, v)) = e.iter().find(|(a, _)| a.and_then(appearance_of) == Some(look)) {
        return Some(*v);
    }
    if e.iter().any(|(a, _)| *a == Some("light")) {
        return e.iter().find(|(a, _)| a.is_none()).map(|(_, v)| *v);
    }
    None
}

fn shadow_of(v: &Value) -> (ShadowKind, f32) {
    let kind = match v.get("kind").and_then(Value::as_str) {
        Some("none") => ShadowKind::None,
        Some("neutral") => ShadowKind::Neutral,
        _ => ShadowKind::Chromatic,
    };
    // the shadow here is drawn a little fainter than Apple's, so its opacity is raised to look the same
    (kind, (v.get("opacity").and_then(Value::as_f64).unwrap_or(0.5) as f32 * 1.5).min(1.0))
}

/// `{enabled, value}` as the see-through amount. Apple's translucency lets the picture behind tint the layer but keeps its
/// colour; here it takes the layer's alpha, so about a fifth of it gives the same look.
fn translucency_of(v: &Value) -> f32 {
    if v.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
        v.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32 * 0.2
    } else {
        0.0
    }
}

/// The glass of a group in its default look.
fn group_glass(g: &Value) -> Glass {
    let specular = match default_of(g, "specular", "specular-specializations") {
        Some(Value::Bool(on)) => *on,
        Some(Value::String(_)) => true,
        _ => false,
    };
    let translucency = default_of(g, "translucency", "translucency-specializations").map_or(0.0, translucency_of);
    let (shadow, shadow_amount) = default_of(g, "shadow", "shadow-specializations").map_or((ShadowKind::None, 0.0), shadow_of);
    let refraction = g.get("refractivity").filter(|r| r.get("enabled").and_then(Value::as_bool).unwrap_or(false)).and_then(|r| r.get("strength")?.as_f64()).unwrap_or(0.0).max(0.0) as f32;
    let blur = g.get("blur-material").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    Glass {
        enabled: specular || translucency > 0.0 || shadow != ShadowKind::None,
        specular: if specular { 0.7 } else { 0.0 },
        blur,
        refraction,
        translucency,
        shadow,
        shadow_amount,
        ..Glass::default()
    }
}

/// A node's settings in one look, from the file's specializations for that look on top of what the look has already.
fn tweak(mut p: Props, node: &Value, look: Appearance, group: bool) -> Props {
    if let Some(v) = special_of(node, "opacity-specializations", look).and_then(Value::as_f64) {
        p.opacity = v as f32;
    }
    if let Some(v) = special_of(node, "fill-specializations", look).and_then(fill_colour) {
        p.fill = Some(v);
    }
    if let Some(h) = special_of(node, "hidden-specializations", look).and_then(Value::as_bool) {
        p.visible = !h;
    }
    if let Some(b) = special_of(node, "blend-mode-specializations", look).and_then(Value::as_str).and_then(blend_of) {
        p.blend = b;
    }
    if group {
        if let Some(on) = special_of(node, "specular-specializations", look).and_then(Value::as_bool) {
            p.glass.specular = if on { 0.7 } else { 0.0 };
        }
        if let Some(v) = special_of(node, "translucency-specializations", look) {
            p.glass.translucency = translucency_of(v);
        }
        if let Some(v) = special_of(node, "shadow-specializations", look) {
            let (kind, amount) = shadow_of(v);
            p.glass.shadow = kind;
            p.glass.shadow_amount = amount;
            p.glass.enabled |= kind != ShadowKind::None;
        }
    }
    p
}

fn has_special(node: &Value, look: Appearance, keys: &[&str]) -> bool {
    keys.iter().any(|k| special_of(node, k, look).is_some())
}

// ----------------------------------------------------------------------------------------------- pictures and placement

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The SVG a layer is made of: the file itself, or, for a PNG or JPEG, a picture in an SVG on the 1024 point canvas, where
/// one pixel is one point (the layer's own scale then makes it fit).
fn layer_svg(path: &Path) -> Result<String, String> {
    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string());
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    if ext == "svg" {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {name}: {e}"))?;
        return Ok(on_canvas(&text, 1.0));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {name}: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{name} is not a picture: {e}"))?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let mime = if ext == "png" { "png" } else { "jpeg" };
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 1024 1024\"><image x=\"{}\" y=\"{}\" width=\"{w}\" height=\"{h}\" xlink:href=\"data:image/{mime};base64,{}\"/></svg>",
        (1024.0 - w) / 2.0,
        (1024.0 - h) / 2.0,
        base64(&bytes)
    ))
}

/// A picture in an SVG painted in one colour wherever it is drawn (its alpha is the shape).
fn tint_picture(svg: &str, c: [f32; 3]) -> String {
    let filter = format!("<defs><filter id=\"mem-tint\" x=\"0\" y=\"0\" width=\"1\" height=\"1\"><feFlood flood-color=\"{}\"/><feComposite in2=\"SourceAlpha\" operator=\"in\"/></filter></defs>", hex(c));
    let svg = svg.replacen("<image ", "<image filter=\"url(#mem-tint)\" ", 1);
    match svg.find("<svg").and_then(|a| svg[a..].find('>').map(|b| a + b + 1)) {
        Some(end) => format!("{}{filter}{}", &svg[..end], &svg[end..]),
        None => svg,
    }
}

/// An SVG that is not 1024 points square, put in the middle of the 1024 point canvas at its own size: the layer's scale is
/// a factor of that size, so a small logo with a scale of 10 fills the icon.
fn on_canvas(svg: &str, scale: f32) -> String {
    let Some(open_at) = svg.find("<svg") else { return svg.to_string() };
    let Some(open_len) = svg[open_at..].find('>') else { return svg.to_string() };
    let tag = &svg[open_at..open_at + open_len];
    let attr = |name: &str| -> Option<String> {
        let key = format!("{name}=\"");
        let from = tag.find(&format!(" {key}"))? + key.len() + 1;
        Some(tag[from..from + tag[from..].find('"')?].to_string())
    };
    let num = |s: String| s.trim_end_matches("px").trim().parse::<f32>().ok();
    let view: Option<Vec<f32>> = attr("viewBox").map(|v| v.split(|c: char| c == ' ' || c == ',').filter_map(|t| t.parse().ok()).collect());
    let (w, h) = match (attr("width").and_then(num), attr("height").and_then(num), view.as_deref()) {
        (Some(w), Some(h), _) => (w, h),
        (_, _, Some([_, _, w, h])) => (*w, *h),
        _ => return svg.to_string(),
    };
    if (w - 1024.0).abs() < 0.5 && (h - 1024.0).abs() < 0.5 && (scale - 1.0).abs() < 1e-3 {
        return svg.to_string();
    }
    // (it is drawn at its size times the scale: bigger is drawn bigger, not stretched from a small picture)
    let (vw, vh) = (w, h);
    let (w, h) = (w * scale, h * scale);
    // the same root tag without its size, with the place instead
    let mut inner_tag = tag.to_string();
    for name in ["width", "height", "x", "y"] {
        if let Some(a) = inner_tag.find(&format!(" {name}=\"")) {
            let from = a + name.len() + 3;
            if let Some(len) = inner_tag[from..].find('"') {
                inner_tag.replace_range(a..from + len + 1, "");
            }
        }
    }
    let view_box = if attr("viewBox").is_some() { String::new() } else { format!(" viewBox=\"0 0 {vw} {vh}\"") };
    let inner_tag = inner_tag.replacen("<svg", &format!("<svg x=\"{}\" y=\"{}\" width=\"{w}\" height=\"{h}\"{view_box}", (1024.0 - w) / 2.0, (1024.0 - h) / 2.0), 1);
    let body_start = open_at + open_len + 1;
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">{inner_tag}>{}</svg>", &svg[body_start..])
}

fn blend_of(name: &str) -> Option<Blend> {
    match name {
        "normal" => Some(Blend::Normal),
        "multiply" => Some(Blend::Multiply),
        "screen" => Some(Blend::Screen),
        "overlay" => Some(Blend::Overlay),
        "darken" => Some(Blend::Darken),
        "lighten" => Some(Blend::Lighten),
        "plus-lighter" | "plus-darker" => Some(Blend::Plus),
        _ => None,
    }
}

/// The opacity, blend mode and placement a layer or group has by itself (the points are those of the 1024 point canvas).
fn placement(node: &Value, p: &mut Props) {
    if let Some(o) = node.get("opacity").and_then(Value::as_f64) {
        p.opacity = o as f32;
    }
    if let Some(b) = node.get("blend-mode").and_then(Value::as_str).and_then(blend_of) {
        p.blend = b;
    }
    if let Some(pos) = node.get("position") {
        if let Some(s) = pos.get("scale").and_then(Value::as_f64) {
            p.scale = s as f32;
        }
        if let Some(t) = pos.get("translation-in-points").and_then(Value::as_array) {
            if let [x, y] = t.as_slice() {
                p.offset = [x.as_f64().unwrap_or(0.0) as f32 / 1024.0, y.as_f64().unwrap_or(0.0) as f32 / 1024.0];
            }
        }
    }
    if node.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
        p.visible = false;
    }
}

// ----------------------------------------------------------------------------------------------- the document

/// The document made of the `.icon` folder `dir`.
pub fn import(dir: &Path) -> Result<IconDoc, String> {
    let text = std::fs::read_to_string(dir.join("icon.json")).map_err(|e| format!("no icon.json in {}: {e}", dir.display()))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| format!("icon.json is not valid: {e}"))?;
    let name = dir.file_stem().map_or("icon".to_string(), |s| s.to_string_lossy().to_string());
    let mut doc = IconDoc::new(&name);
    doc.margin = 0.04;

    // the plate: a gradient round the colour of the file's fill
    // (the fill can be given only for some looks: the default one is what the plate is made of)
    let fill = json.get("fill").or_else(|| default_of(&json, "fill", "fill-specializations"));
    if let Some(c) = fill.and_then(fill_colour) {
        let lighter = |k: f32| c.map(|v| v + (1.0 - v) * k);
        let darker = |k: f32| c.map(|v| v * (1.0 - k));
        // two colours make the plate as they are; one is made lighter at the top and darker at the bottom
        let (top, bottom) = match fill.and_then(gradient_of) {
            Some(g) => (g.stops[0], g.stops[g.stops.len() - 1]),
            // the system's own light plate is white, a little darker at the bottom
            None if fill.and_then(Value::as_str) == Some("system-light") => ([1.0, 1.0, 1.0], [0.90, 0.91, 0.94]),
            None => (lighter(0.22), darker(0.08)),
        };
        // what the file leaves to the system stays that way: a light plate, or one colour made into a gradient
        let kind = match fill.and_then(Value::as_str) {
            Some("system-light") => FillKind::System,
            _ if fill.is_some_and(|f| f.get("automatic-gradient").is_some()) => FillKind::Auto,
            _ => FillKind::Gradient,
        };
        let (top, bottom) = if kind == FillKind::Auto { (c, c) } else { (top, bottom) };
        let base = Background {
            shape: Shape::Squircle,
            fill: kind,
            top,
            bottom,
            glass: Glass { enabled: true, specular: 0.3, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() },
        };
        doc.background = Variants::new(base);
        // the dark look keeps the base every dark icon has; set again so that it stays what the system makes of it
        set_background(&mut doc, Appearance::Default, base);
    }

    let groups = json.get("groups").and_then(Value::as_array).ok_or("icon.json has no groups")?;
    let mut nodes: Vec<Node> = Vec::new();
    // the file lists the front first; a document lists the back first
    for (gi, g) in groups.iter().enumerate().rev() {
        let layers = g.get("layers").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut kids: Vec<Node> = Vec::new();
        for l in layers.iter().rev() {
            let file = l.get("image-name").and_then(Value::as_str).ok_or("a layer has no image-name")?;
            let mut svg = layer_svg(&dir.join("Assets").join(file))?;
            // a vector layer made bigger is drawn bigger, so that it stays sharp
            let magnify = l.get("position").and_then(|p| p.get("scale")).and_then(Value::as_f64).map_or(1.0, |v| v as f32);
            let baked = file.ends_with(".svg") && magnify > 1.01;
            if baked {
                svg = on_canvas(&std::fs::read_to_string(dir.join("Assets").join(file)).map_err(|e| e.to_string())?, magnify);
            }
            let lname = l.get("name").and_then(Value::as_str).unwrap_or(file).to_string();
            let mut props = Props::default();
            match default_of(l, "fill", "fill-specializations") {
                Some(v) => match gradient_of(v) {
                    Some(g) => svg = paint_with_gradient(&svg, &g),
                    None => props.fill = fill_colour(v),
                },
                None => {}
            }
            placement(l, &mut props);
            if baked {
                props.scale = 1.0;
            }
            if let Some(h) = default_of(l, "hidden", "hidden-specializations").and_then(Value::as_bool) {
                props.visible = !h;
            }
            if let Some(o) = special_of(l, "opacity-specializations", Appearance::Default).and_then(Value::as_f64) {
                props.opacity = o as f32;
            }
            // a picture is coloured by its shape (its alpha), which the layer's fill cannot do: bake the colour in
            if let (true, Some(c)) = (svg.contains("<image"), props.fill) {
                svg = tint_picture(&svg, c);
                props.fill = None;
            }
            let id = doc.fresh_id();
            let mut node = Node { id, name: lname, kind: Kind::Layer(Layer { svg, props: Variants::new(props) }) };
            for look in [Appearance::Dark, Appearance::TintedLight, Appearance::ClearLight] {
                if has_special(l, look, &["fill-specializations", "opacity-specializations", "blend-mode-specializations", "hidden-specializations"]) {
                    let p = tweak(effective_props(&node, look), l, look, false);
                    set_node_props(&mut node, look, p);
                }
            }
            kids.push(node);
        }
        let glass = group_glass(g);
        let lighting = match g.get("lighting").and_then(Value::as_str) {
            Some("individual") => GlassMode::Individual,
            _ => GlassMode::Combined,
        };
        let mut props = Props { glass, glass_mode: if glass.enabled { lighting } else { GlassMode::Individual }, ..Props::default() };
        placement(g, &mut props);
        let gname = match kids.as_slice() {
            [only] => only.name.clone(),
            _ => format!("Group {}", groups.len() - gi),
        };
        let id = doc.fresh_id();
        let mut node = Node { id, name: gname, kind: Kind::Group(Group { children: kids, props: Variants::new(props) }) };
        for look in [Appearance::Dark, Appearance::TintedLight, Appearance::ClearLight] {
            if has_special(g, look, &["translucency-specializations", "shadow-specializations", "opacity-specializations", "specular-specializations"]) {
                let p = tweak(effective_props(&node, look), g, look, true);
                set_node_props(&mut node, look, p);
            }
        }
        nodes.push(node);
    }
    doc.nodes = nodes;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_come_out_as_srgb() {
        assert_eq!(colour("gray:0.5,1.0"), Some([0.5; 3]));
        assert_eq!(colour("srgb:1,0,0,1"), Some([1.0, 0.0, 0.0]));
        let blue = colour("display-p3:0.19583,0.49307,1.00000,1.00000").unwrap();
        assert!(blue[2] > 0.99 && blue[0] < 0.2 && blue[1] > 0.4, "the iOS-like blue stays blue: {blue:?}");
        let white = colour("display-p3:1,1,1,1").unwrap();
        assert!(white.iter().all(|c| *c > 0.99), "white stays white");
    }

    #[test]
    fn a_gradient_is_baked_into_the_svg_of_a_layer() {
        let v: Value = serde_json::from_str(r#"{"linear-gradient":["srgb:1,0,0,1","srgb:0,0,1,1"],"orientation":{"start":{"x":0.5,"y":1},"stop":{"x":0.5,"y":0}}}"#).unwrap();
        let g = gradient_of(&v).unwrap();
        assert_eq!((g.start, g.stop), ([0.5, 1.0], [0.5, 0.0]));
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><style>.a{fill:#00ff00;}</style><rect class="a" width="100" height="100"/><rect fill="none" width="5" height="5"/></svg>"##;
        let out = paint_with_gradient(svg, &g);
        assert!(out.contains("fill:url(#mem-gradient)") && out.contains("fill=\"none\""), "{out}");
        // red at the bottom and blue at the top
        let doc = crate::iconrender::import_svg("t", &out).unwrap();
        let img = { let mut r = crate::iconrender::Renderer::new(); r.tune.plate = Shape::None; r }.render(&doc, Appearance::Default, 100);
        let (top, bottom) = (img.get_pixel(50, 8), img.get_pixel(50, 92));
        assert!(top[2] > 200 && top[0] < 60, "blue at the top: {top:?}");
        assert!(bottom[0] > 200 && bottom[2] < 60, "red at the bottom: {bottom:?}");
    }

    #[test]
    fn a_small_svg_sits_in_the_middle_of_the_canvas_at_its_own_size() {
        let svg = r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><rect width="100" height="100" fill="#ff0000"/></svg>"##;
        let out = on_canvas(svg, 1.0);
        let doc = crate::iconrender::import_svg("t", &out).unwrap();
        let img = { let mut r = crate::iconrender::Renderer::new(); r.tune.plate = Shape::None; r }.render(&doc, Appearance::Default, 1024);
        assert_eq!(img.get_pixel(512, 512)[3], 255, "the middle is drawn");
        assert_eq!(img.get_pixel(100, 100)[3], 0, "and the rest of the canvas is empty: it is 100 points, not 1024");
        let big = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"/>"##;
        assert_eq!(on_canvas(big, 1.0), big, "a canvas-sized SVG is left alone");
    }

    #[test]
    fn the_settings_of_a_group_are_read() {
        let g: Value = serde_json::from_str(
            r#"{"specular":"inside","blur-material":0.05,"refractivity":{"enabled":true,"depth":0.25,"strength":0.25},
                "shadow-specializations":[{"value":{"kind":"layer-color","opacity":0.6}},{"appearance":"dark","value":{"kind":"neutral","opacity":0.4}}],
                "translucency-specializations":[{"value":{"enabled":true,"value":0.3}}]}"#,
        )
        .unwrap();
        let glass = group_glass(&g);
        assert!(glass.enabled && glass.specular > 0.0);
        assert_eq!(glass.shadow, ShadowKind::Chromatic);
        assert!((glass.translucency - 0.06).abs() < 1e-6 && (glass.refraction - 0.25).abs() < 1e-6);
        let dark = tweak(Props { glass, ..Props::default() }, &g, Appearance::Dark, true);
        assert_eq!(dark.glass.shadow, ShadowKind::Neutral, "the dark look has its own shadow");
        assert!((dark.glass.shadow_amount - 0.6).abs() < 1e-6);
    }

    /// The icon on the desktop of the author, when it is there.
    #[test]
    fn a_real_icon_is_imported_front_to_back() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Desktop/Clock.icon");
        if !is_icon_dir(&dir) {
            return;
        }
        let doc = import(&dir).unwrap();
        assert_eq!(doc.name, "Clock");
        assert_eq!(doc.nodes.len(), 4, "four groups");
        assert_eq!(doc.nodes[0].children().len(), 2, "the face and its bevel are at the back");
        assert_eq!(doc.nodes.last().unwrap().children().len(), 3, "the pin, the second hand and the index are in front");
        let img = { let mut r = crate::iconrender::Renderer::new(); r.tune.plate = Shape::None; r }.render(&doc, Appearance::Default, 256);
        assert!(img.get_pixel(128, 128)[3] > 0);
    }
}
