//! The mouse cursor: a set of shapes the shell draws itself (arrow, text beam, hand, resize arrows, busy ring...), the
//! preferences for them, and the mapping from the names clients ask for (`cursor-shape-v1` uses the CSS names).

use resvg::tiny_skia::{Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, SpreadMode, Stroke, Transform};
use serde::{Deserialize, Serialize};

/// What the pointer looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
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
    /// a rainbow across the whole shape
    Rainbow,
    /// only the outline, the inside is clear
    Hollow,
    /// the hue picked in the settings (`CursorPrefs::hue`)
    Custom,
}

pub const LOOKS: [(CursorLook, &str); 6] = [
    (CursorLook::Light, "Light"),
    (CursorLook::Dark, "Dark"),
    (CursorLook::Accent, "Accent"),
    (CursorLook::Rainbow, "Rainbow"),
    (CursorLook::Hollow, "Hollow"),
    (CursorLook::Custom, "Custom"),
];

/// The shape of the arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArrowStyle {
    #[default]
    Classic,
    /// narrower
    Slim,
    /// a plain triangle
    Triangle,
}

pub const ARROWS: [(ArrowStyle, &str); 3] = [(ArrowStyle::Classic, "Classic"), (ArrowStyle::Slim, "Slim"), (ArrowStyle::Triangle, "Triangle")];

/// What the "starting / busy" cursor looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BusyStyle {
    /// a turning ring
    #[default]
    Ring,
    /// an hourglass that runs out and turns over
    Hourglass,
    /// the hourglass, its colours flowing like a rainbow
    Rainbow,
}

/// How the starting program's icon beside the busy cursor moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LaunchAnim {
    /// hops up and falls back like a Dock icon, squashing a little where it lands
    #[default]
    Bounce,
    /// breathes: shrinks and fades a little, and back
    Pulse,
    /// stands still
    Still,
}

pub const LAUNCH_ANIMS: [(LaunchAnim, &str); 3] = [(LaunchAnim::Bounce, "Bounce"), (LaunchAnim::Pulse, "Pulse"), (LaunchAnim::Still, "None")];

pub const BUSY_STYLES: [(BusyStyle, &str); 3] = [(BusyStyle::Ring, "Ring"), (BusyStyle::Hourglass, "Hourglass"), (BusyStyle::Rainbow, "Rainbow hourglass")];

/// The colour of the outline around the shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EdgeColor {
    /// whatever stands out against the fill (dark around a light cursor, white around a dark one)
    #[default]
    Auto,
    Dark,
    Light,
    /// the accent colour
    Accent,
}

pub const EDGES: [(EdgeColor, &str); 4] = [(EdgeColor::Auto, "Auto"), (EdgeColor::Dark, "Black"), (EdgeColor::Light, "White"), (EdgeColor::Accent, "Accent")];

/// The cursor over links and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HandStyle {
    /// a pointing hand
    #[default]
    Hand,
    /// the plain arrow, as over everything else
    Arrow,
}

pub const HANDS: [(HandStyle, &str); 2] = [(HandStyle::Hand, "Hand"), (HandStyle::Arrow, "Arrow")];

/// The cursor over text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BeamStyle {
    /// an upright line with short bars at both ends
    #[default]
    Classic,
    /// just the line
    Plain,
    /// thicker, with wider bars: easier to see on a busy page
    Bold,
}

pub const BEAMS: [(BeamStyle, &str); 3] = [(BeamStyle::Classic, "Classic"), (BeamStyle::Plain, "Plain"), (BeamStyle::Bold, "Bold")];

/// The cursor over window edges and corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResizeStyle {
    /// a line with a solid head at each end
    #[default]
    Line,
    /// a dot in the accent colour between two small heads, nothing joining them
    Dot,
}

/// The shapes the Mouse settings page shows, in its order (a preview's key names a shape by its place here).
pub const PREVIEW_SHAPES: [Shape; 12] = [Shape::Default, Shape::Pointer, Shape::Text, Shape::Grab, Shape::Move, Shape::ResizeEw, Shape::ResizeNs, Shape::ResizeNwse, Shape::Wait, Shape::Progress, Shape::NotAllowed, Shape::Crosshair];

/// The atlas key of a settings preview of `shape` (one of `PREVIEW_SHAPES`) with these preferences and accent: it carries
/// all of them, so whoever draws it needs nothing else, and a changed choice is a new key.
pub fn preview_key(shape: Shape, prefs: &CursorPrefs, accent: [f32; 3]) -> String {
    let i = PREVIEW_SHAPES.iter().position(|s| *s == shape).unwrap_or(0);
    format!("cursor:{i}:{},{},{}:{}", accent[0], accent[1], accent[2], serde_json::to_string(prefs).unwrap_or_default())
}

/// The picture behind a `preview_key` (straight RGBA, 64 px), and the part of the key that names the preferences (shared
/// by every shape of one look). None for any other key.
pub fn preview_image(key: &str) -> Option<(image::RgbaImage, String)> {
    let rest = key.strip_prefix("cursor:")?;
    let (i, rest) = rest.split_once(':')?;
    let (accent, json) = rest.split_once(':')?;
    let shape = *PREVIEW_SHAPES.get(i.parse::<usize>().ok()?)?;
    let a: Vec<f32> = accent.split(',').filter_map(|v| v.parse().ok()).collect();
    let accent = [*a.first()?, *a.get(1)?, *a.get(2)?];
    let prefs: CursorPrefs = serde_json::from_str(json).ok()?;
    // a still moment of the busy ones; drawn at the plane's largest size and shown smaller: sharp on a scaled monitor
    let c = render_with(shape, 64, &prefs, accent, 0.3);
    let mut img = image::RgbaImage::new(c.size, c.size);
    for (px, p) in img.pixels_mut().zip(&c.argb) {
        let (a, r, g, b) = ((p >> 24) & 255, (p >> 16) & 255, (p >> 8) & 255, p & 255);
        let un = |v: u32| if a == 0 { 0 } else { ((v * 255 + a / 2) / a).min(255) as u8 };
        *px = image::Rgba([un(r), un(g), un(b), a as u8]);
    }
    Some((img, rest.to_string()))
}

pub const RESIZES: [(ResizeStyle, &str); 2] = [(ResizeStyle::Line, "Line"), (ResizeStyle::Dot, "Dot")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorPrefs {
    /// logical size of the cursor's box, in pixels
    pub size: f32,
    pub look: CursorLook,
    /// the shell changes the cursor over window edges, buttons and while it starts a program (clients always may)
    pub dynamic: bool,
    /// the hue (0 .. 360) of the `Custom` look
    pub hue: f32,
    /// the outline around the shape, in design units (0 = none)
    pub outline: f32,
    pub arrow: ArrowStyle,
    pub busy: BusyStyle,
    /// an installed Xcursor theme shown instead of the drawn cursor (and handed to programs); empty = the shell's own drawing
    pub theme: String,
    /// saturation and brightness (0 .. 1) of the `Custom` look's fill, with `hue`
    pub sat: f32,
    pub bright: f32,
    /// the outline's colour
    pub edge: EdgeColor,
    /// a soft shadow under the cursor: 0 (none) .. 1
    pub shadow: f32,
    /// how fast the busy cursor turns, times the usual pace (0.25 .. 3)
    pub busy_speed: f32,
    pub hand: HandStyle,
    pub beam: BeamStyle,
    pub resize: ResizeStyle,
    /// shaking the mouse makes the cursor big for a moment, so that it can be found on a large screen
    pub shake_to_find: bool,
    /// typing hides the cursor until the mouse moves again
    pub hide_typing: bool,
    /// the starting program's icon is shown beside the busy cursor
    pub launch_icon: bool,
    /// its logical size, in pixels
    pub launch_size: f32,
    /// how far (logical px) it sits right of the cursor's visible shape, and below the shape's bottom (negative: higher)
    pub launch_dx: f32,
    pub launch_dy: f32,
    pub launch_anim: LaunchAnim,
    /// how high the icon hops (`Bounce`), logical px
    pub launch_height: f32,
    /// how fast it moves, times the usual pace (0.25 .. 3)
    pub launch_speed: f32,
    /// a change of shape is animated (see [`Morph`]) instead of swapping the picture at once
    pub morph: bool,
    /// how fast that change runs, times the usual pace (0.5 .. 2)
    pub morph_speed: f32,
}

impl Default for CursorPrefs {
    fn default() -> Self {
        CursorPrefs {
            size: 28.0,
            look: CursorLook::Light,
            dynamic: true,
            hue: 210.0,
            outline: 1.7,
            arrow: ArrowStyle::Classic,
            busy: BusyStyle::Ring,
            theme: String::new(),
            sat: 0.7,
            bright: 1.0,
            edge: EdgeColor::Auto,
            shadow: 0.0,
            busy_speed: 1.0,
            hand: HandStyle::Hand,
            beam: BeamStyle::Classic,
            resize: ResizeStyle::Line,
            shake_to_find: false,
            hide_typing: true,
            launch_icon: true,
            launch_size: 22.0,
            launch_dx: 3.0,
            launch_dy: 0.0,
            launch_anim: LaunchAnim::Bounce,
            launch_height: 10.0,
            launch_speed: 1.0,
            morph: true,
            morph_speed: 1.0,
        }
    }
}

impl CursorPrefs {
    /// Loops per second of the drawn busy cursor (the theme's own pictures keep their own timing, see `cursor_theme`).
    pub fn busy_rate(&self) -> f32 {
        let base = if self.busy == BusyStyle::Ring { 1.1 } else { 0.45 };
        base * self.busy_speed.clamp(0.25, 3.0)
    }
}

/// A drawn cursor: premultiplied ARGB pixels, `size` x `size`, and where the pointer's point is inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
    pub size: u32,
    pub hot: (i32, i32),
    pub argb: Vec<u32>,
}

/// One hop of the `Bounce` launch icon, seconds at speed 1; the share of it spent in the air (the rest it squashes on
/// the ground) and how much it squashes.
const BOUNCE_PERIOD: f32 = 0.72;
const BOUNCE_AIR: f32 = 0.8;
const BOUNCE_SQUASH: f32 = 0.14;
/// One breath of the `Pulse` launch icon, seconds at speed 1.
const PULSE_PERIOD: f32 = 1.1;

