//! The General, Appearance and Keyboard pages of Settings.

use super::*;
use crate::theme::{ControlLook, ScrollClick, ScrollShow, SidebarSize};

fn group(ui: &mut Ui, r: [f32; 4]) {
    ui.fill(r, 12.0, ui.ink, 0.05);
}

fn separator(ui: &mut Ui, x: f32, y: f32, w: f32) {
    ui.fill([x + 16.0, y, w - 32.0, 1.0], 0.0, ui.ink, 0.09);
}

/// A round radio button with its label; true when it was clicked.
fn radio(ui: &mut Ui, x: f32, y: f32, w: f32, label: &str, on: bool) -> bool {
    let hot = ui.hit([x, y - 13.0, w, 26.0]);
    let key = ui.id() * 64;
    let t = ui.ease(key, if on { 1.0 } else { 0.0 }, 18.0);
    // the ring fills with the accent and the dot grows in (and shrinks out of the one that was left)
    ui.fill([x, y - 8.0, 16.0, 16.0], 8.0, ui.ink, (if hot { 0.30 } else { 0.20 }) * (1.0 - t));
    ui.fill([x + 1.5, y - 6.5, 13.0, 13.0], 6.5, if ui.dark { [0.189, 0.189, 0.189] } else { [1.0; 3] }, 1.0 - t);
    ui.fill([x, y - 8.0, 16.0, 16.0], 8.0, ui.accent, t);
    let dot = 6.0 * t;
    ui.fill([x + 8.0 - dot / 2.0, y - dot / 2.0, dot, dot], dot / 2.0, [1.0; 3], t);
    ui.text(x + 26.0, y + 4.5, label, 13.0, false, ui.ink, 0.92, Align::Left);
    ui.click && hot && !on
}

/// A tiny desktop: the wallpaper with a window in the light or dark style (or both halves, for Auto).
fn mode_card(ui: &mut Ui, r: [f32; 4], kind: u8, selected: bool) {
    if selected {
        ui.fill([r[0] - 3.0, r[1] - 3.0, r[2] + 6.0, r[3] + 6.0], 11.0, ui.accent, 1.0);
        ui.fill([r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0], 9.0, if ui.dark { [0.149, 0.149, 0.149] } else { [1.0; 3] }, 1.0);
    }
    ui.grad(r, 8.0, [0.36, 0.52, 0.96], [0.86, 0.42, 0.68], 1.0);
    let win = [r[0] + 10.0, r[1] + 16.0, r[2] - 20.0, r[3] - 16.0];
    let draw = |ui: &mut Ui, w: [f32; 4], dark: bool| {
        let (body, line) = if dark { ([0.161, 0.161, 0.161], [0.8; 3]) } else { ([0.97, 0.97, 1.0], [0.35; 3]) };
        ui.fill(w, 4.0, body, 1.0);
        ui.fill([w[0] + 5.0, w[1] + 6.0, w[2] * 0.5, 3.0], 1.5, line, 0.6);
        ui.fill([w[0] + 5.0, w[1] + 13.0, w[2] * 0.35, 3.0], 1.5, line, 0.35);
        for (i, c) in [[1.0, 0.37, 0.34], [1.0, 0.74, 0.18], [0.16, 0.78, 0.25]].iter().enumerate() {
            ui.fill([w[0] + w[2] - 20.0 + i as f32 * 6.0, w[1] + 4.0, 4.0, 4.0], 2.0, *c, 0.9);
        }
    };
    match kind {
        0 => draw(ui, win, false),
        1 => draw(ui, win, true),
        _ => {
            draw(ui, win, false);
            // the right half in the dark style
            let half = [win[0] + win[2] / 2.0, win[1], win[2] / 2.0, win[3]];
            ui.fill(half, 0.0, [0.14, 0.16, 0.23], 1.0);
            ui.fill([half[0] + 4.0, half[1] + 6.0, half[2] * 0.4, 3.0], 1.5, [0.8; 3], 0.6);
            ui.fill([half[0] + 4.0, half[1] + 13.0, half[2] * 0.3, 3.0], 1.5, [0.8; 3], 0.35);
        }
    }
}

/// A colour swatch that is a ring of hues (the "pick your own" button).
fn rainbow(ui: &mut Ui, cx: f32, cy: f32) {
    ui.fill([cx - 11.0, cy - 11.0, 22.0, 22.0], 11.0, [0.5; 3], 1.0);
    for i in 0..10 {
        let a = i as f32 / 10.0 * std::f32::consts::TAU;
        let c = theme::hsv_to_rgb(i as f32 / 10.0, 0.85, 1.0);
        ui.fill([cx + a.cos() * 6.0 - 5.0, cy + a.sin() * 6.0 - 5.0, 10.0, 10.0], 5.0, c, 1.0);
    }
    ui.fill([cx - 5.0, cy - 5.0, 10.0, 10.0], 5.0, [1.0; 3], 0.9);
}

