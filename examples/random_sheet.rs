//! A sheet of random icons of the saved documents (the ones drawn in the composer), as glass on the photo: `random_sheet <out.png> [cols] [rows] [tinted]`.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::wallpapers;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cols: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);
    let rows: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    // the icons that have been drawn: the saved documents
    let mut docs = icondoc::saved_docs();
    // a small generator, seeded by the clock
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for i in (1..docs.len()).rev() {
        docs.swap(i, (next() % (i as u64 + 1)) as usize);
    }
    docs.truncate((cols * rows) as usize);
    let (n, gap) = (96u32, 18u32);
    let (w, h) = ((n + gap) * cols + gap, (n + gap) * rows + gap);
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap().1, 1600).expect("wallpaper");
    let mut out = image::DynamicImage::ImageRgba8(wall).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    // `tinted`: the tinted look, each icon toned in its own colour
    let tinted = args.get(4).is_some_and(|a| a == "tinted");
    r.tint_by_icon = tinted;
    let look = if tinted { Appearance::TintedLight } else { Appearance::Default };
    for (shown, doc) in (0u32..).zip(docs.iter()) {
        let (x, y) = (gap + (shown % cols) * (n + gap), gap + (shown / cols) * (n + gap));
        r.backdrop = Some(Arc::new(image::imageops::crop_imm(&out, x, y, n, n).to_image()));
        image::imageops::overlay(&mut out, &r.render(doc, look, n), x as i64, y as i64);
    }
    out.save(&args[1]).unwrap();
}
