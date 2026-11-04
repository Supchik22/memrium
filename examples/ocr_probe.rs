//! Read the text in a picture: `ocr_probe <picture>`. Prints each line with its box and confidence.
use memrium_de::{cutout, ocr};

fn main() {
    let path = std::env::args().nth(1).expect("usage: ocr_probe <picture>");
    cutout::preload();
    if !ocr::models_present() {
        eprintln!("fetching models into {}", ocr::model_dir().display());
        ocr::fetch_models().expect("models");
    }
    let t = std::time::Instant::now();
    let r = ocr::Reader::load(&ocr::model_dir()).expect("load");
    eprintln!("load {:?}", t.elapsed());
    let img = image::open(&path).expect("picture").to_rgba8();
    let t = std::time::Instant::now();
    let lines = r.read(img.as_raw(), img.width() as usize, img.height() as usize).expect("read");
    eprintln!("read {:?} ({}x{})", t.elapsed(), img.width(), img.height());
    for l in &lines {
        println!("{:>5.0},{:>5.0} {:>4.0}x{:<3.0} {:.2}  {}", l.rect[0], l.rect[1], l.rect[2], l.rect[3], l.score, l.text);
    }
}
