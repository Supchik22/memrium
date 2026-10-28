//! The tray on the shell's side: what the service reports becomes icons in the atlas and in the panel, and a click does
//! what the application's icon is for (open it, show its menu).

use crate::state::{MenuFor, State};
use crate::tray_service::{MenuNode, TrayEv};
use memrium_de::menu::MenuItem;
use memrium_de::tray::{TrayIcon, TrayStatus};

impl State {
    /// The icons the panel shows now: the active ones, and all of them after a click on the "more" arrow. Kept in
    /// `tray_shown` (the panel looks at it many times a frame), rebuilt here when something changes.
    pub fn tray_refresh(&mut self) {
        self.tray_shown = self.tray_items.iter().filter(|i| self.tray_expanded || i.status != TrayStatus::Passive).cloned().collect();
    }

    pub fn tray_visible(&self) -> &[TrayIcon] {
        &self.tray_shown
    }

    /// Take what the service found out since the last frame.
    pub fn tray_update(&mut self) {
        if !self.tray.active() {
            return;
        }
        let mut changed = false;
        for ev in self.tray.poll() {
            match ev {
                TrayEv::Add(_) => {}
                TrayEv::Remove(id) => {
                    self.tray_items.retain(|i| i.id != id);
                    changed = true;
                }
                TrayEv::Update(mut icon) => {
                    let (cell, inverse) = self.tray_cell(&icon);
                    icon.cell = cell;
                    icon.inverse = inverse;
                    // the picture is in the atlas now: not carried around any more
                    if icon.cell.is_some() {
                        icon.pixmap = None;
                    }
                    match self.tray_items.iter_mut().find(|i| i.id == icon.id) {
                        Some(old) => *old = icon,
                        None => self.tray_items.push(icon),
                    }
                    changed = true;
                }
                TrayEv::Menu { id, at, nodes } => self.open_tray_menu(id, at, nodes),
            }
        }
        // the icon theme may come after the first reading: icons without a picture yet are tried again, now and then
        // (a lookup in the icon theme is a walk over directories, nothing to do every frame)
        if self.tray_items.iter().any(|i| i.cell.is_none()) && self.tray_retry.elapsed().as_secs_f32() > 3.0 {
            self.tray_retry = std::time::Instant::now();
            for n in 0..self.tray_items.len() {
                if self.tray_items[n].cell.is_none() {
                    let icon = self.tray_items[n].clone();
                    let (cell, inverse) = self.tray_cell(&icon);
                    self.tray_items[n].cell = cell;
                    self.tray_items[n].inverse = inverse;
                    if self.tray_items[n].cell.is_some() {
                        self.tray_items[n].pixmap = None;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.tray_refresh();
            self.invalidate();
        }
    }

    /// Make the atlas picture of an icon: its theme icon (or file), else the picture it sent.
    fn tray_cell(&mut self, icon: &TrayIcon) -> (Option<u32>, Option<(u32, bool)>) {
        let key = format!("tray:{}", icon.id);
        let by_name = || -> Option<image::RgbaImage> {
            if icon.icon_name.is_empty() {
                return None;
            }
            let p = std::path::Path::new(&icon.icon_name);
            if p.is_absolute() {
                return memrium_de::apps::load_icon_file(p, 128);
            }
            if !icon.icon_dir.is_empty() {
                for ext in ["png", "svg"] {
                    let f = std::path::Path::new(&icon.icon_dir).join(format!("{}.{ext}", icon.icon_name));
                    if f.exists() {
                        if let Some(img) = memrium_de::apps::load_icon_file(&f, 128) {
                            return Some(img);
                        }
                    }
                }
            }
            self.icons.get()?.load(&icon.icon_name, 128)
        };
        let Some(img) = by_name().or_else(|| icon.pixmap.as_ref().and_then(|p| image::RgbaImage::from_raw(p.w, p.h, p.rgba.clone()))) else { return (None, None) };
        let cell = self.renderer.set_image_cell(&key, &img);
        // a plain white icon is lost on a light bar (and a black one on a dark bar): keep the opposite too
        let inverse = memrium_de::tray::plain_tone(img.as_raw()).and_then(|light| {
            let flipped = image::RgbaImage::from_raw(img.width(), img.height(), memrium_de::tray::inverted(img.as_raw()))?;
            self.renderer.set_image_cell(&format!("{key}:inverse"), &flipped).map(|c| (c, light))
        });
        (cell, inverse)
    }

    /// A click on an icon: `button` 0 left, 1 right, 2 middle.
    pub fn tray_click(&mut self, id: &str, button: u8) {
        let Some(icon) = self.tray_items.iter().find(|i| i.id == id).cloned() else { return };
        let at = self.mouse;
        self.menu.close();
        match button {
            0 if !icon.is_menu => self.tray.call(id, "Activate", at),
            2 => self.tray.call(id, "SecondaryAction", at),
            // right click (and a left click on an item that is only a menu): its menu
            _ if !icon.menu.is_empty() => self.tray.fetch_menu(id, &icon.menu, at),
            _ => self.tray.call(id, "ContextMenu", at),
        }
    }

    /// A right or middle click on the panel: true when it was on a tray icon.
    pub fn tray_alt_click(&mut self, right: bool) -> bool {
        let Some(h) = self.bar_hit(self.mouse) else { return false };
        let Some(n) = self.bars.get(h.bar).and_then(|b| b.tray_part(h.widget, h.part)) else { return false };
        let Some(icon) = self.tray_shown.get(n) else { return false };
        let id = icon.id.clone();
        self.tray_click(&id, if right { 1 } else { 2 });
        true
    }

    fn open_tray_menu(&mut self, id: String, at: [f32; 2], nodes: Vec<MenuNode>) {
        fn convert(nodes: &[MenuNode]) -> Vec<MenuItem> {
            nodes
                .iter()
                .map(|n| {
                    if n.separator {
                        return MenuItem::sep();
                    }
                    // an entry with children is a submenu, which has no id of its own
                    let mut item = MenuItem::new(if n.children.is_empty() { n.id.max(1) as u32 } else { 0 }, n.label.clone()).checked(n.checked);
                    item.enabled = n.enabled;
                    if n.children.is_empty() { item } else { item.sub(convert(&n.children)) }
                })
                .collect()
        }
        let items = convert(&nodes);
        let Some(menu) = self.tray_items.iter().find(|i| i.id == id).map(|i| i.menu.clone()) else { return };
        if items.is_empty() {
            return;
        }
        self.tray_menu = Some((id, menu));
        self.menu_for = MenuFor::Tray;
        // under a top bar, over a bottom one
        let height: f32 = items.iter().map(|i| if i.separator { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
        let y = if at[1] > self.size[1] / 2.0 { (at[1] - 24.0 - height).max(8.0) } else { at[1] + 18.0 };
        self.menu.open_at([at[0] - 20.0, y], items);
    }

    pub fn tray_menu_command(&mut self, id: u32) {
        let Some((item, menu)) = self.tray_menu.clone() else { return };
        self.tray.menu_click(&item, &menu, id as i32);
    }
}

impl State {
    /// With "follow the GTK theme" on: look at it every few seconds (on a thread: it asks `gsettings`), and when light /
    /// dark or the accent changed, take them over.
    pub fn follow_gtk(&mut self) {
        if !self.theme.ui.follow_gtk {
            self.gtk_rx = None;
            self.gtk_last = None;
            return;
        }
        if let Some(rx) = &self.gtk_rx {
            if let Ok(found) = rx.try_recv() {
                self.gtk_rx = None;
                if let Some(look) = found.filter(|l| self.gtk_last.as_ref() != Some(l)) {
                    let mut t = self.theme.clone();
                    if memrium_de::gtk::apply(&mut t, &look) {
                        self.apply_theme(t);
                    }
                    self.gtk_last = Some(look);
                }
            }
        } else if self.gtk_next <= std::time::Instant::now() {
            self.gtk_next = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (tx, rx) = std::sync::mpsc::channel();
            self.gtk_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(memrium_de::gtk::read());
            });
        }
    }
}
