//! Editing the parts of an SVG icon: list its shapes, hide (delete) some, give others another fill or stroke colour.
//! The edits are kept as data (`ElementEdit`, saved with the icon styles) and applied to the SVG text before it is
//! drawn, so the original file is never touched.

use image::RgbaImage;
use resvg::usvg::roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// What was done to the shape number `index` (counted in document order, shapes inside `<defs>` and clip paths excluded).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ElementEdit {
    pub index: usize,
    pub hidden: bool,
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
}

impl ElementEdit {
    pub fn is_empty(&self) -> bool {
        !self.hidden && self.fill.is_none() && self.stroke.is_none()
    }
}

/// A shape of the drawing, as shown in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub index: usize,
    pub tag: String,
    pub id: String,
    /// the colour it is filled with, if any (a gradient shows its first colour)
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
}

/// How the shapes of a drawing are grouped: a shape (its number among `SvgDoc::elements`) or a group of them.
#[derive(Clone, Debug, PartialEq)]
pub enum Outline {
    Shape(usize),
    Group { name: String, children: Vec<Outline> },
}

pub struct SvgDoc {
    text: String,
}

const SHAPES: [&str; 7] = ["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"];
const NOT_DRAWN: [&str; 10] = ["defs", "clipPath", "mask", "pattern", "symbol", "marker", "linearGradient", "radialGradient", "filter", "style"];

fn is_shape(n: Node) -> bool {
    n.is_element() && SHAPES.contains(&n.tag_name().name()) && !n.ancestors().any(|a| a != n && a.is_element() && NOT_DRAWN.contains(&a.tag_name().name()))
}

/// The value of CSS-like property `name` of `n`: its attribute or its `style` declaration (the style wins).
fn own_property(n: Node, name: &str) -> Option<String> {
    if let Some(style) = n.attribute("style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                if k.trim() == name {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    n.attribute(name).map(|v| v.trim().to_string())
}

/// The property as it applies to `n`: its own, else its nearest ancestor's.
fn property(n: Node, name: &str) -> Option<String> {
    n.ancestors().filter(|a| a.is_element()).find_map(|a| own_property(a, name).filter(|v| v != "inherit"))
}

fn named(name: &str) -> Option<[f32; 3]> {
    Some(match name {
        "black" => [0.0, 0.0, 0.0],
        "white" => [1.0, 1.0, 1.0],
        "red" => [1.0, 0.0, 0.0],
        "green" => [0.0, 0.5, 0.0],
        "blue" => [0.0, 0.0, 1.0],
        "yellow" => [1.0, 1.0, 0.0],
        "gray" | "grey" => [0.5, 0.5, 0.5],
        "orange" => [1.0, 0.647, 0.0],
        _ => return None,
    })
}

/// "#rgb", "#rrggbb", "rgb(r,g,b)" or a few names as RGB 0..1. `none` and anything else: None.
pub fn parse_colour(text: &str) -> Option<[f32; 3]> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix('#') {
        return crate::colour::parse_hex(h);
    }
    if let Some(args) = t.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let v: Vec<f32> = args
            .split(',')
            .map(|p| {
                let p = p.trim();
                match p.strip_suffix('%') {
                    Some(pc) => pc.trim().parse::<f32>().ok().map(|x| x / 100.0),
                    None => p.parse::<f32>().ok().map(|x| x / 255.0),
                }
            })
            .collect::<Option<Vec<f32>>>()?;
        return (v.len() == 3).then(|| [v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0), v[2].clamp(0.0, 1.0)]);
    }
    named(t)
}

/// The colour of a paint value: a colour, or the first stop of the gradient it points to.
fn paint_colour(doc: &Document, value: &str) -> Option<[f32; 3]> {
    if let Some(id) = value.strip_prefix("url(#").and_then(|r| r.split(')').next()) {
        let mut node = doc.descendants().find(|n| n.attribute("id") == Some(id))?;
        for _ in 0..4 {
            if let Some(stop) = node.children().find(|c| c.is_element() && c.tag_name().name() == "stop") {
                let c = own_property(stop, "stop-color").unwrap_or_else(|| "black".into());
                return parse_colour(&c);
            }
            let href = node.attribute(("http://www.w3.org/1999/xlink", "href")).or_else(|| node.attribute("href"))?;
            node = doc.descendants().find(|n| n.attribute("id") == href.strip_prefix('#'))?;
        }
        return None;
    }
    parse_colour(value)
}

