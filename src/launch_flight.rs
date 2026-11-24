//! The launch animation: a copy of the icon a program was started from shoots out of it, flies over in an arc, stretched
//! along its way, and drops where the launch icon beside the pointer stands (which takes over with a squash and hops on).

use crate::renderer::Panel;

/// Seconds the copy gathers itself before it shoots out, at speed 1.
const WIND: f32 = 0.09;
/// Seconds in the air over a short way; a long way takes up to `FAR` more.
const AIR: f32 = 0.62;
const FAR: f32 = 0.22;
/// Seconds the copy takes to burst when the pointer does not take it over (the program is already open).
const POP: f32 = 0.22;
/// How much it is flattened before the shot.
const SQUEEZE: f32 = 0.22;
/// How far it stretches at full speed, and the speed (logical px/s) that is most of the way to it.
const STRETCH: f32 = 0.75;
const FAST: f32 = 1300.0;
/// How much it slows over the top of the arc (0: not at all).
const HANG: f32 = 0.28;
/// The copies trailing behind: opacity, the nearest first, and how many seconds apart they follow.
const TRAIL: [f32; 4] = [0.34, 0.22, 0.13, 0.06];
const TRAIL_GAP: f32 = 0.022;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// The way along the arc: quick out of the icon, slower over the top, quick again as it falls.
fn pace(u: f32) -> f32 {
    lerp(u, 4.0 * (u - 0.5).powi(3) + 0.5, HANG)
}

fn pace_rate(u: f32) -> f32 {
    lerp(1.0, 12.0 * (u - 0.5).powi(2), HANG)
}

/// The icon a program was started from, as it was drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub at: [f32; 2],
    pub size: f32,
    pub cell: u32,
    pub pad: f32,
    pub mask: f32,
    /// its opacity (the Dock can dim what is not running): the copy lights up as it gathers itself
    pub alpha: f32,
}

impl Source {
    /// The icon `panel` draws, when it is one that can be seen (not a soft bloom, faded out or cut away).
    pub fn of(panel: &Panel) -> Option<Source> {
        let [x, y, half, _] = panel.rect;
        let [cx, cy, cw, ch] = panel.clip;
        let cut = cw > 0.0 && ((x - cx).abs() > cw || (y - cy).abs() > ch);
        let seen = panel.params[1] == 7.0 && panel.fx[3] <= 0.0 && panel.tint[3] > 0.3 && half >= 2.0 && !cut;
        seen.then_some(Source { at: [x, y], size: half * 2.0, cell: panel.params[2] as u32, pad: panel.fx[0], mask: panel.params[3], alpha: panel.tint[3].min(1.0) })
    }
}

/// Which of the icons on screen (in paint order) the program was started from. `cells`: the atlas cells that show this
/// program. `pointer`: where it was clicked (None: started from the keyboard). A click counts for the icon under it, else
/// for the nearest icon of its row or of the label under it; with none of those, the icon painted last (on top) is it.
pub fn pick(seen: &[Source], cells: &[u32], pointer: Option<[f32; 2]>) -> Option<Source> {
    let mine = || seen.iter().rev().filter(|s| cells.contains(&s.cell));
    if let Some(m) = pointer {
        let off = |s: &Source| [(m[0] - s.at[0]).abs(), (m[1] - s.at[1]).abs()];
        let near = |s: &&Source| {
            let [dx, dy] = off(s);
            let in_row = dy <= s.size / 2.0 + 12.0 && dx <= 420.0;
            let on_label = dx <= s.size / 2.0 + 40.0 && dy <= s.size / 2.0 + 56.0;
            in_row || on_label
        };
        let cost = |s: &&Source| {
            let [dx, dy] = off(s);
            if dx <= s.size / 2.0 && dy <= s.size / 2.0 { 0.0 } else { 1.0 + dx + dy * 4.0 }
        };
        // (of equally near ones `min_by` keeps the first: the topmost, the list being walked from the top)
        if let Some(s) = mine().filter(near).min_by(|a, b| cost(a).total_cmp(&cost(b))) {
            return Some(*s);
        }
    }
    mine().next().copied()
}

