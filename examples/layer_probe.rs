//! Prints how much each layer of an Apple `.icon` draws: `layer_probe <x.icon>`.
use memrium_de::icondoc::{Appearance, Kind};
use memrium_de::iconrender::{effective_props, Renderer};

fn main() {
    let doc = memrium_de::appleicon::import(std::path::Path::new(&std::env::args().nth(1).unwrap())).unwrap();
    let mut r = Renderer::new();
    for l in doc.layers() {
        let node = doc.find(l.id).unwrap();
        let Kind::Layer(layer) = &node.kind else { continue };
        let p = effective_props(node, Appearance::Default);
        let img = r.raster(layer, &p, &doc, Appearance::Default, 128);
        let a: u64 = img.pixels().map(|p| p[3] as u64).sum();
        println!("{:12} svg {} bytes, alpha {a}, props visible={} opacity={} scale={} fill={:?}", node.name, layer.svg.len(), p.visible, p.opacity, p.scale, p.fill);
    }
}
