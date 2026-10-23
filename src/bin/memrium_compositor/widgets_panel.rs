//! The widgets panel (Super+W, or a "Widgets" button): a glass sheet that slides in from the left with the time,
//! a calendar, the machine's state, disk use and, when a city is set in Settings, the weather.

use crate::state::State;
use memrium_de::i18n::tr;
use memrium_de::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

const W: f32 = 430.0;

pub struct WidgetsPanel {
    pub k: Spring,
    open: bool,
    pub weather: Option<String>,
    /// the monitor it opened on
    pub mon: usize,
    weather_rx: Option<Receiver<String>>,
    fetched: Option<Instant>,
    fetched_for: String,
}

impl WidgetsPanel {
    pub fn new() -> Self {
        WidgetsPanel { k: Spring::new(0.0, 260.0, 0.82), open: false, mon: 0, weather: None, weather_rx: None, fetched: None, fetched_for: String::new() }
    }
}

pub struct Now {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub wday: u32,
}

pub fn now() -> Now {
    // SAFETY: valid pointers to initialised values
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        Now { year: tm.tm_year + 1900, month: tm.tm_mon as u32 + 1, day: tm.tm_mday as u32, hour: tm.tm_hour as u32, min: tm.tm_min as u32, wday: tm.tm_wday as u32 }
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 29 } else { 28 },
    }
}

/// Weekday of the 1st of a month, 0 = Sunday (Sakamoto's method).
pub fn first_weekday(year: i32, month: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + T[month as usize - 1] + 1) % 7) as u32
}

pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const SHORT: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

impl State {
    pub fn toggle_widgets(&mut self) {
        let w = &mut self.widgets_panel;
        w.open = !w.open;
        w.k.target = if w.open { 1.0 } else { 0.0 };
        if w.open {
            self.widgets_panel.mon = self.mon_at(self.mouse);
        }
    }

    pub fn widgets_open(&self) -> bool {
        self.widgets_panel.open
    }

    fn widgets_rect(&self) -> [f32; 4] {
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let (top, bottom) = if windows { (14.0, 62.0) } else { (46.0, 112.0) };
        let m = self.mons.get(self.widgets_panel.mon).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        [m[0] + 14.0, m[1] + top, W, m[3] - top - bottom]
    }

    pub fn widgets_hit(&self, p: [f32; 2]) -> bool {
        let r = self.widgets_rect();
        self.widgets_panel.open && p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
    }

    /// A press: a click outside closes the panel (and goes on to whatever is there).
    pub fn widgets_press(&mut self, p: [f32; 2]) -> bool {
        if !self.widgets_panel.open {
            return false;
        }
        if self.widgets_hit(p) {
            return true;
        }
        self.toggle_widgets();
        false
    }

