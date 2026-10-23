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
    tx: Sender<MediaCmd>,
}

impl Media {
    /// Start watching the players.
    pub fn start() -> Media {
        let info = Arc::new(Mutex::new(MediaInfo::default()));
        let (tx, rx) = channel::<MediaCmd>();
        let shared = info.clone();
        std::thread::spawn(move || {
            let Ok(conn) = Connection::session() else { return };
            loop {
                let player = find_player(&conn);
                if let Ok(cmd) = rx.recv_timeout(Duration::from_millis(1500)) {
                    if let Some((_, p)) = &player {
                        let name = match cmd {
                            MediaCmd::PlayPause => "PlayPause",
                            MediaCmd::Next => "Next",
                            MediaCmd::Previous => "Previous",
                        };
                        let _ = p.call_method(name, &());
                    }
                    // give the player a moment to change, then look again at once
                    std::thread::sleep(Duration::from_millis(150));
                }
                let now = match find_player(&conn) {
                    Some((info, _)) => info,
                    None => MediaInfo::default(),
                };
                if let Ok(mut g) = shared.lock() {
                    *g = now;
                }
            }
        });
        Media { info, tx }
    }

    pub fn send(&self, cmd: MediaCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn get(&self) -> MediaInfo {
        self.info.lock().map(|g| g.clone()).unwrap_or_default()
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
