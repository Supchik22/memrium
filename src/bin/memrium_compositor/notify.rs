//! Desktop notifications: the `org.freedesktop.Notifications` service (what `notify-send` and every application
//! talk to) and the toasts that show them, drawn with the shell's glass in a corner of the screen.

use crate::state::State;
use memrium_de::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc,
};
use std::time::Instant;
use zbus::{blocking::connection::Builder, blocking::Connection, interface, zvariant::Value};

const WIDTH: f32 = 372.0;
const GAP: f32 = 10.0;
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

/// A notification as an application sent it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    /// (key, label) pairs; the key "default" is what a click on the toast does
    pub actions: Vec<(String, String)>,
    /// 0 low, 1 normal, 2 critical
    pub urgency: u8,
    /// milliseconds; -1 = server default, 0 = stays until dismissed
    pub expire_ms: i32,
}

pub enum NotifEvent {
    Show(Notification),
    Close(u32),
}

struct Server {
    tx: Sender<NotifEvent>,
    next: Arc<AtomicU32>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["body", "actions"].iter().map(|s| s.to_string()).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: String, replaces_id: u32, _app_icon: String, summary: String, body: String, actions: Vec<String>, hints: HashMap<String, Value<'_>>, expire_timeout: i32) -> u32 {
        let id = if replaces_id != 0 { replaces_id } else { self.next.fetch_add(1, Ordering::Relaxed) };
        let urgency = match hints.get("urgency") {
            Some(Value::U8(n)) => (*n).min(2),
            _ => 1,
        };
        let actions = actions.chunks_exact(2).map(|p| (p[0].clone(), p[1].clone())).collect();
        let _ = self.tx.send(NotifEvent::Show(Notification { id, app: app_name, summary, body, actions, urgency, expire_ms: expire_timeout }));
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(NotifEvent::Close(id));
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("Memrium".into(), "memrium".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

/// One toast on screen.
pub struct Toast {
    pub n: Notification,
    born: Instant,
    /// 0 -> 1 appearing, back to 0 when it goes away
    k: Spring,
    /// slot position (top of the toast), so the stack slides when one leaves
    y: Spring,
    h: f32,
    /// why it is leaving (the spec's close reasons: 1 expired, 2 dismissed, 3 closed by the application)
    closing: Option<u32>,
}

/// A notification kept in the history: when it came, and whether do-not-disturb kept it from being shown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stored {
    pub n: Notification,
    /// seconds since the epoch
    pub at: u64,
    pub silenced: bool,
}

/// The most that are kept.
const KEEP: usize = 200;

#[derive(Default, Serialize, Deserialize)]
struct HistoryFile {
    items: Vec<Stored>,
}

fn history_path() -> std::path::PathBuf {
    memrium_de::theme::config_dir().join("notifications.toml")
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub struct Notifications {
    conn: Option<Connection>,
    rx: Option<Receiver<NotifEvent>>,
    pub toasts: Vec<Toast>,
    /// everything that came, newest first
    pub history: Vec<Stored>,
    /// when the centre was last looked at: what came after it is unread
    pub seen_at: u64,
    /// the history changed and is to be written
    dirty: Option<Instant>,
}

impl Notifications {
    pub fn new() -> Notifications {
        let history = std::fs::read_to_string(history_path()).ok().and_then(|s| toml::from_str::<HistoryFile>(&s).ok()).map(|f| f.items).unwrap_or_default();
        Notifications { conn: None, rx: None, toasts: Vec::new(), history, seen_at: unix_now(), dirty: None }
    }

    /// Keep a notification: a replacement of one that is there takes its place at the top.
    pub fn record(&mut self, n: Notification, silenced: bool) {
        self.history.retain(|s| s.n.id != n.id);
        self.history.insert(0, Stored { n, at: unix_now(), silenced });
        self.history.truncate(KEEP);
        self.dirty = Some(Instant::now());
    }

    pub fn forget(&mut self, id: u32) {
        let before = self.history.len();
        self.history.retain(|s| s.n.id != id);
        if self.history.len() != before {
            self.dirty = Some(Instant::now());
        }
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.dirty = Some(Instant::now());
    }

    /// How many came since the centre was last opened.
    pub fn unread(&self) -> usize {
        self.history.iter().filter(|s| s.at > self.seen_at).count()
    }

    /// Write the history when it changed a moment ago (a burst of them is written once).
    fn flush(&mut self) {
        if !self.dirty.is_some_and(|d| d.elapsed().as_millis() > 1500) {
            return;
        }
        self.dirty = None;
        let path = history_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string(&HistoryFile { items: self.history.clone() }) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Wrap `text` into at most `max_lines` lines of at most `width` px; the last line gets an ellipsis if cut.
fn wrap(text: &str, width: f32, size: f32, measure: &dyn Fn(&str, f32, bool) -> f32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&candidate, size, false) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && measure(&format!("{last}…"), size, false) > width {
                last.pop();
            }
            last.push('…');
        }
    }
    lines.retain(|l| !l.is_empty());
    lines
}

impl State {
    /// Offer the notification service on the session bus. A desktop that already has one keeps it.
    pub fn start_notifications(&mut self) {
        let (tx, rx) = channel();
        let server = Server { tx, next: Arc::new(AtomicU32::new(1)) };
        let built = Builder::session().and_then(|b| b.name("org.freedesktop.Notifications")).and_then(|b| b.serve_at(PATH, server)).and_then(|b| b.build());
        match built {
            Ok(conn) => {
                eprintln!("notifications: serving org.freedesktop.Notifications");
                self.notifications.conn = Some(conn);
                self.notifications.rx = Some(rx);
            }
            Err(e) => eprintln!("notifications: not serving ({e})"),
        }
    }

