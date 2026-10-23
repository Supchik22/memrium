//! Rules per application: how one program looks and is started, apart from the rest.
//!
//! `~/.config/memrium/app-rules.toml` has a table per program, named by (a part of) its `app_id`, window class or
//! desktop id, compared without regard to case (a name with dots goes in quotes: `["org.example.App"]`). An exact name
//! beats a part, and of the parts the longest wins:
//!
//! ```toml
//! [prism]                # org.prismlauncher.PrismLauncher
//! opacity = 0.88         # how much of the program shows: below 1 the glass behind it shows through
//! frost = true           # blur what is behind it (a translucent program gets this anyway)
//! dark = true            # start it in a dark look
//! accent = [0.2, 0.8, 0.5]
//! font = "Inter"
//! ```
//!
//! The compositor applies `opacity` and `frost` to the window's picture; `dark`, `accent` and `font` go to the program
//! when the shell starts it (see [`Rule::launch`]): the Qt style reads them from the environment, a Chromium or Electron
//! program gets the flag for a dark page, a GTK program a dark theme.

use crate::theme::config_dir;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rule {
    /// the name the rule is filed under, lower case
    pub name: String,
    pub opacity: Option<f32>,
    pub frost: Option<bool>,
    pub dark: Option<bool>,
    pub accent: Option<[f32; 3]>,
    pub font: Option<String>,
}

/// What to add to the way a program is started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launch {
    pub env: Vec<(String, String)>,
    /// flags for the program itself (after its own command)
    pub args: Vec<String>,
}

impl Rule {
    /// The environment and flags this rule asks for when a program started by `exec` runs.
    pub fn launch(&self, exec: &str) -> Launch {
        let mut l = Launch::default();
        let mut env = |k: &str, v: String| l.env.push((k.to_string(), v));
        if let Some(d) = self.dark {
            env("CHEBRIUM_DARK", (d as u8).to_string());
            if d {
                env("GTK_THEME", "Adwaita:dark".into());
            }
        }
        if let Some(a) = self.accent {
            env("CHEBRIUM_ACCENT", format!("{:.4},{:.4},{:.4}", a[0], a[1], a[2]));
        }
        if let Some(f) = &self.font {
            env("CHEBRIUM_FONT", f.clone());
        }
        if self.dark == Some(true) && is_chromium(exec) {
            l.args.push("--force-dark-mode".into());
            l.args.push("--enable-features=WebContentsForceDark".into());
        }
        l
    }
}

/// Chromium, Chrome, Electron programs and the like: they take the same flags.
pub fn is_chromium(exec: &str) -> bool {
    let e = exec.to_lowercase();
    ["chromium", "chrome", "brave", "vivaldi", "msedge", "microsoft-edge", "opera", "electron", "discord", "slack", "signal-desktop", "obsidian", "code", "vscodium", "spotify", "teams", "element"]
        .iter()
        .any(|n| e.split(|c: char| !c.is_alphanumeric() && c != '-').any(|w| w == *n))
}

#[derive(Clone, Debug, Default)]
pub struct Rules {
    rules: Vec<Rule>,
    stamp: Option<SystemTime>,
}

fn floats3(v: &toml::Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() < 3 {
        return None;
    }
    let f = |x: &toml::Value| x.as_float().or_else(|| x.as_integer().map(|n| n as f64)).map(|v| v as f32);
    Some([f(&a[0])?, f(&a[1])?, f(&a[2])?])
}

impl Rules {
    pub fn path() -> PathBuf {
        config_dir().join("app-rules.toml")
    }

    pub fn load() -> Rules {
        let stamp = std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok();
        let mut r = std::fs::read_to_string(Self::path()).map(|t| Self::parse(&t)).unwrap_or_default();
        r.stamp = stamp;
        r
    }

    /// Read the file again when it changed. True when the rules are new.
    pub fn reload_if_changed(&mut self) -> bool {
        let stamp = std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok();
        if stamp == self.stamp {
            return false;
        }
        *self = Self::load();
        true
    }

