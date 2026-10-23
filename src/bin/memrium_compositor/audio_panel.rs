//! The sound mixer of the volume icon: output and input devices with a volume each and the one in use marked, and
//! on a second tab the volume of every app that is playing sound. PipeWire is read and driven with `wpctl`.

use crate::popup::Act;
use crate::state::State;
use chebrium::draw::Align;
use chebrium::style::Tone;
use chebrium::ui::Ui;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub const SIZE: [f32; 2] = [420.0, 540.0];
const PAD: f32 = 18.0;
const ROW: f32 = 74.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    /// 0 .. 1 (PipeWire allows more)
    pub vol: f32,
    pub muted: bool,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioData {
    pub sinks: Vec<Node>,
    pub sources: Vec<Node>,
    pub streams: Vec<Node>,
}

/// `   72. Skadi HSW191WB Analog Stereo        [vol: 0.70]` (with a `*` before the number for the default one).
fn parse_node(s: &str) -> Option<Node> {
    let default = s.starts_with('*');
    let s = s.trim_start_matches(['*', ' ']);
    let (id, rest) = s.split_once(". ")?;
    let id: u32 = id.trim().parse().ok()?;
    let (name, vol) = match rest.split_once("[vol:") {
        Some((n, v)) => (n.trim(), Some(v.trim_end_matches(']'))),
        None => (rest.trim(), None),
    };
    let (vol, muted) = vol.map_or((1.0, false), |v| (v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(1.0), v.contains("MUTED")));
    Some(Node { id, name: name.to_string(), vol, muted, default })
}

/// The sinks, sources and playing streams of `wpctl status` (a stream's volume is read separately).
pub fn parse_status(out: &str) -> AudioData {
    let mut data = AudioData::default();
    let (mut in_audio, mut section) = (false, "");
    for line in out.lines() {
        let s = line.trim_start_matches(['│', '├', '└', '─', ' ', '\t']);
        match s.trim_end() {
            "Audio" => in_audio = true,
            "Video" | "Settings" => in_audio = false,
            t if in_audio && t.ends_with(':') => section = if t.starts_with("Sinks") { "sinks" } else if t.starts_with("Sources") { "sources" } else if t.starts_with("Streams") { "streams" } else { "" },
            _ if !in_audio => {}
            _ => match section {
                "sinks" => data.sinks.extend(parse_node(s)),
                "sources" => data.sources.extend(parse_node(s)),
                // a stream's own line has no arrow; its ports below it have
                "streams" if !s.contains('>') && !s.contains('<') => data.streams.extend(parse_node(s)),
                _ => {}
            },
        }
    }
    data
}

fn fetch() -> AudioData {
    let run = |args: &[&str]| std::process::Command::new("wpctl").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string());
    let mut data = run(&["status"]).map(|o| parse_status(&o)).unwrap_or_default();
    for s in &mut data.streams {
        if let Some((v, m)) = run(&["get-volume", &s.id.to_string()]).and_then(|o| memrium_de::sysctl::parse_volume(&o)) {
            s.vol = v;
            s.muted = m;
        }
    }
    data
}

#[derive(Default)]
pub struct Audio {
    tab: usize,
    data: AudioData,
    rx: Option<Receiver<AudioData>>,
    next: Option<Instant>,
    /// a slider was moved a moment ago: what the machine reports lags behind, so it is not taken yet
    hold: Option<Instant>,
    scroll: f32,
}

impl Audio {
    pub fn wake(&mut self) {
        self.next = None;
    }
}

impl State {
    pub fn audio_build(&self, ui: &mut Ui, pos: [f32; 2], a: &mut Audio, acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, y) = (pos[0], pos[1]);
        let (w, h) = (SIZE[0], SIZE[1]);
        ui.glyph(x + PAD + 8.0, y + 29.0, 18.0, "volume-2", ink, 0.85);
        ui.text(x + PAD + 26.0, y + 35.0, "Audio Volume", 15.0, true, ink, 1.0, Align::Left);
        let mut tab = a.tab;
        if ui.segmented([x + PAD, y + 52.0, w - 2.0 * PAD, 32.0], &["Devices", "Applications"], &mut tab) {
            a.tab = tab;
            a.scroll = 0.0;
        }

