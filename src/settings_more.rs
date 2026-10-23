//! More pages of Settings: Notifications, Date & Time, Sound, Network and Power & Battery. Each page is a few groups,
//! each group a card with a small heading above it.

use super::*;
use crate::sysctl::Cmd;
use crate::theme::Corner;

/// A card for a group of settings, with its heading above; returns where its first row goes (the card is `rows` rows
/// of `pitch` tall, and the next group starts at the returned y plus that).
fn card(ui: &mut Ui, x: f32, y: f32, w: f32, heading: &str, height: f32) -> f32 {
    ui.section(x + 4.0, y + 12.0, heading);
    ui.fill([x, y + 22.0, w, height], 12.0, ui.ink, 0.05);
    y + 22.0
}

fn rule(ui: &mut Ui, x: f32, y: f32, w: f32) {
    ui.fill([x + 16.0, y, w - 32.0, 1.0], 0.0, ui.ink, 0.09);
}

fn title(ui: &mut Ui, x: f32, y: f32, name: &str) {
    ui.text(x, y + 52.0, name, 22.0, true, ui.ink, 0.97, Align::Left);
}

pub(super) fn page_notifications(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>) {
    title(ui, x, y, "Notifications");
    let n = &mut ctx.cfg.notifications;
    let top = card(ui, x, y + 70.0, cw, "DELIVERY", 112.0);
    let mut f = Form::new(x + 16.0, top + 32.0, cw - 32.0, 46.0);
    f.toggle(ui, "Do not disturb", &mut n.dnd);
    rule(ui, x, top + 56.0, cw);
    f.note(ui, "Only urgent notifications are shown while it is on.", true);

    let top = card(ui, x, top + 112.0 + 26.0, cw, "POSITION", 104.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    f.pick(ui, "Corner", &mut n.corner, &[(Corner::Auto, "Auto"), (Corner::TopRight, "Top right"), (Corner::TopLeft, "Top left"), (Corner::BottomRight, "Bottom right"), (Corner::BottomLeft, "Bottom left")]);
    f.note(ui, "Auto puts them under a top bar, or in the bottom corner over a taskbar.", true);

    let top = card(ui, x, top + 104.0 + 26.0, cw, "HOW LONG", 130.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    f.slider(ui, "Stay for", &mut n.seconds, 2.0, 20.0, " s");
    n.seconds = n.seconds.round();
    let mut most = n.max_shown as f32;
    f.slider(ui, "Show at most", &mut most, 1.0, 8.0, "");
    n.max_shown = most.round().max(1.0) as u32;

    let by = top + 130.0 + 26.0;
    if ui.button([x, by, 220.0, 34.0], "Show a test notification", false) {
        actions.push(SettingsAction::TestNotification);
    }
}

pub(super) fn page_datetime(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    title(ui, x, y, "Date & Time");
    let c = &mut ctx.theme.ui.clock;
    // what the panel's clock looks like now
    let top = card(ui, x, y + 70.0, cw, "THE CLOCK IN THE PANEL", 96.0);
    let shown = crate::widget::format_time(&c.format());
    ui.text_raw(x + 20.0, top + 58.0, &shown, 26.0, false, ui.ink, 0.95, Align::Left);
    ui.text(x + cw - 20.0, top + 56.0, "A clock with a format of its own in Panels keeps it.", 11.5, false, ui.ink, 0.5, Align::Right);

    let top = card(ui, x, top + 96.0 + 26.0, cw, "FORMAT", 192.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    f.toggle(ui, "24-hour time", &mut c.hour24);
    f.toggle(ui, "Show seconds", &mut c.seconds);
    f.toggle(ui, "Show the day and date", &mut c.date);

    let top = card(ui, x, top + 192.0 + 26.0, cw, "CALENDAR", 76.0);
    let mut f = Form::new(x + 16.0, top + 38.0, cw - 32.0, 46.0);
    f.pick(ui, "The week starts on", &mut c.week_monday, &[(true, "Monday"), (false, "Sunday")]);
}

pub(super) fn page_sound(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>) {
    title(ui, x, y, "Sound");
    let top = card(ui, x, y + 70.0, cw, "OUTPUT", 148.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    match ctx.sys.volume {
        Some(v) => {
            let mut pct = (v.min(1.5) * 100.0).round();
            let before = pct;
            f.slider(ui, "Volume", &mut pct, 0.0, 100.0, "%");
            if pct != before {
                actions.push(SettingsAction::Sys(Cmd::SetVolume(pct / 100.0)));
            }
            let mut muted = ctx.sys.muted;
            f.toggle(ui, "Mute", &mut muted);
            if muted != ctx.sys.muted {
                actions.push(SettingsAction::Sys(Cmd::ToggleMute));
            }
            f.note(ui, "Choose the output device, and the volume of each app, from the sound icon on the panel.", true);
        }
        None => f.note(ui, "No sound server found (PipeWire with wpctl).", false),
    }

    let a = &mut ctx.theme.animations;
    let top = card(ui, x, top + 148.0 + 26.0, cw, "INTERFACE SOUNDS", 110.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    f.toggle(ui, "Play sounds for the interface", &mut a.sounds);
    f.slider(ui, "Their volume", &mut a.sound_volume, 0.0, 1.0, "");
}

pub(super) fn page_network(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>) {
    title(ui, x, y, "Network");
    let top = card(ui, x, y + 70.0, cw, "WI-FI", 120.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    match &ctx.sys.wifi {
        Some(w) => {
            let mut on = w.enabled;
            f.toggle(ui, "Wi-Fi", &mut on);
            if on != w.enabled {
                actions.push(SettingsAction::Sys(Cmd::SetWifi(on)));
            }
            let line = match (&w.ssid, w.enabled) {
                (Some(s), true) => format!("{}  {}", crate::i18n::tr("Connected to"), s),
                (None, true) => crate::i18n::tr("Not connected").into_owned(),
                _ => crate::i18n::tr("Off").into_owned(),
            };
            ui.text_raw(x + 16.0, f.y - 8.0, &line, 12.5, false, ui.ink, 0.65, Align::Left);
        }
        None => f.note(ui, "No Wi-Fi adapter found.", false),
    }

    let top = card(ui, x, top + 120.0 + 26.0, cw, "BLUETOOTH", 96.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    match &ctx.sys.bluetooth {
        Some(b) => {
            let mut on = b.powered;
            f.toggle(ui, "Bluetooth", &mut on);
            if on != b.powered {
                actions.push(SettingsAction::Sys(Cmd::SetBluetooth(on)));
            }
            if b.powered {
                ui.text_raw(x + 16.0, f.y - 8.0, &format!("{} {}", b.connected, crate::i18n::tr("devices connected")), 12.5, false, ui.ink, 0.65, Align::Left);
            }
        }
        None => f.note(ui, "No Bluetooth adapter found.", false),
    }
    ui.text(x + 4.0, top + 96.0 + 30.0, "Join a network, or leave one, from the Wi-Fi icon on the panel.", 12.0, false, ui.ink, 0.55, Align::Left);
}

pub(super) fn page_power(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>) {
    title(ui, x, y, "Power & Battery");
    let mut cy = y + 70.0;
    // the profile, when the machine has any
    if !ctx.sys.profiles.is_empty() {
        let top = card(ui, x, cy, cw, "POWER PROFILE", 92.0);
        let mut f = Form::new(x + 16.0, top + 38.0, cw - 32.0, 46.0);
        let names: Vec<&str> = ctx.sys.profiles.iter().rev().map(|p| match p.as_str() { "power-saver" => "Power saver", "balanced" => "Balanced", "performance" => "Performance", _ => "Other" }).collect();
        let order: Vec<&String> = ctx.sys.profiles.iter().rev().collect();
        let cur = ctx.sys.profile.clone().unwrap_or_default();
        let mut sel = order.iter().position(|p| **p == cur).unwrap_or(0);
        ui.row(f.x, f.y, "Mode");
        let w = (f.cw - 150.0).min(130.0 * names.len() as f32);
        if ui.segmented([f.x + f.cw - w, f.y - 15.0, w, 30.0], &names, &mut sel) {
            actions.push(SettingsAction::Sys(Cmd::SetProfile(order[sel].clone())));
        }
        f.skip(f.pitch);
        cy = top + 92.0 + 26.0;
    }
    // the battery
    if let Some((pct, charging)) = ctx.sys.battery {
        let top = card(ui, x, cy, cw, "BATTERY", 104.0);
        let (px, bw) = (x + 20.0, cw - 40.0);
        ui.text(px, top + 34.0, if pct >= 100 { "Fully charged" } else if charging { "Charging" } else { "On battery" }, 13.0, false, ui.ink, 0.8, Align::Left);
        ui.text_raw(x + cw - 20.0, top + 34.0, &format!("{pct}%"), 13.0, true, ui.ink, 0.95, Align::Right);
        let colour = if pct < 15 && !charging { ui.tone(chebrium::style::Tone::Danger) } else if pct < 30 && !charging { ui.tone(chebrium::style::Tone::Warning) } else if ui.controls.multi { ui.tone(chebrium::style::Tone::Success) } else { ui.accent };
        ui.fill([px, top + 46.0, bw, 8.0], 4.0, ui.ink, 0.14);
        ui.fill([px, top + 46.0, (bw * pct as f32 / 100.0).max(8.0), 8.0], 4.0, colour, 1.0);
        if let Some(h) = ctx.sys.health {
            ui.text_raw(px, top + 82.0, &format!("{}: {h}%", crate::i18n::tr("Battery health")), 12.0, false, ui.ink, 0.6, Align::Left);
        }
        cy = top + 104.0 + 26.0;
    }
    // when the computer is not used
    let top = card(ui, x, cy, cw, "WHEN THE COMPUTER IS NOT USED", 196.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 42.0);
    let cfg = &mut *ctx.cfg;
    for (label, v) in [("Lock screen after", &mut cfg.power.lock_min), ("Screen off after", &mut cfg.power.screen_off_min), ("Suspend after", &mut cfg.power.suspend_min)] {
        let mut m = v.round();
        f.slider(ui, label, &mut m, 0.0, 60.0, " min");
        *v = m;
    }
    f.note(ui, "0 means never. Anything that keeps the screen awake (a video) postpones all three.", true);
}

/// The installed programs, read once when the Programs page is first shown.
fn installed_apps() -> &'static [crate::apps::AppEntry] {
    static APPS: std::sync::OnceLock<Vec<crate::apps::AppEntry>> = std::sync::OnceLock::new();
    APPS.get_or_init(|| crate::apps::scan().into_iter().filter(|a| a.id != crate::apps::SETTINGS_ID && a.id != crate::apps::FILES_ID).collect())
}

/// How many installed programs the picker lists at once (the field above narrows them).
const PICK_ROWS: usize = 6;

/// Programs: how the popups and lists of other programs look, and a few rules for one program at a time.
pub(super) fn page_programs(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, rules: &mut crate::apprules::Rules, new_name: &mut String) {
    title(ui, x, y, "Programs");
    let top = card(ui, x, y + 70.0, cw, "POPUPS AND LISTS OF OTHER PROGRAMS", 150.0);
    let mut f = Form::new(x + 16.0, top + 34.0, cw - 32.0, 46.0);
    f.slider(ui, "Popup density", &mut ctx.theme.ui.popup_opacity, 0.0, 1.0, "");
    f.toggle(ui, "Slide the highlight in menus and lists", &mut ctx.theme.ui.slide);
    f.note(ui, "Density 0 follows the windows. Qt programs, GTK ones and Kate take these in a Memrium session.", true);

    // one card per rule: opacity, frost, a light or dark look, a font
    const ROW: f32 = 42.0;
    const RULE_H: f32 = 62.0 + 4.0 * ROW;
    let before: Vec<crate::apprules::Rule> = rules.rules().to_vec();
    let n = before.len();
    let top = card(ui, x, top + 150.0 + 26.0, cw, "RULES FOR ONE PROGRAM", 30.0 + 52.0 + n as f32 * RULE_H + 60.0 + 20.0 + PICK_ROWS as f32 * 34.0);
    ui.text(x + 16.0, top + 28.0, "Filed under part of the program's app id. Opacity and frost apply at once; the look and the font", 11.5, false, ui.ink, 0.55, Align::Left);
    ui.text(x + 16.0, top + 42.0, "when the program is started from the shell.", 11.5, false, ui.ink, 0.55, Align::Left);
    let mut gone = None;
    let mut y0 = top + 58.0;
    for i in 0..n {
        ui.text_raw(x + 20.0, y0 + 20.0, &before[i].name, 14.0, true, ui.ink, 0.95, Align::Left);
        if ui.button([x + cw - 16.0 - 92.0, y0 + 2.0, 92.0, 28.0], "Remove", false) {
            gone = Some(i);
        }
        let mut f = Form::new(x + 16.0, y0 + 56.0, cw - 32.0, ROW);
        let r = &mut rules.rules_mut()[i];
        let mut op = r.opacity.unwrap_or(1.0);
        f.slider(ui, "Opacity", &mut op, 0.2, 1.0, "");
        r.opacity = if op >= 0.999 { None } else { Some((op * 100.0).round() / 100.0) };
        f.pick(ui, "Frost behind it", &mut r.frost, &[(None, "Auto"), (Some(true), "On"), (Some(false), "Off")]);
        f.pick(ui, "Look", &mut r.dark, &[(None, "Auto"), (Some(false), "Light"), (Some(true), "Dark")]);
        let mut font = r.font.clone().unwrap_or_default();
        if f.text(ui, "Font", &format!("rule-font-{i}"), &mut font, Some("Enter to apply")) {
            r.font = if font.trim().is_empty() { None } else { Some(font.trim().to_string()) };
        }
        y0 += RULE_H;
        if i + 1 < n {
            rule(ui, x, y0 - 10.0, cw);
        }
    }
    let mut f = Form::new(x + 16.0, y0 + 30.0, cw - 32.0, ROW);
    if f.text(ui, "Add a rule for", "rule-new", new_name, Some("Search programs, or type a name and press Enter")) && !new_name.trim().is_empty() {
        rules.add(new_name);
        new_name.clear();
    }
    // the installed programs that fit what is typed: a click files a rule under that program
    let mut py = y0 + 30.0 + ROW + 6.0;
    let q = new_name.trim().to_lowercase();
    let mut picked = None;
    for a in installed_apps().iter().filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q)).take(PICK_ROWS) {
        let key = if a.wm_class.is_empty() { a.id.clone() } else { a.wm_class.clone() };
        let have = rules.rules().iter().any(|r| r.name == key.to_lowercase());
        match (ctx.app_icon)(&a.id) {
            Some(cell) => ui.d.image([x + 18.0, py + 3.0, 26.0, 26.0], 0.0, cell),
            None => ctx.want_icons.push(a.id.clone()),
        }
        ui.text_raw(x + 54.0, py + 19.0, &a.name, 13.0, false, ui.ink, 0.9, Align::Left);
        if ui.button([x + cw - 16.0 - 92.0, py + 2.0, 92.0, 28.0], if have { "Added" } else { "Add" }, false) && !have {
            picked = Some(key);
        }
        py += 34.0;
    }
    if let Some(key) = picked {
        rules.add(&key);
        new_name.clear();
    }
    if let Some(i) = gone {
        rules.remove(i);
    }
    // saved when something changed; a program that is already running keeps what it was started with
    if rules.rules() != before.as_slice() {
        rules.save();
    }
}