/// Where a copy is and how it is deformed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub at: [f32; 2],
    pub size: f32,
    /// the direction it is stretched along, and the factors along and across it
    pub dir: [f32; 2],
    pub along: f32,
    pub across: f32,
    pub pad: f32,
    pub alpha: f32,
    /// logical px a second
    pub speed: f32,
}

#[derive(Clone, Debug)]
pub struct Flight {
    src: Source,
    /// where it lands, from the pointer, and the icon's side there
    land: [f32; 2],
    side: f32,
    /// the arc's handles: from where it starts and from where it lands (so the arc follows a moving pointer smoothly)
    out: [f32; 2],
    fall: [f32; 2],
    air: f32,
    t: f32,
    started: bool,
    burst: Option<f32>,
}

impl Flight {
    /// From `src` to `land` px off the pointer, where an icon `side` px big takes over. `screen`: the monitor it starts on
    /// (x, y, w, h): it shoots out into the screen and loops over on the side with more room.
    pub fn new(src: Source, pointer: [f32; 2], land: [f32; 2], side: f32, screen: [f32; 4]) -> Flight {
        let [sx, sy, sw, sh] = screen;
        let (s, e) = (src.at, [pointer[0] + land[0], pointer[1] + land[1]]);
        let d = (e[0] - s[0]).hypot(e[1] - s[1]);
        let reach = (165.0 + 0.16 * d + 0.3 * src.size).min(280.0);
        // away from an edge it sits at (the Dock, a bar), else up
        let room = (reach * 0.8).min(sw.min(sh) * 0.25);
        let away = if s[1] - sy > sh - room {
            [0.0, -1.0]
        } else if s[0] - sx < room {
            [1.0, 0.0]
        } else if s[0] - sx > sw - room {
            [-1.0, 0.0]
        } else if s[1] - sy < room {
            [0.0, 1.0]
        } else {
            [0.0, -1.0]
        };
        let toward = |v: f32, middle: f32| if v <= middle { 1.0 } else { -1.0 };
        let side_way = if away[0] == 0.0 { [toward(s[0], sx + sw / 2.0), 0.0] } else { [0.0, toward(s[1], sy + sh / 2.0)] };
        // it falls onto the pointer from above, where there is room above
        let above = if e[1] - sy > room { -1.0 } else { 1.0 };
        let swing = toward(e[0], sx + sw / 2.0);
        Flight {
            src,
            land,
            side,
            out: std::array::from_fn(|i| (away[i] * 1.25 - side_way[i] * 0.4) * reach),
            fall: [swing * 0.6 * reach, above * 1.2 * reach],
            air: AIR + (d / 4000.0).min(FAR),
            t: 0.0,
            started: false,
            burst: None,
        }
    }

    /// `dt` seconds on (already at the animation's speed). True on the step it lands.
    pub fn step(&mut self, dt: f32) -> bool {
        // the step it appears in spans the idle time before the click, and a stall must not jump over the flight
        if !std::mem::replace(&mut self.started, true) {
            return false;
        }
        let dt = dt.min(0.05);
        if let Some(b) = &mut self.burst {
            *b += dt;
            return false;
        }
        let before = self.t;
        self.t += dt;
        before < WIND + self.air && self.t >= WIND + self.air
    }

    /// What it lands on changed its size (a Dock's icon growing in, magnified under the pointer).
    pub fn set_side(&mut self, side: f32) {
        self.side = side;
    }

    /// It landed and nothing took it over: it bursts softly instead of vanishing.
    pub fn burst(&mut self) {
        self.burst = Some(0.0);
    }

    pub fn over(&self) -> bool {
        match self.burst {
            Some(b) => b >= POP,
            None => self.t >= WIND + self.air,
        }
    }

