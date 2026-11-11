//! Search by meaning from the shell's side (see `memrium_de::semsearch`). While Settings -> Privacy has it on, the
//! indexer (`memrium_semindex`) runs on mains power: at once while the models are still to be fetched, otherwise only
//! when the computer has been idle a while, and at most every half hour once it has finished. A key press or going on
//! battery stops it (what it did is kept). Figlet gets a `Searcher` for its results. Off, nothing runs.

use crate::state::State;
use memrium_de::semsearch;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long the computer must be left alone before indexing starts, and how long a finished run is good for.
const IDLE: Duration = Duration::from_secs(90);
const FRESH: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub struct SemIndex {
    child: Option<Child>,
    next_check: Option<Instant>,
    last_done: Option<Instant>,
    /// what Settings shows under the switch
    pub status: String,
}

/// On mains power (or a computer with no battery at all).
fn on_mains() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return true };
    let mut mains_seen = false;
    for e in rd.flatten() {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() == "Mains" {
            mains_seen = true;
            if std::fs::read_to_string(p.join("online")).is_ok_and(|s| s.trim() == "1") {
                return true;
            }
        }
    }
    !mains_seen
}

impl State {
    /// Start or stop the indexer as things stand; checked every few seconds.
    pub fn semindex_tick(&mut self) {
        let now = Instant::now();
        if self.semindex.next_check.is_some_and(|t| now < t) {
            return;
        }
        self.semindex.next_check = Some(now + Duration::from_secs(4));
        let on = self.config.privacy.semantic_search;
        // Figlet asks the searcher only while it is on
        match (on, self.launcher.semantic.is_some()) {
            (true, false) => self.launcher.semantic = Some(semsearch::Searcher::new()),
            (false, true) => self.launcher.semantic = None,
            _ => {}
        }
        if let Some(c) = &mut self.semindex.child {
            if let Ok(Some(st)) = c.try_wait() {
                eprintln!("semindex: finished ({st})");
                self.semindex.child = None;
                if st.success() {
                    self.semindex.last_done = Some(now);
                }
            }
        }
        let need_models = on && !semsearch::models_present();
        let idle = self.last_input.elapsed() >= IDLE;
        let fresh = self.semindex.last_done.is_some_and(|t| t.elapsed() < FRESH);
        let want = on && on_mains() && (need_models || (idle && !fresh));
        match (want, self.semindex.child.as_ref().map(|c| c.id())) {
            (true, None) => {
                let mut cmd = Command::new(memrium_de::fsops::own_program("memrium_semindex"));
                cmd.stdin(Stdio::null()).stdout(Stdio::null());
                match cmd.spawn() {
                    Ok(c) => {
                        eprintln!("semindex: started (pid {})", c.id());
                        self.semindex.child = Some(c);
                    }
                    Err(e) => {
                        eprintln!("semindex: could not start memrium_semindex: {e}");
                        self.semindex.last_done = Some(now);
                    }
                }
            }
            (false, Some(pid)) => {
                // it keeps what it did; reaped on a later check
                // SAFETY: a plain signal to our own child
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            }
            _ => {}
        }
        self.semindex.status = if on { semsearch::status() } else { String::new() };
    }

    /// Settings -> Privacy: forget everything indexed.
    pub fn semindex_forget(&mut self) {
        if let Some(c) = &self.semindex.child {
            // SAFETY: a plain signal to our own child
            unsafe { libc::kill(c.id() as libc::pid_t, libc::SIGTERM) };
        }
        let _ = std::fs::remove_dir_all(semsearch::index_dir());
        self.semindex.last_done = None;
        self.semindex.status.clear();
    }
}
