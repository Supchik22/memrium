//! The power card of the battery icon: the power profile, the battery with its health, and whether the machine may
//! go to sleep or lock the screen (an app can keep it awake; so can you, by hand).

use crate::popup::Act;
use crate::state::State;
use chebrium::ui::Ui;
use chebrium::draw::Align;
use chebrium::style::Tone;

const W: f32 = 340.0;
const PAD: f32 = 18.0;

fn profile_label(p: &str) -> &'static str {
    match p {
        "power-saver" => "Saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        _ => "Other",
    }
}

fn profile_title(p: &str) -> &'static str {
    match p {
        "power-saver" => "Power Saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        _ => "Other",
    }
}

impl State {
    /// (height of the card, y of the battery block if any, y of the sleep block, y of its button), from the top.
    fn power_rows(&self) -> (f32, Option<f32>, f32, f32) {
        let mut y = 56.0;
        if !self.sys.profiles.is_empty() {
            y += 74.0;
        }
        let battery = self.sys.battery.map(|_| y);
        if battery.is_some() {
            y += 70.0;
        }
        let sleep = y;
        y += if self.manual_block || !self.inhibitors.is_empty() { 54.0 } else { 30.0 };
        (y + 34.0 + PAD, battery, sleep, y)
    }

    pub fn power_size(&self) -> [f32; 2] {
        [W, self.power_rows().0]
    }

    pub fn power_build(&self, ui: &mut Ui, pos: [f32; 2], acts: &mut Vec<Act>) {
        let (ink, accent) = (ui.ink, ui.accent);
        let (x, top) = (pos[0], pos[1]);
        let (left, right) = (x + PAD, x + W - PAD);
        let (_, battery_y, sleep_y, button_y) = self.power_rows();
        ui.glyph(left + 8.0, top + 29.0, 18.0, "gauge", ink, 0.85);
        ui.text(left + 26.0, top + 35.0, "Power and Battery", 15.0, true, ink, 1.0, Align::Left);

        // ---- profile: the same picker as in Settings
        if !self.sys.profiles.is_empty() {
            let order: Vec<&String> = self.sys.profiles.iter().rev().collect();
            let names: Vec<&str> = order.iter().map(|p| profile_label(p)).collect();
            let cur = self.sys.profile.as_deref().unwrap_or("");
            let mut sel = order.iter().position(|p| p.as_str() == cur).unwrap_or(0);
            ui.text(left, top + 68.0, "Power Profile", 13.0, false, ink, 0.75, Align::Left);
            ui.text(right, top + 68.0, profile_title(cur), 13.0, true, ink, 0.95, Align::Right);
            // with several accents the mode has a colour: green saves, blue balances, orange performs
            if ui.controls.multi {
                let tone = match cur { "power-saver" => Tone::Success, "performance" => Tone::Warning, _ => Tone::Primary };
                let dot = ui.tone(tone);
                let w = ui.measure(profile_title(cur), 13.0, true);
                ui.fill([right - w - 16.0, top + 62.0, 8.0, 8.0], 4.0, dot, 1.0);
            }
            if ui.segmented([left, top + 78.0, W - 2.0 * PAD, 32.0], &names, &mut sel) {
                acts.push(Act::SetProfile(order[sel].clone()));
            }
        }

        // ---- battery
        if let (Some(by), Some((pct, charging))) = (battery_y, self.sys.battery) {
            ui.text(left, top + by + 12.0, "Battery", 13.0, false, ink, 0.75, Align::Left);
            let state = if pct >= 100 { "Fully Charged" } else if charging { "Charging" } else { "On battery" };
            ui.text_raw(right, top + by + 12.0, &format!("{}  {pct}%", memrium_de::i18n::tr(state)), 13.0, true, ink, 0.95, Align::Right);
            let bw = W - 2.0 * PAD;
            ui.fill([left, top + by + 24.0, bw, 8.0], 4.0, ink, 0.14);
            let bar = if pct < 15 && !charging { ui.tone(Tone::Danger) } else if pct < 30 && !charging { ui.tone(Tone::Warning) } else if ui.controls.multi { ui.tone(Tone::Success) } else { accent };
            ui.fill([left, top + by + 24.0, (bw * pct as f32 / 100.0).max(8.0), 8.0], 4.0, bar, 1.0);
            if let Some(h) = self.sys.health {
                ui.text_raw(left, top + by + 56.0, &format!("{}: {h}%", memrium_de::i18n::tr("Battery health")), 12.0, false, ink, 0.6, Align::Left);
            }
        }

        // ---- sleep and screen locking
        let apps = self.inhibitors.len();
        let blocked = self.manual_block || apps > 0;
        ui.text(left, top + sleep_y + 12.0, "Sleep and Screen Locking", 13.0, false, ink, 0.75, Align::Left);
        ui.text(right, top + sleep_y + 12.0, if blocked { "Blocked" } else { "Allowed" }, 13.0, true, ink, 0.95, Align::Right);
        if blocked {
            let why = match (self.manual_block, apps) {
                (true, 0) => memrium_de::i18n::tr("Blocked manually").into_owned(),
                (true, n) => format!("{} · {n}", memrium_de::i18n::tr("Blocked manually")),
                (false, 1) => memrium_de::i18n::tr("An app is keeping the screen awake").into_owned(),
                (false, n) => format!("{n} {}", memrium_de::i18n::tr("apps are keeping the screen awake")),
            };
            ui.text_raw(left, top + sleep_y + 36.0, &why, 12.0, false, ink, 0.6, Align::Left);
        }
        // an app's block cannot be lifted from here, so the button only works while it is our own
        if self.manual_block || apps == 0 {
            if ui.button([left, top + button_y, W - 2.0 * PAD, 34.0], if self.manual_block { "Unblock" } else { "Manually Block" }, false) {
                acts.push(Act::ToggleBlock);
            }
        } else {
            ui.fill([left, top + button_y, W - 2.0 * PAD, 34.0], 8.0, ink, 0.06);
            ui.text(x + W / 2.0, top + button_y + 22.0, "Manually Block", 13.0, false, ink, 0.35, Align::Center);
        }
    }
}