pub(super) fn page_appearance(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, hsv: &mut Option<[f32; 3]>, open: &mut bool, gtk_note: &mut String) {
    ui.text(x, y + 52.0, "Appearance", 22.0, true, ui.ink, 0.97, Align::Left);
    let gy = y + 76.0;
    group(ui, [x, gy, cw, 314.0]);
    let (px, pw) = (x + 16.0, cw - 32.0);

    // ---- light / dark / auto
    ui.row(px, gy + 44.0, "Appearance");
    let current = if ctx.theme.auto_dark { 2 } else if ctx.theme.dark { 1 } else { 0 };
    for (i, name) in ["Light", "Dark", "Auto"].iter().enumerate() {
        let r = [x + cw - 16.0 - (3 - i) as f32 * 92.0 + 6.0, gy + 12.0, 80.0, 56.0];
        mode_card(ui, r, i as u8, current == i);
        ui.text(r[0] + r[2] / 2.0, r[1] + 76.0, name, 11.5, current == i, ui.ink, 0.9, Align::Center);
        if ui.click && ui.hit(r) && current != i {
            match i {
                0 => {
                    ctx.theme.auto_dark = false;
                    ctx.theme.dark = false;
                }
                1 => {
                    ctx.theme.auto_dark = false;
                    ctx.theme.dark = true;
                }
                _ => ctx.theme.auto_dark = true,
            }
            ui.changed = true;
        }
    }
    separator(ui, x, gy + 104.0, cw);

    // ---- accent colour
    let ay = gy + 104.0 + 34.0;
    ui.row(px, ay, "Accent colour");
    let accents: [(&str, [f32; 3]); 8] = [
        ("Blue", [0.04, 0.52, 1.0]),
        ("Purple", [0.55, 0.30, 0.85]),
        ("Pink", [0.95, 0.30, 0.60]),
        ("Red", [0.95, 0.28, 0.25]),
        ("Orange", [1.0, 0.58, 0.10]),
        ("Yellow", [1.0, 0.80, 0.10]),
        ("Green", [0.20, 0.72, 0.40]),
        ("Graphite", [0.45, 0.47, 0.52]),
    ];
    let slots = accents.len() + 1;
    let named = accents.iter().position(|(_, c)| (ctx.theme.accent[0] - c[0]).abs() < 0.01 && (ctx.theme.accent[1] - c[1]).abs() < 0.01 && (ctx.theme.accent[2] - c[2]).abs() < 0.01);
    let mut caption = String::from("Custom colour");
    for s in 0..slots {
        let cx = x + cw - 28.0 - (slots - 1 - s) as f32 * 32.0;
        let r = [cx - 12.0, ay - 12.0, 24.0, 24.0];
        let selected = if s == 0 { named.is_none() || *open } else { named == Some(s - 1) && !*open };
        // the ring around the chosen colour grows in, and shrinks away from the one that was left
        let key = ui.id() * 64;
        let t = ui.ease(key, if selected { 1.0 } else { 0.0 }, 16.0);
        if t > 0.01 {
            let grow = 3.0 * (0.4 + 0.6 * t);
            ui.fill([r[0] - grow, r[1] - grow, r[2] + 2.0 * grow, r[3] + 2.0 * grow], 12.0 + grow, ui.ink, 0.30 * t);
            let g2 = grow * 0.5;
            ui.fill([r[0] - g2, r[1] - g2, r[2] + 2.0 * g2, r[3] + 2.0 * g2], 12.0 + g2, if ui.dark { [0.189, 0.189, 0.189] } else { [1.0; 3] }, t);
        }
        if s == 0 {
            rainbow(ui, cx, ay);
        } else {
            ui.fill(r, 12.0, accents[s - 1].1, 1.0);
            if selected {
                caption = accents[s - 1].0.to_string();
            }
        }
        if ui.click && ui.hit(r) {
            if s == 0 {
                *open = !*open;
            } else {
                *open = false;
                ctx.theme.accent = accents[s - 1].1;
                ui.changed = true;
            }
        }
    }
    ui.text(x + cw - 28.0 - (slots as f32 - 1.0) * 16.0, ay + 34.0, &caption, 11.0, false, ui.ink, 0.6, Align::Center);
    separator(ui, x, gy + 104.0 + 74.0, cw);

    // ---- the rest of the group
    let mut f = Form::new(px, gy + 104.0 + 74.0 + 24.0, pw, 46.0);
    f.pick(ui, "Highlight colour", &mut ctx.theme.ui.highlight, &[(None, "Accent"), (Some([0.55, 0.57, 0.62]), "Graphite"), (Some([0.95, 0.72, 0.15]), "Gold")]);
    f.pick(ui, "Sidebar icon size", &mut ctx.theme.ui.sidebar, &[(SidebarSize::Small, "Small"), (SidebarSize::Medium, "Medium"), (SidebarSize::Large, "Large")]);
    f.toggle(ui, "Allow wallpaper tinting in windows", &mut ctx.theme.ui.wall_tint);

    // ---- scroll bars (or, while it is open, the custom colour editor in their place)
    let sy = gy + 314.0 + 16.0;
    if *open {
        group(ui, [x, sy, cw, 190.0]);
        accent_custom(ui, ctx, hsv, px, sy + 30.0, pw);
        return;
    }
    group(ui, [x, sy, cw, 232.0]);
    ui.row(px, sy + 24.0, "Show scroll bars");
    for (i, (v, label)) in [(ScrollShow::Auto, "Automatically based on mouse or trackpad"), (ScrollShow::Scrolling, "When scrolling"), (ScrollShow::Always, "Always")].into_iter().enumerate() {
        if radio(ui, px, sy + 54.0 + i as f32 * 28.0, pw, label, ctx.theme.ui.scrollbars == v) {
            ctx.theme.ui.scrollbars = v;
            ui.changed = true;
        }
    }
    separator(ui, x, sy + 124.0, cw);
    ui.row(px, sy + 150.0, "Click in the scroll bar to");
    for (i, (v, label)) in [(ScrollClick::Page, "Jump to the next page"), (ScrollClick::Spot, "Jump to the spot that's clicked")].into_iter().enumerate() {
        if radio(ui, px, sy + 178.0 + i as f32 * 28.0, pw, label, ctx.theme.ui.scroll_click == v) {
            ctx.theme.ui.scroll_click = v;
            ui.changed = true;
        }
    }

    // ---- how the controls look, and the GTK theme
    let cy = sy + 232.0 + 16.0;
    group(ui, [x, cy, cw, 418.0 + 3.0 * 46.0]);
    ui.section(px, cy + 26.0, "CONTROLS");
    let c = &mut ctx.theme.ui.controls;
    let mut f = Form::new(px, cy + 60.0, pw, 46.0);
    f.pick(ui, "Style", &mut c.look, &[(ControlLook::Keys, "Keys"), (ControlLook::Flat, "Flat")]);
    f.slider(ui, "Depth", &mut c.depth, 0.0, 1.5, "");
    f.slider(ui, "Roundness", &mut c.round, 0.0, 1.0, "");
    f.pick(ui, "Chosen segment", &mut c.accent_pick, &[(false, "Light"), (true, "Accent")]);
    f.toggle(ui, "Several accent colours", &mut c.multi);
    separator(ui, x, f.y - 14.0, cw);
    ui.section(px, f.y + 12.0, "GTK THEME");
    f.skip(30.0);
    ui.row(px, f.y, "Take light or dark and the accent");
    if ui.button([x + cw - 16.0 - 176.0, f.y - 15.0, 176.0, 30.0], "Take from the GTK theme", false) {
        *gtk_note = match crate::gtk::read() {
            Some(look) => {
                crate::gtk::apply(ctx.theme, &look);
                ui.changed = true;
                format!("{}: {}{}", look.name, if look.dark { "dark" } else { "light" }, if look.accent.is_some() { ", accent set" } else { ", no accent in this theme" })
            }
            None => "No GTK theme found".to_string(),
        };
    }
    f.skip(f.pitch);
    f.toggle(ui, "Keep following it", &mut ctx.theme.ui.follow_gtk);
    if !gtk_note.is_empty() {
        ui.text_raw(px, f.y - 12.0, gtk_note, 11.5, false, ui.ink, 0.6, Align::Left);
    }
    separator(ui, x, f.y - 14.0, cw);
    ui.section(px, f.y + 12.0, "OTHER PROGRAMS");
    f.skip(30.0);
    f.toggle(ui, "Give them the shell's look", &mut ctx.theme.ui.style_apps);
    f.toggle(ui, "Window frame follows the program", &mut ctx.theme.window.adapt);
    // the icon theme they get: a click goes to the next installed one
    ui.row(px, f.y, "Icon theme");
    let themes = crate::apps::installed_icon_themes();
    let label = ctx.theme.ui.icon_theme.clone().unwrap_or_else(|| "Unchanged".to_string());
    if ui.button([x + cw - 16.0 - 176.0, f.y - 15.0, 176.0, 30.0], &label, false) && !themes.is_empty() {
        let at = ctx.theme.ui.icon_theme.as_ref().and_then(|t| themes.iter().position(|n| n == t));
        ctx.theme.ui.icon_theme = match at {
            None => themes.first().cloned(),
            Some(i) if i + 1 < themes.len() => Some(themes[i + 1].clone()),
            Some(_) => None,
        };
        ui.changed = true;
    }
    f.skip(f.pitch);
}

