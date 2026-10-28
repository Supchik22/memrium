//! Icons for files and folders: theme icons (folder, file types) and image thumbnails, loaded lazily
//! into the app-icon atlas (thumbnails are decoded on a worker thread).

use memrium_de::apps::{load_icon_file, IconIndex, IconSlot};
use memrium_de::files::IconState;
use memrium_de::fsops::{self, Entry, Kind};
use memrium_de::renderer::Renderer;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};

/// What the thumbnail worker is asked for: only what is on screen now (rebuilt every frame, so a fast
/// scroll does not leave it decoding pictures that have long gone), and what it has taken but not delivered.
#[derive(Default)]
struct Jobs {
    queue: Vec<(String, PathBuf, Option<image::RgbaImage>)>,
    taken: HashSet<String>,
}

type Shared = Arc<(Mutex<Jobs>, Condvar)>;

/// at most this many thumbnails live in the atlas (it also holds app icons)
const MAX_THUMBS: usize = 140;

pub struct IconCache {
    index: Arc<IconSlot>,
    /// application icons, for the launchers
    app_index: Arc<IconSlot>,
    failed: HashSet<String>,
    thumb_failed: HashSet<String>,
    jobs: Shared,
    rx: Receiver<(String, Option<image::RgbaImage>)>,
    thumbs: usize,
    /// keys of the thumbnails in the atlas, oldest first: the ones off screen go back when it is full
    loaded: std::collections::VecDeque<String>,
}

fn thumb_key(e: &Entry) -> String {
    format!("th:{}:{}", e.path.display(), e.modified)
}

fn folder_key(e: &Entry) -> String {
    format!("fp:{}:{}", e.path.display(), e.modified)
}

fn mime_key(e: &Entry) -> String {
    match &e.app {
        Some((_, icon)) if !icon.is_empty() => format!("fi:app:{icon}"),
        _ => format!("fi:{}", fsops::icon_names(e)[0]),
    }
}

