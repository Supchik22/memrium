//! wgpu renderer for the glass shader.
//!
//! Frame structure: the wallpaper is baked once; every frame it is copied into the scene texture, then the
//! window layer is drawn on top as a list of `Item`s (window frames/shadows as panels, client surfaces as
//! textured quads) with hardware alpha blending; finally the glass shell samples the scene texture.

use bytemuck::{Pod, Zeroable};
use chebrium::lucide;
use std::cell::{Cell, RefCell};
use chebrium::text::TextFonts;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

mod dmabuf;
mod scanout;
mod tile;
pub use tile::TileStyle;
pub use scanout::{create_scanout, primary_dev, scanout_modifiers, ScanoutImage};
pub use dmabuf::{create_device, DmaPlane, DmabufCaps, FOURCC_ABGR8888, FOURCC_ARGB8888, FOURCC_XBGR8888, FOURCC_XRGB8888};

pub const MAX_PANELS: usize = 576;
/// app icon atlas: 16 x 16 cells of 128 px, 3 mip levels
pub const APP_CELL: u32 = 128;
const APP_COLS: u32 = 24;
const APP_MIPS: u32 = 3;
const TEXT_W: u32 = 2048;
const TEXT_H: u32 = 1024;
const WALL_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Panel {
    pub rect: [f32; 4],
    pub params: [f32; 4],
    pub tint: [f32; 4],
    pub fx: [f32; 4],
    pub color2: [f32; 4],
    /// x = shadow opacity, y = shadow blur (px), z = shadow y-offset
    pub extra: [f32; 4],
    /// centre and half size of the box the panel is cut to (half width 0: not cut)
    pub clip: [f32; 4],
}

impl Panel {
    /// Cut the panel to `r` (x, y, w, h).
    pub fn cut_to(mut self, r: [f32; 4]) -> Self {
        self.clip = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] / 2.0, r[3] / 2.0];
        self
    }

    /// Pick black or white ink from what is behind this element (the backdrop under it, mixed with the
    /// tint of the glass it sits on: rgb + strength). Works for glyphs, text and solid fills.
    pub fn auto_ink(mut self, under: [f32; 4]) -> Self {
        let lum = 0.299 * under[0] + 0.587 * under[1] + 0.114 * under[2];
        self.fx[0] = 1.0 + under[3].clamp(0.0, 1.0);
        self.fx[1] = lum;
        self
    }

    /// Let this glass darken (with a cool hue shift) over bright backdrops, as set by the theme's `adapt`.
    pub fn adaptive(mut self) -> Self {
        self.params[2] = -self.params[2].abs();
        self
    }

    /// Opacity of a solid fill (the shader multiplies coverage by it).
    pub fn alpha(mut self, a: f32) -> Self {
        self.tint[3] = a;
        self
    }

    /// Override the glass tint (rgb + strength).
    pub fn tinted(mut self, t: [f32; 4]) -> Self {
        self.tint = t;
        self
    }

    /// Make the whole panel `f` as opaque (0 .. 1): glass fades with its blur, refraction, shadow and rim, not just its tint.
    /// Call it after `shadow`, which resets the field.
    pub fn faded(self, f: f32) -> Self {
        let mut p = self;
        p.fade_by(f);
        p
    }

    /// Multiply the panel's opacity by `f`, whatever kind of panel it is (a solid fill's alpha, a glyph's opacity, a glass
    /// panel's transparency).
    pub fn fade_by(&mut self, f: f32) {
        let f = f.clamp(0.0, 1.0);
        match self.params[1].floor() as u32 {
            // the shape kinds carry their transparency in the fraction of the kind (see the shader's `apply_panel`)
            // glass also keeps its edge sharpness in the fraction: 8 sharpness steps x 64 transparency steps (see `glass_code`)
            0 => {
                let idx = glass_idx(self.params[1]);
                let old = (idx & 63) as f32 / 63.0;
                let t = 1.0 - (1.0 - old) * f;
                self.params[1] = glass_code((idx >> 6) as f32 / 7.0, t);
            }
            k @ (2 | 3 | 4 | 12 | 13 | 14) => {
                let old = ((self.params[1] - k as f32) / 0.49).clamp(0.0, 1.0);
                self.params[1] = k as f32 + 0.49 * (1.0 - (1.0 - old) * f);
            }
            _ => self.tint[3] *= f,
        }
    }

    pub fn shadow(mut self, opacity: f32, blur: f32, dy: f32) -> Self {
        self.extra = [opacity, blur, dy, 0.0];
        self
    }

    /// No gloss / hairline (flat fill, used for window content).
    pub fn flat(mut self) -> Self {
        self.fx[2] = 0.0;
        self
    }

    pub fn glass(cx: f32, cy: f32, w: f32, h: f32, r: f32) -> Self {
        Self {
            rect: [cx, cy, w / 2.0, h / 2.0],
            params: [r, 0.0, 16.0, 26.0],
            tint: [0.80, 0.84, 0.92, 0.10],
            fx: [9.0, 0.05, 1.0, 0.0],
            color2: [0.0; 4],
            extra: [0.16, 22.0, 8.0, 0.0],
            clip: [0.0; 4],
        }
    }

    pub fn solid(cx: f32, cy: f32, w: f32, h: f32, r: f32, top: [f32; 3], bottom: [f32; 3]) -> Self {
        Self {
            rect: [cx, cy, w / 2.0, h / 2.0],
            params: [r, 1.0, 0.0, 0.0],
            tint: [top[0], top[1], top[2], 1.0],
            fx: [0.0, 0.0, 1.0, 1.0],
            color2: [bottom[0], bottom[1], bottom[2], 1.0],
            extra: [0.0; 4],
            clip: [0.0; 4],
        }
    }

    /// Apple-style app icon (squircle tile + SDF glyph `id`, see icons.wgsl).
    pub fn icon(cx: f32, cy: f32, size: f32, id: u32, top: [f32; 3], bottom: [f32; 3]) -> Self {
        let mut p = Self::solid(cx, cy, size, size, size * 0.225, top, bottom);
        p.params[1] = 3.0;
        p.params[2] = id as f32;
        p
    }

    /// A Lucide icon (by name) tinted with `color` at `opacity`; `size` is the square's side in logical px.
    pub fn glyph(cx: f32, cy: f32, size: f32, name: &str, color: [f32; 3], opacity: f32) -> Self {
        Self {
            rect: [cx, cy, size / 2.0, size / 2.0],
            params: [0.0, 5.0, lucide::id(name) as f32, 0.0],
            tint: [color[0], color[1], color[2], opacity],
            ..Default::default()
        }
    }

    /// A vinyl record in one panel (see `glass.wgsl`, kind 15): `radius` px, `tone` is the colour of the vinyl, `label` an
    /// atlas cell for the label (None: a plain one) and `angle` how far it has turned (radians).
    pub fn record(cx: f32, cy: f32, radius: f32, tone: [f32; 3], label: Option<u32>, angle: f32) -> Self {
        Self {
            rect: [cx, cy, radius, radius],
            params: [0.0, 15.0, label.map_or(-1.0, |c| c as f32), 0.0],
            tint: [tone[0], tone[1], tone[2], 1.0],
            fx: [angle, 0.40, 1.0, 0.0],
            ..Default::default()
        }
    }

    /// Cut a record (or anything of kind 15) to a rounded rectangle: centre, half size and corner radius.
    pub fn clipped_to(mut self, centre: [f32; 2], half: [f32; 2], radius: f32) -> Self {
        self.color2 = [centre[0], centre[1], half[0], half[1]];
        self.extra[3] = radius;
        self
    }

    /// An application icon from the app-icon atlas (see `Renderer::app_icon_cell`), `size` logical px square.
    pub fn app_icon(cx: f32, cy: f32, size: f32, cell: u32, opacity: f32) -> Self {
        Self {
            rect: [cx, cy, size / 2.0, size / 2.0],
            params: [0.0, 7.0, cell as f32, 0.0],
            tint: [1.0, 1.0, 1.0, opacity],
            ..Default::default()
        }
    }

    pub fn white(cx: f32, cy: f32, w: f32, h: f32, r: f32) -> Self {
        Self { rect: [cx, cy, w / 2.0, h / 2.0], params: [r, 2.0, 0.0, 0.0], extra: [0.12, 16.0, 6.0, 0.0], ..Default::default() }
    }
}

pub use chebrium::draw::Align;

/// A line of UI text. `x` is the left/centre/right edge per `align`, `y` the baseline (logical px).
#[derive(Clone)]
pub struct Label {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub bold: bool,
    pub color: [f32; 3],
    pub opacity: f32,
    pub align: Align,
    /// choose black / white from the backdrop (see `Panel::auto_ink`)
    pub auto: Option<[f32; 4]>,
    /// the backdrop under `auto` ink is not darkened by the glass adaptation (Fade / solid bars)
    pub flat_ink: bool,
    /// shown exactly as it is (file names, text the user typed), never translated
    pub verbatim: bool,
    /// the box the line is cut to (x, y, w, h)
    pub clip: Option<[f32; 4]>,
}

impl Label {
    pub fn new(text: impl Into<String>, x: f32, y: f32, size: f32, bold: bool, color: [f32; 3], opacity: f32, align: Align) -> Self {
        Label { text: text.into(), x, y, size, bold, color, opacity, align, auto: None, flat_ink: false, verbatim: false, clip: None }
    }

    pub fn auto_ink(mut self, under: [f32; 4]) -> Self {
        self.auto = Some(under);
        self
    }

    pub fn flat(mut self, on: bool) -> Self {
        self.flat_ink = on;
        self
    }

    /// This text is data, not interface: do not translate it.
    pub fn verbatim(mut self) -> Self {
        self.verbatim = true;
        self
    }
}

/// Chebrium widgets painted by the GPU glass renderer: every call becomes a panel or a label of the frame.
pub struct PanelDraw<'a> {
    pub panels: &'a mut Vec<Panel>,
    pub labels: &'a mut Vec<Label>,
    /// the clips in force, innermost last (each already cut by the one around it)
    clips: Vec<[f32; 4]>,
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

impl<'a> PanelDraw<'a> {
    pub fn new(panels: &'a mut Vec<Panel>, labels: &'a mut Vec<Label>) -> Self {
        PanelDraw { panels, labels, clips: Vec::new() }
    }

    fn push_panel(&mut self, p: Panel) {
        let p = match self.clips.last() {
            Some(c) => p.cut_to(*c),
            None => p,
        };
        self.panels.push(p);
    }
}

