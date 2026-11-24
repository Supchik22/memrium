//! Desktop widgets that are programs of their own (`memrium_de::deskwidgets`, `kind = "ext:<name>"`).
//!
//! The shell starts the program, knows its window by the app id `memrium.widget.<name>`, and from then on treats the window
//! as a part of the desktop: it sits under every window (and above the shell's own widgets), has no frame, shadow or title,
//! is not in the Dock, Alt-Tab or the overview, is on every desktop, never takes the keyboard, and goes where its entry in
//! `config.desk_widgets` says. Dragging it in edit mode changes the entry; the window follows. The glass card behind it is the
//! shell's (`desk_widgets.rs`), so the program only draws what is on it.
//!
//! In `wins` such a window has `or` set (the flag that already keeps a window out of every list of managed windows) and
//! `desk`, which makes it the lowest instead of the highest: the desk windows are the first ones in `wins`.

use crate::role::Role;
use crate::state::{root_of, Deco, State};
use chebrium::spring::Spring;
use memrium_de::deskwidgets::{self, DeskWidget};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashSet;
use std::time::{Duration, Instant};

impl State {
    pub fn is_desk_surface(&self, s: &WlSurface) -> bool {
        self.win_index(s).is_some_and(|i| self.wins[i].desk)
    }

    /// Where a window that takes the focus goes in `wins`: in front of the managed ones, but behind the desk widgets' windows
    /// that are the first.
    pub fn managed_insert_at(&self) -> usize {
        self.wins.iter().rposition(|o| !o.or).map(|p| p + 1).unwrap_or_else(|| self.wins.iter().take_while(|w| w.desk).count())
    }

    /// The window of the entry `uid`.
    pub fn desk_win_of(&self, uid: u64) -> Option<usize> {
        self.wins.iter().position(|w| w.desk && w.desk_uid == uid)
    }

    /// How big the program made the window of the entry `uid`: the card and the grab follow it, not the manifest.
    pub fn desk_win_size(&self, uid: u64) -> Option<[f32; 2]> {
        let w = &self.wins[self.desk_win_of(uid)?];
        (w.mapped && w.geo[2] > 0.0 && w.geo[3] > 0.0).then(|| [w.geo[2], w.geo[3]])
    }

    /// The desk window at `p` when nothing is above it there.
    pub fn desk_win_under(&self, p: [f32; 2]) -> Option<usize> {
        let (s, _) = self.surface_under(p)?;
        self.win_index(&root_of(&s)).filter(|&i| self.wins[i].desk)
    }

    fn next_desk_uid(&self) -> u64 {
        self.config.desk_widgets.iter().map(|d| d.uid).max().unwrap_or(0) + 1
    }

    /// The uid a new entry of `kind` gets (the built-in widgets have none).
    pub fn new_desk_uid(&self, kind: &str) -> u64 {
        if deskwidgets::is_ext(kind) {
            self.next_desk_uid()
        } else {
            0
        }
    }

    /// Every external entry has a uid of its own (a config written by hand has none, or the same twice).
    fn ensure_desk_uids(&mut self) {
        let mut seen = HashSet::new();
        for k in 0..self.config.desk_widgets.len() {
            if !deskwidgets::is_ext(&self.config.desk_widgets[k].kind) {
                continue;
            }
            let uid = self.config.desk_widgets[k].uid;
            if uid == 0 || !seen.insert(uid) {
                let n = self.next_desk_uid();
                self.config.desk_widgets[k].uid = n;
                seen.insert(n);
                self.cfg_dirty = Some(Instant::now());
            }
        }
    }

    /// A window has just been mapped: if it is a widget's, it becomes a part of the desktop. Returns where it is in `wins` now.
    pub fn adopt_desk_window(&mut self, i: usize) -> usize {
        let Some(name) = deskwidgets::name_of_app_id(&self.wins[i].app_id).map(str::to_string) else { return i };
        if !matches!(self.wins[i].role, Role::Xdg(_)) || self.wins[i].desk {
            return i;
        }
        self.ensure_desk_uids();
        let kind = format!("{}{name}", deskwidgets::EXT);
        let taken: Vec<u64> = self.wins.iter().filter(|w| w.desk).map(|w| w.desk_uid).collect();
        let uid = match self.config.desk_widgets.iter().find(|d| d.kind == kind && !taken.contains(&d.uid)) {
            Some(d) => d.uid,
            None => {
                // started by hand (from a terminal): it is put on the desktop as if it had been picked from the list
                let uid = self.next_desk_uid();
                let [x, y] = self.desk_free_place(&kind);
                self.config.desk_widgets.push(DeskWidget { kind, uid, x, y, ..Default::default() });
                self.cfg_dirty = Some(Instant::now());
                uid
            }
        };
        let mut w = self.wins.remove(i);
        w.desk = true;
        w.or = true;
        w.desk_uid = uid;
        w.deco = Deco::Legacy;
        w.ins = [0.0; 4];
        w.stamp = 0;
        w.minimized = false;
        // it is there at once: no pop-in, no lift
        w.open_k = Spring::new(1.0, 240.0, 0.74);
        w.scale = Spring::new(1.0, 320.0, 0.45);
        let at = self.wins.iter().take_while(|o| o.desk).count();
        self.wins.insert(at, w);
        self.place_desk_window(at, true);
        eprintln!("desktop widget {name}: its window is on the desktop");
        at
    }

