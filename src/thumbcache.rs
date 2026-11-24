//! The thumbnails kept on disk, as the freedesktop Thumbnail Managing Standard has them: `~/.cache/thumbnails/large/` holds
//! a PNG of at most 256 px per file, named by the MD5 of the file's URI, with the file's URI and modification time written
//! into it (`Thumb::URI`, `Thumb::MTime`). A thumbnail whose time differs from the file's is stale and made again. A file
//! that could not be drawn gets an empty mark in `fail/memrium/`, so it is not tried on every look. Dolphin, Nautilus and
//! the others keep theirs there too: what one of them made is taken as it is.
//!
//! Our own pictures that are not thumbnails of the file as the standard means them (the waveform of a song) are kept
//! the same way under `~/.cache/memrium/thumbnails/<kind>/`.

use md5::{Digest, Md5};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The side of a "large" thumbnail.
pub const LARGE: u32 = 256;

fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| crate::fsops::home().join(".cache"))
}

/// The folder of the shared thumbnails.
pub fn shared_root() -> PathBuf {
    cache_home().join("thumbnails")
}

/// The folder of our own pictures of kind `kind`.
pub fn own_root(kind: &str) -> PathBuf {
    cache_home().join("memrium/thumbnails").join(kind)
}

/// `file:///home/me/My%20File.png`: the absolute path with every byte that is not unreserved written as `%XX`.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let mut s = String::from("file://");
    for &b in abs.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// The picture of a cached PNG and its text fields.
fn read_png(file: &Path) -> Option<(image::RgbaImage, HashMap<String, String>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).ok()?));
    let reader = decoder.read_info().ok()?;
    let mut fields = HashMap::new();
    for t in &reader.info().uncompressed_latin1_text {
        fields.insert(t.keyword.clone(), t.text.clone());
    }
    for t in &reader.info().utf8_text {
        if let Ok(v) = t.get_text() {
            fields.insert(t.keyword.clone(), v);
        }
    }
    drop(reader);
    let img = image::open(file).ok()?.to_rgba8();
    Some((img, fields))
}

fn write_png(file: &Path, img: &image::RgbaImage, fields: &[(&str, String)]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = file.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    // written beside it and moved into place, so that another program never reads half a picture
    let tmp = dir.join(format!(".{}.{}.tmp", file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), std::process::id()));
    {
        let f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.width(), img.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        for (k, v) in fields {
            enc.add_text_chunk(k.to_string(), v.clone()).map_err(|e| e.to_string())?;
        }
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(img.as_raw()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())
}

/// A picture made of a file, and what the cache says about it beside the picture (`Thumb::Document::Pages` ...).
pub struct Cached {
    pub img: image::RgbaImage,
    pub fields: HashMap<String, String>,
}

/// The thumbnail of `path` from the shared cache when it is there and fresh, else made by `make` (with any extra text
/// fields to keep beside it), shrunk to 256 px at most and kept. None: the file cannot be drawn (and that is kept too).
pub fn shared(path: &Path, make: impl FnOnce() -> Option<(image::RgbaImage, Vec<(&'static str, String)>)>) -> Option<Cached> {
    cached(&shared_root().join("large"), &shared_root().join("fail/memrium"), path, make)
}

/// Like `shared`, for our own pictures of kind `kind` (not for the other programs to show).
pub fn own(kind: &str, path: &Path, make: impl FnOnce() -> Option<(image::RgbaImage, Vec<(&'static str, String)>)>) -> Option<Cached> {
    let root = own_root(kind);
    cached(&root, &root.join("fail"), path, make)
}

fn cached(dir: &Path, fail_dir: &Path, path: &Path, make: impl FnOnce() -> Option<(image::RgbaImage, Vec<(&'static str, String)>)>) -> Option<Cached> {
    // (the thumbnails themselves are not thumbnailed)
    if path.starts_with(shared_root()) || path.starts_with(cache_home().join("memrium/thumbnails")) {
        return make().map(|(img, _)| Cached { img, fields: HashMap::new() });
    }
    let uri = file_uri(path);
    let name = format!("{}.png", md5_hex(&uri));
    let Some(time) = mtime(path) else { return make().map(|(img, _)| Cached { img, fields: HashMap::new() }) };
    let fresh = |f: &HashMap<String, String>| f.get("Thumb::MTime").and_then(|t| t.trim().parse::<u64>().ok()) == Some(time) && f.get("Thumb::URI").is_none_or(|u| *u == uri);
    if let Some((img, fields)) = read_png(&dir.join(&name)) {
        if fresh(&fields) {
            return Some(Cached { img, fields });
        }
    }
    if read_png(&fail_dir.join(&name)).is_some_and(|(_, f)| fresh(&f)) {
        return None;
    }
    let base: Vec<(&str, String)> = vec![
        ("Thumb::URI", uri.clone()),
        ("Thumb::MTime", time.to_string()),
        ("Thumb::Size", std::fs::metadata(path).map(|m| m.len()).unwrap_or(0).to_string()),
        ("Software", "Memrium Files".to_string()),
    ];
    match make() {
        Some((img, extra)) => {
            let img = if img.width().max(img.height()) > LARGE { image::DynamicImage::ImageRgba8(img).thumbnail(LARGE, LARGE).to_rgba8() } else { img };
            let fields: Vec<(&str, String)> = base.into_iter().chain(extra).collect();
            if let Err(e) = write_png(&dir.join(&name), &img, &fields) {
                eprintln!("[thumbs] keeping the thumbnail of {}: {e}", path.display());
            }
            Some(Cached { img, fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect() })
        }
        None => {
            let _ = write_png(&fail_dir.join(&name), &image::RgbaImage::new(1, 1), &base);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uri_and_its_name_are_the_ones_the_standard_gives() {
        assert_eq!(file_uri(Path::new("/home/jens/photos/me.png")), "file:///home/jens/photos/me.png");
        assert_eq!(file_uri(Path::new("/tmp/a b/ü.png")), "file:///tmp/a%20b/%C3%BC.png");
        // the example of the standard
        assert_eq!(md5_hex("file:///home/jens/photos/me.png"), "c6ee772d9e49320e97ec29a7eb5b1697");
    }

    #[test]
    fn a_thumbnail_is_made_once_and_made_again_when_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("memrium-thumbcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("one.txt");
        std::fs::write(&file, "x").unwrap();
        let (store, fail) = (dir.join("store"), dir.join("fail"));
        let made = std::cell::Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            Some((image::RgbaImage::from_pixel(300, 150, image::Rgba([1, 2, 3, 255])), vec![("Thumb::Document::Pages", "7".to_string())]))
        };
        let a = cached(&store, &fail, &file, make).unwrap();
        assert_eq!((a.img.width(), a.img.height()), (256, 128), "shrunk to the large size");
        let b = cached(&store, &fail, &file, make).unwrap();
        assert_eq!(made.get(), 1, "the second time it comes from the disk");
        assert_eq!(b.fields.get("Thumb::Document::Pages").map(String::as_str), Some("7"));
        assert_eq!(b.fields.get("Thumb::URI"), Some(&file_uri(&file)));
        // a newer file: made again
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        cached(&store, &fail, &file, make).unwrap();
        assert_eq!(made.get(), 2);
        // a file that cannot be drawn is not tried again
        let bad = dir.join("bad.bin");
        std::fs::write(&bad, "y").unwrap();
        let tries = std::cell::Cell::new(0);
        let none = || {
            tries.set(tries.get() + 1);
            None
        };
        assert!(cached(&store, &fail, &bad, none).is_none());
        assert!(cached(&store, &fail, &bad, none).is_none());
        assert_eq!(tries.get(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
