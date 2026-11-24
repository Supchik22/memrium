//! Window decorations drawn by the compositor, per theme: macOS traffic lights, Fluent flat
//! titlebar, Aero glass frame. Pure functions: geometry, hit rectangles and panels.

use crate::renderer::{Align, Label, Panel};
use crate::theme::{BarMaterial, BodyTone, ButtonColors, ButtonOrder, ButtonShape, ButtonSide, DecoKind, FrameStyle, GlyphShow, MarkSet, MaxControls, Theme, TitleAlign};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Close,
    Minimize,
    Maximize,
}

/// Space the decoration takes around the client area: (left, top, right, bottom).
pub fn insets(theme: &Theme) -> [f32; 4] {
    let tb = theme.window.titlebar;
    match theme.window.decoration {
        DecoKind::Mac | DecoKind::Win11 | DecoKind::Gnome => [0.0, tb, 0.0, 0.0],
        DecoKind::Aero => [8.0, tb, 8.0, 8.0],
        DecoKind::Minimal => [0.0, 0.0, 0.0, 0.0],
        DecoKind::Strip => [0.0, theme.window.max_marks.strip, 0.0, 0.0],
        DecoKind::Custom => {
            let b = theme.window.frame.border;
            [b, tb, b, b]
        }
    }
}

/// Insets of a maximized (or snapped) window: what is left of the title bar when it is left out, else the usual ones.
pub fn max_insets(theme: &Theme) -> [f32; 4] {
    if !theme.window.hide_title_max {
        insets(theme)
    } else if theme.window.max_controls == MaxControls::Strip {
        [0.0, theme.window.max_marks.strip, 0.0, 0.0]
    } else {
        [0.0; 4]
    }
}

/// The frame a maximized window has when its title bar is left out: the strip, or nothing.
pub fn bare_kind(theme: &Theme) -> DecoKind {
    if theme.window.max_controls == MaxControls::Strip {
        DecoKind::Strip
    } else {
        DecoKind::Minimal
    }
}

/// Width of a button of a custom frame (a pill is wider than tall).
fn button_w(fs: &FrameStyle) -> f32 {
    if fs.shape == ButtonShape::Pill {
        fs.button * 1.7
    } else {
        fs.button
    }
}

// ------------------------------------------------------------ the corner and the strip of a maximized window

/// The buttons of the corner or the strip, from the edge inwards.
fn mark_set(set: MarkSet) -> &'static [Btn] {
    match set {
        MarkSet::Close => &[Btn::Close],
        MarkSet::CloseMin => &[Btn::Close, Btn::Minimize],
        MarkSet::All => &[Btn::Close, Btn::Minimize, Btn::Maximize],
    }
}

/// Width and height of one mark of size `s`.
fn mark_wh(shape: ButtonShape, s: f32) -> (f32, f32) {
    match shape {
        ButtonShape::Pill => (s * 1.7, s),
        ButtonShape::Bar => (s * 2.2, (s * 0.42).max(3.0)),
        _ => (s, s),
    }
}

const MARK_PAD: f32 = 7.0;

/// The square in the window's corner that brings the buttons out (`MaxControls::Corner`).
pub fn corner_zone(theme: &Theme, frame: [f32; 4]) -> [f32; 4] {
    let mm = &theme.window.max_marks;
    let c = mm.corner.max(1.0);
    let x = if mm.side == ButtonSide::Left { frame[0] } else { frame[0] + frame[2] - c };
    [x, frame[1], c, c]
}

/// The plate the corner's buttons sit on, fully out (`k` = 1) or on its way (`k` < 1): while the pointer is over it, the
/// buttons stay out.
pub fn corner_plate(theme: &Theme, frame: [f32; 4], k: f32) -> [f32; 4] {
    let mm = &theme.window.max_marks;
    let set = mark_set(mm.set);
    let (mw, mh) = mark_wh(mm.shape, mm.size);
    let gap = (mm.size * 0.55).max(4.0);
    let (pw, ph) = ((set.len() as f32 * mw + (set.len() - 1) as f32 * gap + 2.0 * MARK_PAD) * k, (mh.max(mm.size) + 2.0 * MARK_PAD) * k);
    let x = if mm.side == ButtonSide::Left { frame[0] } else { frame[0] + frame[2] - pw };
    [x, frame[1], pw, ph]
}

/// Where the marks are, as (button, mark rectangle, rectangle a click hits). `k`: how far the corner's buttons are out
/// (the strip's are always there).
fn mark_layout(theme: &Theme, frame: [f32; 4], k: f32) -> Vec<(Btn, [f32; 4], [f32; 4])> {
    let ws = &theme.window;
    let mm = &ws.max_marks;
    let set = mark_set(mm.set);
    let (mw, mh) = mark_wh(mm.shape, mm.size);
    let gap = (mm.size * 0.55).max(4.0);
    let left = mm.side == ButtonSide::Left;
    let [x, y, w, _] = frame;
    let mut out = Vec::new();
    match ws.max_controls {
        MaxControls::Strip => {
            // on the strip, or hanging from it on a tab when they are taller than it
            let tab = ws.max_marks.strip.max(mh.max(mm.size) + 4.0);
            let cy = y + tab / 2.0;
            for (i, b) in set.iter().enumerate() {
                let off = 10.0 + i as f32 * (mw + gap);
                let mx = if left { x + off } else { x + w - off - mw };
                let r = [mx, cy - mh / 2.0, mw, mh];
                // (a click hits from the top edge of the window down, a little round the mark)
                let hit = [mx - gap / 2.0, y, mw + gap, tab + 2.0];
                out.push((*b, r, hit));
            }
        }
        MaxControls::Corner if k > 0.02 => {
            let corner_x = if left { x } else { x + w };
            let dir = if left { 1.0 } else { -1.0 };
            let cy = y + (MARK_PAD + mh.max(mm.size) / 2.0) * k;
            for (i, b) in set.iter().enumerate() {
                let off = (MARK_PAD + i as f32 * (mw + gap) + mw / 2.0) * k;
                let cx = corner_x + dir * off;
                let r = [cx - mw * k / 2.0, cy - mh * k / 2.0, mw * k, mh * k];
                let hit = [cx - (mw + gap) * k / 2.0, y, (mw + gap) * k, (MARK_PAD * 2.0 + mh.max(mm.size)) * k];
                out.push((*b, r, hit));
            }
        }
        _ => {}
    }
    out
}

/// The buttons of the corner or the strip, as rectangles a click hits.
pub fn mark_buttons(theme: &Theme, frame: [f32; 4], k: f32) -> Vec<(Btn, [f32; 4])> {
    mark_layout(theme, frame, k).into_iter().map(|(b, _, hit)| (b, hit)).collect()
}

