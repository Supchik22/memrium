//! The SHAPES part of Settings > Cursor: a row of choices for every shape, each ending in "Custom", and under a row
//! whose choice that is, the shape's editor (what to start from, a big preview, a slider for every number of it, see
//! `cursor::custom`).

use super::{cursor_preview, group, separator};
use crate::cursor::custom::{ArrowGen, BeamGen, BusyGen, Dial, Gen, HandGen, Made, ResizeGen, SignGen};
use crate::cursor::{CursorPrefs, HandStyle, Shape, ARROWS, BEAMS, HANDS, RESIZES};
use chebrium::draw::Align;
use chebrium::ui::{Cursor, Ui};

/// What the page passes on for the previews: their atlas cells, and the ones that are wanted.
pub(super) struct Previews<'a, 'b> {
    pub cell: &'a dyn Fn(&str) -> Option<u32>,
    pub want: &'b mut Vec<String>,
}

const PAD: f32 = 16.0;
const TILE_H: f32 = 64.0;
const TILE_GAP: f32 = 8.0;
/// the narrowest tile
const TILE_MIN: f32 = 70.0;
/// a page narrower than this has a row's label over its tiles, and an editor's preview over its sliders
const WIDE: f32 = 500.0;
const CHIP_H: f32 = 26.0;
const DIAL_H: f32 = 30.0;
/// the right part of a slider: its gap and the number
const NUMBER: f32 = 66.0;

/// Where the tiles of a row go.
#[derive(Clone, Copy)]
struct Tiles {
    x: f32,
    top: f32,
    cols: usize,
    w: f32,
    h: f32,
}

impl Tiles {
    fn new(cw: f32, n: usize) -> Tiles {
        let (x, top) = if cw >= WIDE { (PAD + 134.0, 8.0) } else { (PAD, 30.0) };
        let room = (cw - x - PAD).max(TILE_MIN);
        let most = (((room + TILE_GAP) / (TILE_MIN + TILE_GAP)).floor() as usize).max(1);
        // as many lines as it takes, with about as many tiles in each
        let lines = n.div_ceil(most).max(1);
        let cols = n.div_ceil(lines).max(1);
        let w = ((room + TILE_GAP) / cols as f32 - TILE_GAP).min(104.0);
        Tiles { x, top, cols, w, h: top + lines as f32 * (TILE_H + TILE_GAP) }
    }

    fn rect(&self, x: f32, y: f32, i: usize) -> [f32; 4] {
        [x + self.x + (i % self.cols) as f32 * (self.w + TILE_GAP), y + self.top + (i / self.cols) as f32 * (TILE_H + TILE_GAP), self.w, TILE_H]
    }
}

/// One line of text in `room`: as big as `size` where it fits, smaller where it doesn't, cut where even that is too wide.
fn fitted(ui: &mut Ui, x: f32, y: f32, text: &str, size: f32, bold: bool, alpha: f32, room: f32, align: Align) {
    let mut s = crate::i18n::tr(text).into_owned();
    let mut px = size;
    while px > 9.5 && ui.measure_raw(&s, px, bold) > room {
        px -= 0.5;
    }
    if ui.measure_raw(&s, px, bold) > room {
        while !s.is_empty() && ui.measure_raw(&format!("{s}…"), px, bold) > room {
            s.pop();
        }
        s.push('…');
    }
    ui.text_raw(x, y, &s, px, bold, ui.ink, alpha, align);
}

/// A choice shown as the cursor it gives; true when it was clicked and wasn't the choice yet.
#[allow(clippy::too_many_arguments)]
fn tile(ui: &mut Ui, pv: &mut Previews, r: [f32; 4], slot: &str, shape: Shape, prefs: &CursorPrefs, name: &str, sel: bool) -> bool {
    let hov = ui.hit(r);
    if sel {
        ui.fill([r[0] - 2.0, r[1] - 2.0, r[2] + 4.0, r[3] + 4.0], 13.0, ui.accent, 0.9);
        ui.fill(r, 11.0, if ui.dark { [0.149, 0.149, 0.149] } else { [0.96, 0.97, 1.0] }, 1.0);
    }
    ui.fill(r, 11.0, ui.ink, if hov { 0.10 } else { 0.05 });
    cursor_preview(ui, [r[0] + r[2] / 2.0 - 19.0, r[1] + 4.0, 38.0, 38.0], slot, shape, prefs, pv.cell, pv.want);
    fitted(ui, r[0] + r[2] / 2.0, r[1] + r[3] - 8.0, name, 11.5, sel, if sel { 0.95 } else { 0.7 }, r[2] - 6.0, Align::Center);
    if hov {
        ui.want_cursor(Cursor::Pointer);
    }
    let picked = hov && ui.click && !sel;
    ui.changed |= picked;
    picked
}

