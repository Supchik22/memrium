//! Developer tool: list installed applications and whether their icons resolve.

use memrium_de::apps;

fn main() {
    let t = std::time::Instant::now();
    let list = apps::scan();
    println!("{} applications in {:?}", list.len(), t.elapsed());
    let t = std::time::Instant::now();
    let icons = apps::IconIndex::build();
    println!("icon index built in {:?}", t.elapsed());
    let mut missing = 0;
    for a in &list {
        let found = icons.find(&a.icon, 128);
        if found.is_none() {
            missing += 1;
        }
        println!("{:<28} {:<24} icon={:<28} {}", a.id, a.name, a.icon, found.map(|p| p.display().to_string()).unwrap_or_else(|| "NOT FOUND".into()));
    }
    println!("{} of {} icons missing", missing, list.len());
}
