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
        // (kept on disk in the shared cache: a folder of photos opened again is not decoded again)
        let kept = crate::thumbcache::shared(path, || image::open(path).ok().map(|i| (i.thumbnail(crate::thumbcache::LARGE, crate::thumbcache::LARGE).to_rgba8(), Vec::new())))?;
        Some(square(kept.img))
    }
}

// ------------------------------------------------------------------------------------------- code and text, as a minimap

/// Can `path` be drawn as a minimap of its text (a source file or plain text, not a binary document)?
pub fn is_previewable_text(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    !matches!(ext.as_str(), "pdf" | "doc" | "docx" | "odt" | "rtf" | "epub")
}

/// The colour of a language (its keywords and the strip at the foot of the page), by the file's extension.
fn language_colour(ext: &str) -> [u8; 3] {
    match ext {
        "rs" => [200, 87, 43],
        "py" => [59, 120, 194],
        "js" | "mjs" => [214, 160, 0],
        "ts" | "tsx" => [49, 120, 198],
        "sh" | "bash" | "fish" | "zsh" => [70, 76, 86],
        "json" => [138, 92, 240],
        "html" | "htm" | "xml" => [240, 122, 42],
        "css" | "scss" => [41, 101, 241],
        "c" | "h" | "cpp" | "hpp" | "cc" => [92, 107, 192],
        "go" => [0, 160, 200],
        "java" | "kt" => [231, 111, 0],
        "nix" => [102, 110, 230],
        "lua" => [61, 90, 254],
        "wgsl" | "glsl" => [0, 150, 136],
        "php" => [119, 123, 180],
        "rb" => [204, 52, 45],
        "swift" => [240, 81, 56],
        "kt" | "kts" => [127, 82, 255],
        "zig" => [247, 164, 29],
        "dart" => [1, 117, 194],
        "hs" => [122, 98, 180],
        "toml" => [156, 66, 33],
        "yaml" | "yml" => [203, 75, 90],
        "tex" => [31, 157, 143],
        "md" | "markdown" => [91, 101, 115],
        _ => [63, 127, 240],
    }
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "pub", "use", "mod", "impl", "struct", "enum", "trait", "type", "const", "static", "match", "if", "else", "for", "while",
    "loop", "return", "break", "continue", "in", "as", "where", "self", "Self", "crate", "super", "async", "await", "move", "ref", "unsafe", "dyn",
    "def", "class", "import", "from", "with", "try", "except", "finally", "lambda", "yield", "pass", "raise", "and", "or", "not", "is", "None",
    "True", "False", "function", "var", "export", "default", "new", "this", "true", "false", "null", "undefined", "switch", "case", "do", "then",
    "fi", "esac", "done", "local", "end", "elif", "rec", "inherit", "package", "func", "interface", "public", "private", "protected", "void",
    "int", "char", "float", "double", "bool", "unsigned", "include", "define", "struct", "typedef", "extends", "implements", "echo", "set",
];

/// What a piece of a line is, for its colour.
#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Word,
    Keyword,
    Type,
    Str,
    Number,
    Comment,
    Punct,
    Heading,
}

/// The pieces of one line: (start column, length, kind). `block` carries a `/* */` comment over the lines.
fn tokens(line: &str, ext: &str, prose: bool, block: &mut bool) -> Vec<(usize, usize, Tok)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let hash_comment = matches!(ext, "py" | "sh" | "bash" | "fish" | "zsh" | "toml" | "yaml" | "yml" | "nix" | "conf" | "ini" | "cfg");
    if prose {
        // text: words as grey strokes, a markdown heading in the accent
        let heading = line.trim_start().starts_with('#');
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let s = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            out.push((s, i - s, if heading { Tok::Heading } else { Tok::Word }));
        }
        return out;
    }
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied().unwrap_or(' ');
        if *block {
            let s = i;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            if i < chars.len() {
                i += 2;
                *block = false;
            }
            out.push((s, i - s, Tok::Comment));
        } else if c.is_whitespace() {
            i += 1;
        } else if (c == '/' && next == '/') || (c == '#' && hash_comment) || (c == '-' && next == '-' && ext == "lua") {
            out.push((i, chars.len() - i, Tok::Comment));
            break;
        } else if c == '/' && next == '*' {
            *block = true;
        } else if c == '"' || c == '\'' || c == '`' {
            let s = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            // a key of an object (`"name": ...`) is drawn as a name, not as text
            let key = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&':');
            out.push((s, i - s, if key { Tok::Type } else { Tok::Str }));
        } else if c.is_ascii_digit() {
            let s = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            out.push((s, i - s, Tok::Number));
        } else if c.is_alphanumeric() || c == '_' {
            let s = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let w: String = chars[s..i].iter().collect();
            let t = if KEYWORDS.contains(&w.as_str()) {
                Tok::Keyword
            } else if c.is_uppercase() {
                Tok::Type
            } else {
                Tok::Word
            };
            out.push((s, i - s, t));
        } else {
            let s = i;
            while i < chars.len() && !chars[i].is_alphanumeric() && !chars[i].is_whitespace() && !matches!(chars[i], '"' | '\'' | '`' | '_') && !(chars[i] == '/' && matches!(chars.get(i + 1), Some('/') | Some('*'))) {
                i += 1;
            }
            out.push((s, (i - s).max(1), Tok::Punct));
            i = i.max(s + 1);
        }
    }
    out
}