/// Colour of the strip of a maximized window.
fn strip_colour(theme: &Theme) -> [f32; 3] {
    if theme.dark {
        [0.17, 0.17, 0.19]
    } else {
        [0.93, 0.93, 0.95]
    }
}

/// The marks of the corner or the strip, drawn over the program's picture (with the tab they hang on, or the corner's plate
/// of glass). `k`: how far the corner's buttons are out.
pub fn mark_panels(theme: &Theme, frame: [f32; 4], focused: bool, hover: Option<Btn>, k: f32) -> Vec<Panel> {
    let ws = &theme.window;
    let mm = &ws.max_marks;
    let layout = mark_layout(theme, frame, k);
    let mut out = Vec::new();
    if layout.is_empty() {
        return out;
    }
    let [x, y, w, h] = frame;
    let (mw, mh) = mark_wh(mm.shape, mm.size);
    let bg;
    match ws.max_controls {
        MaxControls::Strip => {
            bg = strip_colour(theme);
            let tab = mm.strip.max(mh.max(mm.size) + 4.0);
            if tab > mm.strip + 0.5 {
                // a tab of the strip's colour under the marks, round at the bottom
                let x0 = layout.iter().map(|(_, r, _)| r[0]).fold(f32::MAX, f32::min) - 5.0;
                let x1 = layout.iter().map(|(_, r, _)| r[0] + r[2]).fold(f32::MIN, f32::max) + 5.0;
                let rad = tab / 2.0;
                out.push(Panel::solid((x0 + x1) / 2.0, y + tab / 2.0, x1 - x0, tab, rad, bg, bg).flat().shadow(0.18, 6.0, 1.5));
                out.push(Panel::solid((x0 + x1) / 2.0, y + rad / 2.0, x1 - x0, rad, 0.0, bg, bg).flat());
            }
        }
        _ => {
            bg = if theme.dark { [0.12, 0.12, 0.14] } else { [0.97, 0.97, 0.98] };
            if mm.plate {
                // a plate of glass coming out of the corner: square in the corner, round elsewhere (drawn bigger, cut back)
                let p = corner_plate(theme, frame, k);
                let r = p[3] / 2.0;
                let left = mm.side == ButtonSide::Left;
                let (ex, ew) = if left { (p[0] - r, p[2] + r) } else { (p[0], p[2] + r) };
                let (ey, eh) = (p[1] - r, p[3] + r);
                let g = theme.glass.panel(ex + ew / 2.0, ey + eh / 2.0, ew, eh, r).shadow(0.22 * k, 14.0, 3.0);
                out.push(g.cut_to([x, y, w, h]).faded(k.clamp(0.0, 1.0)));
            }
        }
    }
    let any_hover = hover.is_some();
    let show_glyphs = match mm.glyphs {
        GlyphShow::Always => true,
        GlyphShow::Hover => any_hover && focused,
        GlyphShow::Never => false,
    };
    let ink_c = if theme.dark { [0.92, 0.93, 0.96] } else { [0.12, 0.13, 0.16] };
    let alpha = if ws.max_controls == MaxControls::Corner { k.clamp(0.0, 1.0) } else { 1.0 };
    for (b, r, _) in layout {
        let is_h = hover == Some(b);
        let grow = if is_h && mm.grow { 1.25 } else { 1.0 };
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let (mut pw, mut ph) = (r[2] * grow, r[3] * grow);
        let close = b == Btn::Close;
        let grey = if theme.dark { [0.45, 0.45, 0.48] } else { [0.70, 0.70, 0.73] };
        let (plate, glyph) = match mm.colors {
            ButtonColors::Traffic => (
                if !focused {
                    grey
                } else {
                    match b {
                        Btn::Close => [1.0, 0.37, 0.34],
                        Btn::Minimize => [1.0, 0.75, 0.18],
                        Btn::Maximize => [0.16, 0.80, 0.26],
                    }
                },
                [0.25, 0.08, 0.06],
            ),
            ButtonColors::Accent => (theme.accent, [1.0; 3]),
            ButtonColors::Mono => (ink_c, bg),
            ButtonColors::CloseRed => (if close { [0.90, 0.26, 0.22] } else { grey }, [1.0; 3]),
        };
        // a dot until the pointer is near
        let dot = matches!(mm.shape, ButtonShape::Dot | ButtonShape::Flat) && !any_hover;
        if dot {
            pw *= 0.45;
            ph *= 0.45;
        }
        let rad = match mm.shape {
            ButtonShape::Circle | ButtonShape::Pill | ButtonShape::Bar | ButtonShape::Dot | ButtonShape::Flat => pw.min(ph) / 2.0,
            ButtonShape::Rounded => pw.min(ph) * 0.28,
            ButtonShape::Square => (pw.min(ph) * 0.12).min(2.0),
        };
        let hi = [plate[0] * 0.8 + 0.2, plate[1] * 0.8 + 0.2, plate[2] * 0.8 + 0.2];
        out.push(Panel::solid(cx, cy, pw, ph, rad, hi, plate).flat().alpha(alpha));
        if is_h {
            out.push(Panel::solid(cx, cy, pw, ph, rad, [1.0; 3], [1.0; 3]).flat().alpha(0.18 * alpha));
        }
        if show_glyphs && !dot && mm.shape != ButtonShape::Bar && ph >= 8.5 {
            let name = match b {
                Btn::Close => "x",
                Btn::Minimize => "minus",
                Btn::Maximize => "copy",
            };
            out.push(Panel::glyph(cx, cy, (ph * 0.68).clamp(7.0, 12.0), name, glyph, 0.8 * alpha));
        }
    }
    let _ = (mw, h);
    out
}

/// Is the whole frame a slab of glass with the client in a hole in it (Aero, or a custom frame with a glass bar)?
pub fn glass_frame(theme: &Theme) -> bool {
    theme.window.decoration == DecoKind::Aero || (theme.window.decoration == DecoKind::Custom && theme.window.frame.bar == BarMaterial::Glass)
}

/// Corner radius of the window shape.
pub fn radius(theme: &Theme) -> f32 {
    theme.window.radius
}