pub(super) fn page_general(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>) {
    ui.text(x, y + 52.0, "General", 22.0, true, ui.ink, 0.97, Align::Left);
    // the language is not part of the theme: it goes to the shell as an action
    let mut choice: &str = ctx.language;
    let mut opts: Vec<(&str, &str)> = vec![("auto", "Auto")];
    opts.extend(crate::i18n::LANGUAGES.iter().copied());
    let mut f = Form::new(x, y + 100.0, cw, 40.0);
    if f.pick(ui, "Language", &mut choice, &opts) {
        actions.push(SettingsAction::SetLanguage(choice.to_string()));
    }

    f.pitch = 36.0;
    let cfg = &mut *ctx.cfg;
    ui.section(x, f.y - 2.0, "SCREENSHOTS");
    f.skip(22.0);
    let s = &mut cfg.screenshots;
    f.toggle(ui, "Flash", &mut s.flash);
    f.toggle(ui, "Notice in the corner", &mut s.notice);
    f.toggle(ui, "Shutter sound", &mut s.sound);
    f.slider(ui, "Volume", &mut s.volume, 0.0, 1.5, "");
    f.text(ui, "Folder", "shots-dir", &mut s.dir, None);
    ui.section(x, f.y - 2.0, "WIDGETS PANEL");
    f.skip(22.0);
    f.text(ui, "Weather city", "weather-city", &mut cfg.weather_city, Some("empty = off"));
}