/// The launch icon's pose `t` seconds after the launch: (lift as a share of the bounce height, width factor, height factor,
/// opacity). The width and height factors scale around the icon's bottom middle (`Bounce`) or its middle (`Pulse`).
pub fn launch_pose(prefs: &CursorPrefs, t: f32) -> (f32, f32, f32, f32) {
    let speed = prefs.launch_speed.clamp(0.25, 3.0);
    let t = t.max(0.0) * speed;
    match prefs.launch_anim {
        LaunchAnim::Still => (0.0, 1.0, 1.0, 1.0),
        LaunchAnim::Bounce => {
            let c = (t / BOUNCE_PERIOD).fract();
            if c < BOUNCE_AIR {
                // a thrown ball: fast off the ground, slowing to the top, then falling faster and faster
                let u = c / BOUNCE_AIR;
                (4.0 * u * (1.0 - u), 1.0, 1.0, 1.0)
            } else {
                // on the ground: flattens and springs back
                let v = (c - BOUNCE_AIR) / (1.0 - BOUNCE_AIR);
                let q = BOUNCE_SQUASH * (std::f32::consts::PI * v).sin();
                (0.0, 1.0 + q, 1.0 - q, 1.0)
            }
        }
        LaunchAnim::Pulse => {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * t / PULSE_PERIOD).cos();
            let k = 1.0 - 0.14 * w;
            (0.0, k, k, 1.0 - 0.3 * w)
        }
    }
}

/// The part of a cursor picture that can be seen (alpha above a faint shadow): x0, y0, x1, y1 (exclusive).
pub fn visible_bounds(img: &CursorImage) -> Option<(i32, i32, i32, i32)> {
    let n = img.size as i32;
    let (mut x0, mut y0, mut x1, mut y1) = (n, n, -1, -1);
    for y in 0..n {
        for x in 0..n {
            if img.argb[(y * n + x) as usize] >> 24 > 40 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x1 >= 0).then_some((x0, y0, x1 + 1, y1 + 1))
}

/// Where the launch icon goes beside a cursor picture: device px, in the cursor picture's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaunchLayout {
    /// the icon's side at rest
    pub side: f32,
    /// its left edge and bottom at rest
    pub left: f32,
    pub bottom: f32,
    /// how high it hops
    pub height: f32,
    /// the combined picture: its side, and where the cursor picture's top left lands in it
    pub size: u32,
    pub origin: (i32, i32),
}

impl LaunchLayout {
    /// Room for the cursor `img` and the icon wherever the animation takes it, in a picture of at most `max` px a side:
    /// when it would not fit the icon, its distance and its hop shrink together.
    pub fn new(img: &CursorImage, prefs: &CursorPrefs, scale: f32, max: u32) -> LaunchLayout {
        let n = img.size as f32;
        let (_, _, bx1, by1) = visible_bounds(img).unwrap_or((0, 0, img.size as i32, img.size as i32));
        let squash = if prefs.launch_anim == LaunchAnim::Bounce { BOUNCE_SQUASH } else { 0.0 };
        let hops = prefs.launch_anim == LaunchAnim::Bounce;
        let max = max.max(img.size) as f32;
        let mut k = 1.0f32;
        loop {
            let side = (prefs.launch_size.clamp(8.0, 64.0) * scale * k).max(6.0);
            let height = if hops { prefs.launch_height.clamp(0.0, 48.0) * scale * k } else { 0.0 };
            // (a squashed icon grows sideways around its middle: the gap keeps even its widest picture off the cursor)
            let left = bx1 as f32 + prefs.launch_dx.clamp(0.0, 48.0) * scale * k + side * squash / 2.0;
            let bottom = by1 as f32 + prefs.launch_dy.clamp(-48.0, 48.0) * scale * k;
            let (x0, y0) = ((left - side * squash / 2.0).min(0.0).floor(), (bottom - side - height).min(0.0).floor());
            let (x1, y1) = ((left + side * (1.0 + squash / 2.0)).max(n).ceil(), bottom.max(n).ceil());
            let size = (x1 - x0).max(y1 - y0);
            if size <= max || k < 0.05 {
                return LaunchLayout { side, left, bottom, height, size: size.min(max) as u32, origin: (-x0 as i32, -y0 as i32) };
            }
            k *= 0.92;
        }
    }

    /// The icon's box for a pose of [`launch_pose`]: x, y, width, height (device px, cursor picture coordinates).
    pub fn icon_box(&self, pose: (f32, f32, f32, f32), anim: LaunchAnim) -> [f32; 4] {
        let (lift, sx, sy, _) = pose;
        let (w, h) = (self.side * sx, self.side * sy);
        let cx = self.left + self.side / 2.0;
        match anim {
            LaunchAnim::Pulse => [cx - w / 2.0, self.bottom - self.side / 2.0 - h / 2.0, w, h],
            _ => [cx - w / 2.0, self.bottom - lift * self.height - h, w, h],
        }
    }
}