impl chebrium::draw::Draw for PanelDraw<'_> {
    fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32) {
        self.grad(r, radius, c, c, alpha);
    }

    fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32) {
        self.push_panel(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], radius, top, bottom).flat().alpha(alpha));
    }

    fn glyph(&mut self, cx: f32, cy: f32, size: f32, icon: &str, c: [f32; 3], alpha: f32) {
        self.push_panel(Panel::glyph(cx, cy, size, icon, c, alpha));
    }

    fn text(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align) {
        let mut l = Label::new(text, x, y, (size * 2.0).round() / 2.0, bold, c, alpha, align);
        l.clip = self.clips.last().copied();
        self.labels.push(l);
    }

    fn push_clip(&mut self, r: [f32; 4]) {
        let r = match self.clips.last() {
            Some(c) => overlap(*c, r),
            None => r,
        };
        self.clips.push(r);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn mark(&self) -> (usize, usize) {
        (self.panels.len(), self.labels.len())
    }

    /// What lies wholly outside `y0..y1` is dropped; what crosses an edge is cut there, so a scrolled row slides out
    /// from under the frame instead of vanishing at once.
    fn cull(&mut self, from: (usize, usize), y0: f32, y1: f32) -> f32 {
        const WIDE: f32 = 1.0e5;
        let band = [-WIDE, y0, 2.0 * WIDE, y1 - y0];
        let mut bottom = f32::MIN;
        let mut keep = from.0;
        for i in from.0..self.panels.len() {
            let mut p = self.panels[i];
            bottom = bottom.max(p.rect[1] + p.rect[3]);
            let (top, low) = (p.rect[1] - p.rect[3], p.rect[1] + p.rect[3]);
            if low <= y0 || top >= y1 {
                continue;
            }
            if top < y0 || low > y1 {
                let cut = if p.clip[2] > 0.0 { overlap([p.clip[0] - p.clip[2], p.clip[1] - p.clip[3], 2.0 * p.clip[2], 2.0 * p.clip[3]], band) } else { band };
                p = p.cut_to(cut);
            }
            self.panels[keep] = p;
            keep += 1;
        }
        self.panels.truncate(keep);
        let mut keep = from.1;
        for i in from.1..self.labels.len() {
            let mut l = self.labels[i].clone();
            let (top, low) = (l.y - l.size, l.y + l.size * 0.4);
            bottom = bottom.max(low);
            if low <= y0 || top >= y1 {
                continue;
            }
            if top < y0 || low > y1 {
                l.clip = Some(match l.clip {
                    Some(c) => overlap(c, band),
                    None => band,
                });
            }
            self.labels[keep] = l;
            keep += 1;
        }
        self.labels.truncate(keep);
        bottom
    }

    /// A thumbnail from the app-icon atlas; the 16:9 quad over the squashed picture restores its aspect ratio.
    fn image(&mut self, r: [f32; 4], radius: f32, handle: u32) {
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let mut pn = Panel::app_icon(cx, cy, 1.0, handle, 1.0);
        pn.rect = [cx, cy, r[2] / 2.0, r[3] / 2.0];
        pn.params[0] = radius;
        pn.params[3] = 3.0;
        self.push_panel(pn);
    }
}

struct TextEntry {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    ascent: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    res: [f32; 2],
    count: u32,
    scale: f32,
    flags: [u32; 4],
    grid: [f32; 4],
    /// x = opacity of the whole draw (window fade in / out)
    fade: [f32; 4],
    /// genie warp: the window's rect (x, y, w, h) and (progress, dock x, dock y, icon width)
    warp: [f32; 4],
    genie: [f32; 4],
    panels: [Panel; MAX_PANELS],
}

/// The part of `Globals` before the panel array (written alone, followed by only the panels in use).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Header {
    res: [f32; 2],
    count: u32,
    scale: f32,
    flags: [u32; 4],
    grid: [f32; 4],
    fade: [f32; 4],
    warp: [f32; 4],
    genie: [f32; 4],
}

/// Spectrum bands handed to live wallpapers, plus four levels (overall, bass, mids, treble).
pub const AUDIO_BANDS: usize = 64;
const AUDIO_BYTES: u64 = ((AUDIO_BANDS + 4) * 4) as u64;

/// WGSL every live wallpaper can use: `lw_band(x)` is the loudness (0..1) at x (0 = lowest pitch, 1 = highest), `lw_level(i)`
/// is 0 overall, 1 bass, 2 mids, 3 treble. Silence reads as zeros.
const AUDIO_WGSL: &str = r#"
struct LwAudio { b: array<vec4<f32>, 16>, lvl: vec4<f32> };
@group(1) @binding(0) var<uniform> lw_audio: LwAudio;
fn lw_bin(i: i32) -> f32 {
    let k = clamp(i, 0, 63);
    let v = lw_audio.b[k / 4];
    let r = k % 4;
    if (r == 0) { return v.x; }
    if (r == 1) { return v.y; }
    if (r == 2) { return v.z; }
    return v.w;
}
fn lw_band(x: f32) -> f32 {
    let f = clamp(x, 0.0, 1.0) * 63.0;
    let i = i32(floor(f));
    return mix(lw_bin(i), lw_bin(i + 1), fract(f));
}
fn lw_level(i: i32) -> f32 {
    if (i == 0) { return lw_audio.lvl.x; }
    if (i == 1) { return lw_audio.lvl.y; }
    if (i == 2) { return lw_audio.lvl.z; }
    return lw_audio.lvl.w;
}
"#;

/// One uniform "slot" per draw: slot 0 = wallpaper bake, 1 = shell, 2.. = window items (dynamic offsets).
const SLOTS: u64 = 128;
/// slots 2..WINDOW_SLOTS: window items; SHELL_SLOT0.. : spatial groups of the shell; MIP_SLOT0.. : mip-chain passes
const WINDOW_SLOTS: u64 = 96;
const SHELL_SLOT0: u64 = 100;
const MAX_SHELL_GROUPS: usize = 20;
const MIP_SLOT0: u64 = 120;
/// levels of the backdrop pyramid (level n is 2^n times smaller)
const BLUR_LEVELS: u32 = 6;
/// binned draws: the draw area is cut into tiles of this size (logical px) and each tile lists its panels
const TILE_PX: f32 = 32.0;
/// bytes of tile table per slot (u32 entries: offsets, then panel indices)
const TILE_STRIDE: u64 = 131072;

fn slot_stride() -> u64 {
    (std::mem::size_of::<Globals>() as u64 + 255) / 256 * 256
}

/// What the compositor draws in the window layer, back to front.
/// Genie effect (minimise to the Dock): the window's pixels are squeezed into the icon, bottom first.
#[derive(Clone, Copy, PartialEq)]
pub struct Warp {
    /// the window's rect (x, y, w, h), logical px
    pub rect: [f32; 4],
    /// 0 = untouched .. 1 = gone into the icon
    pub t: f32,
    pub dock: [f32; 2],
    pub icon_w: f32,
    /// jelly instead of genie (`t` is ignored): how far the window's far parts lag behind (x, y), and where it is held
    /// (the share of its width, along the top edge)
    pub jelly: Option<[f32; 3]>,
}

/// Move everything of a frame by (dx, dy) logical px: the desktop may start left of / above the main monitor, the
/// canvas the renderer draws on always starts at 0.
pub fn shift_scene(items: &mut [Item<'_>], shell: &mut [Panel], dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for p in shell.iter_mut() {
        p.rect[0] += dx;
        p.rect[1] += dy;
    }
    for it in items.iter_mut() {
        match it {
            Item::Warp(Some(w)) => {
                w.rect[0] += dx;
                w.rect[1] += dy;
                w.dock[0] += dx;
                w.dock[1] += dy;
            }
            Item::Warp(None) => {}
            Item::Frame { panels, bbox, .. } | Item::Layer { panels, bbox } => {
                for p in panels.iter_mut() {
                    p.rect[0] += dx;
                    p.rect[1] += dy;
                }
                bbox[0] += dx;
                bbox[2] += dx;
                bbox[1] += dy;
                bbox[3] += dy;
            }
            Item::Surface { rect, clip, .. } => {
                rect[0] += dx;
                rect[1] += dy;
                if let Some((r, _)) = clip {
                    r[0] += dx;
                    r[1] += dy;
                }
            }
            Item::Opaque(b) => {
                b[0] += dx;
                b[2] += dx;
                b[1] += dy;
                b[3] += dy;
            }
        }
    }
}

