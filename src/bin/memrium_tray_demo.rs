//! A stand-in for an application with a tray icon, for trying the tray: registers a StatusNotifierItem with a picture
//! of its own and a two-entry `dbusmenu`, and prints what it is asked to do.
//! `memrium_tray_demo [name]` (the name is its title; run several to see several icons).

use std::collections::HashMap;
use zbus::{
    blocking::connection::Builder,
    interface,
    zvariant::{ObjectPath, OwnedValue, Type, Value},
};

struct Item {
    title: String,
    colour: [u8; 3],
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn id(&self) -> String {
        self.title.clone()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.title.clone()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        // a round blob in the item's colour, ARGB in network order
        let n = 32;
        let mut d = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
                let a = if dx * dx + dy * dy < 14.0 * 14.0 { 255 } else { 0 };
                d.extend_from_slice(&[a, self.colour[0], self.colour[1], self.colour[2]]);
            }
        }
        vec![(n, n, d)]
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked("/Menu")
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    fn activate(&self, x: i32, y: i32) {
        eprintln!("{}: Activate({x}, {y})", self.title);
    }
    fn context_menu(&self, x: i32, y: i32) {
        eprintln!("{}: ContextMenu({x}, {y})", self.title);
    }
    fn secondary_action(&self, x: i32, y: i32) {
        eprintln!("{}: SecondaryAction({x}, {y})", self.title);
    }
    fn scroll(&self, delta: i32, orientation: String) {
        eprintln!("{}: Scroll({delta}, {orientation})", self.title);
    }
}

#[derive(Type, serde::Serialize)]
struct Layout {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<Value<'static>>,
}

fn entry(id: i32, label: &str) -> Layout {
    let mut props = HashMap::new();
    props.insert("label".to_string(), OwnedValue::try_from(Value::from(label.to_string())).unwrap());
    Layout { id, props, children: Vec::new() }
}

struct Menu {
    title: String,
    /// serve a File menu holding the entries (a program's menu bar) instead of a flat list (a tray menu)
    bar: bool,
}

fn node(e: Layout) -> Value<'static> {
    let dict: HashMap<String, OwnedValue> = e.props.iter().map(|(k, v)| (k.clone(), v.try_clone().unwrap())).collect();
    Value::Structure(zbus::zvariant::StructureBuilder::new().append_field(Value::from(e.id)).append_field(Value::Dict(zbus::zvariant::Dict::from(dict))).append_field(Value::Array(zbus::zvariant::Array::from(e.children))).build().unwrap())
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn get_layout(&self, _parent: i32, _depth: i32, _props: Vec<String>) -> (u32, Layout) {
        let mut root = entry(0, "");
        let mut list = Vec::new();
        for (id, label) in [(1, "_Open"), (2, "Say hello"), (3, "_Quit")] {
            list.push(node(entry(id, label)));
        }
        if self.bar {
            let mut file = entry(10, "_File");
            file.children = list;
            root.children.push(node(file));
        } else {
            root.children = list;
        }
        (1, root)
    }
    fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        eprintln!("{}: menu entry {id} {event_id}", self.title);
    }
}

fn main() {
    let title = std::env::args().nth(1).unwrap_or_else(|| "Demo".into());
    // `memrium_tray_demo <name> --appmenu <x11 window id>`: stand in for a GTK/Qt program with a global menu
    let appmenu: Option<u32> = std::env::args().skip_while(|a| a != "--appmenu").nth(1).and_then(|s| s.parse().ok());
    let colour = match title.len() % 3 {
        0 => [230, 70, 70],
        1 => [70, 190, 90],
        _ => [80, 120, 240],
    };
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let conn = Builder::session().unwrap().name(name.clone()).unwrap().serve_at("/StatusNotifierItem", Item { title: title.clone(), colour }).unwrap().serve_at("/Menu", Menu { title: title.clone(), bar: appmenu.is_some() }).unwrap().build().unwrap();
    if let Some(xid) = appmenu {
        for _ in 0..40 {
            let r = conn.call_method(Some("com.canonical.AppMenu.Registrar"), "/com/canonical/AppMenu/Registrar", Some("com.canonical.AppMenu.Registrar"), "RegisterWindow", &(xid, ObjectPath::from_static_str_unchecked("/Menu")));
            if r.is_ok() {
                eprintln!("{title}: registered window {xid}");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    // the watcher may not be there yet: ask again until it is
    for _ in 0..40 {
        let r = conn.call_method(Some("org.kde.StatusNotifierWatcher"), "/StatusNotifierWatcher", Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(name.as_str(),));
        if r.is_ok() {
            eprintln!("{title}: registered as {name}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
