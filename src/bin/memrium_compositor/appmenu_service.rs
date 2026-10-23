//! The menu bar's D-Bus side: `io.github.supchik22.Memrium.AppMenu`. Programs (`chebrium::appmenu::Client`) hand over their
//! menus with `Set`; the shell keeps the latest per `app_id`, shows those of the window in front, and tells the program
//! which entry was chosen with the `Activated` signal.

use chebrium::appmenu::{unflatten, Menu, Row, BUS, IFACE, PATH};
use memrium_de::menu::IconSrc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::{
    blocking::{connection::Builder, Connection},
    interface,
};

type Menus = Arc<Mutex<HashMap<String, Vec<Menu>>>>;

struct Service {
    menus: Menus,
}

#[interface(name = "io.github.supchik22.Memrium.AppMenu")]
impl Service {
    fn set(&self, app_id: String, rows: Vec<Row>) {
        if let Ok(mut m) = self.menus.lock() {
            m.insert(app_id, unflatten(&rows));
        }
    }

    fn clear(&self, app_id: String) {
        if let Ok(mut m) = self.menus.lock() {
            m.remove(&app_id);
        }
    }
}

pub struct AppMenuService {
    conn: Option<Connection>,
    menus: Menus,
    /// menus of programs that register with the Unity-style registrar instead
    pub bridge: crate::dbusmenu_bridge::Bridge,
}

/// Whose menu is open: a program of this desktop (by app id) or a window with a dbusmenu of its own.
#[derive(Clone, Debug)]
pub enum MenuTarget {
    App(String),
    /// a dbusmenu read through the bridge; the key names the window (an X11 id, or `WAYLAND_KEY` plus a window id)
    Dbus(u64),
}

impl AppMenuService {
    pub fn new() -> AppMenuService {
        AppMenuService { conn: None, menus: Arc::new(Mutex::new(HashMap::new())), bridge: crate::dbusmenu_bridge::Bridge::new() }
    }

    /// Offer the service on the session bus (programs then see that a menu bar is there).
    pub fn start(&mut self) {
        let built = Builder::session().and_then(|b| b.name(BUS)).and_then(|b| b.serve_at(PATH, Service { menus: self.menus.clone() })).and_then(|b| b.build());
        match built {
            Ok(conn) => {
                eprintln!("appmenu: serving {BUS}");
                self.bridge.start(&conn);
                self.conn = Some(conn);
            }
            Err(e) => eprintln!("appmenu: not serving ({e})"),
        }
    }

    /// The menus a program gave for its windows.
    pub fn menus_of(&self, app_id: &str) -> Vec<Menu> {
        self.menus.lock().ok().and_then(|m| m.get(app_id).cloned()).unwrap_or_default()
    }

    /// Tell the program an entry was chosen.
    pub fn activate(&self, app_id: &str, id: u32) {
        if let Some(conn) = &self.conn {
            if let Err(e) = conn.emit_signal(None::<&str>, PATH, IFACE, "Activated", &(app_id, id)) {
                eprintln!("appmenu: could not send the choice: {e}");
            }
        }
    }
}

/// Entries of a program get ids above every id of the shell's own menus.
pub const APP_BASE: u32 = 1 << 30;

/// Keys of Wayland windows in the bridge sit above every X11 id (those are 29 bits).
const WAYLAND_KEY: u64 = 1 << 40;

fn to_item(i: &chebrium::appmenu::Item) -> memrium_de::menu::MenuItem {
    use memrium_de::menu::{IconSrc, MenuItem};
    if i.separator {
        return MenuItem::sep();
    }
    let mut m = MenuItem::new(if i.sub.is_empty() { APP_BASE + i.id } else { 0 }, i.label.clone()).enabled(i.enabled).checked(i.checked);
    if !i.key.is_empty() {
        m = m.key(&i.key);
    }
    if !i.sub.is_empty() {
        m = m.sub(i.sub.iter().map(to_item).collect());
    }
    // the icon is looked up when the menu opens (see `resolve_menu_icons`)
    m.icon_src = if !i.icon_png.is_empty() {
        Some(IconSrc::Png(i.icon_png.clone()))
    } else if !i.icon.is_empty() {
        Some(IconSrc::Name(i.icon.clone()))
    } else {
        None
    };
    m
}

impl crate::state::State {
    /// The window in front when it is a program (not one of the shell's own windows).
    fn front_window(&self) -> Option<&crate::state::Win> {
        if self.focused_shell().is_some() {
            return None;
        }
        self.wins.iter().rev().find(|w| w.mapped && !w.minimized && !w.or)
    }