/// Give every panel of `kind` (12 paper, 13 wood) the same colour, whatever the part of the shell that made it asked
/// for: all the boards are cut from the same tree.
pub fn retint_boards(items: &mut [Item<'_>], shell: &mut [Panel], kind: u32, tint: [f32; 4]) {
    let fix = |p: &mut Panel| {
        if p.params[1].floor() as u32 == kind {
            p.tint = tint;
        }
    };
    shell.iter_mut().for_each(fix);
    for it in items.iter_mut() {
        if let Item::Frame { panels, .. } | Item::Layer { panels, .. } = it {
            panels.iter_mut().for_each(fix);
        }
    }
}

/// Clay in pastel colours: small pieces each get their own (peach, mint, lilac, butter, sky), big surfaces a plain
/// cream, and text that was white turns dark so it reads on the light clay. By night everything is deeper.
pub fn pastelize_clay(items: &mut [Item<'_>], shell: &mut [Panel], dark: bool) {
    const PALETTE: [[f32; 3]; 5] = [[1.0, 0.80, 0.74], [0.78, 0.92, 0.82], [0.80, 0.82, 1.0], [1.0, 0.92, 0.70], [0.74, 0.90, 1.0]];
    let fix = |p: &mut Panel| {
        if p.params[1].floor() as u32 == 14 {
            let big = p.rect[2] > 150.0 || p.rect[3] > 100.0;
            let base = if big { [0.98, 0.93, 0.88] } else { PALETTE[((p.rect[2] * 7.0 + p.rect[3] * 13.0) as u32 % 5) as usize] };
            let k = if dark { 0.42 } else { 1.0 };
            p.tint = [base[0] * k, base[1] * k, base[2] * k + if dark { 0.04 } else { 0.0 }, 1.0];
        }
    };
    shell.iter_mut().for_each(fix);
    // text and glyphs of the shell that were white are dark ink on light clay
    if !dark {
        for p in shell.iter_mut() {
            if matches!(p.params[1].floor() as u32, 5 | 6) && p.tint[0] > 0.85 && p.tint[1] > 0.85 && p.tint[2] > 0.85 {
                p.tint[0] = 0.20;
                p.tint[1] = 0.17;
                p.tint[2] = 0.22;
            }
        }
    }
    for it in items.iter_mut() {
        if let Item::Frame { panels, .. } | Item::Layer { panels, .. } = it {
            panels.iter_mut().for_each(fix);
        }
    }
}

pub enum Item<'a> {
    /// Applies `Warp` to every Frame / Surface up to the next `Warp` item (`None` turns it off).
    Warp(Option<Warp>),
    /// Window chrome: shadows, bodies, titlebars, traffic lights... (panels over transparent).
    /// `bbox` = (x0, y0, x1, y1) in logical px, used to limit the work.
    Frame { panels: Vec<Panel>, bbox: [f32; 4], fade: f32 },
    /// A client surface (toplevel, subsurface or popup) at `rect` = (x, y, w, h) logical px.
    /// `clip` = optional (rect, corner radius) of the window shape it is cut to; `frosted` puts blurred
    /// glass behind translucent pixels.
    /// `veil` < 1 shows only that much of the picture, even where it is opaque: the (frosted) glass behind comes through.
    Surface { tex: &'a SurfaceTex, rect: [f32; 4], clip: Option<([f32; 4], f32)>, frosted: bool, alpha: f32, veil: f32 },
    /// Panels that hardly ever change and sit under the windows (the desktop's icons). Drawn like a `Frame`, but once they
    /// have stayed the same for a frame they are painted into a texture of their own and only copied after that, so
    /// moving a window over them does not repaint every icon and label each frame.
    Layer { panels: Vec<Panel>, bbox: [f32; 4] },
    /// The surface just before this item is opaque inside this box (x0, y0, x1, y1 logical px): nothing beneath it is drawn
    /// there. A big window then costs the client's picture, not the glass of its frame and the desktop under it as well.
    Opaque([f32; 4]),
}

/// The picture of a `Item::Layer`, and what it was made from.
struct LayerCache {
    surf: SurfaceTex,
    key: u64,
}

/// An application icon made ready (put on its tile, premultiplied, with its mips) so that the main thread only has to copy it
/// into the atlas.
pub struct BakedIcon {
    tile: Option<TileStyle>,
    levels: Vec<image::RgbaImage>,
    source: image::RgbaImage,
}

/// The cell picture of an icon and its mips, premultiplied (so mips and bilinear filtering do not bleed colour from
/// transparent pixels). `tile`: the style of the tile it sits on, or None to show the icon as it is.
fn bake_icon(img: &image::RgbaImage, tile: Option<TileStyle>) -> Vec<image::RgbaImage> {
    let mut base = match tile.as_ref() {
        Some(t) => tile::bake(img, t, APP_CELL),
        None => image::imageops::resize(img, APP_CELL, APP_CELL, image::imageops::FilterType::Lanczos3),
    };
    for p in base.pixels_mut() {
        let a = p.0[3] as u32;
        for c in 0..3 {
            p.0[c] = ((p.0[c] as u32 * a + 127) / 255) as u8;
        }
    }
    (0..APP_MIPS)
        .map(|mip| {
            let cell = APP_CELL >> mip;
            if mip == 0 { base.clone() } else { image::imageops::resize(&base, cell, cell, image::imageops::FilterType::Triangle) }
        })
        .collect()
}

/// Make an application icon ready for `Renderer::app_icon_insert`: the slow part (the tile, the mips), fit for another thread.
pub fn bake_app_icon(img: &image::RgbaImage, tile: Option<TileStyle>) -> BakedIcon {
    // the source is kept to bake the icon again when the tile style changes; the atlas cell is APP_CELL, so a bigger
    // source would only waste memory
    let source = if img.width() > APP_CELL || img.height() > APP_CELL { image::imageops::thumbnail(img, APP_CELL, APP_CELL) } else { img.clone() };
    BakedIcon { tile, levels: bake_icon(img, tile), source }
}

/// A client's pixels on the GPU (BGRA8).
impl SurfaceTex {
    /// Wrap a texture made elsewhere (an imported DMA-BUF).
    pub(crate) fn from_texture(tex: wgpu::Texture, size: (u32, u32), opaque: bool) -> SurfaceTex {
        let view = tex.create_view(&Default::default());
        SurfaceTex { tex, view, size, opaque, bind: RefCell::new((u64::MAX, None)), gen: std::cell::Cell::new(0) }
    }
}

pub struct SurfaceTex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    pub size: (u32, u32),
    pub opaque: bool,
    /// bind group for the current renderer generation (rebuilt after a resize)
    bind: RefCell<(u64, Option<wgpu::BindGroup>)>,
    /// counts the uploads: a frame knows by it that this surface's picture changed
    gen: std::cell::Cell<u64>,
}

impl SurfaceTex {
    pub fn generation(&self) -> u64 {
        self.gen.get()
    }

    /// The picture changed without an upload (a client redrew a shared GPU buffer): only this surface is redrawn.
    pub fn touch(&self) {
        self.gen.set(self.gen.get() + 1);
    }
}

struct MipPass {
    read_wall: wgpu::BindGroup,
    read_scene: wgpu::BindGroup,
    dst_wall: wgpu::TextureView,
    dst_scene: wgpu::TextureView,
    size: (u32, u32),
}

/// The old frame, shown outside a circle that grows from a point.
struct Reveal {
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// (centre in device px, radius in device px)
    state: ([f32; 2], f32),
    fade: bool,
    rim: bool,
}

/// GPU time of the passes of one frame (a debugging aid: `MEMRIUM_BENCH` in the headless backend).
pub struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period: f32,
}

