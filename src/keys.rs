//! Keyboard shortcuts the shell handles itself, and the user's changes to them.
//!
//! A shortcut is written like `Ctrl+Alt+T` or `Super+Up`. Matching is by a normalised form (`ctrl+alt+t`), so
//! the case, the order of the modifiers and spellings like `Win` or `Meta` do not matter.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// (action id, label, default shortcut). The ids are what a `[keys.binds]` table in the config uses.
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("search", "Search", "Super+S"),
    ("launcher", "Open launcher (Launchpad, start menu)", "Alt+Space"),
    ("menu-search", "Search the program's menus", "Alt+Slash"),
    ("lock", "Lock screen", "Super+L"),
    ("settings", "Open Settings", "Super+Comma"),
    ("wallpaper", "Change wallpaper", "Super+Shift+D"),
    ("files", "Open Files", "Super+E"),
    ("terminal", "Open terminal", "Ctrl+Alt+T"),
    ("close-window", "Close window", "Alt+F4"),
    ("close-window-2", "Close (second)", "Super+Q"),
    ("minimize", "Minimize window", "Super+M"),
    ("maximize", "Maximize window (a snapped half goes to the corner above)", "Super+Up"),
    ("snap-left", "Snap window to the left", "Super+Left"),
    ("snap-right", "Snap window to the right", "Super+Right"),
    ("snap-down", "Restore window (a half goes to the corner below), then minimize", "Super+Down"),
    ("switch-window", "Previous window", "Alt+Tab"),
    ("show-desktop", "Show desktop", "Super+D"),
    ("widgets", "Widgets panel", "Super+W"),
    ("notifications", "Notification centre", "Super+N"),
    ("clipboard", "Clipboard history", "Super+V"),
    ("screenshot", "Screenshot", "Print"),
    ("screenshot-window", "Window screenshot", "Alt+Print"),
    ("screenshot-region", "Region screenshot", "Shift+Print"),
    ("workspace-prev", "Previous desktop", "Ctrl+Alt+Left"),
    ("workspace-next", "Next desktop", "Ctrl+Alt+Right"),
    ("move-prev", "Move to previous desktop", "Ctrl+Alt+Shift+Left"),
    ("move-next", "Move to next desktop", "Ctrl+Alt+Shift+Right"),
];

/// A shortcut that starts a program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CommandKey {
    pub keys: String,
    pub run: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KeyBinds {
    /// action id -> shortcut; missing ones use the default, an empty text turns the action's shortcut off
    pub binds: BTreeMap<String, String>,
    pub commands: Vec<CommandKey>,
    /// how shortcuts are written in menus and tooltips
    pub style: KeyStyle,
}

/// What the Super key is called where shortcuts are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SuperLabel {
    /// ⌘
    #[default]
    Command,
    Windows,
    Super,
    Custom,
}

pub const SUPER_LABELS: [(SuperLabel, &str); 4] = [(SuperLabel::Command, "⌘"), (SuperLabel::Windows, "Win"), (SuperLabel::Super, "Super"), (SuperLabel::Custom, "Custom")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KeyStyle {
    pub super_label: SuperLabel,
    /// the text for `SuperLabel::Custom`
    pub super_custom: String,
    /// ⌃ ⌥ ⇧ for Ctrl, Alt and Shift
    pub symbols: bool,
}

impl KeyStyle {
    /// A shortcut (`Ctrl+Alt+T`, `Super+E`) as this style writes it. The modifiers that are one symbol each run together
    /// (`⌃⌥T`, `⌘E`); anything else is joined with a plus.
    pub fn show(&self, chord: &str) -> String {
        let mut mods: Vec<String> = Vec::new();
        let mut key = String::new();
        let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let label = match part.to_lowercase().as_str() {
                "ctrl" | "control" if !last => Some(if self.symbols { "⌃".to_string() } else { "Ctrl".into() }),
                "alt" | "option" if !last => Some(if self.symbols { "⌥".to_string() } else { "Alt".into() }),
                "shift" if !last => Some(if self.symbols { "⇧".to_string() } else { "Shift".into() }),
                "super" | "win" | "logo" | "meta" | "cmd" | "command" if !last => Some(match self.super_label {
                    SuperLabel::Command => "⌘".to_string(),
                    SuperLabel::Windows => "Win".into(),
                    SuperLabel::Super => "Super".into(),
                    SuperLabel::Custom if self.super_custom.trim().is_empty() => "Super".into(),
                    SuperLabel::Custom => self.super_custom.trim().to_string(),
                }),
                _ => None,
            };
            match label {
                Some(l) => mods.push(l),
                None if self.symbols && part.eq_ignore_ascii_case("space") => key = "␣".into(),
                None => key = part.to_string(),
            }
        }
        if mods.is_empty() {
            return key;
        }
        let joiner = if mods.iter().all(|m| m.chars().count() == 1) { "" } else { "+" };
        let mut out = mods.join(joiner);
        // a one-letter key after symbols is a capital, as on a Mac
        let key = if joiner.is_empty() && key.chars().count() == 1 { key.to_uppercase() } else { key };
        if !key.is_empty() {
            out.push_str(joiner);
            out.push_str(&key);
        }
        out
    }
}