/// A shape's row: its presets, the generator behind its "Custom", and what its editor previews.
struct Row<S: 'static, G: Gen> {
    id: &'static str,
    label: &'static str,
    presets: &'static [(S, &'static str)],
    get: fn(&CursorPrefs) -> S,
    set: fn(&mut CursorPrefs, S),
    made: fn(&mut CursorPrefs) -> &mut Made<G>,
    /// what the tiles show
    shape: Shape,
    /// the editor's small previews (the first is also the big one)
    strip: &'static [Shape],
}

impl<S: Copy + PartialEq, G: Gen> Row<S, G> {
    /// The row's height, and the editor's when it is open.
    fn measure(&self, ui: &mut Ui, c: &CursorPrefs, cw: f32) -> (f32, Option<f32>) {
        let row = Tiles::new(cw, self.presets.len() + 1).h;
        let mut p = c.clone();
        let made = (self.made)(&mut p);
        (row, made.on.then(|| editor_height(ui, &made.clone(), G::DIALS, cw)))
    }

    /// Draw it as it was measured; `sizes` is what `measure` gave.
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, ui: &mut Ui, pv: &mut Previews, c: &mut CursorPrefs, x: f32, y: f32, cw: f32, sizes: (f32, Option<f32>)) {
        let tiles = Tiles::new(cw, self.presets.len() + 1);
        if cw >= WIDE {
            fitted(ui, x + PAD, y + 8.0 + TILE_H / 2.0 + 4.5, self.label, 13.0, false, 0.92, tiles.x - PAD - 10.0, Align::Left);
        } else {
            fitted(ui, x + PAD, y + 22.0, self.label, 13.0, false, 0.92, cw - 2.0 * PAD, Align::Left);
        }
        let base = c.clone();
        let custom = sizes.1.is_some();
        for (i, (value, name)) in self.presets.iter().enumerate() {
            let mut p = base.clone();
            (self.made)(&mut p).on = false;
            (self.set)(&mut p, *value);
            let sel = !custom && (self.get)(&base) == *value;
            if tile(ui, pv, tiles.rect(x, y, i), &format!("{}-t{i}", self.id), self.shape, &p, name, sel) {
                (self.made)(c).on = false;
                (self.set)(c, *value);
            }
        }
        let mut p = base.clone();
        if !custom {
            // what Custom would start as
            *(self.made)(&mut p) = Made { on: true, shape: G::from_preset(&base), last: None };
            G::chosen(&mut p);
        }
        let n = self.presets.len();
        let shown = self.shown(&p);
        if tile(ui, pv, tiles.rect(x, y, n), &format!("{}-t{n}", self.id), shown, &p, "Custom", custom) {
            (self.made)(c).choose(&base);
            G::chosen(c);
        }
        if sizes.1.is_some() {
            editor(ui, pv, c, self.id, self.made, G::DIALS, shown, self.strip, x, y + sizes.0, cw);
        }
    }

    /// The shape that shows what the sliders make (the hand's show on the open hand while links have the arrow).
    fn shown(&self, c: &CursorPrefs) -> Shape {
        if self.shape == Shape::Pointer && c.hand == HandStyle::Arrow { Shape::Grab } else { self.shape }
    }
}

const ARROW: Row<crate::cursor::ArrowStyle, ArrowGen> = Row {
    id: "arrow",
    label: "Arrow",
    presets: &ARROWS,
    get: |p| p.arrow,
    set: |p, v| p.arrow = v,
    made: |p| &mut p.custom.arrow,
    shape: Shape::Default,
    strip: &[Shape::Default, Shape::Progress, Shape::Copy],
};

const HAND: Row<HandStyle, HandGen> = Row {
    id: "hand",
    label: "Over links",
    presets: &HANDS,
    get: |p| p.hand,
    set: |p, v| p.hand = v,
    made: |p| &mut p.custom.hand,
    shape: Shape::Pointer,
    strip: &[Shape::Pointer, Shape::Grab, Shape::Grabbing],
};

const BEAM: Row<crate::cursor::BeamStyle, BeamGen> = Row {
    id: "beam",
    label: "Over text",
    presets: &BEAMS,
    get: |p| p.beam,
    set: |p, v| p.beam = v,
    made: |p| &mut p.custom.beam,
    shape: Shape::Text,
    strip: &[Shape::Text],
};

const RESIZE: Row<crate::cursor::ResizeStyle, ResizeGen> = Row {
    id: "resize",
    label: "Over edges",
    presets: &RESIZES,
    get: |p| p.resize,
    set: |p, v| p.resize = v,
    made: |p| &mut p.custom.resize,
    shape: Shape::ResizeNwse,
    strip: &[Shape::ResizeNwse, Shape::ResizeEw, Shape::Move],
};

