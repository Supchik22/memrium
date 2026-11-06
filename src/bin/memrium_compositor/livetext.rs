//! Live Text for the whole screen (the Copilot key): the screen is frozen, the text on it is read on a worker thread
//! (`memrium_de::ocr`, the image viewer's reader), and every line found is lit so it can be selected by dragging over
//! it, like text in a document. Ctrl+C or Enter copies what is selected (all the text when nothing is), Ctrl+A selects
//! everything, Esc or a right click closes. QR codes on the screen are read too: each is ringed with what it says, and a
//! click opens its link (or copies its text). A click anywhere else lifts the object under it out (MobileSAM, as the
//! image viewer's Alt + drag does) and copies it as a picture with a see-through background.

use crate::state::State;
use memrium_de::ocr;
use memrium_de::renderer::{Align, Label, Panel, SurfaceTex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

/// A place in the text: (line, character).
type Caret = (usize, usize);

/// A QR code on the frozen screen: its box (picture px: x0, y0, x1, y1) and what it says.
pub struct Qr {
    bbox: [f32; 4],
    text: String,
}

/// What the worker finds: the QR codes first (they are quick), then the lines of text.
enum Found {
    Qr(Vec<Qr>),
    Lines(Result<Vec<ocr::Line>, String>),
}

/// What the object worker answers.
enum Lift {
    Done(memrium_de::cutout::Cutout),
    Nothing,
    Failed(String),
}

/// The QR codes in a picture (straight RGBA).
fn find_qr(rgba: &[u8], w: usize, h: usize) -> Vec<Qr> {
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
        let i = (y * w + x) * 4;
        ((rgba[i] as u32 * 77 + rgba[i + 1] as u32 * 150 + rgba[i + 2] as u32 * 29) >> 8) as u8
    });
    img.detect_grids()
        .into_iter()
        .filter_map(|g| {
            let (_, text) = g.decode().ok()?;
            let xs = g.bounds.iter().map(|p| p.x as f32);
            let ys = g.bounds.iter().map(|p| p.y as f32);
            let bbox = [xs.clone().fold(f32::MAX, f32::min), ys.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max), ys.fold(f32::MIN, f32::max)];
            Some(Qr { bbox, text })
        })
        .collect()
}

pub struct LiveShot {
    pub tex: SurfaceTex,
    /// the frozen picture's size (device px) and the desktop it covers (logical: x0, y0, x1, y1)
    size: [f32; 2],
    world: [f32; 4],
    lines: Option<Vec<ocr::Line>>,
    qrs: Vec<Qr>,
    rx: Receiver<Found>,
    error: Option<String>,
    sel: Option<(Caret, Caret)>,
    dragging: bool,
    started: Instant,
    /// the frozen picture (straight RGBA), for lifting objects out of it
    rgba: Arc<Vec<u8>>,
    /// the object worker, started by the first click off the text: points go in, objects come out
    lift_tx: Option<Sender<[f32; 2]>>,
    lift_rx: Option<Receiver<Lift>>,
    lifting: bool,
    /// an object just copied: its box (picture px) is lit for a moment before the screen closes
    lifted: Option<([f32; 4], Instant)>,
    note: Option<String>,
}

impl LiveShot {
    /// A desktop point (logical) in the picture's pixels, and back.
    fn to_pic(&self, p: [f32; 2]) -> [f32; 2] {
        let [x0, y0, x1, y1] = self.world;
        [(p[0] - x0) / (x1 - x0) * self.size[0], (p[1] - y0) / (y1 - y0) * self.size[1]]
    }

    fn to_desk(&self, r: [f32; 4]) -> [f32; 4] {
        let [x0, y0, x1, y1] = self.world;
        let (kx, ky) = ((x1 - x0) / self.size[0], (y1 - y0) / self.size[1]);
        [x0 + r[0] * kx, y0 + r[1] * ky, r[2] * kx, r[3] * ky]
    }

    /// Is a desktop point on (or right next to) a line of text?
    fn on_text(&self, p: [f32; 2]) -> bool {
        let p = self.to_pic(p);
        self.lines.as_ref().is_some_and(|lines| {
            lines.iter().any(|l| {
                let pad = l.rect[3] * 0.4;
                p[0] >= l.rect[0] - pad && p[0] <= l.rect[0] + l.rect[2] + pad && p[1] >= l.rect[1] - pad && p[1] <= l.rect[1] + l.rect[3] + pad
            })
        })
    }

