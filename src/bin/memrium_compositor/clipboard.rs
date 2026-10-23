//! The clipboard history: what was copied (text and pictures), kept so it can be pasted again, with entries that can
//! be pinned to stay. The compositor sees every copy (`new_selection`), reads what was copied through the same
//! protocol a paste would use, and offers a chosen entry as the clipboard again (`send_selection` then serves it).

use crate::state::State;
use serde::{Deserialize, Serialize};
use smithay::{
    backend::input::KeyState,
    input::keyboard::{FilterResult, Keycode},
    utils::SERIAL_COUNTER,
    wayland::selection::data_device::{request_data_device_client_selection, set_data_device_selection},
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc,
};
use std::time::{Duration, Instant};

/// The most that is kept besides the pinned ones, and the biggest single entry.
const KEEP_TEXT: usize = 60;
const KEEP_IMAGES: usize = 10;
const MAX_TEXT: usize = 400_000;
const MAX_BYTES: u64 = 24_000_000;

#[derive(Clone)]
pub enum ClipData {
    Text(String),
    /// PNG bytes and the size of the picture
    Image { png: Arc<Vec<u8>>, w: u32, h: u32 },
}

#[derive(Clone)]
pub struct ClipEntry {
    pub id: u64,
    pub data: ClipData,
    pub at: u64,
    pub pinned: bool,
    hash: u64,
}

impl ClipEntry {
    fn new(id: u64, data: ClipData, at: u64) -> ClipEntry {
        let mut h = DefaultHasher::new();
        match &data {
            ClipData::Text(t) => t.hash(&mut h),
            ClipData::Image { png, .. } => png.hash(&mut h),
        }
        ClipEntry { id, data, at, pinned: false, hash: h.finish() }
    }

    pub fn is_image(&self) -> bool {
        matches!(self.data, ClipData::Image { .. })
    }

    /// The mime types it is offered as.
    fn mimes(&self) -> Vec<String> {
        match self.data {
            ClipData::Text(_) => ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"].iter().map(|s| s.to_string()).collect(),
            ClipData::Image { .. } => vec!["image/png".into()],
        }
    }
}

/// A pinned entry as it is kept in the file; a picture is a file of its own beside it.
#[derive(Serialize, Deserialize, Default)]
struct Saved {
    text: String,
    picture: String,
    at: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct SavedFile {
    pinned: Vec<Saved>,
}

fn dir() -> PathBuf {
    memrium_de::theme::config_dir().join("clipboard")
}

fn file() -> PathBuf {
    memrium_de::theme::config_dir().join("clipboard.toml")
}

struct Fetched {
    mime: String,
    bytes: Vec<u8>,
}

pub struct Clipboard {
    /// pinned first, then newest first
    pub items: Vec<ClipEntry>,
    next_id: u64,
    /// the clipboard changed: these are the types it is offered in (read on the next turn)
    want: Option<Vec<String>>,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
    /// the entry the clipboard is now (we are its owner)
    serving: Option<u64>,
    /// the clipboard is being set by us: the announcement that follows is not a copy
    own_expected: bool,
    dirty: Option<Instant>,
    /// paste into the program in front once this time has come
    paste_at: Option<Instant>,
}

impl Clipboard {
    pub fn new() -> Clipboard {
        let (tx, rx) = channel();
        let mut c = Clipboard::empty(tx, rx);
        c.load();
        c
    }

    fn empty(tx: Sender<Fetched>, rx: Receiver<Fetched>) -> Clipboard {
        Clipboard { items: Vec::new(), next_id: 1, want: None, tx, rx, serving: None, own_expected: false, dirty: None, paste_at: None }
    }

    fn load(&mut self) {
        let Some(saved) = std::fs::read_to_string(file()).ok().and_then(|s| toml::from_str::<SavedFile>(&s).ok()) else { return };
        for s in saved.pinned {
            let data = if !s.picture.is_empty() {
                let Ok(png) = std::fs::read(dir().join(&s.picture)) else { continue };
                let Ok((w, h)) = image::load_from_memory(&png).map(|i| (i.width(), i.height())) else { continue };
                ClipData::Image { png: Arc::new(png), w, h }
            } else if !s.text.is_empty() {
                ClipData::Text(s.text)
            } else {
                continue;
            };
            let mut e = ClipEntry::new(self.next_id, data, s.at);
            e.pinned = true;
            self.next_id += 1;
            self.items.push(e);
        }
    }