    /// The copy `t` seconds into the animation, with the pointer at `pointer`.
    pub fn pose(&self, t: f32, pointer: [f32; 2]) -> Pose {
        let src = self.src;
        let u = ((t - WIND) / self.air).clamp(0.0, 1.0);
        let (p0, p3) = (src.at, [pointer[0] + self.land[0], pointer[1] + self.land[1]]);
        let p1 = [p0[0] + self.out[0], p0[1] + self.out[1]];
        let p2 = [p3[0] + self.fall[0], p3[1] + self.fall[1]];
        let (e, left) = (pace(u), 1.0 - pace(u));
        let mut at: [f32; 2] = std::array::from_fn(|i| left * left * left * p0[i] + 3.0 * left * left * e * p1[i] + 3.0 * left * e * e * p2[i] + e * e * e * p3[i]);
        let vel: [f32; 2] = std::array::from_fn(|i| 3.0 * (left * left * (p1[i] - p0[i]) + 2.0 * left * e * (p2[i] - p1[i]) + e * e * (p3[i] - p2[i])));
        let len = vel[0].hypot(vel[1]);
        let dir = if len > 0.01 { [vel[0] / len, vel[1] / len] } else { [0.0, -1.0] };
        let speed = if t > WIND { len * pace_rate(u) / self.air } else { 0.0 };

        // gathers itself against the shot, lets go into the stretch, and is itself again as it touches down
        let wound = smooth(t / WIND);
        let go = smooth(u / 0.12);
        let settle = 1.0 - smooth((u - 0.8) / 0.2);
        let stretch = 1.0 + STRETCH * (speed / FAST).tanh() * settle;
        let along = lerp(1.0 - SQUEEZE * wound, stretch, go);
        let pull = src.size * 0.1 * wound * (1.0 - go);
        for i in 0..2 {
            at[i] -= dir[i] * pull;
        }
        // a small icon (a list's) swells in the air so that the flight can be seen
        // (it keeps its size over the top and shrinks as it falls)
        let shrunk = smooth((u - 0.35) / 0.65);
        let swell = ((44.0 - src.size).max(0.0) * 0.8 + src.size.min(64.0) * 0.12) * (std::f32::consts::PI * u).sin().max(0.0);
        Pose { at, size: lerp(src.size, self.side, shrunk) + swell, dir, along, across: 1.0 / along.sqrt(), pad: lerp(src.pad, 0.0, shrunk), alpha: lerp(src.alpha, 1.0, wound), speed }
    }