        // ---- the list, which scrolls under the tabs
        let list = [x, y + 96.0, w, h - 96.0 - 54.0];
        let mut rows: Vec<(Option<&str>, Option<(&'static str, Node)>)> = Vec::new();
        if a.tab == 0 {
            rows.push((Some("Output Devices"), None));
            rows.extend(a.data.sinks.iter().map(|n| (None, Some(("sinks", n.clone())))));
            if !a.data.sources.is_empty() {
                rows.push((Some("Input Devices"), None));
                rows.extend(a.data.sources.iter().map(|n| (None, Some(("sources", n.clone())))));
            }
        } else {
            rows.extend(a.data.streams.iter().map(|n| (None, Some(("streams", n.clone())))));
        }
        let content: f32 = rows.iter().map(|(h, _)| if h.is_some() { 34.0 } else { ROW }).sum();
        let max = (content - list[3] + 8.0).max(0.0);
        a.scroll = (a.scroll + ui.scroll).clamp(0.0, max);
        ui.d.push_clip(list);
        let mut cy = list[1] - a.scroll;
        if a.tab == 1 && a.data.streams.is_empty() {
            ui.text(x + w / 2.0, list[1] + 60.0, "No application is playing sound", 13.0, false, ink, 0.5, Align::Center);
        }
        for (head, node) in rows {
            if let Some(title) = head {
                if cy + 34.0 > list[1] && cy < list[1] + list[3] {
                    let tw = ui.measure(title, 12.5, true);
                    ui.text(x + PAD, cy + 22.0, title, 12.5, true, ink, 0.6, Align::Left);
                    ui.fill([x + PAD + tw + 10.0, cy + 17.0, w - 2.0 * PAD - tw - 10.0, 1.0], 0.5, ink, 0.14);
                }
                cy += 34.0;
                continue;
            }
            let Some((group, n)) = node else { continue };
            let top = cy;
            cy += ROW;
            if top + ROW < list[1] || top > list[1] + list[3] {
                continue;
            }
            let header = [x + PAD, top + 4.0, w - 2.0 * PAD, 30.0];
            let pickable = group != "streams";
            if pickable {
                let rc = [x + PAD + 4.0, top + 12.0, 16.0, 16.0];
                ui.fill(rc, 8.0, ink, 0.25);
                if n.default {
                    ui.fill([rc[0] + 3.0, rc[1] + 3.0, 10.0, 10.0], 5.0, accent, 1.0);
                } else {
                    ui.fill([rc[0] + 1.5, rc[1] + 1.5, 13.0, 13.0], 6.5, [1.0; 3], if ui.dark { 0.08 } else { 0.9 });
                }
                if ui.click && ui.hit(header) && !n.default && ui.hit(list) {
                    acts.push(Act::DefaultSink(n.id));
                }
            }
            let name = n.name.clone();
            ui.text_raw(x + PAD + if pickable { 34.0 } else { 6.0 }, top + 25.0, &chebrium_trunc(&name, 38), 13.5, false, ink, 0.95, Align::Left);
            ui.glyph(x + PAD + 14.0, top + 52.0, 15.0, if n.muted { "volume-x" } else if group == "sources" { "mic" } else { "volume-1" }, ink, 0.8);
            let mut pct = (n.vol * 100.0).round();
            let before = pct;
            let track = [x + PAD + 36.0, top + 42.0, w - 2.0 * PAD - 36.0 - 52.0, 20.0];
            // (a slider half under the tabs or the footer is not drawn: it could be moved through them)
            if track[1] >= list[1] && track[1] + track[3] <= list[1] + list[3] {
                let tone = match group { "sources" => Tone::Info, "streams" => Tone::Special, _ => Tone::Primary };
                ui.tinted(tone, |ui| ui.slider(track, &mut pct, 0.0, 100.0, "%"));
            }
            if pct != before {
                a.hold = Some(Instant::now());
                let v = pct / 100.0;
                for list in [&mut a.data.sinks, &mut a.data.sources, &mut a.data.streams] {
                    if let Some(m) = list.iter_mut().find(|m| m.id == n.id) {
                        m.vol = v;
                    }
                }
                acts.push(Act::NodeVolume(n.id, v));
            }
        }
        ui.d.pop_clip();

        // ---- footer: mute the default output
        ui.fill([x + PAD, y + h - 54.0, w - 2.0 * PAD, 1.0], 0.5, ink, 0.12);
        let muted = self.sys.muted;
        let mut on = muted;
        ui.text(x + PAD, y + h - 20.0, "Mute", 13.5, false, ink, 0.9, Align::Left);
        ui.toggle(x + w - PAD - 44.0, y + h - 40.0, &mut on);
        if on != muted {
            acts.push(Act::ToggleMuteDefault);
        }
    }

    /// After drawing: take what the background reading found, and start the next reading.
    pub fn audio_tick(&mut self) {
        let Some(pop) = &mut self.card else { return };
        let a = &mut pop.audio;
        if let Some(rx) = &a.rx {
            if let Ok(data) = rx.try_recv() {
                a.rx = None;
                if a.hold.is_none_or(|t| t.elapsed() > Duration::from_millis(1500)) {
                    a.data = data;
                }
            }
        }
        if a.rx.is_none() && a.next.is_none_or(|t| Instant::now() >= t) {
            let (tx, rx) = channel();
            a.rx = Some(rx);
            a.next = Some(Instant::now() + Duration::from_millis(1200));
            std::thread::spawn(move || {
                let _ = tx.send(fetch());
            });
        }
    }
}

fn chebrium_trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "PipeWire 'pipewire-0'\n └─ Clients:\n        38. kwin\n\nAudio\n ├─ Devices:\n │      62. GA107 [alsa]\n │  \n ├─ Sinks:\n │      35. Applio OUT (virtual mic)            [vol: 1.00]\n │  *   72. Skadi HSW191WB Analog Stereo        [vol: 0.70]\n │  \n ├─ Sources:\n │  *   73. Skadi HSW191WB Mono                 [vol: 1.00]\n │      81. Built-in Audio Pro                  [vol: 1.00 MUTED]\n │  \n └─ Streams:\n       205. Brave                                                       \n            204. output_FL       > Skadi HSW191WB:playback_FL\t[active]\n            206. output_FR       > Skadi HSW191WB:playback_FR\t[active]\n\nVideo\n ├─ Devices:\n";

    #[test]
    fn sinks_sources_and_streams_are_found() {
        let d = parse_status(STATUS);
        assert_eq!(d.sinks.len(), 2);
        assert!(d.sinks[1].default && d.sinks[1].id == 72 && (d.sinks[1].vol - 0.7).abs() < 1e-6);
        assert_eq!(d.sources.len(), 2);
        assert!(d.sources[1].muted);
        assert_eq!(d.streams.iter().map(|s| (s.id, s.name.as_str())).collect::<Vec<_>>(), vec![(205, "Brave")]);
    }
}
