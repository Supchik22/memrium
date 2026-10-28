//! What is playing: the first MPRIS media player on the session bus (Spotify, Firefox, mpv, VLC ...), read on a
//! background thread, and the play / next / previous buttons that talk back to it.

use std::collections::HashMap;
use std::sync::{
    mpsc::{channel, Sender},
    Arc, Mutex,
};
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// where the cover is (file:// or http(s)://), empty when there is none
    pub art: String,
    /// false when there is no player at all
    pub present: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum MediaCmd {
    PlayPause,
    Next,
    Previous,
}

pub struct Media {
    pub info: Arc<Mutex<MediaInfo>>,
    tx: Sender<Wake>,
}

/// What wakes the watching thread: a button pressed, or a player that said it changed.
enum Wake {
    Cmd(MediaCmd),
    Changed,
}

impl Media {
    /// Start watching the players. They are read again when one of them says it changed (or a player comes or goes),
    /// not every second and a half as before: that woke the shell and every player all the time.
    pub fn start() -> Media {
        let info = Arc::new(Mutex::new(MediaInfo::default()));
        let (tx, rx) = channel::<Wake>();
        let shared = info.clone();
        let changes = tx.clone();
        std::thread::spawn(move || {
            let Ok(conn) = Connection::session() else { return };
            watch_players(&conn, changes);
            let read = |conn: &Connection| {
                let found = find_player(conn);
                if let Ok(mut g) = shared.lock() {
                    *g = found.as_ref().map(|(i, _)| i.clone()).unwrap_or_default();
                }
                found.map(|(_, p)| p)
            };
            let press = |player: &Option<Proxy<'static>>, cmd: MediaCmd| {
                if let Some(p) = player {
                    let name = match cmd {
                        MediaCmd::PlayPause => "PlayPause",
                        MediaCmd::Next => "Next",
                        MediaCmd::Previous => "Previous",
                    };
                    let _ = p.call_method(name, &());
                }
                // give the player a moment to change, then look again at once
                std::thread::sleep(Duration::from_millis(150));
            };
            let mut player = read(&conn);
            loop {
                // (a slow look now and then, for a player that changes without saying so)
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(Wake::Cmd(cmd)) => press(&player, cmd),
                    Ok(Wake::Changed) => {
                        // a change comes as a burst of signals: read once after it (a button pressed meanwhile still works)
                        std::thread::sleep(Duration::from_millis(40));
                        while let Ok(w) = rx.try_recv() {
                            if let Wake::Cmd(cmd) = w {
                                press(&player, cmd);
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                player = read(&conn);
            }
        });
        Media { info, tx }
    }

    pub fn send(&self, cmd: MediaCmd) {
        let _ = self.tx.send(Wake::Cmd(cmd));
    }

    pub fn get(&self) -> MediaInfo {
        self.info.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// Tell the watching thread whenever a player's properties change (status, track) or a player comes or goes.
fn watch_players(conn: &Connection, tx: Sender<Wake>) {
    let rules = [
        zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|b| b.member("PropertiesChanged")).and_then(|b| b.path("/org/mpris/MediaPlayer2")).map(|b| b.build()),
        zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg0ns("org.mpris.MediaPlayer2")).map(|b| b.build()),
    ];
    for rule in rules.into_iter().flatten() {
        let (conn, tx) = (conn.clone(), tx.clone());
        std::thread::spawn(move || {
            let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(32)) else { return };
            for _ in signals.flatten() {
                if tx.send(Wake::Changed).is_err() {
                    return;
                }
            }
        });
    }
}

/// The player to show: one that is playing, else the first there is.
fn find_player(conn: &Connection) -> Option<(MediaInfo, Proxy<'static>)> {
    let bus = Proxy::new(conn, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").ok()?;
    let names: Vec<String> = bus.call("ListNames", &()).ok()?;
    let mut best: Option<(MediaInfo, Proxy<'static>)> = None;
    for n in names.into_iter().filter(|n| n.starts_with("org.mpris.MediaPlayer2.")) {
        let Ok(p) = Proxy::new(conn, n, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player") else { continue };
        let status: String = p.get_property("PlaybackStatus").unwrap_or_default();
        let meta: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();
        let title = meta.get("xesam:title").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
        let artist = meta
            .get("xesam:artist")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
            .map(|a| a.join(", "))
            .unwrap_or_default();
        let art = meta.get("mpris:artUrl").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
        let info = MediaInfo { title, artist, playing: status == "Playing", art, present: true };
        let better = best.as_ref().map_or(true, |(b, _)| info.playing && !b.playing);
        if better {
            best = Some((info, p));
        }
    }
    best
}
