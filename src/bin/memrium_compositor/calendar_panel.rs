//! The calendar of the clock: days, months and years to move through, and a Today button.

use crate::state::State;
use crate::widgets_panel::{days_in_month, first_weekday, now, MONTHS, SHORT};
use chebrium::draw::Align;
use chebrium::style::Tone;
use chebrium::ui::Ui;

pub const SIZE: [f32; 2] = [420.0, 412.0];
const PAD: f32 = 16.0;

#[derive(Default)]
pub struct Cal {
    /// 0 days, 1 months, 2 years
    mode: usize,
    year: i32,
    month: u32,
    sel: Option<(i32, u32, u32)>,
}

impl Cal {
    pub fn reset_to_today(&mut self) {
        let t = now();
        *self = Cal { mode: 0, year: t.year, month: t.month, sel: Some((t.year, t.month, t.day)) };
    }

    /// One step back (-1) or forward (+1): a month, a year or twelve years, by what is shown.
    fn step(&mut self, d: i32) {
        match self.mode {
            0 => {
                let m = self.month as i32 - 1 + d;
                self.year += m.div_euclid(12);
                self.month = m.rem_euclid(12) as u32 + 1;
            }
            1 => self.year += d,
            _ => self.year += 12 * d,
        }
    }
}

fn prev_month(y: i32, m: u32) -> (i32, u32) {
    if m == 1 { (y - 1, 12) } else { (y, m - 1) }
}

fn next_month(y: i32, m: u32) -> (i32, u32) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

impl State {
    pub fn calendar_build(&self, ui: &mut Ui, pos: [f32; 2], cal: &mut Cal) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let w = SIZE[0];
        let today = now();
        let monday = self.theme.ui.clock.week_monday;
        let t0 = cal.year - cal.year.rem_euclid(12);
        // today is red as on a wall calendar when the controls have several accents, the accent otherwise
        let today_colour = ui.tone(Tone::Danger);

        // ---- the header: what is shown (a click on it goes up one view), and the way through it
        let (name, year) = match cal.mode {
            0 => (memrium_de::i18n::tr(MONTHS[(cal.month as usize - 1) % 12]).to_string(), cal.year.to_string()),
            1 => (cal.year.to_string(), String::new()),
            _ => (format!("{} – {}", t0, t0 + 11), String::new()),
        };
        // (the shell's painter cannot measure text, the renderer can)
        let name_w = self.renderer.text_width(&name, 22.0, true);
        let year_w = if year.is_empty() { 0.0 } else { self.renderer.text_width(&year, 22.0, false) + 8.0 };
        let title_hit = [x + PAD, y + 18.0, name_w + year_w + 30.0, 38.0];
        let title_hot = ui.hit(title_hit);
        let wash = ui.ease(0xCA00, if title_hot { 1.0 } else { 0.0 }, 22.0);
        if wash > 0.01 {
            ui.fill(title_hit, 10.0, ink, 0.08 * wash);
        }
        ui.text_raw(x + PAD + 8.0, y + 45.0, &name, 22.0, true, ink, 1.0, Align::Left);
        if !year.is_empty() {
            ui.text_raw(x + PAD + 8.0 + name_w + 8.0, y + 45.0, &year, 22.0, false, ink, 0.45, Align::Left);
        }
        let tail = x + PAD + 8.0 + name_w + year_w + 14.0;
        ui.glyph(tail, y + 37.0, 14.0, if cal.mode == 0 { "chevron-down" } else { "chevron-up" }, ink, 0.5 + 0.3 * wash);
        if title_hot {
            ui.want_cursor(chebrium::ui::Cursor::Pointer);
            if ui.click {
                cal.mode = (cal.mode + 1) % 3;
            }
        }
        let next = [x + w - PAD - 32.0, y + 21.0, 32.0, 32.0];
        let today_btn = [next[0] - 6.0 - 68.0, next[1], 68.0, 32.0];
        let prev = [today_btn[0] - 6.0 - 32.0, next[1], 32.0, 32.0];
        if ui.icon_button(prev, "chevron-left", "", false) {
            cal.step(-1);
        }
        if ui.button(today_btn, "Today", false) {
            let sel = (today.year, today.month, today.day);
            *cal = Cal { mode: 0, year: today.year, month: today.month, sel: Some(sel) };
        }
        if ui.icon_button(next, "chevron-right", "", false) {
            cal.step(1);
        }