/// The starting program's icon beside the busy cursor `img` (like KDE's launch feedback): right of the cursor's visible
/// shape, so that it never covers it, `launch_dx` / `launch_dy` logical px away, moving as `launch_anim` says `t` seconds
/// after the launch. `scale`: device px per logical px; `max`: the largest picture the cursor plane takes.
pub fn with_launch_icon(img: CursorImage, icon: &image::RgbaImage, prefs: &CursorPrefs, scale: f32, t: f32, max: u32) -> CursorImage {
    let lay = LaunchLayout::new(&img, prefs, scale, max);
    let size = lay.size as usize;
    let (ox, oy) = lay.origin;
    let n = img.size as usize;
    let mut argb = vec![0u32; size * size];
    for y in 0..n {
        for x in 0..n {
            let (tx, ty) = (x as i32 + ox, y as i32 + oy);
            if tx >= 0 && ty >= 0 && (tx as usize) < size && (ty as usize) < size {
                argb[ty as usize * size + tx as usize] = img.argb[y * n + x];
            }
        }
    }
    let pose = launch_pose(prefs, t);
    let [bx, by, bw, bh] = lay.icon_box(pose, prefs.launch_anim);
    // the icon at its resting size once (good filtering), then sampled bilinearly into the moving, squashed box, so that
    // it glides by fractions of a pixel instead of jumping a whole one
    let si = lay.side.round().max(1.0) as u32;
    let small = image::imageops::resize(icon, si, si, image::imageops::FilterType::Triangle);
    let pre: Vec<[f32; 4]> = small
        .pixels()
        .map(|p| {
            let a = p.0[3] as f32 / 255.0;
            [p.0[0] as f32 * a, p.0[1] as f32 * a, p.0[2] as f32 * a, p.0[3] as f32]
        })
        .collect();
    let si = si as i32;
    let texel = |x: i32, y: i32| if x < 0 || y < 0 || x >= si || y >= si { [0.0; 4] } else { pre[(y * si + x) as usize] };
    let opacity = pose.3.clamp(0.0, 1.0);
    let (bx, by) = (bx + ox as f32, by + oy as f32);
    for py in (by.floor() as i32).max(0)..((by + bh).ceil() as i32).min(size as i32) {
        for px in (bx.floor() as i32).max(0)..((bx + bw).ceil() as i32).min(size as i32) {
            let u = (px as f32 + 0.5 - bx) / bw * si as f32 - 0.5;
            let v = (py as f32 + 0.5 - by) / bh * si as f32 - 0.5;
            let (x0, y0) = (u.floor() as i32, v.floor() as i32);
            let (fx, fy) = (u - x0 as f32, v - y0 as f32);
            let (a, b, c, d) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
            let s: [f32; 4] = std::array::from_fn(|i| ((a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy) * opacity);
            if s[3] <= 0.0 {
                continue;
            }
            // over what is there, premultiplied like the rest
            let at = py as usize * size + px as usize;
            let under = argb[at];
            let keep = 1.0 - s[3] / 255.0;
            let mix = |sh: u32, top: f32| ((((under >> sh) & 255) as f32 * keep + top).round().clamp(0.0, 255.0) as u32) << sh;
            argb[at] = mix(24, s[3]) | mix(16, s[0]) | mix(8, s[1]) | mix(0, s[2]);
        }
    }
    CursorImage { size: lay.size, hot: (img.hot.0 + ox, img.hot.1 + oy), argb }
}

enum Piece {
    /// a closed shape, filled
    Fill(Path),
    /// a stroke of this width (in design units)
    Line(Path, f32),
    /// a closed shape filled with the accent colour (whatever the look)
    Accent(Path),
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

fn arrow_points(style: ArrowStyle, k: f32, dx: f32, dy: f32) -> Vec<(f32, f32)> {
    let pts: &[(f32, f32)] = match style {
        ArrowStyle::Classic => &[(2.0, 2.0), (2.0, 21.0), (6.6, 16.8), (10.2, 25.0), (13.4, 23.6), (9.8, 15.6), (16.2, 15.6)],
        ArrowStyle::Slim => &[(2.0, 2.0), (2.0, 23.0), (6.0, 18.6), (9.2, 26.5), (11.6, 25.5), (8.4, 17.6), (13.4, 17.4)],
        ArrowStyle::Triangle => &[(2.0, 2.0), (2.0, 24.0), (8.0, 18.5), (19.0, 17.0)],
    };
    pts.iter().map(|(x, y)| (dx + x * k, dy + y * k)).collect()
}

/// An hourglass of half-height 11.5 around (`cx`, `cy`), scaled by `s`; `p` (0 .. 1) is the sand running down, then the turn over.
fn hourglass(cx: f32, cy: f32, s: f32, p: f32) -> Vec<Piece> {
    let (fall, flip) = if p < 0.8 { (p / 0.8, 0.0) } else { (1.0, (p - 0.8) / 0.2) };
    let ease = flip * flip * (3.0 - 2.0 * flip);
    let (sin, cos) = (ease * std::f32::consts::PI).sin_cos();
    let pt = |x: f32, y: f32| (cx + s * (x * cos - y * sin), cy + s * (x * sin + y * cos));
    let mut v = Vec::new();
    let glass = [pt(-7.0, -10.5), pt(7.0, -10.5), pt(1.0, 0.0), pt(7.0, 10.5), pt(-7.0, 10.5), pt(-1.0, 0.0)];
    v.extend(poly(&glass).map(|g| Piece::Line(g, 1.7)));
    for y in [-11.5f32, 11.5] {
        v.extend(line(pt(-9.0, y), pt(9.0, y)).map(|l| Piece::Line(l, 2.2)));
    }
    // the sand above (a cone that sinks) and below (a pile that grows)
    let top = 9.0 * (1.0 - fall);
    if top > 0.5 {
        let w = 5.2 * top / 9.0;
        v.extend(poly(&[pt(-w, -top - 0.5), pt(w, -top - 0.5), pt(0.0, -0.6)]).map(Piece::Fill));
    }
    let pile = 9.0 * fall;
    if pile > 0.5 {
        let hw = |y: f32| 0.6 + 5.0 * y / 10.0;
        v.extend(poly(&[pt(-hw(9.8), 9.8), pt(hw(9.8), 9.8), pt(hw(9.8 - pile), 9.8 - pile), pt(-hw(9.8 - pile), 9.8 - pile)]).map(Piece::Fill));
    }
    if fall < 1.0 && flip == 0.0 {
        v.extend(line(pt(0.0, -0.6), pt(0.0, 9.8 - pile)).map(|l| Piece::Line(l, 0.9)));
    }
    v
}

fn hsv(h: f32, sat: f32, val: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let (i, f) = (h.floor() as i32, h - h.floor());
    let (p, q, t) = (val * (1.0 - sat), val * (1.0 - sat * f), val * (1.0 - sat * (1.0 - f)));
    match i {
        0 => [val, t, p],
        1 => [q, val, p],
        2 => [p, val, t],
        3 => [p, q, val],
        4 => [t, p, val],
        _ => [val, p, q],
    }
}

/// A hand as six rounded boxes (x, y, width, height, corner radius) in the design box: four fingers (index first), the
/// palm and the thumb; and the pointer's point. The same layout for every hand, so that one can be blended into another
/// (see [`Morph`]). The pointing hand's little finger has no width: it is folded away inside the palm.
type Hand = ([[f32; 5]; 6], (f32, f32));

const HAND_POINTER: Hand = (
    [[10.0, 3.0, 4.6, 15.0, 2.3], [14.0, 11.5, 4.2, 8.0, 2.1], [18.0, 12.0, 4.2, 8.0, 2.1], [17.5, 17.0, 0.0, 5.0, 0.0], [7.0, 13.0, 18.0, 13.5, 5.5], [4.5, 15.0, 4.6, 8.5, 2.3]],
    (12.0, 3.0),
);
/// (`hot` is the middle of the design box, as for the other centred shapes)
const HAND_GRAB: Hand = (
    [[8.0, 7.0, 4.0, 14.0, 2.0], [12.4, 4.5, 4.0, 14.0, 2.0], [16.8, 5.0, 4.0, 14.0, 2.0], [21.2, 8.0, 4.0, 14.0, 2.0], [7.0, 13.0, 18.5, 13.5, 6.0], [3.5, 14.5, 5.0, 8.5, 2.5]],
    (16.0, 16.0),
);
const HAND_GRABBING: Hand = (
    [[8.0, 10.5, 4.0, 9.0, 2.0], [12.4, 10.5, 4.0, 9.0, 2.0], [16.8, 10.5, 4.0, 9.0, 2.0], [21.2, 10.5, 4.0, 9.0, 2.0], [7.0, 14.0, 18.5, 12.5, 6.0], [4.0, 15.0, 5.0, 7.0, 2.5]],
    (16.0, 16.0),
);

/// The filled pieces of a hand (boxes thinner than half a unit are left out: a finger folded away, or one unfolding).
fn hand_pieces(boxes: &[[f32; 5]; 6]) -> Vec<Piece> {
    boxes.iter().filter(|b| b[2] >= 0.5 && b[3] >= 0.5).filter_map(|b| rrect(b[0], b[1], b[2], b[3], b[4].max(0.0)).map(Piece::Fill)).collect()
}

/// The pieces of a shape in the 32 x 32 design box, the pointer's point, and the angle (degrees) to turn it by.
fn pieces(shape: Shape, phase: f32, prefs: &CursorPrefs) -> (Vec<Piece>, (f32, f32), f32) {
    let arrow = prefs.arrow;
    let mut v: Vec<Piece> = Vec::new();
    let mut hot = (16.0, 16.0);
    let mut turn = 0.0;
    match shape {
        Shape::Default | Shape::Hidden | Shape::Client => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(arrow, 1.0, 0.0, 0.0)).map(Piece::Fill));
        }
        Shape::Copy => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(arrow, 0.85, 1.0, 1.0)).map(Piece::Fill));
            v.extend(line((24.0, 19.0), (24.0, 29.0)).map(|p| Piece::Line(p, 2.6)));
            v.extend(line((19.0, 24.0), (29.0, 24.0)).map(|p| Piece::Line(p, 2.6)));
        }
        Shape::Text => match prefs.beam {
            BeamStyle::Classic => {
                v.extend(line((16.0, 7.0), (16.0, 25.0)).map(|p| Piece::Line(p, 2.2)));
                v.extend(line((12.0, 6.0), (20.0, 6.0)).map(|p| Piece::Line(p, 2.2)));
                v.extend(line((12.0, 26.0), (20.0, 26.0)).map(|p| Piece::Line(p, 2.2)));
            }
            BeamStyle::Plain => v.extend(line((16.0, 6.0), (16.0, 26.0)).map(|p| Piece::Line(p, 2.4))),
            BeamStyle::Bold => {
                v.extend(line((16.0, 6.5), (16.0, 25.5)).map(|p| Piece::Line(p, 3.4)));
                v.extend(line((10.5, 5.0), (21.5, 5.0)).map(|p| Piece::Line(p, 3.0)));
                v.extend(line((10.5, 27.0), (21.5, 27.0)).map(|p| Piece::Line(p, 3.0)));
            }
        },
        Shape::Pointer if prefs.hand == HandStyle::Arrow => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(arrow, 1.0, 0.0, 0.0)).map(Piece::Fill));
        }
        Shape::Pointer => {
            hot = HAND_POINTER.1;
            v.extend(hand_pieces(&HAND_POINTER.0));
        }
        Shape::Grab => {
            hot = HAND_GRAB.1;
            v.extend(hand_pieces(&HAND_GRAB.0));
        }
        Shape::Grabbing => {
            hot = HAND_GRABBING.1;
            v.extend(hand_pieces(&HAND_GRABBING.0));
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
            match prefs.resize {
                ResizeStyle::Line => {
                    v.extend(line((8.0, 16.0), (24.0, 16.0)).map(|p| Piece::Line(p, 2.4)));
                    v.extend(poly(&[(2.5, 16.0), (9.5, 10.0), (9.5, 22.0)]).map(Piece::Fill));
                    v.extend(poly(&[(29.5, 16.0), (22.5, 10.0), (22.5, 22.0)]).map(Piece::Fill));
                }
                ResizeStyle::Dot => {
                    // gaps wider than two outlines, so the three parts stay apart
                    v.extend(poly(&[(2.5, 16.0), (8.0, 11.5), (8.0, 20.5)]).map(Piece::Fill));
                    v.extend(poly(&[(29.5, 16.0), (24.0, 11.5), (24.0, 20.5)]).map(Piece::Fill));
                    v.extend(PathBuilder::from_circle(16.0, 16.0, 3.0).map(Piece::Accent));
                }
            }
        }
        Shape::Wait if prefs.busy != BusyStyle::Ring => v.extend(hourglass(16.0, 16.0, 1.0, phase)),
        Shape::Wait => {
            v.extend(arc(16.0, 16.0, 9.0, phase * 360.0, 270.0).map(|p| Piece::Line(p, 3.2)));
        }
        Shape::Progress => {
            hot = (2.0, 2.0);
            v.extend(poly(&arrow_points(arrow, 0.8, 1.0, 1.0)).map(Piece::Fill));
            if prefs.busy != BusyStyle::Ring {
                v.extend(hourglass(23.5, 22.5, 0.62, phase));
            } else {
                v.extend(arc(23.5, 23.5, 5.2, phase * 360.0, 270.0).map(|p| Piece::Line(p, 2.6)));
            }
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
    render_with(shape, box_px, &CursorPrefs { look, ..CursorPrefs::default() }, accent, phase)
}

/// Like `render`, with all the preferences: arrow shape, outline, busy style, custom hue.
pub fn render_with(shape: Shape, box_px: u32, prefs: &CursorPrefs, accent: [f32; 3], phase: f32) -> CursorImage {
    if shape == Shape::Hidden {
        let n = box_px.clamp(12, 64);
        return CursorImage { size: n, hot: (0, 0), argb: vec![0u32; (n * n) as usize] };
    }
    let (pieces, hot, turn) = pieces(shape, phase, prefs);
    paint(shape, &pieces, hot, turn, false, box_px, prefs, accent, phase)
}

