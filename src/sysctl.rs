//! The machine's state and controls as the panels show them: volume (PipeWire / WirePlumber), Wi-Fi
//! (NetworkManager), Bluetooth (BlueZ), screen brightness and battery. State is read on a background thread
//! with the usual command-line tools, so the shell never waits on them, and commands run there too.
//! A part the machine does not have (no battery on a desktop, no Wi-Fi card) is simply `None`.

use std::path::Path;
use std::process::Command;
use std::sync::{
    mpsc::{channel, Receiver, RecvTimeoutError, Sender},
    Arc, Mutex,
};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub enabled: bool,
    /// the network we are connected to
    pub ssid: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bluetooth {
    pub powered: bool,
    pub connected: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SysInfo {
    /// output volume, 0 .. 1 (above 1 is allowed by PipeWire)
    pub volume: Option<f32>,
    pub muted: bool,
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    /// 0 .. 1
    pub brightness: Option<f32>,
    /// (percent, charging)
    pub battery: Option<(u8, bool)>,
    /// how much of its design capacity the battery still holds, percent
    pub health: Option<u8>,
    /// the power profile (`power-saver`, `balanced`, `performance`) and the ones this machine offers
    pub profile: Option<String>,
    pub profiles: Vec<String>,
}

/// Something to do to the machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    SetVolume(f32),
    ToggleMute,
    SetWifi(bool),
    SetBluetooth(bool),
    SetBrightness(f32),
    SetProfile(String),
    /// a PipeWire node (a device, an app's stream) to this volume; a node to be the default output
    SetNodeVolume(u32, f32),
    SetDefaultSink(u32),
    /// `nmcli` with these arguments (connect, disconnect)
    Net(Vec<String>),
    /// look at everything again now
    Refresh,
}

pub struct SysWatcher {
    shared: Arc<Mutex<SysInfo>>,
    tx: Sender<Cmd>,
    watching: Arc<std::sync::atomic::AtomicBool>,
}

impl SysWatcher {
    pub fn start() -> SysWatcher {
        let shared = Arc::new(Mutex::new(SysInfo::default()));
        let (tx, rx) = channel();
        let s2 = shared.clone();
        let watching = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let w2 = watching.clone();
        std::thread::Builder::new().name("sysctl".into()).spawn(move || worker(s2, rx, w2)).ok();
        SysWatcher { shared, tx, watching }
    }

    /// A panel that shows the machine's state is open (Quick Settings, a flyout, Settings): it is looked at more often.
    /// Opening one looks at once.
    pub fn set_watching(&self, on: bool) {
        let was = self.watching.swap(on, std::sync::atomic::Ordering::Relaxed);
        if on && !was {
            let _ = self.tx.send(Cmd::Refresh);
        }
    }

