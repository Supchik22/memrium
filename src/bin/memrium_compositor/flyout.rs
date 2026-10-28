//! A small slider that appears next to a tray icon when it is clicked (volume, brightness) and goes away when
//! the pointer clicks anywhere else.

use crate::state::State;
use memrium_de::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use memrium_de::sysctl::Cmd;
use memrium_de::theme::Edge;

const W: f32 = 300.0;
const H: f32 = 54.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FlyKind {
    Volume,
    Brightness,
    /// a media key (play, next...): an icon and the track's name, no slider
    Media,
}

/// How long the on-screen indicator of a hardware key stays.
const OSD_SECS: f32 = 1.6;

pub struct Flyout {
    pub kind: FlyKind,
    /// top-left corner on screen
    pub pos: [f32; 2],
    pub k: Spring,
    pub dragging: bool,
    /// the value being dragged (0..1), so the knob follows the pointer before the system answers
    pub value: f32,
    closing: bool,
    /// shown by a hardware key rather than a click: it goes away by itself at this time
    osd_until: Option<std::time::Instant>,
    /// what a media key says: (icon, text)
    media: (String, String),
    /// mute as the key set it, before the system answers
    muted: Option<bool>,
}

impl Flyout {
    fn track(&self) -> [f32; 4] {
        [self.pos[0] + 52.0, self.pos[1] + H / 2.0 - 10.0, W - 52.0 - 56.0, 20.0]
    }
}