/// Paint `pieces` (design units) into a box of `box_px` device px, in the colours `shape` has with these preferences,
/// turned by `turn` degrees around the box's middle. `snap`: move the drawing by the fraction of a pixel that rounding
/// the pointer's point to a whole pixel loses, so that a point that glides (a morph's in-between pictures) stays exactly
/// under the pointer instead of wobbling by half a pixel.
#[allow(clippy::too_many_arguments)]
fn paint(shape: Shape, pieces: &[Piece], hot: (f32, f32), turn: f32, snap: bool, box_px: u32, prefs: &CursorPrefs, accent: [f32; 3], phase: f32) -> CursorImage {
    let look = prefs.look;
    let n = box_px.clamp(12, 64);
    let mut argb = vec![0u32; (n * n) as usize];
    let mut out = CursorImage { size: n, hot: (0, 0), argb: Vec::new() };
    const INK: [f32; 3] = [0.05, 0.06, 0.08];
    const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
    let (fill, edge) = match look {
        CursorLook::Light => (WHITE, INK),
        CursorLook::Dark => ([0.06, 0.07, 0.09], WHITE),
        CursorLook::Accent => (accent, WHITE),
        CursorLook::Rainbow => (WHITE, INK),
        CursorLook::Hollow => (WHITE, INK),
        CursorLook::Custom => {
            let c = hsv(prefs.hue / 360.0, prefs.sat.clamp(0.0, 1.0), prefs.bright.clamp(0.0, 1.0));
            (c, if 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2] > 0.6 { INK } else { WHITE })
        }
    };
    // a rainbow fill: the look itself, or a busy cursor that asks for one (its colours flow with `phase`)
    let busy_shape = matches!(shape, Shape::Wait | Shape::Progress);
    let flowing = busy_shape && prefs.busy == BusyStyle::Rainbow;
    let rainbow = look == CursorLook::Rainbow || flowing;
    let hollow = look == CursorLook::Hollow;
    let (fill, edge) = if shape == Shape::NotAllowed { ([0.95, 0.25, 0.25], WHITE) } else { (fill, edge) };
    // an outline colour picked in the settings beats the look's own
    let edge = match prefs.edge {
        EdgeColor::Auto => edge,
        EdgeColor::Dark => INK,
        EdgeColor::Light => WHITE,
        EdgeColor::Accent => accent,
    };
    let Some(mut pm) = Pixmap::new(n, n) else {
        out.argb = argb;
        return out;
    };
    let k = n as f32 / DESIGN;
    // the hot spot, turned like the picture
    let (mut hx, mut hy) = hot;
    if turn != 0.0 {
        let a = turn.to_radians();
        let (dx, dy) = (hx - 16.0, hy - 16.0);
        hx = 16.0 + dx * a.cos() - dy * a.sin();
        hy = 16.0 + dx * a.sin() + dy * a.cos();
    }
    let (hpx, hpy) = ((hx * k).round(), (hy * k).round());
    let tf = if turn != 0.0 { Transform::from_rotate_at(turn, 16.0, 16.0).post_scale(k, k) } else { Transform::from_scale(k, k) };
    let tf = if snap { tf.post_translate(hpx - hx * k, hpy - hy * k) } else { tf };
    let mut paint = Paint { anti_alias: true, ..Paint::default() };
    let outline = prefs.outline.clamp(0.0, 3.0);
    // the outlines first, so that touching pieces (a hand's fingers) do not draw lines through each other
    paint.set_color(color(edge, 0.92));
    for p in pieces.iter().filter(|_| outline > 0.0 || hollow) {
        let (path, w) = match p {
            Piece::Fill(path) | Piece::Accent(path) => (path, if hollow { 3.6 } else { outline * 2.0 }),
            Piece::Line(path, w) => (path, w + if hollow { 2.0 } else { outline * 2.0 }),
        };
        let stroke = Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(path, &paint, &stroke, tf, None);
    }
    paint.set_color(color(fill, 1.0));
    if rainbow {
        let off = if flowing { phase } else { 0.0 };
        let stops: Vec<GradientStop> = (0..=6).map(|i| GradientStop::new(i as f32 / 6.0, color(hsv(off + i as f32 / 6.0, 0.85, 1.0), 1.0))).collect();
        if let Some(sh) = LinearGradient::new(Point::from_xy(0.0, 0.0), Point::from_xy(DESIGN, DESIGN), stops, SpreadMode::Pad, Transform::identity()) {
            paint.shader = sh;
        }
    }
    for p in pieces {
        match p {
            Piece::Fill(path) if hollow => {
                let stroke = Stroke { width: 1.6, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(path, &paint, &stroke, tf, None);
            }
            Piece::Fill(path) => pm.fill_path(path, &paint, FillRule::Winding, tf, None),
            Piece::Accent(path) => {
                // the accent itself; an accent look (already accent-filled) gets the outline colour so the dot shows
                let dot = Paint { anti_alias: true, shader: resvg::tiny_skia::Shader::SolidColor(color(if look == CursorLook::Accent { edge } else { accent }, 1.0)), ..Paint::default() };
                pm.fill_path(path, &dot, FillRule::Winding, tf, None);
            }
            Piece::Line(path, w) => {
                let stroke = Stroke { width: *w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(path, &paint, &stroke, tf, None);
            }
        }
    }
    for (i, px) in pm.pixels().iter().enumerate() {
        argb[i] = (px.alpha() as u32) << 24 | (px.red() as u32) << 16 | (px.green() as u32) << 8 | px.blue() as u32;
    }
    if prefs.shadow > 0.0 {
        add_shadow(&mut argb, n as usize, k, prefs.shadow.min(1.0));
    }
    out.hot = (hpx as i32, hpy as i32);
    out.argb = argb;
    out
}

/// A soft shadow under the picture: its own outline, blurred, a little down and to the right, put behind it.
/// `k` is device px per design unit. Only done when the picture is drawn again (a change of shape or settings).
fn add_shadow(argb: &mut [u32], n: usize, k: f32, strength: f32) {
    let (dx, dy) = ((0.6 * k).round() as i32, (1.2 * k).round() as i32);
    let r = ((1.1 * k).round() as usize).max(1);
    // the shape's coverage, moved
    let mut a = vec![0f32; n * n];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let (sx, sy) = (x - dx, y - dy);
            if sx >= 0 && sy >= 0 && (sx as usize) < n && (sy as usize) < n {
                a[y as usize * n + x as usize] = (argb[sy as usize * n + sx as usize] >> 24) as f32 / 255.0;
            }
        }
    }
    // two box blurs each way come close to a gaussian
    let mut tmp = vec![0f32; n * n];
    for _ in 0..2 {
        box_blur(&a, &mut tmp, n, r, 1, n);
        box_blur(&tmp, &mut a, n, r, n, 1);
    }
    let dark = 0.55 * strength;
    for (p, s) in argb.iter_mut().zip(&a) {
        let src = (*p >> 24) as f32 / 255.0;
        let sh = s * dark;
        if sh <= 0.0 || src >= 1.0 {
            continue;
        }
        // black under the picture: only the coverage grows (the colour is premultiplied, black adds nothing)
        let out = src + sh * (1.0 - src);
        *p = ((out * 255.0).round().min(255.0) as u32) << 24 | (*p & 0x00FF_FFFF);
    }
}

/// A running mean of width `2r + 1` along rows (`step` 1, `line` n) or columns (`step` n, `line` 1).
fn box_blur(src: &[f32], dst: &mut [f32], n: usize, r: usize, step: usize, line: usize) {
    let w = (2 * r + 1) as f32;
    for l in 0..n {
        let base = l * line;
        let at = |i: i32| if i < 0 || i >= n as i32 { 0.0 } else { src[base + i as usize * step] };
        let mut sum: f32 = (-(r as i32)..=r as i32).map(at).sum();
        for i in 0..n as i32 {
            dst[base + i as usize * step] = sum / w;
            sum += at(i + r as i32 + 1) - at(i - r as i32);
        }
    }
}

/// Notices the pointer being shaken (quick strokes back and forth) and says how much bigger to show the cursor for a
/// moment, so that it can be found. Fed with the pointer's motion in logical px.
pub struct Shake {
    /// the way the current stroke goes (-1, 0, 1) and how far it has gone
    dir: f32,
    stroke: f32,
    /// when the last strokes turned round
    turns: Vec<std::time::Instant>,
    held_until: Option<std::time::Instant>,
    /// 0 (normal) .. 1 (fully grown), eased in and out
    level: f32,
    last: std::time::Instant,
}

/// How far a stroke must go to count (logical px), how many turns make a shake, and how long they may take.
const SHAKE_STROKE: f32 = 36.0;
const SHAKE_TURNS: usize = 4;
const SHAKE_WINDOW: std::time::Duration = std::time::Duration::from_millis(1000);
/// how much bigger the cursor grows (the 64 px cursor plane caps it for big cursors)
pub const SHAKE_ZOOM: f32 = 2.4;

impl Default for Shake {
    fn default() -> Self {
        Shake { dir: 0.0, stroke: 0.0, turns: Vec::new(), held_until: None, level: 0.0, last: std::time::Instant::now() }
    }
}

impl Shake {
    pub fn motion(&mut self, dx: f32, now: std::time::Instant) {
        // sideways strokes: that is how people shake a mouse, and up-and-down scrolling-like moves stay out of it
        if dx == 0.0 {
            return;
        }
        let s = dx.signum();
        if s == self.dir {
            self.stroke += dx.abs();
        } else {
            if self.stroke >= SHAKE_STROKE {
                self.turns.push(now);
            }
            self.dir = s;
            self.stroke = dx.abs();
        }
        self.turns.retain(|t| now.duration_since(*t) < SHAKE_WINDOW);
        if self.turns.len() >= SHAKE_TURNS {
            self.held_until = Some(now + std::time::Duration::from_millis(700));
        }
    }

    /// How big the cursor is now (1 = as set in the settings); it grows quickly and shrinks back slowly.
    pub fn zoom(&mut self, now: std::time::Instant) -> f32 {
        let dt = now.saturating_duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let grow = self.held_until.is_some_and(|t| t > now);
        self.level = if grow { (self.level + dt * 6.0).min(1.0) } else { (self.level - dt * 2.5).max(0.0) };
        let t = self.level * self.level * (3.0 - 2.0 * self.level);
        1.0 + (SHAKE_ZOOM - 1.0) * t
    }

    /// Still growing or shrinking (the picture has to be drawn again soon).
    pub fn moving(&self, now: std::time::Instant) -> bool {
        self.level > 0.0 || self.held_until.is_some_and(|t| t > now)
    }
}

// ---- changing shape smoothly
//
// When the cursor changes shape (arrow to text beam, hand to closed hand, one resize arrow to another...) the picture is
// not swapped at once: for a moment (`MORPH_SECS`) one shape turns into the other.
//
// * Shapes built the same way are blended geometrically ([`Pose`]): the resize arrows turn by the shorter way to their
//   new angle; the hands (pointing, open, closed) move each finger, the palm and the thumb to their new boxes; the arrow,
//   the arrow with a "+" (copy) and the arrow with a busy ring resize the arrow and grow or shrink the badge. The blend
//   eases out with a small overshoot, like a spring that settles.
// * Any other pair cross-fades and scales ([`blend`]): the old picture shrinks and fades, the new one grows from a little
//   smaller (overshooting slightly) and fades in. Programs' own pictures, Xcursor themes and the hidden cursor take this
//   way too.
//
// The pointer's point. Input never uses the picture: a click lands where the pointer is, and that never moves while the
// cursor changes shape. Every in-between picture puts *its* point exactly on the pointer: the cross-fade scales each
// picture around its own hot spot and lines the two hot spots up, and a geometric blend moves the hot spot together with
// the drawing (a pointing fingertip gliding to the middle of a closed hand, say) and then places the picture so that the
// blended point sits on the pointer. So the part of the picture that says "here" is always where a click goes; nothing
// in the animation ever shows a click point the button would not use.
//
// A change in the middle of a change starts from what is on screen at that moment: a geometric blend carries on from the
// in-between pose, anything else cross-fades from the last picture shown, so a quick A -> B -> C never pops.

/// How long a change of shape takes at speed 1, seconds.
pub const MORPH_SECS: f32 = 0.16;

/// A shape's geometry as numbers that can be blended with another shape of the same kind.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pose {
    /// the two-headed arrow, turned by this angle (degrees; it looks the same turned half round)
    Resize(f32),
    /// a hand: see [`Hand`]
    Hand([[f32; 5]; 6], (f32, f32)),
    /// the arrow (its size and its offset in x and y), and how much of the "+" badge and of the busy badge show (0 .. 1)
    Arrow([f32; 3], f32, f32),
}

