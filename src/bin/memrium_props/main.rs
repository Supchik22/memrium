//! Memrium Properties: what the file manager's "Properties" shows for a file or folder: the basics, who may do what,
//! and for pictures the size in pixels and what the camera recorded.
//!
//!   memrium_props <path>

mod exif;

use chebrium::app::{self, App, Config, Host};
use chebrium::draw::Align;
use chebrium::ui::Ui;
use memrium_de::fsops::{self, human_size, Kind};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PAD: f32 = 24.0;
const HEAD_H: f32 = 84.0;
const ROW_H: f32 = 28.0;
const LABEL_W: f32 = 140.0;

type Section = (&'static str, Vec<(String, String)>);

/// Folder contents, counted on a thread of its own: (files, folders, bytes, finished).
type Tally = Arc<Mutex<(u64, u64, u64, bool)>>;

struct PropsApp {
    path: PathBuf,
    is_dir: bool,
    sections: Vec<Section>,
    tally: Option<Tally>,
    scroll: f32,
    waker: Option<app::Waker>,
}

fn user_name(uid: u32) -> String {
    let p = unsafe { libc::getpwuid(uid) };
    if p.is_null() {
        return uid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr((*p).pw_name) }.to_string_lossy().to_string()
}

fn group_name(gid: u32) -> String {
    let g = unsafe { libc::getgrgid(gid) };
    if g.is_null() {
        return gid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr((*g).gr_name) }.to_string_lossy().to_string()
}

fn permissions(mode: u32) -> String {
    let mut s = String::new();
    for shift in [6, 3, 0] {
        let b = mode >> shift;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    format!("{s}  ({:03o})", mode & 0o777)
}

fn count(dir: &Path, tally: &Tally, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        {
            let mut t = tally.lock().unwrap();
            if is_dir {
                t.1 += 1;
            } else {
                t.0 += 1;
                t.2 += m.len();
            }
        }
        if is_dir && depth < 64 {
            count(&e.path(), tally, depth + 1);
        }
    }
}

fn mime_of(path: &Path) -> String {
    std::process::Command::new("file").arg("-b").arg("--mime-type").arg(path).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

impl PropsApp {
    fn new(path: PathBuf) -> PropsApp {
        let meta = std::fs::symlink_metadata(&path).ok();
        let is_dir = path.is_dir();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
        let kind = fsops::kind_of(&name, is_dir);
        let mut general: Vec<(String, String)> = Vec::new();
        general.push(("Name".into(), name));
        general.push(("Kind".into(), kind.label().to_string()));
        if !is_dir {
            let mime = mime_of(&path);
            if !mime.is_empty() {
                general.push(("Type".into(), mime));
            }
        }
        general.push(("Location".into(), path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
        if let Some(m) = &meta {
            if m.file_type().is_symlink() {
                general.push(("Links to".into(), std::fs::read_link(&path).map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
            }
            if !is_dir {
                general.push(("Size".into(), format!("{} ({} bytes)", human_size(m.len()), m.len())));
            }
            general.push(("Modified".into(), fsops::format_time(m.mtime().max(0) as u64)));
            general.push(("Accessed".into(), fsops::format_time(m.atime().max(0) as u64)));
            general.push(("Changed".into(), fsops::format_time(m.ctime().max(0) as u64)));
        }
        let mut sections: Vec<Section> = vec![("General", general)];
        if let Some(m) = &meta {
            sections.push(("Permissions", vec![("Owner".into(), format!("{}:{}", user_name(m.uid()), group_name(m.gid()))), ("Access".into(), permissions(m.permissions().mode()))]));
        }
        if kind == Kind::Image && !is_dir {
            let mut pic: Vec<(String, String)> = Vec::new();
            if let Ok((w, h)) = image::image_dimensions(&path) {
                pic.push(("Dimensions".into(), format!("{w} × {h} px")));
                pic.push(("Megapixels".into(), format!("{:.1} MP", w as f64 * h as f64 / 1e6)));
            }
            if let Ok(data) = std::fs::read(&path) {
                pic.extend(exif::read(&data));
            }
            if !pic.is_empty() {
                sections.push(("Picture", pic));
            }
        }
        let tally = if is_dir {
            let t: Tally = Arc::new(Mutex::new((0, 0, 0, false)));
            let (t2, p2) = (t.clone(), path.clone());
            std::thread::spawn(move || {
                count(&p2, &t2, 0);
                t2.lock().unwrap().3 = true;
            });
            Some(t)
        } else {
            None
        };
        PropsApp { path, is_dir, sections, tally, scroll: 0.0, waker: None }
    }
}

fn fit(ui: &mut Ui<'_>, text: &str, size: f32, max: f32) -> String {
    if ui.measure_raw(text, size, false) <= max {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() && ui.measure_raw(&format!("{s}…"), size, false) > max {
        s.pop();
    }
    format!("{s}…")
}

impl App for PropsApp {
    fn start(&mut self, waker: app::Waker) {
        self.waker = Some(waker);
    }

    fn background(&self, look: &chebrium::theme::Appearance) -> ([f32; 3], f32) {
        (if look.dark { [0.115, 0.115, 0.115] } else { [0.955, 0.960, 0.972] }, 1.0)
    }

    fn tick(&mut self, host: &mut Host) {
        // a folder is still being counted: keep the numbers moving
        if self.tally.as_ref().is_some_and(|t| !t.lock().unwrap().3) {
            host.redraw = true;
        }
    }

    fn key(&mut self, key: chebrium::input::Key, _mods: chebrium::input::KeyMods, host: &mut Host) {
        host.redraw = true;
        if key == chebrium::input::Key::Esc {
            host.quit = true;
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) {
        let [w, h] = size;
        let (ink, accent) = (ui.ink, ui.accent);
        let name = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
        host.title = Some(format!("{name} — Properties"));
        let kind = fsops::kind_of(&name, self.is_dir);
        ui.fill([PAD, 18.0, 48.0, 48.0], 14.0, accent, 0.18);
        ui.glyph(PAD + 24.0, 42.0, 24.0, if self.is_dir { "folder" } else { kind.icon() }, accent, 1.0);
        let title = fit(ui, &name, 18.0, w - PAD * 2.0 - 62.0);
        ui.text(PAD + 62.0, 36.0, &title, 18.0, true, ink, 0.96, Align::Left);
        let sub = match &self.tally {
            Some(t) => {
                let t = t.lock().unwrap();
                format!("{} · {} file{}, {} folder{}{}", human_size(t.2), t.0, if t.0 == 1 { "" } else { "s" }, t.1, if t.1 == 1 { "" } else { "s" }, if t.3 { "" } else { " …" })
            }
            None => std::fs::metadata(&self.path).map(|m| format!("{} · {}", kind.label(), human_size(m.len()))).unwrap_or_default(),
        };
        ui.text(PAD + 62.0, 56.0, &sub, 12.5, false, ink, 0.6, Align::Left);
        ui.fill([0.0, HEAD_H - 1.0, w, 1.0], 0.0, ink, 0.09);

        let view = [0.0, HEAD_H, w, h - HEAD_H];
        let rows: usize = self.sections.iter().map(|(_, r)| r.len()).sum();
        let content = self.sections.len() as f32 * 44.0 + rows as f32 * ROW_H + 24.0;
        let max = (content - view[3]).max(0.0);
        if ui.scroll != 0.0 {
            self.scroll += ui.scroll * 2.5;
        }
        self.scroll = self.scroll.clamp(0.0, max);
        ui.push_clip(view);
        let mut y = HEAD_H + 22.0 - self.scroll;
        for (title, rows) in &self.sections {
            ui.text(PAD, y, &title.to_uppercase(), 11.5, true, ink, 0.50, Align::Left);
            y += 12.0;
            let card = [PAD, y, w - 2.0 * PAD, rows.len() as f32 * ROW_H + 8.0];
            ui.fill(card, 12.0, ink, 0.09);
            let card_c = if ui.dark { [0.165, 0.165, 0.165] } else { [1.0, 1.0, 1.0] };
            ui.fill([card[0] + 1.0, card[1] + 1.0, card[2] - 2.0, card[3] - 2.0], 11.0, card_c, 1.0);
            let mut ry = y + 4.0;
            for (label, value) in rows {
                ui.text(PAD + 16.0, ry + 19.0, label, 13.0, false, ink, 0.55, Align::Left);
                let v = fit(ui, value, 13.0, w - 2.0 * PAD - LABEL_W - 32.0);
                ui.text(PAD + 16.0 + LABEL_W, ry + 19.0, &v, 13.0, false, ink, 0.92, Align::Left);
                ry += ROW_H;
            }
            y = card[1] + card[3] + 32.0;
        }
        ui.pop_clip();
    }
}

fn main() {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(fsops::home);
    app::run(Config::new("Memrium Properties", "memrium-props").size(520, 640).min_size(420, 360), PropsApp::new(path));
}
