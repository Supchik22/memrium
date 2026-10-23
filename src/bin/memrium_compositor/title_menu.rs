//! A program's menus in its own title bar, for the themes that have no menu bar along the top of the screen (Fluent, Aero):
//! the names sit at the left of the title bar, a press opens the menu below the name, and with one open the pointer
//! moving to the next name opens that one. Where the menus come from is the same as for the menu bar
//! (`appmenu_service.rs`).

use crate::state::{Deco, MenuFor, State};
use memrium_de::decor;
use memrium_de::widget::BarHit;

impl State {
    /// Programs put their menus in their title bars when the theme's title bar has room for them and no bar of the shell
    /// holds the app-menu widget.
    pub fn menus_in_titlebars(&self) -> bool {
        decor::title_menus_fit(&self.theme) && !self.bars.iter().any(|b| b.has("app-menu"))
    }

    /// The menus of window `i` as they sit in its title bar: each name with its box.
    pub fn title_menus(&self, i: usize) -> Vec<(String, [f32; 4])> {
        if !self.menus_in_titlebars() {
            return Vec::new();
        }
        let Some(w) = self.wins.get(i) else { return Vec::new() };
        if !w.mapped || w.or || w.deco != Deco::Server || w.fs.is_some() || w.ins[1] <= 0.0 {
            return Vec::new();
        }
        let menus = self.menus_at(self.menu_target_of(w));
        if menus.is_empty() {
            return Vec::new();
        }
        let widths: Vec<f32> = menus.iter().map(|m| self.renderer.text_width(&m.title, decor::TITLE_MENU_PX, false)).collect();
        let rects = decor::title_menu_rects(&w.chrome_theme(&self.theme), Self::frame_rect(w), &widths);
        menus.into_iter().zip(rects).map(|(m, r)| (m.title, r)).collect()
    }

    /// The menu name of window `i`'s title bar under `p`.
    pub fn title_menu_at(&self, i: usize, p: [f32; 2]) -> Option<(String, [f32; 4])> {
        self.title_menus(i).into_iter().find(|(_, r)| Self::in_rect(p, *r))
    }

    /// Open menu `name` of window `i` below its name's box `rect`.
    pub fn open_title_menu(&mut self, i: usize, name: String, rect: [f32; 4]) {
        let Some(w) = self.wins.get(i) else { return };
        let (wid, target) = (w.id, self.menu_target_of(w));
        let Some(mut items) = self.menu_items_at(target.clone(), &name) else { return };
        self.resolve_menu_icons(&mut items);
        self.appmenu_target = target;
        self.title_menu = Some((wid, name));
        // the choices go the way the menu bar's do (see `bar_command`); no bar has this index, so none is lit
        self.menu_for = MenuFor::Bar(BarHit { bar: usize::MAX, widget: 0, part: 0 });
        self.menu.open_at([rect[0], rect[1] + rect[3] + 1.0], items);
    }

    /// With a title-bar menu open, the pointer over another name of the same title bar opens that one.
    pub fn title_menu_hover(&mut self, p: [f32; 2]) {
        if !self.menu.open {
            return;
        }
        let Some((wid, cur)) = self.title_menu.clone() else { return };
        let Some(i) = self.wins.iter().position(|w| w.id == wid) else { return };
        if let Some((name, rect)) = self.title_menu_at(i, p) {
            if name != cur {
                self.open_title_menu(i, name, rect);
            }
        }
    }
}