    fn emit_closed(&self, id: u32, reason: u32) {
        if let Some(c) = &self.notifications.conn {
            let _ = c.emit_signal(None::<&str>, PATH, IFACE, "NotificationClosed", &(id, reason));
        }
    }

    pub(crate) fn emit_action(&self, id: u32, key: &str) {
        if let Some(c) = &self.notifications.conn {
            let _ = c.emit_signal(None::<&str>, PATH, IFACE, "ActionInvoked", &(id, key));
        }
    }

    /// Where the stack starts, and which way it grows.
    fn toast_origin(&self) -> (f32, bool) {
        let area = self.work_area();
        use memrium_de::theme::Corner;
        match self.config.notifications.corner {
            Corner::TopRight | Corner::TopLeft => return (area[1] + GAP, false),
            Corner::BottomRight | Corner::BottomLeft => return (area[1] + area[3] - GAP, true),
            Corner::Auto => {}
        }
        // a top bar: the stack hangs under it; otherwise it sits in the bottom corner above the taskbar
        if area[1] > 20.0 {
            (area[1] + GAP, false)
        } else {
            (area[1] + area[3] - GAP, true)
        }
    }

    fn toast_rect(&self, t: &Toast) -> [f32; 4] {
        // on the monitor the pointer is on
        let m = self.mons.get(self.mon_at(self.mouse)).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        let slide = (1.0 - t.k.x.min(1.0)) * 40.0;
        let x = if self.config.notifications.corner.left() { m[0] + 14.0 - slide } else { m[0] + m[2] - 14.0 - WIDTH + slide };
        [x, t.y.x + m[1], WIDTH, t.h]
    }

    pub fn toast_at(&self, p: [f32; 2]) -> Option<usize> {
        self.notifications.toasts.iter().position(|t| {
            let r = self.toast_rect(t);
            t.closing.is_none() && p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        })
    }

