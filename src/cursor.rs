//! The mouse cursor: a set of shapes the shell draws itself (arrow, text beam, hand, resize arrows, busy ring...), the
//! preferences for them, and the mapping from the names clients ask for (`cursor-shape-v1` uses the CSS names).

use resvg::tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};
use serde::{Deserialize, Serialize};

/// What the pointer looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Default,
    Text,
    Pointer,
    Grab,
    Grabbing,
    Move,
    ResizeEw,
    ResizeNs,
    ResizeNesw,
    ResizeNwse,
    Wait,
    Progress,
    NotAllowed,
    Crosshair,
    Copy,
    Hidden,
    /// the picture the program itself drew (`State::client_cursor_img`): Xwayland and older toolkits
    Client,
}

impl Shape {
    /// The shape for a CSS cursor name (as `wp_cursor_shape_v1` and `cursor-icon` spell them).
    pub fn from_name(name: &str) -> Shape {
        match name {
            "text" | "vertical-text" => Shape::Text,
            "pointer" | "alias" => Shape::Pointer,
            "grab" => Shape::Grab,
            "grabbing" => Shape::Grabbing,
            "move" | "all-scroll" | "all_scroll" => Shape::Move,
            "ew-resize" | "e-resize" | "w-resize" | "col-resize" => Shape::ResizeEw,
            "ns-resize" | "n-resize" | "s-resize" | "row-resize" => Shape::ResizeNs,
            "nesw-resize" | "ne-resize" | "sw-resize" => Shape::ResizeNesw,
            "nwse-resize" | "nw-resize" | "se-resize" => Shape::ResizeNwse,
            "wait" => Shape::Wait,
            "progress" => Shape::Progress,
            "not-allowed" | "no-drop" => Shape::NotAllowed,
            "crosshair" | "cell" => Shape::Crosshair,
            "copy" => Shape::Copy,
            _ => Shape::Default,
        }
    }

    /// The resize cursor for the edges a window is dragged by (bits as in the compositor: 1 left, 2 right, 4 top, 8 bottom).
    pub fn for_edges(edges: u8) -> Shape {
        match edges {
            1 | 2 => Shape::ResizeEw,
            4 | 8 => Shape::ResizeNs,
            5 | 10 => Shape::ResizeNwse,
            6 | 9 => Shape::ResizeNesw,
            _ => Shape::Default,
        }
    }

    /// Does the picture change with time (so the shell has to redraw it now and then)?
    pub fn animated(self) -> bool {
        matches!(self, Shape::Wait | Shape::Progress)
    }
}

/// The colours of the arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CursorLook {
    /// white with a dark outline
    #[default]
    Light,
    /// dark with a white outline
    Dark,
    /// the accent colour
    Accent,
}

pub const LOOKS: [(CursorLook, &str); 3] = [(CursorLook::Light, "Light"), (CursorLook::Dark, "Dark"), (CursorLook::Accent, "Accent")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorPrefs {
    /// logical size of the cursor's box, in pixels
    pub size: f32,
    pub look: CursorLook,
    /// the shell changes the cursor over window edges, buttons and while it starts a program (clients always may)
    pub dynamic: bool,
}

impl Default for CursorPrefs {
    fn default() -> Self {
        CursorPrefs { size: 28.0, look: CursorLook::Light, dynamic: true }
    }
}

/// A drawn cursor: premultiplied ARGB pixels, `size` x `size`, and where the pointer's point is inside it.
pub struct CursorImage {
    pub size: u32,
    pub hot: (i32, i32),
    pub argb: Vec<u32>,
}

enum Piece {
    /// a closed shape, filled
    Fill(Path),
    /// a stroke of this width (in design units)
    Line(Path, f32),
}

const DESIGN: f32 = 32.0;

fn poly(points: &[(f32, f32)]) -> Option<Path> {
    let mut b = PathBuilder::new();
    b.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        b.line_to(p.0, p.1);
    }
    b.close();
    b.finish()
}

