//! A reference picture next to a Papirus icon made in the style: `ref_compare <out.png> <ref.png> <icon name> [<ref.png> <icon name> ...]`.
use memrium_de::icondoc::Appearance;
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::{apps::IconIndex, iconstyle, svgedit};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let index = IconIndex::build_pack("Papirus");
    let n = 256u32;
    let pairs: Vec<(&String, &String)> = a[2..].chunks(2).filter(|c| c.len() == 2).map(|c| (&c[0], &c[1])).collect();
    let mut out = image::RgbaImage::from_pixel((n * 3 + 20) * pairs.len() as u32, n + 20, image::Rgba([40, 40, 52, 255]));
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (i, (refimg, name)) in pairs.iter().enumerate() {
        let x0 = i as u32 * (n * 3 + 20) + 10;
        if let Ok(img) = image::open(refimg) {
            let img = img.resize(n, n, image::imageops::FilterType::Lanczos3).to_rgba8();
            image::imageops::overlay(&mut out, &img, x0 as i64, 10);
        }
        let path = index.find(name, 128).expect("icon");
        let svg = std::fs::read_to_string(&path).unwrap();
        if let Some(img) = svgedit::rasterize(&svg, n) {
            image::imageops::overlay(&mut out, &img, (x0 + n) as i64, 10);
        }
        if let Some(doc) = iconstyle::favourite_style(name, &svg) {
            image::imageops::overlay(&mut out, &r.render(&doc, Appearance::Default, n), (x0 + 2 * n) as i64, 10);
        }
    }
    out.save(&a[1]).unwrap();
}
