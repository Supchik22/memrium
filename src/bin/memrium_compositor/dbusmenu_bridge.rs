//! Menus of programs that know nothing of this desktop: the registrar `com.canonical.AppMenu.Registrar` (the one
//! Unity's global menu offered) that GTK's `appmenu-gtk-module` and Qt's appmenu platform theme register their windows
//! with. A window is registered by its X11 id, with the bus name and path of its `com.canonical.dbusmenu`; the shell
//! reads that menu (off the UI thread) when the window is in front and shows it in the menu bar. Only windows that run
//! under Xwayland have an X11 id, so Wayland-native GTK/Qt programs are not reached this way.

use crate::gtkmenu::{self, Actions, GtkSource};
use crate::tray_service::{node_of, Layout, MenuNode};
use chebrium::appmenu::{Item, Menu};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use zbus::{blocking::Connection, interface, zvariant::{ObjectPath, OwnedObjectPath, Value}};

const MENU: &str = "com.canonical.dbusmenu";
/// How long a read menu is trusted before it is read again (a program changes its menus while it runs).
const REFRESH: Duration = Duration::from_secs(2);

/// A window's menu: where to ask for it (a dbusmenu, or the `org.gtk.Menus` of a GTK program).
#[derive(Clone, PartialEq)]
enum Source {
    Dbusmenu(String, String),
    Gtk(GtkSource),
}

#[derive(Default)]
struct Cached {
    menus: Vec<Menu>,
    /// what the ids of a GTK menu do
    actions: Arc<Actions>,
    read: Option<Instant>,
    reading: bool,
}

#[derive(Default)]
struct Shared {
    windows: Mutex<HashMap<u64, Source>>,
    cache: Mutex<HashMap<u64, Cached>>,
    /// something was read that the bar has not drawn yet
    dirty: AtomicBool,
}

struct Registrar {
    shared: Arc<Shared>,
}

#[interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    fn register_window(&self, window_id: u32, menu_object_path: ObjectPath<'_>, #[zbus(header)] hdr: zbus::message::Header<'_>) {
        let Some(sender) = hdr.sender() else { return };
        if let Ok(mut w) = self.shared.windows.lock() {
            w.insert(window_id as u64, Source::Dbusmenu(sender.to_string(), menu_object_path.to_string()));
        }
        if let Ok(mut c) = self.shared.cache.lock() {
            c.remove(&(window_id as u64));
        }
        self.shared.dirty.store(true, Ordering::Relaxed);
    }

    fn unregister_window(&self, window_id: u32) {
        if let Ok(mut w) = self.shared.windows.lock() {
            w.remove(&(window_id as u64));
        }
        if let Ok(mut c) = self.shared.cache.lock() {
            c.remove(&(window_id as u64));
        }
        self.shared.dirty.store(true, Ordering::Relaxed);
    }

    fn get_menu_for_window(&self, window_id: u32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let found = self.shared.windows.lock().ok().and_then(|w| w.get(&(window_id as u64)).cloned());
        match found {
            Some(Source::Dbusmenu(service, path)) => OwnedObjectPath::try_from(path).map(|p| (service, p)).map_err(|e| zbus::fdo::Error::Failed(e.to_string())),
            _ => Err(zbus::fdo::Error::Failed("no menu for that window".into())),
        }
    }
}

pub struct Bridge {
    shared: Arc<Shared>,
    conn: Option<Connection>,
}

fn item_of(n: &MenuNode) -> Item {
    if n.separator {
        return Item::sep();
    }
    if !n.children.is_empty() {
        let mut parent = Item::parent(n.label.clone(), n.children.iter().map(item_of).collect()).enabled(n.enabled).icon(&n.icon_name);
        parent.icon_png = n.icon_data.clone();
        return parent;
    }
    let mut item = Item::new(n.id as u32, n.label.clone()).key(&n.shortcut).enabled(n.enabled).checked(n.checked).icon(&n.icon_name);
    item.icon_png = n.icon_data.clone();
    item
}

/// The bar's menus out of a dbusmenu layout: the children of the root are the menus, their children the entries.
fn menus_of_layout(root: MenuNode) -> Vec<Menu> {
    root.children.iter().filter(|m| !m.separator && !m.label.is_empty()).map(|m| Menu::new(m.label.clone(), m.children.iter().map(item_of).collect())).collect()
}

fn read_menu(conn: &Connection, source: &Source) -> Option<(Vec<Menu>, Arc<Actions>)> {
    let (service, path) = match source {
        Source::Dbusmenu(s, p) => (s, p),
        Source::Gtk(g) => return gtkmenu::read(conn, g).map(|(m, a)| (m, Arc::new(a))),
    };
    // some programs build a menu only when asked to show it
    let _ = conn.call_method(Some(service.as_str()), path.as_str(), Some(MENU), "AboutToShow", &(0i32,));
    let msg = conn.call_method(Some(service.as_str()), path.as_str(), Some(MENU), "GetLayout", &(0i32, -1i32, Vec::<String>::new())).ok()?;
    let (_, root): (u32, Layout) = msg.body().deserialize().ok()?;
    node_of(root).map(menus_of_layout).map(|m| (m, Arc::default()))
}

