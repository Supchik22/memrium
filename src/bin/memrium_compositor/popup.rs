//! The cards that open from a bar icon and go when the pointer clicks anywhere else: power and battery, the calendar,
//! the sound mixer and the networks. This file is what they share (where one sits, how it opens and closes, the glass
//! it is drawn on, the Chebrium widgets it is made of); each kind has its own file for what it shows and does.
//!
//! A card is drawn the way a Settings page is: every frame, with `Ui` widgets, so a switch, a slider or a segmented
//! picker looks and moves exactly as it does there. What the widgets did comes back as `Act`s, done after drawing.

use crate::state::State;
use chebrium::spring::Spring;
use chebrium::style::Style;
use chebrium::ui::Ui;
use memrium_de::renderer::{Label, Panel, PanelDraw};
use memrium_de::theme::Edge;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PopKind {
    Power,
    Calendar,
    Audio,
    Network,
    Notifications,
    Clipboard,
    Bluetooth,
}

/// What a card asks the shell to do.
#[derive(Clone, Debug)]
pub enum Act {
    SetProfile(String),
    ToggleBlock,
    /// a PipeWire node (a device or an app's stream) to this volume
    NodeVolume(u32, f32),
    DefaultSink(u32),
    ToggleMuteDefault,
    WifiPower(bool),
    /// leave / join a network connection by name, or a Wi-Fi network by name
    Disconnect(String),
    ConnectWifi(String),
    ConnectWired(String),
    /// join a secured Wi-Fi network with a password; the answer goes into the shared cell (empty text = joined)
    /// connect to a network that is saved or open; the answer goes into the shared cell like `JoinWifi`'s
    TryWifi(String, std::sync::Arc<std::sync::Mutex<Option<String>>>),
    JoinWifi(String, String, std::sync::Arc<std::sync::Mutex<Option<String>>>),
    NotifDismiss(u32),
    NotifOpen(u32),
    NotifClear,
    ToggleDnd,
    ClipPaste(u64),
    ClipPin(u64),
    ClipDelete(u64),
    ClipClear,
    /// run `bluetoothctl` with these arguments (connect, disconnect, remove, power, scan)
    Bluetooth(Vec<String>),
    /// pair with a device, trust it and connect
    BluetoothPair(String),
}

pub struct Popup {
    pub kind: PopKind,
    /// top-left corner on screen, and the size the kind asked for
    pub pos: [f32; 2],
    pub size: [f32; 2],
    /// the bar's edge the card hangs from, and whether the card is above it (so it stays put as it grows)
    pub anchor_y: f32,
    pub above: bool,
    pub k: Spring,
    pub closing: bool,
    pub ui: chebrium::ui::State,
    pub cal: crate::calendar_panel::Cal,
    pub audio: crate::audio_panel::Audio,
    pub net: crate::network_panel::Net,
    pub notif: crate::notif_panel::NotifPanel,
    pub clip: crate::clip_panel::ClipPanel,
    pub bt: crate::bluetooth_panel::Bt,
}

impl State {
    /// How big the card of this kind is right now.
    fn card_size(&self, kind: PopKind) -> [f32; 2] {
        match kind {
            PopKind::Power => self.power_size(),
            PopKind::Calendar => crate::calendar_panel::SIZE,
            PopKind::Audio => crate::audio_panel::SIZE,
            PopKind::Network => crate::network_panel::SIZE,
            PopKind::Notifications => crate::notif_panel::SIZE,
            PopKind::Clipboard => crate::notif_panel::SIZE,
            PopKind::Bluetooth => crate::bluetooth_panel::SIZE,
        }
    }

