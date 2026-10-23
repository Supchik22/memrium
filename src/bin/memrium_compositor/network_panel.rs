//! The networks card of the Wi-Fi icon: the Wi-Fi switch, the connections that are up (with the speed they carry) and
//! the Wi-Fi networks in reach, to connect to or leave. NetworkManager is read and driven with `nmcli`.

use crate::popup::Act;
use crate::state::State;
use chebrium::draw::Align;
use chebrium::ui::Ui;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub const SIZE: [f32; 2] = [440.0, 500.0];
const PAD: f32 = 18.0;
const ROW: f32 = 60.0;
/// the room a password field takes under a network
const ASK: f32 = 52.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conn {
    pub name: String,
    pub device: String,
    /// "wired", "wifi" or "vpn"
    pub kind: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ap {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    pub active: bool,
    pub saved: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetData {
    pub active: Vec<Conn>,
    pub aps: Vec<Ap>,
    pub wifi_on: bool,
    pub has_wifi: bool,
    /// saved VPN profiles that are not connected
    pub vpns: Vec<String>,
}

/// The fields of one line of `nmcli -t` (a colon inside a value is written `\:`).
pub fn split_terse(line: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let mut it = line.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => cur.extend(it.next()),
            ':' => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The VPN profiles (OpenVPN, WireGuard, the other plug-ins) of `nmcli -t -f NAME,TYPE connection show`.
pub fn parse_vpns(out: &str) -> Vec<String> {
    out.lines().filter_map(|l| { let f = split_terse(l); matches!(f.get(1).map(String::as_str), Some("vpn") | Some("wireguard")).then(|| f[0].clone()) }).collect()
}

pub fn parse_active(out: &str) -> Vec<Conn> {
    out.lines()
        .filter_map(|l| {
            let f = split_terse(l);
            let kind = match f.get(1)?.as_str() {
                "802-3-ethernet" => "wired",
                "802-11-wireless" => "wifi",
                "vpn" | "wireguard" => "vpn",
                _ => return None,
            };
            Some(Conn { name: f[0].clone(), device: f.get(2).cloned().unwrap_or_default(), kind })
        })
        .collect()
}

/// `IN-USE,SSID,SIGNAL,SECURITY` lines: one entry per network (the strongest of its access points), best first.
pub fn parse_aps(out: &str, saved: &[String]) -> Vec<Ap> {
    let mut by: HashMap<String, Ap> = HashMap::new();
    for l in out.lines() {
        let f = split_terse(l);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let signal = f[2].parse().unwrap_or(0);
        let ap = Ap { ssid: f[1].clone(), signal, secure: !f[3].trim().is_empty() && f[3] != "--", active: f[0].trim() == "*", saved: saved.contains(&f[1]) };
        let e = by.entry(f[1].clone()).or_insert_with(|| ap.clone());
        e.active |= ap.active;
        e.signal = e.signal.max(signal);
    }
    let mut v: Vec<Ap> = by.into_values().collect();
    v.sort_by(|a, b| b.active.cmp(&a.active).then(b.signal.cmp(&a.signal)).then(a.ssid.cmp(&b.ssid)));
    v
}

fn fetch() -> NetData {
    let run = |args: &[&str]| std::process::Command::new("nmcli").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string());
    let active = run(&["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"]).map(|o| parse_active(&o)).unwrap_or_default();
    let saved: Vec<String> = run(&["-t", "-f", "NAME,TYPE", "connection", "show"]).map(|o| o.lines().filter_map(|l| { let f = split_terse(l); (f.get(1).map(String::as_str) == Some("802-11-wireless")).then(|| f[0].clone()) }).collect()).unwrap_or_default();
    let has_wifi = run(&["-t", "-f", "TYPE", "device"]).is_some_and(|o| o.lines().any(|l| l.trim() == "wifi"));
    let wifi_on = run(&["radio", "wifi"]).is_some_and(|o| o.trim() == "enabled");
    let aps = if wifi_on { run(&["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"]).map(|o| parse_aps(&o, &saved)).unwrap_or_default() } else { Vec::new() };
    let vpns = run(&["-t", "-f", "NAME,TYPE", "connection", "show"]).map(|o| parse_vpns(&o)).unwrap_or_default().into_iter().filter(|v| !active.iter().any(|c| &c.name == v)).collect();
    NetData { active, aps, wifi_on, has_wifi, vpns }
}

fn rate(b: f64) -> String {
    if b >= 1048576.0 { format!("{:.1} MB/s", b / 1048576.0) } else if b >= 1024.0 { format!("{:.0} KB/s", b / 1024.0) } else { format!("{b:.0} B/s") }
}

fn read_bytes(dev: &str, which: &str) -> Option<u64> {
    std::fs::read_to_string(format!("/sys/class/net/{dev}/statistics/{which}_bytes")).ok()?.trim().parse().ok()
}

#[derive(Default)]
pub struct Net {
    data: NetData,
    rx: Option<Receiver<NetData>>,
    next: Option<Instant>,
    /// bytes seen at the last look per device, and the speed since (down, up)
    prev: HashMap<String, (u64, u64, Instant)>,
    speed: HashMap<String, (f64, f64)>,
    scroll: f32,
    /// the secured network whose password is being asked, and what has been typed
    entering: Option<String>,
    password: String,
    /// the answer of the attempt that is running (empty = joined, else why not), and the network it was for
    attempt: Option<(String, std::sync::Arc<std::sync::Mutex<Option<String>>>)>,
    /// a network that did not accept the password, and what NetworkManager said
    failed: Option<(String, String)>,
}

impl Net {
    pub fn wake(&mut self) {
        self.next = None;
    }

    /// Look again soon: something was just connected or left.
    fn soon(&mut self) {
        self.next = Some(Instant::now() + Duration::from_millis(1200));
    }
}

impl State {
    pub fn network_build(&self, ui: &mut Ui, pos: [f32; 2], n: &mut Net, acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let (w, h) = (SIZE[0], SIZE[1]);
        ui.glyph(x + PAD + 8.0, y + 29.0, 18.0, if n.data.wifi_on || !n.data.has_wifi { "wifi" } else { "wifi-off" }, ink, 0.85);
        ui.text(x + PAD + 26.0, y + 35.0, "Networks", 15.0, true, ink, 1.0, Align::Left);
        if n.data.has_wifi {
            let mut on = n.data.wifi_on;
            ui.toggle(x + w - PAD - 44.0, y + 16.0, &mut on);
            if on != n.data.wifi_on {
                n.data.wifi_on = on;
                if !on {
                    n.data.aps.clear();
                }
                acts.push(Act::WifiPower(on));
                n.soon();
            }
        }
        let list = [x, y + 58.0, w, h - 58.0 - 8.0];
        let connected: Vec<Conn> = n.data.active.clone();
        let others: Vec<Ap> = n.data.aps.iter().filter(|a| !a.active || !connected.iter().any(|c| c.kind == "wifi" && c.name == a.ssid)).cloned().collect();
        let asking = n.entering.as_ref().is_some_and(|s| others.iter().any(|a| &a.ssid == s));
        let content = (if connected.is_empty() { 0.0 } else { 34.0 + ROW * connected.len() as f32 }) + (if n.data.vpns.is_empty() { 0.0 } else { 34.0 + ROW * n.data.vpns.len() as f32 }) + 34.0 + ROW * others.len().max(1) as f32 + if asking { ASK } else { 0.0 };
        n.scroll = (n.scroll + ui.scroll).clamp(0.0, (content - list[3] + 8.0).max(0.0));
        ui.d.push_clip(list);
        let mut cy = list[1] - n.scroll;
        let head = |ui: &mut Ui, cy: f32, title: &str| {
            let tw = ui.measure(title, 12.5, true);
            ui.text(x + PAD, cy + 22.0, title, 12.5, true, ink, 0.6, Align::Left);
            ui.fill([x + PAD + tw + 10.0, cy + 17.0, w - 2.0 * PAD - tw - 10.0, 1.0], 0.5, ink, 0.14);
        };
        let visible = |cy: f32| cy + ROW > list[1] && cy < list[1] + list[3];
        if !connected.is_empty() {
            head(ui, cy, "Connected");
            cy += 34.0;
            for c in connected.iter() {
                if visible(cy) {
                    let icon = match c.kind { "wired" => "monitor", "wifi" => "wifi", _ => "shield-check" };
                    ui.glyph(x + PAD + 20.0, cy + 30.0, 24.0, icon, ink, 0.9);
                    let title = if c.kind == "wifi" && !c.device.is_empty() { format!("{} ({})", c.name, c.device) } else { c.name.clone() };
                    ui.text_raw(x + PAD + 48.0, cy + 26.0, &title, 13.5, false, ink, 0.95, Align::Left);
                    let (down, up) = n.speed.get(&c.device).copied().unwrap_or((0.0, 0.0));
                    ui.text_raw(x + PAD + 48.0, cy + 44.0, &format!("↓ {}   ↑ {}", rate(down), rate(up)), 11.5, false, ink, 0.55, Align::Left);
                    let b = [x + w - PAD - 104.0, cy + 15.0, 104.0, 30.0];
                    if ui.button(b, "Disconnect", false) && ui.hit(list) {
                        acts.push(Act::Disconnect(c.name.clone()));
                        n.soon();
                    }
                }
                cy += ROW;
            }
        }
        if !n.data.vpns.is_empty() {
            head(ui, cy, "VPN");
            cy += 34.0;
            for v in n.data.vpns.clone() {
                if visible(cy) {
                    ui.glyph(x + PAD + 20.0, cy + 30.0, 24.0, "shield", ink, 0.9);
                    ui.text_raw(x + PAD + 48.0, cy + 26.0, &v, 13.5, false, ink, 0.95, Align::Left);
                    ui.text(x + PAD + 48.0, cy + 44.0, "Not connected", 11.5, false, ink, 0.55, Align::Left);
                    let b = [x + w - PAD - 104.0, cy + 15.0, 104.0, 30.0];
                    if ui.button(b, "Connect", false) && ui.hit(list) {
                        acts.push(Act::ConnectWired(v.clone()));
                        n.soon();
                    }
                }
                cy += ROW;
            }
        }
        head(ui, cy, "Available");
        cy += 34.0;
        if !n.data.has_wifi {
            ui.text(x + w / 2.0, cy + 30.0, "No Wi-Fi adapter", 13.0, false, ink, 0.5, Align::Center);
        } else if !n.data.wifi_on {
            ui.text(x + w / 2.0, cy + 30.0, "Wi-Fi is off", 13.0, false, ink, 0.5, Align::Center);
        } else if others.is_empty() {
            ui.text(x + w / 2.0, cy + 30.0, "No networks in reach", 13.0, false, ink, 0.5, Align::Center);
        }
        for ap in others.iter() {
            if visible(cy) {
                ui.glyph(x + PAD + 20.0, cy + 30.0, 24.0, "wifi", ink, 0.35 + 0.65 * (ap.signal as f32 / 100.0));
                if ap.secure {
                    ui.glyph(x + PAD + 30.0, cy + 40.0, 11.0, "lock", ink, 0.9);
                }
                ui.text_raw(x + PAD + 48.0, cy + 26.0, &ap.ssid, 13.5, false, ink, 0.95, Align::Left);
                let sub = if ap.saved { "Saved" } else if ap.secure { "Needs a password" } else { "Open" };
                ui.text_raw(x + PAD + 48.0, cy + 44.0, &format!("{}  ·  {}%", memrium_de::i18n::tr(sub), ap.signal), 11.5, false, ink, 0.55, Align::Left);
                let b = [x + w - PAD - 104.0, cy + 15.0, 104.0, 30.0];
                let trying = n.attempt.as_ref().is_some_and(|(s, _)| s == &ap.ssid);
                if trying {
                    ui.fill(b, 8.0, ink, 0.05);
                    ui.text(b[0] + b[2] / 2.0, b[1] + 19.5, "Connecting…", 12.5, false, ink, 0.5, Align::Center);
                } else if ap.saved || !ap.secure {
                    if ui.button(b, "Connect", false) && ui.hit(list) {
                        // a saved network: NetworkManager may want its password from an agent that this desktop does not
                        // have; the answer tells (see `network_tick`)
                        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                        n.attempt = Some((ap.ssid.clone(), result.clone()));
                        n.failed = None;
                        acts.push(Act::TryWifi(ap.ssid.clone(), result));
                    }
                } else if ui.button(b, "Connect", false) && ui.hit(list) {
                    // a secured network that is new: its password first
                    n.entering = if n.entering.as_ref() == Some(&ap.ssid) { None } else { Some(ap.ssid.clone()) };
                    n.password.clear();
                    n.failed = None;
                }
                if let Some((s, why)) = &n.failed {
                    if s == &ap.ssid {
                        let why = memrium_de::i18n::tr(if why.is_empty() { "Could not connect: check the password" } else { why }).into_owned();
                        ui.text_raw(x + PAD + 48.0, cy + 58.0 - 2.0, &why, 11.0, false, [0.95, 0.45, 0.35], 0.95, Align::Left);
                    }
                }
            }
            cy += ROW;
            // the password of the network that was chosen
            if n.entering.as_ref() == Some(&ap.ssid) {
                if visible(cy) {
                    let fr = [x + PAD + 48.0, cy + 4.0, w - 2.0 * PAD - 48.0 - 112.0, 32.0];
                    ui.password_field(fr, "wifi-password", &mut n.password);
                    if n.password.is_empty() && !ui.editing() {
                        ui.text(fr[0] + 9.0, fr[1] + 21.0, "Password", 12.5, false, ink, 0.4, Align::Left);
                    }
                    let go = [x + w - PAD - 104.0, cy + 3.0, 104.0, 34.0];
                    if ui.button(go, "Join", true) && ui.hit(list) && !n.password.is_empty() {
                        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                        n.attempt = Some((ap.ssid.clone(), result.clone()));
                        acts.push(Act::JoinWifi(ap.ssid.clone(), n.password.clone(), result));
                        n.failed = None;
                    }
                }
                cy += ASK;
            }
        }
        ui.d.pop_clip();
        let _ = accent;
    }

    /// After drawing: speeds from the byte counters, what the background reading found, and the next reading.
    pub fn network_tick(&mut self) {
        let Some(pop) = &mut self.card else { return };
        let n = &mut pop.net;
        let now = Instant::now();
        for c in n.data.active.clone() {
            let (Some(rx), Some(tx)) = (read_bytes(&c.device, "rx"), read_bytes(&c.device, "tx")) else { continue };
            match n.prev.get(&c.device) {
                Some(&(prx, ptx, t)) if now.duration_since(t) >= Duration::from_millis(900) => {
                    let dt = now.duration_since(t).as_secs_f64();
                    n.speed.insert(c.device.clone(), (rx.saturating_sub(prx) as f64 / dt, tx.saturating_sub(ptx) as f64 / dt));
                    n.prev.insert(c.device.clone(), (rx, tx, now));
                }
                None => {
                    n.prev.insert(c.device.clone(), (rx, tx, now));
                }
                _ => {}
            }
        }
        // the attempt to join a network: what NetworkManager answered
        if let Some((ssid, result)) = n.attempt.clone() {
            let answer = result.lock().ok().and_then(|g| g.clone());
            if let Some(why) = answer {
                n.attempt = None;
                if why.is_empty() {
                    n.entering = None;
                    n.password.clear();
                    n.next = Some(now);
                } else if why.to_lowercase().contains("secret") {
                    // the saved password could not be had: ask for it, and keep it where NetworkManager itself can read it
                    n.entering = Some(ssid.clone());
                    n.password.clear();
                    n.failed = Some((ssid, "The saved password is not available: type it again".into()));
                } else {
                    n.failed = Some((ssid, why));
                }
            }
        }
        if let Some(rx) = &n.rx {
            if let Ok(data) = rx.try_recv() {
                n.rx = None;
                n.data = data;
            }
        }
        if n.rx.is_none() && n.next.is_none_or(|t| now >= t) {
            let (tx, rx) = channel();
            n.rx = Some(rx);
            n.next = Some(now + Duration::from_secs(4));
            std::thread::spawn(move || {
                let _ = tx.send(fetch());
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_lines_split_on_unescaped_colons() {
        assert_eq!(split_terse("Home\\: 5G:802-11-wireless:wlp9s0"), vec!["Home: 5G", "802-11-wireless", "wlp9s0"]);
    }

    #[test]
    fn active_connections_keep_wired_wifi_and_vpn_only() {
        let c = parse_active("Wired connection 2:802-3-ethernet:enp0s20f0u9\nTP-Link:802-11-wireless:wlp9s0\nlo:loopback:lo\ndocker0:bridge:docker0\n");
        assert_eq!(c.iter().map(|c| c.kind).collect::<Vec<_>>(), vec!["wired", "wifi"]);
    }

    #[test]
    fn vpn_profiles_are_found_among_the_saved_connections() {
        let v = parse_vpns("Work VPN:vpn
home-wg:wireguard
TP-Link:802-11-wireless
Wired connection 1:802-3-ethernet
");
        assert_eq!(v, vec!["Work VPN", "home-wg"]);
    }

    #[test]
    fn access_points_are_merged_and_sorted() {
        let out = "*:TP-Link:56:WPA2\n :TP-Link:25:WPA2\n :TP-Link_5G:20:WPA2\n :Cafe:70:\n :: 10:WPA2\n";
        let aps = parse_aps(out, &["TP-Link_5G".to_string()]);
        assert_eq!(aps.iter().map(|a| a.ssid.as_str()).collect::<Vec<_>>(), vec!["TP-Link", "Cafe", "TP-Link_5G"]);
        assert!(aps[0].active && aps[0].signal == 56);
        assert!(!aps[1].secure && aps[2].saved);
    }
}