impl Bridge {
    pub fn new() -> Bridge {
        Bridge { shared: Arc::new(Shared::default()), conn: None }
    }

    /// Offer the registrar on the connection of the menu service. A desktop that already has one keeps it.
    pub fn start(&mut self, conn: &Connection) {
        let served = conn.object_server().at("/com/canonical/AppMenu/Registrar", Registrar { shared: self.shared.clone() });
        match served.map_err(|e| e.to_string()).and_then(|_| conn.request_name("com.canonical.AppMenu.Registrar").map_err(|e| e.to_string())) {
            Ok(()) => {
                eprintln!("appmenu: serving com.canonical.AppMenu.Registrar");
                self.conn = Some(conn.clone());
            }
            Err(e) => eprintln!("appmenu: no registrar ({e})"),
        }
    }

    /// A window whose program told the menu's address itself (not through the registrar): `key` names the window.
    pub fn set_source(&self, key: u64, service: String, path: String) {
        self.set_source_of(key, Source::Dbusmenu(service, path));
    }

    /// A GTK window under Wayland (`gtk_shell1`): `key` names the window.
    pub fn set_gtk_source(&self, key: u64, src: GtkSource) {
        self.set_source_of(key, Source::Gtk(src));
    }

    fn set_source_of(&self, key: u64, new: Source) {
        let changed = self.shared.windows.lock().map(|mut w| w.insert(key, new.clone()).as_ref() != Some(&new)).unwrap_or(false);
        if changed {
            if let Ok(mut c) = self.shared.cache.lock() {
                c.remove(&key);
            }
        }
    }

    /// The menus of an X11 window, as last read (empty until the first reading arrives); a stale one is read again.
    pub fn menus_of(&self, xid: u64) -> Vec<Menu> {
        let Some(conn) = &self.conn else { return Vec::new() };
        let Some(source) = self.shared.windows.lock().ok().and_then(|w| w.get(&xid).cloned()) else { return Vec::new() };
        let (menus, start) = {
            let Ok(mut cache) = self.shared.cache.lock() else { return Vec::new() };
            let c = cache.entry(xid).or_default();
            let stale = c.read.map_or(true, |t| t.elapsed() > REFRESH);
            let start = stale && !c.reading;
            c.reading |= start;
            (c.menus.clone(), start)
        };
        if start {
            let (conn, shared) = (conn.clone(), self.shared.clone());
            std::thread::Builder::new()
                .name("dbusmenu-read".into())
                .spawn(move || {
                    let read = read_menu(&conn, &source);
                    if let Ok(mut cache) = shared.cache.lock() {
                        if let Some(c) = cache.get_mut(&xid) {
                            c.reading = false;
                            c.read = Some(Instant::now());
                            if let Some((menus, actions)) = read {
                                c.actions = actions;
                                if menus != c.menus {
                                    c.menus = menus;
                                    shared.dirty.store(true, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                })
                .ok();
        }
        menus
    }

    /// Tell the program that an entry of a window's menu was chosen.
    pub fn event(&self, xid: u64, id: u32) {
        let Some(conn) = self.conn.clone() else { return };
        let Some(source) = self.shared.windows.lock().ok().and_then(|w| w.get(&xid).cloned()) else { return };
        // the next look at the menu reads it again: a choice often changes what is checked
        let mut actions = Arc::default();
        if let Ok(mut c) = self.shared.cache.lock() {
            if let Some(c) = c.get_mut(&xid) {
                c.read = None;
                actions = c.actions.clone();
            }
        }
        std::thread::spawn(move || {
            let (service, path) = match source {
                Source::Dbusmenu(s, p) => (s, p),
                Source::Gtk(g) => {
                    if let Some((action, target)) = actions.get(&id) {
                        gtkmenu::activate(&conn, &g, action, target.as_ref());
                    }
                    return;
                }
            };
            let _ = conn.call_method(Some(service.as_str()), path.as_str(), Some(MENU), "Event", &(id as i32, "clicked", Value::from(0i32), 0u32));
        });
    }

    /// True once after something new was read (the bar must be drawn again).
    pub fn take_dirty(&self) -> bool {
        self.shared.dirty.swap(false, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, label: &str, children: Vec<MenuNode>) -> MenuNode {
        MenuNode { id, label: label.into(), enabled: true, children, ..Default::default() }
    }

    #[test]
    fn root_children_are_the_bars_menus() {
        let root = node(0, "", vec![
            node(1, "File", vec![MenuNode { shortcut: "Ctrl+O".into(), ..node(2, "Open", vec![]) }, MenuNode { separator: true, ..Default::default() }, node(3, "Recent", vec![node(4, "a.txt", vec![])])]),
            node(5, "Edit", vec![MenuNode { id: 6, label: "Undo".into(), enabled: false, checked: true, ..Default::default() }]),
        ]);
        let menus = menus_of_layout(root);
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "Edit"]);
        assert_eq!(menus[0].items[0], Item::new(2, "Open").key("Ctrl+O"));
        assert!(menus[0].items[1].separator);
        assert_eq!(menus[0].items[2], Item::parent("Recent", vec![Item::new(4, "a.txt")]));
        assert_eq!(menus[1].items[0], Item::new(6, "Undo").enabled(false).checked(true));
    }
}
