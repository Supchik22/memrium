//! Icons without and with the phone's glass edge (`GlassTune::edge_glow`), side by side per look:
//! `edge_glow_sheet <out.png> [names...]`. Rows: icons; columns: Default 0 | Default edge | Dark 0 | Dark edge.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut names: Vec<String> = args[2..].to_vec();
    if names.is_empty() {
        names = ["discord", "telegram", "viber", "steam", "com.spotify.Client", "gvim", "accessories-calculator", "firefox"].map(String::from).to_vec();
    }
    let docs = icondoc::saved_docs();
    let (n, gap) = (256u32, 20u32);
    let cols = [(Appearance::Default, 0.0), (Appearance::Default, 1.0), (Appearance::Dark, 0.0), (Appearance::Dark, 1.0)];
    let found: Vec<_> = names.iter().filter_map(|name| docs.iter().find(|d| &d.name == name)).collect();
    let (w, h) = (gap + cols.len() as u32 * (n + gap), gap + found.len() as u32 * (n + gap));
    let mut out = image::RgbaImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgba([46, 52, 78, 255]) } else { image::Rgba([24, 24, 28, 255]) });
    let base = GlassTune::load();
    for (row, doc) in found.iter().enumerate() {
        for (col, (look, glow)) in cols.iter().enumerate() {
            let mut r = Renderer::new();
            r.tune = GlassTune { edge_glow: *glow, ..base };
            let img = r.render(doc, *look, n);
            image::imageops::overlay(&mut out, &img, (gap + col as u32 * (n + gap)) as i64, (gap + row as u32 * (n + gap)) as i64);
        }
    }
    out.save(&args[1]).unwrap();
}