const SIGNS: Row<(), SignGen> = Row {
    id: "signs",
    label: "Cross and signs",
    presets: &[((), "Standard")],
    get: |_| (),
    set: |_, _| {},
    made: |p| &mut p.custom.signs,
    shape: Shape::Crosshair,
    strip: &[Shape::Crosshair, Shape::NotAllowed],
};

/// The whole card; returns its bottom.
pub(super) fn shapes(ui: &mut Ui, pv: &mut Previews, c: &mut CursorPrefs, x: f32, top: f32, cw: f32) -> f32 {
    let sizes = [ARROW.measure(ui, c, cw), HAND.measure(ui, c, cw), BEAM.measure(ui, c, cw), RESIZE.measure(ui, c, cw), SIGNS.measure(ui, c, cw)];
    let tall = |s: &(f32, Option<f32>)| s.0 + s.1.unwrap_or(0.0);
    let h = 8.0 + sizes.iter().map(tall).sum::<f32>();
    group(ui, [x, top, cw, h]);
    let mut ys = [top + 4.0; 5];
    for i in 1..5 {
        ys[i] = ys[i - 1] + tall(&sizes[i - 1]);
        separator(ui, x, ys[i], cw);
    }
    ARROW.draw(ui, pv, c, x, ys[0], cw, sizes[0]);
    HAND.draw(ui, pv, c, x, ys[1], cw, sizes[1]);
    BEAM.draw(ui, pv, c, x, ys[2], cw, sizes[2]);
    RESIZE.draw(ui, pv, c, x, ys[3], cw, sizes[3]);
    SIGNS.draw(ui, pv, c, x, ys[4], cw, sizes[4]);
    top + h
}

/// The editor of the busy cursor, a card of its own under LOADING (whose Style has the Custom choice); returns its
/// bottom.
pub(super) fn busy(ui: &mut Ui, pv: &mut Previews, c: &mut CursorPrefs, x: f32, top: f32, cw: f32) -> f32 {
    let dials = BusyGen::dials_for(c.busy);
    let h = editor_height(ui, &c.custom.busy, dials, cw) + 10.0;
    group(ui, [x, top, cw, h]);
    editor(ui, pv, c, "busy", |p| &mut p.custom.busy, dials, Shape::Wait, &[Shape::Wait, Shape::Progress], x, top + 10.0, cw);
    top + h
}

// ---- an editor