    /// The copy and its trail, the faintest first.
    pub fn panels(&self, pointer: [f32; 2]) -> Vec<Panel> {
        let panel = |p: &Pose, size: f32, opacity: f32| Panel::flying_icon(p.at, size, self.src.cell, opacity, p.dir, [p.along, p.across], p.pad, self.src.mask);
        let head = self.pose(self.t, pointer);
        if let Some(b) = self.burst {
            let k = (b / POP).clamp(0.0, 1.0);
            return vec![panel(&head, head.size * (1.0 + 0.6 * (1.0 - (1.0 - k).powi(3))), (1.0 - k) * (1.0 - k))];
        }
        let mut out = Vec::with_capacity(TRAIL.len() + 1);
        // (the trail closes up before the landing, or it would be cut off when the cursor takes over)
        let fast = (head.speed / FAST).tanh() * (1.0 - smooth((self.t - WIND - self.air * 0.8) / (self.air * 0.2)));
        for (i, a) in TRAIL.iter().enumerate().rev() {
            let t = self.t - TRAIL_GAP * (i + 1) as f32;
            if t > WIND && a * fast > 0.004 {
                let p = self.pose(t, pointer);
                out.push(panel(&p, p.size * (1.0 - 0.06 * (i + 1) as f32), a * fast));
            }
        }
        out.push(panel(&head, head.size, head.alpha));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: [f32; 4] = [0.0, 0.0, 1600.0, 900.0];

    fn src(at: [f32; 2], cell: u32) -> Source {
        Source { at, size: 64.0, cell, pad: 0.1, mask: 0.0, alpha: 0.4 }
    }

    fn cases() -> Vec<([f32; 2], [f32; 2])> {
        vec![([800.0, 860.0], [805.0, 862.0]), ([30.0, 450.0], [32.0, 455.0]), ([1570.0, 450.0], [1568.0, 452.0]), ([400.0, 18.0], [402.0, 20.0]), ([700.0, 300.0], [700.0, 300.0]), ([800.0, 200.0], [200.0, 800.0])]
    }

    #[test]
    fn starts_as_the_icon_and_ends_as_the_cursors_icon() {
        for (from, mouse) in cases() {
            let f = Flight::new(src(from, 1), mouse, [24.0, 8.0], 22.0, SCREEN);
            let a = f.pose(0.0, mouse);
            assert_eq!((a.at, a.size, a.along, a.across, a.pad, a.alpha), (from, 64.0, 1.0, 1.0, 0.1, 0.4));
            let moved = [mouse[0] + 90.0, mouse[1] - 40.0];
            let z = f.pose(WIND + f.air, moved);
            assert!((z.at[0] - moved[0] - 24.0).abs() < 1e-3 && (z.at[1] - moved[1] - 8.0).abs() < 1e-3, "{z:?}");
            assert!((z.size - 22.0).abs() < 1e-3 && (z.along - 1.0).abs() < 1e-3 && z.pad.abs() < 1e-4, "{z:?}");
        }
    }

    #[test]
    fn moves_without_jumps_and_stretches() {
        for (from, mouse) in cases() {
            let f = Flight::new(src(from, 1), mouse, [24.0, 8.0], 22.0, SCREEN);
            let n = ((WIND + f.air) * 240.0) as usize;
            let poses: Vec<Pose> = (0..=n).map(|i| f.pose(i as f32 / 240.0, mouse)).collect();
            for w in poses.windows(2) {
                let d = (w[0].at[0] - w[1].at[0]).hypot(w[0].at[1] - w[1].at[1]);
                assert!(d < 28.0 && (w[0].along - w[1].along).abs() < 0.14 && (w[0].size - w[1].size).abs() < 3.0, "{from:?}: {w:?}");
            }
            assert!(poses.iter().any(|p| p.along > 1.25), "{from:?}: never stretched");
            // even from right under the pointer it really flies out
            assert!(poses.iter().any(|p| (p.at[0] - from[0]).hypot(p.at[1] - from[1]) > 70.0), "{from:?}: stayed put");
        }
    }

    #[test]
    fn lands_once_after_an_idle_first_step() {
        let mut f = Flight::new(src([800.0, 860.0], 1), [800.0, 860.0], [24.0, 8.0], 22.0, SCREEN);
        assert!(!f.step(3.0) && f.t == 0.0);
        assert_eq!((0..200).filter(|_| f.step(1.0 / 60.0)).count(), 1);
        assert!(f.over());
        f.burst();
        assert!(!f.over());
        f.step(0.05);
        assert_eq!(f.panels([800.0, 860.0]).len(), 1);
    }

    #[test]
    fn picks_the_icon_that_was_used() {
        let (dock, row) = (src([800.0, 860.0], 7), src([120.0, 400.0], 7));
        let seen = [dock, row, src([300.0, 400.0], 9)];
        assert_eq!(pick(&seen, &[7], Some([810.0, 850.0])), Some(dock));
        assert_eq!(pick(&seen, &[7], Some([330.0, 405.0])), Some(row), "a click on the row's text");
        assert_eq!(pick(&seen, &[7], None), Some(row), "the keyboard: the one on top");
        assert_eq!(pick(&seen, &[3], Some([120.0, 400.0])), None);
    }
}
