//! The lock screen. While it is up nothing else is drawn, no window receives keys or pointer events, and the
//! password is checked against the system (PAM) on a background thread. If the compositor dies, so does the
//! session, so the screen can never be left unlocked by a crash.

use crate::state::State;
use memrium_de::renderer::{Align, Label, Panel};
use chebrium::spring::Spring;
use memrium_de::{sysinfo, widget};
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::utils::SERIAL_COUNTER;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

const MAX_PASSWORD: usize = 256;

pub struct Lock {
    password: String,
    /// a password is being checked
    checking: Option<Receiver<Result<bool, String>>>,
    message: Option<(String, Instant)>,
    /// sideways wobble of the field after a wrong password
    shake: Spring,
    /// fade-in
    k: Spring,
}

impl State {
    /// Lock the screen now.
    pub fn lock_screen(&mut self) {
        if self.lock.is_some() {
            return;
        }
        self.menu.close();
        self.launcher.close();
        self.scene.set_cc_open(false);
        self.grab = None;
        self.shot_sel = None;
        let mut k = Spring::new(0.0, 160.0, 0.9);
        k.target = 1.0;
        self.lock = Some(Lock { password: String::new(), checking: None, message: None, shake: Spring::new(0.0, 700.0, 0.35), k });
        // nothing may keep receiving input behind the lock
        let (kb, ptr) = (self.keyboard.clone(), self.pointer.clone());
        kb.set_focus(self, None, SERIAL_COUNTER.next_serial());
        let time = self.time_ms();
        ptr.motion(self, None, &smithay::input::pointer::MotionEvent { location: (self.mouse[0] as f64, self.mouse[1] as f64).into(), serial: SERIAL_COUNTER.next_serial(), time });
        ptr.frame(self);
        self.invalidate();
    }

    fn unlock(&mut self) {
        self.ui_sound(memrium_de::sound::Sound::Unlock);
        self.lock = None;
        self.apply_focus();
        self.invalidate();
    }

