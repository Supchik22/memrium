use memrium_de::{icondoc, iconrender, icondoc::Appearance};
fn main() {
    for d in icondoc::saved_docs() {
        let b = iconrender::background_for(&d, Appearance::Dark);
        println!("m={:.2} {:28} slot_set={} fill={:?} top={:.2?} glass={:?}", d.margin, d.name, d.background.is_set(Appearance::Dark), b.fill, b.top, (b.glass.specular, b.glass.enabled));
    }
}
