//! What the shell does when nobody touches the computer.

use serde::{Deserialize, Serialize};

/// Minutes of inactivity before each step; 0 turns a step off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdlePrefs {
    pub lock_min: f32,
    pub screen_off_min: f32,
    pub suspend_min: f32,
}

impl Default for IdlePrefs {
    fn default() -> Self {
        IdlePrefs { lock_min: 10.0, screen_off_min: 15.0, suspend_min: 0.0 }
    }
}

/// The step that is due after `idle_secs`, given which ones already ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Lock,
    ScreenOff,
    Suspend,
}

impl IdlePrefs {
    pub fn due(&self, idle_secs: f32, done: [bool; 3]) -> Option<Step> {
        let due = |min: f32| min > 0.0 && idle_secs >= min * 60.0;
        // the latest step that is due and not yet done wins, so a long sleep does not replay the earlier ones
        if due(self.suspend_min) && !done[2] {
            Some(Step::Suspend)
        } else if due(self.screen_off_min) && !done[1] {
            Some(Step::ScreenOff)
        } else if due(self.lock_min) && !done[0] {
            Some(Step::Lock)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_fire_in_order_and_once() {
        let p = IdlePrefs { lock_min: 1.0, screen_off_min: 2.0, suspend_min: 0.0 };
        assert_eq!(p.due(30.0, [false; 3]), None);
        assert_eq!(p.due(61.0, [false; 3]), Some(Step::Lock));
        assert_eq!(p.due(61.0, [true, false, false]), None);
        assert_eq!(p.due(130.0, [true, false, false]), Some(Step::ScreenOff));
        assert_eq!(p.due(9999.0, [true, true, false]), None);
    }
}