/// Blend a rectangle of colour `c` (alpha `a`) into `img`; the left and right edges are partly covered pixels.
fn bar(img: &mut image::RgbaImage, x0: f32, x1: f32, y: u32, h: u32, c: [u8; 3], a: f32) {
    let (w, ih) = img.dimensions();
    let (xa, xb) = (x0.max(0.0), x1.min(w as f32));
    if xb <= xa {
        return;
    }
    for py in y..(y + h).min(ih) {
        let mut px = xa.floor() as u32;
        while (px as f32) < xb {
            let cover = ((px as f32 + 1.0).min(xb) - (px as f32).max(xa)).clamp(0.0, 1.0) * a;
            let p = img.get_pixel_mut(px, py);
            for k in 0..3 {
                p.0[k] = (p.0[k] as f32 * (1.0 - cover) + c[k] as f32 * cover).round() as u8;
            }
            px += 1;
        }
    }
}


/// Where to draw on the page of a type icon: inside its margins, and the folded corner at the top right to keep clear of.
#[derive(Clone, Copy)]
struct Page {
    left: f32,
    right: f32,
    top: u32,
    bottom: u32,
    fold_x: f32,
    fold_y: f32,
    /// the paper is dark (the light look of our icons: a graphite page)
    dark: bool,
}

/// The type icon `base` (a page) with the inside of its page made blank (the type's own lines or badge go), and where that
/// is. None: the icon is not a page that can be found.
fn blank_page(base: &image::RgbaImage) -> Option<(image::RgbaImage, Page)> {
    let (w, h) = base.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in base.enumerate_pixels() {
        if p.0[3] > 220 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        return None;
    }
    let (pw, ph) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let inset = (pw * 0.09).max(3.0);
    let (left, right) = (x0 as f32 + inset, x1 as f32 - inset);
    let (top, bottom) = ((y0 as f32 + inset * 1.2) as u32, (y1 as f32 - inset) as u32);
    // the folded corner at the top right: the lines beside it are kept short
    let (fold_x, fold_y) = (x0 as f32 + pw * 0.66, y0 as f32 + ph * 0.24);

    // the page made blank again (the type's own lines or badge go), in the colour of its paper
    // (taken near the top left, where a page has nothing on it yet: no lines, no badge)
    let paper = {
        let p = base.get_pixel(((x0 as f32 + pw * 0.12) as u32).min(w - 1), ((y0 as f32 + ph * 0.1) as u32).min(h - 1)).0;
        if p[3] > 200 { [p[0], p[1], p[2]] } else { [250, 251, 252] }
    };
    let dark = (paper[0] as u32 * 3 + paper[1] as u32 * 6 + paper[2] as u32) / 10 < 128;
    let mut out = base.clone();
    for py in top.saturating_sub(1)..bottom + 1 {
        let r = if (py as f32) < fold_y { fold_x - 2.0 } else { right + 1.0 };
        bar(&mut out, left - 1.0, r, py, 1, paper, 1.0);
    }

    Some((out, Page { left, right, top, bottom, fold_x, fold_y, dark }))
}

