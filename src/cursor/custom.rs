//! The cursors built with sliders: the "Custom" choice of each shape in Settings > Cursor.
//!
//! A shape has a few numbers (its [`Gen`]) and the list of sliders over them ([`Gen::DIALS`]); the pieces it is drawn
//! from are generated from those numbers alone. Every generator starts exactly on the built-in shapes, which are points
//! in its space: `Gen::from_preset` is how a preset is written in those numbers.
//!
//! In the config these live in a table of their own (`[input.cursor.custom.arrow]`...) next to the style names, not as a
//! new style name: an older build doesn't know the table, skips it and shows the style the shape was started from. A new
//! name would have made it throw the whole config away.
//!
//! A generated shape may reach out of the 32 x 32 design box; `cursor::paint` fits it into the picture (see `Fit`).

use super::{arc, arrow_points, hourglass_sized, line, rrect, ArrowStyle, BeamStyle, BusyStyle, CursorPrefs, Hand, HandStyle, Piece, ResizeStyle, Shape};
use resvg::tiny_skia::{Path, PathBuilder, Transform};
use serde::{Deserialize, Serialize};

/// One slider: its label, the number it moves, its range, and whether it only takes whole numbers.
pub struct Dial<T> {
    pub label: &'static str,
    pub get: fn(&mut T) -> &mut f32,
    pub lo: f32,
    pub hi: f32,
    pub whole: bool,
}

/// A shape that is built from sliders.
pub trait Gen: Clone + PartialEq + Default + 'static {
    const DIALS: &'static [Dial<Self>];
    /// What can be started from: the name, and the preset's choice written into the preferences.
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))];

    /// A choice beside the sliders that the preferences keep: its name, whether it is on, and turning it.
    const SWITCH: Option<(&'static str, fn(&CursorPrefs) -> bool, fn(&mut CursorPrefs, bool))> = None;

    /// What choosing Custom changes besides turning it on.
    fn chosen(_prefs: &mut CursorPrefs) {}

    /// The preset chosen in the preferences, as this shape's numbers.
    fn from_preset(prefs: &CursorPrefs) -> Self;

    /// Every number inside its slider's range (a config written by hand can hold anything).
    fn clamped(&self) -> Self {
        let mut out = self.clone();
        let mut plain = Self::default();
        for d in Self::DIALS {
            let fallback = *(d.get)(&mut plain);
            let v = (d.get)(&mut out);
            *v = if v.is_finite() { v.clamp(d.lo, d.hi) } else { fallback };
            if d.whole {
                *v = v.round();
            }
        }
        out
    }

    /// A shape near one of the presets, so that a throw of the dice still looks like a cursor.
    fn random(prefs: &CursorPrefs, seed: u64) -> Self {
        let mut rng = seed | 1;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 40) as f32 / (1u32 << 24) as f32
        };
        let mut base = prefs.clone();
        // (the first few numbers of a small seed are small)
        for _ in 0..4 {
            next();
        }
        if !Self::STARTS.is_empty() {
            Self::STARTS[((next() * Self::STARTS.len() as f32) as usize).min(Self::STARTS.len() - 1)].1(&mut base);
        }
        let mut out = Self::from_preset(&base);
        for d in Self::DIALS {
            *(d.get)(&mut out) += (next() * 2.0 - 1.0) * 0.17 * (d.hi - d.lo);
        }
        out.clamped()
    }
}

/// A shape's Custom choice: whether it is chosen, the sliders, and the sliders as they were before it last started over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Made<T> {
    pub on: bool,
    pub shape: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<T>,
}

impl<T: Gen> Made<T> {
    /// Custom is chosen while the preferences show a preset: it starts as that preset. What was built before is kept.
    pub fn choose(&mut self, prefs: &CursorPrefs) {
        self.start(T::from_preset(prefs));
        self.on = true;
    }

    /// Start over from `shape`; sliders that aren't simply a preset's are kept as `last`.
    pub fn start(&mut self, shape: T) {
        if self.shape != shape && !Self::is_preset(&self.shape) {
            self.last = Some(self.shape.clone());
        }
        self.shape = shape;
    }

    fn is_preset(shape: &T) -> bool {
        *shape == T::default()
            || T::STARTS.iter().any(|(_, set)| {
                let mut p = CursorPrefs::default();
                set(&mut p);
                *shape == T::from_preset(&p)
            })
    }
}

/// (a shape nothing was built for is left out of the config)
fn untouched<T: Default + PartialEq>(made: &Made<T>) -> bool {
    !made.on && made.last.is_none() && made.shape == T::default()
}

/// The custom choice of every shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomShapes {
    #[serde(skip_serializing_if = "untouched")]
    pub arrow: Made<ArrowGen>,
    /// the pointing, open and closed hand
    #[serde(skip_serializing_if = "untouched")]
    pub hand: Made<HandGen>,
    #[serde(skip_serializing_if = "untouched")]
    pub beam: Made<BeamGen>,
    /// the resize arrows and the four-way move arrow
    #[serde(skip_serializing_if = "untouched")]
    pub resize: Made<ResizeGen>,
    /// the busy ring or hourglass (which of the two stays `CursorPrefs::busy`)
    #[serde(skip_serializing_if = "untouched")]
    pub busy: Made<BusyGen>,
    /// the crosshair and the not-allowed sign
    #[serde(skip_serializing_if = "untouched")]
    pub signs: Made<SignGen>,
}

impl CustomShapes {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

type P = (f32, f32);

fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}

fn mul(a: P, k: f32) -> P {
    (a.0 * k, a.1 * k)
}

fn len(a: P) -> f32 {
    a.0.hypot(a.1)
}

fn cross(a: P, b: P) -> f32 {
    a.0 * b.1 - a.1 * b.0
}

