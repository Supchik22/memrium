//! How the Launchpad lays its apps out: loose apps and folders, in order. The layout is made from the apps' categories
//! the first time, then the user's changes (a folder made by dropping one app on another, a renamed folder, an app
//! dragged out) are kept in `launchpad.toml` and win over the categories.

use crate::apps::{self, AppEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Slot {
    App { id: String },
    Folder { name: String, ids: Vec<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub slots: Vec<Slot>,
}

fn path() -> std::path::PathBuf {
    crate::theme::config_dir().join("launchpad.toml")
}

impl Layout {
    /// One folder per category that holds at least two apps; the rest stay loose. Apps keep their (name) order.
    pub fn auto(list: &[AppEntry]) -> Layout {
        let mut names: Vec<&'static str> = Vec::new();
        for a in list {
            let g = apps::group_of(a);
            if !names.contains(&g) {
                names.push(g);
            }
        }
        names.sort();
        let (mut slots, mut loose) = (Vec::new(), Vec::new());
        for g in names {
            let ids: Vec<String> = list.iter().filter(|a| apps::group_of(a) == g).map(|a| a.id.clone()).collect();
            if g != "Other" && ids.len() >= 2 {
                slots.push(Slot::Folder { name: g.to_string(), ids });
            } else {
                loose.extend(ids.into_iter().map(|id| Slot::App { id }));
            }
        }
        // folders first, loose apps after them
        slots.extend(loose);
        Layout { slots }
    }

    pub fn load() -> Option<Layout> {
        toml::from_str(&std::fs::read_to_string(path()).ok()?).ok()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(text) = toml::to_string(self) {
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    /// Drop what is no longer installed, dissolve folders left with one app, and add new apps (into the folder their
    /// category already has, otherwise loose at the end). True when something changed.
    pub fn reconcile(&mut self, list: &[AppEntry]) -> bool {
        let before = self.clone();
        let have: HashSet<&str> = list.iter().map(|a| a.id.as_str()).collect();
        for s in &mut self.slots {
            if let Slot::Folder { ids, .. } = s {
                ids.retain(|i| have.contains(i.as_str()));
            }
        }
        self.slots.retain(|s| match s {
            Slot::App { id } => have.contains(id.as_str()),
            Slot::Folder { ids, .. } => !ids.is_empty(),
        });
        self.dissolve_small();
        let placed: HashSet<String> = self.slots.iter().flat_map(|s| match s {
            Slot::App { id } => vec![id.clone()],
            Slot::Folder { ids, .. } => ids.clone(),
        }).collect();
        for a in list.iter().filter(|a| !placed.contains(&a.id)) {
            let g = apps::group_of(a);
            let home = self.slots.iter_mut().find_map(|s| match s {
                Slot::Folder { name, ids } if name == g => Some(ids),
                _ => None,
            });
            match home {
                Some(ids) => ids.push(a.id.clone()),
                None => self.slots.push(Slot::App { id: a.id.clone() }),
            }
        }
        *self != before
    }

    fn dissolve_small(&mut self) {
        let mut out = Vec::with_capacity(self.slots.len());
        for s in self.slots.drain(..) {
            match s {
                Slot::Folder { ids, .. } if ids.len() == 1 => out.push(Slot::App { id: ids.into_iter().next().unwrap() }),
                s => out.push(s),
            }
        }
        self.slots = out;
    }

    /// Drop the app in slot `from` onto slot `onto`: onto an app it makes a folder (named for their shared group, or
    /// "Folder"), onto a folder it joins it. Returns the slot the result sits in. Folders cannot be dropped into folders.
    pub fn merge(&mut self, from: usize, onto: usize, list: &[AppEntry]) -> Option<usize> {
        if from == onto || from >= self.slots.len() || onto >= self.slots.len() {
            return None;
        }
        let Slot::App { id } = self.slots[from].clone() else { return None };
        match self.slots[onto].clone() {
            Slot::Folder { name, mut ids } => {
                ids.push(id);
                self.slots[onto] = Slot::Folder { name, ids };
            }
            Slot::App { id: other } => {
                let group = |i: &str| list.iter().find(|a| a.id == i).map(apps::group_of);
                let name = match (group(&other), group(&id)) {
                    (Some(a), Some(b)) if a == b && a != "Other" => a.to_string(),
                    _ => "Folder".to_string(),
                };
                self.slots[onto] = Slot::Folder { name, ids: vec![other, id] };
            }
        }
        self.slots.remove(from);
        Some(if from < onto { onto - 1 } else { onto })
    }

    /// Move slot `from` so it sits before the slot that is now at `to` (`to` may be one past the end).
    pub fn move_slot(&mut self, from: usize, to: usize) {
        if from >= self.slots.len() {
            return;
        }
        let s = self.slots.remove(from);
        let to = if to > from { to - 1 } else { to };
        self.slots.insert(to.min(self.slots.len()), s);
    }

    /// Take app `id` out of folder `folder` and put it loose right after it. A folder left with one app dissolves.
    pub fn take_out(&mut self, folder: usize, id: &str) {
        let Some(Slot::Folder { ids, .. }) = self.slots.get_mut(folder) else { return };
        let Some(at) = ids.iter().position(|i| i == id) else { return };
        ids.remove(at);
        self.slots.insert(folder + 1, Slot::App { id: id.to_string() });
        self.slots.retain(|s| !matches!(s, Slot::Folder { ids, .. } if ids.is_empty()));
        self.dissolve_small();
    }

    pub fn rename(&mut self, folder: usize, name: &str) {
        if let Some(Slot::Folder { name: n, .. }) = self.slots.get_mut(folder) {
            *n = name.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, cats: &[&str]) -> AppEntry {
        AppEntry { id: id.into(), name: id.into(), generic_name: String::new(), exec: String::new(), icon: String::new(), keywords: String::new(), terminal: false, wm_class: String::new(), categories: cats.iter().map(|s| s.to_string()).collect() }
    }

    fn sample() -> Vec<AppEntry> {
        vec![app("a", &["Network"]), app("b", &["WebBrowser"]), app("c", &["Game"]), app("d", &[]), app("e", &["Graphics"]), app("f", &["Graphics"])]
    }

    #[test]
    fn auto_makes_folders_of_two_or_more_and_leaves_the_rest_loose() {
        let l = Layout::auto(&sample());
        let folders: Vec<_> = l.slots.iter().filter_map(|s| if let Slot::Folder { name, ids } = s { Some((name.as_str(), ids.len())) } else { None }).collect();
        assert_eq!(folders, vec![("Graphics", 2), ("Internet", 2)]);
        assert_eq!(l.slots.len(), 4); // two folders, Game and the uncategorised one loose
    }

    #[test]
    fn merging_makes_a_folder_and_dragging_out_dissolves_it() {
        let list = sample();
        let mut l = Layout { slots: list.iter().map(|a| Slot::App { id: a.id.clone() }).collect() };
        let at = l.merge(4, 5, &list).unwrap();
        assert_eq!(l.slots[at], Slot::Folder { name: "Graphics".into(), ids: vec!["f".into(), "e".into()] });
        assert_eq!(l.slots.len(), 5);
        let at = l.merge(2, at - 0, &list).unwrap();
        let Slot::Folder { ids, .. } = &l.slots[at] else { panic!() };
        assert_eq!(ids.len(), 3);
        l.take_out(at, "c");
        l.take_out(at, "e");
        assert!(matches!(l.slots[at], Slot::App { .. }), "one app left: the folder is gone");
    }

    #[test]
    fn reconcile_forgets_removed_apps_and_files_new_ones_by_category() {
        let mut list = sample();
        let mut l = Layout::auto(&list);
        list.remove(0);
        list.push(app("g", &["Network"]));
        assert!(l.reconcile(&list));
        assert!(!l.slots.iter().any(|s| matches!(s, Slot::Folder { ids, .. } if ids.iter().any(|i| i == "a"))));
        // "Internet" lost "a" and kept only "b": it dissolved, so "g" goes loose
        assert!(l.slots.iter().any(|s| matches!(s, Slot::App { id } if id == "g")));
        assert!(!l.reconcile(&list));
    }

    #[test]
    fn moving_a_slot_puts_it_before_the_target() {
        let mut l = Layout { slots: ["a", "b", "c"].iter().map(|i| Slot::App { id: i.to_string() }).collect() };
        l.move_slot(0, 3);
        assert_eq!(l.slots.iter().map(|s| if let Slot::App { id } = s { id.as_str() } else { "" }).collect::<String>(), "bca");
    }
}