fn escape(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

fn hex(c: [f32; 3]) -> String {
    crate::colour::to_hex(c)
}

impl SvgDoc {
    pub fn open(path: &Path) -> Option<SvgDoc> {
        if path.extension().and_then(|e| e.to_str()) != Some("svg") {
            return None;
        }
        let text = std::fs::read_to_string(path).ok()?;
        Document::parse_with_options(&text, resvg::usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).ok()?;
        Some(SvgDoc { text })
    }

    pub fn from_text(text: &str) -> Option<SvgDoc> {
        Document::parse(text).ok()?;
        Some(SvgDoc { text: text.to_string() })
    }

    fn parse(&self) -> Document<'_> {
        Document::parse_with_options(&self.text, resvg::usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).expect("checked when opened")
    }

    /// The shapes of the drawing in document order.
    pub fn elements(&self) -> Vec<Element> {
        let doc = self.parse();
        doc.descendants()
            .filter(|n| is_shape(*n))
            .enumerate()
            .map(|(index, n)| Element {
                index,
                tag: n.tag_name().name().to_string(),
                id: n.attribute("id").unwrap_or("").to_string(),
                fill: property(n, "fill").and_then(|v| paint_colour(&doc, &v)).or_else(|| if property(n, "fill").is_none() { Some([0.0; 3]) } else { None }),
                stroke: property(n, "stroke").and_then(|v| paint_colour(&doc, &v)),
            })
            .collect()
    }

    /// The shapes as the drawing groups them (`<g>` elements with one shape in them are not groups).
    pub fn outline(&self) -> Vec<Outline> {
        fn walk(n: Node, next: &mut usize) -> Vec<Outline> {
            let mut out = Vec::new();
            for c in n.children().filter(|c| c.is_element()) {
                if is_shape(c) {
                    out.push(Outline::Shape(*next));
                    *next += 1;
                } else if !NOT_DRAWN.contains(&c.tag_name().name()) {
                    let mut kids = walk(c, next);
                    match kids.len() {
                        0 => {}
                        1 => out.push(kids.remove(0)),
                        _ => out.push(Outline::Group { name: c.attribute("id").unwrap_or("").to_string(), children: kids }),
                    }
                }
            }
            out
        }
        let doc = self.parse();
        let mut next = 0;
        walk(doc.root_element(), &mut next)
    }

    /// The SVG with the edits applied; with `only`, every other shape is removed (to see or hit-test one shape).
    pub fn render_text(&self, edits: &[ElementEdit], only: Option<usize>) -> String {
        let doc = self.parse();
        let by_index: HashMap<usize, &ElementEdit> = edits.iter().map(|e| (e.index, e)).collect();
        let mut cuts: Vec<(usize, usize, String)> = Vec::new();
        for (i, n) in doc.descendants().filter(|n| is_shape(*n)).enumerate() {
            let range = n.range();
            if only.is_some_and(|k| k != i) {
                cuts.push((range.start, range.end, String::new()));
                continue;
            }
            let Some(e) = by_index.get(&i) else { continue };
            if e.hidden && only.is_none() {
                cuts.push((range.start, range.end, String::new()));
            } else if e.fill.is_some() || e.stroke.is_some() {
                cuts.push((range.start, range.end, rebuild(n, e)));
            }
        }
        cuts.sort_by_key(|c| c.0);
        let mut out = String::with_capacity(self.text.len());
        let mut at = 0;
        for (a, b, with) in cuts {
            if a < at {
                continue;
            }
            out.push_str(&self.text[at..a]);
            out.push_str(&with);
            at = b;
        }
        out.push_str(&self.text[at..]);
        out
    }

    /// The topmost shape that has a visible pixel at (x, y) when the drawing is `px` x `px`.
    pub fn element_at(&self, edits: &[ElementEdit], px: u32, x: u32, y: u32) -> Option<usize> {
        let count = self.elements().len();
        let hidden = |i: usize| edits.iter().any(|e| e.index == i && e.hidden);
        (0..count).rev().filter(|&i| !hidden(i)).find(|&i| {
            let svg = self.render_text(edits, Some(i));
            rasterize(&svg, px).is_some_and(|img| img.get_pixel(x.min(px - 1), y.min(px - 1))[3] > 40)
        })
    }
}

/// The shape written again with its fill and stroke replaced where edited. Only drawing attributes are kept.
fn rebuild(n: Node, e: &ElementEdit) -> String {
    let mut s = format!("<{}", n.tag_name().name());
    for a in n.attributes() {
        if a.namespace().is_some() {
            continue;
        }
        match a.name() {
            "fill" if e.fill.is_some() => continue,
            "stroke" if e.stroke.is_some() => continue,
            "style" => {
                let kept: Vec<&str> = a
                    .value()
                    .split(';')
                    .filter(|d| {
                        let k = d.split(':').next().unwrap_or("").trim();
                        !((k == "fill" && e.fill.is_some()) || (k == "stroke" && e.stroke.is_some()) || d.trim().is_empty())
                    })
                    .collect();
                if !kept.is_empty() {
                    s.push_str(&format!(" style=\"{}\"", escape(&kept.join(";"))));
                }
                continue;
            }
            _ => {}
        }
        s.push_str(&format!(" {}=\"{}\"", a.name(), escape(a.value())));
    }
    if let Some(c) = e.fill {
        s.push_str(&format!(" fill=\"{}\"", hex(c)));
    }
    if let Some(c) = e.stroke {
        s.push_str(&format!(" stroke=\"{}\"", hex(c)));
    }
    s.push_str("/>");
    s
}