/// Clickable button rectangles (x, y, w, h) for a window frame (x, y, w, h).
pub fn buttons(theme: &Theme, frame: [f32; 4]) -> Vec<(Btn, [f32; 4])> {
    let [x, y, w, _] = frame;
    match theme.window.decoration {
        DecoKind::Mac => {
            let tb = theme.window.titlebar;
            [Btn::Close, Btn::Minimize, Btn::Maximize]
                .iter()
                .enumerate()
                .map(|(i, b)| (*b, [x + 26.0 + i as f32 * 21.0 - 1.5, y + tb / 2.0 - 8.0, 16.0, 16.0]))
                .collect()
        }
        DecoKind::Win11 => {
            let h = theme.window.titlebar;
            vec![(Btn::Close, [x + w - 46.0, y, 46.0, h]), (Btn::Maximize, [x + w - 92.0, y, 46.0, h]), (Btn::Minimize, [x + w - 138.0, y, 46.0, h])]
        }
        DecoKind::Gnome => {
            // round 24 px buttons on the right: close, then maximise, then minimise towards the middle
            let tb = theme.window.titlebar;
            [Btn::Close, Btn::Maximize, Btn::Minimize].iter().enumerate().map(|(i, b)| (*b, [x + w - 12.0 - 24.0 - i as f32 * 32.0, y + (tb - 24.0) / 2.0, 24.0, 24.0])).collect()
        }
        DecoKind::Aero => {
            let (bh, top) = (19.0, y + 1.0);
            let x0 = x + w - 6.0 - 103.0;
            vec![(Btn::Minimize, [x0, top, 29.0, bh]), (Btn::Maximize, [x0 + 29.0, top, 27.0, bh]), (Btn::Close, [x0 + 56.0, top, 47.0, bh])]
        }
        DecoKind::Minimal => Vec::new(),
        DecoKind::Strip => mark_buttons(theme, frame, 1.0),
        DecoKind::Custom => custom_buttons(theme, frame),
    }
}

/// The three buttons of a custom frame, from the window's edge inwards in the chosen order.
fn custom_buttons(theme: &Theme, frame: [f32; 4]) -> Vec<(Btn, [f32; 4])> {
    let [x, y, w, _] = frame;
    let fs = &theme.window.frame;
    let tb = theme.window.titlebar;
    let order = match fs.order {
        ButtonOrder::CloseMinMax => [Btn::Close, Btn::Minimize, Btn::Maximize],
        ButtonOrder::MinMaxClose => [Btn::Minimize, Btn::Maximize, Btn::Close],
        ButtonOrder::CloseMaxMin => [Btn::Close, Btn::Maximize, Btn::Minimize],
        ButtonOrder::MinCloseMax => [Btn::Minimize, Btn::Close, Btn::Maximize],
    };
    let bar = fs.shape == ButtonShape::Bar;
    let (bw, bh) = if bar { (fs.button, tb) } else { (button_w(fs), fs.button) };
    let margin = if bar { 0.0 } else { 14.0 + fs.border };
    let by = if bar { y } else { y + (tb - bh) / 2.0 + if fs.bar == BarMaterial::Glass { 1.0 } else { 0.0 } };
    order
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let off = margin + i as f32 * (bw + fs.gap);
            let bx = if fs.side == ButtonSide::Left { x + off } else { x + w - off - bw };
            (*b, [bx, by, bw, bh])
        })
        .collect()
}

/// The window buttons of the theme as `appearance.toml` lines, for programs that draw their own header (Chebrium): the
/// geometry comes out of [`buttons`] itself, so a program's buttons sit exactly where the compositor's would.
/// `chrome = false` for a frame without even-sized buttons (Aero, Minimal): those programs go by the decoration name.
pub fn chrome_text(theme: &Theme) -> String {
    let kind = theme.window.decoration;
    let fs = match kind {
        DecoKind::Custom => theme.window.frame,
        DecoKind::Mac | DecoKind::Win11 | DecoKind::Gnome => FrameStyle::from_kind(kind),
        _ => return "chrome = false\n".into(),
    };
    let probe = [0.0, 0.0, 1000.0, theme.window.titlebar];
    let mut bs = buttons(theme, probe);
    if bs.len() != 3 {
        return "chrome = false\n".into();
    }
    let right = bs.iter().map(|(_, r)| r[0] + r[2] / 2.0).sum::<f32>() / 3.0 > 500.0;
    let dist = |r: &[f32; 4]| if right { 1000.0 - (r[0] + r[2]) } else { r[0] };
    bs.sort_by(|a, b| dist(&a.1).total_cmp(&dist(&b.1)));
    let hit = bs[0].1;
    let gap = dist(&bs[1].1) - dist(&hit) - hit[2];
    let name = |b: Btn| match b {
        Btn::Close => "close",
        Btn::Minimize => "minimize",
        Btn::Maximize => "maximize",
    };
    let shape = match fs.shape {
        ButtonShape::Circle => "circle",
        ButtonShape::Square => "square",
        ButtonShape::Rounded => "rounded",
        ButtonShape::Flat => "flat",
        ButtonShape::Bar => "bar",
        ButtonShape::Pill => "pill",
        ButtonShape::Dot => "dot",
    };
    let colors = match fs.colors {
        ButtonColors::Traffic => "traffic",
        ButtonColors::Accent => "accent",
        ButtonColors::Mono => "mono",
        ButtonColors::CloseRed => "close_red",
    };
    let glyphs = match fs.glyphs {
        GlyphShow::Hover => "hover",
        GlyphShow::Always => "always",
        GlyphShow::Never => "never",
    };
    format!(
        "chrome = true\nchrome_side = \"{}\"\nchrome_order = [\"{}\", \"{}\", \"{}\"]\nchrome_shape = \"{shape}\"\nchrome_colors = \"{colors}\"\nchrome_glyphs = \"{glyphs}\"\nchrome_hit = [{:.2}, {:.2}]\nchrome_dot = {:.2}\nchrome_margin = {:.2}\nchrome_gap = {:.2}\n",
        if right { "right" } else { "left" },
        name(bs[0].0),
        name(bs[1].0),
        name(bs[2].0),
        hit[2],
        hit[3],
        fs.button.min(hit[2].max(hit[3])),
        dist(&hit),
        gap
    )
}

/// Horizontal span available for the title text (x0, x1).
pub fn title_span(theme: &Theme, frame: [f32; 4]) -> (f32, f32) {
    let [x, _, w, _] = frame;
    match theme.window.decoration {
        DecoKind::Mac => (x + 96.0, x + w - 96.0),
        DecoKind::Win11 => (x + 14.0, x + w - 150.0),
        DecoKind::Aero => (x + 14.0, x + w - 120.0),
        DecoKind::Gnome => (x + 110.0, x + w - 120.0),
        DecoKind::Minimal | DecoKind::Strip => (x, x),
        DecoKind::Custom => {
            let fs = &theme.window.frame;
            let bar = fs.shape == ButtonShape::Bar;
            let cluster = (if bar { 0.0 } else { 14.0 + fs.border }) + 3.0 * button_w(fs) + 2.0 * fs.gap + 10.0;
            match (theme.window.title_align, fs.side) {
                (TitleAlign::Center, _) => (x + cluster, x + w - cluster),
                (TitleAlign::Left, ButtonSide::Left) => (x + cluster, x + w - 14.0),
                (TitleAlign::Left, ButtonSide::Right) => (x + 14.0, x + w - cluster),
            }
        }
    }
}

fn ink(dark: bool) -> [f32; 3] {
    if dark {
        [0.95, 0.96, 0.98]
    } else {
        [0.10, 0.11, 0.14]
    }
}

