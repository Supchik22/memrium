//! Memrium Share: the window that asks what to share on the screen. The screen-sharing portal starts it with the
//! choices as arguments and reads the answer from stdout (a JSON line; the exit code tells when nothing was chosen):
//!   memrium_share [--app NAME] [--screen] [--window ID<US>TITLE<US>APP]...
//! (`<US>` is the character 0x1f.)

use chebrium::app::{self, App, Config, Host};
use chebrium::draw::Align;
use chebrium::input::{Key, KeyMods};
use chebrium::ui::Ui;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Item {
    /// "screen" or the window's identifier
    id: String,
    title: String,
    sub: String,
    glyph: &'static str,
}

struct Share {
    app: String,
    items: Vec<Item>,
    sel: usize,
    first: usize,
    answer: Arc<Mutex<Option<String>>>,
}

const ROW: f32 = 46.0;

impl Share {
    fn confirm(&mut self, host: &mut Host) {
        if let Some(item) = self.items.get(self.sel) {
            *self.answer.lock().unwrap() = Some(item.id.clone());
            host.quit = true;
        }
    }
}

impl App for Share {
    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) {
        let (w, h, ink) = (size[0], size[1], ui.ink);
        ui.text(24.0, 40.0, &format!("{} wants to share your screen", self.app), 16.0, true, ink, 0.95, Align::Left);
        ui.text(24.0, 62.0, "Choose what it may see", 12.5, false, ink, 0.55, Align::Left);
        let top = 78.0;
        let rows = (((h - top - 76.0) / ROW) as usize).max(1);
        if self.sel < self.first {
            self.first = self.sel;
        } else if self.sel >= self.first + rows {
            self.first = self.sel + 1 - rows;
        }
        let accent = ui.accent;
        for (n, item) in self.items.iter().enumerate().skip(self.first).take(rows) {
            let y = top + (n - self.first) as f32 * ROW;
            let r = [20.0, y, w - 40.0, ROW - 4.0];
            let hover = ui.hit(r);
            if n == self.sel {
                ui.fill(r, 10.0, accent, 0.30);
            } else if hover {
                ui.fill(r, 10.0, ink, 0.08);
            }
            ui.glyph(r[0] + 24.0, r[1] + r[3] / 2.0, 20.0, item.glyph, ink, 0.85);
            ui.text(r[0] + 50.0, r[1] + 19.0, &item.title, 13.5, true, ink, 0.95, Align::Left);
            if !item.sub.is_empty() {
                ui.text(r[0] + 50.0, r[1] + 34.0, &item.sub, 11.0, false, ink, 0.5, Align::Left);
            }
            if hover && ui.click {
                self.sel = n;
            }
        }
        let cancel = [w - 24.0 - 96.0 - 8.0 - 96.0, h - 24.0 - 32.0, 96.0, 32.0];
        let share = [w - 24.0 - 96.0, h - 24.0 - 32.0, 96.0, 32.0];
        if ui.button(cancel, "Cancel", false) {
            host.quit = true;
        } else if ui.button(share, "Share", true) {
            self.confirm(host);
        }
    }

    fn key(&mut self, key: Key, _mods: KeyMods, host: &mut Host) {
        match key {
            Key::Esc => host.quit = true,
            Key::Enter => self.confirm(host),
            Key::Up => self.sel = self.sel.saturating_sub(1),
            Key::Down => self.sel = (self.sel + 1).min(self.items.len().saturating_sub(1)),
            _ => {}
        }
    }
}

fn main() {
    let mut app_name = "A program".to_string();
    let mut items = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app" => app_name = args.next().unwrap_or_default(),
            "--screen" => items.push(Item { id: "screen".into(), title: "Entire screen".into(), sub: "Everything that is on it".into(), glyph: "monitor" }),
            "--window" => {
                let spec = args.next().unwrap_or_default();
                let mut parts = spec.split('\x1f');
                let id = parts.next().unwrap_or("").to_string();
                let title = parts.next().unwrap_or("").to_string();
                let sub = parts.next().unwrap_or("").to_string();
                if !id.is_empty() {
                    items.push(Item { id, title: if title.is_empty() { sub.clone() } else { title }, sub, glyph: "app-window" });
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        std::process::exit(1);
    }
    let answer = Arc::new(Mutex::new(None));
    let mut config = Config::new("Share your screen", "memrium-share").size(460, 420);
    config.resizable = false;
    app::run(config, Share { app: app_name, items, sel: 0, first: 0, answer: answer.clone() });
    let taken = answer.lock().unwrap().take();
    match taken {
        Some(id) => println!("{}", serde_json::json!({ "choice": id })),
        None => std::process::exit(1),
    }
}