    fn save(&mut self) {
        self.dirty = None;
        let _ = std::fs::create_dir_all(dir());
        let mut keep: Vec<String> = Vec::new();
        let pinned: Vec<Saved> = self
            .items
            .iter()
            .filter(|e| e.pinned)
            .map(|e| match &e.data {
                ClipData::Text(t) => Saved { text: t.clone(), picture: String::new(), at: e.at },
                ClipData::Image { png, .. } => {
                    let name = format!("{:016x}.png", e.hash);
                    let path = dir().join(&name);
                    if !path.exists() {
                        let _ = std::fs::write(&path, png.as_slice());
                    }
                    keep.push(name.clone());
                    Saved { text: String::new(), picture: name, at: e.at }
                }
            })
            .collect();
        // pictures that are not pinned any more go
        if let Ok(rd) = std::fs::read_dir(dir()) {
            for f in rd.flatten() {
                if !keep.contains(&f.file_name().to_string_lossy().to_string()) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        if let Ok(text) = toml::to_string(&SavedFile { pinned }) {
            let _ = std::fs::write(file(), text);
        }
    }

    fn touch(&mut self) {
        self.dirty = Some(Instant::now());
    }

    /// Put an entry in (a copy of something already there moves it to the top); the pinned ones stay above.
    fn add(&mut self, data: ClipData) {
        let fresh = ClipEntry::new(self.next_id, data, crate::notify::unix_now());
        if let Some(i) = self.items.iter().position(|e| e.hash == fresh.hash) {
            let mut e = self.items.remove(i);
            e.at = fresh.at;
            self.place(e);
        } else {
            self.next_id += 1;
            self.place(fresh);
        }
        // the oldest of each kind that is not pinned go
        for (image, most) in [(false, KEEP_TEXT), (true, KEEP_IMAGES)] {
            let mut seen = 0;
            self.items.retain(|e| {
                if e.pinned || e.is_image() != image {
                    return true;
                }
                seen += 1;
                seen <= most
            });
        }
        self.touch();
    }

    /// After the pinned ones, before the others.
    fn place(&mut self, e: ClipEntry) {
        let at = if e.pinned { 0 } else { self.items.iter().take_while(|x| x.pinned).count() };
        self.items.insert(at, e);
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(i) = self.items.iter().position(|e| e.id == id) {
            let mut e = self.items.remove(i);
            e.pinned = !e.pinned;
            // a pinned one goes to the end of the pinned ones, an unpinned one to the top of the others
            let at = self.items.iter().take_while(|x| x.pinned).count();
            self.items.insert(at, e);
            self.touch();
        }
    }

    pub fn delete(&mut self, id: u64) {
        self.items.retain(|e| e.id != id);
        self.touch();
    }

    /// Everything that is not pinned.
    pub fn clear(&mut self) {
        self.items.retain(|e| e.pinned);
        self.touch();
    }
}

/// The mime type to read a copy as: text unless the copy is only a picture.
fn pick_mime(mimes: &[String]) -> Option<String> {
    let has = |m: &str| mimes.iter().any(|x| x == m);
    for m in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"] {
        if has(m) {
            return Some(m.into());
        }
    }
    for m in ["image/png", "image/jpeg", "image/bmp", "image/webp"] {
        if has(m) {
            return Some(m.into());
        }
    }
    None
}

impl State {
    /// A program (or an X11 one) copied. Called from the selection handler.
    pub fn clip_copied(&mut self, mimes: Vec<String>) {
        if self.clipboard.own_expected {
            self.clipboard.own_expected = false;
            return;
        }
        // we are not the owner any more
        self.clipboard.serving = None;
        // a password manager marks what it copies as secret
        if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
            return;
        }
        self.clipboard.want = Some(mimes);
    }

    /// Somebody pastes what we offered: write the entry to them.
    pub fn clip_serve(&mut self, mime: &str, fd: OwnedFd) -> Result<(), OwnedFd> {
        let Some(id) = self.clipboard.serving else { return Err(fd) };
        let Some(e) = self.clipboard.items.iter().find(|e| e.id == id) else { return Ok(()) };
        let bytes: Vec<u8> = match (&e.data, mime.starts_with("image/")) {
            (ClipData::Text(t), false) => t.clone().into_bytes(),
            (ClipData::Image { png, .. }, true) => png.as_ref().clone(),
            _ => return Ok(()),
        };
        std::thread::spawn(move || {
            let mut f = std::fs::File::from(fd);
            let _ = f.write_all(&bytes);
        });
        Ok(())
    }

    /// Every turn: read a copy that just happened, take in what was read, write the history, paste.
    pub fn clipboard_tick(&mut self) {
        if let Some(mimes) = self.clipboard.want.take() {
            if let Some(mime) = pick_mime(&mimes) {
                if let Ok((mine, theirs)) = UnixStream::pair() {
                    let _ = mine.set_read_timeout(Some(Duration::from_secs(3)));
                    if request_data_device_client_selection(&self.seat, mime.clone(), OwnedFd::from(theirs)).is_ok() {
                        let tx = self.clipboard.tx.clone();
                        std::thread::spawn(move || {
                            let mut bytes = Vec::new();
                            let _ = (&mine).take(MAX_BYTES).read_to_end(&mut bytes);
                            let _ = tx.send(Fetched { mime, bytes });
                        });
                    }
                }
            }
        }
        while let Ok(f) = self.clipboard.rx.try_recv() {
            if f.mime.starts_with("image/") {
                if f.bytes.is_empty() {
                    continue;
                }
                let png = if f.mime == "image/png" {
                    Some(f.bytes)
                } else {
                    image::load_from_memory(&f.bytes).ok().and_then(|i| {
                        let mut out = Vec::new();
                        i.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok().map(|_| out)
                    })
                };
                if let Some(png) = png {
                    if let Ok((w, h)) = image::load_from_memory(&png).map(|i| (i.width(), i.height())) {
                        self.clipboard.add(ClipData::Image { png: Arc::new(png), w, h });
                    }
                }
            } else {
                let text = String::from_utf8_lossy(&f.bytes).to_string();
                if !text.trim().is_empty() && text.len() <= MAX_TEXT {
                    self.clipboard.add(ClipData::Text(text));
                }
            }
        }
        if self.clipboard.dirty.is_some_and(|d| d.elapsed().as_millis() > 1500) {
            self.clipboard.save();
        }
        if self.clipboard.paste_at.is_some_and(|t| Instant::now() >= t) {
            self.clipboard.paste_at = None;
            self.inject_paste();
        }
    }

