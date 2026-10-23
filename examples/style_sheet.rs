//! The icons of installed apps (Papirus drawings) as they are and in the style of the favourites, side by side:
//! `style_sheet <out.png> [count] [look] [skip]`: `skip` icons are left out first (the ones an earlier sheet showed).
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::{apps, iconstyle, svgedit, wallpapers};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(24);
    let index = apps::IconIndex::build_pack("Papirus");
    let done: Vec<String> = icondoc::saved_names();
    let skip: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut seen = 0usize;
    let mut picked: Vec<(String, String)> = Vec::new();
    for a in apps::scan() {
        if a.icon.is_empty() || a.icon.starts_with('/') || done.contains(&a.icon) || picked.iter().any(|p| p.0 == a.icon) {
            continue;
        }
        match index.find(&a.icon, 128) {
            Some(p) if p.extension().is_some_and(|e| e == "svg") => {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    seen += 1;
                    if seen > skip {
                        picked.push((a.icon.clone(), text));
                    }
                }
            }
            _ => {}
        }
        if picked.len() >= want {
            break;
        }
    }
    // icons of well-known apps that are not installed here, to fill the sheet up (STYLE_EXTRA=a,b,c)
    for name in std::env::var("STYLE_EXTRA").unwrap_or_default().split(',').filter(|n| !n.is_empty()) {
        if picked.len() >= want || done.iter().any(|d| d == name) || picked.iter().any(|p| p.0 == name) {
            continue;
        }
        if let Some(p) = index.find(name, 128).filter(|p| p.extension().is_some_and(|e| e == "svg")) {
            if let Ok(text) = std::fs::read_to_string(&p) {
                picked.push((name.to_string(), text));
            }
        }
    }
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap().1, 1600).expect("wallpaper");
    let (n, gap, cols) = (128u32, 14u32, 4u32);
    let rows = (picked.len() as u32 + cols - 1) / cols;
    let cell = n * 2 + gap;
    let (w, h) = (cell * cols + gap * (cols + 1) - gap * cols + gap * cols - gap * cols, (n + gap) * rows + gap);
    let w = (cell + gap) * cols + gap;
    let mut out = image::DynamicImage::ImageRgba8(wall).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let look = match args.get(3).map(String::as_str) {
        Some("dark") => Appearance::Dark,
        Some("clear") => Appearance::ClearLight,
        _ => Appearance::Default,
    };
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (i, (name, svg)) in picked.iter().enumerate() {
        let (cx, cy) = (gap + (i as u32 % cols) * (cell + gap), gap + (i as u32 / cols) * (n + gap));
        // as it is
        if let Some(img) = svgedit::rasterize(svg, n) {
            image::imageops::overlay(&mut out, &img, cx as i64, cy as i64);
        }
        // in the style
        match iconstyle::favourite_style(name, svg) {
            Some(doc) => {
                let (x, y) = (cx + n, cy);
                r.backdrop = Some(Arc::new(image::imageops::crop_imm(&out, x, y, n, n).to_image()));
                image::imageops::overlay(&mut out, &r.render(&doc, look, n), x as i64, y as i64);
            }
            None => eprintln!("{name}: nothing to draw"),
        }
        eprintln!("{:2} {name}", i + 1);
    }
    out.save(&args[1]).unwrap();
}