pub struct Renderer {
    /// draw the windows over this flat colour instead of the wallpaper (a shot of a window with a see-through background)
    pub solid_bg: Option<[f64; 3]>,
    /// the part of the target (device px) the last frame redrew; `None`: all of it
    last_region: Option<(u32, u32, u32, u32)>,
    gpu_timer: Option<GpuTimer>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// what clients may share as DMA-BUF (None when the device cannot import them)
    dmabuf: Option<DmabufCaps>,
    bgl: wgpu::BindGroupLayout,
    ubuf: wgpu::Buffer,
    tile_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    dummy: wgpu::Texture,
    wall: wgpu::Texture,
    scene: wgpu::Texture,
    /// the painted `Item::Layer`, and the key of the one before it (with how many frames in a row it was the same)
    layer: Option<LayerCache>,
    layer_seen: (u64, u32),
    /// the blur pyramid of the scene was not rebuilt by the last frame (no glass on it): it holds an older picture
    mips_stale: bool,
    p_wall: wgpu::RenderPipeline,
    p_frame: wgpu::RenderPipeline,
    /// the same without the rarely used kinds of panel (see `EXOTIC` in the shader), for draws that have none
    p_frame_lean: wgpu::RenderPipeline,
    p_glass_lean: wgpu::RenderPipeline,
    /// the scene copied to the target, under the shell's glass
    p_copy: wgpu::RenderPipeline,
    p_surf: wgpu::RenderPipeline,
    p_glass: wgpu::RenderPipeline,
    p_down: wgpu::RenderPipeline,
    /// per level 1.. : (read group for the wallpaper, read group for the scene, destination views)
    mips: Vec<MipPass>,
    bg_wall: wgpu::BindGroup,
    bg_frame: wgpu::BindGroup,
    bg_shell: wgpu::BindGroup,
    /// bumped whenever the wall/scene textures are recreated (invalidates per-surface bind groups)
    generation: u64,
    /// bumped whenever GPU-side content changes outside the per-frame draw lists (uploads, wallpaper, atlases)
    epoch: Cell<u64>,
    /// opacity applied to the draw being written (see `Header::fade`)
    fade: Cell<f32>,
    warp: Cell<[f32; 4]>,
    genie: Cell<[f32; 4]>,
    /// logical size (px) and device-pixel scale
    size: [f32; 2],
    scale: f32,
    has_image: bool,
    atlas: wgpu::Texture,
    app_atlas: wgpu::Texture,
    app_cells: HashMap<String, u32>,
    /// cells of the icon atlas given back (thumbnails that went off screen) and the first one never used
    app_free: Vec<u32>,
    app_next: u32,
    /// the icons as loaded, kept so they can be set on tiles (or taken off them) when the style changes
    app_sources: HashMap<String, image::RgbaImage>,
    icon_tile: Option<TileStyle>,
    text_fonts: TextFonts,
    text_atlas: wgpu::Texture,
    text_cache: HashMap<(String, u32, bool), TextEntry>,
    /// shelf packer state: (cursor x, cursor y, current row height)
    text_cursor: (u32, u32, u32),
    format: wgpu::TextureFormat,
    layout: wgpu::PipelineLayout,
    /// live wallpaper pipeline (re-drawn every frame) when one is active
    live: Option<wgpu::RenderPipeline>,
    /// the spectrum a live wallpaper can read (`lw_band`): uniform buffer, its group and layout
    audio_buf: wgpu::Buffer,
    audio_bg: wgpu::BindGroup,
    audio_bgl: wgpu::BindGroupLayout,
    /// the live wallpaper reads the spectrum
    live_audio: bool,
    /// a video wallpaper is playing: frames arrive through `set_video_frame`
    video: bool,
    /// the theme-switch transition in progress (see `begin_reveal`)
    reveal: Option<Reveal>,
    started: std::time::Instant,
    glass_adapt: f32,
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, w: u32, h: u32, scale: f32, wallpaper: Option<image::RgbaImage>) -> Self {
        let dmabuf = DmabufCaps::query(&device);
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: slot_stride() * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tile_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tile tables"),
            size: TILE_STRIDE * SLOTS,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let tex_ty = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(
                    0,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                    },
                ),
                entry(1, wgpu::ShaderStages::FRAGMENT, tex_ty),
                entry(2, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(3, wgpu::ShaderStages::FRAGMENT, tex_ty), // Lucide atlas
                entry(4, wgpu::ShaderStages::FRAGMENT, tex_ty), // text atlas
                entry(5, wgpu::ShaderStages::FRAGMENT, tex_ty), // client surface
                entry(6, wgpu::ShaderStages::FRAGMENT, tex_ty), // app icon atlas
                entry(
                    7,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: true, min_binding_size: std::num::NonZeroU64::new(TILE_STRIDE) },
                ), // tile table
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("glass.wgsl"), "\n", include_str!("icons.wgsl")).into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&bgl], push_constant_ranges: &[] });
        let pipe_x = |entry: &str, fmt: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, exotic: bool| {
            let consts: &[(&str, f64)] = &[("EXOTIC", if exotic { 1.0 } else { 0.0 })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format: fmt, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants: consts, ..Default::default() },
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let pipe = |entry: &str, fmt: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| pipe_x(entry, fmt, blend, true);
        let p_frame_lean = pipe_x("fs_frame", WALL_FMT, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), false);
        let p_glass_lean = pipe_x("fs", format, None, false);
        let p_copy = pipe_x("fs_copy", format, None, false);
        let p_wall = pipe("fs_wall", WALL_FMT, None);
        let p_frame = pipe("fs_frame", WALL_FMT, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let p_surf = pipe("fs_surface", WALL_FMT, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let p_glass = pipe("fs", format, None);
        let p_down = pipe("fs_down", WALL_FMT, None);

        let has_image = wallpaper.is_some();
        let dummy = match &wallpaper {
            Some(img) => device.create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("wallpaper"),
                    size: wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: WALL_FMT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                img.as_raw(),
            ),
            None => Self::make_tex(&device, 1, 1),
        };
        let wall = Self::make_tex(&device, w, h);
        let scene = Self::make_tex(&device, w, h);
        let atlas_px = lucide::ATLAS_COLS * lucide::CELL;
        let atlas = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("lucide atlas"),
                size: wgpu::Extent3d { width: atlas_px, height: atlas_px, depth_or_array_layers: 1 },
                mip_level_count: lucide::MIPS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WALL_FMT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::MipMajor,
            &lucide::build_atlas(),
        );
        let text_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text atlas"),
            size: wgpu::Extent3d { width: TEXT_W, height: TEXT_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let app_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("app icon atlas"),
            size: wgpu::Extent3d { width: APP_CELL * APP_COLS, height: APP_CELL * APP_COLS, depth_or_array_layers: 1 },
            mip_level_count: APP_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WALL_FMT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let audio_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live audio"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let audio_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("live audio"), size: AUDIO_BYTES, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&audio_buf, 0, &[0u8; AUDIO_BYTES as usize]);
        let audio_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("live audio"), layout: &audio_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: audio_buf.as_entire_binding() }] });
        let mut r = Self {
            solid_bg: None,
            last_region: None,
            gpu_timer: None,
            bg_wall: Self::make_group(&device, &bgl, &ubuf, &tile_buf, &sampler, &atlas, &text_atlas, &app_atlas, &dummy.create_view(&Default::default()), &dummy.create_view(&Default::default())),
            bg_frame: Self::make_group(&device, &bgl, &ubuf, &tile_buf, &sampler, &atlas, &text_atlas, &app_atlas, &wall.create_view(&Default::default()), &dummy.create_view(&Default::default())),
            bg_shell: Self::make_group(&device, &bgl, &ubuf, &tile_buf, &sampler, &atlas, &text_atlas, &app_atlas, &scene.create_view(&Default::default()), &dummy.create_view(&Default::default())),
            device,
            queue,
            bgl,
            ubuf,
            tile_buf,
            sampler,
            dummy,
            wall,
            scene,
            layer: None,
            layer_seen: (0, 0),
            mips_stale: false,
            p_wall,
            p_frame,
            p_frame_lean,
            p_glass_lean,
            p_copy,
            p_surf,
            p_glass,
            p_down,
            mips: Vec::new(),
            generation: 0,
            epoch: Cell::new(0),
            fade: Cell::new(1.0),
            warp: Cell::new([0.0; 4]),
            genie: Cell::new([0.0; 4]),
            size: [w as f32 / scale, h as f32 / scale],
            scale,
            has_image,
            atlas,
            app_atlas,
            app_cells: HashMap::new(),
            app_free: Vec::new(),
            app_next: 0,
            app_sources: HashMap::new(),
            icon_tile: None,
            text_fonts: TextFonts::load(),
            dmabuf,
            text_atlas,
            text_cache: HashMap::new(),
            text_cursor: (0, 0, 0),
            format,
            layout,
            live: None,
            audio_buf,
            audio_bg,
            audio_bgl,
            live_audio: false,
            video: false,
            reveal: None,
            started: std::time::Instant::now(),
            glass_adapt: 0.0,
        };
        r.rebuild_groups();
        r.bake_wallpaper();
        r
    }

    /// Render target with a blur pyramid (wallpaper and scene).
    fn make_tex(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        let max_levels = (w.max(h).max(1) as f32).log2().floor() as u32 + 1;
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: BLUR_LEVELS.min(max_levels),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WALL_FMT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// `tex1` is the pass's backdrop/input (binding 1), `surf` the client surface texture (binding 5).
    #[allow(clippy::too_many_arguments)]
    fn make_group(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        ubuf: &wgpu::Buffer,
        tile_buf: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        atlas: &wgpu::Texture,
        text_atlas: &wgpu::Texture,
        app_atlas: &wgpu::Texture,
        tex1: &wgpu::TextureView,
        surf: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let atlas_view = atlas.create_view(&Default::default());
        let text_view = text_atlas.create_view(&Default::default());
        let app_view = app_atlas.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: ubuf, offset: 0, size: std::num::NonZeroU64::new(std::mem::size_of::<Globals>() as u64) }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(tex1) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&text_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(surf) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&app_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: tile_buf, offset: 0, size: std::num::NonZeroU64::new(TILE_STRIDE) }) },
            ],
        })
    }

    fn rebuild_groups(&mut self) {
        let dummy = self.dummy.create_view(&Default::default());
        self.bg_wall = Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &dummy, &dummy);
        self.bg_frame = Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &self.wall.create_view(&Default::default()), &dummy);
        self.bg_shell = Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &self.scene.create_view(&Default::default()), &dummy);
        // pyramid passes: read level k-1 (a single-mip view), write level k
        self.mips.clear();
        let levels = self.wall.mip_level_count();
        let (mut w, mut h) = (self.wall.width(), self.wall.height());
        for k in 1..levels {
            let single = |t: &wgpu::Texture, level: u32| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() });
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
            let rw = single(&self.wall, k - 1);
            let rs = single(&self.scene, k - 1);
            self.mips.push(MipPass {
                read_wall: Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &rw, &dummy),
                read_scene: Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &rs, &dummy),
                dst_wall: single(&self.wall, k),
                dst_scene: single(&self.scene, k),
                size: (w, h),
            });
        }
    }

    /// Time the window pass and the shell pass on the GPU (when the device has timestamp queries).
    pub fn enable_gpu_timer(&mut self) -> bool {
        if !self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return false;
        }
        let set = self.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gpu times"), ty: wgpu::QueryType::Timestamp, count: 8 });
        let resolve = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu times"), size: 64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu times read"), size: 64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        self.gpu_timer = Some(GpuTimer { set, resolve, read, period: self.queue.get_timestamp_period() });
        true
    }

    /// The part of the target (device px: x, y, w, h) the last frame redrew; `None` when it redrew all of it.
    pub fn last_region(&self) -> Option<(u32, u32, u32, u32)> {
        self.last_region
    }

    /// (window pass ms, shell pass ms, whole frame ms) of the last frame submitted, once it has finished.
    pub fn gpu_times(&self) -> Option<(f32, f32, f32)> {
        let t = self.gpu_timer.as_ref()?;
        let slice = t.read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait);
        let v: Vec<u64> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        t.read.unmap();
        let ms = |a: u64, b: u64| (b.saturating_sub(a)) as f32 * t.period / 1e6;
        Some((ms(v[0], v[1]), ms(v[2], v[3]), ms(v[4], v[5])))
    }

    fn mip0(tex: &wgpu::Texture) -> wgpu::TextureView {
        tex.create_view(&wgpu::TextureViewDescriptor { base_mip_level: 0, mip_level_count: Some(1), ..Default::default() })
    }

    /// Fill the blur pyramid of the wallpaper (`wall = true`) or the scene texture.
    fn gen_mips(&self, enc: &mut wgpu::CommandEncoder, wall: bool, region: Option<(u32, u32, u32, u32)>) {
        for (i, m) in self.mips.iter().enumerate() {
            // this level's size goes in as the "screen" so the fragment shader can address its texels
            let slot = MIP_SLOT0 + 1 + i as u64;
            let header = Header { res: [m.size.0 as f32, m.size.1 as f32], count: 0, scale: 1.0, flags: [0; 4], grid: [0.0; 4], fade: [1.0; 4], warp: [0.0; 4], genie: [0.0; 4] };
            self.queue.write_buffer(&self.ubuf, slot * slot_stride(), bytemuck::bytes_of(&header));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur pyramid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: if wall { &m.dst_wall } else { &m.dst_scene }, resolve_target: None, ops: wgpu::Operations { load: if region.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) }, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.p_down);
            pass.set_bind_group(0, if wall { &m.read_wall } else { &m.read_scene }, &Self::offsets(slot));
            if let Some((rx, ry, rw, rh)) = region {
                // only the damaged part of this level (a texel more on every side: the filter reads its neighbours)
                let lv = i as u32 + 1;
                let x0 = (rx >> lv).saturating_sub(1).min(m.size.0.saturating_sub(1));
                let y0 = (ry >> lv).saturating_sub(1).min(m.size.1.saturating_sub(1));
                let x1 = (((rx + rw) >> lv) + 2).min(m.size.0).max(x0 + 1);
                let y1 = (((ry + rh) >> lv) + 2).min(m.size.1).max(y0 + 1);
                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            }
            pass.draw(0..3, 0..1);
        }
    }

    /// Replace the wallpaper image (or fall back to the procedural one with `None`).
    /// How much glass darkens over bright backdrops (theme setting).
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Output size in device pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        ((self.size[0] * self.scale).round() as u32, (self.size[1] * self.scale).round() as u32)
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.get()
    }

    /// Something drawn from outside the shell changed (a client reused a shared buffer): the next frame must be drawn.
    pub fn damage(&self) {
        self.touch();
    }

    fn touch(&self) {
        self.epoch.set(self.epoch.get() + 1);
    }

    pub fn set_glass_adapt(&mut self, a: f32) {
        if a != self.glass_adapt {
            self.touch();
        }
        self.glass_adapt = a;
    }

    pub fn set_wallpaper(&mut self, img: Option<&image::RgbaImage>) {
        self.touch();
        self.video = false;
        self.has_image = img.is_some();
        self.dummy = match img {
            Some(img) => self.device.create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("wallpaper"),
                    size: wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: WALL_FMT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                img.as_raw(),
            ),
            None => Self::make_tex(&self.device, 1, 1),
        };
        self.generation += 1;
        self.rebuild_groups();
        self.bake_wallpaper();
    }

    /// Compile a live wallpaper (WGSL defining `live_wall`) and start drawing it every frame.
    /// `None` stops it. A shader that fails to compile is rejected and the old wallpaper stays.
    pub fn set_live_wallpaper(&mut self, src: Option<&str>) -> Result<(), String> {
        self.touch();
        let Some(src) = src else {
            self.live = None;
            self.bake_wallpaper();
            return Ok(());
        };
        let code = format!(
            "{}\n{}\n{}\n{}\n@fragment\nfn fs_live(in: VOut) -> @location(0) vec4<f32> {{\n    let uv = in.px / g.res;\n    return vec4(live_wall(uv, bitcast<f32>(g.flags.y), g.res.x / g.res.y), 1.0);\n}}\n",
            include_str!("glass.wgsl"),
            include_str!("icons.wgsl"),
            AUDIO_WGSL,
            src
        );
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("live wallpaper"), bind_group_layouts: &[&self.bgl, &self.audio_bgl], push_constant_ranges: &[] });
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live wallpaper"), source: wgpu::ShaderSource::Wgsl(code.into()) });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live wallpaper"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_live"), targets: &[Some(WALL_FMT.into())], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        if let Some(e) = pollster::block_on(self.device.pop_error_scope()) {
            return Err(e.to_string());
        }
        self.live = Some(pipeline);
        self.live_audio = src.contains("lw_band") || src.contains("lw_level");
        self.video = false;
        Ok(())
    }

    /// Redraws every frame: a live shader or a playing video.
    pub fn is_live(&self) -> bool {
        self.live.is_some() || self.video
    }

    /// The running live wallpaper reads the spectrum (the compositor then listens to the sound).
    pub fn live_uses_audio(&self) -> bool {
        self.live.is_some() && self.live_audio
    }

    /// New spectrum for the live wallpaper: `bands` 0..1 low to high pitch, `levels` = overall, bass, mids, treble.
    pub fn set_audio(&mut self, bands: &[f32; AUDIO_BANDS], levels: [f32; 4]) {
        let mut raw = [0f32; AUDIO_BANDS + 4];
        raw[..AUDIO_BANDS].copy_from_slice(bands);
        raw[AUDIO_BANDS..].copy_from_slice(&levels);
        self.queue.write_buffer(&self.audio_buf, 0, bytemuck::cast_slice(&raw));
    }

    /// One frame (tightly packed RGBA, `w` x `h`) of a video wallpaper. The texture is reused while the size stays.
    pub fn set_video_frame(&mut self, rgba: &[u8], w: u32, h: u32) {
        if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
            return;
        }
        self.touch();
        if !self.video || self.dummy.width() != w || self.dummy.height() != h {
            self.dummy = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video wallpaper"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WALL_FMT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.has_image = true;
            self.video = true;
            self.live = None;
            self.generation += 1;
            self.rebuild_groups();
        }
        self.queue.write_texture(
            self.dummy.as_image_copy(),
            &rgba[..(w * h * 4) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.bake_wallpaper();
    }

    /// The video wallpaper is gone (another wallpaper follows).
    pub fn end_video(&mut self) {
        self.video = false;
    }

    /// Forget every loaded application icon (after the icon style or theme changed).
    pub fn clear_app_icons(&mut self) {
        self.app_cells.clear();
        self.app_free.clear();
        self.app_next = 0;
        self.app_sources.clear();
        self.touch();
    }

    pub fn resize(&mut self, w: u32, h: u32, scale: f32) {
        self.touch();
        self.scale = scale;
        self.size = [w as f32 / scale, h as f32 / scale];
        self.wall = Self::make_tex(&self.device, w, h);
        self.scene = Self::make_tex(&self.device, w, h);
        self.generation += 1;
        self.rebuild_groups();
        self.bake_wallpaper();
    }

    // ------------------------------------------------------------ client surfaces

    pub fn create_surface_tex(&self, w: u32, h: u32, opaque: bool) -> SurfaceTex {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("client surface"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        SurfaceTex { tex, view, size: (w, h), opaque, bind: RefCell::new((u64::MAX, None)), gen: std::cell::Cell::new(0) }
    }

    /// Upload BGRA pixels (`stride` bytes per row) into a surface texture.
    pub fn upload_surface(&self, st: &SurfaceTex, bytes: &[u8], stride: u32) {
        self.upload_surface_rows(st, bytes, stride, 0..st.size.1);
    }

    /// Upload only rows `rows` of a surface; `bytes` starts at the first of them.
    pub fn upload_surface_rows(&self, st: &SurfaceTex, bytes: &[u8], stride: u32, rows: std::ops::Range<u32>) {
        let rows = rows.start.min(st.size.1)..rows.end.min(st.size.1);
        if rows.is_empty() {
            return;
        }
        st.gen.set(st.gen.get() + 1);
        let mut dst = st.tex.as_image_copy();
        dst.origin.y = rows.start;
        self.queue.write_texture(
            dst,
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(rows.len() as u32) },
            wgpu::Extent3d { width: st.size.0, height: rows.len() as u32, depth_or_array_layers: 1 },
        );
    }

    fn surface_group(&self, st: &SurfaceTex) -> wgpu::BindGroup {
        let mut b = st.bind.borrow_mut();
        if b.0 != self.generation || b.1.is_none() {
            let g = Self::make_group(&self.device, &self.bgl, &self.ubuf, &self.tile_buf, &self.sampler, &self.atlas, &self.text_atlas, &self.app_atlas, &self.wall.create_view(&Default::default()), &st.view);
            *b = (self.generation, Some(g));
        }
        b.1.clone().unwrap()
    }

    // ------------------------------------------------------------ application icons

    /// Put an application icon (any size, straight alpha) into the app-icon atlas, once per `key`.
    /// Returns its cell id for `Panel::app_icon`, or `None` when the atlas is full.
    pub fn app_icon_cell(&mut self, key: &str, img: &image::RgbaImage) -> Option<u32> {
        self.icon_cell(key, img, true)
    }

    /// Put `img` in the cell called `key`, replacing what was there (a cover that changes with every track).
    pub fn set_image_cell(&mut self, key: &str, img: &image::RgbaImage) -> Option<u32> {
        if let Some(&id) = self.app_cells.get(key) {
            self.upload_icon_cell(id, img, false);
            self.touch();
            return Some(id);
        }
        self.icon_cell(key, img, false)
    }

    /// A free cell of the icon atlas: one given back, else the next never used.
    fn take_cell(&mut self) -> Option<u32> {
        if let Some(id) = self.app_free.pop() {
            return Some(id);
        }
        (self.app_next < APP_COLS * APP_COLS).then(|| {
            self.app_next += 1;
            self.app_next - 1
        })
    }

    /// Give the cell of `key` back (a thumbnail that is no longer on screen); false when it had none.
    pub fn app_icon_free(&mut self, key: &str) -> bool {
        match self.app_cells.remove(key) {
            Some(id) => {
                self.app_sources.remove(key);
                self.app_free.push(id);
                true
            }
            None => false,
        }
    }

    /// Like `app_icon_cell` for pictures that are not application icons (file icons, thumbnails): never tiled.
    pub fn image_cell(&mut self, key: &str, img: &image::RgbaImage) -> Option<u32> {
        self.icon_cell(key, img, false)
    }

    fn icon_cell(&mut self, key: &str, img: &image::RgbaImage, application: bool) -> Option<u32> {
        if let Some(&c) = self.app_cells.get(key) {
            return Some(c);
        }
        // (a new cell is used by panels that are new themselves, so the frame's diff sees them: no full redraw)
        let id = self.take_cell()?;
        self.upload_icon_cell(id, img, application);
        if application {
            // the source is kept to bake the icon again when the tile style changes; the atlas cell is APP_CELL, so a bigger
            // source would only waste memory
            let src = if img.width() > APP_CELL || img.height() > APP_CELL { image::imageops::thumbnail(img, APP_CELL, APP_CELL) } else { img.clone() };
            self.app_sources.insert(key.to_string(), src);
        }
        self.app_cells.insert(key.to_string(), id);
        Some(id)
    }

    /// Put the icon in atlas cell `id`, on its tile when tiles are on.
    fn upload_icon_cell(&self, id: u32, img: &image::RgbaImage, application: bool) {
        let levels = bake_icon(img, self.icon_tile.filter(|_| application));
        self.write_icon_levels(id, &levels);
    }

    /// The tile the application icons sit on now (what `bake_app_icon` is asked for).
    pub fn icon_tile(&self) -> Option<TileStyle> {
        self.icon_tile
    }

    /// Put an icon that was made ready on another thread (`bake_app_icon`) in a cell of its own. False when it was made for
    /// another tile style than the current one: it is stale and has to be made again.
    pub fn app_icon_insert(&mut self, key: &str, baked: BakedIcon) -> bool {
        if baked.tile != self.icon_tile {
            return false;
        }
        if self.app_cells.contains_key(key) {
            return true;
        }
        let Some(id) = self.take_cell() else { return true };
        self.write_icon_levels(id, &baked.levels);
        self.app_sources.insert(key.to_string(), baked.source);
        self.app_cells.insert(key.to_string(), id);
        true
    }

    fn write_icon_levels(&self, id: u32, levels: &[image::RgbaImage]) {
        let (cx, cy) = (id % APP_COLS, id / APP_COLS);
        for (mip, level) in levels.iter().enumerate() {
            let mip = mip as u32;
            let cell = APP_CELL >> mip;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.app_atlas, mip_level: mip, origin: wgpu::Origin3d { x: cx * cell, y: cy * cell, z: 0 }, aspect: wgpu::TextureAspect::All },
                level.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(cell * 4), rows_per_image: Some(cell) },
                wgpu::Extent3d { width: cell, height: cell, depth_or_array_layers: 1 },
            );
        }
    }

    /// Set application icons on tiles (`Some`) or show them as they are (`None`); loaded icons are redone at once.
    pub fn set_icon_tile(&mut self, tile: Option<TileStyle>) {
        if self.icon_tile == tile {
            return;
        }
        self.icon_tile = tile;
        self.touch();
        for (key, &id) in &self.app_cells {
            if let Some(src) = self.app_sources.get(key) {
                self.upload_icon_cell(id, src, true);
            }
        }
    }

    pub fn app_icon_get(&self, key: &str) -> Option<u32> {
        self.app_cells.get(key).copied()
    }

    // ------------------------------------------------------------ text labels

    /// Rasterise (or fetch from the atlas) each label and return the shader panels that draw them.
    pub fn label_panels(&mut self, labels: &[Label]) -> Vec<Panel> {
        let scale = self.scale;
        let mut out = Vec::with_capacity(labels.len());
        for l in labels {
            if l.text.is_empty() {
                continue;
            }
            let px = (l.size * scale).round().max(4.0);
            // the shell's own texts are shown in the chosen language
            let text = if l.verbatim { std::borrow::Cow::Borrowed(l.text.as_str()) } else { crate::i18n::tr(&l.text) };
            let key = (text.to_string(), px as u32, l.bold);
            if !self.text_cache.contains_key(&key) {
                let r = self.text_fonts.rasterize(&text, px, l.bold);
                let (w, h) = (r.w as u32, r.h as u32);
                let (mut cx, mut cy, mut row) = self.text_cursor;
                if cx + w > TEXT_W {
                    cx = 0;
                    cy += row;
                    row = 0;
                }
                if cy + h > TEXT_H {
                    // atlas full: start over (entries already handed out this frame may glitch once)
                    self.text_cache.clear();
                    (cx, cy, row) = (0, 0, 0);
                    // painted layers hold glyphs of the old atlas
                    self.touch();
                }
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &self.text_atlas, mip_level: 0, origin: wgpu::Origin3d { x: cx, y: cy, z: 0 }, aspect: wgpu::TextureAspect::All },
                    &r.bitmap,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                self.text_cache.insert(key.clone(), TextEntry { x: cx, y: cy, w, h, ascent: r.ascent });
                self.text_cursor = (cx + w + 1, cy, row.max(h + 1));
            }
            let e = &self.text_cache[&key];
            // snap the quad to whole device pixels so glyphs stay crisp
            let (wl, hl) = (e.w as f32 / scale, e.h as f32 / scale);
            let left = match l.align {
                Align::Left => l.x,
                Align::Center => l.x - wl / 2.0,
                Align::Right => l.x - wl,
            };
            let top = l.y - e.ascent / scale;
            let (left, top) = ((left * scale).round() / scale, (top * scale).round() / scale);
            let mut p = Panel {
                rect: [left + wl / 2.0, top + hl / 2.0, wl / 2.0, hl / 2.0],
                params: [0.0, 6.0, e.x as f32, e.y as f32],
                tint: [l.color[0], l.color[1], l.color[2], l.opacity],
                // unscaled half size: keeps the whole line visible when the panel is scaled (window animations)
                color2: [wl / 2.0, hl / 2.0, 0.0, 0.0],
                ..Default::default()
            };
            if let Some(c) = l.clip {
                p = p.cut_to(c);
            }
            if let Some(u) = l.auto {
                p = p.auto_ink(u);
                if l.flat_ink {
                    p.fx[2] = 1.0;
                }
            }
            out.push(p);
        }
        out
    }

    /// Lettering made of glass: each label becomes a slab of glass in the shape of its glyphs (big text works best).
    /// `tint` is what the glass is filled with; `liquid` picks the smooth, luminous look over the crystal one.
    pub fn glass_labels(&mut self, labels: &[Label], glass: &crate::theme::GlassStyle, tint: [f32; 4], liquid: bool) -> Vec<Panel> {
        const PAD: f32 = 18.0;
        let flat = self.label_panels(labels);
        let sizes: Vec<f32> = labels.iter().filter(|l| !l.text.is_empty()).map(|l| l.size).collect();
        flat.into_iter()
            .enumerate()
            .map(|(i, t)| {
                // t: rect centre / half size, atlas position in params.z/w
                let (cx, cy, hw, hh) = (t.rect[0], t.rect[1], t.rect[2], t.rect[3]);
                let mut g = glass.panel(cx, cy, 2.0 * (hw + PAD), 2.0 * (hh + PAD), 0.0);
                g.params[1] += if liquid { 10.0 } else { 9.0 };
                // a soft, wide bevel: about a seventh of the letter height
                g.params[2] = (sizes.get(i).copied().unwrap_or(60.0) * 0.14).max(8.0);
                g.tint = tint;
                g.extra = [PAD, t.params[2], t.params[3], glass.saturation];
                g.fx[0] = g.fx[0].max(5.0);
                g
            })
            .collect()
    }

    /// Width in px of `text` at font size `px` (UI font).
    pub fn text_width(&self, text: &str, px: f32, bold: bool) -> f32 {
        self.text_fonts.width(&crate::i18n::tr(text), px, bold)
    }

    /// Device pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn logical_size(&self) -> [f32; 2] {
        self.size
    }

    // ------------------------------------------------------------ drawing

    fn write_slot(&self, slot: u64, panels: &[Panel]) {
        self.write_slot_binned(slot, panels, None);
    }

    /// Upload a draw's panels. With a `bbox` (logical x0, y0, x1, y1) of the area the draw covers, also build the tile
    /// table that lets each pixel look at only the panels near it, instead of all of them.
    fn write_slot_binned(&self, slot: u64, panels: &[Panel], bbox: Option<[f32; 4]>) {
        if panels.len() > MAX_PANELS {
            // nothing to do about it here, but silently dropping icons is a bug worth seeing in the log
            static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("renderer: {} panels in one draw, only the first {MAX_PANELS} are drawn", panels.len());
            }
        }
        let n = panels.len().min(MAX_PANELS);
        let mut header = Header { res: self.size, count: n as u32, scale: self.scale, flags: [self.has_image as u32, self.started.elapsed().as_secs_f32().to_bits(), self.glass_adapt.to_bits(), 0], grid: [0.0; 4], fade: [self.fade.get(), 0.0, 0.0, 0.0], warp: self.warp.get(), genie: self.genie.get() };
        if let Some(bb) = bbox.filter(|_| n > 6) {
            let (x0, y0) = (bb[0].max(0.0), bb[1].max(0.0));
            let tx = (((bb[2].min(self.size[0]) - x0) / TILE_PX).ceil() as usize).max(1);
            let ty = (((bb[3].min(self.size[1]) - y0) / TILE_PX).ceil() as usize).max(1);
            let tiles = tx * ty;
            let cap = (TILE_STRIDE / 4) as usize;
            if tiles + 1 < cap / 2 {
                // pass 1: how many panels per tile; pass 2: fill
                let range = |p: &Panel| -> Option<(usize, usize, usize, usize)> {
                    let b = panel_box(p);
                    let (fx0, fy0) = ((b[0] - x0) / TILE_PX, (b[1] - y0) / TILE_PX);
                    let (fx1, fy1) = ((b[2] - x0) / TILE_PX, (b[3] - y0) / TILE_PX);
                    if fx1 < 0.0 || fy1 < 0.0 || fx0 > tx as f32 || fy0 > ty as f32 {
                        return None;
                    }
                    Some((fx0.floor().max(0.0) as usize, fy0.floor().max(0.0) as usize, (fx1.floor() as usize).min(tx - 1), (fy1.floor() as usize).min(ty - 1)))
                };
                let mut counts = vec![0u32; tiles + 1];
                let mut total = 0usize;
                for p in &panels[..n] {
                    if let Some((ax, ay, bx, by)) = range(p) {
                        for yy in ay..=by {
                            for xx in ax..=bx {
                                counts[yy * tx + xx + 1] += 1;
                                total += 1;
                            }
                        }
                    }
                }
                if tiles + 1 + total <= cap {
                    for i in 1..counts.len() {
                        counts[i] += counts[i - 1];
                    }
                    let mut data = vec![0u32; tiles + 1 + total];
                    data[..tiles + 1].copy_from_slice(&counts);
                    let mut fill = counts.clone();
                    for (i, p) in panels[..n].iter().enumerate() {
                        if let Some((ax, ay, bx, by)) = range(p) {
                            for yy in ay..=by {
                                for xx in ax..=bx {
                                    let t = yy * tx + xx;
                                    data[tiles + 1 + fill[t] as usize] = i as u32;
                                    fill[t] += 1;
                                }
                            }
                        }
                    }
                    self.queue.write_buffer(&self.tile_buf, slot * TILE_STRIDE, bytemuck::cast_slice(&data));
                    header.grid = [x0, y0, TILE_PX, tx as f32];
                    header.flags[3] = ty as u32;
                }
            }
        }
        let mut bytes = Vec::with_capacity(std::mem::size_of::<Header>() + n * std::mem::size_of::<Panel>());
        bytes.extend_from_slice(bytemuck::bytes_of(&header));
        bytes.extend_from_slice(bytemuck::cast_slice(&panels[..n]));
        self.queue.write_buffer(&self.ubuf, slot * slot_stride(), &bytes);
    }

    /// Dynamic offsets of a slot: its uniform block and its tile table.
    fn offsets(slot: u64) -> [u32; 2] {
        [(slot * slot_stride()) as u32, (slot * TILE_STRIDE) as u32]
    }

    fn bake_wallpaper(&mut self) {
        self.write_slot(0, &[]);
        let mut enc = self.device.create_command_encoder(&Default::default());
        let view = Self::mip0(&self.wall);
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wallpaper"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.p_wall);
            pass.set_bind_group(0, &self.bg_wall, &Self::offsets(0));
            pass.draw(0..3, 0..1);
        }
        self.gen_mips(&mut enc, true, None);
        self.queue.submit([enc.finish()]);
    }

    /// The whole device pixels inside a logical (x0, y0, x1, y1) box, as (x, y, w, h).
    fn inner_px(&self, b: [f32; 4]) -> Option<(u32, u32, u32, u32)> {
        let (tw, th) = (self.size[0] * self.scale, self.size[1] * self.scale);
        let x0 = (b[0] * self.scale).ceil().clamp(0.0, tw);
        let y0 = (b[1] * self.scale).ceil().clamp(0.0, th);
        let x1 = (b[2] * self.scale).floor().clamp(0.0, tw);
        let y1 = (b[3] * self.scale).floor().clamp(0.0, th);
        (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then(|| (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
    }

    /// Scissor rect (physical px) for a logical (x0, y0, x1, y1) box, clamped to the target.
    fn scissor(&self, b: [f32; 4]) -> Option<(u32, u32, u32, u32)> {
        let (tw, th) = (self.size[0] * self.scale, self.size[1] * self.scale);
        let x0 = (b[0] * self.scale).floor().clamp(0.0, tw);
        let y0 = (b[1] * self.scale).floor().clamp(0.0, th);
        let x1 = (b[2] * self.scale).ceil().clamp(0.0, tw);
        let y1 = (b[3] * self.scale).ceil().clamp(0.0, th);
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return None;
        }
        Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
    }

    /// Wallpaper -> window layer (`items`) -> glass shell (`shell`) into `target`.
    pub fn render(&mut self, target: &wgpu::TextureView, items: &[Item], shell: &[Panel]) -> wgpu::CommandEncoder {
        self.render_damaged(target, items, shell, None)
    }

    /// As `render`, but when `damage` (logical px: x0, y0, x1, y1) is given only that part of `target` is redrawn: the
    /// rest of it must still hold the previous frame. The glass shell blurs and refracts what is behind it, so the part
    /// that is redone is wider than the damage by the most the glass can reach.
    pub fn render_damaged(&mut self, target: &wgpu::TextureView, items: &[Item], shell: &[Panel], damage: Option<[f32; 4]>) -> wgpu::CommandEncoder {
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.stamp(&mut enc, 4);
        const REACH: f32 = 80.0;
        // What is redone: the damage itself, and the parts of the shell's glass near it (the glass bends and blurs what
        // is up to REACH away). Kept as separate pieces that do not overlap: a video under the top bar redoes the video
        // and a strip of the bar, not the whole box around both with the window's frame and shadow in it.
        let parts: Vec<(u32, u32, u32, u32)> = match damage {
            None => Vec::new(),
            Some(d) => {
                let wide = [d[0] - REACH, d[1] - REACH, d[2] + REACH, d[3] + REACH];
                let mut glass: Vec<[f32; 4]> = Vec::new();
                for p in shell {
                    let b = panel_box(p);
                    if boxes_touch(&b, &wide) {
                        glass.push([b[0].max(wide[0]), b[1].max(wide[1]), b[2].min(wide[2]), b[3].min(wide[3])]);
                    }
                }
                // the panels of one bar touch each other: they become one box
                let mut merged = true;
                while merged {
                    merged = false;
                    'outer: for i in 0..glass.len() {
                        for j in i + 1..glass.len() {
                            if boxes_touch(&glass[i], &glass[j]) {
                                let b = glass.swap_remove(j);
                                let a = &mut glass[i];
                                *a = [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
                                merged = true;
                                break 'outer;
                            }
                        }
                    }
                }
                let mut out: Vec<(u32, u32, u32, u32)> = Vec::new();
                for b in std::iter::once([d[0] - 2.0, d[1] - 2.0, d[2] + 2.0, d[3] + 2.0]).chain(glass) {
                    let Some(r) = self.scissor(b) else { continue };
                    let mut new = vec![r];
                    for o in &out {
                        new = new.into_iter().flat_map(|n| cut(n, *o)).collect();
                    }
                    out.extend(new);
                }
                if out.len() > 12 {
                    // too many pieces: one box around them all
                    out = vec![out.iter().copied().reduce(bound).unwrap()];
                }
                out
            }
        };
        let ds: Option<(u32, u32, u32, u32)> = parts.iter().copied().reduce(bound);
        self.last_region = ds;

        // 0. a live wallpaper redraws the wall texture every frame
        if let Some(live) = &self.live {
            self.write_slot(0, &[]);
            let wall_view = Self::mip0(&self.wall);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live wallpaper"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &wall_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(live);
            pass.set_bind_group(0, &self.bg_wall, &Self::offsets(0));
            pass.set_bind_group(1, &self.audio_bg, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            self.gen_mips(&mut enc, true, None);
        }

        // 1. wallpaper into the scene texture (not when one opaque window, a fullscreen game, hides all of it)
        let (pw, ph) = ((self.size[0] * self.scale) as u32, (self.size[1] * self.scale) as u32);
        let hidden = !skip_flags().contains("occlude")
            && items.iter().enumerate().any(|(i, it)| matches!(it, Item::Opaque(b) if i > 0 && self.inner_px(*b).is_some_and(|r| r.0 == 0 && r.1 == 0 && r.2 >= pw && r.3 >= ph)));
        match ds {
            _ if hidden => {}
            Some(_) => {
                for &(x, y, w, h) in &parts {
                    let origin = wgpu::Origin3d { x, y, z: 0 };
                    let (mut src, mut dst) = (self.wall.as_image_copy(), self.scene.as_image_copy());
                    src.origin = origin;
                    dst.origin = origin;
                    enc.copy_texture_to_texture(src, dst, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
                }
            }
            None => match self.solid_bg {
                Some(c) => {
                    let view = Self::mip0(&self.scene);
                    drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("flat background"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 }), store: wgpu::StoreOp::Store } })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    }));
                }
                None => enc.copy_texture_to_texture(self.wall.as_image_copy(), self.scene.as_image_copy(), wgpu::Extent3d { width: pw.max(1), height: ph.max(1), depth_or_array_layers: 1 }),
            },
        }

        // 2. window layer: every item is one draw with its own uniform slot, blended over the scene
        let scene_view = Self::mip0(&self.scene);
        let mut draws: Vec<(bool, Option<wgpu::BindGroup>, [u32; 2], (u32, u32, u32, u32), bool)> = Vec::new();
        let skip_win = skip_flags().contains("win");
        let mut slot = 2u64;
        let skip_frames = skip_flags().contains("frame");
        let skip_surf = skip_flags().contains("surf");
        let skip_layer = skip_flags().contains("layer");
        // the opaque parts of windows (device px, rounded inwards) and the item each one belongs to
        let opaque: Vec<(usize, (u32, u32, u32, u32))> = if skip_flags().contains("occlude") {
            Vec::new()
        } else {
            items.iter().enumerate().filter_map(|(i, it)| if let Item::Opaque(b) = it { self.inner_px(*b).filter(|_| i > 0).map(|r| (i - 1, r)) } else { None }).collect()
        };
        // an item's scissor rect cut to the redone pieces, less what the windows above it hide
        let pieces = |s: Option<(u32, u32, u32, u32)>, idx: usize| -> Vec<(u32, u32, u32, u32)> {
            let Some(s) = s else { return Vec::new() };
            let mine: Vec<(u32, u32, u32, u32)> = if parts.is_empty() { vec![s] } else { parts.iter().filter_map(|r| clip_to(s, Some(*r))).collect() };
            mine.into_iter().flat_map(|c| uncovered(c, &opaque, idx)).collect()
        };
        let mut cur: Option<Warp> = None;
        let widen = |b: [f32; 4], w: &Option<Warp>| -> [f32; 4] {
            match w {
                Some(w) => [b[0].min(w.dock[0] - w.icon_w), b[1].min(w.dock[1] - w.icon_w), b[2].max(w.dock[0] + w.icon_w), b[3].max(w.dock[1] + w.icon_w)],
                None => b,
            }
        };
        for (idx, it) in items.iter().enumerate().filter(|_| !skip_win) {
            if let Item::Warp(w) = it {
                cur = *w;
                match w {
                    Some(w) => {
                        self.warp.set(w.rect);
                        match w.jelly {
                            // genie.x < 0 selects the jelly mapping in the shader
                            Some(j) => self.genie.set([-1.0, j[0], j[1], j[2]]),
                            None => self.genie.set([w.t, w.dock[0], w.dock[1], w.icon_w]),
                        }
                    }
                    None => {
                        self.warp.set([0.0; 4]);
                        self.genie.set([0.0; 4]);
                    }
                }
                continue;
            }
            if (skip_frames && matches!(it, Item::Frame { .. })) || (skip_surf && matches!(it, Item::Surface { .. })) {
                continue;
            }
            if slot >= WINDOW_SLOTS {
                eprintln!("renderer: too many window items this frame (limit {})", WINDOW_SLOTS - 2);
                break;
            }
            match it {
                Item::Warp(_) | Item::Opaque(_) => continue,
                Item::Layer { panels, bbox } => {
                    let scs = pieces(self.scissor(widen(*bbox, &cur)), idx);
                    if scs.is_empty() {
                        continue;
                    }
                    // the same as last frame (and the frame before it): paint it into a texture once and copy from there
                    let key = layer_key(panels, bbox, self.epoch.get(), self.generation, (pw, ph));
                    self.layer_seen = if self.layer_seen.0 == key { (key, self.layer_seen.1.saturating_add(1)) } else { (key, 0) };
                    let cacheable = self.live.is_none() && !skip_layer && self.layer_seen.1 >= 1 && slot + 1 < WINDOW_SLOTS && !panels.iter().any(is_exotic);
                    if cacheable {
                        if self.layer.as_ref().map_or(true, |c| c.surf.size != (pw.max(1), ph.max(1))) {
                            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("layer cache"),
                                size: wgpu::Extent3d { width: pw.max(1), height: ph.max(1), depth_or_array_layers: 1 },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: WALL_FMT,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                                view_formats: &[],
                            });
                            self.layer = Some(LayerCache { surf: SurfaceTex::from_texture(tex, (pw.max(1), ph.max(1)), false), key: 0 });
                        }
                        if self.layer.as_ref().is_some_and(|c| c.key != key) {
                            // no warp or fade under the cached picture: it is the layer as it is
                            let (w0, g0, f0) = (self.warp.get(), self.genie.get(), self.fade.get());
                            self.warp.set([0.0; 4]);
                            self.genie.set([0.0; 4]);
                            self.fade.set(1.0);
                            self.write_slot_binned(slot, panels, Some(*bbox));
                            self.warp.set(w0);
                            self.genie.set(g0);
                            self.fade.set(f0);
                            let lean = !panels.iter().any(is_exotic);
                            if let Some(c) = &mut self.layer {
                                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("layer cache"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &c.surf.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });
                                pass.set_pipeline(if lean { &self.p_frame_lean } else { &self.p_frame });
                                pass.set_bind_group(0, &self.bg_frame, &Self::offsets(slot));
                                pass.draw(0..3, 0..1);
                                c.key = key;
                            }
                        }
                        if let Some(c) = &self.layer {
                            let p = Panel { rect: [self.size[0] / 2.0, self.size[1] / 2.0, self.size[0] / 2.0, self.size[1] / 2.0], params: [0.0; 4], ..Default::default() };
                            let f0 = self.fade.get();
                            self.fade.set(1.0);
                            self.write_slot(slot + 1, &[p]);
                            self.fade.set(f0);
                            let g = self.surface_group(&c.surf);
                            for &sc in &scs {
                                draws.push((false, Some(g.clone()), Self::offsets(slot + 1), sc, false));
                            }
                            slot += 2;
                            continue;
                        }
                    }
                    // not steady yet (or not cacheable): drawn like any frame
                    self.write_slot_binned(slot, panels, Some(*bbox));
                    for &sc in &scs {
                        draws.push((true, None, Self::offsets(slot), sc, !panels.iter().any(is_exotic)));
                    }
                }
                Item::Frame { panels, bbox, fade } => {
                    if *fade < 0.005 {
                        continue;
                    }
                    let scs = pieces(self.scissor(widen(*bbox, &cur)), idx);
                    if scs.is_empty() {
                        continue;
                    }
                    self.fade.set(*fade);
                    self.write_slot_binned(slot, panels, Some(*bbox));
                    self.fade.set(1.0);
                    for &sc in &scs {
                        draws.push((true, None, Self::offsets(slot), sc, !panels.iter().any(is_exotic)));
                    }
                }
                Item::Surface { tex, rect, clip, frosted, alpha, veil } => {
                    if *alpha < 0.005 {
                        continue;
                    }
                    let scs = pieces(self.scissor(widen([rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]], &cur)), idx);
                    if scs.is_empty() {
                        continue;
                    }
                    let mut p = Panel {
                        rect: [rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, rect[2] / 2.0, rect[3] / 2.0],
                        params: [0.0, (tex.opaque as u32 | ((*frosted as u32) << 1)) as f32, if *veil < 0.999 { *veil } else { 0.0 }, 0.0],
                        ..Default::default()
                    };
                    if let Some((c, r)) = clip {
                        p.params[0] = *r;
                        p.color2 = [c[0] + c[2] / 2.0, c[1] + c[3] / 2.0, c[2] / 2.0, c[3] / 2.0];
                        p.extra[0] = 1.0;
                    }
                    self.fade.set(*alpha);
                    self.write_slot(slot, &[p]);
                    self.fade.set(1.0);
                    let g = self.surface_group(tex);
                    for &sc in &scs {
                        draws.push((false, Some(g.clone()), Self::offsets(slot), sc, false));
                    }
                }
            }
            slot += 1;
        }
        self.warp.set([0.0; 4]);
        self.genie.set([0.0; 4]);
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("windows"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &scene_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: self.gpu_timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: &t.set, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }),
                occlusion_query_set: None,
            });
            for (is_frame, group, off, sc, lean) in &draws {
                pass.set_pipeline(if *is_frame { if *lean { &self.p_frame_lean } else { &self.p_frame } } else { &self.p_surf });
                pass.set_bind_group(0, if *is_frame { &self.bg_frame } else { group.as_ref().unwrap() }, off);
                pass.set_scissor_rect(sc.0, sc.1, sc.2, sc.3);
                pass.draw(0..3, 0..1);
            }
        }

        // 3. the glass shell refracts the scene. The scene's blur pyramid first, then the scene is copied to the
        // target and each spatial group of shell panels is drawn inside its own scissor rect (so the per-pixel
        // panel loop only runs where panels are, not over the whole screen).
        let skip = skip_flags();
        let groups = if skip.contains("shell") { Vec::new() } else { cluster_panels(shell, self.size, MAX_SHELL_GROUPS) };
        // the pyramid is only read by glass: with none on the screen (a game in front of everything) it is not built, and
        // the first frame with glass again builds all of it
        if !skip.contains("mip") && (!groups.is_empty() || self.reveal.is_some()) {
            self.gen_mips(&mut enc, false, if self.mips_stale { None } else { ds });
            self.mips_stale = false;
        } else {
            self.mips_stale = true;
        }
        self.write_slot(1, &[]);
        let mut shell_draws: Vec<([u32; 2], (u32, u32, u32, u32), bool)> = Vec::new();
        for (i, (panels, bbox)) in groups.iter().enumerate() {
            let slot = SHELL_SLOT0 + i as u64;
            let scs = pieces(self.scissor(*bbox), usize::MAX);
            if scs.is_empty() {
                continue;
            }
            self.write_slot_binned(slot, panels, Some(*bbox));
            for sc in scs {
                shell_draws.push((Self::offsets(slot), sc, !panels.iter().any(is_exotic)));
            }
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shell"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, ops: wgpu::Operations { load: if ds.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) }, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: self.gpu_timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: &t.set, beginning_of_pass_write_index: Some(2), end_of_pass_write_index: Some(3) }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.p_copy);
            pass.set_bind_group(0, &self.bg_shell, &Self::offsets(1));
            if parts.is_empty() {
                pass.draw(0..3, 0..1);
            }
            for &(x, y, w, h) in &parts {
                pass.set_scissor_rect(x, y, w, h);
                pass.draw(0..3, 0..1);
            }
            for (off, sc, lean) in &shell_draws {
                pass.set_pipeline(if *lean { &self.p_glass_lean } else { &self.p_glass });
                pass.set_bind_group(0, &self.bg_shell, off);
                pass.set_scissor_rect(sc.0, sc.1, sc.2, sc.3);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some(r) = &self.reveal {
            let (w, h) = self.pixel_size();
            let data: [f32; 8] = [r.state.0[0], r.state.0[1], r.state.1, (2.0 * self.scale).max(1.5), w as f32, h as f32, if r.fade { 1.0 } else { 0.0 }, if r.rim { 1.0 } else { 0.0 }];
            self.queue.write_buffer(&r.ubuf, 0, bytemuck::cast_slice(&data));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reveal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&r.pipeline);
            pass.set_bind_group(0, &r.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.stamp(&mut enc, 5);
        if let Some(t) = &self.gpu_timer {
            enc.resolve_query_set(&t.set, 0..6, &t.resolve, 0);
            enc.copy_buffer_to_buffer(&t.resolve, 0, &t.read, 0, 48);
        }
        enc
    }

    /// Timer: a timestamp at this point of the frame (an empty pass, as timestamps outside passes need another feature).
    fn stamp(&self, enc: &mut wgpu::CommandEncoder, index: u32) {
        let Some(t) = &self.gpu_timer else { return };
        let view = Self::mip0(&self.scene);
        let at = |i| (index == i).then_some(index);
        drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("timer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &t.set, beginning_of_pass_write_index: at(4), end_of_pass_write_index: at(5) }),
            occlusion_query_set: None,
        }));
    }

    /// Start a circular reveal: `old` is the picture to wipe away (a texture of the screen's size and format).
    pub fn begin_reveal(&mut self, old: &wgpu::Texture, center: [f32; 2], fade: bool, rim: bool) {
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("reveal"), source: wgpu::ShaderSource::Wgsl(include_str!("reveal.wgsl").into()) });
        let bgl = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reveal"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reveal"), bind_group_layouts: &[&bgl], push_constant_ranges: &[] });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("reveal"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: self.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("reveal"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let view = old.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reveal"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.reveal = Some(Reveal { pipeline, ubuf, bind, state: (center, 0.0), fade, rim });
        self.touch();
    }

    /// Move the reveal's edge (device px).
    pub fn set_reveal_radius(&mut self, radius: f32) {
        if let Some(r) = &mut self.reveal {
            r.state.1 = radius;
        }
    }

    pub fn end_reveal(&mut self) {
        self.reveal = None;
        self.touch();
    }

    pub fn reveal_active(&self) -> bool {
        self.reveal.is_some()
    }

    /// Changes whenever the reveal moves, so the frame is not skipped as a duplicate.
    pub fn reveal_key(&self) -> u32 {
        self.reveal.as_ref().map_or(0, |r| r.state.1.to_bits() | 1)
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
}

/// How far a panel can touch pixels beyond its own rectangle (its shadow), as the shader computes it.
/// Does this panel need the rarely used parts of the shader (paper, wood, clay, vinyl, glass lettering)?
/// What a layer was painted from: its panels and place, the renderer's epoch (anything that changed outside the panels:
/// the glyph and icon atlases, the wallpaper), and the size.
fn layer_key(panels: &[Panel], bbox: &[f32; 4], epoch: u64, generation: u64, size: (u32, u32)) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(bytemuck::cast_slice(panels));
    h.write(bytemuck::cast_slice(bbox));
    h.write_u64(epoch);
    h.write_u64(generation);
    h.write_u32(size.0);
    h.write_u32(size.1);
    h.finish()
}

