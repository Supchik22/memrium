//! Widgets on the desktop: glass cards under the windows that can be added from the desktop's context menu,
//! dragged anywhere and removed again.

use crate::state::State;
use crate::widgets_panel::{days_in_month, first_weekday, now, DAYS, MONTHS, SHORT};
use memrium_de::deskwidgets::{self, DeskWidget, KINDS};
use memrium_de::i18n::tr;
use memrium_de::renderer::{Align, Label, Panel, Renderer};

/// The record: which cover is loaded, and how it is turning.
#[derive(Default)]
pub struct Album {
    url: String,
    pub cell: Option<u32>,
    rx: Option<std::sync::mpsc::Receiver<image::RgbaImage>>,
    pub angle: f32,
    omega: f32,
    /// the cover's main colour (the disc takes it), and the atlas cells of the turning sheen and the stand-in label
    pub color: Option<[f32; 3]>,
    sheen: Option<u32>,
    fallback: Option<u32>,
    /// the colours those two were drawn for
    drawn_for: ([f32; 3], [f32; 3]),
}

/// 33 1/3 turns a minute
const RPM_33: f32 = 33.333 / 60.0 * std::f32::consts::TAU;

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// A soft, tinted sheen that turns with the record: two opposite bright wedges outside the label.
fn sheen_image(c: [f32; 3]) -> image::RgbaImage {
    let n = 128u32;
    let mut img = image::RgbaImage::new(n, n);
    let tint = [(0.65 + 0.35 * c[0]), (0.65 + 0.35 * c[1]), (0.65 + 0.35 * c[2])];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0);
            let (r, th) = (dx.hypot(dy), dy.atan2(dx));
            let wedge = (2.0 * (th - 0.6)).cos().max(0.0).powi(5);
            let radial = ((r - 0.38) / 0.1).clamp(0.0, 1.0) * (1.0 - ((r - 0.9) / 0.1).clamp(0.0, 1.0));
            let a = (0.62 * wedge + 0.04) * radial;
            img.put_pixel(x, y, image::Rgba([(tint[0] * 255.0) as u8, (tint[1] * 255.0) as u8, (tint[2] * 255.0) as u8, (a * 255.0).clamp(0.0, 255.0) as u8]));
        }
    }
    img
}

/// A label for records whose cover is not known: the accent colour with stripes and a dot, so the turning shows.
fn stand_in_label(c: [f32; 3]) -> image::RgbaImage {
    let n = 128u32;
    let mut img = image::RgbaImage::new(n, n);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0);
            let (r, th) = (dx.hypot(dy), dy.atan2(dx));
            let k = 1.0 - 0.35 * r;
            let mut col = [c[0] * k, c[1] * k, c[2] * k];
            // three light spokes
            let spoke = ((3.0 * th).cos() - 0.55).max(0.0) * 1.6 * (1.0 - r).max(0.0) * ((r - 0.15) / 0.1).clamp(0.0, 1.0);
            for v in &mut col {
                *v = (*v + spoke * 0.35).min(1.0);
            }
            // a dot off to the side, and the hole
            if (dx - 0.58).hypot(dy) < 0.12 {
                col = [1.0, 1.0, 1.0];
            }
            if r < 0.07 {
                col = [0.05, 0.05, 0.06];
            }
            img.put_pixel(x, y, image::Rgba([(col[0] * 255.0) as u8, (col[1] * 255.0) as u8, (col[2] * 255.0) as u8, 255]));
        }
    }
    img
}

/// "13:05", or "1:05" on a 12-hour clock.
fn hhmm(t: &crate::widgets_panel::Now, h24: bool) -> String {
    if h24 {
        format!("{:02}:{:02}", t.hour, t.min)
    } else {
        format!("{}:{:02}", if t.hour % 12 == 0 { 12 } else { t.hour % 12 }, t.min)
    }
}

/// The cover as a square picture: a file:// path is read, an http(s):// address is fetched with curl.
fn load_cover(url: &str) -> Option<image::RgbaImage> {
    let bytes = if let Some(path) = url.strip_prefix("file://") {
        std::fs::read(percent_decode(path)).ok()?
    } else if let Some(data) = url.strip_prefix("data:") {
        // data:image/jpeg;base64,....: what browsers give as a cover
        let (head, body) = data.split_once(',')?;
        if !head.contains("base64") {
            return None;
        }
        base64_decode(body)?
    } else if url.starts_with("http") {
        let out = std::process::Command::new("curl").args(["-sL", "-m", "10", url]).output().ok().filter(|o| o.status.success())?;
        out.stdout
    } else {
        return None;
    };
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let s = img.width().min(img.height());
    Some(image::imageops::crop_imm(&img, (img.width() - s) / 2, (img.height() - s) / 2, s, s).to_image())
}