    pub fn update_widgets(&mut self, dt: f32) {
        let city = self.config.weather_city.trim().to_string();
        let want_weather = self.config.desk_widgets.iter().any(|d| d.kind == "weather");
        let an = self.theme.animations.clone();
        let w = &mut self.widgets_panel;
        an.step("popups", &mut w.k, dt);
        if let Some(rx) = &w.weather_rx {
            if let Ok(text) = rx.try_recv() {
                w.weather = Some(text);
                w.weather_rx = None;
            }
        }
        // fetch the weather when the panel is open and it is stale (every 20 minutes), or the city changed
        let stale = w.fetched.map_or(true, |t| t.elapsed().as_secs() > 1200) || w.fetched_for != city;
        if (w.open || want_weather) && !city.is_empty() && stale && w.weather_rx.is_none() {
            w.fetched = Some(Instant::now());
            w.fetched_for = city.clone();
            let (tx, rx) = channel();
            w.weather_rx = Some(rx);
            std::thread::spawn(move || {
                let url = format!("https://wttr.in/{}?format=%c+%t+%C", city.replace(' ', "+"));
                let out = std::process::Command::new("curl").args(["-s", "-m", "8", &url]).output();
                let text = out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|t| !t.is_empty() && t.len() < 80 && !t.contains("Unknown")).unwrap_or_else(|| "Weather is not available".into());
                let _ = tx.send(text);
            });
        }
    }

    pub fn widgets_panels(&mut self, panels: &mut Vec<Panel>, labels: &mut Vec<Label>) {
        let k = self.widgets_panel.k.x.clamp(0.0, 1.0);
        if k < 0.003 {
            return;
        }
        let dark = self.theme.dark;
        let ink = if dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let r = self.widgets_rect();
        let slide = (1.0 - self.widgets_panel.k.x.min(1.0)) * -60.0;
        let (x, y) = (r[0] + slide, r[1]);
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let tint = if dark { [0.09, 0.09, 0.09, 0.78] } else { [0.97, 0.98, 1.0, 0.76] };
        let mut p = self.theme.glass.panel(x + W / 2.0, y + r[3] / 2.0, W, r[3], if windows { 12.0 } else { 26.0 }).tinted([tint[0], tint[1], tint[2], tint[3] * k]);
        p.extra[0] *= k;
        p.fx[0] = p.fx[0].max(28.0);
        if windows {
            p.params[3] = 0.0;
            p.fx = [p.fx[0].max(32.0), 0.0, 0.0, 0.0];
            p.color2 = if dark { [1.0, 1.0, 1.0, 0.10 * k] } else { [0.0, 0.0, 0.0, 0.10 * k] };
        }
        p.fade_by(k);
        panels.push(p);
        let card = |panels: &mut Vec<Panel>, cy: f32, h: f32| {
            panels.push(Panel::solid(x + W / 2.0, cy + h / 2.0, W - 24.0, h, 14.0, ink, ink).flat().alpha(0.07 * k));
        };
        let t = now();
        let mut cy = y + 12.0;
        let lx = x + 28.0;

        // ---- time and weather
        card(panels, cy, 104.0);
        labels.push(Label::new(format!("{:02}:{:02}", t.hour, t.min), lx, cy + 56.0, 44.0, true, ink, k, Align::Left));
        labels.push(Label::new(format!("{}, {} {}", tr(DAYS[t.wday as usize % 7]), t.day, tr(MONTHS[(t.month as usize - 1) % 12])), lx, cy + 86.0, 14.0, false, ink, 0.7 * k, Align::Left));
        let weather = match (&self.widgets_panel.weather, self.config.weather_city.trim().is_empty()) {
            (Some(w), _) => w.clone(),
            (None, false) => "Checking…".to_string(),
            (None, true) => "Set a city in Settings for the weather".to_string(),
        };
        labels.push(Label::new(weather, x + W - 28.0, cy + 86.0, 12.5, false, ink, 0.75 * k, Align::Right));
        cy += 116.0;

        // ---- calendar
        let ch = 262.0;
        card(panels, cy, ch);
        labels.push(Label::new(format!("{} {}", tr(MONTHS[(t.month as usize - 1) % 12]), t.year), lx, cy + 30.0, 15.0, true, ink, k, Align::Left));
        let cell = (W - 56.0) / 7.0;
        for (i, d) in SHORT.iter().enumerate() {
            labels.push(Label::new(d.to_string(), lx + i as f32 * cell + cell / 2.0, cy + 62.0, 11.5, true, ink, 0.5 * k, Align::Center));
        }
        let first = first_weekday(t.year, t.month);
        for day in 1..=days_in_month(t.year, t.month) {
            let idx = first + day - 1;
            let (col, row) = (idx % 7, idx / 7);
            let (dx, dy) = (lx + col as f32 * cell + cell / 2.0, cy + 90.0 + row as f32 * 29.0);
            if day == t.day {
                panels.push(Panel::solid(dx, dy - 4.0, 26.0, 26.0, 13.0, self.theme.accent, self.theme.accent).flat().alpha(k));
            }
            let today = day == t.day;
            labels.push(Label::new(day.to_string(), dx, dy + 0.5, 13.0, today, if today { [1.0; 3] } else { ink }, if today { k } else { 0.85 * k }, Align::Center));
        }
        cy += ch + 12.0;

        // ---- the machine
        card(panels, cy, 118.0);
        let sys = &self.sys;
        let rows: Vec<(String, String, Option<f32>)> = vec![
            (memrium_de::widget::status_icon("battery", sys), sys.battery.map_or("No battery".into(), |(p, c)| format!("Battery {p}%{}", if c { " (charging)" } else { "" })), sys.battery.map(|(p, _)| p as f32 / 100.0)),
            (memrium_de::widget::status_icon("volume", sys), sys.volume.map_or("Volume".into(), |v| format!("Volume {}%{}", (v * 100.0).round() as i32, if sys.muted { " (muted)" } else { "" })), sys.volume.map(|v| v.min(1.0))),
            ("sun".to_string(), sys.brightness.map_or("Brightness".into(), |b| format!("Brightness {}%", (b * 100.0).round() as i32)), sys.brightness),
        ];
        for (i, (icon, text, frac)) in rows.into_iter().enumerate() {
            let ry = cy + 26.0 + i as f32 * 32.0;
            panels.push(Panel::glyph(lx + 8.0, ry - 4.0, 16.0, &icon, ink, 0.85 * k));
            labels.push(Label::new(text, lx + 28.0, ry, 13.0, false, ink, 0.9 * k, Align::Left));
            if let Some(f) = frac {
                let (bx, bw) = (x + W - 28.0 - 120.0, 120.0);
                panels.push(Panel::solid(bx + bw / 2.0, ry - 4.0, bw, 5.0, 2.5, ink, ink).flat().alpha(0.15 * k));
                let fw = (bw * f.clamp(0.0, 1.0)).max(5.0);
                panels.push(Panel::solid(bx + fw / 2.0, ry - 4.0, fw, 5.0, 2.5, self.theme.accent, self.theme.accent).flat().alpha(k));
            }
        }
        cy += 130.0;

        // ---- disk
        if cy + 70.0 < y + r[3] {
            card(panels, cy, 70.0);
            if let Some(d) = memrium_de::fsops::drives().first() {
                let name = d.name.clone();
                labels.push(Label::new(name, lx, cy + 28.0, 13.0, false, ink, 0.9 * k, Align::Left));
                let total = d.size.max(1) as f32;
                // "free of" is in the name; the bar shows the used share from a fresh statvfs
                let used = used_fraction(&d.path).unwrap_or(0.0);
                let (bx, bw) = (lx, W - 56.0);
                panels.push(Panel::solid(bx + bw / 2.0, cy + 48.0, bw, 6.0, 3.0, ink, ink).flat().alpha(0.15 * k));
                let fw = (bw * used).max(6.0);
                panels.push(Panel::solid(bx + fw / 2.0, cy + 48.0, fw, 6.0, 3.0, self.theme.accent, self.theme.accent).flat().alpha(k));
                let _ = total;
            }
        }
    }
}

pub fn used_fraction(path: &std::path::Path) -> Option<f32> {
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    Some(1.0 - st.f_bavail as f32 / st.f_blocks as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_maths() {
        // 17 October 2026 is a Saturday, so the 1st is a Thursday
        assert_eq!(first_weekday(2026, 10), 4);
        assert_eq!(first_weekday(2000, 1), 6);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
    }
}
