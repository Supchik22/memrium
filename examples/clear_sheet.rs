//! The clear look of the icons in each clear style, a row per style (Frost, Contrast, Ink, Colour), over a picture:
//! `clear_sheet <out.png> [backdrop.png] [dark]`.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{ClearStyle, GlassTune, Renderer};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dark = args.iter().any(|a| a == "dark");
    let names = ["utilities-terminal", "utilities-file-archiver", "preferences-system", "brave-browser", "discord", "steam_icon_730", "krita", "org.prismlauncher.PrismLauncher", "gvim", "mpv", "libreoffice-writer", "org.audacityteam.Audacity"];
    let docs: Vec<_> = icondoc::saved_docs().into_iter().filter(|d| names.contains(&d.name.as_str())).collect();
    let (n, gap) = (112u32, 14u32);
    let w = gap + docs.len() as u32 * (n + gap);
    let h = gap + 4 * (n + gap);
    let back = match args.get(2).filter(|a| a.ends_with(".png")).and_then(|p| image::open(p).ok()) {
        Some(img) => image::imageops::resize(&img.to_rgba8(), w, h, image::imageops::FilterType::Triangle),
        None => image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(150 + 60 * x / w) as u8, (160 + 40 * y / h) as u8, 235, 255])),
    };
    let mut out = back.clone();
    let look = if dark { Appearance::ClearDark } else { Appearance::ClearLight };
    for (row, style) in ClearStyle::ALL.iter().enumerate() {
        let mut r = Renderer::new();
        r.tune = GlassTune { clear: *style, colour_clear: *style == ClearStyle::Colour, ..GlassTune::load() };
        for (k, doc) in docs.iter().enumerate() {
            let (x, y) = (gap + k as u32 * (n + gap), gap + row as u32 * (n + gap));
            // the plate as the shell draws it live: what is behind, frosted, with the style's tint, in a squircle
            let crop = image::imageops::crop_imm(&back, x, y, n, n).to_image();
            let frosted = image::imageops::blur(&crop, 7.0);
            let tint = style.plate_tint(dark);
            let side = n as f32 * memrium_de::iconrender::plate_fit();
            let (c, rad) = (n as f32 / 2.0, side * 0.24);
            for (px, py, p) in frosted.enumerate_pixels() {
                let (dx, dy) = (((px as f32 + 0.5 - c).abs() - (side / 2.0 - rad)).max(0.0), ((py as f32 + 0.5 - c).abs() - (side / 2.0 - rad)).max(0.0));
                let cover = (rad - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0) * ((side / 2.0 + 0.5 - (px as f32 + 0.5 - c).abs()).clamp(0.0, 1.0)) * ((side / 2.0 + 0.5 - (py as f32 + 0.5 - c).abs()).clamp(0.0, 1.0));
                if cover <= 0.0 {
                    continue;
                }
                // a little light along the top edge, as the glass has
                let top = ((py as f32 - (c - side / 2.0)) / 3.0).clamp(0.0, 1.0);
                let rim = (1.0 - top) * 0.35;
                let o = out.get_pixel_mut(x + px, y + py);
                for i in 0..3 {
                    let v = p[i] as f32 / 255.0 * (1.0 - tint[3]) + tint[i] * tint[3];
                    let v = v + (1.0 - v) * rim;
                    o[i] = ((o[i] as f32 / 255.0) * (1.0 - cover) + v * cover).clamp(0.0, 1.0).mul_add(255.0, 0.0) as u8;
                }
            }
            r.bare = true;
            r.backdrop = Some(Arc::new(frosted));
            let mut img = r.render(doc, look, n);
            if style.follows_backdrop() {
                // what the shell's shader does: the symbol in a shade of the colour behind the icon
                let (mut sum, mut cnt) = ([0.0f32; 3], 0.0);
                for p in crop.pixels() {
                    for i in 0..3 {
                        sum[i] += p[i] as f32 / 255.0;
                    }
                    cnt += 1.0;
                }
                let back = sum.map(|v| v / cnt);
                for p in img.pixels_mut() {
                    if p[3] == 0 {
                        continue;
                    }
                    let lum = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0;
                    let c = memrium_de::iconrender::frost_ink(back, lum, dark);
                    for i in 0..3 {
                        p[i] = (c[i] * 255.0) as u8;
                    }
                }
            }
            image::imageops::overlay(&mut out, &img, x as i64, y as i64);
        }
    }
    out.save(&args[1]).unwrap();
    eprintln!("{} icons x {} styles", docs.len(), ClearStyle::ALL.len());
}