fn mix(a: P, b: P, t: f32) -> P {
    add(a, mul(sub(b, a), t))
}

/// The room a shape needs whatever its phase or angle is: a circle that isn't drawn.
fn room(cx: f32, cy: f32, r: f32) -> Option<Piece> {
    PathBuilder::from_circle(cx, cy, r.max(0.5)).map(Piece::Room)
}

// ---- the arrow

/// The arrow: two wings from the tip, the notch between their ends, and a tail out of the notch. The numbers are those
/// of a seven-cornered polygon seen along its own axis, so any of the built-in arrows is exactly one setting of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrowGen {
    /// the way the axis goes from the tip, degrees clockwise from the right (90: the tip points straight up)
    pub rotation: f32,
    /// the mean length of the two wings
    pub length: f32,
    /// the angle between them
    pub width: f32,
    /// the wings' difference in length, as a share of `length`
    pub lean: f32,
    /// the sides bulging out (or in)
    pub curve: f32,
    /// how far from the tip the notch is, 1 = level with the ends of the wings (more: a kite)
    pub notch: f32,
    pub notch_shift: f32,
    pub notch_slant: f32,
    pub neck: f32,
    pub tail: f32,
    pub tail_width: f32,
    pub tail_angle: f32,
    pub tail_cut: f32,
    /// how round the corners are (the tip stays a point: it is where a click goes)
    pub round: f32,
}

impl Default for ArrowGen {
    fn default() -> Self {
        ArrowGen::of(ArrowStyle::Classic)
    }
}

impl Gen for ArrowGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Length", get: |s| &mut s.length, lo: 8.0, hi: 30.0, whole: false },
        Dial { label: "Width", get: |s| &mut s.width, lo: 20.0, hi: 110.0, whole: false },
        Dial { label: "Rotation", get: |s| &mut s.rotation, lo: 0.0, hi: 180.0, whole: false },
        Dial { label: "Lean", get: |s| &mut s.lean, lo: -0.3, hi: 0.3, whole: false },
        Dial { label: "Curved sides", get: |s| &mut s.curve, lo: -0.15, hi: 0.2, whole: false },
        Dial { label: "Roundness", get: |s| &mut s.round, lo: 0.0, hi: 4.0, whole: false },
        Dial { label: "Notch depth", get: |s| &mut s.notch, lo: 0.3, hi: 1.3, whole: false },
        Dial { label: "Notch shift", get: |s| &mut s.notch_shift, lo: -0.9, hi: 0.9, whole: false },
        Dial { label: "Notch slant", get: |s| &mut s.notch_slant, lo: -35.0, hi: 35.0, whole: false },
        Dial { label: "Neck width", get: |s| &mut s.neck, lo: 0.0, hi: 14.0, whole: false },
        Dial { label: "Tail length", get: |s| &mut s.tail, lo: 0.0, hi: 16.0, whole: false },
        Dial { label: "Tail width", get: |s| &mut s.tail_width, lo: 0.0, hi: 14.0, whole: false },
        Dial { label: "Tail angle", get: |s| &mut s.tail_angle, lo: -50.0, hi: 50.0, whole: false },
        Dial { label: "Tail cut", get: |s| &mut s.tail_cut, lo: -40.0, hi: 40.0, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[
        ("Classic", |p| p.arrow = ArrowStyle::Classic),
        ("Slim", |p| p.arrow = ArrowStyle::Slim),
        ("Triangle", |p| p.arrow = ArrowStyle::Triangle),
        ("Bold", |p| p.arrow = ArrowStyle::Bold),
        ("Navigator", |p| p.arrow = ArrowStyle::Navigator),
        ("Sharp", |p| p.arrow = ArrowStyle::Sharp),
    ];

    fn from_preset(prefs: &CursorPrefs) -> Self {
        ArrowGen::of(prefs.arrow)
    }
}

impl ArrowGen {
    /// A built-in arrow written in these numbers (the inverse of `corners`).
    pub fn of(style: ArrowStyle) -> ArrowGen {
        let p: Vec<P> = arrow_points(style, 1.0, 0.0, 0.0).into_iter().map(|q| sub(q, (2.0, 2.0))).collect();
        let seven = p.len() == 7;
        // (the navigator goes round the other way)
        let turned = !seven && cross(p[3], p[1]) < 0.0;
        let (l, nl, tl, tr, nr, r) = if seven {
            (p[1], p[2], p[3], p[4], p[5], p[6])
        } else if turned {
            (p[3], p[2], p[2], p[2], p[2], p[1])
        } else {
            (p[1], p[2], p[2], p[2], p[2], p[3])
        };
        let (a, b) = (l.1.atan2(l.0), r.1.atan2(r.0));
        let (axis, half) = ((a + b) / 2.0, (a - b) / 2.0);
        let u = (axis.cos(), axis.sin());
        let local = |q: P| (q.0 * u.0 + q.1 * u.1, cross(u, q));
        let wings = (len(l) + len(r)) / 2.0;
        let (neck, end) = (local(mul(sub(nl, nr), 0.5)), local(mul(sub(tl, tr), 0.5)));
        let middle = local(mul(add(nl, nr), 0.5));
        let tail = sub(local(mul(add(tl, tr), 0.5)), middle);
        let angle = |v: P| if seven { v.0.atan2(v.1).to_degrees() } else { 0.0 };
        ArrowGen {
            rotation: axis.to_degrees(),
            length: wings,
            width: half.to_degrees() * 2.0,
            lean: (len(r) - len(l)) / (2.0 * wings),
            curve: 0.0,
            notch: middle.0 / (wings * half.cos()),
            notch_shift: middle.1 / (middle.0 * half.tan()),
            notch_slant: angle(neck),
            neck: len(neck) * 2.0,
            tail: if seven { len(tail) } else { 0.0 },
            tail_width: len(end) * 2.0,
            tail_angle: if seven { tail.1.atan2(tail.0).to_degrees() } else { 0.0 },
            tail_cut: angle(end),
            round: 0.0,
        }
    }

