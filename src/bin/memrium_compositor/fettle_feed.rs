//! What Fettle (the controls dropdown in its own look) shows besides the machine's basic state: the Wi-Fi networks in
//! reach and how strong each is, the Bluetooth devices with their batteries, the sound outputs and the apps that play.
//! Read off the main loop with the same tools as the bar's cards (`nmcli`, `bluetoothctl`, `wpctl`), and only while
//! the dropdown is open and shows them: every few seconds, or often while a list is grown open.

use memrium_de::controlcenter::{CcDevice, CcLists, CcNetwork, CcNode, CcWants};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

/// One list being read in the background, and when to read it again.
struct Reader<T> {
    rx: Option<Receiver<T>>,
    next: Option<Instant>,
}

impl<T> Default for Reader<T> {
    fn default() -> Self {
        Reader { rx: None, next: None }
    }
}

impl<T: Send + 'static> Reader<T> {
    /// Take a finished reading; start the next one when it is due (`every` apart). None when nothing new came.
    fn tick(&mut self, want: bool, every: Duration, fetch: fn() -> T) -> Option<T> {
        let mut got = None;
        if let Some(rx) = &self.rx {
            if let Ok(v) = rx.try_recv() {
                self.rx = None;
                got = Some(v);
            }
        }
        if !want {
            // asked again as soon as it is wanted again
            self.next = None;
            return got;
        }
        // a grown list is read sooner than a reading planned for later
        if let Some(n) = self.next {
            if n > Instant::now() + every {
                self.next = Some(Instant::now() + every);
            }
        }
        if self.rx.is_none() && self.next.is_none_or(|t| Instant::now() >= t) {
            let (tx, rx) = channel();
            self.rx = Some(rx);
            self.next = Some(Instant::now() + every);
            std::thread::spawn(move || {
                let _ = tx.send(fetch());
            });
        }
        got
    }
}

#[derive(Default)]
pub struct FettleFeed {
    nets: Reader<crate::network_panel::NetData>,
    devs: Reader<crate::bluetooth_panel::BtData>,
    audio: Reader<crate::audio_panel::AudioData>,
    lists: CcLists,
}

impl FettleFeed {
    /// Read what `wants` asks for; the lists when they changed.
    pub fn tick(&mut self, wants: CcWants) -> Option<&CcLists> {
        let every = if wants.soon { Duration::from_millis(1500) } else { Duration::from_secs(6) };
        let mut changed = false;
        if let Some(n) = self.nets.tick(wants.networks, every, crate::network_panel::fetch) {
            let mut list: Vec<CcNetwork> = Vec::new();
            for ap in n.aps.into_iter().filter(|a| !a.ssid.is_empty()) {
                // one row per name: the strongest of a network's access points
                match list.iter_mut().find(|x| x.ssid == ap.ssid) {
                    Some(x) => {
                        x.signal = x.signal.max(ap.signal);
                        x.active |= ap.active;
                    }
                    None => list.push(CcNetwork { ssid: ap.ssid, signal: ap.signal, secure: ap.secure, active: ap.active, saved: ap.saved }),
                }
            }
            list.sort_by(|a, b| b.active.cmp(&a.active).then(b.saved.cmp(&a.saved)).then(b.signal.cmp(&a.signal)));
            self.lists.networks = Some(list);
            changed = true;
        }
        if let Some(b) = self.devs.tick(wants.devices, every, crate::bluetooth_panel::fetch) {
            self.lists.devices = Some(b.devices.into_iter().map(|d| CcDevice { mac: d.mac, name: d.name, connected: d.connected, paired: d.paired, icon: d.icon, battery: d.battery }).collect());
            changed = true;
        }
        if let Some(a) = self.audio.tick(wants.audio, every.min(Duration::from_millis(1500)), crate::audio_panel::fetch) {
            let node = |n: crate::audio_panel::Node| CcNode { id: n.id, name: n.name, vol: n.vol, muted: n.muted, default: n.default };
            self.lists.outputs = a.sinks.into_iter().map(node).collect();
            // the output in use first (a short list shows only it)
            self.lists.outputs.sort_by_key(|o| !o.default);
            self.lists.streams = a.streams.into_iter().map(node).collect();
            changed = true;
        }
        changed.then_some(&self.lists)
    }
}
