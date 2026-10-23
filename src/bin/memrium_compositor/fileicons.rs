//! Icons for files and folders: theme icons (folder, file types) and image thumbnails, loaded lazily
//! into the app-icon atlas (thumbnails are decoded on a worker thread).

use memrium_de::apps::{load_icon_file, IconIndex, IconSlot};
use memrium_de::files::IconState;
use memrium_de::fsops::{self, Entry, Kind};
use memrium_de::renderer::Renderer;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// at most this many thumbnails live in the atlas (it also holds app icons)
const MAX_THUMBS: usize = 140;

pub struct IconCache {
    index: Arc<IconSlot>,
    /// application icons, for the launchers
    app_index: Arc<IconSlot>,
    failed: HashSet<String>,
    thumb_failed: HashSet<String>,
    thumb_req: HashSet<String>,
    tx: Sender<(String, PathBuf)>,
    rx: Receiver<(String, Option<image::RgbaImage>)>,
    thumbs: usize,
}

fn thumb_key(e: &Entry) -> String {
    format!("th:{}:{}", e.path.display(), e.modified)
}

fn mime_key(e: &Entry) -> String {
    match &e.app {
        Some((_, icon)) if !icon.is_empty() => format!("fi:app:{icon}"),
        _ => format!("fi:{}", fsops::icon_names(e)[0]),
    }
}

/// Fit an image into a 128 px square (centred, transparent padding) so the atlas does not stretch it.
fn square(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img;
    }
    let s = 128.0 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
    let small = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    let mut out = image::RgbaImage::new(128, 128);
    image::imageops::overlay(&mut out, &small, ((128 - nw) / 2) as i64, ((128 - nh) / 2) as i64);
    out
}

impl IconCache {
    pub fn new() -> IconCache {
        let index: Arc<IconSlot> = Default::default();
        {
            let index = index.clone();
            std::thread::spawn(move || {
                index.set(IconIndex::build_mime());
            });
        }
        let app_index: Arc<IconSlot> = Default::default();
        {
            let app_index = app_index.clone();
            std::thread::spawn(move || {
                app_index.set(IconIndex::build());
            });
        }
        let (tx, worker_rx) = channel::<(String, PathBuf)>();
        let (worker_tx, rx) = channel::<(String, Option<image::RgbaImage>)>();
        std::thread::spawn(move || {
            while let Ok((key, path)) = worker_rx.recv() {
                let big = std::fs::metadata(&path).map(|m| m.len() > 40_000_000).unwrap_or(true);
                let img = if big {
                    None
                } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
                    load_icon_file(&path, 128)
                } else {
                    image::open(&path).ok().map(|i| square(i.thumbnail(256, 256).to_rgba8()))
                };
                if worker_tx.send((key, img)).is_err() {
                    break;
                }
            }
        });
        IconCache { index, app_index, failed: HashSet::new(), thumb_failed: HashSet::new(), thumb_req: HashSet::new(), tx, rx, thumbs: 0 }
    }

    /// The slots of the file-type and application indexes, for a rebuild after the icon theme changed.
    pub fn slots(&self) -> (Arc<IconSlot>, Arc<IconSlot>) {
        (self.index.clone(), self.app_index.clone())
    }

    /// Forget what was loaded and what failed: the icons are looked up again in the new indexes.
    pub fn reset(&mut self) {
        self.failed.clear();
        self.thumb_failed.clear();
        self.thumb_req.clear();
        self.thumbs = 0;
    }

    /// What to draw for `e` right now.
    pub fn state(&self, r: &Renderer, e: &Entry, want_thumb: bool) -> IconState {
        if want_thumb && e.kind == Kind::Image && !e.is_dir {
            let key = thumb_key(e);
            if let Some(c) = r.app_icon_get(&key) {
                return IconState::Cell(c);
            }
        }
        let key = mime_key(e);
        if let Some(c) = r.app_icon_get(&key) {
            return IconState::Cell(c);
        }
        if self.failed.contains(&key) {
            IconState::None
        } else {
            IconState::Pending
        }
    }

    /// Load what the views asked for (a few per frame), and collect finished thumbnails.
    pub fn request(&mut self, r: &mut Renderer, wanted: &[Entry]) {
        let mut budget = 4;
        for e in wanted {
            if e.kind == Kind::Image && !e.is_dir && self.thumbs + self.thumb_req.len() < MAX_THUMBS + 8 {
                let key = thumb_key(e);
                if r.app_icon_get(&key).is_none() && !self.thumb_failed.contains(&key) && self.thumb_req.insert(key.clone()) {
                    let _ = self.tx.send((key, e.path.clone()));
                }
            }
            let key = mime_key(e);
            if budget == 0 || r.app_icon_get(&key).is_some() || self.failed.contains(&key) {
                continue;
            }
            let Some(index) = self.index.get() else { continue };
            budget -= 1;
            let mut img = None;
            if let Some((_, icon)) = e.app.as_ref().filter(|(_, i)| !i.is_empty()) {
                // wait for the application icons rather than settle for the generic file
                let Some(apps) = self.app_index.get() else { continue };
                img = apps.load(icon, 128);
            }
            let img = img.or_else(|| fsops::icon_names(e).into_iter().find_map(|n| index.load(n, 128)));
            match img {
                Some(img) => {
                    r.image_cell(&key, &img);
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
        while let Ok((key, img)) = self.rx.try_recv() {
            match img {
                Some(img) => {
                    if r.image_cell(&key, &img).is_some() {
                        self.thumbs += 1;
                    } else {
                        self.thumb_failed.insert(key);
                    }
                }
                None => {
                    self.thumb_failed.insert(key);
                }
            }
        }
    }
}