    /// The corners from the tip (0, 0), round the outline: tip, wing, neck, the tail's two, neck, wing. `keep` (0 .. 1)
    /// is how much of the tail is left; at 0 the outline is seen whole from the tip, so it cannot cross itself.
    fn corners(&self, keep: f32) -> Vec<P> {
        let half = self.width.to_radians() / 2.0;
        let (c, s, tan) = (half.cos(), half.sin(), half.tan());
        let (left, right) = (self.length * (1.0 - self.lean), self.length * (1.0 + self.lean));
        let depth = self.notch * self.length * c;
        let middle = (depth, self.notch_shift * depth * tan);
        // the neck stays inside the wedge of the two wings, in the order the outline takes
        let slant = self.notch_slant.to_radians();
        let free = 0.96 * depth * tan - middle.1.abs();
        let n = (self.neck / 2.0).min(free.max(0.0) / (slant.cos() + 0.96 * tan * slant.sin().abs()));
        let neck = (n * slant.sin(), n * slant.cos());
        let (nl, nr) = (add(middle, neck), sub(middle, neck));
        let mut v = vec![(0.0, 0.0), (left * c, left * s), nl];
        if self.tail * keep > 0.01 {
            let bend = self.tail_angle.to_radians();
            let end = add(middle, mul((bend.cos(), bend.sin()), self.tail * keep));
            let cut = self.tail_cut.to_radians();
            // (a tail that is only starting has the neck's width)
            let w = mix(neck, mul((cut.sin(), cut.cos()), self.tail_width / 2.0), keep * (self.tail / 1.5).min(1.0));
            v.extend([add(end, w), sub(end, w)]);
        }
        v.extend([nr, (right * c, -right * s)]);
        let axis = self.rotation.to_radians();
        let (u, w) = ((axis.cos(), axis.sin()), (-axis.sin(), axis.cos()));
        let mut out: Vec<P> = v.into_iter().map(|q| add(mul(u, q.0), mul(w, q.1))).collect();
        out.dedup_by(|a, b| len(sub(*a, *b)) < 0.01);
        out
    }

    /// The outline from the tip: segments, each a straight one or a quadratic curve's control point and end.
    fn segments(&self, keep: f32) -> Vec<(Option<P>, P)> {
        let v = self.corners(keep);
        let n = v.len();
        let half = self.width.to_radians() / 2.0;
        let axis = self.rotation.to_radians();
        let bulge = self.curve * keep;
        // the control points of the two long sides: out of the middle, square to the side
        let side = |end: P, angle: f32| -> Option<P> { (bulge.abs() > 1e-4).then(|| add(mul(end, 0.5), mul((angle.cos(), angle.sin()), 2.0 * bulge * len(end)))) };
        let first = side(v[1], axis + half + std::f32::consts::FRAC_PI_2);
        let last = side(v[n - 1], axis - half - std::f32::consts::FRAC_PI_2);
        let ctrl = |edge: usize| if edge == 0 { first } else if edge == n - 1 { last } else { None };
        let mut out = Vec::with_capacity(2 * n);
        for i in 1..n {
            let (p, prev, next) = (v[i], ctrl(i - 1).unwrap_or(v[i - 1]), ctrl(i).unwrap_or(v[(i + 1) % n]));
            let (a, b) = (len(sub(prev, p)), len(sub(next, p)));
            let r = (self.round * keep).min(0.45 * a).min(0.45 * b);
            if r > 0.01 {
                out.push((ctrl(i - 1), mix(p, prev, r / a)));
                out.push((Some(p), mix(p, next, r / b)));
            } else {
                out.push((ctrl(i - 1), p));
            }
        }
        out.push((ctrl(n - 1), v[0]));
        out
    }

    /// The outline from the tip at (0, 0), with nothing crossing: the tail, bulge and rounding give way where they would.
    fn outline(&self) -> Vec<(Option<P>, P)> {
        let s = self.clamped();
        let mut keep = 1.0f32;
        loop {
            let seg = s.segments(keep);
            if keep <= 0.0 || simple(&flatten(&seg)) {
                return seg;
            }
            keep = if keep > 0.05 { keep - 0.05 } else { 0.0 };
        }
    }

    /// The arrow `k` times as big with its tip at `tip`.
    pub(super) fn path(&self, k: f32, tip: P) -> Option<Path> {
        let at = |q: P| add(tip, mul(q, k));
        let mut b = PathBuilder::new();
        b.move_to(tip.0, tip.1);
        let seg = self.outline();
        for (i, (c, to)) in seg.iter().enumerate() {
            let to = at(*to);
            match c.map(at) {
                Some(c) => b.quad_to(c.0, c.1, to.0, to.1),
                // (`close` draws the straight way home)
                None if i + 1 == seg.len() => {}
                None => b.line_to(to.0, to.1),
            }
        }
        b.close();
        b.finish()
    }
}

fn flatten(seg: &[(Option<P>, P)]) -> Vec<P> {
    let mut at = (0.0, 0.0);
    let mut out = vec![at];
    for (c, to) in seg {
        match c {
            Some(c) => out.extend((1..=6).map(|i| i as f32 / 6.0).map(|t| mix(mix(at, *c, t), mix(*c, *to, t), t))),
            None => out.push(*to),
        }
        at = *to;
    }
    out.pop();
    out
}

