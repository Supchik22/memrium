//! What Figlet can find besides applications: a calculator, files (indexed in the background),
//! system actions, shell commands and a web search fallback.

use crate::fsops;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What a result does when chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// index into the app list
    App(usize),
    File(PathBuf),
    /// run a shell action by name (`State::run_action`)
    Action(&'static str),
    /// the answer of a calculation: replaces the query
    Calc(String),
    Web(String),
    Run(String),
    /// an entry of the front program's menu (the id the menu bar would send; see `State::open_menu_search`)
    MenuCmd(u32),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub target: Target,
    pub title: String,
    pub sub: String,
    /// small right-aligned category: "Application", "Folder", "Calculator", ...
    pub tag: &'static str,
    pub glyph: &'static str,
    pub tint: [f32; 3],
    /// a shortcut shown at the right instead of the tag
    pub key: String,
    /// a picture in the app-icon atlas shown instead of the glyph (a menu entry's own icon)
    pub cell: Option<u32>,
}

// ---------------------------------------------------------------------------------------- actions

/// One command of a program's menu, as the search lists them.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuCmd {
    /// the id the menu bar sends (`APP_BASE` and all)
    pub id: u32,
    pub title: String,
    /// the menus it is in: "File › Open Recent"
    pub path: String,
    pub key: String,
    pub checked: bool,
    pub cell: Option<u32>,
}

/// The commands that match `query`, best first (`limit` of them). Every word of the query has to be found, in the title
/// (a word's start counts most) or in the path of menus; a title the letters of the query run through in order, like
/// "sva" for "Save As", is a weaker match. No query lists the commands in the order of the menus.
pub fn menu_rank(cmds: &[MenuCmd], query: &str, limit: usize) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return (0..cmds.len().min(limit)).collect();
    }
    let mut scored: Vec<(f32, usize)> = Vec::new();
    for (i, c) in cmds.iter().enumerate() {
        let (title, path) = (c.title.to_lowercase(), c.path.to_lowercase());
        let mut score = 0.0;
        let mut all = true;
        for w in &words {
            let s = if title.starts_with(w.as_str()) {
                4.0
            } else if title.split(|ch: char| !ch.is_alphanumeric()).any(|t| t.starts_with(w.as_str())) {
                3.0
            } else if title.contains(w.as_str()) {
                2.0
            } else if path.contains(w.as_str()) {
                0.8
            } else if is_subsequence(w, &title) {
                0.4
            } else {
                all = false;
                break;
            };
            score += s;
        }
        if all {
            // a shorter title is the likelier one
            scored.push((score - title.chars().count() as f32 * 0.01, i));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, i)| i).collect()
}

fn is_subsequence(word: &str, text: &str) -> bool {
    let mut it = text.chars();
    word.chars().all(|c| it.any(|t| t == c))
}

/// The row of a menu command.
pub fn menu_item(c: &MenuCmd) -> Item {
    let sub = if c.checked { format!("● {}", c.path) } else { c.path.clone() };
    Item { target: Target::MenuCmd(c.id), title: c.title.clone(), sub, tag: "Menu", glyph: "list", tint: [0.40, 0.55, 0.90], key: c.key.clone(), cell: c.cell }
}

/// (id, title, keywords, icon)
pub const ACTIONS: &[(&str, &str, &str, &str)] = &[
    ("settings", "Settings", "preferences options system", "settings"),
    ("files", "Files", "folder explorer finder dolphin", "folder"),
    ("terminal", "Terminal", "console shell command line", "terminal"),
    ("screenshot", "Take Screenshot", "capture screen print prtsc", "camera"),
    ("screenshot-region", "Screenshot of a Selection", "capture region area crop", "camera"),
    ("control-center", "Quick Settings", "wifi bluetooth volume brightness", "sliders-horizontal"),
    ("edit-controls", "Edit Controls", "control center tiles customise", "pencil"),
    ("dark-mode", "Toggle Dark Mode", "dark light theme appearance night", "moon"),
    ("wallpaper", "Change Wallpaper", "background desktop picture image", "image"),
    ("lock", "Lock Screen", "lock screen secure", "lock"),
    ("quit", "Quit Session", "log out exit power shutdown", "power"),
];