/// Title text labels (Aero adds a light "glow" behind the dark text, as the original did).
/// Size of the menu names a program's menus show in its title bar.
pub const TITLE_MENU_PX: f32 = 12.0;

/// Whether this theme's title bar can hold a program's menus: the title sits at the left and the buttons at the right
/// (Fluent, Aero, a custom frame laid out that way).
pub fn title_menus_fit(theme: &Theme) -> bool {
    match theme.window.decoration {
        DecoKind::Win11 | DecoKind::Aero => true,
        DecoKind::Custom => theme.window.title_align == TitleAlign::Left && theme.window.frame.side == ButtonSide::Right,
        _ => false,
    }
}

/// The boxes of a program's menu names along the left of its title bar (`widths`: the width of each name's text). They
/// run from where the title would start and stop when less than `TITLE_KEEP` px would be left for the title and for
/// grabbing the window; a name that does not fit is left out, with all after it.
pub fn title_menu_rects(theme: &Theme, frame: [f32; 4], widths: &[f32]) -> Vec<[f32; 4]> {
    const PAD: f32 = 10.0;
    const TITLE_KEEP: f32 = 120.0;
    let (x0, x1) = title_span(theme, frame);
    let tb = theme.window.titlebar;
    let (h, y) = ((tb - 10.0).max(16.0), frame[1] + 5.0);
    let mut x = x0 - 4.0;
    let mut out = Vec::new();
    for w in widths {
        let bw = w + 2.0 * PAD;
        if x + bw > x1 - TITLE_KEEP {
            break;
        }
        out.push([x, y, bw, h]);
        x += bw + 2.0;
    }
    out
}

/// The names of the menus, as lettering.
pub fn title_menu_labels(theme: &Theme, frame: [f32; 4], names: &[(String, [f32; 4])], focused: bool) -> Vec<Label> {
    let base = frame[1] + theme.window.titlebar / 2.0 + 3.5;
    let (c, a) = (ink(theme.dark), if focused { 0.92 } else { 0.6 });
    names.iter().map(|(n, r)| Label::new(n.clone(), r[0] + 10.0, base, TITLE_MENU_PX, false, c, a, Align::Left)).collect()
}

/// The wash behind the menu name that is open or under the pointer (`lit`: its index).
pub fn title_menu_panels(theme: &Theme, rects: &[[f32; 4]], lit: Option<usize>, open: bool) -> Vec<Panel> {
    let Some(r) = lit.and_then(|i| rects.get(i)) else { return Vec::new() };
    let (c, a) = if theme.dark { ([1.0; 3], if open { 0.16 } else { 0.10 }) } else { ([0.0; 3], if open { 0.10 } else { 0.06 }) };
    vec![Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 5.0, c, c).flat().alpha(a)]
}

pub fn title_labels(theme: &Theme, frame: [f32; 4], title: &str, focused: bool) -> Vec<Label> {
    if title.is_empty() || matches!(theme.window.decoration, DecoKind::Minimal | DecoKind::Strip) {
        return Vec::new();
    }
    let [x, y, w, _] = frame;
    let tb = theme.window.titlebar;
    let (x0, x1) = title_span(theme, frame);
    let _ = (x, w);
    let (ax, align) = match (theme.window.decoration, theme.window.title_align) {
        (_, TitleAlign::Left) | (DecoKind::Win11, _) | (DecoKind::Aero, _) => (x0, Align::Left),
        _ => ((x0 + x1) / 2.0, Align::Center),
    };
    let base = y + tb / 2.0 + 4.5;
    let a = if focused { 0.92 } else { 0.5 };
    match theme.window.decoration {
        DecoKind::Mac => vec![Label::new(title, ax, base, 13.0, true, if theme.dark { [0.90, 0.91, 0.95] } else { [0.22, 0.23, 0.28] }, a, align)],
        DecoKind::Win11 => vec![Label::new(title, ax, base - 1.0, 12.0, false, ink(theme.dark), a, align)],
        DecoKind::Gnome => vec![Label::new(title, ax, base, 13.0, true, ink(theme.dark), a, align)],
        DecoKind::Aero => {
            let mut v = Vec::new();
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.2, 1.2), (-1.2, -1.2)] {
                v.push(Label::new(title, ax + dx, base - 2.0 + dy, 12.0, false, [1.0, 1.0, 1.0], if focused { 0.45 } else { 0.2 }, align));
            }
            v.push(Label::new(title, ax, base - 2.0, 12.0, false, [0.0, 0.0, 0.0], if focused { 0.92 } else { 0.55 }, align));
            v
        }
        DecoKind::Minimal | DecoKind::Strip => Vec::new(),
        DecoKind::Custom => {
            let fs = &theme.window.frame;
            let (on_dark, glow) = bar_ink(theme);
            let ink = if on_dark { [0.96, 0.97, 1.0] } else { [0.10, 0.11, 0.14] };
            let mut v = Vec::new();
            if glow {
                for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.2, 1.2), (-1.2, -1.2)] {
                    v.push(Label::new(title, ax + dx, base - 2.0 + dy, fs.title_size, false, [1.0, 1.0, 1.0], if focused { 0.45 } else { 0.2 }, align));
                }
            }
            v.push(Label::new(title, ax, base, fs.title_size, fs.title_bold, ink, a, align));
            v
        }
    }
}

/// Is the title bar of a custom frame dark (so its ink is light), and does the text need the glass glow?
fn bar_ink(theme: &Theme) -> (bool, bool) {
    let fs = &theme.window.frame;
    match fs.bar {
        BarMaterial::Accent => (true, false),
        BarMaterial::Glass => {
            let g = &theme.glass.tint;
            let dark_glass = g[3] > 0.35 && 0.299 * g[0] + 0.587 * g[1] + 0.114 * g[2] < 0.52;
            (theme.dark || dark_glass, !(theme.dark || dark_glass))
        }
        _ => (body_dark(fs, theme.dark), false),
    }
}

fn body_dark(fs: &FrameStyle, theme_dark: bool) -> bool {
    match fs.body {
        BodyTone::Auto => theme_dark,
        BodyTone::Light => false,
        BodyTone::Dark => true,
    }
}

/// Give a window's body panel (a solid one, already carrying the wide shadow) the theme's tight shadow and, when
/// `outline`, its hairline (with the inner light line when `dark`, the window's look).
pub fn window_edge(theme: &Theme, p: Panel, focused: bool, outline: bool, dark: bool) -> Panel {
    let ws = &theme.window;
    let near = if focused { ws.shadow_near } else { ws.shadow_near_idle };
    let (line, light) = if !outline { (0.0, 0.0) } else if dark { ((ws.outline * 2.0).min(1.0), ws.outline_light) } else { (ws.outline, 0.0) };
    p.window_edge(near, line, light)
}