/// No two sides of the closed polygon cross.
fn simple(p: &[P]) -> bool {
    let n = p.len();
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (c, d) = (p[j], p[(j + 1) % n]);
            let (e, f) = (cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(d, a)));
            let (g, h) = (cross(sub(d, c), sub(a, c)), cross(sub(d, c), sub(b, c)));
            let apart = |x: f32, y: f32| (x > 1e-4 && y < -1e-4) || (x < -1e-4 && y > 1e-4);
            if apart(e, f) && apart(g, h) {
                return false;
            }
        }
    }
    true
}

// ---- the hands

/// The hands, as factors of the built-in ones: the three of them change together, and blend into each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HandGen {
    pub fingers: f32,
    pub finger_length: f32,
    pub index: f32,
    pub spread: f32,
    pub arch: f32,
    pub palm: f32,
    pub palm_height: f32,
    pub thumb: f32,
    pub thumb_length: f32,
    pub thumb_out: f32,
    pub thumb_down: f32,
    pub round: f32,
    pub tilt: f32,
    pub cuff: f32,
}

impl Default for HandGen {
    fn default() -> Self {
        HandGen { fingers: 1.0, finger_length: 1.0, index: 1.0, spread: 1.0, arch: 0.0, palm: 1.0, palm_height: 1.0, thumb: 1.0, thumb_length: 1.0, thumb_out: 0.0, thumb_down: 0.0, round: 1.0, tilt: 0.0, cuff: 0.0 }
    }
}