    /// A place for a new widget of `kind`: the middle of the desktop, or the next free place from there.
    pub fn desk_free_place(&self, kind: &str) -> [f32; 2] {
        let w = self.world;
        let mut at = [(w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0 - 40.0];
        let mut put = deskwidgets::place_at(kind, at, w);
        for _ in 0..16 {
            put = deskwidgets::place_at(kind, at, w);
            if !self.config.desk_widgets.iter().any(|d| (d.x - put[0]).abs() < 24.0 && (d.y - put[1]).abs() < 24.0) {
                break;
            }
            at = [at[0] + 32.0, at[1] + 32.0];
        }
        put
    }

    /// The window `wins[i]` goes where its entry says (at once: the entry is what the pointer is moving).
    fn place_desk_window(&mut self, i: usize, force: bool) {
        let uid = self.wins[i].desk_uid;
        let Some(&DeskWidget { x, y, .. }) = self.config.desk_widgets.iter().find(|d| d.uid == uid && deskwidgets::is_ext(&d.kind)) else { return };
        let w = &mut self.wins[i];
        if force || w.x.target != x || w.y.target != y || w.x.x != x || w.y.x != y {
            w.x.target = x;
            w.y.target = y;
            w.x.snap();
            w.y.snap();
            w.jprev = [x, y];
        }
    }

    /// Each pass of the shell's loop: start the programs that are wanted, put their windows where they belong, close those of
    /// entries that were taken off the desktop.
    pub fn update_desk_apps(&mut self) {
        let any = self.config.desk_widgets.iter().any(|d| deskwidgets::is_ext(&d.kind));
        if !any && self.desk_procs.is_empty() && !self.wins.iter().any(|w| w.desk) {
            return;
        }
        self.ensure_desk_uids();
        let wanted: Vec<(u64, String)> = self.config.desk_widgets.iter().filter(|d| deskwidgets::is_ext(&d.kind)).map(|d| (d.uid, d.kind.clone())).collect();
        let uids: HashSet<u64> = wanted.iter().map(|(u, _)| *u).collect();

        // entries that were removed: the window closes, and the program we started goes
        for w in self.wins.iter().filter(|w| w.desk && !uids.contains(&w.desk_uid)) {
            w.role.close();
        }
        let gone: Vec<u64> = self.desk_procs.keys().filter(|u| !uids.contains(u)).copied().collect();
        for u in gone {
            if let Some(mut child) = self.desk_procs.remove(&u) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.desk_gone.retain(|u| uids.contains(u));

        // windows follow their entries
        for i in 0..self.wins.len() {
            if self.wins[i].desk && uids.contains(&self.wins[i].desk_uid) {
                self.place_desk_window(i, false);
            }
        }

        // a program that stopped is not started again (a crash must not become a loop): taking the widget off and putting it on is
        // the way to start it anew
        if self.desk_check.map_or(true, |t| t.elapsed() > Duration::from_secs(1)) {
            self.desk_check = Some(Instant::now());
            let mut ended = Vec::new();
            for (u, child) in self.desk_procs.iter_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    eprintln!("desktop widget {u}: the program ended ({status})");
                    ended.push(*u);
                }
            }
            for u in ended {
                self.desk_procs.remove(&u);
                self.desk_gone.insert(u);
            }
        }

        for (uid, kind) in wanted {
            if self.desk_win_of(uid).is_some() || self.desk_procs.contains_key(&uid) || self.desk_gone.contains(&uid) {
                continue;
            }
            // a window of this widget that is up already (started by hand) will be bound when it maps
            let Some(ext) = deskwidgets::ext_of(&kind) else {
                eprintln!("desktop widget {kind}: no manifest for it (looked in {:?})", deskwidgets::ext_dirs());
                self.desk_gone.insert(uid);
                continue;
            };
            match self.spawn_desk_app(&ext) {
                Ok(child) => {
                    eprintln!("desktop widget {}: started {}", ext.name, ext.exec);
                    self.desk_procs.insert(uid, child);
                }
                Err(e) => {
                    eprintln!("desktop widget {}: cannot start {}: {e}", ext.name, ext.exec);
                    self.desk_gone.insert(uid);
                }
            }
        }
    }

    /// The program of a widget: a name next to the shell's own file, or on PATH.
    fn spawn_desk_app(&self, ext: &deskwidgets::Ext) -> std::io::Result<std::process::Child> {
        let mut words = ext.exec.split_whitespace();
        let prog = words.next().unwrap_or(&ext.exec);
        let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(prog))).filter(|p| !prog.contains('/') && p.exists());
        let mut cmd = std::process::Command::new(beside.unwrap_or_else(|| prog.into()));
        cmd.args(words).env("WAYLAND_DISPLAY", &self.socket).env("XDG_SESSION_TYPE", "wayland").env("MEMRIUM_WIDGET", &ext.name);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        cmd.spawn()
    }
}

impl State {
    /// The frames and guide lines of the desktop widgets that go over their windows, as a layer of the window stack.
    pub fn desk_over_item(&mut self) -> Option<memrium_de::renderer::Item<'static>> {
        let (mut panels, labels) = self.desk_overlay();
        if panels.is_empty() && labels.is_empty() {
            return None;
        }
        panels.extend(self.renderer.label_panels(&labels));
        Some(memrium_de::renderer::Item::Layer { panels, bbox: self.world })
    }
}