/// Menu ids: 310 + the index in `KINDS` adds one, `REMOVE` takes the one under the pointer away.
pub const REMOVE: u32 = 320;
pub const BIGGER: u32 = 321;
pub const SMALLER: u32 = 322;
/// 330 + the style's index
pub const STYLE_BASE: u32 = 330;
pub const H24: u32 = 340;
pub const DATE: u32 = 341;
pub const EDIT: u32 = 342;

impl State {
    fn desk_rect(&self, d: &DeskWidget) -> [f32; 4] {
        let s = deskwidgets::size(&d.kind);
        let k = if deskwidgets::scalable(&d.kind) { d.scale.clamp(0.5, 2.5) } else { 1.0 };
        [d.x, d.y, s[0] * k, s[1] * k]
    }

    pub fn desk_widget_at(&self, p: [f32; 2]) -> Option<usize> {
        self.config.desk_widgets.iter().rposition(|d| {
            let r = self.desk_rect(d);
            p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        })
    }

    /// A press on the desktop: a widget takes it (left starts a drag, right opens its menu). True when it did.
    pub fn desk_widget_press(&mut self, p: [f32; 2], right: bool) -> bool {
        let Some(i) = self.desk_widget_at(p) else { return false };
        let editing = self.scene.edit_mode();
        if editing && !right {
            // in edit mode a "−" on the corner removes the widget
            let [x, y, _, _] = self.desk_rect(&self.config.desk_widgets[i]);
            if (p[0] - (x + 14.0)).hypot(p[1] - (y + 14.0)) < 13.0 {
                self.config.desk_widgets.remove(i);
                self.cfg_dirty = Some(std::time::Instant::now());
                return true;
            }
        }
        if !right && !editing && self.config.desk_widgets[i].kind == "music" {
            let [x, y, w, _] = self.desk_rect(&self.config.desk_widgets[i]);
            // the three buttons along the bottom
            for (k, cmd) in [crate::media::MediaCmd::Previous, crate::media::MediaCmd::PlayPause, crate::media::MediaCmd::Next].into_iter().enumerate() {
                let bx = x + w / 2.0 + (k as f32 - 1.0) * 56.0;
                if (p[0] - bx).abs() < 24.0 && (p[1] - (y + 86.0)).abs() < 20.0 {
                    if let Some(m) = &self.media {
                        m.send(cmd);
                    }
                    return true;
                }
            }
        }
        if !right && !editing && self.config.desk_widgets[i].kind == "vinyl" {
            let [x, y, w, _] = self.desk_rect(&self.config.desk_widgets[i]);
            // a click on the record itself plays / pauses
            if (p[0] - (x + w / 2.0)).hypot(p[1] - (y + 150.0)) < 60.0 {
                if let Some(m) = &self.media {
                    m.send(crate::media::MediaCmd::PlayPause);
                }
                return true;
            }
        }
        if right {
            self.desk_target = Some(i);
            self.menu_for = crate::state::MenuFor::Desktop;
            self.menu.open_at(p, self.desk_widget_menu(i));
        } else if editing {
            // widgets only move in edit mode
            let d = &self.config.desk_widgets[i];
            self.desk_drag = Some((i, [p[0] - d.x, p[1] - d.y]));
            // the widget just grabbed goes on top
            let last = self.config.desk_widgets.len() - 1;
            self.config.desk_widgets.swap(i, last);
            self.desk_drag = Some((last, [p[0] - self.config.desk_widgets[last].x, p[1] - self.config.desk_widgets[last].y]));
            self.desk_drag_from = [self.config.desk_widgets[last].x, self.config.desk_widgets[last].y];
        } else {
            return false;
        }
        true
    }