impl State {
    /// Open (or, when the same one is open, close) the flyout for a tray icon clicked at `x` on `bar`.
    pub fn toggle_flyout(&mut self, kind: FlyKind, bar: usize, x: f32) {
        if self.flyout.as_ref().is_some_and(|f| f.kind == kind && !f.closing) {
            self.close_flyout();
            return;
        }
        let Some((r, edge)) = self.with_bar_ctx(|c| self.bars.get(bar).map(|b| (b.rect(c), b.cfg.edge))) else { return };
        // the bar's rectangle is in the coordinates of the monitor it was clicked on
        let (o, m) = (self.mon_origin(self.bar_mon.get()), self.mons.get(self.bar_mon.get()).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]));
        let mut px = (x - W / 2.0).clamp(o[0] + 8.0, (o[0] + m[2] - W - 8.0).max(o[0] + 8.0));
        let mut py = o[1] + if edge == Edge::Bottom { r[1] - H - 10.0 } else { r[1] + r[3] + 10.0 };
        // from a bar at the side it opens beside the icon
        if edge.vertical() {
            px = o[0] + if edge == Edge::Left { r[0] + r[2] + 10.0 } else { r[0] - W - 10.0 };
            py = (self.mouse[1] - H / 2.0).clamp(o[1] + 8.0, (o[1] + m[3] - H - 8.0).max(o[1] + 8.0));
        }
        let value = match kind {
            FlyKind::Volume => self.sys.volume,
            FlyKind::Brightness => self.sys.brightness,
            FlyKind::Media => None,
        }
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
        let mut k = Spring::new(0.0, 420.0, 0.8);
        k.target = 1.0;
        self.flyout = Some(Flyout { kind, pos: [px, py], k, dragging: false, value, closing: false, osd_until: None, media: Default::default(), muted: None });
    }

    /// A hardware media key (volume, brightness, play...): do it and show what changed, at the bottom of the screen.
    /// True when `raw` was one of them.
    pub fn media_key(&mut self, raw: u32) -> bool {
        use crate::media::MediaCmd;
        let step = 0.05;
        match raw {
            0x1008FF13 | 0x1008FF11 | 0x1008FF12 => {
                let open = self.flyout.as_ref().filter(|f| f.kind == FlyKind::Volume && !f.closing).map(|f| (f.value, f.muted));
                let base = open.map(|o| o.0).or(self.sys.volume).unwrap_or(0.5).clamp(0.0, 1.0);
                let muted = open.and_then(|o| o.1).unwrap_or(self.sys.muted);
                let (v, m) = match raw {
                    0x1008FF13 => ((base + step).min(1.0), false),
                    0x1008FF11 => ((base - step).max(0.0), false),
                    _ => (base, !muted),
                };
                if raw == 0x1008FF12 {
                    self.sys_watcher.send(Cmd::ToggleMute);
                } else {
                    self.sys_watcher.send(Cmd::SetVolume(v));
                    // turning the volume up on a muted output unmutes it
                    if muted {
                        self.sys_watcher.send(Cmd::ToggleMute);
                    }
                    self.ui_sound(memrium_de::sound::Sound::Tick);
                }
                self.show_osd(FlyKind::Volume, v, Some(m), ("", ""));
            }
            0x1008FF02 | 0x1008FF03 => {
                let open = self.flyout.as_ref().filter(|f| f.kind == FlyKind::Brightness && !f.closing).map(|f| f.value);
                let base = open.or(self.sys.brightness).unwrap_or(0.5).clamp(0.0, 1.0);
                // never all the way dark: the screen could not be found again
                let v = if raw == 0x1008FF02 { (base + step).min(1.0) } else { (base - step).max(0.02) };
                self.sys_watcher.send(Cmd::SetBrightness(v));
                self.show_osd(FlyKind::Brightness, v, None, ("", ""));
            }
            0x1008FF14 | 0x1008FF15 | 0x1008FF16 | 0x1008FF17 => {
                if self.media.is_none() {
                    self.media = Some(crate::media::Media::start());
                }
                let info = self.media.as_ref().map(|m| m.get()).unwrap_or_default();
                let (cmd, icon, text) = match raw {
                    0x1008FF14 => (Some(MediaCmd::PlayPause), if info.playing { "pause" } else { "play" }, if info.playing { "Paused" } else { "Playing" }),
                    0x1008FF16 => (Some(MediaCmd::Previous), "skip-back", "Previous"),
                    0x1008FF17 => (Some(MediaCmd::Next), "skip-forward", "Next"),
                    _ => (None, "square", "Stopped"),
                };
                if let (Some(c), Some(m)) = (cmd, &self.media) {
                    m.send(c);
                }
                let title = if info.title.is_empty() { text.to_string() } else if info.artist.is_empty() { format!("{text} · {}", info.title) } else { format!("{text} · {} – {}", info.artist, info.title) };
                self.show_osd(FlyKind::Media, 0.0, None, (icon, &title));
            }
            _ => return false,
        }
        true
    }

    /// Show (or update) the indicator of a hardware key at the bottom middle of the first monitor.
    fn show_osd(&mut self, kind: FlyKind, value: f32, muted: Option<bool>, media: (&str, &str)) {
        let m = self.mons.first().copied().unwrap_or([self.world[0], self.world[1], self.world[2] - self.world[0], self.world[3] - self.world[1]]);
        let w = if kind == FlyKind::Media { 380.0 } else { W };
        let pos = [m[0] + (m[2] - w) / 2.0, m[1] + m[3] - H - 96.0];
        let until = std::time::Instant::now() + std::time::Duration::from_secs_f32(OSD_SECS);
        match &mut self.flyout {
            // the one on screen is the same kind (a key held down): only its value moves
            Some(f) if f.kind == kind && !f.closing => {
                f.value = value;
                f.osd_until = Some(until);
                f.muted = muted;
                f.media = (media.0.to_string(), media.1.to_string());
            }
            _ => {
                let mut k = Spring::new(0.0, 420.0, 0.8);
                k.target = 1.0;
                self.flyout = Some(Flyout { kind, pos, k, dragging: false, value, closing: false, osd_until: Some(until), media: (media.0.to_string(), media.1.to_string()), muted });
            }
        }
    }

    pub fn close_flyout(&mut self) {
        if let Some(f) = &mut self.flyout {
            f.closing = true;
            f.dragging = false;
            f.k.target = 0.0;
        }
    }

    pub fn update_flyout(&mut self, dt: f32) {
        let an = self.theme.animations.clone();
        let Some(f) = &mut self.flyout else { return };
        an.step("notifications", &mut f.k, dt);
        if !f.closing && f.osd_until.is_some_and(|t| std::time::Instant::now() >= t) {
            f.closing = true;
            f.k.target = 0.0;
        }
        if f.closing && f.k.x < 0.01 && f.k.at_rest() {
            self.flyout = None;
        }
    }

    fn flyout_set(&mut self, mx: f32) {
        let Some(f) = &mut self.flyout else { return };
        let t = f.track();
        let before = f.value;
        f.value = ((mx - t[0]) / t[2]).clamp(0.0, 1.0);
        let ticked = (before * 20.0).round() != (f.value * 20.0).round() && f.kind == FlyKind::Volume;
        let (kind, v) = (f.kind, f.value);
        match kind {
            FlyKind::Volume => self.sys_watcher.send(Cmd::SetVolume(v)),
            FlyKind::Brightness => self.sys_watcher.send(Cmd::SetBrightness(v)),
            FlyKind::Media => {}
        }
        if ticked && self.flyout_last_tick.elapsed().as_millis() > 90 {
            self.flyout_last_tick = std::time::Instant::now();
            self.ui_sound(memrium_de::sound::Sound::Tick);
        }
    }

    /// The pointer went down: true when the flyout took the click. A click elsewhere closes it and carries on.
    pub fn flyout_press(&mut self, p: [f32; 2]) -> bool {
        let Some(f) = &self.flyout else { return false };
        if f.closing {
            return false;
        }
        let inside = p[0] >= f.pos[0] && p[0] <= f.pos[0] + W && p[1] >= f.pos[1] && p[1] <= f.pos[1] + H;
        if !inside || f.kind == FlyKind::Media {
            self.close_flyout();
            return false;
        }
        // the icon mutes (volume only)
        if f.kind == FlyKind::Volume && p[0] < f.pos[0] + 44.0 {
            self.sys_watcher.send(Cmd::ToggleMute);
            return true;
        }
        if let Some(f) = &mut self.flyout {
            f.dragging = true;
        }
        self.flyout_set(p[0]);
        true
    }

    pub fn flyout_release(&mut self) {
        if let Some(f) = &mut self.flyout {
            f.dragging = false;
        }
    }

    pub fn flyout_motion(&mut self, x: f32) {
        if self.flyout.as_ref().is_some_and(|f| f.dragging) {
            self.flyout_set(x);
        }
    }

    pub fn flyout_hit(&self, p: [f32; 2]) -> bool {
        self.flyout.as_ref().is_some_and(|f| p[0] >= f.pos[0] && p[0] <= f.pos[0] + W && p[1] >= f.pos[1] && p[1] <= f.pos[1] + H)
    }

    pub fn flyout_panels(&mut self, panels: &mut Vec<Panel>, labels: &mut Vec<Label>) {
        let Some(f) = &self.flyout else { return };
        let k = f.k.x.clamp(0.0, 1.0);
        let dark = self.theme.dark;
        let windows = self.theme.layout == memrium_de::theme::Layout::Taskbar;
        let ink = if dark { [0.96, 0.97, 1.0] } else { [0.10, 0.12, 0.18] };
        let tint = if dark { [0.09, 0.09, 0.09, 0.80] } else { [0.97, 0.98, 1.0, 0.78] };
        let fw = if f.kind == FlyKind::Media { 380.0 } else { W };
        let (cx, cy) = (f.pos[0] + fw / 2.0, f.pos[1] + H / 2.0 + (1.0 - k) * 8.0);
        let mut p = self.theme.glass.panel(cx, cy, fw, H, if windows { 8.0 } else { 20.0 }).tinted([tint[0], tint[1], tint[2], tint[3] * k]);
        p.extra[0] *= k;
        p.fx[0] = p.fx[0].max(26.0);
        if windows {
            p.params[3] = 0.0;
            p.fx = [p.fx[0].max(32.0), 0.0, 0.0, 0.0];
            p.color2 = if dark { [1.0, 1.0, 1.0, 0.10 * k] } else { [0.0, 0.0, 0.0, 0.10 * k] };
        }
        p.fade_by(k);
        panels.push(p);
        let icon = match f.kind {
            FlyKind::Volume if f.muted == Some(true) => "volume-x".to_string(),
            FlyKind::Volume if f.muted == Some(false) && self.sys.muted => memrium_de::widget::status_icon("volume", &memrium_de::sysctl::SysInfo { muted: false, volume: Some(f.value), ..self.sys.clone() }),
            FlyKind::Volume => memrium_de::widget::status_icon("volume", &self.sys),
            FlyKind::Brightness => "sun".to_string(),
            FlyKind::Media => f.media.0.clone(),
        };
        panels.push(Panel::glyph(f.pos[0] + 26.0, cy, 18.0, &icon, ink, k));
        if f.kind == FlyKind::Media {
            labels.push(Label::new(f.media.1.clone(), f.pos[0] + 54.0, cy + 5.0, 14.0, true, ink, k, Align::Left));
            return;
        }
        let t = f.track();
        let dy = cy - (f.pos[1] + H / 2.0);
        let (ty, th) = (t[1] + t[3] / 2.0 + dy, 4.0);
        let track = if dark { [1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0] };
        panels.push(Panel::solid(t[0] + t[2] / 2.0, ty, t[2], th, th / 2.0, track, track).flat().alpha(0.22 * k));
        let w = (t[2] * f.value).max(th);
        panels.push(Panel::solid(t[0] + w / 2.0, ty, w, th, th / 2.0, self.theme.accent, self.theme.accent).flat().alpha(k));
        let kx = t[0] + t[2] * f.value;
        let knob = if windows { 9.0 } else { 12.0 };
        panels.push(Panel::solid(kx, ty, knob, knob, knob / 2.0, [1.0; 3], [0.92, 0.94, 0.98]).flat().alpha(k));
        labels.push(Label::new(format!("{}%", (f.value * 100.0).round() as i32), f.pos[0] + W - 14.0, ty + 4.8, 13.0, true, ink, k, Align::Right));
    }
}