pub fn actions(query: &str, limit: usize) -> Vec<Item> {
    let q = query.trim().to_lowercase();
    if q.len() < 2 {
        return Vec::new();
    }
    let mut hits: Vec<(i32, &(&str, &str, &str, &str))> = ACTIONS
        .iter()
        .filter_map(|a| {
            let t = a.1.to_lowercase();
            let s = if t.starts_with(&q) {
                80
            } else if t.split_whitespace().any(|w| w.starts_with(&q)) {
                60
            } else if t.contains(&q) {
                40
            } else if a.2.split_whitespace().any(|w| w.starts_with(&q)) {
                25
            } else {
                return None;
            };
            Some((s, a))
        })
        .collect();
    hits.sort_by_key(|h| -h.0);
    hits.into_iter()
        .take(limit)
        .map(|(_, a)| Item { target: Target::Action(a.0), title: a.1.to_string(), sub: String::new(), tag: "Action", glyph: a.3, tint: [0.35, 0.55, 0.95], key: String::new(), cell: None })
        .collect()
}

// ---------------------------------------------------------------------------------------- calculator

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.i < self.s.len() && self.s[self.i] == c {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat(b'*') || self.eat(b'x') {
                v *= self.power()?;
            } else if self.eat(b'/') {
                v /= self.power()?;
            } else if self.eat(b'%') {
                v %= self.power()?;
            } else {
                return Some(v);
            }
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat(b'^') {
            let e = self.power()?;
            return Some(base.powf(e));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<f64> {
        self.ws();
        if self.eat(b'(') {
            let v = self.expr()?;
            return if self.eat(b')') { Some(v) } else { None };
        }
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
                self.i += 1;
            }
            return std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok();
        }
        while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let name = std::str::from_utf8(&self.s[start..self.i]).ok()?;
        match name {
            "pi" => return Some(std::f64::consts::PI),
            "e" => return Some(std::f64::consts::E),
            _ => {}
        }
        if !self.eat(b'(') {
            return None;
        }
        let a = self.expr()?;
        if !self.eat(b')') {
            return None;
        }
        Some(match name {
            "sqrt" => a.sqrt(),
            "sin" => a.sin(),
            "cos" => a.cos(),
            "tan" => a.tan(),
            "ln" => a.ln(),
            "log" => a.log10(),
            "abs" => a.abs(),
            "exp" => a.exp(),
            "round" => a.round(),
            "floor" => a.floor(),
            "ceil" => a.ceil(),
            _ => return None,
        })
    }
}

/// Evaluate an arithmetic expression; `None` when the text is not one (or is just a number).
pub fn calc(query: &str) -> Option<String> {
    let q = query.trim().to_lowercase().replace(',', ".").replace('×', "*").replace('÷', "/");
    if q.is_empty() || !q.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut p = Parser { s: q.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != q.len() || !v.is_finite() {
        return None;
    }
    // a bare number is not a calculation
    if q.parse::<f64>().is_ok() {
        return None;
    }
    let mut out = if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-9) { format!("{v:e}") } else { format!("{v:.10}") };
    if out.contains('.') && !out.contains('e') {
        out = out.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------- files

struct Entry {
    lower: String,
    path: PathBuf,
    dir: bool,
    depth: u8,
}

/// Names of files under the home folder, collected on a background thread.
pub struct FileIndex {
    entries: Arc<Mutex<Vec<Entry>>>,
    busy: Arc<AtomicBool>,
    /// bumped whenever a scan finishes
    done: Arc<AtomicU64>,
    built: Mutex<Option<Instant>>,
}

const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", ".cache", "site-packages", "build", "dist"];
const MAX_ENTRIES: usize = 400_000;
const MAX_DEPTH: u8 = 6;

fn walk(root: &Path, out: &mut Vec<Entry>) {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root.to_path_buf(), 0u8));
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if out.len() >= MAX_ENTRIES {
                return;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let is_dir = ft.is_dir();
            let path = e.path();
            out.push(Entry { lower: name.to_lowercase(), path: path.clone(), dir: is_dir, depth });
            if is_dir && depth + 1 < MAX_DEPTH && !SKIP.contains(&name.as_str()) {
                queue.push_back((path, depth + 1));
            }
        }
    }
}

impl FileIndex {
    pub fn new() -> Arc<FileIndex> {
        let ix = Arc::new(FileIndex { entries: Arc::new(Mutex::new(Vec::new())), busy: Arc::new(AtomicBool::new(false)), done: Arc::new(AtomicU64::new(0)), built: Mutex::new(None) });
        ix.rebuild();
        ix
    }

