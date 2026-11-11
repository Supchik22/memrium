//! The privacy centre: what the camera may be used for, and the reactions the desktop draws when it sees a hand gesture.
//!
//! The camera is only ever opened by the gesture service (`memrium_blink --gestures`), and only while `gestures` is on:
//! turning it off stops that program, which closes the camera. The reactions are plain functions of their age, so they can
//! be drawn in any frame and tested without a camera.

use crate::renderer::Panel;
use serde::{Deserialize, Serialize};

/// A hand gesture the camera can recognise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    Like,
    Dislike,
    Peace,
}

impl Reaction {
    /// The word the gesture service prints for it.
    pub fn parse(word: &str) -> Option<Reaction> {
        match word {
            "like" => Some(Reaction::Like),
            "dislike" => Some(Reaction::Dislike),
            "peace" => Some(Reaction::Peace),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Reaction::Like => "like",
            Reaction::Dislike => "dislike",
            Reaction::Peace => "peace",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyPrefs {
    /// the camera watches for hand gestures (off until the user turns it on: it is the camera)
    pub gestures: bool,
    /// which gestures get a reaction
    pub like: bool,
    pub dislike: bool,
    pub peace: bool,
    /// a dot at the top of the screen while the camera is in use
    pub indicator: bool,
    /// search by meaning: pictures, screenshots and documents are indexed (models fetched, an indexer run on mains
    /// power while idle); off until the user turns it on (see `semsearch`)
    pub semantic_search: bool,
}

impl Default for PrivacyPrefs {
    fn default() -> Self {
        PrivacyPrefs { gestures: false, like: true, dislike: true, peace: true, indicator: true, semantic_search: false }
    }
}

impl PrivacyPrefs {
    pub fn allows(&self, r: Reaction) -> bool {
        match r {
            Reaction::Like => self.like,
            Reaction::Dislike => self.dislike,
            Reaction::Peace => self.peace,
        }
    }
}

/// How long a reaction stays on screen (s).
pub const LIFE: f32 = 2.6;

/// A cheap deterministic 0..1 sequence per (burst, particle, channel).
fn rnd(seed: u32, i: u32, k: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B1) ^ i.wrapping_mul(0x85EB_CA6B) ^ k.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFFFF) as f32 / 65535.0
}

const CONFETTI: [[f32; 3]; 6] = [[1.0, 0.36, 0.4], [1.0, 0.78, 0.2], [0.3, 0.8, 0.5], [0.3, 0.6, 1.0], [0.7, 0.45, 1.0], [1.0, 0.55, 0.25]];

/// The panels of a reaction `t` seconds after it began, over a screen of `screen` logical px. `at` is where the hand was
/// in the camera's picture, as 0..1 of it, already turned the way a mirror shows it. Nothing is drawn after `LIFE`.
pub fn panels(reaction: Reaction, seed: u32, at: [f32; 2], t: f32, screen: [f32; 2], out: &mut Vec<Panel>) {
    if !(0.0..LIFE).contains(&t) {
        return;
    }
    let [w, h] = screen;
    let fade = (1.0 - (t - (LIFE - 0.8)).max(0.0) / 0.8).clamp(0.0, 1.0);
    let (icon, tint) = match reaction {
        Reaction::Like => ("thumbs-up", [0.35, 0.65, 1.0]),
        Reaction::Dislike => ("thumbs-down", [1.0, 0.45, 0.42]),
        Reaction::Peace => ("party-popper", [1.0, 0.8, 0.3]),
    };
    let origin = [at[0].clamp(0.0, 1.0) * w, at[1].clamp(0.0, 1.0) * h];

    // a damped spring towards 1: pops past the size and settles
    let pop = 1.0 - (-7.0 * t).exp() * (16.0 * t).cos();
    let big = 190.0 * pop.max(0.0);
    let disc = big * 1.05;
    let (cx, cy) = (w / 2.0, h * 0.42);
    if disc > 1.0 {
        out.push(Panel::solid(cx, cy, disc, disc, disc / 2.0, [0.0; 3], [0.0; 3]).flat().alpha(0.30 * fade * pop.clamp(0.0, 1.0)));
        out.push(Panel::glyph(cx, cy, big * 0.62, icon, tint, fade));
    }

    match reaction {
        Reaction::Peace => {
            // confetti falls over the whole screen
            for i in 0..46 {
                let x = rnd(seed, i, 0) * w + (t * (2.0 + rnd(seed, i, 1) * 3.0) + i as f32).sin() * 22.0;
                let y = -30.0 + (t - rnd(seed, i, 2) * 0.5).max(0.0) * (h * 0.22 + rnd(seed, i, 3) * h * 0.30);
                let c = CONFETTI[(rnd(seed, i, 4) * 6.0) as usize % 6];
                let (cw, ch) = (7.0 + rnd(seed, i, 5) * 7.0, 11.0 + rnd(seed, i, 6) * 8.0);
                // flutter: the piece seems to turn by changing its width
                let flip = (t * 6.0 + i as f32 * 1.7).cos().abs().max(0.25);
                out.push(Panel::solid(x, y + ch / 2.0, cw * flip, ch, 1.5, c, c).flat().alpha(fade));
            }
        }
        r => {
            // small copies of the icon (and hearts) drift away from the hand
            let up = r == Reaction::Like;
            for i in 0..16 {
                let delay = rnd(seed, i, 0) * 0.5;
                let a = (t - delay).max(0.0);
                if a <= 0.0 {
                    continue;
                }
                let speed = 140.0 + rnd(seed, i, 1) * 190.0;
                let drift = (rnd(seed, i, 2) - 0.5) * 200.0;
                let x = origin[0] + drift * a + (a * 4.0 + i as f32).sin() * 14.0;
                let y = origin[1] + if up { -speed * a } else { speed * a * 0.8 };
                let s = 24.0 + rnd(seed, i, 3) * 26.0;
                let life = (1.0 - a / (LIFE - 0.3)).clamp(0.0, 1.0);
                let (name, c) = if i % 3 == 0 { ("heart", [1.0, 0.4, 0.5]) } else { (icon, tint) };
                out.push(Panel::glyph(x, y, s, name, c, life * fade));
            }
        }
    }
}

/// The camera dot: a small pill with a camera, at the top centre of the screen. `k` is how far it has come in (0..1).
pub fn indicator(screen: [f32; 2], k: f32, out: &mut Vec<Panel>) {
    if k <= 0.01 {
        return;
    }
    let (w, h) = (46.0, 22.0);
    let (cx, cy) = (screen[0] / 2.0, 6.0 + h / 2.0 - (1.0 - k) * 24.0);
    out.push(Panel::solid(cx, cy, w, h, h / 2.0, [0.10, 0.10, 0.11], [0.04, 0.04, 0.05]).flat().alpha(0.86 * k));
    out.push(Panel::glyph(cx - 9.0, cy, 13.0, "camera", [1.0; 3], 0.95 * k));
    out.push(Panel::solid(cx + 10.0, cy, 8.0, 8.0, 4.0, [0.20, 0.85, 0.40], [0.16, 0.72, 0.34]).flat().alpha(k));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(r: Reaction, t: f32) -> usize {
        let mut v = Vec::new();
        panels(r, 7, [0.5, 0.5], t, [1920.0, 1080.0], &mut v);
        v.len()
    }

    #[test]
    fn nothing_before_the_start_or_after_the_end() {
        assert_eq!(count(Reaction::Like, -0.1), 0);
        assert_eq!(count(Reaction::Peace, LIFE), 0);
        assert_eq!(count(Reaction::Dislike, LIFE + 5.0), 0);
    }

    #[test]
    fn peace_has_the_confetti_and_a_like_the_icons() {
        assert!(count(Reaction::Peace, 1.5) > 40);
        let n = count(Reaction::Like, 1.5);
        assert!(n > 5 && n < 25, "{n}");
    }

    #[test]
    fn the_words_round_trip() {
        for r in [Reaction::Like, Reaction::Dislike, Reaction::Peace] {
            assert_eq!(Reaction::parse(r.word()), Some(r));
        }
        assert_eq!(Reaction::parse("wave"), None);
    }

    #[test]
    fn the_camera_is_off_by_default_and_a_gesture_can_be_muted() {
        let mut p = PrivacyPrefs::default();
        assert!(!p.gestures);
        assert!(p.allows(Reaction::Like));
        p.like = false;
        assert!(!p.allows(Reaction::Like));
        assert!(p.allows(Reaction::Peace));
    }
}