    /// The menu of one widget: its settings first, then removal.
    fn desk_widget_menu(&self, i: usize) -> Vec<memrium_de::menu::MenuItem> {
        use memrium_de::menu::MenuItem;
        let d = &self.config.desk_widgets[i];
        let mut items = Vec::new();
        if d.kind == "glassclock" {
            let styles = deskwidgets::CLOCK_STYLES.iter().enumerate().map(|(k, name)| MenuItem::new(STYLE_BASE + k as u32, *name).checked(d.style as usize == k)).collect();
            items.push(MenuItem::new(0, "Style").icon("sliders-horizontal").sub(styles));
            items.push(MenuItem::new(BIGGER, "Bigger").icon("plus"));
            items.push(MenuItem::new(SMALLER, "Smaller").icon("minus"));
        }
        if d.kind == "visualizer" {
            let styles = deskwidgets::VIZ_STYLES.iter().enumerate().map(|(k, name)| MenuItem::new(STYLE_BASE + k as u32, *name).checked(d.style as usize == k)).collect();
            items.push(MenuItem::new(0, "Style").icon("sliders-horizontal").sub(styles));
            items.push(MenuItem::new(BIGGER, "Bigger").icon("plus"));
            items.push(MenuItem::new(SMALLER, "Smaller").icon("minus"));
        }
        if d.kind == "clock" || d.kind == "glassclock" {
            items.push(MenuItem::new(H24, "24-hour clock").checked(d.h24));
            items.push(MenuItem::new(DATE, "Show the date").checked(d.date));
        }
        if !items.is_empty() {
            items.push(MenuItem::sep());
        }
        items.push(MenuItem::new(EDIT, if self.scene.edit_mode() { "Exit Edit Mode" } else { "Enter Edit Mode" }).icon("pencil"));
        items.push(MenuItem::new(REMOVE, "Remove widget").icon("x"));
        items
    }