/// How shortcuts are written in menus: what the Super key is called, whether Ctrl, Alt and Shift are symbols, and a
/// sample menu that shows it. Returns the height it took (from `y`).
fn labels_block(ui: &mut Ui, style: &mut crate::keys::KeyStyle, x: f32, y: f32, cw: f32) -> f32 {
    use crate::keys::{SuperLabel, SUPER_LABELS};
    ui.section(x, y, "SHORTCUT LABELS");
    let custom = style.super_label == SuperLabel::Custom;
    let gy = y + 12.0;
    let h = 52.0 + if custom { 44.0 } else { 0.0 } + 52.0;
    group(ui, [x, gy, cw, h]);
    let rx = x + cw - 16.0;
    ui.row(x + 16.0, gy + 31.0, "Super key is shown as");
    let labels: Vec<&str> = SUPER_LABELS.iter().map(|s| s.1).collect();
    let mut sel = SUPER_LABELS.iter().position(|(l, _)| *l == style.super_label).unwrap_or(0);
    if ui.segmented([rx - 300.0, gy + 12.0, 300.0, 28.0], &labels, &mut sel) {
        style.super_label = SUPER_LABELS[sel].0;
    }
    let mut cy = gy + 52.0;
    if custom {
        separator(ui, x, cy, cw);
        ui.row(x + 16.0, cy + 28.0, "Text");
        ui.text_field([rx - 200.0, cy + 9.0, 200.0, 26.0], "super-custom", &mut style.super_custom);
        cy += 44.0;
    }
    separator(ui, x, cy, cw);
    ui.row(x + 16.0, cy + 31.0, "Use ⌃ ⌥ ⇧ ␣ for Ctrl, Alt, Shift and Space");
    ui.toggle(rx - 44.0, cy + 14.0, &mut style.symbols);

    // ---- a sample menu, written the way the choice makes it
    let py = gy + h + 14.0;
    ui.text(x + 4.0, py + 8.0, "Preview", 12.0, true, ui.ink, 0.5, Align::Left);
    let rows = [("Open Files", "Super+E"), ("Open terminal", "Ctrl+Alt+T"), ("Search", "Super+S"), ("Previous window", "Alt+Tab")];
    let card = [x, py + 18.0, cw.min(360.0), rows.len() as f32 * 30.0 + 16.0];
    ui.fill(card, 12.0, if ui.dark { [0.18, 0.18, 0.18] } else { [1.0, 1.0, 1.0] }, 1.0);
    ui.fill(card, 12.0, ui.ink, 0.05);
    for (i, (name, chord)) in rows.iter().enumerate() {
        let ry = card[1] + 8.0 + i as f32 * 30.0;
        ui.text(card[0] + 16.0, ry + 20.0, name, 13.0, false, ui.ink, 0.92, Align::Left);
        ui.text_raw(card[0] + card[2] - 16.0, ry + 20.0, &style.show(chord), 12.5, false, ui.ink, 0.62, Align::Right);
    }
    py + 18.0 + card[3] + 30.0 - y
}

