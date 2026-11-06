//! Hand gestures for the whole desktop. While Settings -> Privacy has them on, the desktop runs the gesture service
//! (`memrium_blink --gestures`, see its `service.rs`), and a thumbs up, thumbs down or peace sign seen by the camera gets a
//! reaction drawn over everything. Turning them off stops the service, which closes the camera. A small camera mark shows
//! at the top of the screen for as long as the camera is in use.

use crate::state::State;
use memrium_de::privacy::{self, Reaction};
use memrium_de::renderer::Panel;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

/// What the service says (one line each, see its documentation).
enum Event {
    Status(String),
    Error(String),
    CameraOn,
    Gesture(Reaction, [f32; 2]),
    Ended,
}

fn parse(line: &str) -> Option<Event> {
    let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
    match head {
        "status" => Some(Event::Status(rest.to_string())),
        "error" => Some(Event::Error(rest.to_string())),
        "camera" if rest == "on" => Some(Event::CameraOn),
        "gesture" => {
            let mut it = rest.split(' ');
            let r = Reaction::parse(it.next()?)?;
            let (x, y) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?);
            Some(Event::Gesture(r, [x, y]))
        }
        _ => None,
    }
}

/// The running service.
struct Service {
    child: Child,
    /// held so the pipe stays open: when this process goes, it closes and the service ends too
    _stdin: Option<ChildStdin>,
    events: Receiver<Event>,
}

impl Drop for Service {
    fn drop(&mut self) {
        // the camera closes with the process
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Burst {
    reaction: Reaction,
    start: Instant,
    /// where the hand was on the screen, 0..1
    at: [f32; 2],
    seed: u32,
}

#[derive(Default)]
pub struct Gestures {
    service: Option<Service>,
    bursts: Vec<Burst>,
    count: u32,
    /// what the service last said about itself
    status: String,
    camera_on: bool,
    /// it stopped by itself (permission refused, no camera): not started again until gestures are switched off and on
    gave_up: bool,
    /// how far the camera mark is in (0..1)
    dot: f32,
}

impl Gestures {
    fn start(&mut self) {
        let mut cmd = Command::new(memrium_de::fsops::own_program("memrium_blink"));
        cmd.arg("--gestures").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("could not start the gesture service ({e})");
                self.gave_up = true;
                return;
            }
        };
        let stdin = child.stdin.take();
        let Some(out) = child.stdout.take() else { return };
        let (tx, events) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(e) = parse(&line) {
                    if tx.send(e).is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(Event::Ended);
        });
        self.status = "starting…".into();
        self.camera_on = false;
        self.service = Some(Service { child, _stdin: stdin, events });
    }

    fn stop(&mut self) {
        self.service = None;
        self.camera_on = false;
        self.status.clear();
    }

    fn react(&mut self, reaction: Reaction, at: [f32; 2]) {
        self.count += 1;
        self.bursts.push(Burst { reaction, start: Instant::now(), at, seed: self.count.wrapping_mul(7919) });
    }
}

impl State {
    /// Keep the service in step with the setting, take what it says and move the reactions along.
    pub fn gestures_tick(&mut self, dt: f32) {
        let want = self.config.privacy.gestures;
        if !want {
            self.gestures.gave_up = false;
            if self.gestures.service.is_some() {
                self.gestures.stop();
            }
        } else if self.gestures.service.is_none() && !self.gestures.gave_up {
            self.gestures.start();
        }

        let prefs = self.config.privacy;
        let mut ended = false;
        if let Some(s) = &self.gestures.service {
            let events: Vec<Event> = s.events.try_iter().collect();
            for e in events {
                match e {
                    Event::Status(t) => self.gestures.status = t,
                    Event::Error(t) => {
                        self.gestures.status = t;
                        self.gestures.camera_on = false;
                    }
                    Event::CameraOn => self.gestures.camera_on = true,
                    // the hand is where the camera saw it, and the camera looks at you like a mirror
                    Event::Gesture(r, at) if prefs.allows(r) => self.gestures.react(r, [1.0 - at[0], at[1]]),
                    Event::Gesture(..) => {}
                    Event::Ended => ended = true,
                }
            }
        }
        if ended {
            self.gestures.service = None;
            self.gestures.camera_on = false;
            // it ended on its own: say why, and do not keep starting it
            self.gestures.gave_up = true;
            if self.gestures.status.is_empty() || self.gestures.status == "starting…" {
                self.gestures.status = "the gesture service stopped".into();
            }
        }

        self.gestures.bursts.retain(|b| b.start.elapsed().as_secs_f32() < privacy::LIFE);
        let target = if self.gestures.camera_on && prefs.indicator { 1.0 } else { 0.0 };
        let g = &mut self.gestures;
        g.dot += (target - g.dot) * (dt * 9.0).min(1.0);
        if (target - g.dot).abs() < 0.004 {
            g.dot = target;
        }
    }

    /// What the Privacy page says under the switch.
    pub fn gesture_status_line(&self) -> String {
        let g = &self.gestures;
        if g.service.is_none() && !g.gave_up {
            return String::new();
        }
        if g.camera_on {
            return "in use".into();
        }
        g.status.clone()
    }

    /// The reactions and the camera mark, over everything.
    pub fn gesture_panels(&self, out: &mut Vec<Panel>) {
        privacy::indicator(self.size, self.gestures.dot, out);
        for b in &self.gestures.bursts {
            privacy::panels(b.reaction, b.seed, b.at, b.start.elapsed().as_secs_f32(), self.size, out);
        }
    }

    /// Play a reaction without the camera (the Privacy page's try buttons).
    pub fn test_gesture(&mut self, word: &str) {
        if let Some(r) = Reaction::parse(word) {
            self.gestures.react(r, [0.7, 0.6]);
        }
    }

    /// The shortcut: gestures on or off.
    pub fn toggle_gestures(&mut self) {
        self.config.privacy.gestures = !self.config.privacy.gestures;
        self.config.save();
    }
}