/// The pose of `shape`, if it can be blended at all (the shell's own drawing only, not a theme's picture).
fn pose_of(shape: Shape, prefs: &CursorPrefs) -> Option<Pose> {
    if !prefs.theme.is_empty() {
        return None;
    }
    let plain = Pose::Arrow([1.0, 0.0, 0.0], 0.0, 0.0);
    Some(match shape {
        Shape::Default => plain,
        Shape::Pointer if prefs.hand == HandStyle::Arrow => plain,
        Shape::Copy => Pose::Arrow([0.85, 1.0, 1.0], 1.0, 0.0),
        // (a rainbow busy cursor has colours of its own: it cross-fades)
        Shape::Progress if prefs.busy != BusyStyle::Rainbow => Pose::Arrow([0.8, 1.0, 1.0], 0.0, 1.0),
        Shape::Pointer => Pose::Hand(HAND_POINTER.0, HAND_POINTER.1),
        Shape::Grab => Pose::Hand(HAND_GRAB.0, HAND_GRAB.1),
        Shape::Grabbing => Pose::Hand(HAND_GRABBING.0, HAND_GRABBING.1),
        Shape::ResizeEw => Pose::Resize(0.0),
        Shape::ResizeNs => Pose::Resize(90.0),
        Shape::ResizeNesw => Pose::Resize(-45.0),
        Shape::ResizeNwse => Pose::Resize(45.0),
        _ => return None,
    })
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Pose {
    /// `t` of the way from this pose to `to` (t may overshoot 1 a little); None when the two are not of one kind.
    fn lerp(&self, to: &Pose, t: f32) -> Option<Pose> {
        Some(match (self, to) {
            (Pose::Resize(a), Pose::Resize(b)) => {
                // the shorter way round: the arrow has two heads, so half a turn brings it back
                let d = (b - a + 90.0).rem_euclid(180.0) - 90.0;
                Pose::Resize(a + d * t)
            }
            (Pose::Hand(a, ha), Pose::Hand(b, hb)) => Pose::Hand(std::array::from_fn(|i| std::array::from_fn(|j| mix(a[i][j], b[i][j], t))), (mix(ha.0, hb.0, t), mix(ha.1, hb.1, t))),
            (Pose::Arrow(a, pa, ba), Pose::Arrow(b, pb, bb)) => Pose::Arrow(std::array::from_fn(|i| mix(a[i], b[i], t)), mix(*pa, *pb, t), mix(*ba, *bb, t)),
            _ => return None,
        })
    }

    /// The pieces, the pointer's point and the turn, as `pieces` gives them for a shape.
    fn pieces(&self, phase: f32, prefs: &CursorPrefs) -> (Vec<Piece>, (f32, f32), f32) {
        match self {
            Pose::Resize(angle) => {
                let (v, hot, _) = pieces(Shape::ResizeEw, phase, prefs);
                (v, hot, *angle)
            }
            Pose::Hand(boxes, hot) => (hand_pieces(boxes), *hot, 0.0),
            Pose::Arrow([k, dx, dy], plus, busy) => {
                let mut v: Vec<Piece> = poly(&arrow_points(prefs.arrow, *k, *dx, *dy)).map(Piece::Fill).into_iter().collect();
                // the badges grow from their middles (too small to see: left out, rather than a speck of outline)
                let plus = plus.clamp(0.0, 1.2);
                if plus > 0.12 {
                    let r = 5.0 * plus;
                    v.extend(line((24.0, 24.0 - r), (24.0, 24.0 + r)).map(|p| Piece::Line(p, 2.6)));
                    v.extend(line((24.0 - r, 24.0), (24.0 + r, 24.0)).map(|p| Piece::Line(p, 2.6)));
                }
                let busy = busy.clamp(0.0, 1.2);
                if busy > 0.12 {
                    if prefs.busy != BusyStyle::Ring {
                        v.extend(hourglass(23.5, 22.5, 0.62 * busy, phase));
                    } else {
                        v.extend(arc(23.5, 23.5, 5.2 * busy, phase * 360.0, 270.0).map(|p| Piece::Line(p, 2.6 * busy.min(1.0))));
                    }
                }
                (v, (2.0, 2.0), 0.0)
            }
        }
    }
}

/// Eases out with a small overshoot (about 4%) and settles exactly on 1 at `u` = 1: a spring that is nearly damped.
fn ease_spring(u: f32) -> f32 {
    if u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    let x = u - 1.0;
    const C: f32 = 1.1;
    1.0 + (C + 1.0) * x * x * x + C * x * x
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How the two pictures of a cross-fade are drawn `u` (0 .. 1) of the way through: (scale, opacity) of the old one and of
/// the new one. The old one is gone by two thirds of the way; the new one comes in from 60% of its size, overshoots a
/// little and is exactly itself (scale 1, opaque) at the end.
fn fade_curves(u: f32) -> ((f32, f32), (f32, f32)) {
    let u = u.clamp(0.0, 1.0);
    let out = (u / 0.65).min(1.0);
    let old = (1.0 - 0.35 * out * out, 1.0 - smoothstep(0.0, 0.65, u));
    let new = (0.6 + 0.4 * ease_spring(u), smoothstep(0.05, 0.75, u));
    (old, new)
}

/// The largest cross-fade picture (a side, device px): enough for the biggest cursor plane.
const BLEND_MAX: i32 = 256;

/// The old picture `from` and the new one `to` cross-faded `u` (0 .. 1) of the way, each scaled around its own hot spot,
/// with the two hot spots on one pixel (the pointer). Premultiplied ARGB like the pictures.
fn blend(from: &CursorImage, to: &CursorImage, u: f32) -> CursorImage {
    let ((so, ao), (sn, an)) = fade_curves(u);
    // a margin for the new picture's overshoot
    let pad = 2;
    let hot = (from.hot.0.max(to.hot.0) + pad, from.hot.1.max(to.hot.1) + pad);
    let reach = |img: &CursorImage| (hot.0 - img.hot.0 + img.size as i32).max(hot.1 - img.hot.1 + img.size as i32);
    let size = (reach(from).max(reach(to)) + pad).min(BLEND_MAX).max(1);
    let mut argb = vec![0u32; (size * size) as usize];
    // one picture over what is there: `s` its scale, `a` its opacity
    let mut layer = |img: &CursorImage, s: f32, a: f32| {
        if a <= 0.0 || s <= 0.01 {
            return;
        }
        let n = img.size as i32;
        let texel = |x: i32, y: i32| -> [f32; 4] {
            if x < 0 || y < 0 || x >= n || y >= n {
                return [0.0; 4];
            }
            let p = img.argb[(y * n + x) as usize];
            [(p >> 24) as f32, ((p >> 16) & 255) as f32, ((p >> 8) & 255) as f32, (p & 255) as f32]
        };
        // the picture's box on the canvas, scaled around its hot spot
        let (hx, hy) = (img.hot.0 as f32, img.hot.1 as f32);
        let x0 = ((hot.0 as f32 - hx * s).floor() as i32).max(0);
        let y0 = ((hot.1 as f32 - hy * s).floor() as i32).max(0);
        let x1 = ((hot.0 as f32 + (n as f32 - hx) * s).ceil() as i32).min(size);
        let y1 = ((hot.1 as f32 + (n as f32 - hy) * s).ceil() as i32).min(size);
        for y in y0..y1 {
            for x in x0..x1 {
                // the canvas pixel's middle, in the picture's px (at scale 1 exactly one of its pixels: a plain copy)
                let sx = (x as f32 + 0.5 - hot.0 as f32) / s + hx - 0.5;
                let sy = (y as f32 + 0.5 - hot.1 as f32) / s + hy - 0.5;
                let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
                let (fx, fy) = (sx - ix as f32, sy - iy as f32);
                let (p, q, r, t) = (texel(ix, iy), texel(ix + 1, iy), texel(ix, iy + 1), texel(ix + 1, iy + 1));
                let c: [f32; 4] = std::array::from_fn(|i| ((p[i] * (1.0 - fx) + q[i] * fx) * (1.0 - fy) + (r[i] * (1.0 - fx) + t[i] * fx) * fy) * a);
                if c[0] <= 0.0 {
                    continue;
                }
                let at = (y * size + x) as usize;
                let under = argb[at];
                let keep = 1.0 - c[0] / 255.0;
                let ch = |sh: u32, top: f32| ((((under >> sh) & 255) as f32 * keep + top).round().clamp(0.0, 255.0) as u32) << sh;
                argb[at] = ch(24, c[0]) | ch(16, c[1]) | ch(8, c[2]) | ch(0, c[3]);
            }
        }
    };
    layer(from, so, ao);
    layer(to, sn, an);
    CursorImage { size: size as u32, hot, argb }
}

/// What a change of shape starts from.
#[derive(Clone, Debug)]
enum Start {
    /// a geometric blend from one pose to the other
    Pose(Pose, Pose),
    /// a cross-fade from this picture (the one on screen when the change began) to the new shape's
    Picture(CursorImage),
}

#[derive(Clone, Debug)]
struct Run {
    began: std::time::Instant,
    secs: f32,
    start: Start,
}

impl Run {
    fn progress(&self, now: std::time::Instant) -> f32 {
        now.saturating_duration_since(self.began).as_secs_f32() / self.secs.max(0.001)
    }
}

/// The cursor picture while the shape changes: keeps what was shown last and the change in progress (see the notes
/// above [`MORPH_SECS`]). The compositor tells it of a new shape with [`Morph::retarget`] and asks for the picture of
/// the moment with [`Morph::frame`], as long as [`Morph::running`] says so (and whenever else it would draw anyway).
#[derive(Default)]
pub struct Morph {
    /// the picture last handed out (what a change starts from)
    shown: Option<CursorImage>,
    /// the shape it is going to (or is)
    target: Shape,
    /// that shape's picture is the shell's drawing alone (no program's picture, no icon beside it)
    plain: bool,
    run: Option<Run>,
}

impl Morph {
    /// The shape the cursor is or is changing to.
    pub fn target(&self) -> Shape {
        self.target
    }

    /// A change is in progress: the picture must be drawn again soon (every frame), until this turns false.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The cursor is now to show `to`. `plain`: its picture is the shell's own drawing and nothing else (not a program's
    /// picture, no launch icon beside it); only then may it be blended geometrically.
    pub fn retarget(&mut self, to: Shape, plain: bool, prefs: &CursorPrefs, now: std::time::Instant) {
        if to == self.target {
            self.plain = plain;
            return;
        }
        let (from, was_plain) = (self.target, self.plain);
        self.target = to;
        self.plain = plain;
        let Some(shown) = &self.shown else { return };
        if !prefs.morph {
            self.run = None;
            return;
        }
        // where the picture is now, as a pose (mid-way through a blend: the in-between pose)
        let current = match &self.run {
            Some(run @ Run { start: Start::Pose(a, b), .. }) => a.lerp(b, ease_spring(run.progress(now))),
            Some(_) => None,
            None => pose_of(from, prefs).filter(|_| was_plain),
        };
        let next = pose_of(to, prefs).filter(|_| plain);
        let start = match (current, next) {
            (Some(a), Some(b)) if a.lerp(&b, 0.0).is_some() => {
                if a == b {
                    // (the same drawing: an arrow over links that looks like the arrow)
                    self.run = None;
                    return;
                }
                Start::Pose(a, b)
            }
            _ => Start::Picture(shown.clone()),
        };
        self.run = Some(Run { began: now, secs: MORPH_SECS / prefs.morph_speed.clamp(0.5, 2.0), start });
    }

    /// Forget the change in progress and what was shown (the next picture is put up as it is).
    pub fn reset(&mut self) {
        self.run = None;
        self.shown = None;
    }

    /// The picture to show at `now`. `picture` draws the target shape's own picture (as the compositor would without a
    /// change: theme, program's picture, launch icon and all); a geometric blend draws its in-between poses itself, into
    /// a box of `box_px` device px. When the change is over this is exactly `picture()`.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, now: std::time::Instant, prefs: &CursorPrefs, box_px: u32, accent: [f32; 3], phase: f32, picture: impl FnOnce() -> CursorImage) -> &CursorImage {
        let img = match self.run.take() {
            Some(run) if run.progress(now) < 1.0 => {
                let u = run.progress(now);
                let img = match &run.start {
                    Start::Pose(a, b) => match a.lerp(b, ease_spring(u)) {
                        Some(p) => {
                            let (pieces, hot, turn) = p.pieces(phase, prefs);
                            paint(Shape::Default, &pieces, hot, turn, true, box_px, prefs, accent, phase)
                        }
                        None => picture(),
                    },
                    Start::Picture(from) => blend(from, &picture(), u),
                };
                self.run = Some(run);
                img
            }
            _ => picture(),
        };
        self.shown.insert(img)
    }
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
        let mut img = image::RgbaImage::from_pixel(64 * shapes.len() as u32 * 2, 192, image::Rgba([120, 130, 150, 255]));
        // the light and dark looks, then the light look with the dot over edges
        let rows = [(CursorLook::Light, ResizeStyle::Line), (CursorLook::Dark, ResizeStyle::Line), (CursorLook::Light, ResizeStyle::Dot)];
        for (row, (look, resize)) in rows.iter().enumerate() {
            for (i, s) in shapes.iter().enumerate() {
                let c = render_with(*s, 56, &CursorPrefs { look: *look, resize: *resize, ..CursorPrefs::default() }, [0.1, 0.5, 1.0], 0.1);
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
    fn old_cursor_settings_still_load() {
        // a config written before the newer options: they take their defaults
        let old: CursorPrefs = toml::from_str("size = 32.0\nlook = \"dark\"\nhue = 100.0\noutline = 1.0\narrow = \"slim\"\nbusy = \"hourglass\"\n").unwrap();
        assert_eq!(old.size, 32.0);
        assert_eq!(old.look, CursorLook::Dark);
        assert!(old.theme.is_empty());
        assert_eq!((old.edge, old.hand, old.beam, old.shadow, old.busy_speed, old.shake_to_find), (EdgeColor::Auto, HandStyle::Hand, BeamStyle::Classic, 0.0, 1.0, false));
        let back: CursorPrefs = toml::from_str(&toml::to_string(&old).unwrap()).unwrap();
        assert_eq!(back, old);
    }

    #[test]
    fn options_change_the_picture() {
        let base = CursorPrefs::default();
        let draw = |p: &CursorPrefs, s: Shape| render_with(s, 32, p, [0.1, 0.5, 1.0], 0.0).argb;
        let cover = |v: &[u32]| v.iter().map(|p| (p >> 24) as u32).sum::<u32>();
        // a shadow only adds coverage around the shape
        let shadowed = CursorPrefs { shadow: 1.0, ..base.clone() };
        assert!(cover(&draw(&shadowed, Shape::Default)) > cover(&draw(&base, Shape::Default)));
        // the arrow over links, and the text beam variants
        let arrow = CursorPrefs { hand: HandStyle::Arrow, ..base.clone() };
        assert_eq!(draw(&arrow, Shape::Pointer), draw(&base, Shape::Default));
        assert_eq!(render_with(Shape::Pointer, 32, &arrow, [0.0; 3], 0.0).hot, render_with(Shape::Default, 32, &base, [0.0; 3], 0.0).hot);
        for b in [BeamStyle::Plain, BeamStyle::Bold] {
            assert_ne!(draw(&CursorPrefs { beam: b, ..base.clone() }, Shape::Text), draw(&base, Shape::Text), "{b:?}");
        }
        // the outline's colour and the custom fill
        assert_ne!(draw(&CursorPrefs { edge: EdgeColor::Accent, ..base.clone() }, Shape::Default), draw(&base, Shape::Default));
        let dim = CursorPrefs { look: CursorLook::Custom, bright: 0.3, ..base.clone() };
        assert_ne!(draw(&dim, Shape::Default), draw(&CursorPrefs { look: CursorLook::Custom, ..base.clone() }, Shape::Default));
        // the busy pace
        assert!(CursorPrefs { busy_speed: 2.0, ..base.clone() }.busy_rate() > base.busy_rate());
        // the dot between two heads over edges: nothing joins them, and the dot takes the accent
        let dot = CursorPrefs { resize: ResizeStyle::Dot, ..base.clone() };
        for s in [Shape::ResizeEw, Shape::ResizeNs, Shape::ResizeNesw, Shape::ResizeNwse] {
            assert_ne!(draw(&dot, s), draw(&base, s), "{s:?}");
        }
        let ew = draw(&dot, Shape::ResizeEw);
        let at = |x: usize| ew[16 * 32 + x];
        assert_eq!(at(16) & 0x00ff_ffff, 0x001a80ff, "{:08x}", at(16));
        assert!(at(10) >> 24 < 0x80, "between a head and the dot: {:08x}", at(10));
    }

    #[test]
    fn shaking_grows_the_cursor_and_it_shrinks_back() {
        use std::time::{Duration, Instant};
        let mut s = Shake::default();
        let t0 = Instant::now();
        assert_eq!(s.zoom(t0), 1.0);
        // slow moves one way do nothing
        for i in 0..20 {
            s.motion(10.0, t0 + Duration::from_millis(i * 20));
        }
        assert_eq!(s.zoom(t0 + Duration::from_millis(400)), 1.0);
        // quick strokes back and forth
        let mut t = t0 + Duration::from_millis(500);
        for stroke in 0..6 {
            let dir = if stroke % 2 == 0 { -1.0 } else { 1.0 };
            for _ in 0..4 {
                t += Duration::from_millis(15);
                s.motion(dir * 15.0, t);
            }
        }
        let mut z = 1.0;
        for _ in 0..10 {
            t += Duration::from_millis(30);
            z = s.zoom(t);
        }
        assert!(z > 2.0, "grown to {z}");
        for _ in 0..40 {
            t += Duration::from_millis(50);
            z = s.zoom(t);
        }
        assert_eq!(z, 1.0);
        assert!(!s.moving(t));
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

    /// `CURSOR_BUSY_SHEET=/path.png cargo test cursor_busy_sheet -- --ignored`: the hourglass at several phases, and the new looks.
    #[test]
    #[ignore]
    fn cursor_busy_sheet() {
        let Ok(path) = std::env::var("CURSOR_BUSY_SHEET") else { return };
        let phases = [0.0, 0.2, 0.4, 0.6, 0.78, 0.85, 0.92];
        let mut img = image::RgbaImage::from_pixel(64 * 2 * phases.len() as u32, 64 * 6, image::Rgba([120, 130, 150, 255]));
        let rows: [(Shape, CursorLook, BusyStyle, ArrowStyle); 6] = [
            (Shape::Wait, CursorLook::Light, BusyStyle::Rainbow, ArrowStyle::Classic),
            (Shape::Progress, CursorLook::Dark, BusyStyle::Hourglass, ArrowStyle::Slim),
            (Shape::Default, CursorLook::Rainbow, BusyStyle::Ring, ArrowStyle::Classic),
            (Shape::Default, CursorLook::Hollow, BusyStyle::Ring, ArrowStyle::Triangle),
            // the newer options: a shadow, the custom fill with an accent outline, the bold text beam
            (Shape::Default, CursorLook::Custom, BusyStyle::Ring, ArrowStyle::Classic),
            (Shape::Text, CursorLook::Light, BusyStyle::Ring, ArrowStyle::Classic),
        ];
        for (row, (shape, look, busy, arrow)) in rows.iter().enumerate() {
            for (i, ph) in phases.iter().enumerate() {
                let mut prefs = CursorPrefs { look: *look, busy: *busy, arrow: *arrow, ..CursorPrefs::default() };
                if row >= 4 {
                    // across the row: more shadow, and for the custom look a darker fill
                    prefs.shadow = i as f32 / (phases.len() - 1) as f32;
                    prefs.bright = 1.0 - 0.5 * prefs.shadow;
                    prefs.hue = 30.0;
                    prefs.edge = if row == 4 { EdgeColor::Accent } else { EdgeColor::Auto };
                    prefs.beam = BeamStyle::Bold;
                }
                let c = render_with(*shape, 56, &prefs, [0.1, 0.5, 1.0], *ph);
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

    /// A stand-in program icon: a rounded square with a gradient and a white dot.
    fn test_icon(px: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(px, px, |x, y| {
            let (u, v) = ((x as f32 + 0.5) / px as f32, (y as f32 + 0.5) / px as f32);
            let (dx, dy) = ((u - 0.5).abs() - 0.32, (v - 0.5).abs() - 0.32);
            let out = (dx.max(0.0).hypot(dy.max(0.0)) - 0.14).max(dx.max(dy).min(0.0) - 0.14);
            let a = (0.5 - out * px as f32).clamp(0.0, 1.0);
            let dot = (u - 0.5).hypot(v - 0.5) < 0.17;
            let c = if dot { [255, 255, 255] } else { [(60.0 + 160.0 * u) as u8, (90.0 + 60.0 * v) as u8, 230] };
            image::Rgba([c[0], c[1], c[2], (a * 255.0) as u8])
        })
    }

    #[test]
    fn old_cursor_prefs_get_the_launch_defaults() {
        let p: CursorPrefs = toml::from_str("size = 32.0\nbusy_speed = 1.5\n").unwrap();
        assert_eq!(p.size, 32.0);
        assert!(p.launch_icon);
        assert_eq!(p.launch_anim, LaunchAnim::Bounce);
        assert_eq!(p.launch_size, CursorPrefs::default().launch_size);
        let back: CursorPrefs = toml::from_str(&toml::to_string(&CursorPrefs { launch_anim: LaunchAnim::Pulse, launch_dy: -4.0, ..p.clone() }).unwrap()).unwrap();
        assert_eq!(back.launch_anim, LaunchAnim::Pulse);
        assert_eq!(back.launch_dy, -4.0);
    }

    #[test]
    fn launch_icon_never_covers_the_cursor_and_fits_the_plane() {
        let icon = test_icon(48);
        for (box_px, scale) in [(28u32, 1.0f32), (35, 1.25), (56, 2.0), (64, 2.0)] {
            for anim in [LaunchAnim::Bounce, LaunchAnim::Pulse, LaunchAnim::Still] {
                for max in [64u32, 128, 256] {
                    for (dx, dy, size, height) in [(3.0, 0.0, 22.0, 10.0), (0.0, -20.0, 40.0, 30.0), (12.0, 16.0, 64.0, 48.0)] {
                        let prefs = CursorPrefs { launch_anim: anim, launch_dx: dx, launch_dy: dy, launch_size: size, launch_height: height, ..CursorPrefs::default() };
                        let cur = render_with(Shape::Progress, box_px, &prefs, [0.1, 0.5, 1.0], 0.3);
                        let (bx0, by0, bx1, by1) = visible_bounds(&cur).unwrap();
                        let lay = LaunchLayout::new(&cur, &prefs, scale, max);
                        assert!(lay.size <= max, "{box_px} {anim:?} {max}: {} > {max}", lay.size);
                        let mut hot = None;
                        for i in 0..200 {
                            let t = i as f32 * 0.013;
                            let [x, y, w, h] = lay.icon_box(launch_pose(&prefs, t), anim);
                            let clear = x >= bx1 as f32 || x + w <= bx0 as f32 || y >= by1 as f32 || y + h <= by0 as f32;
                            assert!(clear, "{box_px} {anim:?} t {t}: icon {:?} over the cursor {:?}", [x, y, w, h], (bx0, by0, bx1, by1));
                            // inside the picture (the picture may have been cut down only when even the smallest icon did not fit)
                            let (ox, oy) = (lay.origin.0 as f32, lay.origin.1 as f32);
                            let room = max >= 128 || box_px <= 35;
                            assert!(!room || x + ox >= -0.01 && y + oy >= -0.01 && x + w + ox <= lay.size as f32 + 0.01 && y + h + oy <= lay.size as f32 + 0.01, "{box_px} {anim:?} {max} t {t}: {:?} outside {}", [x + ox, y + oy, w, h], lay.size);
                            let pic = with_launch_icon(render_with(Shape::Progress, box_px, &prefs, [0.1, 0.5, 1.0], 0.3), &icon, &prefs, scale, t, max);
                            // the hot spot stays put while the icon moves
                            assert_eq!(*hot.get_or_insert(pic.hot), pic.hot);
                            if i > 20 {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bounce_goes_up_and_comes_down() {
        let prefs = CursorPrefs::default();
        let lifts: Vec<f32> = (0..72).map(|i| launch_pose(&prefs, i as f32 * 0.01).0).collect();
        let top = lifts.iter().cloned().fold(0.0, f32::max);
        assert!(top > 0.95 && lifts[0] < 0.05);
        // a hop, then a squash on the ground
        assert!((60..72).all(|i| lifts[i] == 0.0));
        assert!(launch_pose(&prefs, 0.65).1 > 1.05);
        assert_eq!(launch_pose(&CursorPrefs { launch_anim: LaunchAnim::Still, ..prefs }, 0.3), (0.0, 1.0, 1.0, 1.0));
    }

    /// `LAUNCH_FRAMES=/dir cargo test --lib launch_frames -- --ignored`: the busy cursor with a launch icon at a few moments
    /// of each animation, on a grey sheet (`LAUNCH_ICON=/some.png` for a real icon), magnified 3x.
    #[test]
    #[ignore]
    fn launch_frames() {
        let Ok(dir) = std::env::var("LAUNCH_FRAMES") else { return };
        let icon = std::env::var("LAUNCH_ICON").ok().and_then(|p| image::open(p).ok()).map(|i| i.to_rgba8()).unwrap_or_else(|| test_icon(48));
        let times: Vec<f32> = (0..12).map(|i| i as f32 * 0.06).collect();
        for (name, anim, scale, look) in [("bounce", LaunchAnim::Bounce, 1.25f32, CursorLook::Light), ("bounce_dark_2x", LaunchAnim::Bounce, 2.0, CursorLook::Dark), ("pulse", LaunchAnim::Pulse, 1.25, CursorLook::Light), ("still", LaunchAnim::Still, 1.25, CursorLook::Light)] {
            let prefs = CursorPrefs { launch_anim: anim, look, ..CursorPrefs::default() };
            let box_px = (prefs.size * scale).round() as u32;
            let cell = (64.0 * scale) as u32;
            let mag = if scale > 1.5 { 2 } else { 3 };
            let mut sheet = image::RgbaImage::from_pixel(cell * times.len() as u32 * mag, cell * mag, image::Rgba([120, 130, 150, 255]));
            for (i, t) in times.iter().enumerate() {
                let cur = render_with(Shape::Progress, box_px, &prefs, [0.1, 0.5, 1.0], *t);
                let pic = with_launch_icon(cur, &icon, &prefs, scale, *t, 128);
                // the hot spot at the same place in every cell
                let (x0, y0) = (i as i32 * cell as i32 + 8, cell as i32 * 2 / 5 - pic.hot.1);
                for y in 0..pic.size {
                    for x in 0..pic.size {
                        let p = pic.argb[(y * pic.size + x) as usize];
                        let a = (p >> 24) as f32 / 255.0;
                        let (cx, cy) = (x0 + x as i32, y0 + y as i32);
                        if a <= 0.0 || cx < i as i32 * cell as i32 || cy < 0 || cx >= (i as i32 + 1) * cell as i32 || cy >= cell as i32 {
                            continue;
                        }
                        for my in 0..mag {
                            for mx in 0..mag {
                                let (px, py) = (cx as u32 * mag + mx, cy as u32 * mag + my);
                                let old = sheet.get_pixel(px, py).0;
                                let mix = |sh: u32, o: u8| (((p >> sh) & 255) as f32 + o as f32 * (1.0 - a)) as u8;
                                sheet.put_pixel(px, py, image::Rgba([mix(16, old[0]), mix(8, old[1]), mix(0, old[2]), 255]));
                            }
                        }
                    }
                }
                // the picture's frame (the hardware plane's content), a thin line
                for k in 0..pic.size {
                    for (fx, fy) in [(k, 0), (k, pic.size - 1), (0, k), (pic.size - 1, k)] {
                        let (cx, cy) = (x0 + fx as i32, y0 + fy as i32);
                        if cx >= i as i32 * cell as i32 && cy >= 0 && cx < (i as i32 + 1) * cell as i32 && cy < cell as i32 {
                            sheet.put_pixel(cx as u32 * mag + mag / 2, cy as u32 * mag + mag / 2, image::Rgba([255, 230, 0, 255]));
                        }
                    }
                }
            }
            sheet.save(format!("{dir}/launch_{name}.png")).unwrap();
        }
    }

    /// A morph that has shown `from` and is then sent to `to` at `t0`.
    fn morph_from(from: Shape, to: Shape, prefs: &CursorPrefs, t0: std::time::Instant) -> Morph {
        let mut m = Morph::default();
        m.retarget(from, true, prefs, t0);
        m.frame(t0, prefs, 32, [0.1, 0.5, 1.0], 0.0, || render_with(from, 32, prefs, [0.1, 0.5, 1.0], 0.0));
        m.retarget(to, true, prefs, t0);
        m
    }

    fn at(m: &mut Morph, shape: Shape, prefs: &CursorPrefs, t: std::time::Instant) -> CursorImage {
        m.frame(t, prefs, 32, [0.1, 0.5, 1.0], 0.0, || render_with(shape, 32, prefs, [0.1, 0.5, 1.0], 0.0)).clone()
    }

    /// Do two pictures look the same with their hot spots on one point (what is outside either counts as clear)?
    fn same_on_screen(a: &CursorImage, b: &CursorImage) -> bool {
        let px = |img: &CursorImage, x: i32, y: i32| {
            let (x, y) = (x + img.hot.0, y + img.hot.1);
            if x < 0 || y < 0 || x >= img.size as i32 || y >= img.size as i32 { 0 } else { img.argb[(y * img.size as i32 + x) as usize] }
        };
        (-300..300).all(|y| (-300..300).all(|x| px(a, x, y) == px(b, x, y)))
    }

    fn coverage(img: &CursorImage) -> u32 {
        img.argb.iter().map(|p| p >> 24).sum()
    }

    #[test]
    fn morph_curves_start_and_settle() {
        assert_eq!(ease_spring(0.0), 0.0);
        assert_eq!(ease_spring(1.0), 1.0);
        let most = (0..=100).map(|i| ease_spring(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(most > 1.0 && most < 1.06, "a small overshoot: {most}");
        let ((so, ao), (sn, an)) = fade_curves(0.0);
        assert_eq!((so, ao, an), (1.0, 1.0, 0.0));
        assert!(sn < 1.0);
        let ((_, ao), (sn, an)) = fade_curves(1.0);
        assert_eq!((ao, sn, an), (0.0, 1.0, 1.0));
    }

    #[test]
    fn resize_arrows_turn_the_short_way() {
        let p = CursorPrefs::default();
        let turn = |a: Shape, b: Shape, t: f32| match pose_of(a, &p).unwrap().lerp(&pose_of(b, &p).unwrap(), t).unwrap() {
            Pose::Resize(r) => r,
            other => panic!("{other:?}"),
        };
        // (a quarter turn either way is as short: one of them)
        assert_eq!(turn(Shape::ResizeEw, Shape::ResizeNs, 0.5).abs(), 45.0);
        // 90 -> -45 is a quarter turn less by going on to 135 (the same arrow)
        assert_eq!(turn(Shape::ResizeNs, Shape::ResizeNesw, 1.0), 135.0);
        assert_eq!((turn(Shape::ResizeNesw, Shape::ResizeNwse, 1.0) + 45.0).abs(), 90.0);
        assert_eq!(turn(Shape::ResizeNwse, Shape::ResizeEw, 1.0), 0.0);
        // unrelated shapes are not blended geometrically
        assert!(pose_of(Shape::Default, &p).unwrap().lerp(&pose_of(Shape::Grab, &p).unwrap(), 0.5).is_none());
        assert!(pose_of(Shape::Text, &p).is_none());
        assert!(pose_of(Shape::Grab, &CursorPrefs { theme: "Breeze".into(), ..p.clone() }).is_none());
    }

    #[test]
    fn poses_draw_their_shapes() {
        // the end of each geometric blend is the shape itself (box 32: one px per unit, nothing to snap)
        let p = CursorPrefs::default();
        for (a, b) in [(Shape::Grab, Shape::Grabbing), (Shape::Grabbing, Shape::Pointer), (Shape::Pointer, Shape::Grab), (Shape::ResizeEw, Shape::ResizeNwse), (Shape::Default, Shape::Copy), (Shape::Copy, Shape::Progress)] {
            let pose = pose_of(a, &p).unwrap().lerp(&pose_of(b, &p).unwrap(), 1.0).unwrap();
            let (pieces, hot, turn) = pose.pieces(0.0, &p);
            let img = paint(Shape::Default, &pieces, hot, turn, true, 32, &p, [0.1, 0.5, 1.0], 0.0);
            let want = render_with(b, 32, &p, [0.1, 0.5, 1.0], 0.0);
            assert_eq!(img.hot, want.hot, "{a:?} -> {b:?}");
            let diff = img.argb.iter().zip(&want.argb).filter(|(x, y)| x != y).count();
            assert!(diff <= 4, "{a:?} -> {b:?}: {diff} px differ");
        }
    }

    #[test]
    fn morph_ends_on_exactly_the_new_picture() {
        let p = CursorPrefs::default();
        let t0 = std::time::Instant::now();
        for (a, b) in [(Shape::Default, Shape::Text), (Shape::Grab, Shape::Grabbing), (Shape::ResizeEw, Shape::ResizeNs), (Shape::Text, Shape::Hidden), (Shape::Hidden, Shape::Pointer)] {
            let mut m = morph_from(a, b, &p, t0);
            assert!(m.running(), "{a:?} -> {b:?}");
            let mid = at(&mut m, b, &p, t0 + std::time::Duration::from_millis(60));
            assert!(m.running());
            assert_ne!(mid, render_with(b, 32, &p, [0.1, 0.5, 1.0], 0.0), "{a:?} -> {b:?} shows something in between");
            let end = at(&mut m, b, &p, t0 + std::time::Duration::from_millis(400));
            assert!(!m.running());
            assert_eq!(end, render_with(b, 32, &p, [0.1, 0.5, 1.0], 0.0), "{a:?} -> {b:?}");
        }
        // hiding: the old picture shrinks away to nothing
        let mut m = morph_from(Shape::Default, Shape::Hidden, &p, t0);
        let full = coverage(&render_with(Shape::Default, 32, &p, [0.1, 0.5, 1.0], 0.0));
        let mid = coverage(&at(&mut m, Shape::Hidden, &p, t0 + std::time::Duration::from_millis(50)));
        assert!(mid > 0 && mid < full, "{mid} of {full}");
    }

    #[test]
    fn morph_starts_from_what_is_shown() {
        let p = CursorPrefs::default();
        let t0 = std::time::Instant::now();
        // cross-fade: the first picture is the old one (on a larger canvas, the hot spot where it was)
        let mut m = morph_from(Shape::Default, Shape::Text, &p, t0);
        let first = at(&mut m, Shape::Text, &p, t0);
        assert!(same_on_screen(&first, &render_with(Shape::Default, 32, &p, [0.1, 0.5, 1.0], 0.0)));
        // geometric: the first pose is the old shape
        let mut m = morph_from(Shape::Grab, Shape::Grabbing, &p, t0);
        let first = at(&mut m, Shape::Grabbing, &p, t0);
        assert!(same_on_screen(&first, &render_with(Shape::Grab, 32, &p, [0.1, 0.5, 1.0], 0.0)));
    }

    #[test]
    fn a_change_mid_way_carries_on_without_a_pop() {
        let p = CursorPrefs::default();
        let t0 = std::time::Instant::now();
        let mid = t0 + std::time::Duration::from_millis(70);
        // geometric: on from the in-between angle
        let mut m = morph_from(Shape::ResizeEw, Shape::ResizeNs, &p, t0);
        let before = at(&mut m, Shape::ResizeNs, &p, mid);
        m.retarget(Shape::ResizeNesw, true, &p, mid);
        assert!(matches!(m.run.as_ref().map(|r| &r.start), Some(Start::Pose(..))));
        let after = at(&mut m, Shape::ResizeNesw, &p, mid);
        assert_eq!(before, after);
        // cross-fade: on from the picture on screen, and again (A -> B -> C -> D)
        let mut m = morph_from(Shape::Default, Shape::Text, &p, t0);
        let mut t = mid;
        let mut shape = Shape::Text;
        for next in [Shape::Pointer, Shape::NotAllowed, Shape::Crosshair] {
            let before = at(&mut m, shape, &p, t);
            m.retarget(next, true, &p, t);
            let after = at(&mut m, next, &p, t);
            assert!(same_on_screen(&before, &after), "-> {next:?}");
            shape = next;
            t += std::time::Duration::from_millis(40);
        }
        let end = at(&mut m, shape, &p, t + std::time::Duration::from_millis(400));
        assert_eq!(end, render_with(shape, 32, &p, [0.1, 0.5, 1.0], 0.0));
    }

    #[test]
    fn morph_respects_the_preferences_and_the_kind_of_picture() {
        let t0 = std::time::Instant::now();
        // off: the new picture at once
        let off = CursorPrefs { morph: false, ..CursorPrefs::default() };
        let mut m = morph_from(Shape::Default, Shape::Text, &off, t0);
        assert!(!m.running());
        assert_eq!(at(&mut m, Shape::Text, &off, t0), render_with(Shape::Text, 32, &off, [0.1, 0.5, 1.0], 0.0));
        // the same drawing under another name: nothing to animate
        let arrow = CursorPrefs { hand: HandStyle::Arrow, ..CursorPrefs::default() };
        assert!(!morph_from(Shape::Default, Shape::Pointer, &arrow, t0).running());
        // a picture with more than the drawing (a program's own, a launch icon beside it): cross-fade only
        let p = CursorPrefs::default();
        let mut m = Morph::default();
        m.retarget(Shape::Default, true, &p, t0);
        m.frame(t0, &p, 32, [0.0; 3], 0.0, || render_with(Shape::Default, 32, &p, [0.0; 3], 0.0));
        m.retarget(Shape::Progress, false, &p, t0);
        assert!(matches!(m.run.as_ref().map(|r| &r.start), Some(Start::Picture(_))));
        // and back from it, too
        m.retarget(Shape::Default, true, &p, t0 + std::time::Duration::from_millis(500));
        assert!(matches!(m.run.as_ref().map(|r| &r.start), Some(Start::Picture(_))));
        // a slower pace takes longer
        let slow = CursorPrefs { morph_speed: 0.5, ..CursorPrefs::default() };
        let mut m = morph_from(Shape::Default, Shape::Text, &slow, t0);
        at(&mut m, Shape::Text, &slow, t0 + std::time::Duration::from_millis(250));
        assert!(m.running());
    }

    /// `CURSOR_MORPH=/dir cargo test cursor_morph_sheet -- --ignored`: film strips of shape changes (one row each, hot spot
    /// on the same point in every cell, 3x), to look at.
    #[test]
    #[ignore]
    fn cursor_morph_sheet() {
        let Ok(dir) = std::env::var("CURSOR_MORPH") else { return };
        let p = CursorPrefs::default();
        let pairs = [(Shape::Default, Shape::Text), (Shape::Default, Shape::Pointer), (Shape::Pointer, Shape::Grab), (Shape::Grab, Shape::Grabbing), (Shape::ResizeEw, Shape::ResizeNs), (Shape::ResizeNs, Shape::ResizeNwse), (Shape::Default, Shape::Progress), (Shape::Default, Shape::Copy), (Shape::Text, Shape::Hidden), (Shape::Default, Shape::ResizeNwse)];
        let steps = 12u32;
        let (cell, mag) = (72u32, 3u32);
        let mut sheet = image::RgbaImage::from_pixel(cell * (steps + 1) * mag, cell * pairs.len() as u32 * mag, image::Rgba([120, 130, 150, 255]));
        let t0 = std::time::Instant::now();
        for (row, (a, b)) in pairs.iter().enumerate() {
            let mut m = Morph::default();
            m.retarget(*a, true, &p, t0);
            m.frame(t0, &p, 40, [0.1, 0.5, 1.0], 0.3, || render_with(*a, 40, &p, [0.1, 0.5, 1.0], 0.3));
            m.retarget(*b, true, &p, t0);
            for i in 0..=steps {
                let t = t0 + std::time::Duration::from_secs_f32(MORPH_SECS * i as f32 / steps as f32);
                let pic = m.frame(t, &p, 40, [0.1, 0.5, 1.0], 0.3, || render_with(*b, 40, &p, [0.1, 0.5, 1.0], 0.3)).clone();
                let (ox, oy) = (i * cell + cell / 2, row as u32 * cell + cell / 2);
                for y in 0..pic.size as i32 {
                    for x in 0..pic.size as i32 {
                        let px = pic.argb[(y * pic.size as i32 + x) as usize];
                        let al = (px >> 24) as f32 / 255.0;
                        let (cx, cy) = (ox as i32 + x - pic.hot.0, oy as i32 + y - pic.hot.1);
                        if al <= 0.0 || cx < (i * cell) as i32 || cy < (row as u32 * cell) as i32 || cx >= ((i + 1) * cell) as i32 || cy >= ((row as u32 + 1) * cell) as i32 {
                            continue;
                        }
                        for my in 0..mag {
                            for mx in 0..mag {
                                let (sx, sy) = (cx as u32 * mag + mx, cy as u32 * mag + my);
                                let old = sheet.get_pixel(sx, sy).0;
                                let mix = |sh: u32, o: u8| (((px >> sh) & 255) as f32 + o as f32 * (1.0 - al)) as u8;
                                sheet.put_pixel(sx, sy, image::Rgba([mix(16, old[0]), mix(8, old[1]), mix(0, old[2]), 255]));
                            }
                        }
                    }
                }
                // the pointer's point: a red dot
                sheet.put_pixel(ox * mag + 1, oy * mag + 1, image::Rgba([255, 0, 0, 255]));
            }
        }
        sheet.save(format!("{dir}/morph.png")).unwrap();
    }
}