impl Gen for HandGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Finger width", get: |s| &mut s.fingers, lo: 0.6, hi: 1.2, whole: false },
        Dial { label: "Finger length", get: |s| &mut s.finger_length, lo: 0.6, hi: 1.35, whole: false },
        Dial { label: "Index finger", get: |s| &mut s.index, lo: 0.75, hi: 1.4, whole: false },
        Dial { label: "Spread", get: |s| &mut s.spread, lo: 0.7, hi: 1.15, whole: false },
        Dial { label: "Arch", get: |s| &mut s.arch, lo: -2.0, hi: 2.0, whole: false },
        Dial { label: "Palm width", get: |s| &mut s.palm, lo: 0.75, hi: 1.25, whole: false },
        Dial { label: "Palm height", get: |s| &mut s.palm_height, lo: 0.7, hi: 1.3, whole: false },
        Dial { label: "Thumb width", get: |s| &mut s.thumb, lo: 0.5, hi: 1.4, whole: false },
        Dial { label: "Thumb length", get: |s| &mut s.thumb_length, lo: 0.6, hi: 1.4, whole: false },
        Dial { label: "Thumb out", get: |s| &mut s.thumb_out, lo: -3.0, hi: 1.2, whole: false },
        Dial { label: "Thumb down", get: |s| &mut s.thumb_down, lo: -3.0, hi: 3.0, whole: false },
        Dial { label: "Roundness", get: |s| &mut s.round, lo: 0.0, hi: 1.5, whole: false },
        Dial { label: "Tilt", get: |s| &mut s.tilt, lo: -45.0, hi: 45.0, whole: false },
        Dial { label: "Cuff", get: |s| &mut s.cuff, lo: 0.0, hi: 6.0, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[("Hand", |p| p.hand = HandStyle::Hand)];

    // (the hands are still there to drag with when links have the arrow, so they can be built with that on)
    const SWITCH: Option<(&'static str, fn(&CursorPrefs) -> bool, fn(&mut CursorPrefs, bool))> =
        Some(("Arrow over links", |p| p.hand == HandStyle::Arrow, |p, on| p.hand = if on { HandStyle::Arrow } else { HandStyle::Hand }));

    /// Custom over links is a hand that is built: the arrow is one switch away.
    fn chosen(prefs: &mut CursorPrefs) {
        prefs.hand = HandStyle::Hand;
    }

    fn from_preset(_: &CursorPrefs) -> Self {
        HandGen::default()
    }
}

impl HandGen {
    /// One of the built-in hands as these numbers make it (still upright: see `finish`). `pointing`: the index finger is
    /// the pointer's point.
    pub(super) fn hand(&self, base: &Hand, pointing: bool) -> Hand {
        let g = self.clamped();
        let (src, hot) = base;
        let mut out = *src;
        let [px, py, pw, ph, pr] = src[4];
        let (w, h) = (pw * g.palm, ph * g.palm_height);
        let middle = px + pw / 2.0;
        let left = middle - w / 2.0;
        out[4] = [left, py, w, h, pr * g.round];
        for i in 0..4 {
            let [x, y, fw, fh, r] = src[i];
            let fw2 = fw * g.fingers;
            // (a finger stays on the palm, and the built-in hands reach its edge exactly)
            let cx = middle + (x + fw / 2.0 - middle) * g.spread * g.palm;
            let cx = cx.min(left + w - fw2 / 2.0 + 0.2).max(left + fw2 / 2.0 - 0.2);
            let lead = i == 0 && pointing;
            let len = fh * g.finger_length * if lead { g.index } else { 1.0 };
            let bottom = y + fh;
            let lift = if i == 1 || i == 2 { g.arch } else { 0.0 };
            let top = (bottom - len - lift).min(bottom - 1.0);
            out[i] = [cx - fw2 / 2.0, top, fw2, bottom - top, r * g.round];
        }
        let [x, y, tw, th, tr] = src[5];
        let (tw2, th2) = (tw * g.thumb, th * g.thumb_length);
        let edge = left + (x + tw - px) - g.thumb_out;
        out[5] = [edge - tw2, y + th - th2 + g.thumb_down, tw2, th2, tr * g.round];
        // the fingertip: as far across the finger as the built-in hand has it (all but its middle)
        let hot = if pointing { (out[0][0] + out[0][2] * (hot.0 - src[0][0]) / src[0][2], out[0][1]) } else { *hot };
        (out, hot)
    }

    /// The pieces of a hand (or of one on its way to another), with the cuff, tilted.
    pub(super) fn finish(&self, boxes: &[[f32; 5]; 6], hot: P) -> (Vec<Piece>, P) {
        let g = self.clamped();
        let mut v = super::hand_pieces(boxes);
        let [x, y, w, h, _] = boxes[4];
        if g.cuff >= 0.5 {
            v.extend(rrect(x + 2.0, y + h - 0.2, w - 4.0, g.cuff + 0.2, 1.5).map(Piece::Fill));
        }
        if g.tilt == 0.0 {
            return (v, hot);
        }
        let tf = Transform::from_rotate_at(g.tilt, 16.0, 16.0);
        let (sin, cos) = g.tilt.to_radians().sin_cos();
        let (dx, dy) = (hot.0 - 16.0, hot.1 - 16.0);
        (v.into_iter().filter_map(|p| p.moved(tf)).collect(), (16.0 + dx * cos - dy * sin, 16.0 + dx * sin + dy * cos))
    }
}

// ---- over text

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BeamGen {
    /// between the two bars
    pub height: f32,
    pub stem: f32,
    pub thickness: f32,
    pub bar: f32,
    pub bar_thickness: f32,
    /// the bars bending outwards at their ends, as on a Mac
    pub bend: f32,
    pub tick: f32,
    pub gap: f32,
    pub slant: f32,
}

impl Default for BeamGen {
    fn default() -> Self {
        BeamGen { height: 20.0, stem: 18.0, thickness: 2.2, bar: 8.0, bar_thickness: 2.2, bend: 0.0, tick: 0.0, gap: 0.0, slant: 0.0 }
    }
}

impl Gen for BeamGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Height", get: |s| &mut s.height, lo: 10.0, hi: 28.0, whole: false },
        Dial { label: "Line length", get: |s| &mut s.stem, lo: 6.0, hi: 28.0, whole: false },
        Dial { label: "Thickness", get: |s| &mut s.thickness, lo: 0.8, hi: 6.0, whole: false },
        Dial { label: "Bar width", get: |s| &mut s.bar, lo: 0.0, hi: 16.0, whole: false },
        Dial { label: "Bar thickness", get: |s| &mut s.bar_thickness, lo: 0.8, hi: 5.0, whole: false },
        Dial { label: "Bar bend", get: |s| &mut s.bend, lo: -3.0, hi: 3.0, whole: false },
        Dial { label: "Middle tick", get: |s| &mut s.tick, lo: 0.0, hi: 12.0, whole: false },
        Dial { label: "Middle gap", get: |s| &mut s.gap, lo: 0.0, hi: 8.0, whole: false },
        Dial { label: "Slant", get: |s| &mut s.slant, lo: -30.0, hi: 30.0, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[("Classic", |p| p.beam = BeamStyle::Classic), ("Plain", |p| p.beam = BeamStyle::Plain), ("Bold", |p| p.beam = BeamStyle::Bold)];

    fn from_preset(prefs: &CursorPrefs) -> Self {
        match prefs.beam {
            BeamStyle::Classic => BeamGen::default(),
            BeamStyle::Plain => BeamGen { stem: 20.0, thickness: 2.4, bar: 0.0, ..BeamGen::default() },
            BeamStyle::Bold => BeamGen { height: 22.0, stem: 19.0, thickness: 3.4, bar: 11.0, bar_thickness: 3.0, ..BeamGen::default() },
        }
    }
}

impl BeamGen {
    fn pieces(&self) -> Vec<Piece> {
        let g = self.clamped();
        let (hh, stem, bar, c) = (g.height / 2.0, g.stem / 2.0, g.bar / 2.0, 16.0);
        let mut v = Vec::new();
        let gap = (g.gap / 2.0).min(stem - 1.0);
        if gap > 0.05 {
            v.extend(line((c, c - stem), (c, c - gap)).map(|p| Piece::Line(p, g.thickness)));
            v.extend(line((c, c + gap), (c, c + stem)).map(|p| Piece::Line(p, g.thickness)));
        } else {
            v.extend(line((c, c - stem), (c, c + stem)).map(|p| Piece::Line(p, g.thickness)));
        }
        if bar > 0.2 {
            for side in [-1.0f32, 1.0] {
                let y = c + side * hh;
                let path = if g.bend.abs() < 0.05 {
                    line((c - bar, y), (c + bar, y))
                } else {
                    let out = y + side * g.bend;
                    let mut b = PathBuilder::new();
                    b.move_to(c - bar, out);
                    b.quad_to(c, out, c, y);
                    b.quad_to(c, out, c + bar, out);
                    b.finish()
                };
                v.extend(path.map(|p| Piece::Line(p, g.bar_thickness)));
            }
        }
        if g.tick > 0.4 {
            v.extend(line((c - g.tick / 2.0, c), (c + g.tick / 2.0, c)).map(|p| Piece::Line(p, g.bar_thickness)));
        }
        if g.slant == 0.0 {
            return v;
        }
        let tf = Transform::from_rotate_at(g.slant, c, c);
        v.into_iter().filter_map(|p| p.moved(tf)).collect()
    }
}

// ---- over edges

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResizeGen {
    pub length: f32,
    pub head: f32,
    pub head_width: f32,
    /// the back of a head pulled in (or pushed out), as a share of its length
    pub barb: f32,
    pub shaft: f32,
    pub dot: f32,
    pub round: f32,
}

impl Default for ResizeGen {
    fn default() -> Self {
        ResizeGen { length: 27.0, head: 7.0, head_width: 12.0, barb: 0.0, shaft: 2.4, dot: 0.0, round: 0.0 }
    }
}

impl Gen for ResizeGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Length", get: |s| &mut s.length, lo: 12.0, hi: 30.0, whole: false },
        Dial { label: "Head length", get: |s| &mut s.head, lo: 3.0, hi: 11.0, whole: false },
        Dial { label: "Head width", get: |s| &mut s.head_width, lo: 4.0, hi: 16.0, whole: false },
        Dial { label: "Head notch", get: |s| &mut s.barb, lo: -0.5, hi: 0.6, whole: false },
        Dial { label: "Roundness", get: |s| &mut s.round, lo: 0.0, hi: 2.5, whole: false },
        Dial { label: "Line thickness", get: |s| &mut s.shaft, lo: 0.0, hi: 6.0, whole: false },
        Dial { label: "Dot size", get: |s| &mut s.dot, lo: 0.0, hi: 6.0, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[("Line", |p| p.resize = ResizeStyle::Line), ("Dot", |p| p.resize = ResizeStyle::Dot)];

    fn from_preset(prefs: &CursorPrefs) -> Self {
        match prefs.resize {
            ResizeStyle::Line => ResizeGen::default(),
            ResizeStyle::Dot => ResizeGen { head: 5.5, head_width: 9.0, shaft: 0.0, dot: 3.0, ..ResizeGen::default() },
        }
    }
}

impl ResizeGen {
    /// Two heads along x, or four for the move arrow.
    fn pieces(&self, four: bool) -> Vec<Piece> {
        let g = self.clamped();
        let mut span = g.length / 2.0;
        let (mut head, mut wide) = (g.head.min(span - 1.5), g.head_width / 2.0);
        if four {
            // four of them in the same box: heads that would run into their neighbours get smaller
            let k = ((span - 1.0) / (head + wide)).min(1.0);
            head *= k;
            wide *= k;
            // (the built-in move arrow is a little wider than the resize one)
            span += 0.5;
        }
        let dirs: &[P] = if four { &[(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] } else { &[(-1.0, 0.0), (1.0, 0.0)] };
        let mut v = Vec::new();
        let reach = span - head + 1.5;
        if g.shaft >= 0.3 {
            v.extend(line((16.0 - reach, 16.0), (16.0 + reach, 16.0)).map(|p| Piece::Line(p, g.shaft)));
            if four {
                v.extend(line((16.0, 16.0 - reach), (16.0, 16.0 + reach)).map(|p| Piece::Line(p, g.shaft)));
            }
        }
        let mut radius = span;
        for d in dirs {
            let across = (d.1, -d.0);
            let (tip, back) = (add((16.0, 16.0), mul(*d, span)), add((16.0, 16.0), mul(*d, span - head)));
            let (a, b) = (add(back, mul(across, wide)), sub(back, mul(across, wide)));
            let mut pts = vec![tip, a];
            if g.barb.abs() > 1e-3 {
                pts.push(add(back, mul(*d, g.barb * head)));
            }
            pts.push(b);
            for p in &pts {
                radius = radius.max(len(sub(*p, (16.0, 16.0))));
            }
            v.extend(rounded(&pts, g.round).map(Piece::Fill));
        }
        if g.dot >= 0.3 {
            v.extend(PathBuilder::from_circle(16.0, 16.0, g.dot).map(Piece::Accent));
        }
        // the same room at every angle it is turned to
        let shaft = if g.shaft >= 0.3 { reach + g.shaft / 2.0 } else { 0.0 };
        v.extend(room(16.0, 16.0, radius.max(shaft).max(g.dot)));
        v
    }
}

/// A polygon with rounded corners (the first, a tip, stays sharp).
fn rounded(p: &[P], r: f32) -> Option<Path> {
    let n = p.len();
    let mut b = PathBuilder::new();
    b.move_to(p[0].0, p[0].1);
    for i in 1..n {
        let (at, prev, next) = (p[i], p[i - 1], p[(i + 1) % n]);
        let (la, lb) = (len(sub(prev, at)), len(sub(next, at)));
        let r = r.min(0.45 * la).min(0.45 * lb);
        if r > 0.01 {
            let (a, z) = (mix(at, prev, r / la), mix(at, next, r / lb));
            b.line_to(a.0, a.1);
            b.quad_to(at.0, at.1, z.0, z.1);
        } else {
            b.line_to(at.0, at.1);
        }
    }
    b.close();
    b.finish()
}

// ---- while loading

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BusyGen {
    pub radius: f32,
    pub thickness: f32,
    pub sweep: f32,
    pub parts: f32,
    pub dot: f32,
    pub glass_width: f32,
    pub glass_height: f32,
    pub glass_line: f32,
}

impl Default for BusyGen {
    fn default() -> Self {
        BusyGen { radius: 9.0, thickness: 3.2, sweep: 270.0, parts: 1.0, dot: 0.0, glass_width: 1.0, glass_height: 1.0, glass_line: 1.0 }
    }
}

impl Gen for BusyGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Radius", get: |s| &mut s.radius, lo: 5.0, hi: 13.0, whole: false },
        Dial { label: "Thickness", get: |s| &mut s.thickness, lo: 1.0, hi: 6.0, whole: false },
        Dial { label: "Arc", get: |s| &mut s.sweep, lo: 6.0, hi: 340.0, whole: false },
        Dial { label: "Parts", get: |s| &mut s.parts, lo: 1.0, hi: 8.0, whole: true },
        Dial { label: "Middle dot", get: |s| &mut s.dot, lo: 0.0, hi: 4.0, whole: false },
        Dial { label: "Glass width", get: |s| &mut s.glass_width, lo: 0.5, hi: 1.4, whole: false },
        Dial { label: "Glass height", get: |s| &mut s.glass_height, lo: 0.6, hi: 1.3, whole: false },
        Dial { label: "Glass line", get: |s| &mut s.glass_line, lo: 0.5, hi: 1.8, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[("Ring", |p| p.busy = BusyStyle::Ring), ("Hourglass", |p| p.busy = BusyStyle::Hourglass), ("Rainbow hourglass", |p| p.busy = BusyStyle::Rainbow)];

    fn from_preset(_: &CursorPrefs) -> Self {
        BusyGen::default()
    }
}

impl BusyGen {
    /// The sliders that mean something for the style it is built on (the first five are the ring's).
    pub fn dials_for(style: BusyStyle) -> &'static [Dial<Self>] {
        if style == BusyStyle::Ring { &Self::DIALS[..5] } else { &Self::DIALS[5..] }
    }

    fn pieces(&self, at: P, k: f32, thick: f32, phase: f32, style: BusyStyle) -> Vec<Piece> {
        let g = self.clamped();
        let (cx, cy) = at;
        let mut v = Vec::new();
        if style == BusyStyle::Ring {
            let (r, w, parts) = (g.radius * k, g.thickness * thick, g.parts as usize);
            let step = 360.0 / parts as f32;
            let mut each = g.sweep / parts as f32;
            if parts > 1 {
                // round ends and the outline grow into the gap: it stays open however many parts there are
                each = each.min(step - ((g.thickness + 3.6) / g.radius).to_degrees()).max(2.0);
            }
            for i in 0..parts {
                let from = phase * 360.0 + i as f32 * step;
                v.extend(arc(cx, cy, r, from, each).map(|p| Piece::Line(p, w)));
            }
            if g.dot >= 0.3 {
                v.extend(PathBuilder::from_circle(cx, cy, (g.dot * k).max(0.2)).map(Piece::Fill));
            }
            v.extend(room(cx, cy, r + w / 2.0));
        } else {
            let shape = [g.glass_width, g.glass_height, g.glass_line];
            v = hourglass_sized(cx, cy, k, phase, shape);
            v.extend(room(cx, cy, (9.0 * shape[0]).hypot(11.5 * shape[1]) * k + 1.1 * shape[2]));
        }
        v
    }

    /// How far the badge beside the arrow reaches from its middle, at full size.
    fn badge_reach(&self, style: BusyStyle) -> f32 {
        let g = self.clamped();
        if style == BusyStyle::Ring { g.radius * RING_BADGE + g.thickness * RING_BADGE_LINE / 2.0 } else { 0.62 * (9.0 * g.glass_width + 1.1 * g.glass_line) }
    }
}

/// The ring beside the arrow next to the big one: its radius, and its thickness.
const RING_BADGE: f32 = 5.2 / 9.0;
const RING_BADGE_LINE: f32 = 2.6 / 3.2;

// ---- the signs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignGen {
    pub size: f32,
    pub gap: f32,
    pub thickness: f32,
    pub dot: f32,
    pub ring: f32,
    pub no_size: f32,
    pub no_thickness: f32,
}

