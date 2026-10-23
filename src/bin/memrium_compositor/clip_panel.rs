//! The clipboard history card (Super+V): the pinned entries, then what was copied lately; a click pastes one, the pin
//! keeps it, the cross drops it. Pictures show a thumbnail.

use crate::clipboard::{ClipData, ClipEntry};
use crate::notif_panel::{ago, SIZE};
use crate::notify::unix_now;
use crate::popup::Act;
use crate::state::State;
use chebrium::draw::Align;
use chebrium::ui::Ui;
use memrium_de::renderer::Panel;

const PAD: f32 = 16.0;
const HEAD: f32 = 54.0;
const TEXT_H: f32 = 74.0;
const PIC_H: f32 = 96.0;
const SECTION: f32 = 28.0;
const GAP: f32 = 8.0;

#[derive(Default)]
pub struct ClipPanel {
    scroll: f32,
    /// thumbnails to put over the list once it is drawn: (rectangle, atlas cell)
    pub thumbs: Vec<([f32; 4], u32)>,
    /// the part of the card the list shows (thumbnails are cut to it)
    pub view: [f32; 4],
}

fn thumb_key(id: u64) -> String {
    format!("clip:{id}")
}

/// One line of a text entry: whitespace gathered, cut to `max` px.
fn flat(ui: &mut Ui, text: &str, size: f32, max: f32, lines: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut i = 0;
    while i < words.len() {
        let candidate = if line.is_empty() { words[i].to_string() } else { format!("{line} {}", words[i]) };
        if line.is_empty() || ui.measure_raw(&candidate, size, false) <= max {
            line = candidate;
            i += 1;
        } else {
            out.push(std::mem::take(&mut line));
            if out.len() == lines {
                break;
            }
        }
    }
    if !line.is_empty() && out.len() < lines {
        out.push(line);
    }
    let cut = i < words.len();
    if let Some(last) = out.last_mut() {
        if cut || ui.measure_raw(last, size, false) > max {
            while !last.is_empty() && ui.measure_raw(&format!("{last}…"), size, false) > max {
                last.pop();
            }
            last.push('…');
        }
    }
    out
}

fn kib(n: usize) -> String {
    if n < 1024 { format!("{n} B") } else if n < 1024 * 1024 { format!("{:.0} KiB", n as f32 / 1024.0) } else { format!("{:.1} MiB", n as f32 / 1048576.0) }
}

impl State {
    /// Before drawing: a thumbnail in the atlas for every picture that is shown.
    pub fn clip_prepare(&mut self) {
        let wanted: Vec<(u64, std::sync::Arc<Vec<u8>>)> = self.clipboard.items.iter().filter_map(|e| if let ClipData::Image { png, .. } = &e.data { Some((e.id, png.clone())) } else { None }).take(12).collect();
        for (id, png) in wanted {
            let key = thumb_key(id);
            if self.renderer.app_icon_get(&key).is_some() {
                continue;
            }
            if let Ok(img) = image::load_from_memory(&png) {
                let (w, h) = (img.width(), img.height());
                let s = 128.0 / w.max(h).max(1) as f32;
                let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
                let small = image::imageops::resize(&img.to_rgba8(), nw, nh, image::imageops::FilterType::Triangle);
                let mut out = image::RgbaImage::new(128, 128);
                image::imageops::overlay(&mut out, &small, ((128 - nw) / 2) as i64, ((128 - nh) / 2) as i64);
                self.renderer.app_icon_cell(&key, &out);
            }
        }
    }