    pub fn info(&self) -> SysInfo {
        self.shared.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

/// How often the slow (process-spawning) readings are refreshed when nothing asked for them sooner, while a panel that
/// shows them is open, and while none is (the bar's icons then follow a little later: each look starts several programs,
/// which costs battery). Sysfs readings are free and follow every wake-up.
const SLOW: Duration = Duration::from_secs(8);
const SLOW_UNWATCHED: Duration = Duration::from_secs(30);

fn worker(shared: Arc<Mutex<SysInfo>>, rx: Receiver<Cmd>, watching: Arc<std::sync::atomic::AtomicBool>) {
    let mut last_slow = std::time::Instant::now() - SLOW;
    // the profiles the machine offers do not change: asked for once
    let mut profiles: Option<Vec<String>> = None;
    // the first look happens at once, so the panels never show "not available" for a few seconds at start-up
    let mut first = true;
    loop {
        let mut force = false;
        let waited = if first { Err(RecvTimeoutError::Timeout) } else { rx.recv_timeout(Duration::from_secs(3)) };
        first = false;
        match waited {
            Ok(cmd) => {
                // whatever piled up while the last command ran is one batch; of a burst of slider moves only the last
                // value of each matters (each one costs a process), and moving a slider needs no look at Wi-Fi & co
                let mut batch = vec![cmd];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                // the same thing set several times: only the last counts (a node's volume is one thing per node)
                let key = |c: &Cmd| match c {
                    Cmd::SetVolume(_) => Some((0, 0)),
                    Cmd::SetBrightness(_) => Some((1, 0)),
                    Cmd::SetNodeVolume(id, _) => Some((2, *id)),
                    _ => None,
                };
                for (i, c) in batch.iter().enumerate() {
                    let stale = key(c).is_some_and(|k| batch[i + 1..].iter().any(|later| key(later) == Some(k)));
                    if !stale {
                        run(c);
                    }
                    force |= !matches!(c, Cmd::SetVolume(_) | Cmd::SetBrightness(_) | Cmd::SetNodeVolume(..));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut info = shared.lock().map(|g| g.clone()).unwrap_or_default();
        info.battery = read_battery();
        info.health = read_health();
        info.brightness = read_brightness();
        let slow = if watching.load(std::sync::atomic::Ordering::Relaxed) { SLOW } else { SLOW_UNWATCHED };
        if force || last_slow.elapsed() >= slow {
            last_slow = std::time::Instant::now();
            if let Some((v, m)) = run_out("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|o| parse_volume(&o)) {
                info.volume = Some(v);
                info.muted = m;
            }
            info.profile = run_out("powerprofilesctl", &["get"]).map(|o| o.trim().to_string()).filter(|p| !p.is_empty());
            info.profiles = match (&info.profile, &profiles) {
                (None, _) => Vec::new(),
                (Some(_), Some(p)) => p.clone(),
                (Some(_), None) => {
                    let p = run_out("powerprofilesctl", &["list"]).map(|o| parse_profiles(&o)).unwrap_or_default();
                    if !p.is_empty() {
                        profiles = Some(p.clone());
                    }
                    p
                }
            };
            info.wifi = read_wifi();
            info.bluetooth = read_bluetooth();
        }
        if let Ok(mut g) = shared.lock() {
            *g = info;
        }
    }
}

fn run_out(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn run(cmd: &Cmd) {
    let quiet = |program: &str, args: &[&str]| {
        let _ = Command::new(program).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    };
    match cmd {
        Cmd::SetVolume(v) => quiet("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", v.clamp(0.0, 1.5))]),
        Cmd::ToggleMute => quiet("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
        Cmd::SetWifi(on) => quiet("nmcli", &["radio", "wifi", if *on { "on" } else { "off" }]),
        Cmd::SetBluetooth(on) => quiet("bluetoothctl", &["power", if *on { "on" } else { "off" }]),
        Cmd::SetBrightness(v) => {
            if let Some((dev, max)) = backlight() {
                let value = ((v.clamp(0.02, 1.0)) * max as f32).round() as u32;
                // the login manager may set it without root rights
                quiet("busctl", &["call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "SetBrightness", "ssu", "backlight", &dev, &value.to_string()]);
            }
        }
        Cmd::SetProfile(name) => quiet("powerprofilesctl", &["set", name]),
        Cmd::SetNodeVolume(id, v) => quiet("wpctl", &["set-volume", &id.to_string(), &format!("{:.2}", v.clamp(0.0, 1.5))]),
        Cmd::SetDefaultSink(id) => quiet("wpctl", &["set-default", &id.to_string()]),
        Cmd::Net(args) => quiet("nmcli", &args.iter().map(String::as_str).collect::<Vec<_>>()),
        Cmd::Refresh => {}
    }
}

/// The profile names of `powerprofilesctl list`: lines like `* balanced:` and `  power-saver:` (the star marks the active one).
pub fn parse_profiles(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(['*', ' ']);
            let name = l.strip_suffix(':')?;
            (!name.contains(' ') && !name.is_empty() && l.len() < 24 && !l.starts_with(char::is_whitespace)).then(|| name.to_string())
        })
        .collect()
}

/// Full capacity against design capacity of the first battery, in percent.
fn read_health() -> Option<u8> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        if std::fs::read_to_string(p.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        for (now, design) in [("energy_full", "energy_full_design"), ("charge_full", "charge_full_design")] {
            if let (Some(a), Some(b)) = (read_num(&p.join(now)), read_num(&p.join(design))) {
                if b > 0 {
                    return Some((a as f32 / b as f32 * 100.0).round().clamp(0.0, 100.0) as u8);
                }
            }
        }
    }
    None
}

/// `Volume: 0.45` or `Volume: 0.45 [MUTED]`
pub fn parse_volume(out: &str) -> Option<(f32, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?;
    let v: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some((v, rest.contains("[MUTED]")))
}

/// Lines of `nmcli -t -f ACTIVE,SSID device wifi`: the connected network, if any (`\:` escapes a colon in a name).
pub fn parse_active_ssid(out: &str) -> Option<String> {
    out.lines().find_map(|l| l.strip_prefix("yes:").map(|s| s.replace("\\:", ":"))).filter(|s| !s.is_empty())
}

fn read_wifi() -> Option<Wifi> {
    let devices = run_out("nmcli", &["-t", "-f", "TYPE", "device"])?;
    if !devices.lines().any(|l| l.trim() == "wifi") {
        return None;
    }
    let enabled = run_out("nmcli", &["radio", "wifi"])?.trim() == "enabled";
    // (`--rescan no`: listing the networks otherwise makes the card scan the air when its last scan is half a minute old,
    // which costs power every time we look)
    let ssid = if enabled { run_out("nmcli", &["-t", "-f", "ACTIVE,SSID", "device", "wifi", "list", "--rescan", "no"]).and_then(|o| parse_active_ssid(&o)) } else { None };
    Some(Wifi { enabled, ssid })
}

/// `bluetoothctl show` has a `Powered: yes` line when there is an adapter.
pub fn parse_powered(out: &str) -> Option<bool> {
    out.lines().find_map(|l| l.trim().strip_prefix("Powered:").map(|v| v.trim() == "yes"))
}

fn read_bluetooth() -> Option<Bluetooth> {
    let powered = parse_powered(&run_out("bluetoothctl", &["show"])?)?;
    let connected = if powered { run_out("bluetoothctl", &["devices", "Connected"]).map(|o| o.lines().filter(|l| l.starts_with("Device ")).count()).unwrap_or(0) } else { 0 };
    Some(Bluetooth { powered, connected })
}

fn read_num(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_battery() -> Option<(u8, bool)> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        if std::fs::read_to_string(p.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        let percent = read_num(&p.join("capacity"))?.min(100) as u8;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some((percent, matches!(status.trim(), "Charging" | "Full")));
    }
    None
}

/// (device name, max brightness) of the first backlight
fn backlight() -> Option<(String, u32)> {
    let entry = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?;
    let max = read_num(&entry.path().join("max_brightness"))?;
    (max > 0).then(|| (entry.file_name().to_string_lossy().to_string(), max))
}

fn read_brightness() -> Option<f32> {
    let entry = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?;
    let max = read_num(&entry.path().join("max_brightness"))? as f32;
    let now = read_num(&entry.path().join("brightness"))? as f32;
    (max > 0.0).then(|| (now / max).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profiles_are_read_from_the_list() {
        let out = "  performance:\n    CpuDriver: amd\n\n* balanced:\n    CpuDriver: amd\n\n  power-saver:\n    CpuDriver: amd\n";
        assert_eq!(parse_profiles(out), vec!["performance", "balanced", "power-saver"]);
    }

    #[test]
    fn volume_lines() {
        assert_eq!(parse_volume("Volume: 0.45\n"), Some((0.45, false)));
        assert_eq!(parse_volume("Volume: 1.00 [MUTED]\n"), Some((1.0, true)));
        assert_eq!(parse_volume("Could not connect"), None);
    }

    #[test]
    fn the_connected_network_is_found() {
        assert_eq!(parse_active_ssid("no:Cafe\nyes:Home\\:5G\nno:Other\n"), Some("Home:5G".into()));
        assert_eq!(parse_active_ssid("no:Cafe\n"), None);
        assert_eq!(parse_active_ssid("yes:\n"), None);
    }

    #[test]
    fn bluetooth_power() {
        assert_eq!(parse_powered("Controller AA:BB\n\tPowered: yes\n\tDiscoverable: no\n"), Some(true));
        assert_eq!(parse_powered("Controller AA:BB\n\tPowered: no\n"), Some(false));
        assert_eq!(parse_powered("No default controller available\n"), None);
    }
}