impl Default for SignGen {
    fn default() -> Self {
        SignGen { size: 12.0, gap: 3.0, thickness: 2.2, dot: 0.0, ring: 0.0, no_size: 10.0, no_thickness: 3.0 }
    }
}

impl Gen for SignGen {
    const DIALS: &'static [Dial<Self>] = &[
        Dial { label: "Cross size", get: |s| &mut s.size, lo: 4.0, hi: 15.0, whole: false },
        Dial { label: "Cross gap", get: |s| &mut s.gap, lo: 0.0, hi: 9.0, whole: false },
        Dial { label: "Cross thickness", get: |s| &mut s.thickness, lo: 0.8, hi: 5.0, whole: false },
        Dial { label: "Cross dot", get: |s| &mut s.dot, lo: 0.0, hi: 4.0, whole: false },
        Dial { label: "Cross ring", get: |s| &mut s.ring, lo: 0.0, hi: 12.0, whole: false },
        Dial { label: "Forbidden size", get: |s| &mut s.no_size, lo: 5.0, hi: 14.0, whole: false },
        Dial { label: "Forbidden thickness", get: |s| &mut s.no_thickness, lo: 1.0, hi: 6.0, whole: false },
    ];
    const STARTS: &'static [(&'static str, fn(&mut CursorPrefs))] = &[("Standard", |_| {})];

    fn from_preset(_: &CursorPrefs) -> Self {
        SignGen::default()
    }
}

