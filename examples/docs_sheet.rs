//! Draws the saved icon documents that replace a Papirus icon, on a wallpaper, in a grid: `docs_sheet <out.png> [look]`.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::{apps::IconIndex, wallpapers};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let look = match args.get(2).map(String::as_str) {
        Some("dark") => Appearance::Dark,
        Some("clear") => Appearance::ClearLight,
        Some("tinted") => Appearance::TintedLight,
        _ => Appearance::Default,
    };
    let index = IconIndex::build_pack("Papirus");
    let docs: Vec<_> = icondoc::saved_docs().into_iter().filter(|d| index.find(&d.name, 128).is_some()).collect();
    eprintln!("{} of the documents are Papirus icons", docs.len());
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap().1, 1600).expect("wallpaper");
    let (n, gap, cols) = (150u32, 18u32, 8u32);
    let rows = (docs.len() as u32 + cols - 1) / cols;
    let (w, h) = ((n + gap) * cols + gap, (n + gap) * rows + gap);
    let mut out = image::DynamicImage::ImageRgba8(wall).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (i, doc) in docs.iter().enumerate() {
        let (x, y) = (gap + (i as u32 % cols) * (n + gap), gap + (i as u32 / cols) * (n + gap));
        r.backdrop = Some(Arc::new(image::imageops::crop_imm(&out, x, y, n, n).to_image()));
        let img = r.render(doc, look, n);
        image::imageops::overlay(&mut out, &img, x as i64, y as i64);
        eprintln!("{:2} {}", i + 1, doc.name);
    }
    out.save(&args[1]).unwrap();
}
