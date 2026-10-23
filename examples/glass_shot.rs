//! Draws an icon in a look over a wallpaper and writes the picture: `glass_shot <icon name> <out.png>`.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::wallpapers;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let docs = icondoc::saved_docs();
    let imported;
    let doc = if args[1].ends_with(".icon") {
        imported = memrium_de::appleicon::import(std::path::Path::new(&args[1])).expect("an Apple icon");
        &imported
    } else {
        docs.iter().find(|d| d.name == args[1]).expect("no such icon")
    };
    let walls = wallpapers::scan();
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap_or_else(|| walls[0].clone()).1, 1600).expect("wallpaper");
    let (n, gap) = (320u32, 24u32);
    let looks = [Appearance::Default, Appearance::Dark, Appearance::ClearLight, Appearance::TintedLight];
    let mut out = image::imageops::crop_imm(&wall, 0, wall.height() / 4, (n + gap) * 4 + gap, n + 2 * gap).to_image();
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (k, look) in looks.iter().enumerate() {
        let x = gap + k as u32 * (n + gap);
        let back = image::imageops::crop_imm(&out, x, gap, n, n).to_image();
        r.backdrop = Some(Arc::new(back));
        let img = r.render(doc, *look, n);
        image::imageops::overlay(&mut out, &img, x as i64, gap as i64);
    }
    out.save(&args[2]).unwrap();
}