impl SignGen {
    fn crosshair(&self) -> Vec<Piece> {
        let g = self.clamped();
        let (c, far, near) = (16.0, g.size, g.gap.min(g.size - 1.0));
        let mut v = Vec::new();
        for (a, b) in [((c, c - far), (c, c - near)), ((c, c + near), (c, c + far)), ((c - far, c), (c - near, c)), ((c + near, c), (c + far, c))] {
            v.extend(line(a, b).map(|p| Piece::Line(p, g.thickness)));
        }
        if g.dot >= 0.3 {
            v.extend(PathBuilder::from_circle(c, c, g.dot).map(Piece::Fill));
        }
        if g.ring >= 1.0 {
            v.extend(arc(c, c, g.ring, 0.0, 359.9).map(|p| Piece::Line(p, g.thickness)));
        }
        v
    }

    fn forbidden(&self) -> Vec<Piece> {
        let g = self.clamped();
        let (c, d) = (16.0, 0.7 * g.no_size);
        let mut v: Vec<Piece> = arc(c, c, g.no_size, 0.0, 359.9).map(|p| Piece::Line(p, g.no_thickness)).into_iter().collect();
        v.extend(line((c - d, c - d), (c + d, c + d)).map(|p| Piece::Line(p, g.no_thickness)));
        v
    }
}

// ---- what the cursor module asks

/// Is the hand of `shape` the plain arrow (over links, set to "Arrow")?
fn links_arrow(shape: Shape, prefs: &CursorPrefs) -> bool {
    shape == Shape::Pointer && prefs.hand == HandStyle::Arrow
}