    /// The id of the window in front when it is a program.
    pub fn focused_app_id(&self) -> Option<String> {
        self.front_window().map(|w| w.app_id.clone())
    }

    /// Where the menus of the window in front come from: its own (by app id) when it gave some, else the dbusmenu of an
    /// X11 window.
    pub fn menu_target(&self) -> Option<MenuTarget> {
        self.menu_target_of(self.front_window()?)
    }

    /// Where the menus of window `w` come from (see `menu_target`).
    pub fn menu_target_of(&self, w: &crate::state::Win) -> Option<MenuTarget> {
        if !self.appmenu.menus_of(&w.app_id).is_empty() {
            return Some(MenuTarget::App(w.app_id.clone()));
        }
        if let Some(x) = w.role.x11() {
            return Some(MenuTarget::Dbus(x.window_id() as u64));
        }
        let surface = w.role.surface()?;
        let key = WAYLAND_KEY | w.id;
        if let Some((service, path)) = self.menu_address(&surface) {
            self.appmenu.bridge.set_source(key, service, path);
            return Some(MenuTarget::Dbus(key));
        }
        let g = self.gtk_address(&surface)?;
        self.appmenu.bridge.set_gtk_source(key, crate::gtkmenu::GtkSource { service: g.service, menubar: g.menubar, window: g.window, app: g.app });
        Some(MenuTarget::Dbus(key))
    }

    /// The menus the window in front gave for the bar.
    pub fn app_menus(&self) -> Vec<Menu> {
        self.menus_at(self.menu_target())
    }

    /// The menus found at a target.
    pub fn menus_at(&self, target: Option<MenuTarget>) -> Vec<Menu> {
        match target {
            Some(MenuTarget::App(id)) => self.appmenu.menus_of(&id),
            Some(MenuTarget::Dbus(xid)) => self.appmenu.bridge.menus_of(xid),
            None => Vec::new(),
        }
    }

    /// The entries of the program's menu called `title`, as the shell's own menu draws them (icons not looked up yet).
    pub fn app_menu_items(&self, title: &str) -> Option<Vec<memrium_de::menu::MenuItem>> {
        self.menu_items_at(self.menu_target(), title)
    }

    /// The entries of the menu called `title` found at a target.
    pub fn menu_items_at(&self, target: Option<MenuTarget>, title: &str) -> Option<Vec<memrium_de::menu::MenuItem>> {
        self.menus_at(target).into_iter().find(|m| m.title == title).map(|m| m.items.iter().map(to_item).collect())
    }

    /// Open the search over the commands of the front program's menus: every entry that does something, with the menus it
    /// is in ("File › Save As…"), its shortcut and its icon. Enter (or a click) sends the choice to the program as if the
    /// entry had been picked in its menu.
    pub fn menu_search(&mut self) {
        use memrium_de::figlet::MenuCmd;
        use memrium_de::menu::MenuItem;
        let shell = self.focused_shell();
        let target = self.menu_target();
        let name = match shell {
            Some(crate::state::Sw::Files) => Some("Files".to_string()),
            Some(crate::state::Sw::Settings) => Some("Settings".to_string()),
            None => self.front_window().map(|w| w.title.clone()).filter(|t| !t.is_empty()),
        };
        let mut menus: Vec<(String, Vec<MenuItem>)> = if shell.is_some() {
            // a window of the shell itself: the menus of the bar are its menus (the same entries, the same commands)
            self.bar_menus().into_iter().filter_map(|t| self.bar_menu_items(&t, None).map(|items| (t, items))).filter(|(t, _)| t != "Window").collect()
        } else {
            self.menus_at(target.clone()).iter().map(|m| (m.title.clone(), m.items.iter().map(to_item).collect())).collect()
        };
        if menus.is_empty() {
            // nothing to search: say so rather than show an empty box
            self.shot_notice = Some((memrium_de::i18n::tr("No menus to search in this program").into_owned(), 1.8));
            return;
        }
        for (_, items) in menus.iter_mut() {
            self.resolve_menu_icons(items);
        }
        fn walk(items: &[MenuItem], path: &str, out: &mut Vec<MenuCmd>) {
            for it in items {
                if it.separator {
                    continue;
                }
                if it.sub.is_empty() {
                    if it.enabled && !it.label.is_empty() {
                        out.push(MenuCmd { id: it.id, title: it.label.clone(), path: path.to_string(), key: it.shortcut.clone(), checked: it.checked, cell: it.cell });
                    }
                } else {
                    walk(&it.sub, &format!("{path} › {}", it.label), out);
                }
            }
        }
        let mut cmds = Vec::new();
        for (title, items) in &menus {
            walk(items, title, &mut cmds);
        }
        let program = name.unwrap_or_default();
        let program = program.rsplit(" — ").next().unwrap_or(&program).to_string();
        self.menu_search_target = target;
        self.launcher.open_menu_search(&program, cmds);
    }