    /// A key while locked: typing the password. Always consumed.
    pub fn lock_key(&mut self, mods: &ModifiersState, sym: Keysym) {
        // the way out when the password cannot be checked at all (no PAM): end the session
        if mods.ctrl && mods.alt && sym == Keysym::BackSpace {
            self.quit = true;
            return;
        }
        let Some(lock) = self.lock.as_mut() else { return };
        if lock.checking.is_some() {
            return;
        }
        match sym {
            Keysym::Return | Keysym::KP_Enter => {
                if lock.password.is_empty() {
                    return;
                }
                let password = std::mem::take(&mut lock.password);
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(memrium_de::pam::authenticate(&sysinfo::user_name(), &password));
                });
                lock.checking = Some(rx);
                lock.message = None;
            }
            Keysym::BackSpace => {
                lock.password.pop();
            }
            Keysym::Escape => lock.password.clear(),
            _ => {
                if let (Some(c), false) = (sym.key_char(), mods.ctrl || mods.alt || mods.logo) {
                    if !c.is_control() && lock.password.chars().count() < MAX_PASSWORD {
                        lock.password.push(c);
                    }
                }
            }
        }
    }

    /// Animations and the answer of the password check.
    pub fn update_lock(&mut self, dt: f32) {
        let Some(lock) = self.lock.as_mut() else { return };
        lock.k.update(dt);
        lock.shake.update(dt);
        if lock.message.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs() >= 4) {
            lock.message = None;
        }
        let answer = match lock.checking.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(a)) => Some(a),
            Some(Err(TryRecvError::Disconnected)) => Some(Err("the password check stopped".into())),
            _ => None,
        };
        let Some(answer) = answer else { return };
        lock.checking = None;
        match answer {
            Ok(true) => self.unlock(),
            Ok(false) => {
                lock.message = Some(("Incorrect password".into(), Instant::now()));
                if self.theme.animations.sounds {
                    memrium_de::sound::play(memrium_de::sound::Sound::Error, self.theme.animations.sound_volume);
                }
                lock.shake.kick(900.0);
            }
            Err(e) => {
                eprintln!("lock: {e}");
                lock.message = Some(("The password cannot be checked here. Ctrl+Alt+Backspace logs out.".into(), Instant::now()));
            }
        }
    }

    /// The whole frame while locked.
    pub fn lock_panels(&mut self) -> (Vec<Panel>, Vec<Label>) {
        let (sw, sh) = (self.size[0], self.size[1]);
        let Some(lock) = self.lock.as_ref() else { return (Vec::new(), Vec::new()) };
        let k = lock.k.x.clamp(0.0, 1.0);
        let mut p = Vec::new();
        let mut l = Vec::new();
        // the wallpaper, blurred and darkened
        let mut veil = Panel::glass(sw / 2.0, sh / 2.0, sw, sh, 0.0);
        veil.params[3] = 0.0;
        veil.fx = [48.0 * k, 0.0, 0.0, 0.0];
        veil.extra = [0.0; 4];
        veil.tint = [0.03, 0.05, 0.11, 0.50 * k];
        p.push(veil);

        let white = [1.0; 3];
        // the time is made of Jelly Glass, as on a phone's lock screen
        let digits = Label::new(widget::format_time("%H:%M"), sw / 2.0, sh * 0.20 + 40.0, 170.0, true, white, 1.0, Align::Center);
        let gs = self.theme.glass.lettering_liquid();
        let mut glass = self.renderer.glass_labels(&[digits], &gs, [0.94, 0.97, 1.0, 0.20 * k], true);
        for g in &mut glass {
            g.extra[3] = g.extra[3].max(1.0);
        }
        p.extend(glass);
        l.push(Label::new(widget::format_time("%A, %e %B"), sw / 2.0, sh * 0.20 + 90.0, 22.0, false, white, 0.80 * k, Align::Center));

        let cy = sh * 0.52;
        p.push(Panel::solid(sw / 2.0, cy - 8.0, 92.0, 92.0, 46.0, self.theme.accent, [self.theme.accent[0] * 0.6, self.theme.accent[1] * 0.6, self.theme.accent[2] * 0.7]).flat().alpha(k).shadow(0.35 * k, 24.0, 8.0));
        p.push(Panel::glyph(sw / 2.0, cy - 8.0, 46.0, "user", white, 0.96 * k));
        l.push(Label::new(sysinfo::user_name(), sw / 2.0, cy + 62.0, 20.0, true, white, 0.96 * k, Align::Center));

        // the password field
        let fx = sw / 2.0 + lock.shake.x;
        let fy = cy + 108.0;
        let (fw, fh) = (300.0, 46.0);
        let mut field = self.theme.glass.panel(fx, fy, fw, fh, fh / 2.0).tinted([1.0, 1.0, 1.0, 0.16 * k]);
        field.fx[0] = field.fx[0].max(14.0);
        field.extra[0] *= k;
        p.push(field);
        let n = lock.password.chars().count();
        if n == 0 && lock.checking.is_none() {
            l.push(Label::new("Enter password", fx, fy + 5.0, 14.0, false, white, 0.55 * k, Align::Center));
        } else {
            // one dot per character; a long password scrolls, the last dots stay in view
            let shown = n.min(14);
            let start = fx - (shown as f32 - 1.0) * 10.0;
            for i in 0..shown {
                let pulse = if lock.checking.is_some() { 0.5 + 0.5 * ((self.start.elapsed().as_secs_f32() * 6.0) - i as f32 * 0.5).sin() } else { 1.0 };
                p.push(Panel::solid(start + i as f32 * 20.0, fy, 10.0, 10.0, 5.0, white, white).flat().alpha(0.95 * k * pulse.clamp(0.25, 1.0)));
            }
        }
        // what happened
        let note = match (&lock.message, lock.checking.is_some()) {
            (Some((m, _)), _) => Some(m.clone()),
            (None, true) => Some("Checking…".to_string()),
            _ => None,
        };
        if let Some(note) = note {
            l.push(Label::new(note, sw / 2.0, fy + 52.0, 14.0, false, [1.0, 0.82, 0.82], 0.90 * k, Align::Center));
        }
        l.push(Label::new("Press Enter to unlock", sw / 2.0, sh - 40.0, 13.0, false, white, 0.45 * k, Align::Center));
        (p, l)
    }
}