/// The panel under a window whose program draws its own frame (or a full-screen one: `sh` all zero, nothing drawn):
/// only the shadow, the tight shadow and the hairline round the shape `f` with corner radius `r`.
pub fn client_shadow(theme: &Theme, f: [f32; 4], r: f32, sh: [f32; 3], focused: bool, outline: bool) -> Panel {
    let p = Panel::solid(f[0] + f[2] / 2.0, f[1] + f[3] / 2.0, f[2], f[3], r, [0.97; 3], [0.95; 3]).flat().alpha(0.0).shadow(sh[0], sh[1], sh[2]);
    if sh[0] <= 0.0 && !outline {
        return p;
    }
    window_edge(theme, p, focused, outline, theme.dark)
}

/// Round the corners of a shell window's own content (Files, Settings) the way the window's are: a fill (solid or glass,
/// square) that reaches a corner of `frame` gets that corner cut to the radius `r`. The fill is made bigger on its sides
/// away from the frame's edges and cut back to where it was, so only its corners at the frame's corners round off.
pub fn round_to_frame(panels: &mut [Panel], frame: [f32; 4], r: f32) {
    if r < 0.5 {
        return;
    }
    let [fx0, fy0] = [frame[0], frame[1]];
    let [fx1, fy1] = [frame[0] + frame[2], frame[1] + frame[3]];
    let e = 0.75;
    for p in panels.iter_mut() {
        // glass (0) and solid fills (1) only, and only square ones
        if p.params[1] + 0.5 >= 2.0 || p.params[0].abs() > 0.5 {
            continue;
        }
        let mut b = [p.rect[0] - p.rect[2], p.rect[1] - p.rect[3], p.rect[0] + p.rect[2], p.rect[1] + p.rect[3]];
        if p.clip[2] > 0.0 {
            b = [b[0].max(p.clip[0] - p.clip[2]), b[1].max(p.clip[1] - p.clip[3]), b[2].min(p.clip[0] + p.clip[2]), b[3].min(p.clip[1] + p.clip[3])];
        }
        // (a fill wider than the window counts as far as the window)
        b = [b[0].max(fx0), b[1].max(fy0), b[2].min(fx1), b[3].min(fy1)];
        if b[2] - b[0] < 1.0 || b[3] - b[1] < 1.0 {
            continue;
        }
        let (l, t, rt, bt) = (b[0] <= fx0 + e, b[1] <= fy0 + e, b[2] >= fx1 - e, b[3] >= fy1 - e);
        if !((l || rt) && (t || bt)) {
            continue;
        }
        let k = 2.0 * r;
        let n = [if l { b[0] } else { b[0] - k }, if t { b[1] } else { b[1] - k }, if rt { b[2] } else { b[2] + k }, if bt { b[3] } else { b[3] + k }];
        p.rect = [(n[0] + n[2]) / 2.0, (n[1] + n[3]) / 2.0, (n[2] - n[0]) / 2.0, (n[3] - n[1]) / 2.0];
        p.params[0] = r;
        p.clip = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, (b[2] - b[0]) / 2.0, (b[3] - b[1]) / 2.0];
    }
}

