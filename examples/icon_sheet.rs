//! Draws every Apple `.icon` folder of a directory in four looks over a wallpaper, one row each:
//! `icon_sheet <dir with .icon folders> <out.png>`.
use memrium_de::icondoc::Appearance;
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::{appleicon, wallpapers};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let icons = appleicon::find_icons(&[std::path::PathBuf::from(&args[1])]);
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap_or_else(|| wallpapers::scan().remove(0)).1, 1600).expect("wallpaper");
    let (n, gap) = (256u32, 20u32);
    let looks = [Appearance::Default, Appearance::Dark, Appearance::ClearLight, Appearance::TintedLight];
    let (w, h) = ((n + gap) * looks.len() as u32 + gap, (n + gap) * icons.len() as u32 + gap);
    let mut out = image::DynamicImage::ImageRgba8(wall).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (row, dir) in icons.iter().enumerate() {
        let doc = match appleicon::import(dir) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                continue;
            }
        };
        for (k, look) in looks.iter().enumerate() {
            let (x, y) = (gap + k as u32 * (n + gap), gap + row as u32 * (n + gap));
            r.backdrop = Some(Arc::new(image::imageops::crop_imm(&out, x, y, n, n).to_image()));
            let img = r.render(&doc, *look, n);
            image::imageops::overlay(&mut out, &img, x as i64, y as i64);
        }
        eprintln!("{}: {} layers", doc.name, doc.layers().len());
    }
    out.save(&args[2]).unwrap();
}
