//! The shell's look for GTK programs: a stylesheet written from the theme, the way `appearance.toml` is for Chebrium
//! programs and the `chebrium` style (qtstyle/) is for Qt ones.
//!
//! Only in a Memrium session (see `kdecolors::active`) and while "Give them the shell's look" is on: a shell started inside
//! another desktop to try it must not change what that desktop's programs look like. When the session ends the import
//! lines are taken out again (`restore`).
//!
//! `gtk-memrium.css` lives in the shell's config directory; `~/.config/gtk-3.0/gtk.css` and `gtk-4.0/gtk.css` import it
//! (the one line is added when it is missing, nothing else of those files is touched). It names libadwaita's colours
//! (`accent_bg_color`, `window_bg_color` ...) and GTK 3's (`theme_bg_color` ...), so a libadwaita program takes the
//! palette whole, and draws buttons, fields, switches, sliders, scrollbars and menus the shape the controls of the theme
//! have. The colours match the Qt style's, so the three toolkits agree.

use crate::theme::{config_dir, ControlLook, DecoKind, Theme};
use std::path::Path;

type Rgb = [f32; 3];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn hex(c: Rgb) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// The stylesheet for a theme.
pub fn css(theme: &Theme) -> String {
    let dark = theme.dark;
    let c = |r: u8, g: u8, b: u8| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    let (window, view, fg) = if dark { (c(33, 33, 33), c(27, 27, 27), c(238, 238, 238)) } else { (c(247, 249, 255), c(255, 255, 255), c(28, 30, 36)) };
    let accent = theme.accent;
    let white = [1.0; 3];
    // a key's face, a field, a groove: a little off the window towards the ink
    let button = if dark { c(48, 48, 48) } else { c(229, 232, 240) };
    let field = mix(window, fg, if dark { 0.08 } else { 0.06 });
    let groove = mix(window, fg, if dark { 0.16 } else { 0.14 });
    let card = if dark { c(41, 41, 41) } else { white };
    let popover = if dark { c(43, 43, 43) } else { white };
    let ctl = &theme.ui.controls;
    let flat = ctl.look == ControlLook::Flat;
    let depth = if flat { 0.0 } else { ctl.depth.clamp(0.0, 1.5) };
    let round = ctl.round.clamp(0.0, 1.0);
    // pills when the controls are round all the way, a squarer corner below
    let key_r = if round >= 0.95 { "999px".to_string() } else { format!("{:.0}px", 4.0 + 12.0 * round) };
    let field_r = format!("{:.0}px", 5.0 + 7.0 * round);
    let win_r = theme.window.radius.clamp(0.0, 18.0);
    let shadow = if depth > 0.0 { format!("0 1px 1px alpha(black, {:.2})", (if dark { 0.30 } else { 0.12 }) * depth.min(1.0)) } else { "none".into() };
    let grad = |base: Rgb| if depth > 0.0 { format!("linear-gradient(to bottom, {}, {})", hex(mix(base, white, 0.05 * depth)), hex(mix(base, [0.0; 3], 0.02 * depth))) } else { "none".into() };
    let (btn_hover, btn_active) = (mix(button, if dark { white } else { accent }, if dark { 0.08 } else { 0.07 }), mix(button, if dark { white } else { [0.0; 3] }, if dark { 0.14 } else { 0.08 }));

    let names = [
        ("accent_color", mix(accent, fg, if dark { 0.0 } else { 0.15 })),
        ("accent_bg_color", accent),
        ("accent_fg_color", white),
        ("window_bg_color", window),
        ("window_fg_color", fg),
        ("view_bg_color", view),
        ("view_fg_color", fg),
        ("headerbar_bg_color", window),
        ("headerbar_fg_color", fg),
        ("headerbar_backdrop_color", window),
        ("headerbar_border_color", mix(window, fg, 0.10)),
        ("card_bg_color", card),
        ("card_fg_color", fg),
        ("popover_bg_color", popover),
        ("popover_fg_color", fg),
        ("dialog_bg_color", window),
        ("dialog_fg_color", fg),
        ("sidebar_bg_color", mix(window, fg, 0.03)),
        ("sidebar_fg_color", fg),
        ("sidebar_backdrop_color", window),
        ("theme_bg_color", window),
        ("theme_fg_color", fg),
        ("theme_base_color", view),
        ("theme_text_color", fg),
        ("theme_selected_bg_color", accent),
        ("theme_selected_fg_color", white),
        ("theme_unfocused_bg_color", window),
        ("theme_unfocused_fg_color", fg),
        ("theme_unfocused_base_color", view),
        ("theme_unfocused_text_color", fg),
        ("theme_unfocused_selected_bg_color", mix(accent, window, 0.3)),
        ("theme_unfocused_selected_fg_color", white),
        ("borders", mix(window, fg, 0.12)),
    ];
    let mut out = String::from("/* written by the shell from the current theme; changes here are overwritten */\n");
    for (n, v) in names {
        out += &format!("@define-color {n} {};\n", hex(v));
    }
    // popups are a little denser than windows; the shell frosts what is behind them when they are see-through
    let po = if theme.ui.popup_opacity > 0.0 { theme.ui.popup_opacity - 0.06 } else { theme.window.client_opacity };
    let pop_bg = if po >= 0.939 { hex(popover) } else { format!("rgba({}, {}, {}, {:.2})", (popover[0] * 255.0).round(), (popover[1] * 255.0).round(), (popover[2] * 255.0).round(), (po + 0.06).min(1.0)) };
    let (b, bh, ba, f, g, a) = (hex(button), hex(btn_hover), hex(btn_active), hex(field), hex(groove), hex(accent));
    let (bg_grad, bh_grad) = (grad(button), grad(btn_hover));
    out += &format!(
        r#"
/* keys */
button, button.flat:hover, .button {{
  background-color: {b};
  background-image: {bg_grad};
  border: none;
  border-radius: {key_r};
  box-shadow: {shadow};
  outline-offset: 2px;
}}
button:hover {{ background-color: {bh}; background-image: {bh_grad}; }}
button:active, button:checked {{ background-color: {ba}; background-image: none; box-shadow: none; }}
button:disabled {{ box-shadow: none; opacity: 0.55; }}
button.flat, button.image-button.flat, headerbar button.flat {{ background-color: transparent; background-image: none; box-shadow: none; }}
button.flat:hover {{ background-color: {bh}; }}
button.suggested-action, button.default, button.text-button.suggested-action {{ background-color: {a}; background-image: none; color: #ffffff; box-shadow: none; }}
button.suggested-action:hover {{ background-color: {ah}; }}
button.destructive-action {{ background-color: #e62d42; background-image: none; color: #ffffff; box-shadow: none; }}
button:focus-visible {{ outline: 2px solid alpha({a}, 0.6); outline-offset: 2px; }}

/* fields */
entry, spinbutton, searchbar entry, textview, .view {{ border-radius: {field_r}; }}
entry, spinbutton {{ background-color: {f}; background-image: none; border: none; box-shadow: none; }}
entry:focus, entry:focus-within, spinbutton:focus-within {{ box-shadow: inset 0 0 0 2px {a}; outline: none; }}
spinbutton button {{ background-image: none; box-shadow: none; border-radius: {field_r}; }}
combobox button, dropdown > button {{ border-radius: {key_r}; }}

/* check boxes and radio buttons */
check, radio {{ background-color: {f}; background-image: none; border: none; border-radius: 6px; min-width: 16px; min-height: 16px; box-shadow: none; }}
radio {{ border-radius: 999px; }}
check:checked, check:indeterminate, radio:checked {{ background-color: {a}; color: #ffffff; }}

/* switches */
switch {{ background-color: {g}; background-image: none; border: none; border-radius: 999px; box-shadow: none; }}
switch:checked {{ background-color: {a}; }}
switch slider {{ background-color: #ffffff; background-image: none; border: none; border-radius: 999px; box-shadow: 0 1px 3px alpha(black, 0.35); }}

/* sliders and bars */
scale trough, progressbar trough, levelbar trough {{ background-color: {g}; background-image: none; border: none; border-radius: 999px; min-height: 6px; min-width: 6px; box-shadow: none; }}
scale highlight, scale trough highlight, progressbar progress, levelbar block.filled {{ background-color: {a}; background-image: none; border: none; border-radius: 999px; }}
scale slider {{ background-color: #ffffff; background-image: none; border: 1px solid alpha(black, 0.18); border-radius: 999px; box-shadow: {shadow}; }}

/* scroll bars: thin, they widen under the pointer */
scrollbar, scrollbar trough {{ background: none; border: none; }}
scrollbar slider {{ background-color: alpha(@window_fg_color, 0.30); border-radius: 999px; }}
scrollbar slider:hover {{ background-color: alpha(@window_fg_color, 0.45); }}
scrollbar slider:active {{ background-color: alpha(@window_fg_color, 0.60); }}
scrollbar button {{ min-width: 0; min-height: 0; background: none; box-shadow: none; }}

/* menus, option lists and popovers */
popover > contents, popover.background > contents {{ background-color: {pop_bg}; border-radius: {pop_r}px; border: 1px solid alpha(@window_fg_color, 0.10); box-shadow: 0 6px 24px alpha(black, 0.30); }}
popover.menu > contents, dropdown popover > contents, combobox popover > contents {{ padding: 4px; }}
menu, .menu, .popup menu {{ background-color: {pop_bg}; border-radius: {pop_r}px; padding: 4px; border: 1px solid alpha(@window_fg_color, 0.10); }}
menuitem, modelbutton, popover modelbutton {{ border-radius: {field_r}; padding: 4px 10px; }}
menuitem:hover, modelbutton:hover, popover modelbutton:hover {{ background-color: {a}; color: #ffffff; }}
menuitem:hover *, modelbutton:hover * {{ color: #ffffff; }}
dropdown popover listview, dropdown popover list, combobox popover list {{ background: none; }}
dropdown popover listview > row, dropdown popover row, combobox popover row {{ border-radius: {field_r}; padding: 6px 10px; margin: 1px 0; min-height: 0; }}
dropdown popover listview > row:hover, dropdown popover row:hover {{ background-color: alpha(@window_fg_color, 0.08); }}
dropdown popover listview > row:selected, dropdown popover row:selected {{ background-color: {a}; color: #ffffff; }}
dropdown popover listview > row:selected:hover {{ background-color: {ah}; }}
dropdown popover searchbar entry, dropdown popover entry {{ margin: 0 0 4px 0; }}

/* lists and rows */
row:selected, row:selected:hover, iconview:selected, treeview.view:selected, listview > row:selected {{ background-color: {a}; color: #ffffff; }}
list row, listview > row {{ border-radius: {field_r}; }}

/* tool tips: a small round card, the popover colour, a soft shadow */
tooltip, tooltip.background {{ background-color: {pop_bg}; background-image: none; color: @popover_fg_color; border-radius: {field_r}; border: 1px solid alpha(@window_fg_color, 0.12); box-shadow: 0 4px 16px alpha(black, 0.28); }}
tooltip label, tooltip.background label {{ color: @popover_fg_color; }}
tooltip > box, tooltip.background > box {{ padding: 6px 10px; }}

/* tabs */
notebook > header > tabs > tab {{ border-radius: {field_r}; }}
notebook > header > tabs > tab:checked {{ background-color: {f}; }}

/* header bars and the windows a program draws itself */
headerbar, .titlebar {{ background-color: @headerbar_bg_color; background-image: none; box-shadow: inset 0 -1px alpha(@window_fg_color, 0.08); border: none; }}
{controls}
window.csd, decoration {{ border-radius: {win_r:.0}px; }}
window.csd.maximized, window.csd.fullscreen, window.csd.tiled {{ border-radius: 0; }}
"#,
        controls = window_controls(theme.window.decoration),
        ah = hex(mix(accent, white, 0.12)),
        pop_r = (win_r * 0.6).clamp(6.0, 14.0).round()
    );
    out
}

/// The window buttons of a program's own header bar (libadwaita's `windowcontrols`, GTK 3's `titlebutton`) drawn the way
/// the theme's windows draw theirs; `layout` below puts them on the matching side.
fn window_controls(kind: DecoKind) -> String {
    let both = |rule: &str| format!("windowcontrols button{rule}, headerbar button.titlebutton{rule}");
    match kind {
        DecoKind::Mac => format!(
            "/* traffic lights: three round colour dots, glyphs only while the pointer is over them */\n{} {{ min-width: 13px; min-height: 13px; padding: 0; margin: 0 4px; border-radius: 999px; border: none; box-shadow: inset 0 0 0 1px alpha(black, 0.12); background-image: none; color: transparent; -gtk-icon-size: 8px; }}\n{} {{ background-color: #ff5f57; }}\n{} {{ background-color: #febc2e; }}\n{} {{ background-color: #28c840; }}\nwindowcontrols:hover button, headerbar:hover button.titlebutton {{ color: alpha(black, 0.55); }}\nwindowcontrols:backdrop button, headerbar:backdrop button.titlebutton {{ background-color: alpha(@window_fg_color, 0.18); }}\nwindowcontrols button image, headerbar button.titlebutton image {{ margin: 0; padding: 0; }}\nwindowcontrols, headerbar windowcontrols {{ margin: 0 6px; }}\nheaderbar, .titlebar {{ box-shadow: none; }}",
            both(""), both(".close"), both(".minimize"), both(".maximize")
        ),
        DecoKind::Win11 => format!(
            "/* Fluent: flat 46 px keys, the close key goes red */\n{} {{ min-width: 46px; min-height: 32px; padding: 0; margin: 0; border-radius: 0; background-image: none; background-color: transparent; box-shadow: none; }}\n{} {{ background-color: alpha(@window_fg_color, 0.08); }}\n{} {{ background-color: #c42b1c; color: white; }}",
            both(""), both(":hover"), both(".close:hover")
        ),
        _ => String::new(),
    }
}

/// `gtk-decoration-layout` for a theme: where a program's own header bar puts its buttons.
fn layout(kind: DecoKind) -> Option<&'static str> {
    match kind {
        DecoKind::Mac => Some("close,minimize,maximize:"),
        DecoKind::Win11 | DecoKind::Aero => Some(":minimize,maximize,close"),
        DecoKind::Gnome => Some("appmenu:minimize,maximize,close"),
        _ => None,
    }
}

const LAYOUT_MARK: &str = "# memrium";

/// Put the theme's button layout in GTK 3 and 4's `settings.ini`. A layout the user wrote themselves (no mark) stays.
fn write_layout(kind: DecoKind) {
    let Some(value) = layout(kind) else {
        remove_layout();
        return;
    };
    let home = crate::gtk::config_home();
    for v in ["gtk-3.0", "gtk-4.0"] {
        let file = home.join(v).join("settings.ini");
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        let ours = format!("gtk-decoration-layout={value} {LAYOUT_MARK}");
        let mut lines: Vec<String> = Vec::new();
        let (mut done, mut foreign, mut in_settings) = (false, false, false);
        for l in old.lines() {
            let t = l.trim();
            if t.starts_with('[') {
                if in_settings && !done && !foreign {
                    lines.push(ours.clone());
                    done = true;
                }
                in_settings = t == "[Settings]";
            }
            if in_settings && t.starts_with("gtk-decoration-layout") {
                if t.ends_with(LAYOUT_MARK) {
                    lines.push(ours.clone());
                    done = true;
                    continue;
                }
                foreign = true;
            }
            lines.push(l.to_string());
        }
        if !done && !foreign {
            if !in_settings {
                lines.push("[Settings]".into());
            }
            lines.push(ours);
        }
        let text = lines.join("\n") + "\n";
        if text != old {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&file, text);
        }
    }
}

/// Take our layout line out of `settings.ini` again.
fn remove_layout() {
    let home = crate::gtk::config_home();
    for v in ["gtk-3.0", "gtk-4.0"] {
        let file = home.join(v).join("settings.ini");
        let Ok(old) = std::fs::read_to_string(&file) else { continue };
        if old.contains(LAYOUT_MARK) {
            let kept: Vec<&str> = old.lines().filter(|l| !l.trim_end().ends_with(LAYOUT_MARK)).collect();
            let _ = std::fs::write(&file, kept.join("\n") + "\n");
        }
    }
}

/// Write the stylesheet (only when it changed) and make sure GTK 3 and 4 import it.
pub fn export(theme: &Theme) {
    if !crate::kdecolors::active() || !theme.ui.style_apps {
        restore();
        return;
    }
    write_layout(theme.window.decoration);
    let text = css(theme);
    let dir = config_dir();
    let path = dir.join("gtk-memrium.css");
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let tmp = path.with_extension("css.tmp");
    if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
        let home = crate::gtk::config_home();
        for v in ["gtk-3.0", "gtk-4.0"] {
            import_line(&home.join(v).join("gtk.css"));
        }
    }
}

/// Take the import out of GTK 3 and 4's `gtk.css` (what they had before, without it).
pub fn restore() {
    remove_layout();
    crate::cursor_theme::restore_gtk();
    let home = crate::gtk::config_home();
    for v in ["gtk-3.0", "gtk-4.0"] {
        let file = home.join(v).join("gtk.css");
        let Ok(old) = std::fs::read_to_string(&file) else { continue };
        if !old.contains("gtk-memrium.css") {
            continue;
        }
        let kept: Vec<&str> = old.lines().filter(|l| !l.contains("gtk-memrium.css")).collect();
        let _ = std::fs::write(&file, kept.join("\n"));
    }
}

const IMPORT: &str = "@import url(\"../memrium/gtk-memrium.css\");";

/// Add the import to a user stylesheet. An import has to come before any rule, so it goes after the file's own imports.
fn import_line(file: &Path) {
    let old = std::fs::read_to_string(file).unwrap_or_default();
    if old.contains("gtk-memrium.css") {
        return;
    }
    let mut lines: Vec<&str> = old.lines().collect();
    let at = lines.iter().rposition(|l| l.trim_start().starts_with("@import")).map_or(0, |i| i + 1);
    lines.insert(at, IMPORT);
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, lines.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stylesheet_is_balanced_and_names_the_accent() {
        let mut t = crate::theme::preset("macos").unwrap();
        t.accent = [0.0, 0.5, 1.0];
        let s = css(&t);
        assert_eq!(s.matches('{').count(), s.matches('}').count());
        assert!(s.contains("@define-color accent_bg_color #0080ff;"));
        assert!(!s.contains("{{") && !s.contains("}}"), "format escapes left over");
    }

    #[test]
    fn the_import_goes_after_the_files_own_imports() {
        let dir = std::env::temp_dir().join(format!("gtkcss-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("gtk.css");
        std::fs::write(&f, "@import 'colors.css';").unwrap();
        import_line(&f);
        import_line(&f);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), format!("@import 'colors.css';\n{IMPORT}\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_button_layout_is_ours_to_remove_and_the_users_to_keep() {
        let dir = std::env::temp_dir().join(format!("memrium-gtkcss-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let file = dir.join("gtk-4.0/settings.ini");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "[Settings]\ngtk-application-prefer-dark-theme=1\n").unwrap();
        write_layout(DecoKind::Mac);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("gtk-decoration-layout=close,minimize,maximize: # memrium"), "{text}");
        write_layout(DecoKind::Win11);
        assert!(std::fs::read_to_string(&file).unwrap().contains(":minimize,maximize,close # memrium"));
        remove_layout();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[Settings]\ngtk-application-prefer-dark-theme=1\n");
        std::fs::write(&file, "[Settings]\ngtk-decoration-layout=menu:close\n").unwrap();
        write_layout(DecoKind::Mac);
        assert!(!std::fs::read_to_string(&file).unwrap().contains("memrium"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