/// Chrome panels (shadow, frame, titlebar, buttons) for a window frame. The client surface is drawn on top.
pub fn frame_panels(theme: &Theme, frame: [f32; 4], focused: bool, maximized: bool, hover: Option<Btn>) -> Vec<Panel> {
    let [x, y, w, h] = frame;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let ws = &theme.window;
    let sh = if focused { ws.shadow } else { ws.shadow_idle };
    let r = if maximized && ws.max_gap < 0.5 { 0.0 } else { ws.radius };
    // a maximized window that fills its area has no hairline round it (it would run along the bars)
    let line = !(maximized && ws.max_gap < 0.5);
    let mut out = Vec::new();
    match ws.decoration {
        DecoKind::Mac => {
            // the title bar follows the light / dark look
            let (top, bot) = if theme.dark { ([0.19, 0.19, 0.21], [0.15, 0.15, 0.17]) } else { ([0.97, 0.97, 0.98], [0.93, 0.93, 0.95]) };
            out.push(window_edge(theme, Panel::solid(cx, cy, w, h, r, top, bot).flat().shadow(sh[0], sh[1], sh[2]), focused, line, theme.dark));
            let tb = ws.titlebar;
            let lights: [[f32; 3]; 3] = if focused { [[1.0, 0.37, 0.34], [1.0, 0.75, 0.18], [0.16, 0.80, 0.26]] } else { [[0.74, 0.74, 0.76]; 3] };
            let glyphs = ["x", "minus", "plus"];
            for (i, c) in lights.iter().enumerate() {
                let hi = [c[0] * 0.8 + 0.2, c[1] * 0.8 + 0.2, c[2] * 0.8 + 0.2];
                let (lx, ly) = (x + 26.0 + i as f32 * 21.0 + 6.5, y + tb / 2.0);
                out.push(Panel::solid(lx, ly, 13.0, 13.0, 6.5, hi, *c).flat());
                if hover.is_some() && focused {
                    out.push(Panel::glyph(lx, ly, 9.0, glyphs[i], [0.25, 0.08, 0.06], 0.7));
                }
            }
        }
        DecoKind::Win11 => {
            let (bg, border) = if theme.dark { ([0.14, 0.15, 0.17], 0.35) } else { ([0.96, 0.97, 0.98], 0.16) };
            out.push(Panel::solid(cx, cy, w + 2.0, h + 2.0, r + 1.0, [0.0; 3], [0.0; 3]).flat().alpha(border).shadow(sh[0], sh[1], sh[2]));
            out.push(Panel::solid(cx, cy, w, h, r, bg, bg).flat());
            for (b, rect) in buttons(theme, frame) {
                let is_h = hover == Some(b);
                let close = b == Btn::Close;
                if is_h {
                    let (c, a) = if close { ([0.77, 0.17, 0.17], 1.0) } else if theme.dark { ([1.0; 3], 0.10) } else { ([0.0; 3], 0.06) };
                    // the close button keeps the window's rounded top-right corner
                    let rr = if close { r } else { 0.0 };
                    out.push(Panel::solid(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, rect[2], rect[3], rr, c, c).flat().alpha(a));
                }
                let name = match b {
                    Btn::Close => "x",
                    Btn::Minimize => "minus",
                    Btn::Maximize => {
                        if maximized {
                            "copy"
                        } else {
                            "square"
                        }
                    }
                };
                let col = if close && is_h { [1.0; 3] } else { ink(theme.dark) };
                out.push(Panel::glyph(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, 11.0, name, col, if focused { 0.9 } else { 0.5 }));
            }
        }
        DecoKind::Gnome => {
            // Adwaita: a light (or dark) window, a slightly different header bar with square lower corners, a hairline
            let (body, head) = if theme.dark { ([0.141, 0.141, 0.141], [0.188, 0.188, 0.188]) } else { ([0.98, 0.98, 0.98], [0.922, 0.922, 0.922]) };
            let tb = ws.titlebar;
            out.push(window_edge(theme, Panel::solid(cx, cy, w, h, r, body, body).flat().shadow(sh[0], sh[1], sh[2]), focused, line, theme.dark));
            let head = if focused { head } else { body };
            // the header bar is rounded at the top only: draw it a corner-radius too tall and put the body over the excess
            out.push(Panel::solid(cx, y + (tb + r) / 2.0, w, tb + r, r, head, head).flat());
            out.push(Panel::solid(cx, y + tb + r / 2.0, w, r, 0.0, body, body).flat());
            out.push(Panel::solid(cx, y + tb - 0.5, w, 1.0, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(if theme.dark { 0.45 } else { 0.13 }));
            for (b, rect) in buttons(theme, frame) {
                let (bx, by) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
                let is_h = hover == Some(b);
                let wash = if theme.dark { [1.0; 3] } else { [0.0; 3] };
                out.push(Panel::solid(bx, by, rect[2], rect[3], 12.0, wash, wash).flat().alpha(if is_h { 0.16 } else { 0.07 }));
                let name = match b {
                    Btn::Close => "x",
                    Btn::Minimize => "minus",
                    Btn::Maximize => {
                        if maximized {
                            "copy"
                        } else {
                            "square"
                        }
                    }
                };
                out.push(Panel::glyph(bx, by, 11.0, name, ink(theme.dark), if focused { 0.9 } else { 0.5 }));
            }
        }
        DecoKind::Aero => {
            // outer dark hairline, then the glass slab, then a thin dark line around the client
            out.push(Panel::solid(cx, cy, w + 2.0, h + 2.0, r + 1.0, [0.0; 3], [0.0; 3]).flat().alpha(0.55).shadow(sh[0], sh[1], sh[2]));
            let mut g = theme.glass.panel(cx, cy, w, h, r);
            g.extra[0] = 0.0; // shadow comes from the hairline panel
            if !focused {
                g.tint[3] *= 0.6;
            }
            out.push(g);
            let ins = insets(theme);
            let client = [x + ins[0], y + ins[1], w - ins[0] - ins[2], h - ins[1] - ins[3]];
            out.push(Panel::solid(client[0] + client[2] / 2.0, client[1] + client[3] / 2.0, client[2] + 2.0, client[3] + 2.0, 1.0, [0.0; 3], [0.0; 3]).flat().alpha(0.5));
            for (b, rect) in buttons(theme, frame) {
                let (c, cy2) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
                let is_h = hover == Some(b);
                let rr = 4.0;
                if b == Btn::Close {
                    let (top, bot) = if is_h { ([0.96, 0.52, 0.38], [0.80, 0.18, 0.10]) } else { ([0.86, 0.42, 0.32], [0.66, 0.14, 0.08]) };
                    out.push(Panel::solid(c, cy2, rect[2], rect[3], rr, top, bot).flat().alpha(if focused { 1.0 } else { 0.6 }));
                    out.push(Panel::glyph(c, cy2, 12.0, "x", [1.0; 3], 1.0));
                } else {
                    let a = if is_h { 0.62 } else { 0.38 };
                    out.push(Panel::solid(c, cy2, rect[2], rect[3], rr, [1.0; 3], [0.78, 0.86, 0.96]).flat().alpha(a));
                    let name = if b == Btn::Minimize {
                        "minus"
                    } else if maximized {
                        "copy"
                    } else {
                        "square"
                    };
                    out.push(Panel::glyph(c, cy2, 11.0, name, [0.05, 0.12, 0.25], 0.9));
                }
            }
        }
        DecoKind::Minimal => {
            out.push(client_shadow(theme, frame, r, sh, focused, line));
        }
        DecoKind::Strip => {
            // the strip along the top (its marks are drawn over the program's picture: `mark_panels`)
            let c = strip_colour(theme);
            out.push(window_edge(theme, Panel::solid(cx, cy, w, h, r, c, c).flat().shadow(sh[0], sh[1], sh[2]), focused, line, theme.dark));
        }
        DecoKind::Custom => custom_panels(theme, frame, focused, maximized, hover, &mut out),
    }
    out
}

/// A frame built from the parts of `WindowStyle::frame`.
fn custom_panels(theme: &Theme, frame: [f32; 4], focused: bool, maximized: bool, hover: Option<Btn>, out: &mut Vec<Panel>) {
    let [x, y, w, h] = frame;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let ws = &theme.window;
    let fs = &ws.frame;
    let sh = if focused { ws.shadow } else { ws.shadow_idle };
    let r = if maximized && ws.max_gap < 0.5 { 0.0 } else { ws.radius };
    let line = !(maximized && ws.max_gap < 0.5);
    let tb = ws.titlebar;
    let dark = body_dark(fs, theme.dark);
    let (body, tone) = if dark { ([0.141, 0.145, 0.165], [0.19, 0.195, 0.215]) } else { ([0.975, 0.977, 0.985], [0.92, 0.924, 0.935]) };
    let (bar_dark, _) = bar_ink(theme);
    let ink_c = if bar_dark { [0.96, 0.97, 1.0] } else { [0.10, 0.11, 0.14] };
    if fs.bar == BarMaterial::Glass {
        // a slab of glass with the client in a hole in it
        out.push(Panel::solid(cx, cy, w + 2.0, h + 2.0, r + 1.0, [0.0; 3], [0.0; 3]).flat().alpha(0.55).shadow(sh[0], sh[1], sh[2]));
        let mut g = theme.glass.panel(cx, cy, w, h, r);
        g.extra[0] = 0.0;
        if !focused {
            g.tint[3] *= 0.6;
        }
        out.push(g);
        let ins = insets(theme);
        let client = [x + ins[0], y + ins[1], w - ins[0] - ins[2], h - ins[1] - ins[3]];
        out.push(Panel::solid(client[0] + client[2] / 2.0, client[1] + client[3] / 2.0, client[2] + 2.0, client[3] + 2.0, 1.0, [0.0; 3], [0.0; 3]).flat().alpha(0.5));
    } else {
        // the body carries the shadows and the hairline (drawn in the band round its edge, no extra panel over the window)
        out.push(window_edge(theme, Panel::solid(cx, cy, w, h, r, body, body).flat().shadow(sh[0], sh[1], sh[2]), focused, fs.outline && line, dark));
        let head = match fs.bar {
            BarMaterial::Tone => tone,
            BarMaterial::Accent => theme.accent,
            _ => body,
        };
        if fs.bar != BarMaterial::Body {
            let head = if focused { head } else if fs.bar == BarMaterial::Accent { [head[0] * 0.55 + 0.35, head[1] * 0.55 + 0.35, head[2] * 0.55 + 0.35] } else { body };
            // the bar is rounded at the top only: draw it a corner-radius too tall and put the body over the excess
            out.push(Panel::solid(cx, y + (tb + r) / 2.0, w, tb + r, r, head, head).flat());
            out.push(Panel::solid(cx, y + tb + r / 2.0, w, r, 0.0, body, body).flat());
            out.push(Panel::solid(cx, y + tb - 0.5, w, 1.0, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(if dark { 0.45 } else { 0.13 }));
        }
    }
    // the buttons
    let show_glyphs = match fs.glyphs {
        GlyphShow::Always => true,
        GlyphShow::Hover => hover.is_some() && focused,
        GlyphShow::Never => false,
    };
    let plate_always = !matches!(fs.shape, ButtonShape::Flat | ButtonShape::Bar);
    for (b, rect) in custom_buttons(theme, frame) {
        let (bx, by) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        let is_h = hover == Some(b);
        let close = b == Btn::Close;
        // a dot grows into the button while the pointer is over the buttons
        let rect = if fs.shape == ButtonShape::Dot && hover.is_none() {
            let s = rect[2] * 0.42;
            [bx - s / 2.0, by - s / 2.0, s, s]
        } else {
            rect
        };
        let rad = match fs.shape {
            ButtonShape::Circle | ButtonShape::Pill | ButtonShape::Dot => rect[2].min(rect[3]) / 2.0,
            ButtonShape::Rounded => rect[2].min(rect[3]) * 0.28,
            ButtonShape::Square => 3.0,
            // a close plate keeps the window's rounded corner when it sits at the edge
            ButtonShape::Flat => rect[2].min(rect[3]) * 0.2,
            ButtonShape::Bar => if close && fs.side == ButtonSide::Right { r } else { 0.0 },
        };
        let red = [0.86, 0.30, 0.25];
        // (plate colour, plate opacity, glyph colour)
        let (plate, alpha, glyph) = match fs.colors {
            ButtonColors::Traffic => {
                let c = if !focused {
                    [0.74, 0.74, 0.76]
                } else {
                    match b {
                        Btn::Close => [1.0, 0.37, 0.34],
                        Btn::Minimize => [1.0, 0.75, 0.18],
                        Btn::Maximize => [0.16, 0.80, 0.26],
                    }
                };
                (c, 1.0, [0.25, 0.08, 0.06])
            }
            ButtonColors::Accent => (if focused { theme.accent } else { [0.74, 0.74, 0.76] }, 1.0, [1.0; 3]),
            ButtonColors::Mono => (ink_c, if is_h { 0.20 } else { 0.10 }, ink_c),
            ButtonColors::CloseRed => {
                if close {
                    (red, if focused { 1.0 } else { 0.6 }, [1.0; 3])
                } else {
                    (ink_c, if is_h { 0.20 } else { 0.10 }, ink_c)
                }
            }
        };
        if plate_always {
            out.push(Panel::solid(bx, by, rect[2], rect[3], rad, plate, plate).flat().alpha(alpha));
            if is_h {
                out.push(Panel::solid(bx, by, rect[2], rect[3], rad, [1.0; 3], [1.0; 3]).flat().alpha(0.18));
            }
        } else if is_h {
            let (c, a) = if close { (red, 0.95) } else { (ink_c, 0.12) };
            out.push(Panel::solid(bx, by, rect[2], rect[3], rad, c, c).flat().alpha(a));
        }
        let hover_glyph = if plate_always { glyph } else if close && is_h { [1.0; 3] } else { ink_c };
        let name = match b {
            Btn::Close => "x",
            Btn::Minimize => "minus",
            Btn::Maximize => {
                if fs.colors == ButtonColors::Traffic && fs.shape == ButtonShape::Circle {
                    "plus"
                } else if maximized {
                    "copy"
                } else {
                    "square"
                }
            }
        };
        let shrunk = fs.shape == ButtonShape::Dot && hover.is_none();
        if (show_glyphs || (!plate_always && matches!(fs.glyphs, GlyphShow::Always))) && !shrunk {
            let size = (rect[2].min(rect[3]) * 0.6).clamp(8.0, 12.0);
            out.push(Panel::glyph(bx, by, size, name, hover_glyph, if focused { 0.9 } else { 0.5 }));
        }
    }
}

/// Take the window buttons out of `panels` (from `frame_panels` for the same theme and frame) and return them, for a
/// window whose own content covers the title bar (the shell's unified toolbar): the content goes over the frame, the
/// buttons over the content. A button's panels are the ones that sit inside its rectangle; the first panel (the
/// window's body) always stays.
pub fn take_buttons(theme: &Theme, frame: [f32; 4], panels: &mut Vec<Panel>) -> Vec<Panel> {
    let rects = buttons(theme, frame);
    let on_button = |p: &Panel| {
        let [cx, cy, hw, hh] = p.rect;
        rects.iter().any(|(_, r)| cx - hw >= r[0] - 1.5 && cx + hw <= r[0] + r[2] + 1.5 && cy - hh >= r[1] - 1.5 && cy + hh <= r[1] + r[3] + 1.5)
    };
    let mut out = Vec::new();
    let mut keep = Vec::with_capacity(panels.len());
    for (i, p) in panels.drain(..).enumerate() {
        if i > 0 && on_button(&p) {
            out.push(p);
        } else {
            keep.push(p);
        }
    }
    *panels = keep;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(deco: DecoKind) -> Theme {
        let mut t = Theme::default();
        t.window.decoration = deco;
        t.window.titlebar = 36.0;
        t
    }

    fn bare(controls: MaxControls) -> Theme {
        let mut t = Theme::default();
        t.window.hide_title_max = true;
        t.window.max_controls = controls;
        t
    }

    #[test]
    fn the_strip_keeps_a_few_pixels_and_its_marks_are_buttons() {
        let mut t = bare(MaxControls::Strip);
        t.window.max_marks.strip = 3.0;
        t.window.max_marks.set = MarkSet::All;
        assert_eq!(max_insets(&t), [0.0, 3.0, 0.0, 0.0]);
        assert_eq!(bare_kind(&t), DecoKind::Strip);
        t.window.decoration = DecoKind::Strip;
        let frame = [0.0, 30.0, 1200.0, 800.0];
        let b = buttons(&t, frame);
        assert_eq!(b.iter().map(|(b, _)| *b).collect::<Vec<_>>(), vec![Btn::Close, Btn::Minimize, Btn::Maximize]);
        // they start at the window's top edge (a mark taller than the strip hangs from it) and stay at the left
        assert!(b.iter().all(|(_, r)| r[1] == frame[1] && r[0] < 120.0));
        // no title on a strip
        assert!(title_labels(&t, frame, "Files", true).is_empty());
        // the right side
        t.window.max_marks.side = ButtonSide::Right;
        assert!(buttons(&t, frame).iter().all(|(_, r)| r[0] > 1000.0));
        // the marks are drawn
        assert!(!mark_panels(&t, frame, true, Some(Btn::Close), 1.0).is_empty());
    }

    #[test]
    fn the_corner_opens_only_when_asked_and_from_its_corner() {
        let t = bare(MaxControls::Corner);
        assert_eq!(max_insets(&t), [0.0; 4]);
        assert_eq!(bare_kind(&t), DecoKind::Minimal);
        let frame = [100.0, 30.0, 1200.0, 800.0];
        assert!(mark_buttons(&t, frame, 0.0).is_empty());
        assert!(mark_panels(&t, frame, true, None, 0.0).is_empty());
        let z = corner_zone(&t, frame);
        assert_eq!([z[0], z[1]], [frame[0], frame[1]]);
        let b = mark_buttons(&t, frame, 1.0);
        assert_eq!(b.len(), 1);
        let p = corner_plate(&t, frame, 1.0);
        assert!(b[0].1[0] >= p[0] && b[0].1[0] + b[0].1[2] <= p[0] + p[2] + 1.0);
        // half way out: smaller
        assert!(corner_plate(&t, frame, 0.5)[2] < p[2]);
        // the menu-bar mode has no marks
        assert!(mark_buttons(&bare(MaxControls::MenuBar), frame, 1.0).is_empty());
    }

    #[test]
    fn pill_buttons_are_wider_and_chebrium_knows_the_new_shapes() {
        use chebrium::theme::{Appearance, ChromeShape};
        let mut t = with(DecoKind::Custom);
        t.window.frame.shape = ButtonShape::Pill;
        let b = buttons(&t, [0.0, 0.0, 1000.0, 36.0]);
        assert!(b[0].1[2] > b[0].1[3] * 1.5);
        assert_eq!(Appearance::parse(&chrome_text(&t)).chrome.expect("chrome").shape, ChromeShape::Pill);
        t.window.frame.shape = ButtonShape::Dot;
        assert_eq!(Appearance::parse(&chrome_text(&t)).chrome.expect("chrome").shape, ChromeShape::Dot);
        // a dot is small until the pointer is over the buttons
        let small = frame_panels(&t, [0.0, 0.0, 1000.0, 600.0], true, false, None);
        let big = frame_panels(&t, [0.0, 0.0, 1000.0, 600.0], true, false, Some(Btn::Close));
        let widest = |v: &[Panel]| v.iter().skip(1).map(|p| p.rect[2]).fold(0.0f32, f32::max);
        assert!(widest(&big) > widest(&small));
    }

    #[test]
    fn only_a_title_bar_with_its_title_at_the_left_holds_menus() {
        assert!(title_menus_fit(&with(DecoKind::Win11)));
        assert!(title_menus_fit(&with(DecoKind::Aero)));
        assert!(!title_menus_fit(&with(DecoKind::Mac)));
        assert!(!title_menus_fit(&with(DecoKind::Gnome)));
    }

    #[test]
    fn the_menu_boxes_run_from_the_left_and_leave_room_for_the_title() {
        let t = with(DecoKind::Win11);
        let frame = [100.0, 50.0, 900.0, 500.0];
        let r = title_menu_rects(&t, frame, &[30.0, 30.0, 40.0]);
        assert_eq!(r.len(), 3);
        assert!(r[0][0] < r[1][0] && r[1][0] < r[2][0]);
        assert!(r[2][0] + r[2][2] <= title_span(&t, frame).1 - 120.0);
        // all inside the title bar
        assert!(r.iter().all(|b| b[1] >= frame[1] && b[1] + b[3] <= frame[1] + t.window.titlebar));
        // a narrow window shows the ones that fit
        let narrow = title_menu_rects(&t, [0.0, 0.0, 420.0, 300.0], &[40.0; 10]);
        assert!(!narrow.is_empty() && narrow.len() < 10);
        // and none at all when nothing fits
        assert!(title_menu_rects(&t, [0.0, 0.0, 200.0, 300.0], &[40.0; 3]).is_empty());
    }

    #[test]
    fn the_wash_is_for_the_lit_name_only() {
        let t = with(DecoKind::Win11);
        let rects = [[0.0, 0.0, 40.0, 26.0], [42.0, 0.0, 40.0, 26.0]];
        assert!(title_menu_panels(&t, &rects, None, false).is_empty());
        assert_eq!(title_menu_panels(&t, &rects, Some(1), true).len(), 1);
        assert!(title_menu_panels(&t, &rects, Some(5), true).is_empty());
    }

    #[test]
    fn the_buttons_come_out_of_the_frame_and_the_body_stays() {
        let frame = [100.0, 50.0, 900.0, 500.0];
        for deco in [DecoKind::Mac, DecoKind::Win11, DecoKind::Gnome, DecoKind::Custom] {
            let t = with(deco);
            let mut p = frame_panels(&t, frame, true, false, Some(Btn::Close));
            let all = p.len();
            let b = take_buttons(&t, frame, &mut p);
            assert!(!b.is_empty(), "{deco:?}");
            assert_eq!(p.len() + b.len(), all, "{deco:?}");
            // the window's body (with its shadow) is not a button
            assert!(p[0].rect[2] * 2.0 >= frame[2] - 1.0, "{deco:?}");
        }
    }

    #[test]
    fn chebrium_reads_the_buttons_the_theme_lays_out() {
        use chebrium::theme::{Appearance, ChromeShape, WinBtn};
        for deco in [DecoKind::Mac, DecoKind::Win11, DecoKind::Gnome, DecoKind::Custom] {
            let t = with(deco);
            let a = Appearance::parse(&format!("{}", chrome_text(&t)));
            let c = a.chrome.expect("chrome");
            // the first button of the chebrium layout sits where the compositor puts its first one
            let frame = [0.0, 0.0, 1000.0, t.window.titlebar];
            let mut bs = buttons(&t, frame);
            bs.sort_by(|a, b| if c.right { b.1[0].total_cmp(&a.1[0]) } else { a.1[0].total_cmp(&b.1[0]) });
            let first = bs[0].1;
            let edge = if c.right { 1000.0 - first[0] - first[2] } else { first[0] };
            assert!((edge - c.margin).abs() < 0.01, "{deco:?}");
            assert_eq!(c.shape == ChromeShape::Bar, deco == DecoKind::Win11);
            assert_eq!(c.order[0], match bs[0].0 { Btn::Close => WinBtn::Close, Btn::Minimize => WinBtn::Minimize, Btn::Maximize => WinBtn::Maximize });
        }
        assert!(Appearance::parse(&chrome_text(&with(DecoKind::Aero))).chrome.is_none());
    }
}