static STYLE: std::sync::RwLock<Option<KeyStyle>> = std::sync::RwLock::new(None);

/// Use this style wherever the shell shows a shortcut.
pub fn set_style(style: &KeyStyle) {
    if let Ok(mut g) = STYLE.write() {
        *g = Some(style.clone());
    }
}

/// A shortcut as the shell shows it now.
pub fn display(chord: &str) -> String {
    match STYLE.read().ok().and_then(|g| g.clone()) {
        Some(s) => s.show(chord),
        None => KeyStyle::default().show(chord),
    }
}

/// `Win + t` -> `super+t`.
pub fn normalize(text: &str) -> String {
    let (mut ctrl, mut alt, mut shift, mut sup) = (false, false, false, false);
    let mut key = String::new();
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            "super" | "win" | "logo" | "meta" | "cmd" => sup = true,
            other => key = other.to_string(),
        }
    }
    if key.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (on, name) in [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift"), (sup, "super")] {
        if on {
            out.push_str(name);
            out.push('+');
        }
    }
    out.push_str(&key);
    out
}

impl KeyBinds {
    /// The shortcut text an action currently has.
    pub fn chord(&self, action: &str) -> String {
        match self.binds.get(action) {
            Some(c) => c.clone(),
            None => ACTIONS.iter().find(|(id, ..)| *id == action).map(|(.., d)| d.to_string()).unwrap_or_default(),
        }
    }

    pub fn set(&mut self, action: &str, chord: String) {
        let default = ACTIONS.iter().find(|(id, ..)| *id == action).map(|(.., d)| *d).unwrap_or("");
        if chord == default {
            self.binds.remove(action);
        } else {
            self.binds.insert(action.to_string(), chord);
        }
    }

    /// Which action (id) a pressed combination, already normalised, triggers.
    pub fn action_for(&self, pressed: &str) -> Option<&'static str> {
        ACTIONS.iter().map(|(id, ..)| *id).find(|id| normalize(&self.chord(id)) == pressed)
    }

    pub fn command_for(&self, pressed: &str) -> Option<&str> {
        self.commands.iter().find(|c| !c.run.trim().is_empty() && normalize(&c.keys) == pressed).map(|c| c.run.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_and_order_do_not_matter() {
        assert_eq!(normalize("Win + T"), "super+t");
        assert_eq!(normalize("shift+CTRL+Print"), "ctrl+shift+print");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("Ctrl+"), "");
    }

    #[test]
    fn defaults_and_changes() {
        let mut k = KeyBinds::default();
        assert_eq!(k.action_for("super+l"), Some("lock"));
        k.set("lock", "Ctrl+Alt+L".into());
        assert_eq!(k.action_for("super+l"), None);
        assert_eq!(k.action_for("ctrl+alt+l"), Some("lock"));
        k.set("lock", "Super+L".into());
        assert!(k.binds.is_empty(), "going back to the default must not leave an override");
        k.set("lock", String::new());
        assert_eq!(k.action_for("super+l"), None);
    }

    #[test]
    fn commands_match_by_chord() {
        let mut k = KeyBinds::default();
        k.commands.push(CommandKey { keys: "Super+B".into(), run: "brave".into() });
        assert_eq!(k.command_for("super+b"), Some("brave"));
        assert_eq!(k.command_for("super+c"), None);
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn super_and_symbols_are_written_as_chosen() {
        let mut s = KeyStyle::default();
        assert_eq!(s.show("Super+E"), "⌘E");
        assert_eq!(s.show("Ctrl+Alt+T"), "Ctrl+Alt+T");
        assert_eq!(s.show("Super+Up"), "⌘Up");
        assert_eq!(s.show("Print"), "Print");
        s.super_label = SuperLabel::Windows;
        assert_eq!(s.show("Super+E"), "Win+E");
        s.super_label = SuperLabel::Custom;
        s.super_custom = "Meta".into();
        assert_eq!(s.show("super+shift+d"), "Meta+Shift+d");
        s.super_label = SuperLabel::Command;
        s.symbols = true;
        assert_eq!(s.show("Ctrl+Alt+T"), "⌃⌥T");
        assert_eq!(s.show("Ctrl+Shift+D"), "⌃⇧D");
        assert_eq!(s.show("Super+Shift+Up"), "⌘⇧Up");
        assert_eq!(s.show("Alt+Tab"), "⌥Tab");
        assert_eq!(s.show("Ctrl+Space"), "⌃␣");
        s.symbols = false;
        assert_eq!(s.show("Ctrl+Space"), "Ctrl+Space");
    }
}
