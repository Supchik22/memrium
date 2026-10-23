//! Which file the shell would take for an icon with a theme chosen: `icon_find <theme> <icon name>`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    memrium_de::apps::set_icon_theme(&a[1]);
    let index = memrium_de::apps::IconIndex::build();
    println!("{:?}", index.find(&a[2], 128));
}
