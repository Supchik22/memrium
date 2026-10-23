//! The notification centre of the bell in the bar: everything that came, newest first, so a notification that was
//! missed (or went with do-not-disturb on) can still be read. A click on one does what its toast would have done.

use crate::notify::unix_now;
use crate::popup::Act;
use crate::state::State;
use chebrium::draw::Align;
use chebrium::ui::Ui;

pub const SIZE: [f32; 2] = [400.0, 560.0];
const PAD: f32 = 16.0;
const HEAD: f32 = 54.0;
const DND: f32 = 54.0;
/// an item with a body and one without
const ITEM: f32 = 92.0;
const ITEM_SHORT: f32 = 66.0;
const GAP: f32 = 8.0;

#[derive(Default)]
pub struct NotifPanel {
    scroll: f32,
}

/// "now", "5 min ago", "3 h ago", "Yesterday", "4 d ago".
pub fn ago(secs: u64) -> String {
    use chebrium::i18n::{tr, trf};
    match secs {
        0..=44 => tr("now").into_owned(),
        45..=3599 => trf("{} min ago", &[((secs / 60).max(1)).to_string()]),
        3600..=86_399 => trf("{} h ago", &[(secs / 3600).to_string()]),
        86_400..=172_799 => tr("Yesterday").into_owned(),
        _ => trf("{} d ago", &[(secs / 86_400).to_string()]),
    }
}

/// `text` in at most `lines` lines of at most `width` px; the last one ends in an ellipsis when it was cut.
fn wrap(ui: &mut Ui, text: &str, width: f32, size: f32, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut words = text.split_whitespace().peekable();
    let mut line = String::new();
    while let Some(word) = words.next() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if line.is_empty() || ui.measure_raw(&candidate, size, false) <= width {
            line = candidate;
        } else {
            out.push(std::mem::take(&mut line));
            if out.len() == lines {
                break;
            }
            line = word.to_string();
        }
        if words.peek().is_none() {
            out.push(std::mem::take(&mut line));
        }
    }
    if out.len() > lines {
        out.truncate(lines);
    }
    // anything left over (or a line that is too long by itself) is cut with an ellipsis
    let cut = words.peek().is_some() || out.iter().any(|l| ui.measure_raw(l, size, false) > width);
    if let Some(last) = out.last_mut().filter(|_| cut) {
        while !last.is_empty() && ui.measure_raw(&format!("{last}…"), size, false) > width {
            last.pop();
        }
        last.push('…');
    }
    out
}

