//! Menus of GTK programs under Wayland: `org.gtk.Menus` (the menu bar as a tree of numbered menus, sections and
//! submenus) and `org.gtk.Actions` (what each action is: enabled, its state). `gtk_shell.rs` learns where they are; this
//! turns them into the bar's menus and sends a choice back as `Activate`.

use chebrium::appmenu::{Item, Menu};
use std::collections::{HashMap, HashSet};
use zbus::{
    blocking::Connection,
    zvariant::{OwnedValue, Signature, Value},
};

const MENUS: &str = "org.gtk.Menus";
const ACTIONS: &str = "org.gtk.Actions";

/// Where one window's menus are.
#[derive(Clone, Debug, PartialEq)]
pub struct GtkSource {
    pub service: String,
    pub menubar: String,
    pub window: Option<String>,
    pub app: Option<String>,
}

/// What an id in the bar's menu does: the action (with its `app.` / `win.` prefix) and its target value.
pub type Actions = HashMap<u32, (String, Option<OwnedValue>)>;

/// One item of a menu as the bus sends it.
#[derive(Default)]
pub struct RawItem {
    label: Option<String>,
    action: Option<String>,
    target: Option<OwnedValue>,
    accel: Option<String>,
    icon: Option<String>,
    hidden_when: Option<String>,
    section: Option<(u32, u32)>,
    submenu: Option<(u32, u32)>,
}

type RawMenus = HashMap<(u32, u32), Vec<RawItem>>;

/// What `DescribeAll` says of an action.
#[derive(Clone)]
pub struct ActState {
    pub enabled: bool,
    pub state: Option<OwnedValue>,
}

