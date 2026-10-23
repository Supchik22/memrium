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

        // ---- title
        let title = match cal.mode {
            0 => format!("{} {}", memrium_de::i18n::tr(MONTHS[(cal.month as usize - 1) % 12]), cal.year),
            1 => cal.year.to_string(),
            _ => format!("{} – {}", t0, t0 + 11),
        };
        ui.text_raw(x + PAD + 4.0, y + 40.0, &title, 20.0, false, ink, 1.0, Align::Left);

        // ---- the three views, and the way through them
        let mut mode = cal.mode;
        if ui.segmented([x + PAD, y + 56.0, 252.0, 32.0], &["Days", "Months", "Years"], &mut mode) {
            cal.mode = mode;
        }
        let nav = [x + 276.0, y + 56.0, 30.0, 32.0];
        if ui.button(nav, "‹", false) {
            cal.step(-1);
        }
        if ui.button([x + 308.0, y + 56.0, 62.0, 32.0], "Today", false) {
            let sel = (today.year, today.month, today.day);
            *cal = Cal { mode: 0, year: today.year, month: today.month, sel: Some(sel) };
        }
        if ui.button([x + w - PAD - 30.0, y + 56.0, 30.0, 32.0], "›", false) {
            cal.step(1);
        }

        let body_x = x + PAD;
        let body_w = w - 2.0 * PAD;
        match cal.mode {
            0 => {
                let cw = body_w / 7.0;
                for i in 0..7 {
                    ui.text(body_x + cw * (i as f32 + 0.5), y + 118.0, SHORT[(i + if monday { 1 } else { 0 }) % 7], 12.0, true, ink, 0.55, Align::Center);
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
                    let r = [body_x + (i % 7) as f32 * cw + 2.0, y + 130.0 + (i / 7) as f32 * 45.0, cw - 4.0, 41.0];
                    let is_today = (cy, cm, cd) == (today.year, today.month, today.day);
                    let selected = cal.sel == Some((cy, cm, cd));
                    let hot = ui.hit(r);
                    if selected {
                        ui.fill(r, 9.0, accent, 0.85);
                    } else if is_today {
                        // (with several accents today is red, as on a wall calendar)
                        let today_colour = ui.tone(Tone::Danger);
                        ui.fill(r, 9.0, today_colour, 0.30);
                    } else if hot {
                        ui.fill(r, 9.0, ink, 0.10);
                    }
                    let col = if selected { [1.0; 3] } else { ink };
                    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, &cd.to_string(), 15.0, is_today || selected, col, if mine || selected { 0.95 } else { 0.35 }, Align::Center);
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
                    let r = [body_x + (i % 3) as f32 * cw + 3.0, y + 106.0 + (i / 3) as f32 * (ch + 6.0), cw - 6.0, ch];
                    let now_m = cal.year == today.year && i as u32 + 1 == today.month;
                    let hot = ui.hit(r);
                    if now_m {
                        ui.fill(r, 12.0, accent, 0.30);
                    } else if hot {
                        ui.fill(r, 12.0, ink, 0.10);
                    }
                    ui.text(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, MONTHS[i], 15.0, now_m, ink, 0.95, Align::Center);
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
                    let r = [body_x + (i % 3) as f32 * cw + 3.0, y + 106.0 + (i / 3) as f32 * (ch + 6.0), cw - 6.0, ch];
                    let now_y = yr == today.year;
                    let hot = ui.hit(r);
                    if now_y {
                        ui.fill(r, 12.0, accent, 0.30);
                    } else if hot {
                        ui.fill(r, 12.0, ink, 0.10);
                    }
                    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 5.5, &yr.to_string(), 15.0, now_y, ink, 0.95, Align::Center);
                    if hot && ui.click {
                        cal.year = yr;
                        cal.mode = 1;
                    }
                }
            }
        }
    }
}