/// The keyboard layouts: the ones in use, in the order they are cycled, the ones that can be added and how to switch.
/// Writes into the input preferences; returns the height it took (from `y`).
fn layouts_block(ui: &mut Ui, input: &mut crate::input::InputPrefs, x: f32, y: f32, cw: f32) -> f32 {
    use crate::input::{join_layouts, layout_entries, layout_name, switch_index, with_switch, COMMON_LAYOUTS, SWITCH_KEYS};
    // what is in use now, whether it came from the config or from the system
    let (l, v, o) = crate::input::xkb_settings(input);
    let mut list = layout_entries(&l, &v);
    if list.is_empty() {
        list.push(("us".into(), String::new()));
    }
    let before = list.clone();
    let mut options = o.clone();
    ui.section(x, y, "LAYOUTS");
    let rows = list.len() as f32;
    let gy = y + 12.0;
    let h = rows * 40.0 + 8.0;
    group(ui, [x, gy, cw, h]);
    let rx = x + cw - 16.0;
    let (mut up, mut down, mut remove) = (None, None, None);
    let n = list.len();
    for (i, (code, var)) in list.iter().enumerate() {
        let cy = gy + 24.0 + i as f32 * 40.0;
        if i > 0 {
            separator(ui, x, cy - 20.0, cw);
        }
        let name = layout_name(code);
        ui.text_raw(x + 16.0, cy + 4.5, &name, 13.0, false, ui.ink, 0.92, Align::Left);
        let sub = if var.is_empty() { code.to_uppercase() } else { format!("{} · {var}", code.to_uppercase()) };
        let nw = ui.measure_raw(&name, 13.0, false);
        ui.text_raw(x + 16.0 + nw + 10.0, cy + 4.5, &sub, 11.5, false, ui.ink, 0.45, Align::Left);
        for (k, (icon, on)) in [("chevron-up", i > 0), ("chevron-down", i + 1 < n), ("x", n > 1)].into_iter().enumerate() {
            let r = [rx - 28.0 * (3 - k) as f32 + 4.0, cy - 12.0, 24.0, 24.0];
            let hov = on && ui.hit(r);
            if hov {
                ui.fill(r, 6.0, ui.ink, 0.10);
            }
            ui.glyph(r[0] + 12.0, r[1] + 12.0, 14.0, icon, ui.ink, if on { 0.8 } else { 0.2 });
            if on && hov && ui.click {
                match k {
                    0 => up = Some(i),
                    1 => down = Some(i),
                    _ => remove = Some(i),
                }
            }
        }
    }
    if let Some(i) = up {
        list.swap(i, i - 1);
    }
    if let Some(i) = down {
        list.swap(i, i + 1);
    }
    if let Some(i) = remove {
        list.remove(i);
    }

    // ---- what can be added
    let ay = gy + h + 14.0;
    ui.text(x + 4.0, ay + 8.0, "Add a layout", 12.0, true, ui.ink, 0.5, Align::Left);
    let (mut cx, mut cy) = (x, ay + 18.0);
    let free: Vec<&(&str, &str)> = COMMON_LAYOUTS.iter().filter(|(c, _)| !list.iter().any(|e| e.0 == *c)).collect();
    for (code, name) in free {
        let w = ui.measure_raw(name, 12.0, false) + 22.0;
        if cx + w > x + cw {
            cx = x;
            cy += 32.0;
        }
        let r = [cx, cy, w, 26.0];
        let hov = ui.hit(r);
        ui.fill(r, 13.0, ui.ink, if hov { 0.13 } else { 0.07 });
        ui.text_raw(cx + w / 2.0, cy + 17.5, name, 12.0, false, ui.ink, 0.9, Align::Center);
        if hov && ui.click {
            list.push((code.to_string(), String::new()));
        }
        cx += w + 8.0;
    }
    let sy = cy + 54.0;

    // ---- the switch
    ui.section(x, sy, "SWITCHING");
    let wy = sy + 12.0;
    group(ui, [x, wy, cw, 52.0]);
    ui.row(x + 16.0, wy + 31.0, "Switch with");
    let labels: Vec<&str> = SWITCH_KEYS.iter().map(|s| s.1).collect();
    let mut sel = switch_index(&options);
    if ui.segmented([rx - 470.0, wy + 12.0, 470.0, 28.0], &labels, &mut sel) {
        options = with_switch(&options, sel);
    }

    if list != before || options != o {
        let (nl, nv) = join_layouts(&list);
        input.xkb_layout = nl;
        input.xkb_variant = nv;
        input.xkb_options = options;
    }
    sy + 12.0 + 52.0 + 30.0 - y
}

