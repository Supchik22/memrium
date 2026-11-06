//! Pictures of files: thumbnails of images and folder icons with a few of their photos fanned out over them. Shared by
//! the desktop's icons (drawn by the shell) and the file manager (a program of its own).

use std::path::{Path, PathBuf};

pub fn is_picture(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "gif", "bmp"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Up to `n` pictures of a folder: its own first, else those of its subfolders (one level down).
pub fn pictures_in(dir: &Path, n: usize) -> Vec<PathBuf> {
    let list = |d: &Path| -> (Vec<PathBuf>, Vec<PathBuf>) {
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
pub fn folder_preview(base: &image::RgbaImage, dir: &Path) -> Option<image::RgbaImage> {
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
pub fn square(img: image::RgbaImage) -> image::RgbaImage {
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


/// A thumbnail of the picture at `path`, fitted into a 128 px square (None for a file too big or one that cannot be
/// read). SVG drawings are drawn at that size.
pub fn thumbnail(path: &Path) -> Option<image::RgbaImage> {
    let big = std::fs::metadata(path).map(|m| m.len() > 40_000_000).unwrap_or(true);
    if big {
        None
    } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        crate::apps::load_icon_file(path, 128)
    } else {
        image::open(path).ok().map(|i| square(i.thumbnail(256, 256).to_rgba8()))
    }
}