fn is_exotic(p: &Panel) -> bool {
    matches!(p.params[1].floor() as u32, 9 | 10 | 12 | 13 | 14 | 15)
}

/// Intersect a scissor rectangle with the redrawn region (None = the whole frame): `None` when nothing is left.
/// What is left of scissor rect `sc` of item `idx` once the opaque parts of the items above it are cut away (at most a
/// few pieces: past that the rest is simply drawn, which is never wrong).
fn uncovered(sc: (u32, u32, u32, u32), opaque: &[(usize, (u32, u32, u32, u32))], idx: usize) -> Vec<(u32, u32, u32, u32)> {
    let mut parts = vec![sc];
    for &(owner, o) in opaque {
        if owner <= idx || parts.len() > 16 {
            continue;
        }
        parts = parts.into_iter().flat_map(|p| cut(p, o)).collect();
    }
    parts
}

/// Rect `p` less rect `o` (x, y, w, h): up to four pieces.
fn cut(p: (u32, u32, u32, u32), o: (u32, u32, u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    let (ox0, oy0, ox1, oy1) = (o.0, o.1, o.0 + o.2, o.1 + o.3);
    let (x0, y0, x1, y1) = (p.0, p.1, p.0 + p.2, p.1 + p.3);
    if ox1 <= x0 || ox0 >= x1 || oy1 <= y0 || oy0 >= y1 {
        return vec![p];
    }
    // the bands above and below the cover, then left and right of it between them
    let mut out = Vec::with_capacity(4);
    let (my0, my1) = (y0.max(oy0), y1.min(oy1));
    if oy0 > y0 {
        out.push((x0, y0, x1 - x0, oy0 - y0));
    }
    if oy1 < y1 {
        out.push((x0, oy1, x1 - x0, y1 - oy1));
    }
    if ox0 > x0 {
        out.push((x0, my0, ox0 - x0, my1 - my0));
    }
    if ox1 < x1 {
        out.push((ox1, my0, x1 - ox1, my1 - my0));
    }
    out
}

/// The box around two rects (x, y, w, h).
fn bound(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
    (x0, y0, (a.0 + a.2).max(b.0 + b.2) - x0, (a.1 + a.3).max(b.1 + b.3) - y0)
}

fn clip_to(a: (u32, u32, u32, u32), region: Option<(u32, u32, u32, u32)>) -> Option<(u32, u32, u32, u32)> {
    let Some(b) = region else { return Some(a) };
    let (x0, y0) = (a.0.max(b.0), a.1.max(b.1));
    let (x1, y1) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some((x0, y0, x1 - x0, y1 - y0))
    }
}