/// Does `shape` have anything built with the sliders (and is it then fitted into its picture)?
pub fn is_custom(shape: Shape, prefs: &CursorPrefs) -> bool {
    let c = &prefs.custom;
    match shape {
        Shape::Default | Shape::Hidden | Shape::Client | Shape::Copy => c.arrow.on,
        Shape::Pointer if links_arrow(shape, prefs) => c.arrow.on,
        Shape::Pointer | Shape::Grab | Shape::Grabbing => c.hand.on,
        Shape::Text => c.beam.on,
        Shape::Move | Shape::ResizeEw | Shape::ResizeNs | Shape::ResizeNesw | Shape::ResizeNwse => c.resize.on,
        Shape::Wait => c.busy.on,
        Shape::Progress => c.arrow.on || c.busy.on,
        Shape::Crosshair | Shape::NotAllowed => c.signs.on,
    }
}

/// The pieces of a custom shape, the pointer's point and the angle to turn them by (see `cursor::pieces`).
pub(super) fn pieces(shape: Shape, phase: f32, prefs: &CursorPrefs) -> (Vec<Piece>, P, f32) {
    let (c, middle) = (&prefs.custom, (16.0, 16.0));
    let arrow = |pose: [f32; 3], plus: f32, busy: f32| {
        let (v, hot) = arrow_family(pose, plus, busy, phase, prefs);
        (v, hot, 0.0)
    };
    match shape {
        Shape::Default | Shape::Hidden | Shape::Client => arrow([1.0, 0.0, 0.0], 0.0, 0.0),
        Shape::Pointer if links_arrow(shape, prefs) => arrow([1.0, 0.0, 0.0], 0.0, 0.0),
        Shape::Copy => arrow([0.85, 1.0, 1.0], 1.0, 0.0),
        Shape::Progress => arrow([0.8, 1.0, 1.0], 0.0, 1.0),
        Shape::Pointer | Shape::Grab | Shape::Grabbing => {
            let base = match shape {
                Shape::Pointer => &super::HAND_POINTER,
                Shape::Grab => &super::HAND_GRAB,
                _ => &super::HAND_GRABBING,
            };
            let g = &c.hand.shape;
            let (boxes, hot) = g.hand(base, shape == Shape::Pointer);
            let (v, hot) = g.finish(&boxes, hot);
            (v, hot, 0.0)
        }
        Shape::Text => (c.beam.shape.pieces(), middle, 0.0),
        Shape::Move => (c.resize.shape.pieces(true), middle, 0.0),
        Shape::ResizeEw => (c.resize.shape.pieces(false), middle, 0.0),
        Shape::ResizeNs => (c.resize.shape.pieces(false), middle, 90.0),
        Shape::ResizeNesw => (c.resize.shape.pieces(false), middle, -45.0),
        Shape::ResizeNwse => (c.resize.shape.pieces(false), middle, 45.0),
        Shape::Wait => (c.busy.shape.pieces(middle, 1.0, 1.0, phase, prefs.busy), middle, 0.0),
        Shape::Crosshair => (c.signs.shape.crosshair(), middle, 0.0),
        Shape::NotAllowed => (c.signs.shape.forbidden(), middle, 0.0),
    }
}

/// The arrow with how much of the "+" and of the busy badge shows, when the arrow or the badge is custom. A badge an
/// arrow would cover steps aside.
pub(super) fn arrow_family([k, dx, dy]: [f32; 3], plus: f32, busy: f32, phase: f32, prefs: &CursorPrefs) -> (Vec<Piece>, P) {
    let c = &prefs.custom;
    // the tip itself, also for an arrow that is made smaller beside a badge
    let tip = (dx + 2.0 * k, dy + 2.0 * k);
    let path = if c.arrow.on { c.arrow.shape.path(k, tip) } else { super::poly(&arrow_points(prefs.arrow, k, dx, dy)) };
    let bounds = path.as_ref().map(|p| p.bounds());
    let aside = |cx: f32, cy: f32, reach: f32, shown: f32| -> P {
        let Some(b) = bounds.filter(|_| shown > 0.12) else { return (cx, cy) };
        let (over_x, over_y) = (b.right() + 1.0 - (cx - reach), b.bottom() + 1.0 - (cy - reach));
        if over_x <= 0.05 || over_y <= 0.05 {
            (cx, cy)
        } else if over_x <= over_y {
            (cx + over_x * shown.min(1.0), cy)
        } else {
            (cx, cy + over_y * shown.min(1.0))
        }
    };
    let mut v: Vec<Piece> = path.clone().map(Piece::Fill).into_iter().collect();
    let plus = plus.clamp(0.0, 1.2);
    if plus > 0.12 {
        let r = 5.0 * plus;
        let (x, y) = aside(24.0, 24.0, 6.3, plus);
        v.extend(line((x, y - r), (x, y + r)).map(|p| Piece::Line(p, 2.6)));
        v.extend(line((x - r, y), (x + r, y)).map(|p| Piece::Line(p, 2.6)));
    }
    let busy = busy.clamp(0.0, 1.2);
    if busy > 0.12 {
        let g = if c.busy.on { c.busy.shape.clone() } else { BusyGen::default() };
        let ring = prefs.busy == BusyStyle::Ring;
        let (x, y) = if ring { (23.5, 23.5) } else { (23.5, 22.5) };
        let at = aside(x, y, g.badge_reach(prefs.busy), busy);
        let (k, thick) = if ring { (RING_BADGE * busy, RING_BADGE_LINE * busy.min(1.0)) } else { (0.62 * busy, 1.0) };
        v.extend(g.pieces(at, k, thick, phase, prefs.busy));
    }
    (v, tip)
}

#[cfg(test)]
mod tests;
