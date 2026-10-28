//! The document of the icon composer: a background and a tree of layers and groups, each with its colour, blending,
//! Liquid Glass and placement settings, in the six looks of an icon (default, dark, clear and tinted, each light or dark). It is
//! saved in the `.memicon` format, a small binary container of our own:
//!
//! ```text
//!   "MEMICON5"            8 bytes   magic and format version (versions 1 to 4 are still read)
//!   flags                 1 byte    bit 0: the payload is deflated
//!   raw length            u32 LE    size of the payload before deflating
//!   payload               ...       the document, written field by field (see `Writer`)
//!   crc32                 u32 LE    of the raw payload
//! ```
//!
//! Numbers are little-endian, strings are a `u32` length and UTF-8 bytes, a list is a `u32` count and its items.

use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression, Crc};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"MEMICON5";
const MAGIC_V4: &[u8; 8] = b"MEMICON4";
const MAGIC_V3: &[u8; 8] = b"MEMICON3";
const MAGIC_V2: &[u8; 8] = b"MEMICON2";
const MAGIC_V1: &[u8; 8] = b"MEMICON1";

macro_rules! coded_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $code:expr, $label:expr;)+ }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn code(self) -> u8 { match self { $($name::$var => $code),+ } }
            pub fn from_code(c: u8) -> Option<$name> { match c { $($code => Some($name::$var),)+ _ => None } }
            pub fn label(self) -> &'static str { match self { $($name::$var => $label),+ } }
        }
    };
}

coded_enum!(Blend {
    Normal = 0, "Normal";
    Multiply = 1, "Multiply";
    Screen = 2, "Screen";
    Overlay = 3, "Overlay";
    Darken = 4, "Darken";
    Lighten = 5, "Lighten";
    Plus = 6, "Plus";
});

coded_enum!(ShadowKind {
    None = 0, "None";
    Neutral = 1, "Neutral";
    Chromatic = 2, "Chromatic";
});

coded_enum!(
    /// Whether a group's layers are glass each on their own, or the group is one piece of glass.
    GlassMode {
        Individual = 0, "Individual";
        Combined = 1, "Combined";
    }
);

coded_enum!(Shape {
    None = 0, "None";
    Circle = 1, "Circle";
    Squircle = 2, "Squircle";
    Rounded = 3, "Rounded";
    Super = 4, "Super";
});

coded_enum!(FillKind {
    None = 0, "None";
    Solid = 1, "Solid";
    Gradient = 2, "Gradient";
    // what the system makes of it, as in Apple's icons: a light plate, a dark one in the dark looks
    System = 3, "System";
    // one colour, lighter at the top and deeper at the bottom
    Auto = 4, "Auto";
});

/// Where a dragged node is dropped, relative to another node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    /// right in front of the node (above it in the list), in its group
    InFront(u32),
    /// right behind the node (below it in the list), in its group
    Behind(u32),
    /// into the group, in front of what is there
    Into(u32),
    /// a new group made of the node and the dragged one
    Pair(u32),
}

/// The looks of an icon: the usual one and the dark one, and the "clear" (frosted glass) and "tinted" (one colour) looks
/// that the system makes of every icon, each for a light and for a dark theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Default,
    Dark,
    ClearLight,
    ClearDark,
    TintedLight,
    TintedDark,
}

impl Appearance {
    pub const ALL: [Appearance; 6] = [Appearance::Default, Appearance::Dark, Appearance::ClearLight, Appearance::ClearDark, Appearance::TintedLight, Appearance::TintedDark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::Default => "Default",
            Appearance::Dark => "Dark",
            Appearance::ClearLight => "Clear (light)",
            Appearance::ClearDark => "Clear (dark)",
            Appearance::TintedLight => "Tinted (light)",
            Appearance::TintedDark => "Tinted (dark)",
        }
    }

    /// Which of the five variant slots of a setting this look has; the default look has none.
    pub fn slot(self) -> Option<usize> {
        match self {
            Appearance::Default => None,
            Appearance::Dark => Some(0),
            Appearance::ClearLight => Some(1),
            Appearance::ClearDark => Some(2),
            Appearance::TintedLight => Some(3),
            Appearance::TintedDark => Some(4),
        }
    }

    /// The looks whose settings make this one, from the nearest the default: a look inherits from its parent. Dark follows
    /// the default look; Clear (dark) follows Clear (light), which follows the default look; Tinted the same.
    pub fn chain(self) -> &'static [Appearance] {
        match self {
            Appearance::Default => &[],
            Appearance::Dark => &[Appearance::Dark],
            Appearance::ClearLight => &[Appearance::ClearLight],
            Appearance::ClearDark => &[Appearance::ClearLight, Appearance::ClearDark],
            Appearance::TintedLight => &[Appearance::TintedLight],
            Appearance::TintedDark => &[Appearance::TintedLight, Appearance::TintedDark],
        }
    }

    pub fn is_clear(self) -> bool {
        matches!(self, Appearance::ClearLight | Appearance::ClearDark)
    }

    pub fn is_tinted(self) -> bool {
        matches!(self, Appearance::TintedLight | Appearance::TintedDark)
    }

    /// For a dark theme?
    pub fn is_dark(self) -> bool {
        matches!(self, Appearance::Dark | Appearance::ClearDark | Appearance::TintedDark)
    }

    /// The look as (style, dark): style 0 is the usual look, 1 clear, 2 tinted.
    pub fn parts(self) -> (usize, bool) {
        (if self.is_clear() { 1 } else if self.is_tinted() { 2 } else { 0 }, self.is_dark())
    }

    pub fn from_parts(style: usize, dark: bool) -> Appearance {
        match (style, dark) {
            (1, false) => Appearance::ClearLight,
            (1, true) => Appearance::ClearDark,
            (2, false) => Appearance::TintedLight,
            (2, true) => Appearance::TintedDark,
            (_, false) => Appearance::Default,
            (_, true) => Appearance::Dark,
        }
    }
}

/// A value that looks can override in part: a patch holds only the fields that were changed.
pub trait Patchable: Copy + PartialEq {
    type Patch: Copy + PartialEq + Default;
    fn apply(&self, patch: &Self::Patch) -> Self;
    /// The patch that makes `base` into `new`.
    fn diff(base: &Self, new: &Self) -> Self::Patch;
}

/// A setting that differs between the looks. The default look holds the value; every other look holds only what it
/// changed, and the rest it inherits from its parent look (see `Appearance::chain`), so a change in the default look reaches
/// every look that did not set that field itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Variants<T: Patchable> {
    pub default: T,
    /// dark, clear (light), clear (dark), tinted (light), tinted (dark)
    pub slots: [T::Patch; 5],
}

impl<T: Patchable> Variants<T> {
    pub fn new(default: T) -> Variants<T> {
        Variants { default, slots: Default::default() }
    }