    pub fn desk_widget_motion(&mut self, p: [f32; 2]) {
        let Some((i, off)) = self.desk_drag else { return };
        if i >= self.config.desk_widgets.len() {
            return;
        }
        let m = self.keyboard.modifier_state();
        let mode = deskwidgets::Snap::from_keys(m.ctrl, m.shift, m.alt);
        let [_, _, w, h] = self.desk_rect(&self.config.desk_widgets[i]);
        let others: Vec<[f32; 4]> = self.config.desk_widgets.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, d)| self.desk_rect(d)).collect();
        let (pos, guides) = deskwidgets::snap(mode, [p[0] - off[0], p[1] - off[1]], [w, h], &others, self.world, self.desk_drag_from);
        let d = &mut self.config.desk_widgets[i];
        d.x = pos[0];
        d.y = pos[1];
        self.desk_guides = guides;
        self.desk_snap = mode;
    }

    pub fn desk_widget_release(&mut self) {
        self.desk_guides.clear();
        if self.desk_drag.take().is_some() {
            self.cfg_dirty = Some(std::time::Instant::now());
        }
    }

    /// Put a new widget of `kind` on the desktop: in the middle of the screen, nudged so it does not sit exactly on another.
    pub fn desk_add_widget(&mut self, kind: &str) {
        let size = deskwidgets::size(kind);
        let w = self.world;
        let mut at = [(w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0 - 40.0];
        for _ in 0..16 {
            let [x, y] = deskwidgets::place_at(kind, at, self.world);
            if self.config.desk_widgets.iter().any(|d| (d.x - x).abs() < 24.0 && (d.y - y).abs() < 24.0) {
                at = [at[0] + 32.0, at[1] + 32.0];
                continue;
            }
            self.config.desk_widgets.push(DeskWidget { kind: kind.to_string(), x, y, ..Default::default() });
            self.cfg_dirty = Some(std::time::Instant::now());
            return;
        }
        let [x, y] = deskwidgets::place_at(kind, at, self.world);
        let _ = size;
        self.config.desk_widgets.push(DeskWidget { kind: kind.to_string(), x, y, ..Default::default() });
        self.cfg_dirty = Some(std::time::Instant::now());
    }

    pub fn desk_menu_command(&mut self, id: u32) -> bool {
        if (STYLE_BASE..STYLE_BASE + deskwidgets::CLOCK_STYLES.len() as u32).contains(&id) {
            if let Some(w) = self.desk_target.take().and_then(|i| self.config.desk_widgets.get_mut(i)) {
                w.style = (id - STYLE_BASE) as u8;
                self.cfg_dirty = Some(std::time::Instant::now());
            }
            return true;
        }
        if id == H24 || id == DATE {
            if let Some(w) = self.desk_target.take().and_then(|i| self.config.desk_widgets.get_mut(i)) {
                if id == H24 {
                    w.h24 = !w.h24;
                } else {
                    w.date = !w.date;
                }
                self.cfg_dirty = Some(std::time::Instant::now());
            }
            return true;
        }
        if id == EDIT {
            let on = !self.scene.edit_mode();
            self.scene.set_edit(on);
            return true;
        }
        if id == BIGGER || id == SMALLER {
            if let Some(w) = self.desk_target.take().and_then(|i| self.config.desk_widgets.get_mut(i)) {
                w.scale = (w.scale * if id == BIGGER { 1.25 } else { 0.8 }).clamp(0.5, 2.5);
                self.cfg_dirty = Some(std::time::Instant::now());
            }
            return true;
        }
        if id == REMOVE {
            if let Some(i) = self.desk_target.take().filter(|&i| i < self.config.desk_widgets.len()) {
                self.config.desk_widgets.remove(i);
                self.cfg_dirty = Some(std::time::Instant::now());
            }
            return true;
        }
        false
    }

    /// Keep the record turning (up to speed while the music plays, coasting to a stop after) and the cover current.
    pub fn update_vinyl(&mut self, dt: f32) {
        // the cover is wanted by the record and by the Quick Settings's Now Playing tile
        let has_vinyl = self.config.desk_widgets.iter().any(|d| d.kind == "vinyl");
        // the Quick Settings turns a record too when its Now Playing tile is given that look
        let cc_record = self.scene.cc_open() && self.scene.cc_wants_record();
        if !has_vinyl && !self.scene.cc_open() {
            return;
        }
        if self.media.is_none() {
            self.media = Some(crate::media::Media::start());
        }
        let mut info = self.media.as_ref().map(|m| m.get()).unwrap_or_default();
        // test hook: a cover from a file, playing
        if let Ok(path) = std::env::var("MEMRIUM_TEST_COVER") {
            info = crate::media::MediaInfo { title: "Олівер_Дерево_Цибуля_часник_дуже_довга_назва".into(), artist: "The Glass Band Orchestra".into(), playing: true, art: format!("file://{path}"), present: true };
        }
        // a new cover: fetch it away from the main thread
        if info.art != self.album.url {
            self.album.url = info.art.clone();
            self.album.rx = None;
            if !info.art.is_empty() {
                let (tx, rx) = std::sync::mpsc::channel();
                let url = info.art.clone();
                std::thread::spawn(move || match load_cover(&url) {
                    Some(img) => {
                        let _ = tx.send(img);
                    }
                    None => eprintln!("vinyl: could not load the cover {url}"),
                });
                self.album.rx = Some(rx);
            } else {
                self.album.cell = None;
            }
        }
        if let Some(rx) = &self.album.rx {
            if let Ok(img) = rx.try_recv() {
                self.album.color = Some(memrium_de::wallpapers::accent_of(&img));
                self.album.cell = self.renderer.set_image_cell("album-art", &img);
                self.album.rx = None;
            }
        }
        if !has_vinyl && !cc_record {
            return;
        }
        // the pictures that turn with the desktop record, in the cover's colour
        if has_vinyl {
            let accent = self.theme.accent;
            let colour = if self.album.cell.is_some() { self.album.color.unwrap_or(accent) } else { accent };
            if self.album.sheen.is_none() || self.album.drawn_for != (colour, accent) {
                self.album.sheen = self.renderer.set_image_cell("vinyl-sheen", &sheen_image(colour));
                self.album.fallback = self.renderer.set_image_cell("vinyl-fallback", &stand_in_label(accent));
                self.album.drawn_for = (colour, accent);
            }
        }
        let target = if info.playing { RPM_33 } else { 0.0 };
        let a = &mut self.album;
        a.omega += (target - a.omega) * (dt * 2.5).min(1.0);
        if a.omega.abs() < 0.002 && target == 0.0 {
            a.omega = 0.0;
        }
        a.angle = (a.angle + a.omega * dt).rem_euclid(std::f32::consts::TAU);
    }

    /// Listen to the sound while a visualizer widget is on the desktop (the wallpaper may already be listening).
    pub fn update_visualizer(&mut self) {
        let wanted = self.config.desk_widgets.iter().any(|d| d.kind == "visualizer");
        if !wanted {
            self.viz_tap = None;
        } else if self.audio_tap.is_none() && self.viz_tap.is_none() {
            self.viz_tap = memrium_de::audiotap::AudioTap::start();
        } else if self.audio_tap.is_some() {
            self.viz_tap = None;
        }
    }

    /// The bars of the visualizer widget, 32 of them from low to high pitch (0..1).
    fn viz_bars(&self) -> [f32; 32] {
        let mut out = [0.0; 32];
        if let Some(t) = self.audio_tap.as_ref().or(self.viz_tap.as_ref()) {
            let s = t.spectrum();
            for (i, o) in out.iter_mut().enumerate() {
                *o = (s.bands[2 * i] + s.bands[2 * i + 1]) / 2.0;
            }
        }
        out
    }

    /// Panels and labels of every desktop widget.
    pub fn desk_widget_panels(&mut self) -> (Vec<Panel>, Vec<Label>) {
        let (mut panels, mut labels) = (Vec::new(), Vec::new());
        if self.media.is_none() && self.config.desk_widgets.iter().any(|d| d.kind == "music") {
            self.media = Some(crate::media::Media::start());
        }
        if self.config.desk_widgets.is_empty() {
            return (panels, labels);
        }
        let dark = self.theme.dark;
        let ink = if dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let tint = if dark { [0.09, 0.09, 0.09, 0.66] } else { [0.97, 0.98, 1.0, 0.62] };
        let accent = self.theme.accent;
        let t = now();
        let sys = self.sys.clone();
        let sys = &sys;
        let widgets = self.config.desk_widgets.clone();
        for d in &widgets {
            let [x, y, w, h] = self.desk_rect(d);
            if d.kind == "glassclock" {
                // no card: the digits are the glass, over the wallpaper
                let k = d.scale.clamp(0.5, 2.5);
                let big = Label::new(hhmm(&t, d.h24), x + w / 2.0, y + 150.0 * k, 150.0 * k, true, [1.0; 3], 1.0, Align::Center);
                // the three looks: smooth Jelly Glass, faceted Crystal, and a frosted, brighter slab
                let (gs, tint, liquid) = match d.style {
                    1 => (self.theme.glass.lettering(), [0.92, 0.96, 1.0, 0.17], false),
                    2 => (self.theme.glass.lettering_liquid(), [0.96, 0.98, 1.0, 0.42], true),
                    _ => (self.theme.glass.lettering_liquid(), [0.94, 0.97, 1.0, 0.20], true),
                };
                panels.extend(self.renderer.glass_labels(&[big], &gs, tint, liquid));
                if d.date {
                    // the date sits on a faint dark pill so it can be read over any wallpaper
                    let mut pill = self.theme.glass.panel(x + w / 2.0, y + 194.0 * k, 230.0 * k.max(0.8), 34.0 * k.max(0.8), 17.0 * k.max(0.8)).tinted([0.06, 0.08, 0.12, 0.30]);
                    pill.fx[0] = pill.fx[0].max(18.0);
                    pill.extra[0] *= 0.6;
                    panels.push(pill);
                    labels.push(Label::new(format!("{}, {} {}", tr(DAYS[t.wday as usize % 7]), t.day, tr(MONTHS[(t.month as usize - 1) % 12])), x + w / 2.0, y + 200.0 * k, 18.0 * k.max(0.8), true, [1.0; 3], 0.85, Align::Center));
                }
                continue;
            }
            let viz = if d.kind == "visualizer" { Some(self.viz_bars()) } else { None };
            let mut p = self.theme.glass.panel(x + w / 2.0, y + h / 2.0, w, h, if windows { 10.0 } else { 24.0 }).tinted(tint);
            p.fx[0] = p.fx[0].max(24.0);
            if windows {
                p.params[3] = 0.0;
                p.fx = [p.fx[0].max(30.0), 0.0, 0.0, 0.0];
                p.color2 = if dark { [1.0, 1.0, 1.0, 0.10] } else { [0.0, 0.0, 0.0, 0.10] };
            }
            // the record is the widget: no card behind it
            if d.kind != "vinyl" {
                panels.push(p);
            }
            let (lx, rx) = (x + 20.0, x + w - 20.0);
            match d.kind.as_str() {
                "visualizer" => {
                    let bars = viz.unwrap_or([0.0; 32]);
                    let k = d.scale.clamp(0.5, 2.5);
                    let (area_w, area_h) = (w - 40.0 * k, h - 36.0 * k);
                    let (x0, floor) = (x + 20.0 * k, y + h - 18.0 * k);
                    let step = area_w / 32.0;
                    for (i, v) in bars.iter().enumerate() {
                        let cx = x0 + step * (i as f32 + 0.5);
                        let col = |a: f32| [accent[0] + (1.0 - accent[0]) * a, accent[1] + (1.0 - accent[1]) * a, accent[2] + (1.0 - accent[2]) * a];
                        let bw = (step * 0.62).max(2.0);
                        match d.style {
                            // bars from the floor
                            0 => {
                                let bh = (area_h * v).max(bw);
                                panels.push(Panel::solid(cx, floor - bh / 2.0, bw, bh, bw / 2.0, col(0.45 * v), col(0.0)).flat());
                            }
                            // mirrored around the middle
                            1 => {
                                let bh = (area_h * v).max(bw);
                                panels.push(Panel::solid(cx, y + h / 2.0, bw, bh, bw / 2.0, col(0.45 * v), col(0.1)).flat());
                            }
                            // one dot per bar riding at the level, with a faint trail
                            _ => {
                                let dy = floor - area_h * v;
                                panels.push(Panel::solid(cx, (dy + floor) / 2.0, bw * 0.35, (floor - dy).max(1.0), bw * 0.17, accent, accent).flat().alpha(0.25));
                                panels.push(Panel::solid(cx, dy, bw, bw, bw / 2.0, col(0.5 * v), col(0.5 * v)).flat());
                            }
                        }
                    }
                }
                "clock" => {
                    labels.push(Label::new(hhmm(&t, d.h24), lx, y + 70.0, 52.0, true, ink, 1.0, Align::Left));
                    if d.date {
                        labels.push(Label::new(format!("{}, {} {}", tr(DAYS[t.wday as usize % 7]), t.day, tr(MONTHS[(t.month as usize - 1) % 12])), lx, y + 98.0, 14.0, false, ink, 0.7, Align::Left));
                    }
                }
                "calendar" => {
                    labels.push(Label::new(format!("{} {}", tr(MONTHS[(t.month as usize - 1) % 12]), t.year), lx, y + 34.0, 15.0, true, ink, 1.0, Align::Left));
                    let cell = (w - 40.0) / 7.0;
                    for (i, n) in SHORT.iter().enumerate() {
                        labels.push(Label::new(n.to_string(), lx + i as f32 * cell + cell / 2.0, y + 64.0, 11.5, true, ink, 0.5, Align::Center));
                    }
                    let first = first_weekday(t.year, t.month);
                    for day in 1..=days_in_month(t.year, t.month) {
                        let idx = first + day - 1;
                        let (dx, dy) = (lx + (idx % 7) as f32 * cell + cell / 2.0, y + 92.0 + (idx / 7) as f32 * 29.0);
                        let today = day == t.day;
                        if today {
                            panels.push(Panel::solid(dx, dy - 4.0, 26.0, 26.0, 13.0, accent, accent).flat());
                        }
                        labels.push(Label::new(day.to_string(), dx, dy + 0.5, 13.0, today, if today { [1.0; 3] } else { ink }, if today { 1.0 } else { 0.85 }, Align::Center));
                    }
                }
                "system" => {
                    let rows: [(String, String, Option<f32>); 3] = [
                        (memrium_de::widget::status_icon("battery", sys), sys.battery.map_or("No battery".into(), |(p, c)| format!("Battery {p}%{}", if c { " charging" } else { "" })), sys.battery.map(|(p, _)| p as f32 / 100.0)),
                        (memrium_de::widget::status_icon("volume", sys), sys.volume.map_or("Volume".into(), |v| format!("Volume {}%", (v * 100.0).round() as i32)), sys.volume.map(|v| v.min(1.0))),
                        ("sun".to_string(), sys.brightness.map_or("Brightness".into(), |b| format!("Brightness {}%", (b * 100.0).round() as i32)), sys.brightness),
                    ];
                    for (i, (icon, text, frac)) in rows.into_iter().enumerate() {
                        let ry = y + 32.0 + i as f32 * 34.0;
                        panels.push(Panel::glyph(lx + 8.0, ry - 4.0, 16.0, &icon, ink, 0.85));
                        labels.push(Label::new(text, lx + 28.0, ry, 13.0, false, ink, 0.9, Align::Left));
                        if let Some(f) = frac {
                            let (bx, bw) = (rx - 80.0, 80.0);
                            panels.push(Panel::solid(bx + bw / 2.0, ry - 4.0, bw, 5.0, 2.5, ink, ink).flat().alpha(0.15));
                            let fw = (bw * f.clamp(0.0, 1.0)).max(5.0);
                            panels.push(Panel::solid(bx + fw / 2.0, ry - 4.0, fw, 5.0, 2.5, accent, accent).flat());
                        }
                    }
                }
                "disk" => {
                    if let Some(disk) = memrium_de::fsops::drives().first() {
                        let short = disk.name.rsplit_once(" - ").map_or(disk.name.clone(), |(_, tail)| tail.to_string());
                        labels.push(Label::new(short, lx, y + 34.0, 13.0, false, ink, 0.9, Align::Left));
                        let used = crate::widgets_panel::used_fraction(&disk.path).unwrap_or(0.0);
                        let bw = w - 40.0;
                        panels.push(Panel::solid(lx + bw / 2.0, y + 56.0, bw, 6.0, 3.0, ink, ink).flat().alpha(0.15));
                        let fw = (bw * used).max(6.0);
                        panels.push(Panel::solid(lx + fw / 2.0, y + 56.0, fw, 6.0, 3.0, accent, accent).flat());
                    }
                }
                "vinyl" => {
                    let (cx, cy, r) = (x + w / 2.0, y + 150.0, 140.0);
                    let mut info = self.media.as_ref().map(|m| m.get()).unwrap_or_default();
                    if std::env::var_os("MEMRIUM_TEST_COVER").is_some() {
                        info = crate::media::MediaInfo { title: "Олівер_Дерево_Цибуля_часник_дуже_довга_назва".into(), artist: "The Glass Band Orchestra".into(), playing: true, art: String::new(), present: true };
                    }
                    // the record takes the cover's colour: a dark vinyl with a tint of it
                    let tone = if self.album.cell.is_some() { self.album.color.unwrap_or(accent) } else { accent };
                    let shade = |v: f32| [tone[0] * v, tone[1] * v, tone[2] * v];
                    panels.push(Panel::solid(cx, cy, 2.0 * r, 2.0 * r, r, shade(0.10), shade(0.10)).flat().shadow(0.45, 28.0, 12.0));
                    let mut rad = r - 3.0;
                    let mut i = 0;
                    while rad > 62.0 {
                        let v = 0.10 + 0.10 * ((i as f32 * 1.7).sin() * 0.5 + 0.5) + 0.05 * (((i * 7) % 5) as f32 / 5.0);
                        panels.push(Panel::solid(cx, cy, 2.0 * rad, 2.0 * rad, rad, shade(v), shade(v)).flat());
                        rad -= 3.2 + (i % 3) as f32 * 0.8;
                        i += 1;
                    }
                    // the smooth ring by the label
                    panels.push(Panel::solid(cx, cy, 2.0 * 62.0, 2.0 * 62.0, 62.0, shade(0.07), shade(0.07)).flat());
                    // light wedges that turn with the record
                    if let Some(c) = self.album.sheen {
                        let mut sh = Panel::app_icon(cx, cy, 2.0 * (r - 2.0), c, 1.0);
                        sh.params[3] = 2.0;
                        sh.fx[2] = self.album.angle;
                        panels.push(sh);
                    }
                    // the label: the cover (or a stand-in), turning
                    let label = self.album.cell.or(self.album.fallback);
                    if let Some(cell) = label {
                        let mut a = Panel::app_icon(cx, cy, 112.0, cell, 1.0);
                        a.params[3] = 2.0;
                        a.fx[2] = self.album.angle;
                        panels.push(a);
                    }
                    // a rim of light that stays where the light is, and the spindle
                    panels.push(Panel::solid(cx - r * 0.38, cy - r * 0.38, r * 0.9, r * 0.9, r * 0.45, [1.0; 3], [1.0; 3]).flat().alpha(0.03));
                    panels.push(Panel::solid(cx, cy, 10.0, 10.0, 5.0, [0.88, 0.89, 0.93], [0.55, 0.56, 0.6]).flat());
                    let (title, artist) = if info.present { (info.title.clone(), info.artist.clone()) } else { ("Nothing is playing".to_string(), String::new()) };
                    // the texts are cut to the widget's width by their real size, and the pill fits what is left
                    let fit = |r: &Renderer, text: &str, size: f32, bold: bool, max: f32| -> String {
                        let mut t = text.to_string();
                        if r.text_width(&t, size, bold) <= max {
                            return t;
                        }
                        while !t.is_empty() && r.text_width(&format!("{t}…"), size, bold) > max {
                            t.pop();
                        }
                        format!("{t}…")
                    };
                    let title = fit(&self.renderer, &title, 15.0, true, 250.0);
                    let artist = fit(&self.renderer, &artist, 12.5, false, 250.0);
                    let widest = self.renderer.text_width(&title, 15.0, true).max(if artist.is_empty() { 0.0 } else { self.renderer.text_width(&artist, 12.5, false) });
                    let (pw, ph) = ((widest + 44.0).clamp(150.0, 296.0), if artist.is_empty() { 38.0 } else { 56.0 });
                    let pcy = y + 326.0;
                    // a pill of Jelly Glass, tinted with the cover's colour, under the title
                    let mut pill = self.theme.glass.panel(cx, pcy, pw, ph, ph / 2.0).tinted([tone[0] * 0.35, tone[1] * 0.35, tone[2] * 0.35, 0.40]);
                    pill.fx[0] = pill.fx[0].max(18.0);
                    pill.extra[0] *= 0.7;
                    panels.push(pill);
                    if artist.is_empty() {
                        labels.push(Label::new(title, cx, pcy + 5.5, 15.0, true, [1.0; 3], 0.95, Align::Center));
                    } else {
                        labels.push(Label::new(title, cx, pcy - 4.0, 15.0, true, [1.0; 3], 0.95, Align::Center));
                        labels.push(Label::new(artist, cx, pcy + 14.0, 12.5, false, [1.0; 3], 0.7, Align::Center));
                    }
                }
                "music" => {
                    let info = self.media.as_ref().map(|m| m.get()).unwrap_or_default();
                    let (title, artist) = if info.present { (info.title.clone(), info.artist.clone()) } else { ("Nothing is playing".to_string(), String::new()) };
                    let short = |s: &str, n: usize| if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() };
                    panels.push(Panel::glyph(lx + 22.0, y + 36.0, 34.0, "music-2", ink, 0.9));
                    labels.push(Label::new(short(&title, 26), lx + 52.0, y + 34.0, 14.5, true, ink, 1.0, Align::Left));
                    labels.push(Label::new(short(&artist, 30), lx + 52.0, y + 54.0, 12.5, false, ink, 0.65, Align::Left));
                    // previous, play / pause, next
                    for (i, icon) in ["skip-back", if info.playing { "pause" } else { "play" }, "skip-forward"].into_iter().enumerate() {
                        let bx = x + w / 2.0 + (i as f32 - 1.0) * 56.0;
                        panels.push(Panel::glyph(bx, y + 86.0, if i == 1 { 24.0 } else { 20.0 }, icon, ink, if info.present { 0.95 } else { 0.35 }));
                    }
                }
                "weather" => {
                    let text = match (&self.widgets_panel.weather, self.config.weather_city.trim().is_empty()) {
                        (Some(t), _) => t.clone(),
                        (None, false) => "Checking…".to_string(),
                        (None, true) => "Set a city in Settings".to_string(),
                    };
                    labels.push(Label::new(self.config.weather_city.trim().to_string(), lx, y + 30.0, 12.5, true, ink, 0.7, Align::Left));
                    labels.push(Label::new(text, lx, y + 60.0, 16.0, true, ink, 1.0, Align::Left));
                }
                _ => {}
            }
        }
        // in edit mode every widget shows a frame to grab and a "−" to remove it
        if self.scene.edit_mode() {
            let accent = self.theme.accent;
            for d in &widgets {
                let [x, y, w, h] = self.desk_rect(d);
                for (ex, ey, ew, eh) in [(x, y, w, 2.0), (x, y + h - 2.0, w, 2.0), (x, y, 2.0, h), (x + w - 2.0, y, 2.0, h)] {
                    panels.push(Panel::solid(ex + ew / 2.0, ey + eh / 2.0, ew, eh, 1.0, accent, accent).flat().alpha(0.85));
                }
                for (cx, cy) in [(x + w, y), (x, y + h), (x + w, y + h)] {
                    panels.push(Panel::solid(cx, cy, 12.0, 12.0, 6.0, [1.0; 3], [1.0; 3]).flat());
                    panels.push(Panel::solid(cx, cy, 8.0, 8.0, 4.0, accent, accent).flat());
                }
                panels.push(Panel::glass(x + 14.0, y + 14.0, 22.0, 22.0, 11.0).tinted([0.30, 0.33, 0.42, 0.78]).shadow(0.25, 6.0, 2.0));
                panels.push(Panel::glyph(x + 14.0, y + 14.0, 13.0, "minus", [1.0; 3], 1.0));
            }
        }
        // while a widget is dragged: the lines it is pulled onto, and which way of pulling the keys have chosen
        if let Some((i, _)) = self.desk_drag {
            for g in &self.desk_guides {
                let (cx, cy, gw, gh) = if g.vertical { (g.at, (g.a + g.b) / 2.0, 1.0, g.b - g.a) } else { ((g.a + g.b) / 2.0, g.at, g.b - g.a, 1.0) };
                panels.push(Panel::solid(cx, cy, gw.max(1.0), gh.max(1.0), 0.5, [1.0, 0.25, 0.55], [1.0, 0.25, 0.55]).flat().alpha(0.9));
            }
            if let Some(d) = self.config.desk_widgets.get(i) {
                let [x, y, w, _] = self.desk_rect(d);
                let text = format!("{} · {}", tr(self.desk_snap.label()), match self.desk_snap {
                    deskwidgets::Snap::Smart => "Ctrl grid · Shift line · Ctrl+Shift gap · Alt free",
                    _ => "release the key for Align",
                });
                let cw = (text.chars().count() as f32 * 6.4 + 24.0).min(w.max(320.0));
                let cy = (y - 20.0).max(self.world[1] + 14.0);
                panels.push(self.theme.glass.panel(x + cw / 2.0, cy, cw, 24.0, 12.0).tinted([0.08, 0.09, 0.12, 0.72]));
                labels.push(Label::new(text, x + 12.0, cy + 4.5, 11.5, true, [1.0; 3], 0.95, Align::Left));
            }
        }
        (panels, labels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_covers_decode() {
        assert_eq!(base64_decode("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64_decode("aGVs\nbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64_decode("a*b"), None);
    }

    #[test]
    fn urls_are_decoded() {
        assert_eq!(percent_decode("/a%20b/c%C3%A9.png"), "/a b/cé.png");
    }
}