    pub fn clip_build(&self, ui: &mut Ui, pos: [f32; 2], cp: &mut ClipPanel, acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let (w, h) = (SIZE[0], SIZE[1]);
        let items = &self.clipboard.items;
        cp.thumbs.clear();

        ui.glyph(x + PAD + 8.0, y + 29.0, 18.0, "clipboard-paste", ink, 0.85);
        ui.text(x + PAD + 26.0, y + 35.0, "Clipboard", 15.0, true, ink, 1.0, Align::Left);
        if items.iter().any(|e| !e.pinned) {
            let label = "Clear";
            let tw = ui.measure(label, 12.5, true) + 20.0;
            let r = [x + w - PAD - tw, y + 17.0, tw, 26.0];
            let hov = ui.hit(r);
            ui.fill(r, 13.0, ink, if hov { 0.13 } else { 0.07 });
            ui.text(r[0] + tw / 2.0, r[1] + 17.5, label, 12.5, true, ink, 0.9, Align::Center);
            if hov && ui.click {
                acts.push(Act::ClipClear);
            }
        }

        let list = [x, y + HEAD, w, h - HEAD - 8.0];
        cp.view = list;
        if items.is_empty() {
            ui.glyph(x + w / 2.0, list[1] + list[3] / 2.0 - 24.0, 34.0, "clipboard-paste", ink, 0.22);
            ui.text(x + w / 2.0, list[1] + list[3] / 2.0 + 20.0, "Nothing copied yet", 14.0, true, ink, 0.55, Align::Center);
            ui.text(x + w / 2.0, list[1] + list[3] / 2.0 + 40.0, "Copy text or a picture and it will be here", 12.0, false, ink, 0.4, Align::Center);
            return;
        }

        // rows: a heading before the pinned ones and before the rest
        enum Row<'a> {
            Head(&'static str),
            Item(&'a ClipEntry),
        }
        let mut rows: Vec<Row> = Vec::new();
        if items.iter().any(|e| e.pinned) {
            rows.push(Row::Head("Pinned"));
            rows.extend(items.iter().filter(|e| e.pinned).map(Row::Item));
        }
        if items.iter().any(|e| !e.pinned) {
            rows.push(Row::Head("Recent"));
            rows.extend(items.iter().filter(|e| !e.pinned).map(Row::Item));
        }
        let height = |r: &Row| match r {
            Row::Head(_) => SECTION,
            Row::Item(e) if e.is_image() => PIC_H + GAP,
            Row::Item(_) => TEXT_H + GAP,
        };
        let content: f32 = rows.iter().map(height).sum();
        let max = (content - list[3] + 4.0).max(0.0);
        cp.scroll = (cp.scroll + ui.scroll).clamp(0.0, max);
        ui.d.push_clip(list);
        let now = unix_now();
        let inner = w - 2.0 * PAD;
        let mut cy = list[1] - cp.scroll;
        for row in &rows {
            let top = cy;
            let rh = height(row);
            cy += rh;
            if top + rh < list[1] || top > list[1] + list[3] {
                continue;
            }
            let e = match row {
                Row::Head(t) => {
                    ui.text(x + PAD + 4.0, top + 18.0, t, 12.0, true, ink, 0.5, Align::Left);
                    continue;
                }
                Row::Item(e) => e,
            };
            let ih = rh - GAP;
            let r = [x + PAD, top, inner, ih];
            let hov = ui.hit(r) && ui.hit(list);
            ui.fill(r, 14.0, ink, if hov { 0.11 } else { 0.07 });
            if e.pinned {
                ui.fill([r[0], r[1] + 14.0, 3.0, r[3] - 28.0], 1.5, accent, 0.9);
            }
            let right = r[0] + inner - 14.0;
            let ago_text = ago(now.saturating_sub(e.at));
            let text_x = match &e.data {
                ClipData::Image { w: iw, h: ih2, png } => {
                    let tr = [r[0] + 12.0, r[1] + 12.0, ih - 24.0, ih - 24.0];
                    ui.fill(tr, 8.0, ink, 0.08);
                    if let Some(cell) = self.renderer.app_icon_get(&thumb_key(e.id)) {
                        cp.thumbs.push((tr, cell));
                    }
                    ui.text_raw(tr[0] + tr[2] + 14.0, r[1] + 34.0, "Picture", 13.5, true, ink, 0.95, Align::Left);
                    ui.text_raw(tr[0] + tr[2] + 14.0, r[1] + 54.0, &format!("{iw} × {ih2}  ·  {}", kib(png.len())), 12.0, false, ink, 0.6, Align::Left);
                    tr[0] + tr[2] + 14.0
                }
                ClipData::Text(t) => {
                    let lines = flat(ui, t, 12.5, inner - 28.0 - if hov { 52.0 } else { 0.0 }, 3);
                    for (i, l) in lines.iter().enumerate() {
                        ui.text_raw(r[0] + 14.0, r[1] + 22.0 + i as f32 * 16.5, l, 12.5, false, ink, 0.92, Align::Left);
                    }
                    r[0] + 14.0
                }
            };
            let _ = text_x;
            ui.text_raw(right, r[1] + r[3] - 10.0, &ago_text, 10.5, false, ink, 0.4, Align::Right);
            // pin and cross, while the pointer is on the entry (a pinned one always shows its pin)
            let pin = [r[0] + inner - 54.0, r[1] + 8.0, 22.0, 22.0];
            let cross = [r[0] + inner - 30.0, r[1] + 8.0, 22.0, 22.0];
            let (on_pin, on_cross) = (hov && ui.hit(pin), hov && ui.hit(cross));
            if hov {
                for (b, on) in [(pin, on_pin), (cross, on_cross)] {
                    if on {
                        ui.fill(b, 11.0, ink, 0.14);
                    }
                }
                ui.glyph(pin[0] + 11.0, pin[1] + 11.0, 12.0, "pin", if e.pinned { accent } else { ink }, if e.pinned { 1.0 } else { 0.7 });
                ui.glyph(cross[0] + 11.0, cross[1] + 11.0, 12.0, "x", ink, 0.8);
            } else if e.pinned {
                ui.glyph(cross[0] + 11.0, cross[1] + 11.0, 12.0, "pin", accent, 0.9);
            }
            if hov && ui.click {
                if on_pin {
                    acts.push(Act::ClipPin(e.id));
                } else if on_cross {
                    acts.push(Act::ClipDelete(e.id));
                } else {
                    acts.push(Act::ClipPaste(e.id));
                }
            }
        }
        ui.d.pop_clip();
    }

    /// The thumbnails of the pictures, over the card (only those that are wholly in the list's view).
    pub fn clip_thumb_panels(&self, cp: &ClipPanel, k: f32, panels: &mut Vec<Panel>) {
        for (r, cell) in &cp.thumbs {
            if r[1] >= cp.view[1] && r[1] + r[3] <= cp.view[1] + cp.view[3] {
                panels.push(Panel::app_icon(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], *cell, k));
            }
        }
    }
}