    /// A broken file or entry is left out; the rest still holds.
    pub fn parse(text: &str) -> Rules {
        let mut rules = Vec::new();
        let Ok(t) = text.parse::<toml::Table>() else { return Rules::default() };
        for (name, v) in &t {
            let Some(tab) = v.as_table() else { continue };
            let num = |k: &str| tab.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64))).map(|v| v as f32);
            rules.push(Rule {
                name: name.to_lowercase(),
                opacity: num("opacity").map(|v| v.clamp(0.2, 1.0)),
                frost: tab.get("frost").and_then(|v| v.as_bool()),
                dark: tab.get("dark").and_then(|v| v.as_bool()),
                accent: tab.get("accent").and_then(floats3),
                font: tab.get("font").and_then(|v| v.as_str()).map(String::from),
            });
        }
        Rules { rules, stamp: None }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn rules_mut(&mut self) -> &mut [Rule] {
        &mut self.rules
    }

    /// A rule with nothing set yet, filed under `name` (the same name twice is one rule).
    pub fn add(&mut self, name: &str) {
        let name = name.trim().to_lowercase();
        if !name.is_empty() && !self.rules.iter().any(|r| r.name == name) {
            self.rules.push(Rule { name, ..Rule::default() });
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.rules.len() {
            self.rules.remove(i);
        }
    }

    /// The file's text for these rules (comments are not kept).
    pub fn to_toml(&self) -> String {
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut out = String::from("# written by the shell (Settings > Programs); see apprules.rs for what a rule can hold\n");
        for r in &self.rules {
            out += &format!("\n[\"{}\"]\n", q(&r.name));
            if let Some(v) = r.opacity {
                out += &format!("opacity = {v:.2}\n");
            }
            if let Some(v) = r.frost {
                out += &format!("frost = {v}\n");
            }
            if let Some(v) = r.dark {
                out += &format!("dark = {v}\n");
            }
            if let Some(a) = r.accent {
                out += &format!("accent = [{:.4}, {:.4}, {:.4}]\n", a[0], a[1], a[2]);
            }
            if let Some(f) = &r.font {
                out += &format!("font = \"{}\"\n", q(f));
            }
        }
        out
    }

    pub fn save(&mut self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("toml.tmp");
        if std::fs::write(&tmp, self.to_toml()).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            self.stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        }
    }

    /// The rule for a program known by any of `names` (its app_id, class, desktop id, command).
    pub fn find(&self, names: &[&str]) -> Option<&Rule> {
        let names: Vec<String> = names.iter().filter(|n| !n.is_empty()).map(|n| n.to_lowercase()).collect();
        // an exact name first, then the longest part that fits
        self.rules
            .iter()
            .find(|r| names.iter().any(|n| *n == r.name))
            .or_else(|| self.rules.iter().filter(|r| !r.name.is_empty() && names.iter().any(|n| n.contains(&r.name))).max_by_key(|r| r.name.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "[prism]\nopacity = 0.88\nfrost = true\ndark = true\naccent = [0.2, 0.8, 0.5]\nfont = \"Inter\"\n[firefox]\nopacity = 0.95\n[\"org.prismlauncher.prismlauncher\"]\nopacity = 0.5\n";

    #[test]
    fn a_rule_is_found_by_part_of_a_name() {
        let r = Rules::parse(FILE);
        assert_eq!(r.find(&["org.freedesktop.firefox"]).and_then(|r| r.opacity), Some(0.95));
        assert_eq!(r.find(&["org.prismlauncher.PrismLauncher"]).and_then(|r| r.opacity), Some(0.5), "the exact name beats a part");
        assert_eq!(r.find(&["org.prismlauncher.Other"]).and_then(|r| r.opacity), Some(0.88));
        assert!(r.find(&["calc"]).is_none());
        assert!(r.find(&[""]).is_none());
    }

    #[test]
    fn rules_survive_a_round_trip_through_the_file() {
        let r = Rules::parse(FILE);
        let back = Rules::parse(&r.to_toml());
        assert_eq!(r.rules(), back.rules());
        let mut r = Rules::default();
        r.add("Firefox");
        r.add("firefox");
        assert_eq!(r.rules().len(), 1, "the same name is one rule");
        r.rules_mut()[0].font = Some("My \"Font\"".into());
        assert_eq!(Rules::parse(&r.to_toml()).rules()[0].font.as_deref(), Some("My \"Font\""));
        r.remove(0);
        assert!(r.is_empty());
    }

    #[test]
    fn values_are_read_and_clamped() {
        let r = Rules::parse("[a]\nopacity = 7\n[b]\nopacity = 0.0\n[c]\naccent = [1.0]\n[d]\nfont = 3\n");
        assert_eq!(r.find(&["a"]).unwrap().opacity, Some(1.0));
        assert_eq!(r.find(&["b"]).unwrap().opacity, Some(0.2));
        assert_eq!(r.find(&["c"]).unwrap().accent, None);
        assert_eq!(r.find(&["d"]).unwrap().font, None);
        assert!(Rules::parse("not { toml").is_empty());
    }

    #[test]
    fn launch_gives_each_engine_what_it_takes() {
        let r = Rules::parse(FILE);
        let l = r.find(&["prism"]).unwrap().launch("prismlauncher");
        assert!(l.env.contains(&("CHEBRIUM_DARK".into(), "1".into())));
        assert!(l.env.contains(&("CHEBRIUM_FONT".into(), "Inter".into())));
        assert!(l.args.is_empty());
        let c = r.find(&["prism"]).unwrap().launch("flatpak run com.google.Chrome");
        assert!(c.args.contains(&"--force-dark-mode".to_string()));
        assert!(!is_chromium("firefox") && !is_chromium("prismlauncher") && is_chromium("/usr/bin/chromium %U"));
    }
}