/// The chips under "Start from": their names, and where each goes from the editor's inside corner.
fn chips<G: Gen>(ui: &mut Ui, made: &Made<G>, w: f32) -> (Vec<(&'static str, [f32; 4])>, f32) {
    let caption = ui.measure("Start from", 12.0, false) + 12.0;
    let mut names: Vec<&'static str> = G::STARTS.iter().map(|s| s.0).collect();
    if made.last.is_some() {
        names.push("Previous");
    }
    let (mut x, mut y) = (caption, 0.0);
    let mut out = Vec::new();
    for name in names {
        let cw = (ui.measure(name, 12.0, false) + 22.0).min(w);
        if x > 0.0 && x + cw > w {
            x = 0.0;
            y += CHIP_H + 6.0;
        }
        out.push((name, [x, y, cw, CHIP_H]));
        x += cw + 6.0;
    }
    (out, y + CHIP_H)
}

/// The preview column's size, and whether it stands beside the sliders.
fn preview_column(cw: f32) -> (f32, f32, bool) {
    let side = cw >= WIDE;
    let hero = if side { 148.0 } else { 128.0 };
    // the big one, the small ones, two buttons
    (hero, hero + 8.0 + 44.0 + 8.0 + CHIP_H, side)
}

fn editor_height<G: Gen>(ui: &mut Ui, made: &Made<G>, dials: &[Dial<G>], cw: f32) -> f32 {
    let (_, chips_h) = chips(ui, made, cw - 2.0 * PAD);
    let (_, pv_h, side) = preview_column(cw);
    let sliders = dials.len() as f32 * DIAL_H;
    switch_height::<G>() + chips_h + 14.0 + if side { pv_h.max(sliders) } else { pv_h + 12.0 + sliders } + 16.0
}

/// The line over "Start from" that a shape with a switch has.
fn switch_height<G: Gen>() -> f32 {
    if G::SWITCH.is_some() { 36.0 } else { 0.0 }
}

#[allow(clippy::too_many_arguments)]
fn editor<G: Gen>(ui: &mut Ui, pv: &mut Previews, c: &mut CursorPrefs, id: &str, get: fn(&mut CursorPrefs) -> &mut Made<G>, dials: &[Dial<G>], big: Shape, strip: &[Shape], x: f32, y: f32, cw: f32) {
    let base = c.clone();
    let mut p = base.clone();
    let made = get(&mut p).clone();
    let (left, room) = (x + PAD, cw - 2.0 * PAD);

    if let Some((name, is_on, set)) = G::SWITCH {
        let mut on = is_on(&base);
        fitted(ui, left, y + 18.0, name, 12.0, false, 0.85, room - 56.0, Align::Left);
        let was = on;
        ui.toggle(left + room - 44.0, y + 1.5, &mut on);
        if on != was {
            set(c, on);
            ui.changed = true;
        }
    }
    let y = y + switch_height::<G>();

    // ---- where to start
    let (list, chips_h) = chips(ui, &made, room);
    ui.text(left, y + 17.5, "Start from", 12.0, false, ui.ink, 0.6, Align::Left);
    for (i, (name, at)) in list.iter().enumerate() {
        let r = [left + at[0], y + at[1], at[2], at[3]];
        let hov = ui.hit(r);
        // lit while the sliders still are that preset
        let start = G::STARTS.get(i);
        let lit = start.is_some_and(|(_, set)| {
            let mut s = base.clone();
            set(&mut s);
            made.shape == G::from_preset(&s) && (s == base || G::SWITCH.is_some())
        });
        ui.fill(r, CHIP_H / 2.0, if lit { ui.accent } else { ui.ink }, if lit { 0.85 } else if hov { 0.14 } else { 0.07 });
        ui.text(r[0] + r[2] / 2.0, r[1] + 17.5, name, 12.0, false, if lit { [1.0; 3] } else { ui.ink }, 0.95, Align::Center);
        if hov {
            ui.want_cursor(Cursor::Pointer);
            if ui.click {
                let next = match start {
                    Some((_, set)) => {
                        set(c);
                        G::from_preset(c)
                    }
                    None => made.last.clone().unwrap_or_default(),
                };
                get(c).start(next);
                ui.changed = true;
            }
        }
    }

    // ---- the previews
    let top = y + chips_h + 14.0;
    let (hero, pv_h, side) = preview_column(cw);
    let r = [left, top, hero, hero];
    ui.fill(r, 14.0, [0.12, 0.13, 0.15], 1.0);
    ui.push_clip([r[0] + hero / 2.0, r[1], hero / 2.0, hero]);
    ui.fill(r, 14.0, [0.86, 0.88, 0.91], 1.0);
    ui.pop_clip();
    let inset = (hero - 128.0).max(0.0) / 2.0;
    let side_px = hero.min(128.0);
    cursor_preview(ui, [r[0] + inset, r[1] + inset, side_px, side_px], &format!("{id}-big"), big, &base, pv.cell, pv.want);
    for (i, shape) in strip.iter().enumerate() {
        let sr = [left + i as f32 * 50.0, top + hero + 8.0, 44.0, 44.0];
        ui.fill(sr, 10.0, ui.ink, 0.07);
        cursor_preview(ui, [sr[0] + 3.0, sr[1] + 3.0, 38.0, 38.0], &format!("{id}-s{i}"), *shape, &base, pv.cell, pv.want);
    }
    let by = top + hero + 60.0;
    let bw = (hero - 6.0) / 2.0;
    if ui.button([left, by, bw, CHIP_H], "Reset", false) {
        get(c).start(G::from_preset(&base));
        ui.changed = true;
    }
    if ui.button([left + bw + 6.0, by, bw, CHIP_H], "Random", false) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
        get(c).start(G::random(&base, now.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
        ui.changed = true;
    }

    // ---- the sliders
    let (sx, sy) = if side { (left + hero + 20.0, top) } else { (left, top + pv_h + 12.0) };
    let sw = left + room - sx;
    let names = dials.iter().map(|d| ui.measure(d.label, 12.0, false)).fold(0.0f32, f32::max);
    let label_w = (names + 12.0).min(sw * 0.42);
    let slider_w = (sw - label_w - NUMBER).max(36.0);
    for (i, d) in dials.iter().enumerate() {
        let cy = sy + (i as f32 + 0.5) * DIAL_H;
        fitted(ui, sx, cy + 4.5, d.label, 12.0, false, 0.85, label_w - 6.0, Align::Left);
        let mut shape = made.shape.clone();
        let before = *(d.get)(&mut shape);
        let mut v = before;
        ui.slider([sx + label_w, cy - 9.0, slider_w, 18.0], &mut v, d.lo, d.hi, "");
        if v != before {
            let v = if d.whole { v.round() } else { v };
            *(d.get)(&mut get(c).shape) = v;
        }
    }
}