pub(super) fn page_keys(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    use crate::keys::{normalize, ACTIONS};
    ui.text(x, y + 52.0, "Keyboard", 22.0, true, ui.ink, 0.97, Align::Left);
    let used = layouts_block(ui, &mut ctx.cfg.input, x, y + 94.0, cw);
    let y = y + used;
    let used = labels_block(ui, &mut ctx.cfg.keys.style, x, y + 94.0, cw);
    let y = y + used;
    ui.section(x, y + 94.0, "SHORTCUTS");
    ui.text(x, y + 120.0, "Shortcuts are written like Ctrl+Alt+T or Super+Up. Empty turns one off.", 12.0, false, ui.ink, 0.55, Align::Left);
    let y = y + 36.0;
    let keys = &mut ctx.cfg.keys;
    let half = (cw - 24.0) / 2.0;
    let per_col = ACTIONS.len().div_ceil(2);
    for (i, (id, label, _)) in ACTIONS.iter().enumerate() {
        let (col, row) = (i / per_col, i % per_col);
        let (cx, cy) = (x + col as f32 * (half + 24.0), y + 124.0 + row as f32 * 36.0);
        ui.text(cx, cy + 4.5, label, 12.0, false, ui.ink, 0.9, Align::Left);
        let mut chord = keys.chord(id);
        let before = chord.clone();
        if ui.text_field([cx + half - 138.0, cy - 13.0, 138.0, 26.0], &format!("key-{id}"), &mut chord) && chord != before {
            // keep what was typed, spelled the usual way
            keys.set(id, chord.trim().to_string());
        }
        // a shortcut used twice only works for the first action: say so
        let mine = normalize(&keys.chord(id));
        if !mine.is_empty() && ACTIONS.iter().any(|(o, ..)| o != id && normalize(&keys.chord(o)) == mine && ACTIONS.iter().position(|(a, ..)| a == o) < Some(i)) {
            ui.fill([cx + half - 148.0, cy - 5.0, 6.0, 10.0], 3.0, [0.95, 0.35, 0.3], 0.9);
        }
    }
    let by = y + 124.0 + per_col as f32 * 36.0 + 18.0;
    ui.section(x, by, "RUN A PROGRAM");
    while keys.commands.len() < 3 {
        keys.commands.push(Default::default());
    }
    for i in 0..3 {
        let cy = by + 30.0 + i as f32 * 38.0;
        let c = &mut keys.commands[i];
        ui.text_field([x, cy - 13.0, 150.0, 26.0], &format!("cmd-key-{i}"), &mut c.keys);
        ui.text_field([x + 162.0, cy - 13.0, cw - 162.0, 26.0], &format!("cmd-run-{i}"), &mut c.run);
    }
    ui.text(x, by + 30.0 + 3.0 * 38.0, "Shortcut on the left, command on the right.", 12.0, false, ui.ink, 0.55, Align::Left);
}

pub(super) fn page_mouse(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    use crate::cursor::{CursorLook, LOOKS};
    use crate::input::Accel;
    ui.text(x, y + 52.0, "Mouse", 22.0, true, ui.ink, 0.97, Align::Left);
    let input = &mut ctx.cfg.input;
    let mut changed = false;

    // ---- the cursor
    ui.section(x, y + 94.0, "CURSOR");
    let gy = y + 106.0;
    group(ui, [x, gy, cw, 168.0]);
    let (px, rx) = (x + 16.0, x + cw - 16.0);
    ui.row(px, gy + 30.0, "Size");
    let mut size = input.cursor.size;
    ui.slider([rx - 300.0, gy + 20.0, 240.0, 20.0], &mut size, 16.0, 64.0, " px");
    changed |= size != input.cursor.size;
    input.cursor.size = size.round();
    separator(ui, x, gy + 58.0, cw);
    ui.row(px, gy + 88.0, "Colour");
    let mut sel = LOOKS.iter().position(|(l, _)| *l == input.cursor.look).unwrap_or(0);
    let names: Vec<&str> = LOOKS.iter().map(|(_, n)| *n).collect();
    if ui.segmented([rx - 252.0, gy + 74.0, 252.0, 28.0], &names, &mut sel) {
        input.cursor.look = LOOKS[sel].0;
        changed = true;
    }
    separator(ui, x, gy + 116.0, cw);
    ui.row(px, gy + 142.0, "Change the cursor over edges, buttons and while a program starts");
    ui.toggle(rx - 44.0, gy + 130.0, &mut input.cursor.dynamic);

    // ---- the pointer
    ui.section(x, gy + 200.0, "POINTER");
    let py = gy + 212.0;
    group(ui, [x, py, cw, 232.0]);
    ui.row(px, py + 30.0, "Speed");
    ui.slider([rx - 300.0, py + 20.0, 240.0, 20.0], &mut input.pointer_speed, -1.0, 1.0, "");
    separator(ui, x, py + 58.0, cw);
    ui.row(px, py + 88.0, "Acceleration");
    let mut acc = if input.accel == Accel::Flat { 1 } else { 0 };
    if ui.segmented([rx - 190.0, py + 74.0, 190.0, 28.0], &["Adaptive", "Flat"], &mut acc) {
        input.accel = if acc == 1 { Accel::Flat } else { Accel::Adaptive };
    }
    separator(ui, x, py + 116.0, cw);
    ui.row(px, py + 142.0, "Natural scrolling");
    ui.toggle(rx - 44.0, py + 130.0, &mut input.natural_scroll);
    ui.row(px, py + 178.0, "Tap to click");
    ui.toggle(rx - 44.0, py + 166.0, &mut input.tap_to_click);
    ui.row(px, py + 214.0, "Left-handed");
    ui.toggle(rx - 44.0, py + 202.0, &mut input.left_handed);
    let _ = (changed, CursorLook::Light);
}

