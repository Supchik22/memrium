//! The look the Memrium board games share (tic-tac-toe, lights out, chess): the paper sheet over the window glass,
//! the toolbar row under the window buttons, pill buttons, score chips, pictures drawn from SVG and the result banner.
//! Included by each game with `#[path = "../../games_ui.rs"] mod games_ui;`.
#![allow(dead_code)]

use chebrium::app::Host;
use chebrium::appmenu::Menu;
use chebrium::draw::Align;
use chebrium::picture::Pic;
use chebrium::ui::{Cursor, Ui};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Height of the toolbar row the window buttons sit in.
pub const BAR_H: f32 = 52.0;

/// Room the window buttons take at the left and at the right of the toolbar row (they are left on a Mac-style desktop).
pub fn insets(host: &Host) -> (f32, f32) {
    let room = chebrium::chrome::button_room(host);
    (if room[0] > 0.0 { room[0] } else { 16.0 }, if room[1] > 0.0 { room[1] + 6.0 } else { 16.0 })
}

/// A game's title at the left of its toolbar row and, after it, the program's menus when the desktop has no menu bar for
/// them (`chrome::header_menus`; the ≡ button when they do not fit before the pills at the right). True when the pointer
/// is on the menus, so a press there is not a press on the bare header.
pub fn header_title(ui: &mut Ui<'_>, host: &mut Host, menus: &[Menu], title: &str, size: [f32; 2]) -> bool {
    let (lp, rp) = insets(host);
    let ink = ui.ink;
    ui.text(lp, 32.0, title, 15.0, true, ink, 0.9, Align::Left);
    let x = lp + ui.measure(title, 15.0, true) + 14.0;
    // the pills of the toolbar take about 170 px at the right
    let room = size[0] - rp - 170.0 - x;
    if room < 36.0 {
        chebrium::chrome::header_menus(ui, host, &[], x, BAR_H / 2.0, 0.0);
        return false;
    }
    let w = chebrium::chrome::header_menus(ui, host, menus, x, BAR_H / 2.0, room);
    ui.hit([x, 0.0, w, BAR_H])
}

/// The content paper over the whole window (the glass shows faintly through it).
pub fn sheet(ui: &mut Ui<'_>, size: [f32; 2]) {
    let (paper, a) = if ui.dark { ([0.0; 3], 0.32) } else { ([1.0; 3], 0.78) };
    ui.fill([0.0, 0.0, size[0], size[1]], 0.0, paper, a);
}

/// A stable key for eased values from a name and a number.
pub fn key(name: &str, n: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    n.hash(&mut h);
    h.finish() | 1 << 63
}

/// A pill with an icon and a label; true when pressed. `on` puts it on the accent.
pub fn pill(ui: &mut Ui<'_>, r: [f32; 4], glyph: &str, label: &str, on: bool, enabled: bool) -> bool {
    let hov = enabled && ui.hit(r);
    if hov {
        ui.want_cursor(Cursor::Pointer);
    }
    let wash = ui.ease(key(label, r[0].to_bits() as u64), if hov { 1.0 } else { 0.0 }, 22.0);
    let ink = ui.ink;
    let fade = if enabled { 1.0 } else { 0.35 };
    if on {
        ui.fill(r, r[3] / 2.0, ui.accent, 0.9 + 0.1 * wash);
    } else {
        ui.fill(r, r[3] / 2.0, ink, 0.065 + 0.07 * wash);
    }
    let fg = if on { [1.0; 3] } else { ink };
    let shown = chebrium::i18n::tr(label).into_owned();
    let tw = if shown.is_empty() { 0.0 } else { ui.measure_raw(&shown, 13.0, true) };
    let gw = if glyph.is_empty() { 0.0 } else { 16.0 };
    let gap = if gw > 0.0 && tw > 0.0 { 6.0 } else { 0.0 };
    let x0 = r[0] + (r[2] - gw - gap - tw) / 2.0;
    if gw > 0.0 {
        ui.glyph(x0 + 8.0, r[1] + r[3] / 2.0, 15.0, glyph, fg, 0.85 * fade);
    }
    if tw > 0.0 {
        ui.text_raw(x0 + gw + gap, r[1] + r[3] / 2.0 + 4.5, &shown, 13.0, true, fg, 0.85 * fade, Align::Left);
    }
    hov && ui.click
}