    /// The value in look `a`. `auto` makes the starting point of a look from the default value (the clear and tinted looks
    /// start as glass, the dark one as a darker plate); what the look and its parents set is put on top.
    pub fn resolve(&self, a: Appearance, auto: &dyn Fn(Appearance, &T) -> T) -> T {
        let mut v = if a == Appearance::Default { self.default } else { auto(a, &self.default) };
        for l in a.chain() {
            v = v.apply(&self.slots[l.slot().expect("not the default look")]);
        }
        v
    }

    /// The value in look `a` without any automatic starting point.
    pub fn get(&self, a: Appearance) -> T {
        self.resolve(a, &|_, d| *d)
    }

    /// Make look `a` have the value `new`: only what differs from what it inherits is kept in the look.
    pub fn set(&mut self, a: Appearance, new: T, auto: &dyn Fn(Appearance, &T) -> T) {
        match a.slot() {
            None => self.default = new,
            Some(i) => {
                let mut inherited = auto(a, &self.default);
                for l in a.chain() {
                    if *l != a {
                        inherited = inherited.apply(&self.slots[l.slot().expect("not the default look")]);
                    }
                }
                self.slots[i] = T::diff(&inherited, &new);
            }
        }
    }

    /// Does look `a` change something itself (always true for the default)?
    pub fn is_set(&self, a: Appearance) -> bool {
        a.slot().map_or(true, |i| self.slots[i] != T::Patch::default())
    }

    /// Make `a` inherit everything again.
    pub fn clear(&mut self, a: Appearance) {
        if let Some(i) = a.slot() {
            self.slots[i] = T::Patch::default();
        }
    }
}

fn pick<T: PartialEq + Copy>(base: T, new: T) -> Option<T> {
    (base != new).then_some(new)
}

/// The glass settings a look changed.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GlassPatch {
    pub enabled: Option<bool>,
    pub specular: Option<f32>,
    pub blur: Option<f32>,
    pub refraction: Option<f32>,
    pub translucency: Option<f32>,
    pub shadow: Option<ShadowKind>,
    pub shadow_amount: Option<f32>,
    pub bevel: Option<f32>,
    pub bevel_width: Option<f32>,
    pub inner_shadow: Option<f32>,
    pub glow: Option<f32>,
}

impl Patchable for Glass {
    type Patch = GlassPatch;
    fn apply(&self, p: &GlassPatch) -> Glass {
        Glass {
            enabled: p.enabled.unwrap_or(self.enabled),
            specular: p.specular.unwrap_or(self.specular),
            blur: p.blur.unwrap_or(self.blur),
            refraction: p.refraction.unwrap_or(self.refraction),
            translucency: p.translucency.unwrap_or(self.translucency),
            shadow: p.shadow.unwrap_or(self.shadow),
            shadow_amount: p.shadow_amount.unwrap_or(self.shadow_amount),
            bevel: p.bevel.unwrap_or(self.bevel),
            bevel_width: p.bevel_width.unwrap_or(self.bevel_width),
            inner_shadow: p.inner_shadow.unwrap_or(self.inner_shadow),
            glow: p.glow.unwrap_or(self.glow),
        }
    }
    fn diff(b: &Glass, n: &Glass) -> GlassPatch {
        GlassPatch {
            enabled: pick(b.enabled, n.enabled),
            specular: pick(b.specular, n.specular),
            blur: pick(b.blur, n.blur),
            refraction: pick(b.refraction, n.refraction),
            translucency: pick(b.translucency, n.translucency),
            shadow: pick(b.shadow, n.shadow),
            shadow_amount: pick(b.shadow_amount, n.shadow_amount),
            bevel: pick(b.bevel, n.bevel),
            bevel_width: pick(b.bevel_width, n.bevel_width),
            inner_shadow: pick(b.inner_shadow, n.inner_shadow),
            glow: pick(b.glow, n.glow),
        }
    }
}

/// The settings of a layer or group that a look changed.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PropsPatch {
    pub visible: Option<bool>,
    pub opacity: Option<f32>,
    pub blend: Option<Blend>,
    pub fill: Option<Option<[f32; 3]>>,
    pub gradient: Option<Option<Gradient>>,
    pub offset: Option<[f32; 2]>,
    pub scale: Option<f32>,
    pub glass: GlassPatch,
    pub glass_mode: Option<GlassMode>,
}

impl Patchable for Props {
    type Patch = PropsPatch;
    fn apply(&self, p: &PropsPatch) -> Props {
        Props {
            visible: p.visible.unwrap_or(self.visible),
            opacity: p.opacity.unwrap_or(self.opacity),
            blend: p.blend.unwrap_or(self.blend),
            fill: p.fill.unwrap_or(self.fill),
            gradient: p.gradient.unwrap_or(self.gradient),
            offset: p.offset.unwrap_or(self.offset),
            scale: p.scale.unwrap_or(self.scale),
            glass: self.glass.apply(&p.glass),
            glass_mode: p.glass_mode.unwrap_or(self.glass_mode),
        }
    }
    fn diff(b: &Props, n: &Props) -> PropsPatch {
        PropsPatch {
            visible: pick(b.visible, n.visible),
            opacity: pick(b.opacity, n.opacity),
            blend: pick(b.blend, n.blend),
            fill: pick(b.fill, n.fill),
            gradient: pick(b.gradient, n.gradient),
            offset: pick(b.offset, n.offset),
            scale: pick(b.scale, n.scale),
            glass: Glass::diff(&b.glass, &n.glass),
            glass_mode: pick(b.glass_mode, n.glass_mode),
        }
    }
}

/// The plate settings that a look changed.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BackgroundPatch {
    pub shape: Option<Shape>,
    pub fill: Option<FillKind>,
    pub top: Option<[f32; 3]>,
    pub bottom: Option<[f32; 3]>,
    pub glass: GlassPatch,
}

impl Patchable for Background {
    type Patch = BackgroundPatch;
    fn apply(&self, p: &BackgroundPatch) -> Background {
        Background { shape: p.shape.unwrap_or(self.shape), fill: p.fill.unwrap_or(self.fill), top: p.top.unwrap_or(self.top), bottom: p.bottom.unwrap_or(self.bottom), glass: self.glass.apply(&p.glass) }
    }
    fn diff(b: &Background, n: &Background) -> BackgroundPatch {
        BackgroundPatch { shape: pick(b.shape, n.shape), fill: pick(b.fill, n.fill), top: pick(b.top, n.top), bottom: pick(b.bottom, n.bottom), glass: Glass::diff(&b.glass, &n.glass) }
    }
}

/// Liquid Glass: how a layer treats the light and what is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub enabled: bool,
    /// light caught by the edges, 0 .. 1
    pub specular: f32,
    /// frosting of what is behind, 0 .. 1
    pub blur: f32,
    /// bending of what is behind at the edges, 0 .. 1
    pub refraction: f32,
    /// how much the layer lets through of what is behind, 0 .. 1
    pub translucency: f32,
    pub shadow: ShadowKind,
    pub shadow_amount: f32,
    /// the light and shade of a rounded edge (an embossed look), 0 .. 1
    pub bevel: f32,
    /// how wide the rounded edge is, 0 .. 1
    pub bevel_width: f32,
    /// a shade inside along the edge, as of a hollow, 0 .. 1
    pub inner_shadow: f32,
    /// the layer's own colour glowing round it, 0 .. 1
    pub glow: f32,
}