pub(super) fn page_themes(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>, share: &mut (String, String)) {
    ui.text(x, y + 52.0, "Themes", 22.0, true, ui.ink, 0.97, Align::Left);
    ui.section(x, y + 86.0, "TEMPLATES");
    let gap = 14.0;
    let cardw = (cw - gap * 2.0) / 3.0;
    for (i, (id, name)) in ctx.presets.iter().take(9).enumerate() {
        let (col, row) = (i % 3, i / 3);
        let r = [x + col as f32 * (cardw + gap), y + 98.0 + row as f32 * 128.0, cardw, 116.0];
        let selected = id == ctx.preset_id;
        let hov = ui.hit(r);
        ui.fill(r, 12.0, ui.ink, if hov { 0.10 } else { 0.06 });
        if selected {
            ui.fill([r[0] - 2.0, r[1] - 2.0, r[2] + 4.0, r[3] + 4.0], 14.0, ui.accent, 0.9);
            ui.fill(r, 12.0, if ui.dark { [0.149, 0.149, 0.149] } else { [0.96, 0.97, 1.0] }, 1.0);
        }
        mini_preview(ui, [r[0] + 8.0, r[1] + 8.0, r[2] - 16.0, 74.0], id);
        ui.text(r[0] + r[2] / 2.0, r[1] + 102.0, name, 12.5, selected, ui.ink, 0.95, Align::Center);
        if ui.click && hov && !selected {
            actions.push(SettingsAction::Preset(id.clone()));
        }
    }
    let rows = ((ctx.presets.len().min(9) + 2) / 3).max(1) as f32;
    let by = y + 98.0 + rows * 128.0 + 18.0;
    ui.section(x, by, "SHARE");
    let btn_w = 200.0;
    let field_w = cw - 110.0 - btn_w - 12.0;
    let mut yy = by + 34.0;
    ui.row(x, yy, "Name");
    ui.text_field([x + 110.0, yy - 13.0, field_w, 26.0], "share-name", &mut share.0);
    if ui.button([x + cw - btn_w, yy - 14.0, btn_w, 28.0], "Save this look as a template", true) && !share.0.trim().is_empty() {
        actions.push(SettingsAction::SaveTemplate(share.0.trim().to_string()));
    }
    yy += 44.0;
    ui.row(x, yy, "Theme file");
    ui.text_field([x + 110.0, yy - 13.0, field_w, 26.0], "share-file", &mut share.1);
    if ui.button([x + cw - btn_w, yy - 14.0, btn_w, 28.0], "Import a theme file", false) && !share.1.trim().is_empty() {
        actions.push(SettingsAction::ImportTemplate(share.1.trim().to_string()));
    }
    let mut f = Form::new(x, yy + 36.0, cw, 42.0);
    f.note(ui, "Templates are plain files in ~/.config/memrium/themes: copy one to share it.", true);
}