    /// Lift the object under a desktop point: the worker is started the first time (it needs a second to look at the
    /// picture), then each point costs a few tens of milliseconds.
    fn lift(&mut self, p: [f32; 2]) {
        if self.lift_tx.is_none() {
            let (tx, jobs) = channel::<[f32; 2]>();
            let (done, rx) = channel();
            let (rgba, w, h) = (self.rgba.clone(), self.size[0] as usize, self.size[1] as usize);
            std::thread::spawn(move || {
                use memrium_de::cutout;
                if !cutout::models_present() {
                    let _ = done.send(Lift::Failed("the object models are not downloaded (open a picture and Alt-drag once)".into()));
                    return;
                }
                let seg = match cutout::Segmenter::load(&cutout::model_dir()) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = done.send(Lift::Failed(e));
                        return;
                    }
                };
                let emb = match seg.embed(&rgba, w, h) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = done.send(Lift::Failed(e));
                        return;
                    }
                };
                while let Ok(at) = jobs.recv() {
                    // (a whole window is not the object: at most a quarter of the screen)
                    let _ = done.send(match seg.cut_within(&emb, &rgba, at, 0.25) {
                        Ok(Some(c)) => Lift::Done(c),
                        Ok(None) => Lift::Nothing,
                        Err(e) => Lift::Failed(e),
                    });
                }
            });
            self.lift_tx = Some(tx);
            self.lift_rx = Some(rx);
        }
        if let Some(tx) = &self.lift_tx {
            let _ = tx.send(self.to_pic(p));
            self.lifting = true;
            self.note = None;
        }
    }

    /// The caret under a desktop point, or the nearest one (a drag that wanders off the lines).
    fn caret(&self, p: [f32; 2]) -> Option<Caret> {
        let lines = self.lines.as_ref()?;
        let p = self.to_pic(p);
        let dist = |l: &ocr::Line| {
            let (y0, y1) = (l.rect[1], l.rect[1] + l.rect[3]);
            let dy = if p[1] < y0 { y0 - p[1] } else if p[1] > y1 { p[1] - y1 } else { 0.0 };
            let (x0, x1) = (l.rect[0], l.rect[0] + l.rect[2]);
            let dx = if p[0] < x0 { x0 - p[0] } else if p[0] > x1 { p[0] - x1 } else { 0.0 };
            dy * 4.0 + dx
        };
        let (i, l) = lines.iter().enumerate().min_by(|a, b| dist(a.1).total_cmp(&dist(b.1)))?;
        Some((i, l.caret_at(p[0])))
    }

    fn ordered(&self) -> Option<(Caret, Caret)> {
        let (a, b) = self.sel?;
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The selected pieces: (line, first character, end). A screen has columns (a sidebar beside a page), and the lines
    /// are listed top to bottom across all of them, so a line between the two ends counts only when it lies across the
    /// same stretch as the ends do.
    fn picked(&self) -> Vec<(usize, usize, usize)> {
        let (Some((a, b)), Some(lines)) = (self.ordered(), self.lines.as_ref()) else { return Vec::new() };
        let (la, lb) = (&lines[a.0], &lines[b.0]);
        let (x0, x1) = (la.rect[0].min(lb.rect[0]), (la.rect[0] + la.rect[2]).max(lb.rect[0] + lb.rect[2]));
        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate().take(b.0 + 1).skip(a.0) {
            let across = line.rect[0] < x1 && line.rect[0] + line.rect[2] > x0;
            if i != a.0 && i != b.0 && !across {
                continue;
            }
            let n = line.spans.len();
            let from = if i == a.0 { a.1 } else { 0 }.min(n);
            let to = if i == b.0 { b.1 } else { n }.min(n);
            if to > from {
                out.push((i, from, to));
            }
        }
        out
    }

    /// The selected text, lines joined by line breaks; all of it when nothing is selected.
    fn text(&self) -> Option<String> {
        let lines = self.lines.as_ref()?;
        let picked = self.picked();
        if picked.is_empty() {
            return (!lines.is_empty()).then(|| lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"));
        }
        let pieces: Vec<String> = picked.iter().map(|&(i, from, to)| lines[i].text.chars().skip(from).take(to - from).collect()).collect();
        Some(pieces.join("\n"))
    }

    /// The pieces of the lines the selection covers (picture px).
    fn selected_rects(&self) -> Vec<[f32; 4]> {
        let Some(lines) = self.lines.as_ref() else { return Vec::new() };
        self.picked()
            .into_iter()
            .map(|(i, from, to)| {
                let l = &lines[i];
                let (x0, x1) = (l.spans[from][0], l.spans[to - 1][1]);
                [x0, l.rect[1], x1 - x0, l.rect[3]]
            })
            .collect()
    }
}

impl State {
    /// The Copilot key: freeze the screen and start reading its text.
    pub fn begin_live_text(&mut self) {
        if self.live_shot.is_some() || self.lock.is_some() {
            self.live_shot = None;
            return;
        }
        self.shot_sel = None;
        let (w, h, rgba) = self.capture_rgba_hi(None, true);
        let tex = self.renderer.create_surface_tex(w, h, true);
        let bgra: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        self.renderer.upload_surface_rows(&tex, &bgra, w * 4, 0..h);
        let (tx, rx) = channel();
        let rgba = Arc::new(rgba);
        let pixels = rgba.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Found::Qr(find_qr(&rgba, w as usize, h as usize)));
            let read = || -> Result<Vec<ocr::Line>, String> {
                if !memrium_de::cutout::onnxruntime() {
                    return Err("onnxruntime is not installed".into());
                }
                if !ocr::models_present() {
                    ocr::fetch_models()?;
                }
                ocr::Reader::load(&ocr::model_dir())?.read(&rgba, w as usize, h as usize)
            };
            let _ = tx.send(Found::Lines(read()));
        });
        self.live_shot = Some(LiveShot { tex, size: [w as f32, h as f32], world: self.world, lines: None, qrs: Vec::new(), rx, error: None, sel: None, dragging: false, started: Instant::now(), rgba: pixels, lift_tx: None, lift_rx: None, lifting: false, lifted: None, note: None });
        self.invalidate();
    }

    /// Take the reader's answer when it comes.
    pub fn live_text_tick(&mut self) {
        let Some(ls) = self.live_shot.as_mut() else { return };
        if let Some((_, at)) = ls.lifted {
            if at.elapsed().as_secs_f32() > 1.1 {
                self.live_shot = None;
                self.shot_notice = Some(("Object copied: Ctrl+V pastes it".into(), 3.0));
                self.invalidate();
            }
            return;
        }
        let mut lifted = None;
        if let Some(rx) = &ls.lift_rx {
            while let Ok(m) = rx.try_recv() {
                ls.lifting = false;
                match m {
                    Lift::Done(c) => lifted = Some(c),
                    Lift::Nothing => ls.note = Some("Nothing to lift there".into()),
                    Lift::Failed(e) => ls.note = Some(format!("Could not lift it: {e}")),
                }
            }
        }
        if let Some(c) = lifted {
            let (w, h) = (c.image.width(), c.image.height());
            ls.lifted = Some(([c.origin[0] as f32, c.origin[1] as f32, w as f32, h as f32], Instant::now()));
            if let Ok(png) = memrium_de::shots::ScreenshotPrefs::png(c.image.as_raw(), w, h) {
                self.clip_set_image(png, w, h);
            }
            self.invalidate();
            return;
        }
        let mut got = false;
        while let Ok(f) = ls.rx.try_recv() {
            match f {
                Found::Qr(q) => ls.qrs = q,
                Found::Lines(Ok(lines)) => ls.lines = Some(lines),
                Found::Lines(Err(e)) => ls.error = Some(e),
            }
            got = true;
        }
        if got {
            self.invalidate();
        }
    }

    pub fn live_text_motion(&mut self) {
        let mouse = self.mouse;
        let Some(ls) = self.live_shot.as_mut().filter(|l| l.dragging) else { return };
        if let (Some((a, _)), Some(c)) = (ls.sel, ls.caret(mouse)) {
            ls.sel = Some((a, c));
        }
    }

    /// A button over the frozen screen: the left one selects, the right one closes.
    pub fn live_text_button(&mut self, left: bool, pressed: bool) {
        let mouse = self.mouse;
        let Some(ls) = self.live_shot.as_mut() else { return };
        if !left {
            if pressed {
                self.live_shot = None;
            }
            return;
        }
        if pressed {
            // a QR code: its link opens, its text is copied
            let p = ls.to_pic(mouse);
            if let Some(q) = ls.qrs.iter().find(|q| p[0] >= q.bbox[0] && p[0] <= q.bbox[2] && p[1] >= q.bbox[1] && p[1] <= q.bbox[3]) {
                let text = q.text.clone();
                self.live_shot = None;
                if text.starts_with("http://") || text.starts_with("https://") {
                    let mut c = std::process::Command::new("xdg-open");
                    c.arg(&text).env("WAYLAND_DISPLAY", &self.socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                    let _ = c.spawn();
                    self.shot_notice = Some(("Opening the QR code's link".into(), 2.5));
                } else {
                    self.clip_set_text(text);
                    self.shot_notice = Some(("Copied the QR code's text".into(), 2.5));
                }
                self.invalidate();
                return;
            }
            if ls.lines.is_some() && !ls.on_text(mouse) {
                // off the text: the object under the pointer
                ls.sel = None;
                ls.lift(mouse);
                self.invalidate();
                return;
            }
            ls.sel = ls.caret(mouse).map(|c| (c, c));
            ls.dragging = ls.sel.is_some();
        } else {
            ls.dragging = false;
        }
    }

    /// A key while it is open: true when it was used (every key is, while it is open).
    pub fn live_text_key(&mut self, ctrl: bool, sym: smithay::input::keyboard::Keysym) -> bool {
        use smithay::input::keyboard::Keysym as K;
        let Some(ls) = self.live_shot.as_mut() else { return false };
        if sym == K::Escape {
            self.live_shot = None;
        } else if ctrl && (sym == K::a || sym == K::A) {
            if let Some(lines) = &ls.lines {
                if let Some(last) = lines.len().checked_sub(1) {
                    ls.sel = Some(((0, 0), (last, lines[last].spans.len())));
                }
            }
        } else if (ctrl && (sym == K::c || sym == K::C)) || sym == K::Return || sym == K::KP_Enter {
            match ls.text() {
                Some(t) if !t.trim().is_empty() => {
                    let n = t.chars().count();
                    self.clip_set_text(t);
                    self.shot_notice = Some((format!("Copied {n} characters"), 2.5));
                    self.live_shot = None;
                }
                _ => {}
            }
        }
        self.invalidate();
        true
    }

    /// The frozen screen's veil: the lines found lit, the selection in the accent, a hint at the top.
    pub fn live_text_panels(&mut self, shell: &mut Vec<Panel>) {
        let Some(ls) = &self.live_shot else { return };
        let [wx0, wy0, wx1, wy1] = self.world;
        let (sw, sh) = (wx1 - wx0, wy1 - wy0);
        let accent = self.theme.accent;
        if let Some((b, at)) = ls.lifted {
            // the object just copied: everything else darkened, the object ringed
            let r = ls.to_desk(b);
            let k = (at.elapsed().as_secs_f32() / 0.2).min(1.0);
            let dim = |x: f32, y: f32, w: f32, h: f32| Panel::solid(x + w / 2.0, y + h / 2.0, w.max(0.0), h.max(0.0), 0.0, [0.0; 3], [0.0; 3]).flat().alpha(0.32 + 0.33 * k);
            let (x0, y0, x1, y1) = (r[0], r[1], r[0] + r[2], r[1] + r[3]);
            shell.push(dim(wx0, wy0, sw, y0 - wy0));
            shell.push(dim(wx0, y1, sw, wy1 - y1));
            shell.push(dim(wx0, y0, x0 - wx0, y1 - y0));
            shell.push(dim(x1, y0, wx1 - x1, y1 - y0));
            for (x, y, w, h) in [(x0 - 2.0, y0 - 2.0, x1 - x0 + 4.0, 2.0), (x0 - 2.0, y1, x1 - x0 + 4.0, 2.0), (x0 - 2.0, y0, 2.0, y1 - y0), (x1, y0, 2.0, y1 - y0)] {
                shell.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 1.0, accent, accent).flat().alpha(k));
            }
            return;
        }
        shell.push(Panel::solid(wx0 + sw / 2.0, wy0 + sh / 2.0, sw, sh, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(0.32));
        if let Some(lines) = &ls.lines {
            for l in lines {
                let r = ls.to_desk(l.rect);
                let pad = (r[3] * 0.18).max(2.0);
                shell.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] + 2.0 * pad, r[3] + 2.0 * pad, pad + 2.0, [1.0; 3], [1.0; 3]).flat().alpha(0.16));
            }
            for s in ls.selected_rects() {
                let r = ls.to_desk(s);
                shell.push(Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] + 2.0, r[3] + 2.0, 3.0, accent, accent).flat().alpha(0.45));
            }
        }
        // QR codes: ringed in the accent, with what they say beside them
        let mut chips = Vec::new();
        for q in &ls.qrs {
            let r = ls.to_desk([q.bbox[0], q.bbox[1], q.bbox[2] - q.bbox[0], q.bbox[3] - q.bbox[1]]);
            let pad = 6.0;
            let (x0, y0, x1, y1) = (r[0] - pad, r[1] - pad, r[0] + r[2] + pad, r[1] + r[3] + pad);
            for (x, y, w, h) in [(x0, y0, x1 - x0, 3.0), (x0, y1 - 3.0, x1 - x0, 3.0), (x0, y0, 3.0, y1 - y0), (x1 - 3.0, y0, 3.0, y1 - y0)] {
                shell.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 1.5, accent, accent).flat());
            }
            let link = q.text.starts_with("http://") || q.text.starts_with("https://");
            let shown: String = q.text.chars().take(48).collect::<String>() + if q.text.chars().count() > 48 { "…" } else { "" };
            chips.push(((x0 + x1) / 2.0, y1 + 22.0, format!("{shown}   ·   click to {}", if link { "open" } else { "copy" })));
        }
        for (cx, cy, text) in chips {
            let w = self.renderer.text_width(&text, 12.5, true) + 32.0;
            shell.push(Panel::solid(cx, cy, w, 30.0, 15.0, accent, accent).flat().alpha(0.92));
            let labels = vec![Label::new(text, cx, cy + 4.5, 12.5, true, [1.0; 3], 1.0, Align::Center)];
            shell.extend(self.renderer.label_panels(&labels));
        }
        // the hint, on the main monitor
        let text = if ls.lifting {
            "Lifting the object…".to_string()
        } else if let Some(n) = &ls.note {
            n.clone()
        } else {
            match (&ls.lines, &ls.error) {
            (_, Some(e)) => format!("Could not read the text: {e}"),
            (None, _) => {
                let dots = ".".repeat(1 + (ls.started.elapsed().as_millis() / 350 % 3) as usize);
                format!("Reading the text{dots}")
            }
            (Some(l), _) if l.is_empty() => "No text on the screen   ·   Esc closes".to_string(),
            (Some(l), _) => format!("{} lines{}   ·   drag over text to select   ·   click an object to copy it   ·   Ctrl+C copies   ·   Ctrl+A all   ·   Esc closes", l.len(), match ls.qrs.len() {
                0 => String::new(),
                1 => "   ·   a QR code".to_string(),
                n => format!("   ·   {n} QR codes"),
            }),
        }
        };
        let o = self.mon_origin(self.main_mon);
        let [mw, _] = self.size;
        let w = self.renderer.text_width(&text, 13.0, true) + 40.0;
        let (cx, cy) = (o[0] + mw / 2.0, o[1] + 64.0);
        let mut g = self.theme.glass.panel(cx, cy, w, 38.0, 19.0).tinted([0.08, 0.08, 0.10, 0.72]);
        g.extra[0] = 0.3;
        shell.push(g);
        let labels = vec![Label::new(text, cx, cy + 4.8, 13.0, true, [1.0; 3], 1.0, Align::Center)];
        shell.extend(self.renderer.label_panels(&labels));
    }
}