fn text(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn link(v: &OwnedValue) -> Option<(u32, u32)> {
    match &**v {
        Value::Structure(s) => match s.fields() {
            [Value::U32(a), Value::U32(b)] => Some((*a, *b)),
            _ => None,
        },
        _ => None,
    }
}

fn raw_item(mut m: HashMap<String, OwnedValue>) -> RawItem {
    RawItem {
        label: m.get("label").and_then(text),
        action: m.get("action").and_then(text),
        target: m.remove("target"),
        accel: m.get("accel").and_then(text),
        icon: m.get("icon").and_then(text),
        hidden_when: m.get("hidden-when").and_then(text),
        section: m.get(":section").and_then(link),
        submenu: m.get(":submenu").and_then(link),
    }
}

/// "_File" -> "File" (the underscore marks the mnemonic; "__" is a real one).
fn plain_label(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '_' {
            if it.peek() == Some(&'_') {
                out.push('_');
                it.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// GTK's accelerator text ("<Control><Shift>s") as the bar shows it ("Ctrl+Shift+S").
pub fn accel_text(a: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = a.trim();
    while let Some(r) = rest.strip_prefix('<') {
        let Some(end) = r.find('>') else { break };
        parts.push(
            match r[..end].to_ascii_lowercase().as_str() {
                "control" | "ctrl" | "primary" => "Ctrl",
                "shift" => "Shift",
                "alt" | "mod1" => "Alt",
                "super" | "meta" | "mod4" => "Super",
                _ => "",
            }
            .to_string(),
        );
        rest = &r[end + 1..];
    }
    parts.retain(|p| !p.is_empty());
    let key = match rest {
        "Return" => "Enter".to_string(),
        "Escape" => "Esc".to_string(),
        "minus" => "-".to_string(),
        "plus" => "+".to_string(),
        "equal" => "=".to_string(),
        "slash" => "/".to_string(),
        "comma" => ",".to_string(),
        "period" => ".".to_string(),
        "space" => "Space".to_string(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_string(),
    };
    if key.is_empty() {
        return String::new();
    }
    parts.push(key);
    parts.join("+")
}

struct Builder<'a> {
    menus: &'a RawMenus,
    act: &'a dyn Fn(&str) -> Option<ActState>,
    next: u32,
    actions: Actions,
    /// guards against a menu that contains itself
    open: HashSet<(u32, u32)>,
}

impl Builder<'_> {
    /// The entries of menu `key`: sections are flattened with a separator between them.
    fn walk(&mut self, key: (u32, u32)) -> Vec<Item> {
        let menus = self.menus;
        let Some(raw) = menus.get(&key) else { return Vec::new() };
        if !self.open.insert(key) {
            return Vec::new();
        }
        let mut out: Vec<Item> = Vec::new();
        let mut gap = false;
        for r in raw {
            let items = if let Some(sec) = r.section {
                self.walk(sec)
            } else if let Some(item) = self.item(r) {
                vec![item]
            } else {
                continue;
            };
            if items.is_empty() {
                continue;
            }
            if (gap || r.section.is_some() && !out.is_empty()) && out.last().is_some_and(|l| !l.separator) {
                out.push(Item::sep());
            }
            out.extend(items);
            gap = r.section.is_some();
        }
        self.open.remove(&key);
        out
    }

    fn item(&mut self, r: &RawItem) -> Option<Item> {
        let label = plain_label(r.label.as_deref().unwrap_or(""));
        if let Some(sub) = r.submenu {
            let children = self.walk(sub);
            return Some(Item::parent(label, children));
        }
        let Some(action) = r.action.as_deref() else {
            // a heading
            return (!label.is_empty()).then(|| Item::new(0, label).enabled(false));
        };
        let state = (self.act)(action);
        match (&state, r.hidden_when.as_deref()) {
            (None, Some("action-missing")) => return None,
            (Some(s), Some("action-disabled")) if !s.enabled => return None,
            _ => {}
        }
        let checked = match (&state, &r.target) {
            (Some(ActState { state: Some(s), .. }), Some(t)) => **s == **t,
            (Some(ActState { state: Some(s), .. }), None) => matches!(&**s, Value::Bool(true)),
            _ => false,
        };
        let id = self.next;
        self.next += 1;
        self.actions.insert(id, (action.to_string(), r.target.as_ref().and_then(|t| t.try_clone().ok())));
        let mut item = Item::new(id, label).enabled(state.as_ref().is_some_and(|s| s.enabled)).checked(checked);
        if let Some(a) = &r.accel {
            item = item.key(&accel_text(a));
        }
        if let Some(i) = &r.icon {
            item = item.icon(i);
        }
        Some(item)
    }
}

/// The bar's menus out of the numbered menus: the entries of the root menu that open a submenu.
pub fn build(menus: &RawMenus, act: &dyn Fn(&str) -> Option<ActState>) -> (Vec<Menu>, Actions) {
    let mut b = Builder { menus, act, next: 1, actions: HashMap::new(), open: HashSet::new() };
    let mut bar = Vec::new();
    fn tops<'a>(menus: &'a RawMenus, key: (u32, u32), out: &mut Vec<&'a RawItem>, depth: usize) {
        for r in menus.get(&key).into_iter().flatten() {
            match r.section {
                Some(s) if depth < 8 => tops(menus, s, out, depth + 1),
                _ => out.push(r),
            }
        }
    }
    let mut roots = Vec::new();
    tops(menus, (0, 0), &mut roots, 0);
    for r in roots {
        if let Some(sub) = r.submenu {
            let title = plain_label(r.label.as_deref().unwrap_or(""));
            let items = b.walk(sub);
            if !title.is_empty() {
                bar.push(Menu::new(title, items));
            }
        }
    }
    (bar, b.actions)
}

type Reply = Vec<(u32, u32, Vec<HashMap<String, OwnedValue>>)>;

/// Subscribe to the menu groups reachable from the root, read them, and unsubscribe.
fn fetch_menus(conn: &Connection, src: &GtkSource) -> Option<RawMenus> {
    let mut all: RawMenus = HashMap::new();
    let mut asked: HashSet<u32> = HashSet::new();
    let mut want = vec![0u32];
    for _ in 0..8 {
        want.retain(|g| asked.insert(*g));
        if want.is_empty() {
            break;
        }
        let msg = conn.call_method(Some(src.service.as_str()), src.menubar.as_str(), Some(MENUS), "Start", &(want.clone(),)).ok()?;
        let reply: Reply = msg.body().deserialize().ok()?;
        want.clear();
        for (group, num, items) in reply {
            let items: Vec<RawItem> = items.into_iter().map(raw_item).collect();
            for r in &items {
                for (g, _) in r.section.iter().chain(r.submenu.iter()) {
                    if !asked.contains(g) {
                        want.push(*g);
                    }
                }
            }
            all.insert((group, num), items);
        }
    }
    let mut groups: Vec<u32> = asked.into_iter().collect();
    groups.sort();
    let _ = conn.call_method(Some(src.service.as_str()), src.menubar.as_str(), Some(MENUS), "End", &(groups,));
    all.contains_key(&(0, 0)).then_some(all)
}

fn describe(conn: &Connection, service: &str, path: &str) -> Option<HashMap<String, ActState>> {
    let msg = conn.call_method(Some(service), path, Some(ACTIONS), "DescribeAll", &()).ok()?;
    let all: HashMap<String, (bool, Signature, Vec<OwnedValue>)> = msg.body().deserialize().ok()?;
    Some(all.into_iter().map(|(k, (enabled, _, mut state))| (k, ActState { enabled, state: (!state.is_empty()).then(|| state.remove(0)) })).collect())
}

/// Read a window's menu bar.
pub fn read(conn: &Connection, src: &GtkSource) -> Option<(Vec<Menu>, Actions)> {
    let menus = fetch_menus(conn, src)?;
    let win = src.window.as_deref().and_then(|p| describe(conn, &src.service, p));
    let app = src.app.as_deref().and_then(|p| describe(conn, &src.service, p));
    let act = |name: &str| -> Option<ActState> {
        let (table, short) = match name.split_once('.') {
            Some(("win", s)) => (&win, s),
            Some(("app", s)) => (&app, s),
            _ => return None,
        };
        match table {
            Some(t) => t.get(short).cloned(),
            // the actions could not be read: show the entries as usable rather than all grey
            None => Some(ActState { enabled: true, state: None }),
        }
    };
    Some(build(&menus, &act))
}

/// Run an action of the window or the application.
pub fn activate(conn: &Connection, src: &GtkSource, action: &str, target: Option<&OwnedValue>) {
    let (path, short) = match action.split_once('.') {
        Some(("win", s)) => (src.window.as_deref(), s),
        Some(("app", s)) => (src.app.as_deref(), s),
        _ => (None, action),
    };
    let Some(path) = path else { return };
    let params: Vec<&Value> = target.map(|t| &**t).into_iter().collect();
    let platform: HashMap<&str, Value> = HashMap::new();
    let _ = conn.call_method(Some(src.service.as_str()), path, Some(ACTIONS), "Activate", &(short, params, platform));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(v)).unwrap()
    }

    fn leaf(label: &str, action: &str) -> RawItem {
        RawItem { label: Some(label.into()), action: Some(action.into()), ..Default::default() }
    }

    fn sample() -> RawMenus {
        let mut m: RawMenus = HashMap::new();
        m.insert((0, 0), vec![
            RawItem { label: Some("_File".into()), submenu: Some((0, 1)), ..Default::default() },
            RawItem { label: Some("_View".into()), submenu: Some((0, 2)), ..Default::default() },
        ]);
        m.insert((0, 1), vec![
            RawItem { section: Some((0, 3)), ..Default::default() },
            RawItem { section: Some((0, 4)), ..Default::default() },
        ]);
        m.insert((0, 3), vec![RawItem { accel: Some("<Control>o".into()), ..leaf("_Open", "win.open") }]);
        m.insert((0, 4), vec![leaf("_Quit", "app.quit")]);
        m.insert((0, 2), vec![
            RawItem { target: Some(s("list")), ..leaf("List", "win.mode") },
            RawItem { target: Some(s("grid")), ..leaf("Grid", "win.mode") },
            RawItem { hidden_when: Some("action-missing".into()), ..leaf("Gone", "win.nothing") },
        ]);
        m
    }

    #[test]
    fn menu_tree_becomes_bar_menus() {
        let act = |n: &str| match n {
            "win.open" => Some(ActState { enabled: true, state: None }),
            "app.quit" => Some(ActState { enabled: false, state: None }),
            "win.mode" => Some(ActState { enabled: true, state: Some(s("grid")) }),
            _ => None,
        };
        let (menus, actions) = build(&sample(), &act);
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "View"]);
        let file = &menus[0].items;
        assert_eq!(file.len(), 3);
        assert_eq!((file[0].label.as_str(), file[0].key.as_str(), file[0].enabled), ("Open", "Ctrl+O", true));
        assert!(file[1].separator);
        assert_eq!((file[2].label.as_str(), file[2].enabled), ("Quit", false));
        let view = &menus[1].items;
        assert_eq!(view.len(), 2, "the entry whose action is missing is hidden");
        assert!(!view[0].checked && view[1].checked, "the radio entry whose target is the state is checked");
        assert_eq!(actions[&file[0].id].0, "win.open");
        assert_eq!(actions[&view[1].id].0, "win.mode");
    }

    #[test]
    fn accelerators_read_like_shortcuts() {
        assert_eq!(accel_text("<Control><Shift>s"), "Ctrl+Shift+S");
        assert_eq!(accel_text("<Primary>comma"), "Ctrl+,");
        assert_eq!(accel_text("F1"), "F1");
        assert_eq!(accel_text(""), "");
    }

    #[test]
    fn mnemonics_are_dropped() {
        assert_eq!(plain_label("_Save _As…"), "Save As…");
        assert_eq!(plain_label("Rock__Roll"), "Rock_Roll");
    }
}