    /// Make an entry the clipboard, and paste it into the program in front.
    pub fn clip_paste(&mut self, id: u64) {
        let Some(i) = self.clipboard.items.iter().position(|e| e.id == id) else { return };
        let mimes = self.clipboard.items[i].mimes();
        // the one used last is the first of its kind
        if !self.clipboard.items[i].pinned {
            let e = self.clipboard.items.remove(i);
            self.clipboard.place(e);
        }
        self.clipboard.serving = Some(id);
        self.clipboard.own_expected = true;
        set_data_device_selection(&self.dh, &self.seat, mimes, ());
        self.clipboard.own_expected = false;
        self.clipboard.paste_at = Some(Instant::now() + Duration::from_millis(140));
    }

    /// Ctrl+V (Ctrl+Shift+V in a terminal) to the program that has the keyboard.
    fn inject_paste(&mut self) {
        let Some(top) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) else { return };
        if self.focused_shell().is_some() {
            return;
        }
        let app = self.wins[top].app_id.to_lowercase();
        let terminal = ["kitty", "alacritty", "foot", "konsole", "terminal", "wezterm", "xterm", "tilix", "ghostty", "st-256color"].iter().any(|t| app.contains(t));
        let mut keys: Vec<(u32, bool)> = vec![(29, true)];
        if terminal {
            keys.push((42, true));
        }
        keys.extend([(47, true), (47, false)]);
        if terminal {
            keys.push((42, false));
        }
        keys.push((29, false));
        let kb = self.keyboard.clone();
        for (evdev, down) in keys {
            let time = self.time_ms();
            kb.input::<(), _>(self, Keycode::new(evdev + 8), if down { KeyState::Pressed } else { KeyState::Released }, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Clipboard {
        let (tx, rx) = channel();
        Clipboard::empty(tx, rx)
    }

    fn text(c: &Clipboard) -> Vec<String> {
        c.items.iter().map(|e| if let ClipData::Text(t) = &e.data { t.clone() } else { "<picture>".into() }).collect()
    }

    #[test]
    fn copies_are_kept_newest_first_and_a_repeat_moves_up() {
        let mut c = clip();
        for t in ["one", "two", "three", "one"] {
            c.add(ClipData::Text(t.into()));
        }
        assert_eq!(text(&c), ["one", "three", "two"]);
    }

    #[test]
    fn pinned_entries_stay_on_top_and_survive_the_limit() {
        let mut c = clip();
        c.add(ClipData::Text("keep me".into()));
        let id = c.items[0].id;
        c.toggle_pin(id);
        for i in 0..KEEP_TEXT + 20 {
            c.add(ClipData::Text(format!("n{i}")));
        }
        assert_eq!(text(&c)[0], "keep me");
        assert_eq!(c.items.len(), KEEP_TEXT + 1);
        // a copy of the pinned one does not unpin it or move it below the others
        c.add(ClipData::Text("keep me".into()));
        assert!(c.items[0].pinned);
        assert_eq!(c.items.iter().filter(|e| e.pinned).count(), 1);
        c.clear();
        assert_eq!(text(&c), ["keep me"]);
        c.toggle_pin(id);
        assert!(!c.items[0].pinned);
        c.delete(id);
        assert!(c.items.is_empty());
    }

    #[test]
    fn pictures_have_a_limit_of_their_own() {
        let mut c = clip();
        for i in 0..KEEP_IMAGES + 5 {
            c.add(ClipData::Image { png: Arc::new(vec![i as u8, 1, 2]), w: 1, h: 1 });
        }
        c.add(ClipData::Text("t".into()));
        assert_eq!(c.items.iter().filter(|e| e.is_image()).count(), KEEP_IMAGES);
        assert_eq!(c.items.len(), KEEP_IMAGES + 1);
    }

    #[test]
    fn text_is_read_before_a_picture_and_secrets_are_not_offered() {
        let m = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_mime(&m(&["image/png", "text/plain"])).as_deref(), Some("text/plain"));
        assert_eq!(pick_mime(&m(&["image/png", "text/html"])).as_deref(), Some("image/png"));
        assert_eq!(pick_mime(&m(&["application/x-thing"])), None);
    }
}