/// Width a pill needs for `label` (with an icon when `glyph`).
pub fn pill_w(ui: &mut Ui<'_>, label: &str, glyph: bool) -> f32 {
    let shown = chebrium::i18n::tr(label).into_owned();
    let tw = if shown.is_empty() { 0.0 } else { ui.measure_raw(&shown, 13.0, true) };
    tw + if glyph { 16.0 + if tw > 0.0 { 6.0 } else { 0.0 } } else { 0.0 } + 28.0
}

/// A small card with a dim label over a big number; `tone` colours the number (or the ink when `None`); `lit` eases a
/// wash of that colour behind it (whose turn it is).
pub fn chip(ui: &mut Ui<'_>, r: [f32; 4], label: &str, value: &str, tone: Option<[f32; 3]>, lit: f32) {
    let ink = ui.ink;
    let c = tone.unwrap_or(ink);
    ui.fill(r, 14.0, ink, 0.05);
    if lit > 0.01 {
        ui.fill(r, 14.0, c, 0.14 * lit);
    }
    ui.text(r[0] + r[2] / 2.0, r[1] + 19.0, label, 11.0, true, ink, 0.55, Align::Center);
    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] - 12.0, value, 24.0, true, c, 0.95, Align::Center);
}

/// SVG pictures drawn once per size.
#[derive(Default)]
pub struct Pics {
    map: HashMap<(u64, u32), Option<Arc<Pic>>>,
}

impl Pics {
    pub fn get(&mut self, svg: &str, px: u32) -> Option<Arc<Pic>> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        svg.hash(&mut h);
        let k = (h.finish(), px.max(4));
        if self.map.len() > 400 {
            self.map.clear();
        }
        self.map
            .entry(k)
            .or_insert_with(|| memrium_de::svgedit::rasterize(svg, k.1).map(|i| Arc::new(Pic::from_rgba(i.width(), i.height(), i.as_raw()))))
            .clone()
    }

    /// Draw `svg` into the square `r` (sharp on the screen's pixels).
    pub fn draw(&mut self, ui: &mut Ui<'_>, svg: &str, r: [f32; 4], alpha: f32) {
        let px = (r[2] * ui.px()).round().max(4.0) as u32;
        if let Some(p) = self.get(svg, px) {
            ui.picture(r, &p, [0.0, 0.0, px as f32, px as f32], alpha);
        }
    }
}

/// The banner over the board when a game ends: it pops in on `shown` and returns the button rectangle when it is up
/// (so the caller can see whether it was pressed).
pub fn banner(ui: &mut Ui<'_>, area: [f32; 4], title: &str, line: &str, button: &str, shown: bool) -> bool {
    let k = key("banner", 0);
    let t = ui.ease(k, if shown { 1.0 } else { 0.0 }, 9.0);
    if t < 0.01 {
        return false;
    }
    // a little overshoot as it arrives
    let s = 0.9 + 0.1 * t + 0.04 * (t * std::f32::consts::PI).sin();
    let (bw, bh) = (area[2].min(320.0), 150.0);
    let (cx, cy) = (area[0] + area[2] / 2.0, area[1] + area[3] / 2.0);
    let r = [cx - bw * s / 2.0, cy - bh * s / 2.0, bw * s, bh * s];
    let paper = if ui.dark { [0.13, 0.135, 0.15] } else { [1.0; 3] };
    ui.fill([r[0] - 2.0, r[1] + 6.0, r[2] + 4.0, r[3] + 4.0], 26.0, [0.0; 3], 0.16 * t);
    ui.fill(r, 24.0, paper, 0.97 * t);
    let ink = ui.ink;
    ui.text(cx, r[1] + 44.0 * s, title, 22.0 * s, true, ink, t, Align::Center);
    ui.text_raw(cx, r[1] + 70.0 * s, line, 13.0 * s, false, ink, 0.6 * t, Align::Center);
    let btn = [cx - 70.0 * s, r[1] + r[3] - 50.0 * s, 140.0 * s, 34.0 * s];
    shown && t > 0.6 && pill(ui, btn, "", button, true, true)
}

/// Rounded-polygon helpers for game pictures (viewBox 0 0 100 100).
pub fn svg100(body: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">{body}</svg>")
}
