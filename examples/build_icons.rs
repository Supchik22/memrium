//! Writes the icon themes (every look) from all the saved documents, as the editor's "Build icon theme" does.
use memrium_de::{icondoc, iconrender};

fn main() {
    let docs = icondoc::saved_docs();
    let n = iconrender::build_theme(&docs, "Papirus", &iconrender::theme_dir(), &|| false, &|d, t| eprintln!("{d}/{t}")).unwrap();
    println!("{} documents, {n} pictures", docs.len());
}
