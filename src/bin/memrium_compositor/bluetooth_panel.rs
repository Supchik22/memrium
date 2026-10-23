//! The Bluetooth card of the Bluetooth icon: the switch, the devices that are paired (connect, disconnect, forget, their
//! battery) and the ones in reach to pair with. BlueZ is read and driven with `bluetoothctl`.

use crate::popup::Act;
use crate::state::State;
use chebrium::draw::Align;
use chebrium::ui::Ui;
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub const SIZE: [f32; 2] = [440.0, 520.0];
const PAD: f32 = 18.0;
const ROW: f32 = 60.0;
const FOOT: f32 = 58.0;
/// how long a search for new devices runs
const SCAN_SECS: u64 = 12;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dev {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// the BlueZ icon name (`audio-headset`, `input-keyboard`, ...)
    pub icon: String,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtData {
    pub adapter: bool,
    pub powered: bool,
    pub devices: Vec<Dev>,
}

/// `Device AA:BB:CC:DD:EE:FF Name` lines.
pub fn parse_devices(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ').unwrap_or((rest, ""));
            (mac.len() == 17).then(|| (mac.to_string(), if name.trim().is_empty() { mac.to_string() } else { name.trim().to_string() }))
        })
        .collect()
}

/// What `bluetoothctl info` says: the icon and the battery (`Battery Percentage: 0x55 (85)`).
pub fn parse_info(out: &str) -> (String, Option<u8>) {
    let mut icon = String::new();
    let mut battery = None;
    for l in out.lines().map(str::trim) {
        if let Some(v) = l.strip_prefix("Icon:") {
            icon = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("Battery Percentage:") {
            battery = v.split(['(', ')']).nth(1).and_then(|n| n.trim().parse().ok());
        }
    }
    (icon, battery)
}

fn run(args: &[&str]) -> Option<String> {
    std::process::Command::new("bluetoothctl").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

fn fetch() -> BtData {
    let Some(powered) = run(&["show"]).and_then(|o| memrium_de::sysctl::parse_powered(&o)) else { return BtData::default() };
    if !powered {
        return BtData { adapter: true, powered, devices: Vec::new() };
    }
    let all = run(&["devices"]).map(|o| parse_devices(&o)).unwrap_or_default();
    let set = |kind: &str| -> HashSet<String> { run(&["devices", kind]).map(|o| parse_devices(&o).into_iter().map(|d| d.0).collect()).unwrap_or_default() };
    let (paired, connected) = (set("Paired"), set("Connected"));
    let mut devices: Vec<Dev> = all
        .into_iter()
        .map(|(mac, name)| {
            let is_paired = paired.contains(&mac);
            let (icon, battery) = if is_paired { run(&["info", &mac]).map(|o| parse_info(&o)).unwrap_or_default() } else { Default::default() };
            Dev { connected: connected.contains(&mac), paired: is_paired, mac, name, icon, battery }
        })
        .collect();
    // connected first, then the paired, then those in reach; by name inside each
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    BtData { adapter: true, powered, devices }
}

/// The glyph of a device by what BlueZ says it is.
fn icon_of(d: &Dev) -> &'static str {
    match d.icon.as_str() {
        i if i.contains("keyboard") => "keyboard",
        i if i.contains("audio") || i.contains("headset") || i.contains("headphone") => "music-2",
        i if i.contains("phone") => "monitor-smartphone",
        i if i.contains("computer") => "laptop",
        _ => "bluetooth",
    }
}

#[derive(Default)]
pub struct Bt {
    data: BtData,
    rx: Option<Receiver<BtData>>,
    next: Option<Instant>,
    /// a search for new devices is on until this time
    scanning: Option<Instant>,
    /// a search was started while this card was open: unpaired devices are shown
    searched: bool,
    scroll: f32,
}

impl Bt {
    pub fn wake(&mut self) {
        self.next = None;
    }

    fn soon(&mut self) {
        self.next = Some(Instant::now() + Duration::from_millis(1500));
    }
}

impl State {
    pub fn bluetooth_build(&self, ui: &mut Ui, pos: [f32; 2], b: &mut Bt, acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let (w, h) = (SIZE[0], SIZE[1]);
        ui.glyph(x + PAD + 8.0, y + 29.0, 18.0, if b.data.powered || !b.data.adapter { "bluetooth" } else { "bluetooth-off" }, ink, 0.85);
        ui.text(x + PAD + 26.0, y + 35.0, "Bluetooth", 15.0, true, ink, 1.0, Align::Left);
        if b.data.adapter {
            let mut on = b.data.powered;
            ui.toggle(x + w - PAD - 44.0, y + 16.0, &mut on);
            if on != b.data.powered {
                b.data.powered = on;
                if !on {
                    b.data.devices.clear();
                    b.scanning = None;
                }
                acts.push(Act::Bluetooth(vec!["power".into(), if on { "on" } else { "off" }.into()]));
                b.soon();
            }
        }
        let list = [x, y + 58.0, w, h - 58.0 - FOOT];
        if !b.data.adapter {
            ui.glyph(x + w / 2.0, y + h / 2.0 - 20.0, 34.0, "bluetooth-off", ink, 0.25);
            ui.text(x + w / 2.0, y + h / 2.0 + 22.0, "No Bluetooth adapter", 14.0, true, ink, 0.55, Align::Center);
            return;
        }
        if !b.data.powered {
            ui.glyph(x + w / 2.0, y + h / 2.0 - 20.0, 34.0, "bluetooth-off", ink, 0.25);
            ui.text(x + w / 2.0, y + h / 2.0 + 22.0, "Bluetooth is off", 14.0, true, ink, 0.55, Align::Center);
            return;
        }
        let mine: Vec<Dev> = b.data.devices.iter().filter(|d| d.paired).cloned().collect();
        let near: Vec<Dev> = if b.searched { b.data.devices.iter().filter(|d| !d.paired).cloned().collect() } else { Vec::new() };
        let content = (if mine.is_empty() { 0.0 } else { 34.0 + ROW * mine.len() as f32 }) + if b.searched { 34.0 + ROW * near.len().max(1) as f32 } else { 0.0 };
        b.scroll = (b.scroll + ui.scroll).clamp(0.0, (content - list[3] + 8.0).max(0.0));
        ui.d.push_clip(list);
        let mut cy = list[1] - b.scroll;
        let head = |ui: &mut Ui, cy: f32, title: &str| {
            let tw = ui.measure(title, 12.5, true);
            ui.text(x + PAD, cy + 22.0, title, 12.5, true, ink, 0.6, Align::Left);
            ui.fill([x + PAD + tw + 10.0, cy + 17.0, w - 2.0 * PAD - tw - 10.0, 1.0], 0.5, ink, 0.14);
        };
        let visible = |cy: f32| cy + ROW > list[1] && cy < list[1] + list[3];
        if mine.is_empty() && !b.searched {
            ui.text(x + w / 2.0, list[1] + 60.0, "No paired devices", 13.5, true, ink, 0.55, Align::Center);
            ui.text(x + w / 2.0, list[1] + 82.0, "Search for devices to pair one", 12.0, false, ink, 0.4, Align::Center);
        }
        if !mine.is_empty() {
            head(ui, cy, "My devices");
            cy += 34.0;
            for d in &mine {
                if visible(cy) {
                    ui.glyph(x + PAD + 20.0, cy + 30.0, 24.0, icon_of(d), if d.connected { accent } else { ink }, if d.connected { 1.0 } else { 0.8 });
                    ui.text_raw(x + PAD + 48.0, cy + 26.0, &d.name, 13.5, false, ink, 0.95, Align::Left);
                    let sub = match (d.connected, d.battery) {
                        (true, Some(p)) => format!("{} · {p}%", memrium_de::i18n::tr("Connected")),
                        (true, None) => memrium_de::i18n::tr("Connected").into_owned(),
                        _ => memrium_de::i18n::tr("Paired").into_owned(),
                    };
                    ui.text_raw(x + PAD + 48.0, cy + 44.0, &sub, 11.5, false, ink, 0.55, Align::Left);
                    let main = [x + w - PAD - 104.0, cy + 15.0, 104.0, 30.0];
                    if ui.button(main, if d.connected { "Disconnect" } else { "Connect" }, false) && ui.hit(list) {
                        acts.push(Act::Bluetooth(vec![if d.connected { "disconnect" } else { "connect" }.into(), d.mac.clone()]));
                        b.soon();
                    }
                    // forget it
                    let fr = [main[0] - 34.0, cy + 17.0, 26.0, 26.0];
                    let hov = ui.hit(fr) && ui.hit(list);
                    if hov {
                        ui.fill(fr, 13.0, ink, 0.12);
                    }
                    ui.glyph(fr[0] + 13.0, fr[1] + 13.0, 13.0, "trash-2", ink, if hov { 0.9 } else { 0.5 });
                    if hov && ui.click {
                        acts.push(Act::Bluetooth(vec!["remove".into(), d.mac.clone()]));
                        b.soon();
                    }
                }
                cy += ROW;
            }
        }
        if b.searched {
            head(ui, cy, "Nearby");
            cy += 34.0;
            if near.is_empty() {
                ui.text(x + w / 2.0, cy + 30.0, if b.scanning.is_some() { "Looking for devices…" } else { "No new devices found" }, 13.0, false, ink, 0.5, Align::Center);
            }
            for d in &near {
                if visible(cy) {
                    ui.glyph(x + PAD + 20.0, cy + 30.0, 24.0, "bluetooth", ink, 0.7);
                    ui.text_raw(x + PAD + 48.0, cy + 26.0, &d.name, 13.5, false, ink, 0.95, Align::Left);
                    ui.text(x + PAD + 48.0, cy + 44.0, "Not paired", 11.5, false, ink, 0.55, Align::Left);
                    if ui.button([x + w - PAD - 104.0, cy + 15.0, 104.0, 30.0], "Pair", false) && ui.hit(list) {
                        acts.push(Act::BluetoothPair(d.mac.clone()));
                        b.soon();
                    }
                }
                cy += ROW;
            }
        }
        ui.d.pop_clip();

        // ---- the foot: search for new devices
        ui.fill([x + PAD, y + h - FOOT, w - 2.0 * PAD, 1.0], 0.5, ink, 0.12);
        let busy = b.scanning.is_some();
        let label = if busy { "Searching…" } else { "Search for devices" };
        let r = [x + PAD, y + h - FOOT + 14.0, w - 2.0 * PAD, 32.0];
        if busy {
            ui.fill(r, 9.0, ink, 0.05);
            ui.text(r[0] + r[2] / 2.0, r[1] + 21.0, label, 13.0, false, ink, 0.45, Align::Center);
        } else if ui.button(r, label, false) {
            b.searched = true;
            b.scanning = Some(Instant::now() + Duration::from_secs(SCAN_SECS));
            acts.push(Act::Bluetooth(vec!["--timeout".into(), SCAN_SECS.to_string(), "scan".into(), "on".into()]));
            b.soon();
        }
    }

    /// After drawing: what the background reading found, and the next reading.
    pub fn bluetooth_tick(&mut self) {
        let Some(pop) = &mut self.card else { return };
        let b = &mut pop.bt;
        let now = Instant::now();
        if b.scanning.is_some_and(|t| now >= t) {
            b.scanning = None;
        }
        if let Some(rx) = &b.rx {
            if let Ok(data) = rx.try_recv() {
                b.rx = None;
                b.data = data;
            }
        }
        if b.rx.is_none() && b.next.is_none_or(|t| now >= t) {
            let (tx, rx) = channel();
            b.rx = Some(rx);
            b.next = Some(now + Duration::from_secs(if b.scanning.is_some() { 2 } else { 5 }));
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
    fn devices_are_read_from_the_list() {
        let d = parse_devices("Device 00:1A:7D:DA:71:13 WH-1000XM4\nDevice AA:BB:CC:DD:EE:FF AA:BB:CC:DD:EE:FF\nDevice bad x\nnoise\n");
        assert_eq!(d, vec![("00:1A:7D:DA:71:13".to_string(), "WH-1000XM4".to_string()), ("AA:BB:CC:DD:EE:FF".to_string(), "AA:BB:CC:DD:EE:FF".to_string())]);
    }

    #[test]
    fn icon_and_battery_come_from_the_info() {
        let (icon, battery) = parse_info("Device 00:1A:7D:DA:71:13 (public)\n\tName: X\n\tIcon: audio-headset\n\tPaired: yes\n\tBattery Percentage: 0x55 (85)\n");
        assert_eq!((icon.as_str(), battery), ("audio-headset", Some(85)));
        assert_eq!(parse_info("Name: x\n"), (String::new(), None));
    }
}