/// An SVG text drawn into `px` x `px` pixels (fitted and centred), straight alpha.
pub fn rasterize(svg: &str, px: u32) -> Option<RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let sc = (px as f32 / size.width()).min(px as f32 / size.height());
    let (ox, oy) = ((px as f32 - size.width() * sc) / 2.0, (px as f32 - size.height() * sc) / 2.0);
    resvg::render(&tree, tiny_skia::Transform::from_scale(sc, sc).post_translate(ox, oy), &mut pm.as_mut());
    let mut img = RgbaImage::from_raw(px, px, pm.take())?;
    for p in img.pixels_mut() {
        let a = p.0[3] as f32 / 255.0;
        if a > 0.0 {
            for c in 0..3 {
                p.0[c] = (p.0[c] as f32 / a).min(255.0) as u8;
            }
        }
    }
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="64" height="64" viewBox="0 0 64 64">
 <defs><linearGradient id="g"><stop offset="0" stop-color="#00ff00"/><stop offset="1" stop-color="#0000ff"/></linearGradient>
  <clipPath id="c"><rect x="0" y="0" width="64" height="64"/></clipPath></defs>
 <rect id="back" x="0" y="0" width="64" height="64" fill="#ff0000" inkscape:label="x"/>
 <circle cx="32" cy="32" r="16" style="fill:#ffffff;opacity:0.9"/>
 <path d="M 8 8 H 24 V 24 H 8 Z" fill="url(#g)"/>
</svg>"##;

    fn centre(img: &RgbaImage) -> [u8; 4] {
        img.get_pixel(32, 32).0
    }

    #[test]
    fn the_shapes_are_listed_without_those_in_defs() {
        let els = SvgDoc::from_text(ICON).unwrap().elements();
        assert_eq!(els.len(), 3, "{els:?}");
        assert_eq!(els[0].id, "back");
        assert_eq!(els[0].fill, Some([1.0, 0.0, 0.0]));
        assert_eq!(els[1].fill, Some([1.0, 1.0, 1.0]), "a style declaration counts");
        assert_eq!(els[2].fill, Some([0.0, 1.0, 0.0]), "a gradient shows its first colour");
    }

    #[test]
    fn a_hidden_shape_is_gone_and_a_recoloured_one_changes() {
        let doc = SvgDoc::from_text(ICON).unwrap();
        let plain = rasterize(&doc.render_text(&[], None), 64).unwrap();
        assert!(centre(&plain)[0] > 200 && centre(&plain)[1] > 200, "the white circle is on top");

        let hidden = rasterize(&doc.render_text(&[ElementEdit { index: 1, hidden: true, ..Default::default() }], None), 64).unwrap();
        assert!(centre(&hidden)[0] > 200 && centre(&hidden)[1] < 40, "without the circle the red square shows: {:?}", centre(&hidden));

        let blue = rasterize(&doc.render_text(&[ElementEdit { index: 1, fill: Some([0.0, 0.0, 1.0]), ..Default::default() }], None), 64).unwrap();
        let c = centre(&blue);
        assert!(c[2] > 200 && c[0] < 60, "the circle is blue now, its style fill removed: {c:?}");
        // the opacity of the style is kept
        assert!(c[0] > 0 || c[1] > 0 || c[3] == 255);
    }

    #[test]
    fn the_topmost_shape_under_a_point_is_found() {
        let doc = SvgDoc::from_text(ICON).unwrap();
        assert_eq!(doc.element_at(&[], 64, 32, 32), Some(1), "the circle is above the square");
        assert_eq!(doc.element_at(&[], 64, 60, 60), Some(0), "only the square is there");
        assert_eq!(doc.element_at(&[], 64, 12, 12), Some(2));
        let gone = [ElementEdit { index: 1, hidden: true, ..Default::default() }];
        assert_eq!(doc.element_at(&gone, 64, 32, 32), Some(0), "a hidden shape is skipped");
    }

    #[test]
    fn the_outline_follows_the_groups() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><path id="d" d="M0 0"/></defs>
          <rect width="8" height="8"/><g id="pair"><circle r="2"/><path d="M0 0 L1 1"/></g><g><g><rect width="1" height="1"/></g></g></svg>"##;
        let doc = SvgDoc::from_text(svg).unwrap();
        assert_eq!(doc.elements().len(), 4);
        assert_eq!(
            doc.outline(),
            vec![Outline::Shape(0), Outline::Group { name: "pair".into(), children: vec![Outline::Shape(1), Outline::Shape(2)] }, Outline::Shape(3)]
        );
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_colour("#f00"), Some([1.0, 0.0, 0.0]));
        assert_eq!(parse_colour("rgb(0, 255, 0)"), Some([0.0, 1.0, 0.0]));
        assert_eq!(parse_colour("none"), None);
    }
}
