//! Prints the settings of a saved icon document: `doc_dump <icon name>`.
use memrium_de::icondoc::{self, Appearance, Kind, Node};
use memrium_de::iconrender::{background_for, effective_props};

fn show(n: &Node, depth: usize) {
    let p = effective_props(n, Appearance::Default);
    let kind = match &n.kind { Kind::Layer(l) => format!("layer svg {}b", l.svg.len()), Kind::Group(g) => format!("group of {}", g.children.len()) };
    println!("{}{} [{}] vis={} op={:.2} blend={:?} fill={:?} scale={:.2} off={:?} glass={:?} mode={:?}", "  ".repeat(depth), n.name, kind, p.visible, p.opacity, p.blend, p.fill, p.scale, p.offset, p.glass, p.glass_mode);
    for c in n.children() { show(c, depth + 1); }
}

fn main() {
    let name = std::env::args().nth(1).unwrap();
    let doc = icondoc::saved_docs().into_iter().find(|d| d.name == name).expect("no such document");
    println!("== {} margin={} tint={:?} bases={:?}", doc.name, doc.margin, doc.icon_tint, doc.bases);
    for look in [Appearance::Default, Appearance::Dark] {
        let b = background_for(&doc, look);
        println!("plate {:?}: {:?} {:?} top={:?} bottom={:?} glass={:?}", look, b.shape, b.fill, b.top, b.bottom, b.glass);
    }
    for n in &doc.nodes { show(n, 0); }
    if std::env::var("SVG").is_ok() { fn walk(n: &Node) { if let Kind::Layer(l) = &n.kind { println!("--- {}\n{}", n.name, l.svg); } for c in n.children() { walk(c); } } for n in &doc.nodes { walk(n); } }
}