impl Default for Glass {
    fn default() -> Glass {
        Glass { enabled: false, specular: 0.6, blur: 0.0, refraction: 0.5, translucency: 0.0, shadow: ShadowKind::Neutral, shadow_amount: 0.3, bevel: 0.0, bevel_width: 0.5, inner_shadow: 0.0, glow: 0.0 }
    }
}

/// Most blobs a gradient can have.
pub const MAX_BLOBS: usize = 6;

/// One soft spot of colour: where its middle is (as fractions of the shape's box), how far it reaches (a fraction of the
/// box's longer side) and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    pub at: [f32; 2],
    pub size: f32,
    pub colour: [f32; 3],
}

/// A gradient made of blobs: spots of colour fading into each other over a base colour (the `a` of the gradient).
/// `count` spots of `spots` are in use; none means a plain gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blobs {
    pub count: u8,
    pub spots: [Blob; MAX_BLOBS],
}

impl Default for Blobs {
    fn default() -> Blobs {
        Blobs { count: 0, spots: [Blob { at: [0.5, 0.5], size: 0.6, colour: [1.0; 3] }; MAX_BLOBS] }
    }
}

impl Blobs {
    /// Three blobs spread over the box: a start for a fresh blob gradient.
    pub fn starter(a: [f32; 3], b: [f32; 3]) -> Blobs {
        let mut s = Blobs::default();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
        s.spots[0] = Blob { at: [0.2, 0.15], size: 0.7, colour: b };
        s.spots[1] = Blob { at: [0.85, 0.35], size: 0.6, colour: [mid[2], mid[0], mid[1]] };
        s.spots[2] = Blob { at: [0.4, 0.95], size: 0.65, colour: [1.0 - b[0] * 0.5, 1.0 - b[1] * 0.4, 1.0 - b[2] * 0.2] };
        s.count = 3;
        s
    }

    pub fn used(&self) -> &[Blob] {
        &self.spots[..(self.count as usize).min(MAX_BLOBS)]
    }

    /// Add a blob in the emptiest looking place; false when full.
    pub fn add(&mut self) -> bool {
        let n = self.count as usize;
        if n >= MAX_BLOBS {
            return false;
        }
        let mut best = ([0.5, 0.5], -1.0f32);
        for gy in 0..5 {
            for gx in 0..5 {
                let p = [0.1 + gx as f32 * 0.2, 0.1 + gy as f32 * 0.2];
                let d = self.used().iter().map(|b| (b.at[0] - p[0]).hypot(b.at[1] - p[1])).fold(f32::MAX, f32::min);
                if d > best.1 {
                    best = (p, d);
                }
            }
        }
        let c = if n > 0 { self.spots[n - 1].colour } else { [1.0; 3] };
        self.spots[n] = Blob { at: best.0, size: 0.55, colour: [c[1], c[2], c[0]] };
        self.count += 1;
        true
    }

    pub fn remove(&mut self, i: usize) {
        let n = self.count as usize;
        if i < n {
            self.spots.copy_within(i + 1..n, i);
            self.count -= 1;
        }
    }

    /// The colour at (`x`, `y`) of the box (0 to 1) over `base`: each blob covers what is below it by a soft falloff.
    pub fn colour_at(&self, base: [f32; 3], x: f32, y: f32) -> [f32; 3] {
        let mut c = base;
        for b in self.used() {
            let d = (x - b.at[0]).hypot(y - b.at[1]) / b.size.max(0.02);
            if d < 1.0 {
                let t = 1.0 - d;
                let a = t * t * (3.0 - 2.0 * t);
                for k in 0..3 {
                    c[k] += (b.colour[k] - c[k]) * a;
                }
            }
        }
        c
    }
}

/// A gradient the layer is painted with: from colour `a` to `b`, along the angle (0 is from the top to the bottom, degrees,
/// clockwise) or, when `radial`, from the middle out; or, with `blobs`, spots of colour over `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub angle: f32,
    pub radial: bool,
    pub blobs: Blobs,
}

impl Default for Gradient {
    fn default() -> Gradient {
        Gradient { a: [1.0, 0.85, 0.3], b: [0.95, 0.35, 0.15], angle: 0.0, radial: false, blobs: Blobs::default() }
    }
}

/// The settings of a layer or a group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Props {
    pub visible: bool,
    pub opacity: f32,
    pub blend: Blend,
    /// the whole layer painted in this one colour instead of its own
    pub fill: Option<[f32; 3]>,
    /// the layer painted with a gradient (over `fill`)
    pub gradient: Option<Gradient>,
    /// shift as a fraction of the icon, and size factor
    pub offset: [f32; 2],
    pub scale: f32,
    pub glass: Glass,
    /// for a group
    pub glass_mode: GlassMode,
}

impl Default for Props {
    fn default() -> Props {
        Props { visible: true, opacity: 1.0, blend: Blend::Normal, fill: None, gradient: None, offset: [0.0, 0.0], scale: 1.0, glass: Glass::default(), glass_mode: GlassMode::Individual }
    }
}

/// The plate the layers sit on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub shape: Shape,
    pub fill: FillKind,
    pub top: [f32; 3],
    pub bottom: [f32; 3],
    pub glass: Glass,
}

impl Default for Background {
    fn default() -> Background {
        Background { shape: Shape::None, fill: FillKind::None, top: [0.96, 0.96, 0.98], bottom: [0.86, 0.87, 0.92], glass: Glass { enabled: false, specular: 0.4, shadow: ShadowKind::Neutral, shadow_amount: 0.35, ..Glass::default() } }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// a whole SVG document with this one shape in it (so a layer needs nothing else)
    pub svg: String,
    pub props: Variants<Props>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub children: Vec<Node>,
    pub props: Variants<Props>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Layer(Layer),
    Group(Group),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
}

impl Node {
    pub fn props(&self) -> &Variants<Props> {
        match &self.kind {
            Kind::Layer(l) => &l.props,
            Kind::Group(g) => &g.props,
        }
    }

    pub fn props_mut(&mut self) -> &mut Variants<Props> {
        match &mut self.kind {
            Kind::Layer(l) => &mut l.props,
            Kind::Group(g) => &mut g.props,
        }
    }

    pub fn children(&self) -> &[Node] {
        match &self.kind {
            Kind::Group(g) => &g.children,
            Kind::Layer(_) => &[],
        }
    }
}

/// The whole document. Layers are stored back to front (the last one is on top).
#[derive(Clone, Debug, PartialEq)]
pub struct IconDoc {
    /// the name of the icon it is made for (`firefox`)
    pub name: String,
    pub background: Variants<Background>,
    pub nodes: Vec<Node>,
    /// the empty border around the icon as a fraction, room for shadows
    pub margin: f32,
    /// the colour of the "tinted by icon" look, when it is set by hand (None: found from the colours of the layers)
    pub icon_tint: Option<[f32; 3]>,
    pub next_id: u32,
}

/// What the file holds in the place of the icon's tint when there is none (files from before it was kept held this).
const NO_ICON_TINT: [f32; 3] = [0.42, 0.34, 0.80];

impl IconDoc {
    pub fn new(name: &str) -> IconDoc {
        IconDoc { name: name.to_string(), background: Variants::new(Background::default()), nodes: Vec::new(), margin: 0.05, icon_tint: None, next_id: 1 }
    }

