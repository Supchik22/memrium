//! Lift the objects under some points out of a picture: `cutout_probe <picture> <out dir> x,y [x,y ...]` (picture px).
//! Writes `<out dir>/<n>.png` for each point and prints how long the encoder and each cut took. The models are fetched
//! first when they are missing.
use memrium_de::cutout;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: cutout_probe <picture> <out dir> x,y [x,y ...]");
        std::process::exit(2);
    }
    if !cutout::models_present() {
        eprintln!("fetching models into {}", cutout::model_dir().display());
        cutout::fetch_models().expect("models");
    }
    let t = Instant::now();
    let seg = cutout::Segmenter::load(&cutout::model_dir()).expect("load");
    eprintln!("load {:?} ({})", t.elapsed(), seg.engine());
    let img = image::open(&a[1]).expect("picture").to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let t = Instant::now();
    let emb = seg.embed(img.as_raw(), w, h).expect("embed");
    eprintln!("encoder {:?} ({w}x{h})", t.elapsed());
    seg.warm(&emb);
    let out = std::path::Path::new(&a[2]);
    std::fs::create_dir_all(out).expect("out dir");
    for (n, p) in a[3..].iter().enumerate() {
        let (x, y) = p.split_once(',').expect("x,y");
        let at = [x.parse().unwrap(), y.parse().unwrap()];
        let t = Instant::now();
        match seg.cut(&emb, img.as_raw(), at).expect("decode") {
            Some(c) => {
                c.image.save(out.join(format!("{n}.png"))).expect("save");
                eprintln!("{n}: {at:?} -> {}x{} at {:?}, score {:.2}, {:?}", c.image.width(), c.image.height(), c.origin, c.score, t.elapsed());
            }
            None => eprintln!("{n}: {at:?} -> nothing, {:?}", t.elapsed()),
        }
    }
}