pub(super) fn page_displays(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, actions: &mut Vec<SettingsAction>, sel: &mut usize) {
    use crate::display::{self, mode_text, Side};
    ui.text(x, y + 52.0, "Displays", 22.0, true, ui.ink, 0.97, Align::Left);
    if ctx.monitors.is_empty() {
        ui.text(x, y + 100.0, "The monitors are managed by the session. Run the shell from a TTY to arrange them.", 13.0, false, ui.ink, 0.7, Align::Left);
        return;
    }
    let monitors = ctx.monitors;
    let prefs = &mut ctx.cfg.display;
    let placed = display::place(monitors, prefs);

    // ---- the arrangement: each monitor as a rectangle, to scale
    ui.section(x, y + 86.0, "ARRANGEMENT");
    let area = [x, y + 100.0, cw, 170.0];
    ui.fill(area, 12.0, ui.ink, 0.05);
    let (bx0, by0, bx1, by1) = display::bounds(&placed);
    let (bw, bh) = ((bx1 - bx0).max(1) as f32, (by1 - by0).max(1) as f32);
    let k = ((area[2] - 40.0) / bw).min((area[3] - 40.0) / bh).min(0.12);
    let (ox, oy) = (area[0] + (area[2] - bw * k) / 2.0, area[1] + (area[3] - bh * k) / 2.0);
    *sel = (*sel).min(monitors.len().saturating_sub(1));
    for p in &placed {
        let r = [ox + (p.x - bx0) as f32 * k, oy + (p.y - by0) as f32 * k, p.w as f32 * k, p.h as f32 * k];
        let idx = monitors.iter().position(|m| m.name == p.name).unwrap_or(0);
        let on = *sel == idx;
        let hov = ui.hit(r);
        if on {
            ui.fill([r[0] - 2.0, r[1] - 2.0, r[2] + 4.0, r[3] + 4.0], 7.0, ui.accent, 1.0);
        }
        ui.grad(r, 5.0, [0.36, 0.52, 0.96], [0.62, 0.40, 0.80], if hov { 0.9 } else { 1.0 });
        if p.primary {
            ui.fill([r[0], r[1], r[2], 5.0], 3.0, [1.0; 3], 0.85);
        }
        ui.text(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 4.5, &p.name, 12.0, true, [1.0; 3], 0.95, Align::Center);
        if ui.click && hov {
            *sel = idx;
        }
    }

    // ---- the selected monitor
    let m = &monitors[*sel];
    let is_main = placed.iter().any(|p| p.primary && p.name == m.name);
    let enabled = !prefs.disabled.contains(&m.name);
    ui.text(x, y + 288.0, &m.name, 11.5, true, ui.ink, 0.55, Align::Left);
    let mut f = Form::new(x, y + 322.0, cw, 42.0);
    // resolution and refresh rate
    let cur = display::chosen_mode(m, prefs);
    let at = m.modes.iter().position(|c| *c == cur).unwrap_or(0);
    let step = f.stepper(ui, "Resolution", &format!("{}×{} · {} Hz", cur.0, cur.1, cur.2));
    if step != 0 && !m.modes.is_empty() {
        let n = (at as i32 + step).rem_euclid(m.modes.len() as i32) as usize;
        let text = mode_text(m.modes[n]);
        if n == 0 {
            prefs.modes.remove(&m.name);
        } else {
            prefs.modes.insert(m.name.clone(), text);
        }
        ui.changed = true;
    }
    if is_main {
        ui.row(f.x, f.y, "Main display");
        ui.text(f.x + f.cw, f.y + 4.5, "this monitor carries the menu bar and the Dock", 12.0, false, ui.ink, 0.6, Align::Right);
        f.y += f.pitch;
    } else {
        let mut on = enabled;
        f.toggle(ui, "Use this display", &mut on);
        if on != enabled {
            if on {
                prefs.disabled.retain(|n| n != &m.name);
            } else {
                prefs.disabled.push(m.name.clone());
            }
            ui.changed = true;
        }
        ui.row(f.x, f.y, "Make it the main display");
        if ui.button([f.x + f.cw - 130.0, f.y - 14.0, 130.0, 28.0], "Make main", false) {
            prefs.primary = m.name.clone();
            prefs.disabled.retain(|n| n != &m.name);
            ui.changed = true;
        }
        f.y += f.pitch;
    }
    f.skip(8.0);
    ui.section(x, f.y, "ALL DISPLAYS");
    f.skip(26.0);
    f.pick(ui, "Other displays are", &mut prefs.side, &[(Side::Left, "Left"), (Side::Right, "Right")]);
    let mut scale = prefs.scale;
    f.pick(ui, "Interface size", &mut scale, &[(0.0, "Auto"), (1.0, "100%"), (1.25, "125%"), (1.5, "150%"), (2.0, "200%")]);
    if scale != prefs.scale {
        prefs.scale = scale;
    }
    f.skip(8.0);
    if ui.button([x + cw - 140.0, f.y - 14.0, 140.0, 32.0], "Apply", true) {
        actions.push(SettingsAction::ApplyDisplays);
    }
    ui.text(x, f.y + 6.0, "Changes take effect when you press Apply.", 12.0, false, ui.ink, 0.55, Align::Left);
}