/// The type icon `base` (a page) with the start of the text file at `path` drawn on it as a minimap: its lines with their
/// indentation, the pieces of each in the colours of a code editor, the keywords in the colour of the language. None for
/// a file that is not text, or an empty one.
pub fn code_preview(base: &image::RgbaImage, path: &Path, prose: bool) -> Option<image::RgbaImage> {
    use std::io::Read;
    let mut buf = vec![0u8; 12 * 1024];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(&buf);
    if text.trim().is_empty() {
        return None;
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    let lang = language_colour(&ext);
    let (mut out, pg) = blank_page(base)?;
    let Page { left, right, top, bottom, fold_x, fold_y, dark } = pg;
    // on graphite paper: the language's colour lighter, so that a dark one (shell) still shows
    let lang = if dark { lang.map(|v| v + ((255 - v) as f32 * 0.35) as u8) } else { lang };
    let h = base.height();

    // a row is 3 pixels at 128 (a stroke of 2 and a gap of 1); 80 columns fill the width
    let scale = h as f32 / 128.0;
    let row = (3.0 * scale).round().max(2.0) as u32;
    let stroke = (row as f32 * 0.62).round().max(1.0) as u32;
    let col = (right - left) / 80.0;
    let colour = |t: Tok| -> ([u8; 3], f32) {
        if dark {
            return match t {
                Tok::Keyword | Tok::Heading => (lang, 0.95),
                Tok::Type => ([110, 172, 240], 0.9),
                Tok::Str => ([112, 208, 150], 0.9),
                Tok::Number => ([240, 182, 70], 0.9),
                Tok::Comment => ([120, 130, 142], 0.8),
                Tok::Punct => ([150, 158, 170], 0.6),
                Tok::Word => ([206, 212, 220], if prose { 0.6 } else { 0.85 }),
            };
        }
        match t {
            Tok::Keyword | Tok::Heading => (lang, 0.95),
            Tok::Type => ([47, 127, 193], 0.85),
            Tok::Str => ([40, 150, 96], 0.85),
            Tok::Number => ([205, 140, 0], 0.85),
            Tok::Comment => ([150, 160, 172], 0.6),
            Tok::Punct => ([150, 158, 170], 0.55),
            Tok::Word => ([88, 98, 112], if prose { 0.55 } else { 0.8 }),
        }
    };
    let foot = bottom.saturating_sub((5.0 * scale) as u32);
    let mut y = top;
    let mut block = false;
    for line in text.lines() {
        if y + stroke > foot {
            break;
        }
        let line = line.replace('\t', "    ");
        let limit = if (y as f32) < fold_y { fold_x - 3.0 } else { right };
        for (s, len, t) in tokens(&line, &ext, prose, &mut block) {
            let (c, a) = colour(t);
            let (bx0, bx1) = (left + s as f32 * col, (left + (s + len) as f32 * col).min(limit));
            bar(&mut out, bx0, bx1, y, if t == Tok::Heading { stroke + 1 } else { stroke }, c, a);
        }
        y += row;
    }
    // the language's strip at the foot of the page
    if !prose {
        bar(&mut out, left, right, bottom - (2.0 * scale).round().max(2.0) as u32, (2.0 * scale).round().max(2.0) as u32, lang, 0.9);
    }
    // the badge of the type (README, Nix, a licence ...) stays as a small stamp in the corner: the lines say what is in the
    // file, the stamp what kind of file it is
    if let Some(stamp) = badge_sprite(base, &pg) {
        let width = ((right - left) * 0.64).round().max(8.0) as u32;
        let height = (stamp.height() as f32 * width as f32 / stamp.width() as f32).round().max(1.0) as u32;
        let stamp = image::imageops::resize(&stamp, width, height, image::imageops::FilterType::Lanczos3);
        let x0 = (right as u32).saturating_sub(width + (scale * 0.5) as u32);
        let y0 = bottom.saturating_sub(height + (5.0 * scale) as u32);
        // a soft shadow under the stamp first (it sits on the lines), then the stamp itself
        let drop = (1.5 * scale).round().max(1.0) as u32;
        for pass in 0..2 {
            for (sx, sy, p) in stamp.enumerate_pixels() {
                let (x, y) = (x0 + sx, y0 + sy + if pass == 0 { drop } else { 0 });
                if x >= out.width() || y >= out.height() || p.0[3] == 0 {
                    continue;
                }
                let (a, rgb) = if pass == 0 { (p.0[3] as f32 / 255.0 * 0.45, [0u8, 0, 0]) } else { (p.0[3] as f32 / 255.0, [p.0[0], p.0[1], p.0[2]]) };
                let q = out.get_pixel_mut(x, y);
                // (not past the edge of the page)
                if q.0[3] == 0 {
                    continue;
                }
                for k in 0..3 {
                    q.0[k] = (q.0[k] as f32 * (1.0 - a) + rgb[k] as f32 * a).round() as u8;
                }
            }
        }
    }
    Some(out)
}

/// The badge on the page of a type icon (the coloured block with a mark on it that our own file icons carry), cut out by how
/// much each pixel differs from the paper beside it. None: nothing on the page is a badge (the lines of a plain text icon
/// are far too thin and gappy to count, and an empty page has nothing).
fn badge_sprite(base: &image::RgbaImage, pg: &Page) -> Option<image::RgbaImage> {
    let (left, right) = (pg.left.ceil() as u32 + 1, pg.right.floor() as u32);
    let (top, bottom) = (pg.top, pg.bottom.min(base.height().saturating_sub(1)));
    if right <= left + 8 || bottom <= top + 8 {
        return None;
    }
    // the page's paper is darker or lighter from row to row (a gradient), so each row is compared with its own left edge
    let diff = |x: u32, y: u32| -> f32 {
        let (p, r) = (base.get_pixel(x, y).0, base.get_pixel(left, y).0);
        (0..3).map(|k| (p[k] as f32 - r[k] as f32).abs()).fold(0.0, f32::max)
    };
    let (mut x0, mut y0, mut x1, mut y1, mut inked) = (right, bottom, left, top, 0u32);
    for y in top..=bottom {
        for x in left..right {
            // (the folded corner is not the badge's)
            if (y as f32) < pg.fold_y && x as f32 >= pg.fold_x - 2.0 {
                continue;
            }
            if diff(x, y) > 40.0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                inked += 1;
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let fill = inked as f32 / (bw * bh) as f32;
    let (pw, ph) = ((right - left) as f32, (bottom - top) as f32);
    // a badge is a block: mostly filled, and a good part of the page
    if fill < 0.6 || (bw as f32) < pw * 0.35 || (bh as f32) < ph * 0.2 {
        return None;
    }
    let mut sprite = image::RgbaImage::new(bw, bh);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let t = ((diff(x, y) - 12.0) / 33.0).clamp(0.0, 1.0);
            let a = t * t * (3.0 - 2.0 * t);
            let p = base.get_pixel(x, y).0;
            sprite.put_pixel(x - x0, y - y0, image::Rgba([p[0], p[1], p[2], (a * 255.0).round() as u8]));
        }
    }
    Some(sprite)
}

#[cfg(test)]
mod code_tests {
    use super::*;

    #[test]
    fn a_source_file_is_drawn_on_its_page_and_a_binary_is_not() {
        let dir = std::env::temp_dir().join(format!("memrium-code-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a white page on nothing, 128 px
        let mut page = image::RgbaImage::new(128, 128);
        for y in 14..114 {
            for x in 29..102 {
                page.put_pixel(x, y, image::Rgba([250, 251, 252, 255]));
            }
        }
        let src = dir.join("main.rs");
        std::fs::write(&src, "// hello\nfn main() {\n    let s = \"text\";\n    println!(\"{s}\", 42);\n}\n").unwrap();
        let img = code_preview(&page, &src, false).expect("drawn");
        let inked = img.pixels().filter(|p| p.0[3] == 255 && (p.0[0] as i32 - p.0[2] as i32).abs() > 20).count();
        assert!(inked > 20, "coloured strokes on the page: {inked}");
        assert_eq!(img.get_pixel(5, 5).0[3], 0, "nothing outside the page");
        // a page with a badge on it keeps the badge as a stamp; a page with only lines has nothing to keep
        let mut badged = page.clone();
        for y in 60..90 {
            for x in 40..90 {
                badged.put_pixel(x, y, image::Rgba([40, 110, 230, 255]));
            }
        }
        let stamped = code_preview(&badged, &src, false).expect("drawn");
        let blue = |img: &image::RgbaImage| img.pixels().filter(|p| p.0[3] == 255 && p.0[2] > 180 && p.0[0] < 90).count();
        assert!(blue(&stamped) > 40, "the badge is a stamp now: {}", blue(&stamped));
        assert_eq!(blue(&img), 0, "no badge, no stamp");
        let bin = dir.join("data.json");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert!(code_preview(&page, &bin, false).is_none());
        let empty = dir.join("empty.txt");
        std::fs::write(&empty, "  \n").unwrap();
        assert!(code_preview(&page, &empty, true).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pieces_of_a_line_are_told_apart() {
        let mut block = false;
        let t = tokens("let x = \"a\"; // note", "rs", false, &mut block);
        let kinds: Vec<Tok> = t.iter().map(|p| p.2).collect();
        assert_eq!(kinds[0] as u8, Tok::Keyword as u8);
        assert!(kinds.contains(&Tok::Str) && *kinds.last().unwrap() == Tok::Comment);
        let t = tokens("  \"name\": 1,", "json", false, &mut block);
        assert!(t[0].2 == Tok::Type && t.iter().any(|p| p.2 == Tok::Number), "a key is a name");
        tokens("a /* open", "c", false, &mut block);
        assert!(block, "a comment left open goes on");
        assert_eq!(tokens("still */ b", "c", false, &mut block)[0].2 as u8, Tok::Comment as u8);
        assert!(!block);
    }
}

// ---------------------------------------------------------------------------------------------- PDF: the first page

/// The first page of a PDF, about `LARGE` px high, and how many pages it has. Drawn in pure Rust (hayro); a file that is
/// locked with a password, damaged or too big gives None.
fn pdf_page(path: &Path) -> Option<(image::RgbaImage, usize)> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    if std::fs::metadata(path).ok()?.len() > 80_000_000 {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    let pdf = Pdf::new(std::sync::Arc::new(data)).ok()?;
    let pages = pdf.pages();
    let page = pages.first()?;
    let (w, h) = page.render_dimensions();
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let side = crate::thumbcache::LARGE as f32;
    let scale = (side / w.max(h)).min(4.0);
    let settings = hayro::RenderSettings { x_scale: scale, y_scale: scale, bg_color: hayro::vello_cpu::color::palette::css::WHITE, ..Default::default() };
    let pix = hayro::render(page, &hayro::RenderCache::new(), &InterpreterSettings::default(), &settings);
    let (pw, ph) = (pix.width() as u32, pix.height() as u32);
    let raw: Vec<u8> = pix.take_unpremultiplied().into_iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    Some((image::RgbaImage::from_raw(pw, ph, raw)?, pages.len()))
}

/// How many sheets lie behind the first page for a document of `pages` pages.
fn sheets_behind(pages: usize) -> usize {
    match pages {
        0 | 1 => 0,
        2..=5 => 1,
        6..=20 => 2,
        21..=100 => 3,
        _ => 4,
    }
}

/// A PDF as a stack of paper: its first page on top, as many sheets behind it as its length asks (a leaflet is one sheet,
/// a book a thick stack), in a 128 px square. The page is kept in the shared cache with its page count.
pub fn pdf_preview(path: &Path) -> Option<image::RgbaImage> {
    let kept = crate::thumbcache::shared(path, || pdf_page(path).map(|(img, n)| (img, vec![("Thumb::Document::Pages", n.to_string())])))?;
    let pages = kept.fields.get("Thumb::Document::Pages").and_then(|n| n.parse().ok()).unwrap_or(1);
    let behind = sheets_behind(pages);
    let n = 128u32;
    // the front page fitted into the square, leaving room up and to the right for the sheets behind
    let step = 3u32;
    let room = behind as u32 * step;
    let (iw, ih) = kept.img.dimensions();
    let s = ((n - 14 - room) as f32 / iw as f32).min((n - 10 - room) as f32 / ih as f32);
    let (fw, fh) = (((iw as f32 * s).round() as u32).max(8), ((ih as f32 * s).round() as u32).max(8));
    let page = image::imageops::resize(&kept.img, fw, fh, image::imageops::FilterType::Triangle);
    let (x0, y0) = ((n - fw - room) / 2, (n - fh + room) / 2);
    let mut out = image::RgbaImage::new(n, n);
    // the sheets behind, back first, each a little lighter grey at the edge
    for i in (1..=behind as u32).rev() {
        let (x, y) = (x0 + i * step, y0 - i * step);
        frame(&mut out, x as f32, y as f32, fw as f32, fh as f32, [252, 252, 253], 1.0, true);
        frame(&mut out, x as f32, y as f32, fw as f32, fh as f32, [190, 195, 202], 1.0, false);
    }
    image::imageops::overlay(&mut out, &page, x0 as i64, y0 as i64);
    frame(&mut out, x0 as f32, y0 as f32, fw as f32, fh as f32, [190, 195, 202], 1.0, false);
    Some(out)
}

/// A rectangle filled (or only its one-pixel edge) in colour `c`, blended over what is there.
#[allow(clippy::too_many_arguments)]
fn frame(img: &mut image::RgbaImage, x: f32, y: f32, w: f32, h: f32, c: [u8; 3], a: f32, filled: bool) {
    let (x0, y0, x1, y1) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32, ((x + w).round() as u32).min(img.width()), ((y + h).round() as u32).min(img.height()));
    for py in y0..y1 {
        for px in x0..x1 {
            if !filled && px != x0 && px != x1 - 1 && py != y0 && py != y1 - 1 {
                continue;
            }
            let p = img.get_pixel_mut(px, py);
            let under = p.0[3] as f32 / 255.0;
            let out_a = a + under * (1.0 - a);
            if out_a <= 0.0 {
                continue;
            }
            for k in 0..3 {
                p.0[k] = ((c[k] as f32 * a + p.0[k] as f32 * under * (1.0 - a)) / out_a).round() as u8;
            }
            p.0[3] = (out_a * 255.0).round() as u8;
        }
    }
}

// ------------------------------------------------------------------------------------------------ songs: the waveform

/// How many bars the waveform of a song has.
const WAVE_BARS: usize = 48;

/// The loudness of a song along its length, `WAVE_BARS` values 0..255 (the loudest bar is 255), by ffmpeg (the first ten
/// minutes, as one channel at 4 kHz). Kept as a one-row picture in our own cache.
fn wave_of(path: &Path) -> Option<Vec<u8>> {
    let kept = crate::thumbcache::own("waveform", path, || {
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-nostdin", "-t", "600", "-i"])
            .arg(path)
            .args(["-vn", "-ac", "1", "-ar", "4000", "-f", "s16le", "-"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        let samples: Vec<f32> = out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        if samples.len() < WAVE_BARS * 4 {
            return None;
        }
        // the loudness of each part (root mean square), then the loudest made full height
        let per = samples.len() / WAVE_BARS;
        let rms: Vec<f32> = samples.chunks(per).take(WAVE_BARS).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect();
        let top = rms.iter().cloned().fold(0.0f32, f32::max).max(1e-4);
        let row = image::RgbaImage::from_fn(WAVE_BARS as u32, 1, |x, _| {
            let v = ((rms[x as usize] / top).powf(0.8) * 255.0).round() as u8;
            image::Rgba([v, v, v, 255])
        });
        Some((row, Vec::new()))
    })?;
    Some(kept.img.pixels().map(|p| p.0[0]).collect())
}

/// The bars of a waveform across [`left`, `right`] around the line `mid`, `half` px up and down at most.
#[allow(clippy::too_many_arguments)]
fn draw_wave(img: &mut image::RgbaImage, wave: &[u8], left: f32, right: f32, mid: f32, half: f32, c: [u8; 3], a: f32) {
    let step = (right - left) / wave.len() as f32;
    for (i, &v) in wave.iter().enumerate() {
        let hh = (v as f32 / 255.0 * half).max(1.0);
        let (x0, x1) = (left + i as f32 * step + step * 0.18, left + (i + 1) as f32 * step - step * 0.18);
        let (y0, y1) = ((mid - hh).round() as u32, (mid + hh).round() as u32);
        bar(img, x0, x1, y0, (y1 - y0).max(1), c, a);
    }
}

/// A song: its cover with the waveform across the lower part in white, or (without a cover) the waveform drawn on its
/// type icon `base` in the colour of music. None: neither could be had.
pub fn audio_preview(base: Option<&image::RgbaImage>, path: &Path) -> Option<image::RgbaImage> {
    let wave = wave_of(path);
    let cover = crate::thumbcache::shared(path, || {
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-nostdin", "-i"])
            .arg(path)
            .args(["-an", "-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        image::load_from_memory(&out.stdout).ok().map(|i| (i.thumbnail(crate::thumbcache::LARGE, crate::thumbcache::LARGE).to_rgba8(), Vec::new()))
    });
    match (cover, wave) {
        (Some(c), wave) => {
            let mut out = square(c.img);
            if let Some(w) = wave {
                // the lower third darkened, so that the white bars read on any cover
                let (n, wd, from) = (out.height(), out.width() as f32, out.height() * 62 / 100);
                for y in from..n {
                    let t = (y - from) as f32 / (n - from) as f32;
                    bar(&mut out, 0.0, wd, y, 1, [0, 0, 0], 0.15 + 0.4 * t);
                }
                let mid = n as f32 * 0.82;
                draw_wave(&mut out, &w, 10.0, wd - 10.0, mid, n as f32 * 0.11, [255, 255, 255], 0.95);
            }
            Some(out)
        }
        (None, Some(w)) => {
            let (mut out, pg) = blank_page(base?)?;
            // (on the narrow page half as many bars, each the louder of two)
            let w: Vec<u8> = w.chunks(2).map(|c| c.iter().copied().max().unwrap_or(0)).collect();
            let mid = (pg.top as f32 + pg.bottom as f32) / 2.0 + 4.0;
            draw_wave(&mut out, &w, pg.left, pg.right, mid, (pg.bottom - pg.top) as f32 * 0.3, [255, 59, 107], 0.95);
            Some(out)
        }
        (None, None) => None,
    }
}

// ------------------------------------------------------------------------------------ a folder with its files in it

/// Where the front of the folder icons of our theme begins, as a fraction of the picture's height (`FRONT_TOP` of the
/// `kind_icons` generator, 216 of 512): the files lie between the back and the front, so only their tops peek out.
pub const FOLDER_FRONT: f32 = 216.0 / 512.0;

/// A file drawn as a paper that peeks out of a folder.
pub struct Sheet {
    pub img: image::RgbaImage,
    /// a photo (it gets a white mat) and not a page of its own
    pub photo: bool,
}

/// `img` cut down to what is drawn on it (a thumbnail comes in a transparent square, a type icon with margins).
fn trimmed(img: &image::RgbaImage) -> Option<image::RgbaImage> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 24 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(image::imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image())
}

/// `path` as a paper: what its icon shows in Files (the photo, the first page of a PDF, the start of a text on its page,
/// a song's cover or waveform), else the type icon `index` has for it.
pub fn file_sheet(index: &crate::apps::IconIndex, path: &Path) -> Option<Sheet> {
    use crate::fsops::{self, Kind};
    let name = path.file_name()?.to_string_lossy().to_string();
    let kind = fsops::kind_of(&name, false);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let e = fsops::Entry { name: name.clone(), path: path.to_path_buf(), is_dir: false, size, modified: 0, hidden: false, kind, app: None };
    let type_icon = || fsops::icon_names(&e).iter().find_map(|n| index.load(n, 128));
    let page = |img: Option<image::RgbaImage>| img.and_then(|i| trimmed(&i)).map(|img| Sheet { img, photo: false });
    if kind == Kind::Image {
        return thumbnail(path).and_then(|i| trimmed(&i)).map(|img| Sheet { img, photo: true });
    }
    if name.to_ascii_lowercase().ends_with(".pdf") {
        let kept = crate::thumbcache::shared(path, || pdf_page(path).map(|(img, n)| (img, vec![("Thumb::Document::Pages", n.to_string())])));
        if let Some(k) = kept {
            return Some(Sheet { img: k.img, photo: false });
        }
    }
    if kind == Kind::Audio {
        if let Some(s) = page(audio_preview(type_icon().as_ref(), path)) {
            return Some(s);
        }
    }
    if matches!(kind, Kind::Code | Kind::Text) && size < 4_000_000 && is_previewable_text(path) {
        if let Some(base) = type_icon() {
            if let Some(s) = page(code_preview(&base, path, kind == Kind::Text)) {
                return Some(s);
            }
        }
    }
    page(type_icon())
}

/// Up to `n` files of `dir` to show: those with something to see (a photo, a PDF, a text, a song) before the rest, the newest
/// first. Hidden files and folders are left out.
fn files_in(dir: &Path, n: usize) -> Vec<PathBuf> {
    use crate::fsops::{kind_of, Kind};
    let mut found: Vec<(bool, std::time::SystemTime, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten().take(800) {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        let path = e.path();
        let Ok(m) = std::fs::metadata(&path) else { continue };
        if m.is_dir() {
            continue;
        }
        let rich = name.to_ascii_lowercase().ends_with(".pdf") || matches!(kind_of(&name, false), Kind::Image | Kind::Audio | Kind::Code | Kind::Text);
        found.push((rich, m.modified().unwrap_or(std::time::UNIX_EPOCH), path));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    found.into_iter().take(n).map(|f| f.2).collect()
}

fn premultiplied(p: [u8; 4]) -> [f32; 4] {
    let a = p[3] as f32 / 255.0;
    [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
}

/// Bilinear sample of `img` at (`x`, `y`) in pixels, premultiplied, clamped at the edge.
fn sample(img: &image::RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = img.dimensions();
    let (fx, fy) = ((x - 0.5).clamp(0.0, w as f32 - 1.0), (y - 0.5).clamp(0.0, h as f32 - 1.0));
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0f32; 4];
    for (px, py, wgt) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x1, y0, tx * (1.0 - ty)), (x0, y1, (1.0 - tx) * ty), (x1, y1, tx * ty)] {
        let p = premultiplied(img.get_pixel(px, py).0);
        for k in 0..4 {
            out[k] += p[k] * wgt;
        }
    }
    out
}

/// One paper of the fan laid over `canvas` (premultiplied, `w` x `h`; the 512 grid of the theme's icons scaled by `k`): a
/// sheet of up to 256 x 230 whose top edge is at `top`, turned by `deg` about the point the other papers turn about.
#[allow(clippy::too_many_arguments)]
fn lay_sheet(canvas: &mut [[f32; 4]], w: u32, h: u32, k: f32, sheet: &Sheet, deg: f32, top: f32) {
    const BOX: (f32, f32) = (268.0, 236.0);
    const PIVOT: (f32, f32) = (256.0, 340.0);
    let mat = if sheet.photo { 6.0 } else { 0.0 };
    let (sw, sh) = (sheet.img.width() as f32, sheet.img.height() as f32);
    let fit = ((BOX.0 - 2.0 * mat) / sw).min((BOX.1 - 2.0 * mat) / sh);
    let (iw, ih) = (sw * fit, sh * fit);
    let (pw, ph) = (iw + 2.0 * mat, ih + 2.0 * mat);
    let (x0, x1, y0, y1) = (256.0 - pw / 2.0, 256.0 + pw / 2.0, top, top + ph);
    let (s, c) = (-deg.to_radians()).sin_cos();
    // the signed distance of a point of the unturned paper to the paper's edge (negative inside), in grid units
    let dist = |u: f32, v: f32, down: f32| {
        let (dx, dy) = ((x0 - u).max(u - x1), (y0 - (v - down)).max((v - down) - y1));
        (dx.max(0.0).hypot(dy.max(0.0))) + dx.max(dy).min(0.0)
    };
    for py in 0..h {
        for px in 0..w {
            let (gx, gy) = ((px as f32 + 0.5) / k - PIVOT.0, (py as f32 + 0.5) / k - PIVOT.1);
            // back into the unturned paper
            let (u, v) = (gx * c - gy * s + PIVOT.0, gx * s + gy * c + PIVOT.1);
            let at = py as usize * w as usize + px as usize;
            // its shadow first: soft, a little down
            let sd = dist(u, v, 5.0) * k;
            // (cast only onto what is there: the folder and the papers behind, not onto the air above them)
            let d0 = canvas[at][3];
            let shadow = (0.5 - sd / 7.0).clamp(0.0, 1.0) * 0.22 * d0;
            if shadow > 0.0 {
                for ch in canvas[at].iter_mut().take(3) {
                    *ch *= 1.0 - shadow;
                }
            }
            let cover = (0.5 - dist(u, v, 0.0) * k).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let src = if sheet.photo && (u < x0 + mat || u > x1 - mat || v < y0 + mat || v > y1 - mat) {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                let (ix, iy) = ((u - x0 - mat) / iw * sw, (v - y0 - mat) / ih * sh);
                sample(&sheet.img, ix, iy)
            };
            let d = &mut canvas[at];
            for ch in 0..4 {
                d[ch] = src[ch] * cover + d[ch] * (1.0 - src[3] * cover);
            }
        }
    }
}

/// The empty folder icon `base` of our theme with a few of the files of `dir` in it: their tops peek out from between the back
/// and the front, fanned like the papers of the full folder. `sheet` makes the paper of a file. None: nothing to show.
pub fn folder_contents(base: &image::RgbaImage, dir: &Path, sheet: &dyn Fn(&Path) -> Option<Sheet>) -> Option<image::RgbaImage> {
    let sheets: Vec<Sheet> = files_in(dir, 3).iter().filter_map(|p| sheet(p)).collect();
    if sheets.is_empty() {
        return None;
    }
    let (w, h) = base.dimensions();
    let k = w as f32 / 512.0;
    let mut canvas: Vec<[f32; 4]> = base.pixels().map(|p| premultiplied(p.0)).collect();
    // (turn, top edge) of the papers, the one in front first
    let slots: &[(f32, f32)] = match sheets.len() {
        1 => &[(0.0, 84.0)],
        2 => &[(0.0, 84.0), (-9.0, 76.0)],
        _ => &[(0.0, 84.0), (-11.0, 76.0), (10.0, 74.0)],
    };
    for (i, s) in sheets.iter().enumerate().rev() {
        lay_sheet(&mut canvas, w, h, k, s, slots[i].0, slots[i].1);
    }
    // the front shades what is just above it
    let front = (FOLDER_FRONT * h as f32) as u32;
    let band = ((h as f32 * 0.07) as u32).max(2);
    for y in front.saturating_sub(band)..front {
        let t = (y - (front - band)) as f32 / band as f32;
        let m = 1.0 - 0.30 * t * t;
        for x in 0..w {
            for ch in canvas[y as usize * w as usize + x as usize].iter_mut().take(3) {
                *ch *= m;
            }
        }
    }
    // the front and what is below it, from the icon itself, over the papers
    for y in front..h {
        for x in 0..w {
            let b = premultiplied(base.get_pixel(x, y).0);
            let d = &mut canvas[y as usize * w as usize + x as usize];
            for ch in 0..4 {
                d[ch] = b[ch] + d[ch] * (1.0 - b[3]);
            }
        }
    }
    let mut out = image::RgbaImage::new(w, h);
    for (p, c) in out.pixels_mut().zip(&canvas) {
        let a = c[3].clamp(0.0, 1.0);
        let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
        *p = image::Rgba([un(c[0]), un(c[1]), un(c[2]), (a * 255.0).round() as u8]);
    }
    Some(out)
}

#[cfg(test)]
mod folder_tests {
    use super::*;

    #[test]
    fn the_files_of_a_folder_peek_out_above_its_front_and_the_front_stays_as_it_was() {
        let (w, h) = (128u32, 128u32);
        let front = (FOLDER_FRONT * h as f32) as u32;
        // a folder of a back (blue, all the way up) and a red front below the line
        let mut base = image::RgbaImage::new(w, h);
        for y in 10..h - 10 {
            for x in 10..w - 10 {
                base.put_pixel(x, y, if y >= front { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([30, 30, 200, 255]) });
            }
        }
        let dir = std::env::temp_dir().join(format!("memrium-fan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::write(dir.join("b.txt"), "world").unwrap();
        let green = |_: &Path| Some(Sheet { img: image::RgbaImage::from_pixel(60, 80, image::Rgba([20, 220, 20, 255])), photo: false });
        let out = folder_contents(&base, &dir, &green).expect("two files, so a picture");
        assert_eq!(out.dimensions(), (w, h));
        // above the front, in the middle: a green paper now, where the back was
        let p = out.get_pixel(w / 2, front - 6).0;
        assert!(p[1] > 150 && p[0] < 120, "paper expected, got {p:?}");
        // below the front line, where the paper would reach: still the front
        assert_eq!(out.get_pixel(w / 2, front + 20).0, [200, 30, 30, 255]);
        // an empty folder has nothing to show
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(folder_contents(&base, &empty, &green).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