fn panel_reach(p: &Panel) -> f32 {
    let kind = p.params[1] as u32;
    if matches!(kind, 5 | 6 | 7 | 9 | 10) {
        2.0
    } else {
        (p.extra[1] * 1.6 + p.extra[2].abs()).max(2.0)
    }
}

pub fn panel_box(p: &Panel) -> [f32; 4] {
    let r = panel_reach(p) + 1.0;
    [p.rect[0] - p.rect[2] - r, p.rect[1] - p.rect[3] - r, p.rect[0] + p.rect[2] + r, p.rect[1] + p.rect[3] + r]
}

fn boxes_touch(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Split panels (drawn in order) into groups whose bounding boxes do not overlap, keeping each group's
/// panels in their original order. Groups that overlap are merged, so the result is equivalent to one big draw.
fn cluster_panels(panels: &[Panel], screen: [f32; 2], max_groups: usize) -> Vec<(Vec<Panel>, [f32; 4])> {
    // (first index, boxes, panels with their index)
    let mut groups: Vec<([f32; 4], Vec<(usize, Panel)>)> = Vec::new();
    for (i, p) in panels.iter().enumerate() {
        let mut b = panel_box(p);
        let mut members = vec![(i, *p)];
        // merge every group this panel touches (and any groups the merged box now touches)
        loop {
            let hit = groups.iter().position(|(gb, _)| boxes_touch(gb, &b));
            let Some(k) = hit else { break };
            let (gb, gm) = groups.swap_remove(k);
            b = [b[0].min(gb[0]), b[1].min(gb[1]), b[2].max(gb[2]), b[3].max(gb[3])];
            members.extend(gm);
        }
        groups.push((b, members));
    }
    let mut out: Vec<(Vec<Panel>, [f32; 4])> = groups
        .into_iter()
        .map(|(b, mut m)| {
            m.sort_by_key(|(i, _)| *i);
            (m.into_iter().map(|(_, p)| p).collect(), [b[0].max(0.0), b[1].max(0.0), b[2].min(screen[0]), b[3].min(screen[1])])
        })
        .collect();
    if out.len() > max_groups {
        // too many islands: fall back to a single group over everything
        return vec![(panels.to_vec(), [0.0, 0.0, screen[0], screen[1]])];
    }
    out.retain(|(p, b)| !p.is_empty() && b[2] > b[0] && b[3] > b[1]);
    out
}

/// Render one frame into an offscreen texture of the renderer's target format and return RGBA8 pixels.
/// `MEMRIUM_SKIP` (a bench switch), read once: the render path asks for it every frame
fn skip_flags() -> &'static str {
    static SKIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SKIP.get_or_init(|| std::env::var("MEMRIUM_SKIP").unwrap_or_default())
}