fn is_picture(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "gif", "bmp"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Up to `n` pictures of a folder: its own first, else those of its subfolders (one level down).
fn pictures_in(dir: &std::path::Path, n: usize) -> Vec<PathBuf> {
    let list = |d: &std::path::Path| -> (Vec<PathBuf>, Vec<PathBuf>) {
        let (mut files, mut dirs) = (Vec::new(), Vec::new());
        // a huge folder is not worth walking to the end
        for e in std::fs::read_dir(d).into_iter().flatten().flatten().take(600) {
            let p = e.path();
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if p.is_dir() {
                dirs.push(p);
            } else if is_picture(&p) && e.metadata().is_ok_and(|m| m.len() < 40_000_000) {
                files.push(p);
            }
        }
        files.sort();
        dirs.sort();
        (files, dirs)
    };
    let (mut files, dirs) = list(dir);
    for d in dirs {
        if files.len() >= n {
            break;
        }
        files.extend(list(&d).0);
    }
    files.truncate(n);
    files
}

/// The folder icon with a few of its pictures fanned out over the body.
fn folder_preview(base: &image::RgbaImage, dir: &std::path::Path) -> Option<image::RgbaImage> {
    let pics: Vec<image::RgbaImage> = pictures_in(dir, 3).iter().filter_map(|p| image::open(p).ok()).map(|i| i.thumbnail(96, 96).to_rgba8()).collect();
    if pics.is_empty() {
        return None;
    }
    let mut out = base.clone();
    let slots: &[(i64, i64)] = match pics.len() {
        1 => &[(38, 50)],
        2 => &[(20, 50), (58, 46)],
        _ => &[(10, 52), (38, 44), (66, 52)],
    };
    let side = if pics.len() == 1 { 52.0 } else { 46.0 };
    for (img, &(x, y)) in pics.iter().zip(slots) {
        let (w, h) = img.dimensions();
        let s = side / w.max(h) as f32;
        let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
        let small = image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle);
        let (px, py) = (x + ((side as u32 - nw) / 2) as i64, y + ((side as u32 - nh) / 2) as i64);
        // a thin white mat and a soft shadow so the photos read as prints
        let mut mat = image::RgbaImage::from_pixel(nw + 4, nh + 4, image::Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut mat, &small, 2, 2);
        let shadow = image::RgbaImage::from_pixel(nw + 4, nh + 4, image::Rgba([0, 0, 0, 70]));
        image::imageops::overlay(&mut out, &shadow, px - 1, py + 1);
        image::imageops::overlay(&mut out, &mat, px - 2, py - 2);
    }
    Some(out)
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
        let jobs: Shared = Default::default();
        let (tx, rx) = channel::<(String, Option<image::RgbaImage>)>();
        // several decoders: a folder of photos is not shown one picture after another
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        for _ in 0..workers {
            let worker_jobs = jobs.clone();
            let worker_tx = tx.clone();
            std::thread::spawn(move || {
            loop {
                let (key, path, base) = {
                    let (lock, cv) = &*worker_jobs;
                    let mut j = lock.lock().unwrap();
                    while j.queue.is_empty() {
                        j = cv.wait(j).unwrap();
                    }
                    let job = j.queue.remove(0);
                    j.taken.insert(job.0.clone());
                    job
                };
                if let Some(base) = base {
                    if worker_tx.send((key, folder_preview(&base, &path))).is_err() {
                        break;
                    }
                    continue;
                }
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
        }
        IconCache { index, app_index, failed: HashSet::new(), thumb_failed: HashSet::new(), jobs, rx, thumbs: 0, loaded: Default::default() }
    }

    /// The slots of the file-type and application indexes, for a rebuild after the icon theme changed.
    pub fn slots(&self) -> (Arc<IconSlot>, Arc<IconSlot>) {
        (self.index.clone(), self.app_index.clone())
    }

    /// Forget what was loaded and what failed: the icons are looked up again in the new indexes.
    pub fn reset(&mut self) {
        self.failed.clear();
        self.thumb_failed.clear();
        self.jobs.0.lock().unwrap().queue.clear();
        self.thumbs = 0;
        self.loaded.clear();
    }

    /// What to draw for `e` right now.
    pub fn state(&self, r: &Renderer, e: &Entry, want_thumb: bool) -> IconState {
        if want_thumb && (e.kind == Kind::Image || e.is_dir) {
            let key = if e.is_dir { folder_key(e) } else { thumb_key(e) };
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
        // the atlas holds so many thumbnails: when it is full, those that are off screen make room for the ones that are on
        if self.thumbs + 8 >= MAX_THUMBS {
            let visible: HashSet<String> = wanted.iter().map(|e| if e.is_dir { folder_key(e) } else { thumb_key(e) }).collect();
            let mut keep = std::collections::VecDeque::new();
            while let Some(k) = self.loaded.pop_front() {
                if self.thumbs + 24 > MAX_THUMBS && !visible.contains(&k) {
                    if r.app_icon_free(&k) {
                        self.thumbs -= 1;
                    }
                } else {
                    keep.push_back(k);
                }
            }
            self.loaded = keep;
        }
        {
            let (lock, cv) = &*self.jobs;
            let mut j = lock.lock().unwrap();
            j.queue.clear();
            for e in wanted {
                if e.is_dir && self.thumbs + j.taken.len() + j.queue.len() < MAX_THUMBS + 8 {
                    let key = folder_key(e);
                    if r.app_icon_get(&key).is_none() && !self.thumb_failed.contains(&key) && !j.taken.contains(&key) && !j.queue.iter().any(|(k, ..)| *k == key) {
                        // the folder icon the photos go onto; without it yet, try again next frame
                        let base = self.index.get().and_then(|ix| fsops::icon_names(e).into_iter().find_map(|n| ix.load(n, 128)));
                        if let Some(base) = base {
                            j.queue.push((key, e.path.clone(), Some(base)));
                        }
                    }
                    continue;
                }
                if e.kind == Kind::Image && !e.is_dir && self.thumbs + j.taken.len() + j.queue.len() < MAX_THUMBS + 8 {
                    let key = thumb_key(e);
                    if r.app_icon_get(&key).is_none() && !self.thumb_failed.contains(&key) && !j.taken.contains(&key) && !j.queue.iter().any(|(k, ..)| *k == key) {
                        j.queue.push((key, e.path.clone(), None));
                    }
                }
            }
            if !j.queue.is_empty() {
                cv.notify_one();
            }
        }
        for e in wanted {
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
            self.jobs.0.lock().unwrap().taken.remove(&key);
            match img {
                Some(img) => {
                    if r.image_cell(&key, &img).is_some() {
                        self.thumbs += 1;
                        self.loaded.push_back(key);
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