    /// Give the entries their pictures: an icon from the theme (looked up by name; the "actions" folders hold the ones
    /// menus use) or the PNG the program sent, put in the app-icon atlas. A grey icon (a symbolic one, Breeze's
    /// dark glyphs) is painted in the ink of the menu, or it would be lost on its glass.
    pub fn resolve_menu_icons(&mut self, items: &mut [memrium_de::menu::MenuItem]) {
        let ink: [u8; 3] = if self.theme.dark { [240, 242, 250] } else { [26, 30, 43] };
        for it in items.iter_mut() {
            if let Some(src) = it.icon_src.take() {
                it.cell = self.menu_icon_cell(&src, ink);
            }
            if !it.sub.is_empty() {
                self.resolve_menu_icons(&mut it.sub);
            }
        }
    }

    fn menu_icon_cell(&mut self, src: &IconSrc, ink: [u8; 3]) -> Option<u32> {
        use std::hash::{Hash, Hasher};
        let key = match src {
            IconSrc::Name(n) => format!("mi:{n}:{ink:?}"),
            IconSrc::Png(b) => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                b.hash(&mut h);
                format!("mi:png:{:x}:{ink:?}", h.finish())
            }
        };
        if let Some(c) = self.renderer.app_icon_get(&key) {
            return Some(c);
        }
        let img = match src {
            IconSrc::Name(n) => self.action_icons.get().and_then(|i| i.load(n, 64)).or_else(|| self.icons.get().and_then(|i| i.load(n, 64)))?,
            IconSrc::Png(b) => image::load_from_memory(b).ok()?.to_rgba8(),
        };
        self.renderer.set_image_cell(&key, &mono_to_ink(img, ink))
    }

    /// A choice in a program's menu: the program is told, and acts.
    pub fn app_menu_command(&mut self, id: u32) {
        match self.appmenu_target.clone() {
            Some(MenuTarget::App(app)) => self.appmenu.activate(&app, id - APP_BASE),
            Some(MenuTarget::Dbus(xid)) => self.appmenu.bridge.event(xid, id - APP_BASE),
            None => {}
        }
    }

    /// Draw the bar again when a menu was read.
    pub fn appmenu_update(&mut self) {
        if self.appmenu.bridge.take_dirty() {
            self.invalidate();
        }
    }
}

/// An icon that is one grey (a symbolic icon, or a line icon of a theme) takes the colour `ink`, keeping its shape; one with
/// colours of its own is left as it is.
fn mono_to_ink(mut img: image::RgbaImage, ink: [u8; 3]) -> image::RgbaImage {
    let (mut opaque, mut grey) = (0u32, 0u32);
    for p in img.pixels().filter(|p| p.0[3] > 100) {
        opaque += 1;
        let (mx, mn) = (p.0[0].max(p.0[1]).max(p.0[2]), p.0[0].min(p.0[1]).min(p.0[2]));
        if mx - mn <= 28 {
            grey += 1;
        }
    }
    if opaque > 0 && grey * 100 >= opaque * 96 {
        for p in img.pixels_mut() {
            p.0[..3].copy_from_slice(&ink);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grey_icon_takes_the_ink_and_a_coloured_one_does_not() {
        let grey = image::RgbaImage::from_pixel(4, 4, image::Rgba([35, 38, 41, 255]));
        assert_eq!(mono_to_ink(grey, [240, 242, 250]).get_pixel(1, 1).0, [240, 242, 250, 255]);
        let red = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 30, 30, 255]));
        assert_eq!(mono_to_ink(red, [240, 242, 250]).get_pixel(1, 1).0, [200, 30, 30, 255]);
        // transparent pixels stay transparent whatever colour they hold
        let mut clear = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
        clear.put_pixel(0, 0, image::Rgba([30, 30, 30, 255]));
        assert_eq!(mono_to_ink(clear, [1, 2, 3]).get_pixel(1, 1).0[3], 0);
    }
}
