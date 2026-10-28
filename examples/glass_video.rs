//! Writes the frames of a short film of the glass: `glass_video <icon name> <out dir>`. The icon drifts over the
//! wallpaper (so the glass bends what is behind it), the light turns round, and the look changes now and then.
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
    std::fs::create_dir_all(&args[2]).unwrap();
    let walls = wallpapers::scan();
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap_or_else(|| walls[0].clone()).1, 1600).expect("wallpaper");
    let (w, h, n, frames) = (1280u32, 720u32, 360u32, 240u32);
    let wall = image::imageops::crop_imm(&wall, 0, (wall.height() - h) / 2, w, h).to_image();
    let looks = [Appearance::Default, Appearance::Dark, Appearance::ClearLight, Appearance::TintedLight];
    let mut r = Renderer::new();
    let tune = GlassTune::load();
    for f in 0..frames {
        let t = f as f32 / frames as f32;
        let a = t * std::f32::consts::TAU;
        let cx = w as f32 / 2.0 + a.sin() * 330.0;
        let cy = h as f32 / 2.0 + (a * 2.0).sin() * 90.0;
        let (x, y) = ((cx - n as f32 / 2.0) as u32, (cy - n as f32 / 2.0) as u32);
        r.tune = GlassTune { light: 231.0 + t * 360.0, ..tune };
        let mut frame = wall.clone();
        let back = image::imageops::crop_imm(&frame, x, y, n, n).to_image();
        r.backdrop = Some(Arc::new(back));
        let img = r.render(doc, looks[(f * 4 / frames) as usize], n);
        image::imageops::overlay(&mut frame, &img, x as i64, y as i64);
        frame.save(format!("{}/{:04}.png", args[2], f)).unwrap();
    }
}