    /// A click on a toast: do its default action, if any, and dismiss it. True when the click was on a toast.
    pub fn toast_click(&mut self, p: [f32; 2]) -> bool {
        let Some(i) = self.toast_at(p) else { return false };
        let (id, default) = {
            let t = &self.notifications.toasts[i];
            (t.n.id, t.n.actions.iter().any(|(k, _)| k == "default"))
        };
        if default {
            self.emit_action(id, "default");
        }
        self.close_toast(i, 2);
        true
    }

    fn close_toast(&mut self, i: usize, reason: u32) {
        let t = &mut self.notifications.toasts[i];
        if t.closing.is_none() {
            t.closing = Some(reason);
            t.k.target = 0.0;
        }
    }

    /// Take in new notifications, expire old ones and animate the stack. Cheap when there is nothing to do.
    pub fn update_toasts(&mut self, dt: f32) {
        let an = self.theme.animations.clone();
        let mut incoming = Vec::new();
        if let Some(rx) = &self.notifications.rx {
            while let Ok(ev) = rx.try_recv() {
                incoming.push(ev);
            }
        }
        for ev in incoming {
            match ev {
                NotifEvent::Close(id) => {
                    // the program took it back: it is not news any more
                    self.notifications.forget(id);
                    if let Some(i) = self.notifications.toasts.iter().position(|t| t.n.id == id) {
                        self.close_toast(i, 3);
                    } else {
                        self.emit_closed(id, 3);
                    }
                }
                NotifEvent::Show(n) => self.take_in(n),
            }
        }
        self.notifications.flush();
        if self.notifications.toasts.is_empty() {
            return;
        }
        // expire
        let now = Instant::now();
        let expired: Vec<usize> = self
            .notifications
            .toasts
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                let ms = match (t.n.expire_ms, t.n.urgency) {
                    (_, 2) | (0, _) => return false,
                    (m, _) if m > 0 => m as u128,
                    _ => (self.config.notifications.seconds.max(1.0) * 1000.0) as u128,
                };
                t.closing.is_none() && now.duration_since(t.born).as_millis() > ms
            })
            .map(|(i, _)| i)
            .collect();
        for i in expired {
            self.close_toast(i, 1);
        }
        // stack: first toast nearest the origin
        let (origin, up) = self.toast_origin();
        let mut cursor = origin;
        let mut gone = Vec::new();
        // a stack in the bottom corner grows upwards with the newest toast nearest the corner
        let mut order: Vec<usize> = (0..self.notifications.toasts.len()).collect();
        if up {
            order.reverse();
        }
        for i in order {
            let t = &mut self.notifications.toasts[i];
            let top = if up { cursor - t.h } else { cursor };
            t.y.target = top;
            an.step("notifications", &mut t.k, dt);
            an.step("notifications", &mut t.y, dt);
            if t.closing.is_none() {
                cursor = if up { top - GAP } else { top + t.h + GAP };
            }
            if t.closing.is_some() && t.k.x < 0.02 {
                gone.push((t.n.id, t.closing.unwrap_or(1)));
            }
        }
        for (id, reason) in gone {
            self.notifications.toasts.retain(|t| t.n.id != id);
            self.emit_closed(id, reason);
        }
    }

    /// A notification arrived: keep it, and show it unless do-not-disturb is on (urgent ones always come through).
    pub fn take_in(&mut self, n: Notification) {
        let silenced = self.config.notifications.dnd && n.urgency < 2;
        self.notifications.record(n.clone(), silenced);
        // the centre is open: it shows the new one itself
        let centre_open = self.card.as_ref().is_some_and(|p| p.kind == crate::popup::PopKind::Notifications && !p.closing);
        if silenced {
            self.emit_closed(n.id, 2);
        } else if !centre_open {
            self.show_toast(n);
        }
    }

    /// The headless test hook: `notify app/summary/body[/urgency]` (underscores are spaces).
    pub fn test_notify(&mut self, spec: &str) {
        let mut it = spec.split('/').map(|s| s.replace('_', " "));
        let (app, summary, body) = (it.next().unwrap_or_default(), it.next().unwrap_or_default(), it.next().unwrap_or_default());
        let urgency = it.next().and_then(|u| u.parse().ok()).unwrap_or(1);
        let id = 800_000 + self.notifications.history.len() as u32;
        self.take_in(Notification { id, app, summary, body, actions: Vec::new(), urgency, expire_ms: -1 });
    }

    /// A notification of our own, to see how they look with the preferences as they are now.
    pub fn test_notification(&mut self) {
        let id = 900_000 + (self.start.elapsed().as_millis() % 1000) as u32;
        self.show_toast(Notification { id, app: "Settings".into(), summary: "A test notification".into(), body: "This is how notifications look and where they appear.".into(), actions: Vec::new(), urgency: 1, expire_ms: -1 });
    }

    fn show_toast(&mut self, n: Notification) {
        if n.urgency != 0 {
            self.ui_sound(memrium_de::sound::Sound::Notify);
        }
        // layout is measured once, here
        let measure = |t: &str, s: f32, b: bool| self.renderer.text_width(t, s, b);
        let text_w = WIDTH - 14.0 - 52.0 - 14.0;
        let body_lines = wrap(&n.body, text_w, 12.5, &measure, 3).len().max(usize::from(!n.body.is_empty()));
        let sum_lines = wrap(&n.summary, text_w, 13.5, &measure, 2).len().max(1);
        let h = 16.0 + 14.0 + sum_lines as f32 * 18.0 + body_lines as f32 * 17.0 + 14.0;
        let (origin, up) = self.toast_origin();
        let start = if up { origin - h } else { origin };
        if let Some(t) = self.notifications.toasts.iter_mut().find(|t| t.n.id == n.id) {
            // replacing a notification keeps its place
            t.n = n;
            t.h = h;
            t.born = Instant::now();
            t.closing = None;
            t.k.target = 1.0;
            return;
        }
        // the most that are shown at once: the oldest makes room
        let most = self.config.notifications.max_shown.max(1) as usize;
        while self.notifications.toasts.iter().filter(|t| t.closing.is_none()).count() >= most {
            match self.notifications.toasts.iter().position(|t| t.closing.is_none()) {
                Some(i) => self.close_toast(i, 2),
                None => break,
            }
        }
        let mut k = Spring::new(0.0, 300.0, 0.7);
        k.target = 1.0;
        let mut y = Spring::new(start, 400.0, 0.85);
        y.target = start;
        self.notifications.toasts.push(Toast { n, born: Instant::now(), k, y, h, closing: None });
    }

    /// Panels and labels of every toast (they go above windows, in the shell layer).
    pub fn toast_panels(&mut self, panels: &mut Vec<Panel>, labels: &mut Vec<Label>) {
        if self.notifications.toasts.is_empty() {
            return;
        }
        let dark = self.theme.dark;
        let ink = if dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let tint = if dark { [0.09, 0.09, 0.09, 0.74] } else { [0.97, 0.98, 1.0, 0.72] };
        let tint = if windows { [tint[0], tint[1], tint[2], 0.88] } else { tint };
        let toasts = std::mem::take(&mut self.notifications.toasts);
        for t in &toasts {
            let k = t.k.x.clamp(0.0, 1.0);
            let r = self.toast_rect(t);
            let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            let mut p = self.theme.glass.panel(cx, cy, r[2], r[3], if windows { 8.0 } else { 20.0 }).tinted([tint[0], tint[1], tint[2], tint[3] * k]);
            if !windows {
                p = p.adaptive();
            }
            p.extra[0] *= k;
            // text on top of a busy desktop needs a frosted pane, whatever the theme's glass does
            p.fx[0] = p.fx[0].max(26.0);
            if windows {
                // Windows toasts are flat acrylic cards: no refraction, no edge light, a hairline border
                p.params[3] = 0.0;
                p.fx = [p.fx[0].max(32.0), 0.0, 0.0, 0.0];
                p.color2 = if dark { [1.0, 1.0, 1.0, 0.10 * k] } else { [0.0, 0.0, 0.0, 0.10 * k] };
            }
            p.fade_by(k);
            panels.push(p);
            // the application's icon, or a bell
            let icon_c = [r[0] + 14.0 + 20.0, r[1] + 16.0 + 20.0];
            let app = t.n.app.to_lowercase();
            let idx = self.apps.iter().position(|a| a.name.to_lowercase() == app || a.id.trim_end_matches(".desktop").to_lowercase() == app);
            let mut budget = 2u32;
            if let Some(i) = idx {
                self.ensure_icon_pub(i, &mut budget);
            }
            match idx.and_then(|i| self.renderer.app_icon_get(&self.apps[i].id)) {
                Some(cell) => {
                    let mut tmp = Vec::new();
                    memrium_de::theme::push_app_icon(&self.theme, &mut tmp, icon_c[0], icon_c[1], 40.0, cell, (0.0, 0.0, 0.0));
                    for mut q in tmp {
                        q.tint[3] *= k;
                        panels.push(q);
                    }
                }
                None => {
                    panels.push(Panel::solid(icon_c[0], icon_c[1], 40.0, 40.0, if windows { 6.0 } else { 11.0 }, [0.30, 0.52, 0.98], [0.20, 0.38, 0.85]).flat().alpha(k));
                    panels.push(Panel::glyph(icon_c[0], icon_c[1], 20.0, "bell", [1.0; 3], k));
                }
            }
            let measure = |s: &str, size: f32, b: bool| self.renderer.text_width(s, size, b);
            let tx = r[0] + 14.0 + 52.0;
            let text_w = WIDTH - 14.0 - 52.0 - 14.0;
            let mut y = r[1] + 16.0;
            labels.push(Label::new(t.n.app.clone(), tx, y + 8.0, 11.0, false, ink, 0.55 * k, Align::Left));
            y += 14.0;
            for line in wrap(&t.n.summary, text_w, 13.5, &measure, 2) {
                y += 18.0;
                labels.push(Label::new(line, tx, y - 4.0, 13.5, true, ink, 0.97 * k, Align::Left));
            }
            for line in wrap(&t.n.body, text_w, 12.5, &measure, 3) {
                y += 17.0;
                labels.push(Label::new(line, tx, y - 4.0, 12.5, false, ink, 0.80 * k, Align::Left));
            }
        }
        self.notifications.toasts = toasts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32) -> Notification {
        Notification { id, app: "a".into(), summary: format!("s{id}"), body: String::new(), actions: Vec::new(), urgency: 1, expire_ms: -1 }
    }

    fn empty() -> Notifications {
        Notifications { conn: None, rx: None, toasts: Vec::new(), history: Vec::new(), seen_at: 0, dirty: None }
    }

    #[test]
    fn history_is_newest_first_and_a_replacement_takes_the_top() {
        let mut n = empty();
        for id in [1, 2, 3] {
            n.record(note(id), false);
        }
        n.record(note(1), true);
        let ids: Vec<u32> = n.history.iter().map(|s| s.n.id).collect();
        assert_eq!(ids, [1, 3, 2]);
        assert!(n.history[0].silenced);
        n.forget(3);
        assert_eq!(n.history.len(), 2);
        assert_eq!(n.unread(), 2);
        n.seen_at = u64::MAX;
        assert_eq!(n.unread(), 0);
        n.clear();
        assert!(n.history.is_empty());
    }

    #[test]
    fn the_history_is_capped() {
        let mut n = empty();
        for id in 0..KEEP as u32 + 30 {
            n.record(note(id), false);
        }
        assert_eq!(n.history.len(), KEEP);
        assert_eq!(n.history[0].n.id, KEEP as u32 + 29);
    }
}