    /// Start a re-scan unless one is running.
    pub fn rebuild(&self) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (entries, busy, done) = (self.entries.clone(), self.busy.clone(), self.done.clone());
        *self.built.lock().unwrap() = Some(Instant::now());
        std::thread::spawn(move || {
            let mut v = Vec::new();
            walk(&fsops::home(), &mut v);
            *entries.lock().unwrap() = v;
            done.fetch_add(1, Ordering::SeqCst);
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn generation(&self) -> u64 {
        self.done.load(Ordering::SeqCst)
    }

    /// Re-scan when the last one is older than two minutes.
    pub fn refresh_if_stale(&self) {
        let stale = self.built.lock().unwrap().is_none_or(|t| t.elapsed().as_secs() > 120);
        if stale {
            self.rebuild();
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Item> {
        let q = query.trim().to_lowercase();
        if q.len() < 2 {
            return Vec::new();
        }
        let entries = self.entries.lock().unwrap();
        let mut best: Vec<(i32, usize)> = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let s = if e.lower == q {
                100
            } else if e.lower.starts_with(&q) {
                80
            } else if e.lower.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&q)) {
                65
            } else if e.lower.contains(&q) {
                45
            } else {
                continue;
            };
            let s = s * 10 - e.depth as i32 * 3 + if e.dir { 2 } else { 0 };
            best.push((s, i));
        }
        best.sort_by_key(|b| -b.0);
        best.into_iter()
            .take(limit)
            .map(|(_, i)| {
                let e = &entries[i];
                let name = e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let kind = fsops::kind_of(&name, e.dir);
                let parent = e.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
                let home = fsops::home().display().to_string();
                let sub = parent.strip_prefix(&home).map(|r| format!("~{r}")).unwrap_or(parent);
                Item { target: Target::File(e.path.clone()), title: name, sub, tag: kind.label(), glyph: kind.icon(), tint: kind.tint([0.30, 0.55, 0.95]), key: String::new(), cell: None }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------- the rest

pub fn calc_item(answer: String, expr: &str) -> Item {
    Item { target: Target::Calc(answer.clone()), title: answer, sub: format!("{} =", expr.trim()), tag: "Calculator", glyph: "equal", tint: [0.95, 0.60, 0.25], key: String::new(), cell: None }
}

pub fn web_item(query: &str) -> Item {
    Item { target: Target::Web(query.trim().to_string()), title: format!("Search the web for “{}”", query.trim()), sub: String::new(), tag: "Web", glyph: "globe", tint: [0.30, 0.65, 0.95], key: String::new(), cell: None }
}

pub fn run_item(cmd: &str) -> Item {
    Item { target: Target::Run(cmd.trim().to_string()), title: format!("Run {}", cmd.trim()), sub: "Shell command".into(), tag: "Command", glyph: "terminal", tint: [0.50, 0.55, 0.70], key: String::new(), cell: None }
}

/// URL for a web search.
pub fn search_url(q: &str) -> String {
    let mut enc = String::new();
    for b in q.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => enc.push(b as char),
            b' ' => enc.push('+'),
            _ => enc.push_str(&format!("%{b:02X}")),
        }
    }
    format!("https://duckduckgo.com/?q={enc}")
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    fn cmd(id: u32, title: &str, path: &str) -> MenuCmd {
        MenuCmd { id, title: title.into(), path: path.into(), key: String::new(), checked: false, cell: None }
    }

    fn sample() -> Vec<MenuCmd> {
        vec![cmd(1, "Open…", "File"), cmd(2, "Save", "File"), cmd(3, "Save As…", "File"), cmd(4, "Save All", "File"), cmd(5, "Zoom In", "View"), cmd(6, "Reload", "File"), cmd(7, "Закрити вікно", "File")]
    }

    #[test]
    fn no_query_lists_the_menus_in_order() {
        assert_eq!(menu_rank(&sample(), "", 3), [0, 1, 2]);
        assert_eq!(menu_rank(&sample(), "   ", 10).len(), 7);
    }

    #[test]
    fn every_word_has_to_match_and_a_title_start_wins() {
        let c = sample();
        let ranked = menu_rank(&c, "save", 10);
        // the shortest title first; titles as long as each other stay in the order of the menu
        assert_eq!(ranked, [1, 2, 3]);
        assert_eq!(menu_rank(&c, "save as", 10), [2]);
        assert!(menu_rank(&c, "nothing like it", 10).is_empty());
    }

    #[test]
    fn the_path_and_loose_letters_also_find_a_command() {
        let c = sample();
        // "view" is only in the path of Zoom In
        assert_eq!(menu_rank(&c, "view", 10), [4]);
        // the letters in order, a weaker match than a word
        assert!(menu_rank(&c, "sva", 10).contains(&2));
        // other alphabets lower-case too
        assert_eq!(menu_rank(&c, "ЗАКРИТИ", 10), [6]);
    }

    #[test]
    fn limit_is_kept() {
        assert_eq!(menu_rank(&sample(), "s", 2).len(), 2);
    }
}