pub fn render_to_rgba(r: &mut Renderer, w: u32, h: u32, items: &[Item], shell: &[Panel]) -> Vec<u8> {
    let format = r.format;
    let tex = r.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let mut enc = r.render(&view, items, shell);
    // rows in a texture->buffer copy must be 256-byte aligned
    let padded = (w * 4 + 255) / 256 * 256;
    let buf = r.device().create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    r.queue().submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |x| x.unwrap());
    r.device().poll(wgpu::PollType::Wait).unwrap();
    let mapped = slice.get_mapped_range();
    let bgra = matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let line = &mapped[row * padded as usize..row * padded as usize + (w * 4) as usize];
        if bgra {
            for px in line.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(line);
        }
    }
    out
}

/// The kind of a glass panel (0) with its fraction packed as `sharpness * 7` (3 bits) and `transparency * 63` (6 bits):
/// the shader reads both from the one number (see `glass_code` in `glass.wgsl`).
pub fn glass_code(sharp: f32, transparent: f32) -> f32 {
    let sq = (sharp.clamp(0.0, 1.0) * 7.0).round() as u32;
    let n = (transparent.clamp(0.0, 1.0) * 63.0).round() as u32;
    0.49 * ((sq << 6) | n) as f32 / 512.0
}

fn glass_idx(kind: f32) -> u32 {
    ((kind.max(0.0) / 0.49 * 512.0).round() as u32).min(511)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chebrium::draw::Draw;

    #[test]
    fn cull_cuts_a_row_on_the_edge_and_drops_one_outside() {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        let mut d = PanelDraw::new(&mut p, &mut l);
        let from = d.mark();
        d.fill([10.0, 80.0, 100.0, 40.0], 4.0, [1.0; 3], 1.0); // crosses the bottom edge at 100
        d.fill([10.0, 20.0, 100.0, 40.0], 4.0, [1.0; 3], 1.0); // inside
        d.fill([10.0, 300.0, 100.0, 40.0], 4.0, [1.0; 3], 1.0); // outside
        d.text(10.0, 95.0, "edge", 14.0, false, [1.0; 3], 1.0, Align::Left);
        d.cull(from, 0.0, 100.0);
        assert_eq!(p.len(), 2);
        assert!(p[0].clip[2] > 0.0 && p[1].clip[2] == 0.0);
        assert_eq!(p[0].clip[1] + p[0].clip[3], 100.0);
        assert!(l[0].clip.is_some());
    }

    #[test]
    fn a_pushed_clip_reaches_what_is_drawn_inside_it() {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        let mut d = PanelDraw::new(&mut p, &mut l);
        d.push_clip([0.0, 0.0, 50.0, 50.0]);
        d.push_clip([25.0, 25.0, 100.0, 100.0]);
        d.fill([0.0, 0.0, 200.0, 200.0], 0.0, [1.0; 3], 1.0);
        d.pop_clip();
        d.pop_clip();
        d.fill([0.0, 0.0, 200.0, 200.0], 0.0, [1.0; 3], 1.0);
        assert_eq!(p[0].clip, [37.5, 37.5, 12.5, 12.5]);
        assert_eq!(p[1].clip[2], 0.0);
    }
}
