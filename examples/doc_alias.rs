//! Saves a copy of a saved icon document under the names programs ask for: `doc_alias <saved name> <name>...`.
use memrium_de::icondoc;

fn main() {
    let mut args = std::env::args().skip(1);
    let src = args.next().expect("the saved name to copy");
    let doc = icondoc::load(&icondoc::doc_path(&src)).expect("a saved document of that name");
    for name in args {
        let mut copy = doc.clone();
        copy.name = name.clone();
        icondoc::save(&icondoc::doc_path(&name), &copy).unwrap();
        println!("{src} -> {name}");
    }
}
