//! The system tray's D-Bus side: the `org.kde.StatusNotifierWatcher` that applications register their icons with, the
//! reading of each icon (name, picture, tooltip, menu), and what a click does to it (activate, its own menu, the menu it
//! describes over `com.canonical.dbusmenu`). Everything that waits for an application runs on its own thread, so a
//! frozen program never freezes the shell; the shell only sees events.

use memrium_de::tray::{best_pixmap, item_id, split_id, TrayIcon, TrayStatus};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex,
};
use std::time::Duration;
use zbus::{
    blocking::{connection::Builder, Connection},
    interface,
    zvariant::{OwnedValue, Value},
};

const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// One entry of an application's menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub checked: bool,
    /// the first key combination of the entry, as text ("Ctrl+O"); empty when it has none
    pub shortcut: String,
    /// the entry's icon: a theme name, or a picture (PNG) the program sent itself
    pub icon_name: String,
    pub icon_data: Vec<u8>,
    pub children: Vec<MenuNode>,
}

pub enum TrayEv {
    Add(String),
    Remove(String),
    Update(TrayIcon),
    /// an item's menu, fetched for a click at `at`
    Menu { id: String, at: [f32; 2], nodes: Vec<MenuNode> },
}

struct Watcher {
    tx: Sender<TrayEv>,
    items: Arc<Mutex<Vec<String>>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String, #[zbus(header)] hdr: zbus::message::Header<'_>) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let id = item_id(&service, &sender);
        if let Ok(mut v) = self.items.lock() {
            if v.contains(&id) {
                return;
            }
            v.push(id.clone());
        }
        let _ = self.tx.send(TrayEv::Add(id));
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().map(|v| v.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

pub struct TrayService {
    conn: Option<Connection>,
    rx: Option<Receiver<TrayEv>>,
    tx: Option<Sender<TrayEv>>,
    items: Arc<Mutex<Vec<String>>>,
    /// the reading thread of each item, to be told when it is gone
    alive: HashMap<String, Arc<AtomicBool>>,
    /// how to wake the reading thread of an item when it signals a change (by its unique bus name and path)
    kicks: Kicks,
}

/// The reading threads waiting for their item to change, by (unique bus name, object path).
type Kicks = Arc<Mutex<HashMap<(String, String), Sender<()>>>>;

impl TrayService {
    pub fn new() -> TrayService {
        TrayService { conn: None, rx: None, tx: None, items: Arc::new(Mutex::new(Vec::new())), alive: HashMap::new(), kicks: Arc::default() }
    }

    /// Offer the watcher on the session bus. A desktop that already has one keeps it (and we have no tray).
    pub fn start(&mut self) {
        let (tx, rx) = channel();
        let watcher = Watcher { tx: tx.clone(), items: self.items.clone() };
        let built = Builder::session().map(|b| b.allow_name_replacements(false).replace_existing_names(false)).and_then(|b| b.name("org.kde.StatusNotifierWatcher")).and_then(|b| b.serve_at("/StatusNotifierWatcher", watcher)).and_then(|b| b.build());
        match built {
            Ok(conn) => {
                eprintln!("tray: serving org.kde.StatusNotifierWatcher");
                // the bus tells when an application goes away
                let (c2, t2, items) = (conn.clone(), tx.clone(), self.items.clone());
                std::thread::Builder::new().name("tray-names".into()).spawn(move || watch_names(c2, t2, items)).ok();
                // the items say when they change (a new icon, status, tooltip...): they are read again then
                let (c3, kicks) = (conn.clone(), self.kicks.clone());
                std::thread::Builder::new().name("tray-signals".into()).spawn(move || watch_signals(c3, kicks)).ok();
                self.conn = Some(conn);
                self.rx = Some(rx);
                self.tx = Some(tx);
            }
            Err(e) => eprintln!("tray: not serving ({e})"),
        }
    }

    pub fn active(&self) -> bool {
        self.conn.is_some()
    }

    /// What happened since the last look. New items start being read here.
    pub fn poll(&mut self) -> Vec<TrayEv> {
        let mut out = Vec::new();
        let (Some(rx), Some(conn), Some(tx)) = (&self.rx, &self.conn, &self.tx) else { return out };
        while let Ok(ev) = rx.try_recv() {
            match &ev {
                TrayEv::Add(id) => {
                    let alive = Arc::new(AtomicBool::new(true));
                    self.alive.insert(id.clone(), alive.clone());
                    let (c, t, id, kicks) = (conn.clone(), tx.clone(), id.clone(), self.kicks.clone());
                    std::thread::Builder::new().name("tray-item".into()).spawn(move || read_item(c, id, t, alive, kicks)).ok();
                }
                TrayEv::Remove(id) => {
                    if let Some(a) = self.alive.remove(id) {
                        a.store(false, Ordering::Relaxed);
                    }
                    if let Ok(mut v) = self.items.lock() {
                        v.retain(|i| i != id);
                    }
                }
                _ => {}
            }
            out.push(ev);
        }
        out
    }

    /// A call to the item (`Activate`, `ContextMenu`, `SecondaryAction`) with the pointer's place, off the UI thread.
    pub fn call(&self, id: &str, method: &'static str, at: [f32; 2]) {
        let Some(conn) = self.conn.clone() else { return };
        let id = id.to_string();
        std::thread::spawn(move || {
            let (bus, path) = split_id(&id);
            let _ = conn.call_method(Some(bus), path, Some(ITEM), method, &(at[0] as i32, at[1] as i32));
        });
    }

    pub fn scroll(&self, id: &str, delta: i32) {
        let Some(conn) = self.conn.clone() else { return };
        let id = id.to_string();
        std::thread::spawn(move || {
            let (bus, path) = split_id(&id);
            let _ = conn.call_method(Some(bus), path, Some(ITEM), "Scroll", &(delta, "vertical"));
        });
    }

    /// Fetch the menu of an item (it is answered as a `TrayEv::Menu`).
    pub fn fetch_menu(&self, id: &str, menu: &str, at: [f32; 2]) {
        let (Some(conn), Some(tx)) = (self.conn.clone(), self.tx.clone()) else { return };
        let (id, menu) = (id.to_string(), menu.to_string());
        std::thread::spawn(move || {
            let (bus, _) = split_id(&id);
            // some applications build their menu only when asked to show it
            let _ = conn.call_method(Some(bus), menu.as_str(), Some(MENU), "AboutToShow", &(0i32,));
            let reply = conn.call_method(Some(bus), menu.as_str(), Some(MENU), "GetLayout", &(0i32, -1i32, Vec::<String>::new()));
            let Ok(msg) = reply else { return };
            let Ok((_, root)): Result<(u32, Layout), _> = msg.body().deserialize() else { return };
            if let Some(root) = node_of(root) {
                let _ = tx.send(TrayEv::Menu { id, at, nodes: root.children });
            }
        });
    }

    /// Tell the item that menu entry `entry` was clicked.
    pub fn menu_click(&self, id: &str, menu: &str, entry: i32) {
        let Some(conn) = self.conn.clone() else { return };
        let (id, menu) = (id.to_string(), menu.to_string());
        std::thread::spawn(move || {
            let (bus, _) = split_id(&id);
            let _ = conn.call_method(Some(bus), menu.as_str(), Some(MENU), "Event", &(entry, "clicked", Value::from(0i32), 0u32));
        });
    }
}

/// Items whose application has left the bus are removed.
fn watch_names(conn: Connection, tx: Sender<TrayEv>, items: Arc<Mutex<Vec<String>>>) {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
    let Ok(changes) = proxy.receive_name_owner_changed() else { return };
    for ch in changes {
        let Ok(args) = ch.args() else { continue };
        if args.new_owner().is_some() {
            continue;
        }
        let gone = args.name().to_string();
        let ids: Vec<String> = items.lock().map(|v| v.iter().filter(|i| split_id(i).0 == gone).cloned().collect()).unwrap_or_default();
        for id in ids {
            let _ = tx.send(TrayEv::Remove(id));
        }
    }
}

fn string_of(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

fn pixmaps_of(v: Option<&OwnedValue>) -> Vec<(i32, i32, Vec<u8>)> {
    v.and_then(|v| Vec::<(i32, i32, Vec<u8>)>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

fn read_once(conn: &Connection, id: &str) -> Option<TrayIcon> {
    let (bus, path) = split_id(id);
    let props = zbus::blocking::fdo::PropertiesProxy::builder(conn).destination(bus).ok()?.path(path).ok()?.build().ok()?;
    let all = props.get_all(zbus::names::InterfaceName::try_from(ITEM).ok()?).ok()?;
    let tip = all.get("ToolTip").and_then(|v| <(String, Vec<(i32, i32, Vec<u8>)>, String, String)>::try_from(v.try_clone().ok()?).ok());
    let mut icon = TrayIcon {
        id: id.to_string(),
        title: string_of(&all, "Title"),
        tooltip: tip.map(|t| t.2).unwrap_or_default(),
        status: TrayStatus::parse(&string_of(&all, "Status")),
        icon_name: string_of(&all, "IconName"),
        icon_dir: string_of(&all, "IconThemePath"),
        pixmap: best_pixmap(pixmaps_of(all.get("IconPixmap"))),
        menu: all.get("Menu").and_then(|v| zbus::zvariant::OwnedObjectPath::try_from(v.try_clone().ok()?).ok()).map(|p| p.to_string()).filter(|p| p != "/").unwrap_or_default(),
        is_menu: all.get("ItemIsMenu").and_then(|v| bool::try_from(v.try_clone().ok()?).ok()).unwrap_or(false),
        cell: None,
        inverse: None,
    };
    if icon.title.is_empty() {
        icon.title = string_of(&all, "Id");
    }
    // an item that needs attention shows its attention icon
    if icon.status == TrayStatus::Attention {
        let name = string_of(&all, "AttentionIconName");
        if !name.is_empty() {
            icon.icon_name = name;
            icon.pixmap = best_pixmap(pixmaps_of(all.get("AttentionIconPixmap"))).or(icon.pixmap);
        }
    }
    Some(icon)
}

/// Wake the reading thread of an item whenever the item sends one of its signals (NewIcon, NewStatus, NewToolTip...).
fn watch_signals(conn: Connection, kicks: Kicks) {
    let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(ITEM).map(|b| b.build()) else { return };
    let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
    for msg in signals.flatten() {
        let h = msg.header();
        let (Some(sender), Some(path)) = (h.sender(), h.path()) else { continue };
        if let Ok(k) = kicks.lock() {
            if let Some(tx) = k.get(&(sender.to_string(), path.to_string())) {
                let _ = tx.send(());
            }
        }
    }
}

/// Read an item, then again whenever it signals a change, sending what changed; stop when it is gone (or never answered).
/// An item that never signals is still looked at now and then. (It used to be read every 1.5 s: that woke the shell and
/// every program in the tray all the time, and read their pictures again and again.)
fn read_item(conn: Connection, id: String, tx: Sender<TrayEv>, alive: Arc<AtomicBool>, kicks: Kicks) {
    // the signals come from the unique name of the program, whatever name it registered with
    let (bus, path) = split_id(&id);
    let unique = if bus.starts_with(':') {
        Some(bus.to_string())
    } else {
        zbus::blocking::fdo::DBusProxy::new(&conn).ok().and_then(|p| p.get_name_owner(zbus::names::BusName::try_from(bus).ok()?).ok()).map(|n| n.to_string())
    };
    let (kick_tx, kick_rx) = channel::<()>();
    let key = unique.map(|u| (u, path.to_string()));
    if let (Some(key), Ok(mut k)) = (&key, kicks.lock()) {
        k.insert(key.clone(), kick_tx);
    }
    // with signals heard, the look now and then only catches an item that changes without saying so
    let every = if key.is_some() { Duration::from_secs(20) } else { Duration::from_millis(1500) };
    let (mut last, mut failures) = (None::<TrayIcon>, 0);
    while alive.load(Ordering::Relaxed) {
        match read_once(&conn, &id) {
            Some(icon) => {
                failures = 0;
                if last.as_ref() != Some(&icon) {
                    last = Some(icon.clone());
                    let _ = tx.send(TrayEv::Update(icon));
                }
            }
            None => {
                failures += 1;
                if failures >= 4 {
                    let _ = tx.send(TrayEv::Remove(id.clone()));
                    break;
                }
            }
        }
        // a failed read is tried again soon; otherwise wait for a signal (a burst of them is read once)
        let wait = if failures > 0 { Duration::from_millis(1500) } else { every };
        if kick_rx.recv_timeout(wait).is_ok() {
            std::thread::sleep(Duration::from_millis(40));
            while kick_rx.try_recv().is_ok() {}
        }
    }
    if let (Some(key), Ok(mut k)) = (&key, kicks.lock()) {
        k.remove(key);
    }
}

/// `(id, {properties}, [children])` of a `dbusmenu` layout; the children are variants holding the same.
pub(crate) type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

pub(crate) fn node_of((id, props, children): Layout) -> Option<MenuNode> {
    let get_s = |k: &str| props.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
    let get_b = |k: &str, d: bool| props.get(k).and_then(|v| bool::try_from(v.try_clone().ok()?).ok()).unwrap_or(d);
    if !get_b("visible", true) {
        return None;
    }
    Some(MenuNode {
        id,
        label: memrium_de::tray::clean_label(&get_s("label").unwrap_or_default()),
        enabled: get_b("enabled", true),
        separator: get_s("type").as_deref() == Some("separator"),
        shortcut: props.get("shortcut").and_then(|v| Vec::<Vec<String>>::try_from(v.try_clone().ok()?).ok()).map(|s| shortcut_text(&s)).unwrap_or_default(),
        icon_name: get_s("icon-name").unwrap_or_default(),
        icon_data: props.get("icon-data").and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default(),
        checked: props.get("toggle-state").and_then(|v| i32::try_from(v.try_clone().ok()?).ok()) == Some(1),
        children: children.into_iter().filter_map(|c| Layout::try_from(c).ok().and_then(node_of)).collect(),
    })
}

/// A `dbusmenu` shortcut (`[["Control", "Shift", "O"]]`, the alternatives of an entry) as the first one's text.
pub(crate) fn shortcut_text(combos: &[Vec<String>]) -> String {
    let Some(first) = combos.first() else { return String::new() };
    first
        .iter()
        .map(|k| match k.as_str() {
            "Control" => "Ctrl".to_string(),
            "Meta" => "Super".to_string(),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => k.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::shortcut_text;

    #[test]
    fn shortcuts_read_like_the_shells_own() {
        let combo = |k: &[&str]| vec![k.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
        assert_eq!(shortcut_text(&combo(&["Control", "o"])), "Ctrl+O");
        assert_eq!(shortcut_text(&combo(&["Control", "Shift", "S"])), "Ctrl+Shift+S");
        assert_eq!(shortcut_text(&combo(&["Alt", "F4"])), "Alt+F4");
        assert_eq!(shortcut_text(&combo(&["Meta", "Return"])), "Super+Return");
        assert_eq!(shortcut_text(&[]), "");
    }

    /// A tray item on the session bus whose icon name can be changed.
    struct FakeItem {
        icon: std::sync::Arc<std::sync::Mutex<String>>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl FakeItem {
        #[zbus(property)]
        fn icon_name(&self) -> String {
            self.icon.lock().unwrap().clone()
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "Active".into()
        }
        #[zbus(signal)]
        async fn new_icon(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    }

    #[test]
    fn an_item_is_read_again_when_it_signals_a_change_not_by_polling() {
        use super::*;
        // (needs a session bus; skipped without one)
        let Ok(bus) = zbus::blocking::Connection::session() else { return };
        let icon = Arc::new(Mutex::new("first".to_string()));
        bus.object_server().at("/StatusNotifierItem", FakeItem { icon: icon.clone() }).unwrap();
        let id = format!("{}/StatusNotifierItem", bus.unique_name().unwrap());
        let shell = zbus::blocking::Connection::session().unwrap();
        let (tx, rx) = channel();
        let (alive, kicks): (_, Kicks) = (Arc::new(AtomicBool::new(true)), Arc::default());
        let (c, k) = (shell.clone(), kicks.clone());
        std::thread::spawn(move || watch_signals(c, k));
        let (c, k, a, i) = (shell.clone(), kicks.clone(), alive.clone(), id.clone());
        std::thread::spawn(move || read_item(c, i, tx, a, k));
        let name = |ev: TrayEv| match ev {
            TrayEv::Update(t) => t.icon_name,
            _ => String::new(),
        };
        assert_eq!(name(rx.recv_timeout(Duration::from_secs(5)).unwrap()), "first");
        // wait until the reader listens, then change the icon and say so
        let started = std::time::Instant::now();
        while kicks.lock().unwrap().is_empty() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        *icon.lock().unwrap() = "second".into();
        let at = std::time::Instant::now();
        let iface = bus.object_server().interface::<_, FakeItem>("/StatusNotifierItem").unwrap();
        zbus::block_on(FakeItem::new_icon(iface.signal_emitter())).unwrap();
        assert_eq!(name(rx.recv_timeout(Duration::from_secs(5)).unwrap()), "second");
        assert!(at.elapsed() < Duration::from_secs(2), "read on the signal, not on the slow look: {:?}", at.elapsed());
        alive.store(false, Ordering::Relaxed);
    }
}