    /// Open the card of `kind` under (or over) the bar icon clicked at `x` on `bar`; a click on the icon of the card that
    /// is open closes it.
    pub fn toggle_card(&mut self, kind: PopKind, bar: Option<usize>, x: f32) {
        if self.card.as_ref().is_some_and(|p| p.kind == kind && !p.closing) {
            self.close_card();
            return;
        }
        let size = self.card_size(kind);
        let (o, m) = (self.mon_origin(self.bar_mon.get()), self.mons.get(self.bar_mon.get()).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]));
        let anchor = bar.and_then(|b| self.with_bar_ctx(|c| self.bars.get(b).map(|bar| (bar.rect(c), bar.cfg.edge))));
        let mut px = (x - size[0] / 2.0).clamp(o[0] + 8.0, (o[0] + m[2] - size[0] - 8.0).max(o[0] + 8.0));
        let (anchor_y, above) = match anchor {
            Some((r, Edge::Bottom)) => (o[1] + r[1] - 10.0, true),
            // from a bar at the side the card opens beside the icon (and stays where it was put)
            Some((r, edge)) if edge.vertical() => {
                px = o[0] + if edge == Edge::Left { r[0] + r[2] + 10.0 } else { r[0] - size[0] - 10.0 };
                ((self.mouse[1] - size[1] / 2.0).clamp(o[1] + 46.0, (o[1] + m[3] - size[1] - 8.0).max(o[1] + 46.0)), false)
            }
            Some((r, _)) => (o[1] + r[1] + r[3] + 10.0, false),
            None => (o[1] + 46.0, false),
        };
        let mut k = Spring::new(0.0, 420.0, 0.8);
        k.target = 1.0;
        let py = if above { anchor_y - size[1] } else { anchor_y };
        let mut pop = Popup { kind, pos: [px, py], size, anchor_y, above, k, closing: false, ui: Default::default(), cal: Default::default(), audio: Default::default(), net: Default::default(), notif: Default::default(), clip: Default::default(), bt: Default::default() };
        match kind {
            PopKind::Calendar => pop.cal.reset_to_today(),
            PopKind::Audio => pop.audio.wake(),
            PopKind::Network => pop.net.wake(),
            PopKind::Bluetooth => pop.bt.wake(),
            PopKind::Power | PopKind::Notifications | PopKind::Clipboard => {}
        }
        pop.ui.pointer_move(self.mouse);
        self.card = Some(pop);
    }

    pub fn close_card(&mut self) {
        if let Some(p) = &mut self.card {
            p.closing = true;
            p.k.target = 0.0;
        }
    }

    pub fn card_hit(&self, p: [f32; 2]) -> bool {
        self.card.as_ref().is_some_and(|pop| !pop.closing && p[0] >= pop.pos[0] && p[0] <= pop.pos[0] + pop.size[0] && p[1] >= pop.pos[1] && p[1] <= pop.pos[1] + pop.size[1])
    }

    /// The pointer went down: true when a card took the click; a click elsewhere closes it and carries on.
    pub fn card_press(&mut self, p: [f32; 2]) -> bool {
        if !self.card.as_ref().is_some_and(|pop| !pop.closing) {
            return false;
        }
        if !self.card_hit(p) {
            self.close_card();
            return false;
        }
        if let Some(pop) = &mut self.card {
            pop.ui.pointer_down(p);
        }
        true
    }

    pub fn card_release(&mut self) {
        if let Some(p) = &mut self.card {
            p.ui.pointer_up();
        }
    }

    pub fn card_motion(&mut self, p: [f32; 2]) {
        if let Some(pop) = &mut self.card {
            pop.ui.pointer_move(p);
        }
    }

    /// The wheel over a card: true when it was used.
    pub fn card_scroll(&mut self, dy: f64) -> bool {
        if !self.card_hit(self.mouse) {
            return false;
        }
        if let Some(pop) = &mut self.card {
            pop.ui.pointer_scroll(dy as f32 * 2.5);
        }
        true
    }

    pub fn update_card(&mut self, dt: f32) {
        let an = self.theme.animations.clone();
        let Some(p) = &mut self.card else { return };
        an.step("popups", &mut p.k, dt);
        if p.closing && p.k.x < 0.01 && p.k.at_rest() {
            self.card = None;
        }
    }

    /// Draw the open card (and do what its widgets asked for).
    pub fn card_panels(&mut self, panels: &mut Vec<Panel>, labels: &mut Vec<Label>) {
        let Some(mut pop) = self.card.take() else { return };
        let kind = pop.kind;
        if kind == PopKind::Power {
            pop.size = self.power_size();
        }
        pop.pos[1] = if pop.above { pop.anchor_y - pop.size[1] } else { pop.anchor_y };
        let k = pop.k.x.clamp(0.0, 1.0);
        let pos = [pop.pos[0], pop.pos[1] + (1.0 - k) * 8.0];
        let mut p: Vec<Panel> = Vec::new();
        let mut l: Vec<Label> = Vec::new();
        if kind == PopKind::Clipboard {
            self.clip_prepare();
        }
        let dt = pop.ui.begin_frame();
        let inert = pop.closing || k < 0.6;
        let mut acts = Vec::new();
        let edit_seen;
        {
            let Popup { ui: ui_state, cal, audio, net, notif, clip, bt, size, .. } = &mut pop;
            p.push(self.card_frame(pos, *size, k, 22.0));
            let mut painter = PanelDraw::new(&mut p, &mut l);
            let mut ui = Ui::new(&mut painter, ui_state, Style::new(self.theme.dark, self.theme.accent).with_controls(self.theme.ui.controls.to_chebrium()), dt);
            // a card that is still opening or closing takes no clicks
            if inert {
                ui.click = false;
            }
            match kind {
                PopKind::Power => self.power_build(&mut ui, pos, &mut acts),
                PopKind::Calendar => self.calendar_build(&mut ui, pos, cal),
                PopKind::Audio => self.audio_build(&mut ui, pos, audio, &mut acts),
                PopKind::Network => self.network_build(&mut ui, pos, net, &mut acts),
                PopKind::Notifications => self.notif_build(&mut ui, pos, notif, &mut acts),
                PopKind::Clipboard => self.clip_build(&mut ui, pos, clip, &mut acts),
                PopKind::Bluetooth => self.bluetooth_build(&mut ui, pos, bt, &mut acts),
            }
            edit_seen = ui.edit_seen;
        }
        pop.ui.end_frame(edit_seen);
        // the whole card fades with its opening (the frame, at index 0, already does)
        p[1..].iter_mut().for_each(|x| x.fade_by(k));
        l.iter_mut().for_each(|x| x.opacity *= k);
        panels.extend(p);
        if kind == PopKind::Clipboard {
            self.clip_thumb_panels(&pop.clip, k, panels);
        }
        labels.extend(l);
        self.card = Some(pop);
        for a in acts {
            self.card_act(a);
        }
        match kind {
            PopKind::Notifications => self.notif_seen(),
            PopKind::Audio => self.audio_tick(),
            PopKind::Network => self.network_tick(),
            PopKind::Bluetooth => self.bluetooth_tick(),
            _ => {}
        }
    }

    fn card_act(&mut self, a: Act) {
        use memrium_de::sysctl::Cmd;
        match a {
            Act::SetProfile(name) => {
                self.sys.profile = Some(name.clone());
                self.sys_watcher.send(Cmd::SetProfile(name));
            }
            Act::ToggleBlock => self.manual_block = !self.manual_block,
            Act::NodeVolume(id, v) => self.sys_watcher.send(Cmd::SetNodeVolume(id, v)),
            Act::DefaultSink(id) => self.sys_watcher.send(Cmd::SetDefaultSink(id)),
            Act::ToggleMuteDefault => self.sys_watcher.send(Cmd::ToggleMute),
            Act::WifiPower(on) => self.sys_watcher.send(Cmd::SetWifi(on)),
            Act::Disconnect(name) => self.sys_watcher.send(Cmd::Net(vec!["connection".into(), "down".into(), "id".into(), name])),
            Act::ConnectWifi(ssid) => self.sys_watcher.send(Cmd::Net(vec!["device".into(), "wifi".into(), "connect".into(), ssid])),
            Act::NotifDismiss(id) => self.notifications.forget(id),
            Act::NotifOpen(id) => {
                if self.notifications.history.iter().any(|s| s.n.id == id && s.n.actions.iter().any(|(k, _)| k == "default")) {
                    self.emit_action(id, "default");
                }
                self.notifications.forget(id);
            }
            Act::ClipPaste(id) => {
                self.close_card();
                self.clip_paste(id);
            }
            Act::ClipPin(id) => self.clipboard.toggle_pin(id),
            Act::ClipDelete(id) => self.clipboard.delete(id),
            Act::Bluetooth(args) => {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("bluetoothctl").args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
                });
            }
            Act::BluetoothPair(mac) => {
                std::thread::spawn(move || {
                    let ctl = |a: &[&str]| std::process::Command::new("bluetoothctl").args(a).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
                    if ctl(&["pair", &mac]) {
                        ctl(&["trust", &mac]);
                        ctl(&["connect", &mac]);
                    }
                });
            }
            Act::ClipClear => self.clipboard.clear(),
            Act::NotifClear => self.notifications.clear(),
            Act::ToggleDnd => {
                self.config.notifications.dnd = !self.config.notifications.dnd;
                self.cfg_dirty = Some(std::time::Instant::now());
            }
            Act::TryWifi(ssid, result) => {
                std::thread::spawn(move || {
                    let out = std::process::Command::new("nmcli").args(["--wait", "25", "device", "wifi", "connect", &ssid]).stdin(std::process::Stdio::null()).output();
                    let answer = match out {
                        Ok(o) if o.status.success() => String::new(),
                        Ok(o) => {
                            let text = String::from_utf8_lossy(&o.stderr).to_string();
                            let first = text.lines().next().unwrap_or("").trim().trim_start_matches("Error: ").to_string();
                            if first.is_empty() { "Could not connect".to_string() } else { first }
                        }
                        Err(e) => e.to_string(),
                    };
                    if let Ok(mut g) = result.lock() {
                        *g = Some(answer);
                    }
                });
            }
            Act::JoinWifi(ssid, password, result) => {
                std::thread::spawn(move || {
                    // a profile saved by another desktop keeps its password with that desktop's agent (`psk-flags` 1);
                    // rewrite it so NetworkManager owns the password, then bring it up
                    let nm = |args: &[&str]| std::process::Command::new("nmcli").args(args).stdin(std::process::Stdio::null()).output();
                    let owned = nm(&["connection", "modify", "id", &ssid, "wifi-sec.psk-flags", "0", "wifi-sec.psk", &password]).is_ok_and(|o| o.status.success());
                    let out = if owned { nm(&["--wait", "25", "connection", "up", "id", &ssid]) } else { nm(&["--wait", "25", "device", "wifi", "connect", &ssid, "password", &password]) };
                    let answer = match out {
                        Ok(o) if o.status.success() => String::new(),
                        Ok(o) => {
                            let text = String::from_utf8_lossy(&o.stderr).to_string();
                            let first = text.lines().next().unwrap_or("").trim().trim_start_matches("Error: ").to_string();
                            if first.is_empty() { "Could not connect".to_string() } else { first }
                        }
                        Err(e) => e.to_string(),
                    };
                    if let Ok(mut g) = result.lock() {
                        *g = Some(answer);
                    }
                });
            }
            Act::ConnectWired(name) => self.sys_watcher.send(Cmd::Net(vec!["connection".into(), "up".into(), "id".into(), name])),
        }
    }

    /// The card's glass, `pos`/`size` as laid out.
    pub fn card_frame(&self, pos: [f32; 2], size: [f32; 2], k: f32, radius: f32) -> Panel {
        let dark = self.theme.dark;
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let tint = if dark { [0.09, 0.09, 0.09, 0.84] } else { [0.97, 0.98, 1.0, 0.82] };
        let mut p = self.theme.glass.panel(pos[0] + size[0] / 2.0, pos[1] + size[1] / 2.0, size[0], size[1], if windows { (radius * 0.45).max(8.0) } else { radius }).tinted([tint[0], tint[1], tint[2], tint[3] * k]);
        p.extra[0] *= k;
        p.fx[0] = p.fx[0].max(26.0);
        if windows {
            p.params[3] = 0.0;
            p.fx = [p.fx[0].max(32.0), 0.0, 0.0, 0.0];
            p.color2 = if dark { [1.0, 1.0, 1.0, 0.10 * k] } else { [0.0, 0.0, 0.0, 0.10 * k] };
        }
        p.fade_by(k);
        p
    }
}