        let body_x = x + PAD;
        let body_w = w - 2.0 * PAD;
        match cal.mode {
            0 => {
                let cw = body_w / 7.0;
                for i in 0..7 {
                    let weekend = (i + if monday { 1 } else { 0 }) % 7 == 0 || (i + if monday { 1 } else { 0 }) % 7 == 6;
                    ui.text(body_x + cw * (i as f32 + 0.5), y + 92.0, SHORT[(i + if monday { 1 } else { 0 }) % 7], 11.5, true, ink, if weekend { 0.30 } else { 0.50 }, Align::Center);
                }
                let lead = (first_weekday(cal.year, cal.month) + if monday { 6 } else { 0 }) % 7;
                let (py, pm) = prev_month(cal.year, cal.month);
                let (ny, nm) = next_month(cal.year, cal.month);
                let (dim, pdim) = (days_in_month(cal.year, cal.month) as i32, days_in_month(py, pm) as i32);
                let mut go: Option<(i32, u32, u32)> = None;
                for i in 0..42i32 {
                    let d = i - lead as i32 + 1;
                    let (cy, cm, cd, mine) = if d < 1 {
                        (py, pm, (pdim + d) as u32, false)
                    } else if d > dim {
                        (ny, nm, (d - dim) as u32, false)
                    } else {
                        (cal.year, cal.month, d as u32, true)
                    };
                    let col = (i % 7) as usize;
                    let weekend = (col + if monday { 1 } else { 0 }) % 7 == 0 || (col + if monday { 1 } else { 0 }) % 7 == 6;
                    let r = [body_x + col as f32 * cw + 2.0, y + 104.0 + (i / 7) as f32 * 46.0, cw - 4.0, 42.0];
                    let is_today = (cy, cm, cd) == (today.year, today.month, today.day);
                    let selected = cal.sel == Some((cy, cm, cd));
                    let hot = ui.hit(r);
                    let wash = ui.ease(0xCA100 + i as u64, if hot && !is_today { 1.0 } else { 0.0 }, 22.0);
                    if is_today {
                        ui.fill(r, 13.0, today_colour, 1.0);
                    } else if selected {
                        ui.fill(r, 13.0, ink, 0.13);
                    } else if wash > 0.01 {
                        ui.fill(r, 13.0, ink, 0.075 * wash);
                    }
                    let text = if is_today { [1.0; 3] } else { ink };
                    let alpha = if is_today { 1.0 } else if !mine { 0.26 } else if weekend { 0.58 } else { 0.92 };
                    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, &cd.to_string(), 15.0, is_today || selected, text, alpha, Align::Center);
                    if hot && ui.click {
                        go = Some((cy, cm, cd));
                    }
                }
                if let Some(s) = go {
                    cal.sel = Some(s);
                    cal.year = s.0;
                    cal.month = s.1;
                }
            }
            1 => {
                let (cw, ch) = (body_w / 3.0, 72.0);
                for i in 0..12usize {
                    let r = [body_x + (i % 3) as f32 * cw + 3.0, y + 82.0 + (i / 3) as f32 * (ch + 6.0), cw - 6.0, ch];
                    let now_m = cal.year == today.year && i as u32 + 1 == today.month;
                    let hot = ui.hit(r);
                    let wash = ui.ease(0xCA200 + i as u64, if hot && !now_m { 1.0 } else { 0.0 }, 22.0);
                    if now_m {
                        ui.fill(r, 16.0, today_colour, 1.0);
                    } else if wash > 0.01 {
                        ui.fill(r, 16.0, ink, 0.08 * wash);
                    }
                    ui.text(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, MONTHS[i], 15.0, now_m, if now_m { [1.0; 3] } else { ink }, 0.95, Align::Center);
                    if hot && ui.click {
                        cal.month = i as u32 + 1;
                        cal.mode = 0;
                    }
                }
            }
            _ => {
                let (cw, ch) = (body_w / 3.0, 72.0);
                for i in 0..12i32 {
                    let yr = t0 + i;
                    let r = [body_x + (i % 3) as f32 * cw + 3.0, y + 82.0 + (i / 3) as f32 * (ch + 6.0), cw - 6.0, ch];
                    let now_y = yr == today.year;
                    let hot = ui.hit(r);
                    let wash = ui.ease(0xCA300 + i as u64, if hot && !now_y { 1.0 } else { 0.0 }, 22.0);
                    if now_y {
                        ui.fill(r, 16.0, today_colour, 1.0);
                    } else if wash > 0.01 {
                        ui.fill(r, 16.0, ink, 0.08 * wash);
                    }
                    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, &yr.to_string(), 15.0, now_y, if now_y { [1.0; 3] } else { ink }, 0.95, Align::Center);
                    if hot && ui.click {
                        cal.year = yr;
                        cal.mode = 1;
                    }
                }
            }
        }
        let _ = accent;
    }
}
