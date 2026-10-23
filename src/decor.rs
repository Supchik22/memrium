//! Window decorations drawn by the compositor, per theme: macOS traffic lights, Fluent flat
//! titlebar, Aero glass frame. Pure functions: geometry, hit rectangles and panels.

use crate::renderer::{Align, Label, Panel};
use crate::theme::{BarMaterial, BodyTone, ButtonColors, ButtonOrder, ButtonShape, ButtonSide, DecoKind, FrameStyle, GlyphShow, Theme, TitleAlign};

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
        DecoKind::Custom => {
            let b = theme.window.frame.border;
            [b, tb, b, b]
        }
    }
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
    let (bw, bh) = if bar { (fs.button, tb) } else { (fs.button, fs.button) };
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

/// Horizontal span available for the title text (x0, x1).
pub fn title_span(theme: &Theme, frame: [f32; 4]) -> (f32, f32) {
    let [x, _, w, _] = frame;
    match theme.window.decoration {
        DecoKind::Mac => (x + 96.0, x + w - 96.0),
        DecoKind::Win11 => (x + 14.0, x + w - 150.0),
        DecoKind::Aero => (x + 14.0, x + w - 120.0),
        DecoKind::Gnome => (x + 110.0, x + w - 120.0),
        DecoKind::Minimal => (x, x),
        DecoKind::Custom => {
            let fs = &theme.window.frame;
            let bar = fs.shape == ButtonShape::Bar;
            let cluster = (if bar { 0.0 } else { 14.0 + fs.border }) + 3.0 * fs.button + 2.0 * fs.gap + 10.0;
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
    if title.is_empty() || theme.window.decoration == DecoKind::Minimal {
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
        DecoKind::Minimal => Vec::new(),
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

/// Chrome panels (shadow, frame, titlebar, buttons) for a window frame. The client surface is drawn on top.
pub fn frame_panels(theme: &Theme, frame: [f32; 4], focused: bool, maximized: bool, hover: Option<Btn>) -> Vec<Panel> {
    let [x, y, w, h] = frame;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let ws = &theme.window;
    let sh = if focused { ws.shadow } else { ws.shadow_idle };
    let r = if maximized && ws.max_gap < 0.5 { 0.0 } else { ws.radius };
    let mut out = Vec::new();
    match ws.decoration {
        DecoKind::Mac => {
            // the title bar follows the light / dark look
            let (top, bot) = if theme.dark { ([0.19, 0.19, 0.21], [0.15, 0.15, 0.17]) } else { ([0.97, 0.97, 0.98], [0.93, 0.93, 0.95]) };
            out.push(Panel::solid(cx, cy, w, h, r, top, bot).flat().shadow(sh[0], sh[1], sh[2]));
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
            out.push(Panel::solid(cx, cy, w, h, r, body, body).flat().shadow(sh[0], sh[1], sh[2]));
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
            out.push(Panel::solid(cx, cy, w, h, r, [0.97; 3], [0.95; 3]).flat().alpha(0.0).shadow(sh[0], sh[1], sh[2]));
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
        if fs.outline {
            let border = if dark { 0.35 } else { 0.16 };
            out.push(Panel::solid(cx, cy, w + 2.0, h + 2.0, r + 1.0, [0.0; 3], [0.0; 3]).flat().alpha(border).shadow(sh[0], sh[1], sh[2]));
            out.push(Panel::solid(cx, cy, w, h, r, body, body).flat());
        } else {
            out.push(Panel::solid(cx, cy, w, h, r, body, body).flat().shadow(sh[0], sh[1], sh[2]));
        }
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
        let rad = match fs.shape {
            ButtonShape::Circle => rect[2].min(rect[3]) / 2.0,
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
        if show_glyphs || (!plate_always && matches!(fs.glyphs, GlyphShow::Always)) {
            let size = (rect[2].min(rect[3]) * 0.6).clamp(8.0, 12.0);
            out.push(Panel::glyph(bx, by, size, name, hover_glyph, if focused { 0.9 } else { 0.5 }));
        }
    }
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
}