fn fit(ui: &mut Ui, text: &str, size: f32, bold: bool, max: f32) -> String {
    if ui.measure_raw(text, size, bold) <= max {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() && ui.measure_raw(&format!("{s}…"), size, bold) > max {
        s.pop();
    }
    format!("{s}…")
}

impl State {
    pub fn notif_build(&self, ui: &mut Ui, pos: [f32; 2], np: &mut NotifPanel, acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let (w, h) = (SIZE[0], SIZE[1]);
        let items = &self.notifications.history;
        let dnd = self.config.notifications.dnd;

        // ---- the header
        ui.glyph(x + PAD + 8.0, y + 29.0, 18.0, if dnd { "bell-off" } else { "bell" }, ink, 0.85);
        ui.text(x + PAD + 26.0, y + 35.0, "Notifications", 15.0, true, ink, 1.0, Align::Left);
        if !items.is_empty() {
            let label = "Clear all";
            let tw = ui.measure(label, 12.5, true) + 20.0;
            let r = [x + w - PAD - tw, y + 17.0, tw, 26.0];
            let hov = ui.hit(r);
            ui.fill(r, 13.0, ink, if hov { 0.13 } else { 0.07 });
            ui.text(r[0] + tw / 2.0, r[1] + 17.5, label, 12.5, true, ink, 0.9, Align::Center);
            if hov && ui.click {
                acts.push(Act::NotifClear);
            }
        }

        // ---- do not disturb
        let dr = [x + PAD, y + HEAD, w - 2.0 * PAD, DND - 8.0];
        ui.fill(dr, 12.0, ink, 0.06);
        ui.glyph(dr[0] + 22.0, dr[1] + dr[3] / 2.0, 16.0, "bell-off", if dnd { accent } else { ink }, if dnd { 1.0 } else { 0.6 });
        ui.text(dr[0] + 44.0, dr[1] + 20.0, "Do not disturb", 13.0, true, ink, 0.95, Align::Left);
        ui.text(dr[0] + 44.0, dr[1] + 36.0, if dnd { "Only urgent ones pop up" } else { "Notifications pop up as usual" }, 11.0, false, ink, 0.5, Align::Left);
        let mut on = dnd;
        ui.toggle(dr[0] + dr[2] - 56.0, dr[1] + (dr[3] - 24.0) / 2.0, &mut on);
        if on != dnd {
            acts.push(Act::ToggleDnd);
        }

        // ---- the list
        let list = [x, y + HEAD + DND + 4.0, w, h - HEAD - DND - 4.0 - 8.0];
        if items.is_empty() {
            ui.glyph(x + w / 2.0, list[1] + list[3] / 2.0 - 24.0, 34.0, "bell", ink, 0.22);
            ui.text(x + w / 2.0, list[1] + list[3] / 2.0 + 20.0, "No notifications", 14.0, true, ink, 0.55, Align::Center);
            ui.text(x + w / 2.0, list[1] + list[3] / 2.0 + 40.0, "What you miss will be kept here", 12.0, false, ink, 0.4, Align::Center);
            return;
        }
        let heights: Vec<f32> = items.iter().map(|s| if s.n.body.trim().is_empty() { ITEM_SHORT } else { ITEM }).collect();
        let content: f32 = heights.iter().map(|h| h + GAP).sum();
        let max = (content - list[3] + 4.0).max(0.0);
        np.scroll = (np.scroll + ui.scroll).clamp(0.0, max);
        ui.d.push_clip(list);
        let now = unix_now();
        let mut cy = list[1] - np.scroll;
        let inner = w - 2.0 * PAD;
        for (s, ih) in items.iter().zip(&heights) {
            let top = cy;
            cy += ih + GAP;
            if top + ih < list[1] || top > list[1] + list[3] {
                continue;
            }
            let r = [x + PAD, top, inner, *ih];
            let hov = ui.hit(r) && ui.hit(list);
            ui.fill(r, 14.0, ink, if hov { 0.11 } else { 0.07 });
            // the app and how long ago
            let app = fit(ui, &s.n.app, 11.0, true, inner - 150.0);
            ui.text_raw(r[0] + 14.0, r[1] + 20.0, &app, 11.0, true, ink, 0.55, Align::Left);
            let when = if s.silenced { format!("{} · {}", chebrium::i18n::tr("Silenced"), ago(now.saturating_sub(s.at))) } else { ago(now.saturating_sub(s.at)) };
            let right = r[0] + inner - if hov { 36.0 } else { 14.0 };
            ui.text_raw(right, r[1] + 20.0, &when, 11.0, false, ink, 0.45, Align::Right);
            let summary = fit(ui, &s.n.summary, 13.5, true, inner - 28.0);
            ui.text_raw(r[0] + 14.0, r[1] + 42.0, &summary, 13.5, true, ink, 0.97, Align::Left);
            if !s.n.body.trim().is_empty() {
                for (i, line) in wrap(ui, &s.n.body, inner - 28.0, 12.5, 2).iter().enumerate() {
                    ui.text_raw(r[0] + 14.0, r[1] + 62.0 + i as f32 * 16.0, line, 12.5, false, ink, 0.75, Align::Left);
                }
            }
            if s.n.urgency == 2 {
                ui.fill([r[0], r[1] + 14.0, 3.0, r[3] - 28.0], 1.5, [0.95, 0.35, 0.3], 0.9);
            }
            // the dismiss cross shows when the pointer is on the item
            let cross = [r[0] + inner - 30.0, r[1] + 8.0, 22.0, 22.0];
            let on_cross = hov && ui.hit(cross);
            if hov {
                if on_cross {
                    ui.fill(cross, 11.0, ink, 0.14);
                }
                ui.glyph(cross[0] + 11.0, cross[1] + 11.0, 12.0, "x", ink, 0.8);
            }
            if hov && ui.click {
                if on_cross {
                    acts.push(Act::NotifDismiss(s.n.id));
                } else {
                    acts.push(Act::NotifOpen(s.n.id));
                }
            }
        }
        ui.d.pop_clip();
    }

    /// The card was opened: everything in it is seen.
    pub fn notif_seen(&mut self) {
        self.notifications.seen_at = unix_now();
    }
}