fn line(a: (f32, f32), b: (f32, f32)) -> Option<Path> {
    let mut p = PathBuilder::new();
    p.move_to(a.0, a.1);
    p.line_to(b.0, b.1);
    p.finish()
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.quad_to(x + w, y, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.quad_to(x + w, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.quad_to(x, y + h, x, y + h - r);
    b.line_to(x, y + r);
    b.quad_to(x, y, x + r, y);
    b.close();
    b.finish()
}

/// An arc of a circle as a polyline (degrees, clockwise from the right).
fn arc(cx: f32, cy: f32, r: f32, from: f32, sweep: f32) -> Option<Path> {
    let mut b = PathBuilder::new();
    let n = 28;
    for i in 0..=n {
        let a = (from + sweep * i as f32 / n as f32).to_radians();
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            b.move_to(x, y);
        } else {
            b.line_to(x, y);
        }
    }
    b.finish()
}

fn arrow_points(k: f32, dx: f32, dy: f32) -> Vec<(f32, f32)> {
    [(2.0, 2.0), (2.0, 21.0), (6.6, 16.8), (10.2, 25.0), (13.4, 23.6), (9.8, 15.6), (16.2, 15.6)].iter().map(|(x, y)| (dx + x * k, dy + y * k)).collect()
}

/// The pieces of a shape in the 32 x 32 design box, the pointer's point, and the angle (degrees) to turn it by.
fn pieces(shape: Shape, phase: f32) -> (Vec<Piece>, (f32, f32), f32) {
    let mut v: Vec<Piece> = Vec::new();
    let mut hot = (16.0, 16.0);
    let mut turn = 0.0;
    match shape {
        Shape::Default | Shape::Hidden | Shape::Client => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(1.0, 0.0, 0.0)).map(Piece::Fill));
        }
        Shape::Copy => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(0.85, 1.0, 1.0)).map(Piece::Fill));
            v.extend(line((24.0, 19.0), (24.0, 29.0)).map(|p| Piece::Line(p, 2.6)));
            v.extend(line((19.0, 24.0), (29.0, 24.0)).map(|p| Piece::Line(p, 2.6)));
        }
        Shape::Text => {
            v.extend(line((16.0, 7.0), (16.0, 25.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((12.0, 6.0), (20.0, 6.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((12.0, 26.0), (20.0, 26.0)).map(|p| Piece::Line(p, 2.2)));
        }
        Shape::Pointer => {
            hot = (12.0, 3.0);
            v.extend(rrect(10.0, 3.0, 4.6, 15.0, 2.3).map(Piece::Fill));
            v.extend(rrect(7.0, 13.0, 18.0, 13.5, 5.5).map(Piece::Fill));
            v.extend(rrect(14.0, 11.5, 4.2, 8.0, 2.1).map(Piece::Fill));
            v.extend(rrect(18.0, 12.0, 4.2, 8.0, 2.1).map(Piece::Fill));
            v.extend(rrect(4.5, 15.0, 4.6, 8.5, 2.3).map(Piece::Fill));
        }
        Shape::Grab => {
            for (i, top) in [7.0, 4.5, 5.0, 8.0].iter().enumerate() {
                v.extend(rrect(8.0 + i as f32 * 4.4, *top, 4.0, 14.0, 2.0).map(Piece::Fill));
            }
            v.extend(rrect(7.0, 13.0, 18.5, 13.5, 6.0).map(Piece::Fill));
            v.extend(rrect(3.5, 14.5, 5.0, 8.5, 2.5).map(Piece::Fill));
        }
        Shape::Grabbing => {
            for i in 0..4 {
                v.extend(rrect(8.0 + i as f32 * 4.4, 10.5, 4.0, 9.0, 2.0).map(Piece::Fill));
            }
            v.extend(rrect(7.0, 14.0, 18.5, 12.5, 6.0).map(Piece::Fill));
            v.extend(rrect(4.0, 15.0, 5.0, 7.0, 2.5).map(Piece::Fill));
        }
        Shape::Move => {
            v.extend(line((16.0, 6.0), (16.0, 26.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((6.0, 16.0), (26.0, 16.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(poly(&[(16.0, 2.0), (11.5, 7.5), (20.5, 7.5)]).map(Piece::Fill));
            v.extend(poly(&[(16.0, 30.0), (11.5, 24.5), (20.5, 24.5)]).map(Piece::Fill));
            v.extend(poly(&[(2.0, 16.0), (7.5, 11.5), (7.5, 20.5)]).map(Piece::Fill));
            v.extend(poly(&[(30.0, 16.0), (24.5, 11.5), (24.5, 20.5)]).map(Piece::Fill));
        }
        Shape::ResizeEw | Shape::ResizeNs | Shape::ResizeNesw | Shape::ResizeNwse => {
            turn = match shape {
                Shape::ResizeNs => 90.0,
                Shape::ResizeNesw => -45.0,
                Shape::ResizeNwse => 45.0,
                _ => 0.0,
            };
            v.extend(line((8.0, 16.0), (24.0, 16.0)).map(|p| Piece::Line(p, 2.4)));
            v.extend(poly(&[(2.5, 16.0), (9.5, 10.0), (9.5, 22.0)]).map(Piece::Fill));
            v.extend(poly(&[(29.5, 16.0), (22.5, 10.0), (22.5, 22.0)]).map(Piece::Fill));
        }
        Shape::Wait => {
            v.extend(arc(16.0, 16.0, 9.0, phase * 360.0, 270.0).map(|p| Piece::Line(p, 3.2)));
        }
        Shape::Progress => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(0.8, 1.0, 1.0)).map(Piece::Fill));
            v.extend(arc(23.5, 23.5, 5.2, phase * 360.0, 270.0).map(|p| Piece::Line(p, 2.6)));
        }
        Shape::NotAllowed => {
            v.extend(arc(16.0, 16.0, 10.0, 0.0, 359.9).map(|p| Piece::Line(p, 3.0)));
            v.extend(line((9.0, 9.0), (23.0, 23.0)).map(|p| Piece::Line(p, 3.0)));
        }
        Shape::Crosshair => {
            v.extend(line((16.0, 4.0), (16.0, 13.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((16.0, 19.0), (16.0, 28.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((4.0, 16.0), (13.0, 16.0)).map(|p| Piece::Line(p, 2.2)));
            v.extend(line((19.0, 16.0), (28.0, 16.0)).map(|p| Piece::Line(p, 2.2)));
        }
    }
    (v, hot, turn)
}

fn color(c: [f32; 3], a: f32) -> Color {
    Color::from_rgba(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), a.clamp(0.0, 1.0)).unwrap_or(Color::BLACK)
}

/// A cursor picture a program drew itself, as it came from its shared-memory buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientCursor {
    pub w: u32,
    pub h: u32,
    /// the buffer's scale: the picture is `w / scale` logical px wide
    pub scale: i32,
    /// the pointer's point in buffer px
    pub hot: (i32, i32),
    /// premultiplied ARGB, row after row
    pub argb: Vec<u32>,
}

impl ClientCursor {
    /// From the bytes of an `Argb8888` / `Xrgb8888` buffer (little endian: B, G, R, A). None for an empty picture.
    pub fn from_bgra(bytes: &[u8], w: u32, h: u32, stride: u32, opaque: bool, scale: i32, hot: (i32, i32)) -> Option<ClientCursor> {
        let (wu, hu, su) = (w as usize, h as usize, stride as usize);
        if wu == 0 || hu == 0 || su < wu * 4 || bytes.len() < su * hu {
            return None;
        }
        let mut argb = Vec::with_capacity(wu * hu);
        for y in 0..hu {
            for x in 0..wu {
                let p = &bytes[y * su + x * 4..y * su + x * 4 + 4];
                let a = if opaque { 255 } else { p[3] };
                argb.push(u32::from_le_bytes([p[0], p[1], p[2], a]));
            }
        }
        Some(ClientCursor { w, h, scale: scale.max(1), hot, argb })
    }

    /// The picture for a hardware cursor plane: `factor` device px per logical px (the monitor's scale), at most 64 x 64,
    /// bilinear so a scaled arrow stays smooth.
    pub fn image(&self, factor: f32) -> CursorImage {
        let k = (factor / self.scale as f32).max(0.05);
        let n = (((self.w.max(self.h)) as f32 * k).round() as u32).clamp(1, 64);
        // the same factor in both directions, so that the shape is not squeezed
        let k = n as f32 / self.w.max(self.h) as f32;
        let (ow, oh) = (((self.w as f32 * k).round() as usize).clamp(1, n as usize), ((self.h as f32 * k).round() as usize).clamp(1, n as usize));
        let at = |x: i32, y: i32| -> [f32; 4] {
            let (x, y) = (x.clamp(0, self.w as i32 - 1) as usize, y.clamp(0, self.h as i32 - 1) as usize);
            let p = self.argb[y * self.w as usize + x];
            [(p >> 24) as f32, ((p >> 16) & 255) as f32, ((p >> 8) & 255) as f32, (p & 255) as f32]
        };
        let mut argb = vec![0u32; (n * n) as usize];
        for y in 0..oh {
            for x in 0..ow {
                // the centre of the output pixel, in source px
                let (sx, sy) = ((x as f32 + 0.5) / k - 0.5, (y as f32 + 0.5) / k - 0.5);
                let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
                let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
                let mut px = 0u32;
                for ch in 0..4 {
                    let v = (a[ch] * (1.0 - fx) + b[ch] * fx) * (1.0 - fy) + (c[ch] * (1.0 - fx) + d[ch] * fx) * fy;
                    px = (px << 8) | v.round().clamp(0.0, 255.0) as u32;
                }
                argb[y * n as usize + x] = px;
            }
        }
        CursorImage { size: n, hot: ((self.hot.0 as f32 * k).round() as i32, (self.hot.1 as f32 * k).round() as i32), argb }
    }

    /// Straight (not premultiplied) RGBA bytes, for a window system's own cursor.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.argb.len() * 4);
        for p in &self.argb {
            let (a, r, g, b) = ((p >> 24) & 255, (p >> 16) & 255, (p >> 8) & 255, p & 255);
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            out.extend_from_slice(&[un(r), un(g), un(b), a as u8]);
        }
        out
    }
}

/// Draw `shape` into a box of `box_px` device pixels (at most 64: that is what a hardware cursor plane takes).
/// `phase` (0 .. 1) turns the busy ring.
pub fn render(shape: Shape, box_px: u32, look: CursorLook, accent: [f32; 3], phase: f32) -> CursorImage {
    let n = box_px.clamp(12, 64);
    let mut argb = vec![0u32; (n * n) as usize];
    let mut out = CursorImage { size: n, hot: (0, 0), argb: Vec::new() };
    if shape == Shape::Hidden {
        out.argb = argb;
        return out;
    }
    let (fill, edge) = match look {
        CursorLook::Light => ([1.0, 1.0, 1.0], [0.05, 0.06, 0.08]),
        CursorLook::Dark => ([0.06, 0.07, 0.09], [1.0, 1.0, 1.0]),
        CursorLook::Accent => (accent, [1.0, 1.0, 1.0]),
    };
    let (fill, edge) = if shape == Shape::NotAllowed { ([0.95, 0.25, 0.25], [1.0, 1.0, 1.0]) } else { (fill, edge) };
    let Some(mut pm) = Pixmap::new(n, n) else {
        out.argb = argb;
        return out;
    };
    let k = n as f32 / DESIGN;
    let (pieces, hot, turn) = pieces(shape, phase);
    let tf = if turn != 0.0 { Transform::from_rotate_at(turn, 16.0, 16.0).post_scale(k, k) } else { Transform::from_scale(k, k) };
    let mut paint = Paint { anti_alias: true, ..Paint::default() };
    let outline = 1.7;
    // the outlines first, so that touching pieces (a hand's fingers) do not draw lines through each other
    paint.set_color(color(edge, 0.92));
    for p in &pieces {
        let (path, w) = match p {
            Piece::Fill(path) => (path, outline * 2.0),
            Piece::Line(path, w) => (path, w + outline * 2.0),
        };
        let stroke = Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(path, &paint, &stroke, tf, None);
    }
    paint.set_color(color(fill, 1.0));
    for p in &pieces {
        match p {
            Piece::Fill(path) => pm.fill_path(path, &paint, FillRule::Winding, tf, None),
            Piece::Line(path, w) => {
                let stroke = Stroke { width: *w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(path, &paint, &stroke, tf, None);
            }
        }
    }
    for (i, px) in pm.pixels().iter().enumerate() {
        argb[i] = (px.alpha() as u32) << 24 | (px.red() as u32) << 16 | (px.green() as u32) << 8 | px.blue() as u32;
    }
    // the hot spot, turned like the picture
    let (mut hx, mut hy) = hot;
    if turn != 0.0 {
        let a = turn.to_radians();
        let (dx, dy) = (hx - 16.0, hy - 16.0);
        hx = 16.0 + dx * a.cos() - dy * a.sin();
        hy = 16.0 + dx * a.sin() + dy * a.cos();
    }
    out.hot = ((hx * k).round() as i32, (hy * k).round() as i32);
    out.argb = argb;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CURSOR_SHEET=/path.png cargo test cursor_sheet -- --ignored`: every shape on a grey strip, to look at.
    #[test]
    #[ignore]
    fn cursor_sheet() {
        let Ok(path) = std::env::var("CURSOR_SHEET") else { return };
        let shapes = [Shape::Default, Shape::Text, Shape::Pointer, Shape::Grab, Shape::Grabbing, Shape::Move, Shape::ResizeEw, Shape::ResizeNs, Shape::ResizeNesw, Shape::ResizeNwse, Shape::Wait, Shape::Progress, Shape::NotAllowed, Shape::Crosshair, Shape::Copy];
        let mut img = image::RgbaImage::from_pixel(64 * shapes.len() as u32 * 2, 128, image::Rgba([120, 130, 150, 255]));
        for (row, look) in [CursorLook::Light, CursorLook::Dark].iter().enumerate() {
            for (i, s) in shapes.iter().enumerate() {
                let c = render(*s, 56, *look, [0.1, 0.5, 1.0], 0.1);
                for y in 0..c.size {
                    for x in 0..c.size {
                        let p = c.argb[(y * c.size + x) as usize];
                        let a = (p >> 24) as f32 / 255.0;
                        if a > 0.0 {
                            let (px, py) = (i as u32 * 128 + 4 + x, row as u32 * 64 + y);
                            let old = img.get_pixel(px, py).0;
                            let mix = |sh: u32, o: u8| (((p >> sh) & 255) as f32 + o as f32 * (1.0 - a)) as u8;
                            img.put_pixel(px, py, image::Rgba([mix(16, old[0]), mix(8, old[1]), mix(0, old[2]), 255]));
                        }
                    }
                }
            }
        }
        img.save(path).unwrap();
    }

    #[test]
    fn css_names_map_to_shapes() {
        assert_eq!(Shape::from_name("text"), Shape::Text);
        assert_eq!(Shape::from_name("ew-resize"), Shape::ResizeEw);
        assert_eq!(Shape::from_name("something-new"), Shape::Default);
    }

    #[test]
    fn edges_pick_the_resize_cursor() {
        assert_eq!(Shape::for_edges(1), Shape::ResizeEw);
        assert_eq!(Shape::for_edges(8), Shape::ResizeNs);
        assert_eq!(Shape::for_edges(5), Shape::ResizeNwse);
        assert_eq!(Shape::for_edges(10), Shape::ResizeNwse);
        assert_eq!(Shape::for_edges(6), Shape::ResizeNesw);
        assert_eq!(Shape::for_edges(0), Shape::Default);
    }

    #[test]
    fn every_shape_draws_something_with_the_hot_spot_inside() {
        for s in [Shape::Default, Shape::Text, Shape::Pointer, Shape::Grab, Shape::Grabbing, Shape::Move, Shape::ResizeEw, Shape::ResizeNs, Shape::ResizeNesw, Shape::ResizeNwse, Shape::Wait, Shape::Progress, Shape::NotAllowed, Shape::Crosshair, Shape::Copy] {
            let img = render(s, 32, CursorLook::Light, [0.0, 0.5, 1.0], 0.0);
            assert!(img.argb.iter().any(|p| p >> 24 > 0), "{s:?} is empty");
            assert!(img.hot.0 >= 0 && img.hot.0 < 32 && img.hot.1 >= 0 && img.hot.1 < 32, "{s:?} hot spot {:?}", img.hot);
        }
        assert!(render(Shape::Hidden, 32, CursorLook::Light, [0.0; 3], 0.0).argb.iter().all(|p| p >> 24 == 0));
    }

    #[test]
    fn a_programs_own_cursor_is_scaled_to_the_monitor() {
        // a 4 x 4 opaque red picture, hot spot at (1, 1)
        let bytes: Vec<u8> = (0..16).flat_map(|_| [0u8, 0, 255, 255]).collect();
        let cur = ClientCursor::from_bgra(&bytes, 4, 4, 16, false, 1, (1, 1)).unwrap();
        let same = cur.image(1.0);
        assert_eq!((same.size, same.hot), (4, (1, 1)));
        assert!(same.argb[..16].iter().all(|&p| p == 0xFFFF0000));
        let big = cur.image(2.0);
        assert_eq!((big.size, big.hot), (8, (2, 2)));
        // a big picture is cut down to what a cursor plane takes
        let huge = ClientCursor::from_bgra(&vec![255u8; 128 * 128 * 4], 128, 128, 512, true, 1, (0, 0)).unwrap();
        assert_eq!(huge.image(1.0).size, 64);
        // transparent pixels stay transparent in the straight form
        let clear = ClientCursor::from_bgra(&[0u8; 16], 2, 2, 8, false, 1, (0, 0)).unwrap();
        assert!(clear.rgba().iter().all(|&b| b == 0));
        assert!(ClientCursor::from_bgra(&[0u8; 4], 2, 2, 8, false, 1, (0, 0)).is_none());
    }
}