    pub fn fresh_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn find(&self, id: u32) -> Option<&Node> {
        fn walk(nodes: &[Node], id: u32) -> Option<&Node> {
            nodes.iter().find_map(|n| if n.id == id { Some(n) } else { walk(n.children(), id) })
        }
        walk(&self.nodes, id)
    }

    pub fn find_mut(&mut self, id: u32) -> Option<&mut Node> {
        fn walk(nodes: &mut [Node], id: u32) -> Option<&mut Node> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Kind::Group(g) = &mut n.kind {
                    if let Some(f) = walk(&mut g.children, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    /// The list that holds node `id` and its place in it.
    fn locate(&mut self, id: u32) -> Option<(&mut Vec<Node>, usize)> {
        fn walk(nodes: &mut Vec<Node>, id: u32) -> Option<(&mut Vec<Node>, usize)> {
            if let Some(i) = nodes.iter().position(|n| n.id == id) {
                return Some((nodes, i));
            }
            for n in nodes.iter_mut() {
                if let Kind::Group(g) = &mut n.kind {
                    if let Some(found) = walk(&mut g.children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    /// Remove node `id` (with what is in it).
    pub fn remove(&mut self, id: u32) -> Option<Node> {
        let (list, i) = self.locate(id)?;
        Some(list.remove(i))
    }

    /// Move node `id` one step towards the front (`up`) or the back, within its list.
    pub fn shift(&mut self, id: u32, up: bool) -> bool {
        let Some((list, i)) = self.locate(id) else { return false };
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if j >= list.len() {
            return false;
        }
        list.swap(i, j);
        true
    }

    /// Put node `id` in a new group of its own, in its place.
    pub fn group(&mut self, id: u32) -> Option<u32> {
        let gid = self.fresh_id();
        let (list, i) = self.locate(id)?;
        let node = list.remove(i);
        let name = format!("Group {gid}");
        list.insert(i, Node { id: gid, name, kind: Kind::Group(Group { children: vec![node], props: Variants::new(Props::default()) }) });
        Some(gid)
    }

    /// Is node `inner` the node `outer` or somewhere inside it?
    pub fn is_within(&self, outer: u32, inner: u32) -> bool {
        self.find(outer).is_some_and(|n| IconDoc::contains(n, inner))
    }

    fn contains(node: &Node, id: u32) -> bool {
        node.id == id || node.children().iter().any(|c| IconDoc::contains(c, id))
    }

    /// Move node `id` (with what is in it) to `to`. False, and nothing changes, if that is not possible (onto itself, or
    /// into something that is inside it).
    pub fn move_node(&mut self, id: u32, to: Drop) -> bool {
        let target = match to {
            Drop::InFront(t) | Drop::Behind(t) | Drop::Into(t) | Drop::Pair(t) => t,
        };
        let Some(moving) = self.find(id) else { return false };
        if IconDoc::contains(moving, target) || self.find(target).is_none() {
            return false;
        }
        if matches!(to, Drop::Into(t) if !matches!(self.find(t).map(|n| &n.kind), Some(Kind::Group(_)))) {
            return false;
        }
        let Some(node) = self.remove(id) else { return false };
        let gid = if matches!(to, Drop::Pair(_)) { Some(self.fresh_id()) } else { None };
        match to {
            Drop::InFront(t) | Drop::Behind(t) => {
                let (list, i) = self.locate(t).expect("checked above");
                list.insert(if matches!(to, Drop::InFront(_)) { i + 1 } else { i }, node);
            }
            Drop::Into(t) => {
                if let Some(Node { kind: Kind::Group(g), .. }) = self.find_mut(t) {
                    g.children.push(node);
                }
            }
            Drop::Pair(t) => {
                let (list, i) = self.locate(t).expect("checked above");
                let under = list.remove(i);
                let gid = gid.expect("made for a pair");
                list.insert(i, Node { id: gid, name: format!("Group {gid}"), kind: Kind::Group(Group { children: vec![under, node], props: Variants::new(Props::default()) }) });
            }
        }
        self.drop_empty_groups();
        true
    }

    /// A group that lost all its layers is not kept.
    fn drop_empty_groups(&mut self) {
        fn walk(nodes: &mut Vec<Node>) {
            for n in nodes.iter_mut() {
                if let Kind::Group(g) = &mut n.kind {
                    walk(&mut g.children);
                }
            }
            nodes.retain(|n| !matches!(&n.kind, Kind::Group(g) if g.children.is_empty()));
        }
        walk(&mut self.nodes);
    }

    /// Put a copy of node `id` (with new ids) right in front of it. Returns the id of the copy.
    pub fn duplicate(&mut self, id: u32) -> Option<u32> {
        fn renumber(n: &mut Node, next: &mut u32) {
            n.id = *next;
            *next += 1;
            if let Kind::Group(g) = &mut n.kind {
                for c in &mut g.children {
                    renumber(c, next);
                }
            }
        }
        let mut copy = self.find(id)?.clone();
        let mut next = self.next_id;
        renumber(&mut copy, &mut next);
        self.next_id = next;
        copy.name = format!("{} copy", copy.name);
        let new_id = copy.id;
        let (list, i) = self.locate(id)?;
        list.insert(i + 1, copy);
        Some(new_id)
    }

    /// Replace group `id` by its children, in its place. Returns false if `id` is not a group.
    pub fn ungroup(&mut self, id: u32) -> bool {
        let Some((list, i)) = self.locate(id) else { return false };
        match list.remove(i) {
            Node { kind: Kind::Group(g), .. } => {
                for (k, child) in g.children.into_iter().enumerate() {
                    list.insert(i + k, child);
                }
                true
            }
            other => {
                list.insert(i, other);
                false
            }
        }
    }

    /// Every layer (not group) in drawing order.
    pub fn layers(&self) -> Vec<&Node> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
            for n in nodes {
                match &n.kind {
                    Kind::Layer(_) => out.push(n),
                    Kind::Group(g) => walk(&g.children, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }
}

// ----------------------------------------------------------------------------------------------- the binary format

struct Writer {
    buf: Vec<u8>,
    /// the format written (the tests write older ones to see that they are still read)
    version: u8,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
    }
    fn rgb(&mut self, c: [f32; 3]) {
        c.iter().for_each(|v| self.f32(*v));
    }
    fn opt_rgb(&mut self, c: Option<[f32; 3]>) {
        self.bool(c.is_some());
        if let Some(c) = c {
            self.rgb(c);
        }
    }
    fn glass(&mut self, g: &Glass) {
        self.bool(g.enabled);
        self.f32(g.specular);
        self.f32(g.blur);
        self.f32(g.refraction);
        self.f32(g.translucency);
        self.u8(g.shadow.code());
        self.f32(g.shadow_amount);
        if self.version >= 4 {
            self.f32(g.bevel);
            self.f32(g.bevel_width);
            self.f32(g.inner_shadow);
            self.f32(g.glow);
        }
    }
    fn gradient(&mut self, g: &Gradient) {
        self.rgb(g.a);
        self.rgb(g.b);
        self.f32(g.angle);
        self.bool(g.radial);
        if self.version >= 5 {
            self.u8(g.blobs.count);
            for b in g.blobs.used() {
                self.f32(b.at[0]);
                self.f32(b.at[1]);
                self.f32(b.size);
                self.rgb(b.colour);
            }
        }
    }
    fn props(&mut self, p: &Props) {
        self.bool(p.visible);
        self.f32(p.opacity);
        self.u8(p.blend.code());
        self.opt_rgb(p.fill);
        if self.version >= 4 {
            self.opt(p.gradient, |w, g| w.gradient(&g));
        }
        self.f32(p.offset[0]);
        self.f32(p.offset[1]);
        self.f32(p.scale);
        self.glass(&p.glass);
        self.u8(p.glass_mode.code());
    }
    fn background(&mut self, b: &Background) {
        self.u8(b.shape.code());
        self.u8(b.fill.code());
        self.rgb(b.top);
        self.rgb(b.bottom);
        self.glass(&b.glass);
    }
    fn opt<T: Copy>(&mut self, v: Option<T>, f: impl FnOnce(&mut Writer, T)) {
        self.bool(v.is_some());
        if let Some(x) = v {
            f(self, x);
        }
    }
    fn glass_patch(&mut self, g: &GlassPatch) {
        self.opt(g.enabled, |w, v| w.bool(v));
        self.opt(g.specular, |w, v| w.f32(v));
        self.opt(g.blur, |w, v| w.f32(v));
        self.opt(g.refraction, |w, v| w.f32(v));
        self.opt(g.translucency, |w, v| w.f32(v));
        self.opt(g.shadow, |w, v| w.u8(v.code()));
        self.opt(g.shadow_amount, |w, v| w.f32(v));
        if self.version >= 4 {
            self.opt(g.bevel, |w, v| w.f32(v));
            self.opt(g.bevel_width, |w, v| w.f32(v));
            self.opt(g.inner_shadow, |w, v| w.f32(v));
            self.opt(g.glow, |w, v| w.f32(v));
        }
    }
    fn props_patch(&mut self, p: &PropsPatch) {
        self.opt(p.visible, |w, v| w.bool(v));
        self.opt(p.opacity, |w, v| w.f32(v));
        self.opt(p.blend, |w, v| w.u8(v.code()));
        self.opt(p.fill, |w, v| w.opt_rgb(v));
        if self.version >= 4 {
            self.opt(p.gradient, |w, v| w.opt(v, |w, g| w.gradient(&g)));
        }
        self.opt(p.offset, |w, v| {
            w.f32(v[0]);
            w.f32(v[1]);
        });
        self.opt(p.scale, |w, v| w.f32(v));
        self.glass_patch(&p.glass);
        self.opt(p.glass_mode, |w, v| w.u8(v.code()));
    }
    fn background_patch(&mut self, b: &BackgroundPatch) {
        self.opt(b.shape, |w, v| w.u8(v.code()));
        self.opt(b.fill, |w, v| w.u8(v.code()));
        self.opt(b.top, |w, v| w.rgb(v));
        self.opt(b.bottom, |w, v| w.rgb(v));
        self.glass_patch(&b.glass);
    }
    /// The default value, then what each of the five other looks changes.
    fn variants<T: Patchable>(&mut self, v: &Variants<T>, value: impl Fn(&mut Writer, &T), patch: impl Fn(&mut Writer, &T::Patch)) {
        value(self, &v.default);
        for p in &v.slots {
            patch(self, p);
        }
    }
    fn node(&mut self, n: &Node) {
        self.u32(n.id);
        self.str(&n.name);
        match &n.kind {
            Kind::Layer(l) => {
                self.u8(0);
                self.str(&l.svg);
                self.variants(&l.props, |w, p| w.props(p), |w, p| w.props_patch(p));
            }
            Kind::Group(g) => {
                self.u8(1);
                self.variants(&g.props, |w, p| w.props(p), |w, p| w.props_patch(p));
                self.u32(g.children.len() as u32);
                for c in &g.children {
                    self.node(c);
                }
            }
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    version: u8,
}

type Res<T> = Result<T, String>;

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len()).ok_or("the file ends too early")?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Res<bool> {
        Ok(self.u8()? != 0)
    }
    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Res<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Res<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "a text is not valid UTF-8".to_string())
    }
    fn rgb(&mut self) -> Res<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn opt_rgb(&mut self) -> Res<Option<[f32; 3]>> {
        Ok(if self.bool()? { Some(self.rgb()?) } else { None })
    }
    fn coded<T>(&mut self, from: fn(u8) -> Option<T>) -> Res<T> {
        let c = self.u8()?;
        from(c).ok_or_else(|| format!("unknown code {c}"))
    }
    fn glass(&mut self) -> Res<Glass> {
        Ok(Glass {
            enabled: self.bool()?,
            specular: self.f32()?,
            blur: self.f32()?,
            refraction: self.f32()?,
            translucency: self.f32()?,
            shadow: self.coded(ShadowKind::from_code)?,
            shadow_amount: self.f32()?,
            ..self.glass_more()?
        })
    }
    /// The edge settings that came with version 4 (a plain glass has none).
    fn glass_more(&mut self) -> Res<Glass> {
        let d = Glass::default();
        if self.version < 4 {
            return Ok(d);
        }
        Ok(Glass { bevel: self.f32()?, bevel_width: self.f32()?, inner_shadow: self.f32()?, glow: self.f32()?, ..d })
    }
    fn gradient(&mut self) -> Res<Gradient> {
        let (a, b, angle, radial) = (self.rgb()?, self.rgb()?, self.f32()?, self.bool()?);
        let mut blobs = Blobs::default();
        if self.version >= 5 {
            let n = self.u8()? as usize;
            if n > MAX_BLOBS {
                return Err("a gradient has too many blobs".into());
            }
            for i in 0..n {
                blobs.spots[i] = Blob { at: [self.f32()?, self.f32()?], size: self.f32()?, colour: self.rgb()? };
            }
            blobs.count = n as u8;
        }
        Ok(Gradient { a, b, angle, radial, blobs })
    }
    fn props(&mut self) -> Res<Props> {
        Ok(Props {
            visible: self.bool()?,
            opacity: self.f32()?,
            blend: self.coded(Blend::from_code)?,
            fill: self.opt_rgb()?,
            gradient: if self.version >= 4 { self.opt(|r| r.gradient())? } else { None },
            offset: [self.f32()?, self.f32()?],
            scale: self.f32()?,
            glass: self.glass()?,
            glass_mode: self.coded(GlassMode::from_code)?,
        })
    }
    fn background(&mut self) -> Res<Background> {
        Ok(Background { shape: self.coded(Shape::from_code)?, fill: self.coded(FillKind::from_code)?, top: self.rgb()?, bottom: self.rgb()?, glass: self.glass()? })
    }
    fn opt<T>(&mut self, f: impl FnOnce(&mut Reader<'a>) -> Res<T>) -> Res<Option<T>> {
        Ok(if self.bool()? { Some(f(self)?) } else { None })
    }
    fn glass_patch(&mut self) -> Res<GlassPatch> {
        Ok(GlassPatch {
            enabled: self.opt(|r| r.bool())?,
            specular: self.opt(|r| r.f32())?,
            blur: self.opt(|r| r.f32())?,
            refraction: self.opt(|r| r.f32())?,
            translucency: self.opt(|r| r.f32())?,
            shadow: self.opt(|r| r.coded(ShadowKind::from_code))?,
            shadow_amount: self.opt(|r| r.f32())?,
            bevel: if self.version >= 4 { self.opt(|r| r.f32())? } else { None },
            bevel_width: if self.version >= 4 { self.opt(|r| r.f32())? } else { None },
            inner_shadow: if self.version >= 4 { self.opt(|r| r.f32())? } else { None },
            glow: if self.version >= 4 { self.opt(|r| r.f32())? } else { None },
        })
    }
    fn props_patch(&mut self) -> Res<PropsPatch> {
        Ok(PropsPatch {
            visible: self.opt(|r| r.bool())?,
            opacity: self.opt(|r| r.f32())?,
            blend: self.opt(|r| r.coded(Blend::from_code))?,
            fill: self.opt(|r| r.opt_rgb())?,
            gradient: if self.version >= 4 { self.opt(|r| r.opt(|r| r.gradient()))? } else { None },
            offset: self.opt(|r| Ok([r.f32()?, r.f32()?]))?,
            scale: self.opt(|r| r.f32())?,
            glass: self.glass_patch()?,
            glass_mode: self.opt(|r| r.coded(GlassMode::from_code))?,
        })
    }
    fn background_patch(&mut self) -> Res<BackgroundPatch> {
        Ok(BackgroundPatch {
            shape: self.opt(|r| r.coded(Shape::from_code))?,
            fill: self.opt(|r| r.coded(FillKind::from_code))?,
            top: self.opt(|r| r.rgb())?,
            bottom: self.opt(|r| r.rgb())?,
            glass: self.glass_patch()?,
        })
    }
    fn variants<T: Patchable>(&mut self, value: fn(&mut Reader<'a>) -> Res<T>, patch: fn(&mut Reader<'a>) -> Res<T::Patch>) -> Res<Variants<T>> {
        let default = value(self)?;
        let mut v = Variants::new(default);
        match self.version {
            // the first versions kept whole copies for the looks they set: what differs from the default is the patch
            1 => {
                // a dark and a mono variant; the mono one is the tinted (light) look now
                if let Some(x) = self.opt(|r| value(r))? {
                    v.slots[0] = T::diff(&default, &x);
                }
                if let Some(x) = self.opt(|r| value(r))? {
                    v.slots[3] = T::diff(&default, &x);
                }
            }
            2 => {
                for i in 0..5 {
                    if let Some(x) = self.opt(|r| value(r))? {
                        v.slots[i] = T::diff(&default, &x);
                    }
                }
            }
            _ => {
                for i in 0..5 {
                    v.slots[i] = patch(self)?;
                }
            }
        }
        Ok(v)
    }
    fn node(&mut self, depth: u32) -> Res<Node> {
        if depth > 64 {
            return Err("groups are nested too deeply".into());
        }
        let id = self.u32()?;
        let name = self.str()?;
        let kind = match self.u8()? {
            0 => Kind::Layer(Layer { svg: self.str()?, props: self.variants(|r| r.props(), |r| r.props_patch())? }),
            1 => {
                let props = self.variants(|r| r.props(), |r| r.props_patch())?;
                let n = self.u32()? as usize;
                let mut children = Vec::new();
                for _ in 0..n {
                    children.push(self.node(depth + 1)?);
                }
                Kind::Group(Group { children, props })
            }
            k => return Err(format!("unknown node kind {k}")),
        };
        Ok(Node { id, name, kind })
    }
}

/// The container: magic, flags, length, the deflated payload and its checksum.
fn pack(magic: &[u8; 8], raw: Vec<u8>) -> Vec<u8> {
    let mut crc = Crc::new();
    crc.update(&raw);
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw).expect("writing to memory");
    let packed = enc.finish().expect("writing to memory");
    let mut out = Vec::with_capacity(packed.len() + 17);
    out.extend_from_slice(magic);
    out.push(1);
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&packed);
    out.extend_from_slice(&crc.sum().to_le_bytes());
    out
}

/// The document as the bytes of a `.memicon` file.
pub fn encode(doc: &IconDoc) -> Vec<u8> {
    let mut w = Writer { buf: Vec::new(), version: 5 };
    w.str(&doc.name);
    w.variants(&doc.background, |w, b| w.background(b), |w, p| w.background_patch(p));
    w.f32(doc.margin);
    // (the tint colour of the "tinted by icon" look; the placeholder of older files stands for none)
    w.rgb(match doc.icon_tint {
        Some(c) if c != NO_ICON_TINT => c,
        Some(c) => [c[0] + 0.004, c[1], c[2]],
        None => NO_ICON_TINT,
    });
    w.u32(doc.next_id);
    w.u32(doc.nodes.len() as u32);
    for n in &doc.nodes {
        w.node(n);
    }
    pack(MAGIC, w.buf)
}

/// The document in the bytes of a `.memicon` file.
pub fn decode(bytes: &[u8]) -> Result<IconDoc, String> {
    if bytes.len() < 17 || (&bytes[..8] != MAGIC && &bytes[..8] != MAGIC_V4 && &bytes[..8] != MAGIC_V3 && &bytes[..8] != MAGIC_V2 && &bytes[..8] != MAGIC_V1) {
        return Err("this is not a .memicon file".into());
    }
    let version = if &bytes[..8] == MAGIC_V1 { 1 } else if &bytes[..8] == MAGIC_V2 { 2 } else if &bytes[..8] == MAGIC_V3 { 3 } else if &bytes[..8] == MAGIC_V4 { 4 } else { 5 };
    let flags = bytes[8];
    let raw_len = u32::from_le_bytes(bytes[9..13].try_into().unwrap()) as usize;
    if raw_len > 256 << 20 {
        return Err("the file claims to be too big".into());
    }
    let (body, tail) = bytes[13..].split_at(bytes.len() - 13 - 4);
    let crc = u32::from_le_bytes(tail.try_into().unwrap());
    let raw = if flags & 1 != 0 {
        let mut out = Vec::with_capacity(raw_len);
        DeflateDecoder::new(body).take(raw_len as u64 + 1).read_to_end(&mut out).map_err(|e| format!("cannot unpack the file: {e}"))?;
        out
    } else {
        body.to_vec()
    };
    if raw.len() != raw_len {
        return Err("the file is damaged (wrong length)".into());
    }
    let mut check = Crc::new();
    check.update(&raw);
    if check.sum() != crc {
        return Err("the file is damaged (checksum)".into());
    }
    let mut r = Reader { data: &raw, at: 0, version };
    let name = r.str()?;
    let background = r.variants(|r| r.background(), |r| r.background_patch())?;
    let margin = r.f32()?;
    let icon_tint = Some(r.rgb()?).filter(|c| *c != NO_ICON_TINT);
    let next_id = r.u32()?;
    let n = r.u32()? as usize;
    let mut nodes = Vec::new();
    for _ in 0..n {
        nodes.push(r.node(0)?);
    }
    if r.at != raw.len() {
        return Err("the file has extra data at the end".into());
    }
    Ok(IconDoc { name, background, nodes, margin, icon_tint, next_id })
}

pub fn save(path: &Path, doc: &IconDoc) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // written next to the file first, so that a crash never leaves half a file
    let tmp = path.with_extension("memicon.tmp");
    std::fs::write(&tmp, encode(doc)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<IconDoc, String> {
    decode(&std::fs::read(path).map_err(|e| e.to_string())?)
}

/// Where the documents are kept: `~/.local/share/memrium/icons`.
pub fn docs_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("memrium/icons")
}

/// The file of the document of icon `name`.
pub fn doc_path(name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    docs_dir().join(format!("{safe}.memicon"))
}

/// The names of the icons that have a document.
pub fn saved_names() -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(docs_dir()) else { return Vec::new() };
    rd.flatten().filter_map(|e| load(&e.path()).ok().map(|d| d.name)).collect()
}

/// Every saved document.
pub fn saved_docs() -> Vec<IconDoc> {
    let Ok(rd) = std::fs::read_dir(docs_dir()) else { return Vec::new() };
    let mut docs: Vec<IconDoc> = rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "memicon")).filter_map(|e| load(&e.path()).ok()).collect();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    docs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IconDoc {
        let mut d = IconDoc::new("firefox");
        let a = d.fresh_id();
        let b = d.fresh_id();
        let g = d.fresh_id();
        let mut props = Variants::new(Props { opacity: 0.8, blend: Blend::Multiply, fill: Some([0.1, 0.2, 0.3]), ..Props::default() });
        props.slots[0] = PropsPatch { opacity: Some(0.5), ..PropsPatch::default() };
        d.nodes.push(Node { id: a, name: "orange é".into(), kind: Kind::Layer(Layer { svg: "<svg/>".into(), props }) });
        d.nodes.push(Node {
            id: g,
            name: "Group".into(),
            kind: Kind::Group(Group {
                children: vec![Node { id: b, name: "blue".into(), kind: Kind::Layer(Layer { svg: "<svg>x</svg>".repeat(200), props: Variants::new(Props::default()) }) }],
                props: Variants { default: Props { glass_mode: GlassMode::Combined, ..Props::default() }, slots: [PropsPatch::default(), PropsPatch::default(), PropsPatch::default(), PropsPatch { visible: Some(false), ..PropsPatch::default() }, PropsPatch::default()] },
            }),
        });
        d.background.default.shape = Shape::Squircle;
        d.background.default.fill = FillKind::Gradient;
        d.background.slots[0] = BackgroundPatch { top: Some([0.1, 0.1, 0.1]), ..BackgroundPatch::default() };
        d
    }

    #[test]
    fn a_document_survives_the_file_format() {
        let d = sample();
        let bytes = encode(&d);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(decode(&bytes).unwrap(), d);
        assert!(bytes.len() < 400 + 30, "the deflated file is small: {}", bytes.len());
    }

    #[test]
    fn damaged_files_are_refused_not_misread() {
        let bytes = encode(&sample());
        assert!(decode(b"nonsense").is_err());
        let mut bad = bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x55;
        assert!(decode(&bad).is_err(), "a flipped byte is caught");
        assert!(decode(&bytes[..bytes.len() - 6]).is_err(), "a cut file is caught");
        let mut other = bytes.clone();
        other[7] = b'9';
        assert!(decode(&other).is_err(), "another version is not read as this one");
    }

    #[test]
    fn a_version_1_file_is_still_read() {
        // the first version had a dark and a mono variant after each default setting
        let mut w = Writer { buf: Vec::new(), version: 3 };
        w.str("old");
        w.background(&Background::default());
        w.bool(true);
        w.background(&Background { top: [0.1, 0.2, 0.3], ..Background::default() });
        w.bool(false);
        w.f32(0.05);
        w.rgb([0.2, 0.55, 0.95]);
        w.u32(3);
        w.u32(1);
        w.u32(1);
        w.str("a");
        w.u8(0);
        w.str("<svg/>");
        w.props(&Props::default());
        w.bool(false);
        w.bool(true);
        w.props(&Props { opacity: 0.4, ..Props::default() });
        let doc = decode(&pack(MAGIC_V1, w.buf)).expect("version 1 opens");
        assert_eq!(doc.name, "old");
        assert_eq!(doc.background.get(Appearance::Dark).top, [0.1, 0.2, 0.3], "the dark variant is the dark look");
        assert_eq!(doc.background.get(Appearance::Dark).bottom, Background::default().bottom, "and only what differed is kept in it");
        assert!(!doc.background.is_set(Appearance::TintedLight));
        let layer = &doc.layers()[0];
        assert!((layer.props().get(Appearance::TintedLight).opacity - 0.4).abs() < 1e-6, "the mono variant became the tinted (light) look");
        assert!((layer.props().get(Appearance::Dark).opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn layers_are_moved_grouped_and_removed() {
        let mut d = sample();
        let (a, b, g) = (1, 2, 3);
        assert_eq!(d.layers().len(), 2);
        assert!(d.shift(a, true), "a goes to the front");
        assert_eq!(d.nodes[1].id, a);
        assert!(!d.shift(a, true), "it cannot go further");
        let ng = d.group(a).unwrap();
        assert_eq!(d.find(ng).unwrap().children().len(), 1);
        assert!(d.ungroup(ng));
        assert_eq!(d.nodes.iter().filter(|n| n.id == a).count(), 1);
        assert!(d.remove(b).is_some(), "a layer inside a group is found");
        assert!(d.find(b).is_none());
        assert!(d.find(g).is_some());
        assert!(!d.ungroup(a), "a layer is not a group");
    }

    #[test]
    fn nodes_are_dragged_in_front_behind_into_groups_and_in_pairs() {
        let mut d = IconDoc::new("t");
        let ids: Vec<u32> = (0..4).map(|_| d.fresh_id()).collect();
        for (k, id) in ids.iter().enumerate() {
            d.nodes.push(Node { id: *id, name: format!("l{k}"), kind: Kind::Layer(Layer { svg: String::new(), props: Variants::new(Props::default()) }) });
        }
        let order = |d: &IconDoc| d.nodes.iter().map(|n| n.id).collect::<Vec<u32>>();
        assert_eq!(order(&d), vec![1, 2, 3, 4]);
        // the back layer goes in front of the top one
        assert!(d.move_node(1, Drop::InFront(4)));
        assert_eq!(order(&d), vec![2, 3, 4, 1]);
        assert!(d.move_node(1, Drop::Behind(2)));
        assert_eq!(order(&d), vec![1, 2, 3, 4]);
        // two layers made into a group, in the place of the one dropped on
        assert!(d.move_node(4, Drop::Pair(2)));
        assert_eq!(d.nodes.len(), 3);
        let group = d.nodes[1].id;
        assert_eq!(d.nodes[1].children().iter().map(|c| c.id).collect::<Vec<_>>(), vec![2, 4], "the dragged one is in front");
        // a layer dragged into the group, and out of it again in front of another
        assert!(d.move_node(3, Drop::Into(group)));
        assert_eq!(d.find(group).unwrap().children().len(), 3);
        assert!(d.move_node(3, Drop::InFront(1)));
        assert_eq!(d.find(group).unwrap().children().len(), 2);
        // a group cannot go into itself, a layer is not a place to go into, and a group that is emptied goes away
        assert!(!d.move_node(group, Drop::Into(group)));
        assert!(!d.move_node(group, Drop::InFront(2)), "2 is inside the group");
        assert!(!d.move_node(3, Drop::Into(1)));
        assert!(d.move_node(2, Drop::InFront(1)));
        assert!(d.move_node(4, Drop::InFront(1)));
        assert!(d.find(group).is_none(), "the empty group is gone");
        assert_eq!(d.layers().len(), 4);
    }

    #[test]
    fn a_duplicate_has_new_ids_and_sits_in_front() {
        let mut d = sample();
        let copy = d.duplicate(3).unwrap();
        assert_ne!(copy, 3);
        assert_eq!(d.nodes.iter().position(|n| n.id == copy), Some(2), "in front of the original");
        let ids: Vec<u32> = d.layers().iter().map(|n| n.id).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "no id is used twice: {ids:?}");
    }

    #[test]
    fn a_look_keeps_only_what_it_changed_and_inherits_the_rest_from_its_parent() {
        let mut v = Variants::new(Props { opacity: 0.8, ..Props::default() });
        let none = |_: Appearance, d: &Props| *d;
        assert_eq!(v.get(Appearance::Dark).opacity, 0.8);
        // the dark look changes the blend mode only
        let mut dark = v.get(Appearance::Dark);
        dark.blend = Blend::Screen;
        v.set(Appearance::Dark, dark, &none);
        assert!(v.is_set(Appearance::Dark));
        assert_eq!(v.slots[0], PropsPatch { blend: Some(Blend::Screen), ..PropsPatch::default() }, "nothing else is stored");
        // so a later change of the default look reaches it, except for the blend mode it set itself
        v.default.opacity = 0.3;
        v.default.blend = Blend::Multiply;
        assert_eq!(v.get(Appearance::Dark).opacity, 0.3, "inherited");
        assert_eq!(v.get(Appearance::Dark).blend, Blend::Screen, "its own");
        assert_eq!(v.get(Appearance::Default).blend, Blend::Multiply);
        // a look that was set to what it would inherit anyway keeps nothing
        let inherited = v.get(Appearance::Dark);
        let mut same = inherited;
        same.blend = Blend::Multiply;
        v.set(Appearance::Dark, same, &none);
        assert!(!v.is_set(Appearance::Dark));
        // dark clear follows clear light, which follows the default look
        let mut cl = v.get(Appearance::ClearLight);
        cl.scale = 0.5;
        v.set(Appearance::ClearLight, cl, &none);
        assert_eq!(v.get(Appearance::ClearDark).scale, 0.5, "inherited from clear (light)");
        let mut cd = v.get(Appearance::ClearDark);
        cd.scale = 0.7;
        v.set(Appearance::ClearDark, cd, &none);
        assert_eq!((v.get(Appearance::ClearLight).scale, v.get(Appearance::ClearDark).scale), (0.5, 0.7));
        assert_eq!(v.get(Appearance::TintedDark).scale, 1.0, "the tinted looks are a branch of their own");
        v.clear(Appearance::ClearDark);
        assert_eq!(v.get(Appearance::ClearDark).scale, 0.5);
    }

    #[test]
    fn a_version_2_file_is_still_read() {
        // version 2 had five optional whole copies after each default setting
        let mut w = Writer { buf: Vec::new(), version: 3 };
        w.str("v2");
        w.background(&Background::default());
        for _ in 0..5 {
            w.bool(false);
        }
        w.f32(0.05);
        w.rgb([0.4, 0.3, 0.8]);
        w.u32(2);
        w.u32(1);
        w.u32(1);
        w.str("a");
        w.u8(0);
        w.str("<svg/>");
        w.props(&Props::default());
        for i in 0..5 {
            w.bool(i == 2);
            if i == 2 {
                w.props(&Props { scale: 0.5, ..Props::default() });
            }
        }
        let doc = decode(&pack(MAGIC_V2, w.buf)).expect("version 2 opens");
        let layer = &doc.layers()[0];
        assert_eq!(layer.props().get(Appearance::ClearDark).scale, 0.5);
        assert_eq!(layer.props().get(Appearance::ClearLight).scale, 1.0);
    }

    #[test]
    fn gradients_and_edge_settings_are_saved_and_a_version_3_file_still_opens() {
        let mut d = IconDoc::new("grad");
        let id = d.fresh_id();
        let props = Props {
            gradient: Some(Gradient { a: [0.1, 0.2, 0.3], b: [0.9, 0.8, 0.7], angle: 135.0, radial: true, blobs: Blobs::starter([0.1, 0.2, 0.3], [0.9, 0.8, 0.7]) }),
            glass: Glass { enabled: true, bevel: 0.7, bevel_width: 0.3, inner_shadow: 0.4, glow: 0.2, ..Glass::default() },
            ..Props::default()
        };
        let mut variants = Variants::new(props);
        variants.slots[0] = PropsPatch { gradient: Some(None), glass: GlassPatch { bevel: Some(0.1), ..GlassPatch::default() }, ..PropsPatch::default() };
        d.nodes.push(Node { id, name: "g".into(), kind: Kind::Layer(Layer { svg: "<svg/>".into(), props: variants }) });
        assert_eq!(decode(&encode(&d)).unwrap(), d, "what was set comes back");
        // the same document written the way version 3 did: the new settings are lost, the rest is as it was
        let mut w = Writer { buf: Vec::new(), version: 3 };
        w.str(&d.name);
        w.variants(&d.background, |w, b| w.background(b), |w, b| w.background_patch(b));
        w.f32(d.margin);
        w.rgb([0.42, 0.34, 0.80]);
        w.u32(d.next_id);
        w.u32(1);
        w.node(&d.nodes[0]);
        let old = decode(&pack(MAGIC_V3, w.buf)).expect("version 3 opens");
        let Kind::Layer(l) = &old.nodes[0].kind else { panic!() };
        assert_eq!(l.props.default.gradient, None);
        assert_eq!(l.props.default.glass.bevel, 0.0);
        assert!(l.props.default.glass.enabled);
    }
}
