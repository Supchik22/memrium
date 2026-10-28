//! `org.freedesktop.impl.portal.ScreenCast`: a program asks to see the screen or a window (a browser's "share your
//! screen", a recorder). The user chooses in `memrium_share`; the picture is taken from the compositor and offered as a
//! PipeWire node, whose id goes back to the portal.

use crate::grab::{self, Choice};
use crate::stream::PwStream;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedValue, Type, Value};
use zbus::{interface, Connection};

type Options = HashMap<String, OwnedValue>;
type Reply = (u32, HashMap<String, OwnedValue>);

/// A running share.
struct Running {
    stop: Arc<AtomicBool>,
    stream: Arc<PwStream>,
    capture: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Session {
    /// 1 = a monitor, 2 = a window (what the program allows)
    types: u32,
    running: Option<Running>,
}

type Sessions = Arc<Mutex<HashMap<String, Session>>>;

impl Session {
    fn end(&mut self) {
        if let Some(r) = self.running.take() {
            r.stop.store(true, Ordering::SeqCst);
            r.stream.stop();
            if let Some(t) = r.capture {
                let _ = t.join();
            }
        }
    }
}

struct SessionObject {
    path: String,
    sessions: Sessions,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        let gone = self.sessions.lock().ok().and_then(|mut s| s.remove(&self.path));
        if let Some(mut s) = gone {
            blocking::unblock(move || s.end()).await;
        }
        let _ = server.remove::<SessionObject, _>(self.path.as_str()).await;
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// The request object the portal may `Close` while the dialog is open.
struct Request {
    pid: Arc<AtomicU32>,
    closed: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid != 0 {
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
}

/// One entry of the `streams` answer: the node and what it shows.
#[derive(Type, Value)]
struct StreamInfo {
    node: u32,
    props: HashMap<String, OwnedValue>,
}

pub struct ScreenCast {
    sessions: Sessions,
}

impl ScreenCast {
    pub fn new() -> ScreenCast {
        ScreenCast { sessions: Arc::new(Mutex::new(HashMap::new())) }
    }
}

fn owned<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).expect("a plain value")
}

/// Starts `memrium_share` and waits for it. `None` when the user cancelled or the request was closed.
async fn choose(conn: &Connection, handle: &str, app: &str, types: u32) -> Option<Choice> {
    let wins = blocking::unblock(grab::list_toplevels).await;
    let mut args: Vec<String> = vec!["--app".into(), app.into()];
    if types & 1 != 0 {
        args.push("--screen".into());
    }
    if types & 2 != 0 {
        for w in wins {
            args.push("--window".into());
            args.push(format!("{}\x1f{}\x1f{}", w.identifier, w.title, w.app_id));
        }
    }
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("memrium_share"))).filter(|p| p.exists()).unwrap_or_else(|| "memrium_share".into());
    let (pid, closed) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
    conn.object_server().at(handle, Request { pid: pid.clone(), closed: closed.clone() }).await.ok()?;
    let child = Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn();
    let out = match child {
        Ok(child) => {
            pid.store(child.id(), Ordering::SeqCst);
            if closed.load(Ordering::SeqCst) {
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            }
            blocking::unblock(move || child.wait_with_output()).await.ok()
        }
        Err(e) => {
            eprintln!("memrium_portal: cannot start memrium_share: {e}");
            None
        }
    };
    let _ = conn.object_server().remove::<Request, _>(handle).await;
    if closed.load(Ordering::SeqCst) {
        return None;
    }
    let text = String::from_utf8(out?.stdout).ok()?;
    let json: serde_json::Value = text.lines().rev().find(|l| l.starts_with('{')).and_then(|l| serde_json::from_str(l).ok())?;
    match json["choice"].as_str()? {
        "screen" => Some(Choice::Screen),
        id => Some(Choice::Window(id.to_string())),
    }
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        // hidden, embedded in the picture
        3
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    async fn create_session(&self, #[zbus(object_server)] server: &zbus::ObjectServer, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, _app_id: &str, _options: Options) -> Reply {
        let path = session_handle.to_string();
        // a session made again under the same name ends the old one
        let old = self.sessions.lock().unwrap().insert(path.clone(), Session { types: 3, running: None });
        if let Some(mut old) = old {
            blocking::unblock(move || old.end()).await;
        }
        if server.at(session_handle, SessionObject { path, sessions: self.sessions.clone() }).await.is_err() {
            return (2, HashMap::new());
        }
        (0, HashMap::new())
    }

    async fn select_sources(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, _app_id: &str, options: Options) -> Reply {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions.get_mut(session_handle.as_str()) else { return (2, HashMap::new()) };
        if let Some(t) = options.get("types").and_then(|v| if let Value::U32(t) = &**v { Some(*t) } else { None }) {
            s.types = t & 3;
        }
        (0, HashMap::new())
    }

    async fn start(&self, #[zbus(connection)] conn: &Connection, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, _parent_window: &str, _options: Options) -> Reply {
        let key = session_handle.to_string();
        let types = match self.sessions.lock().unwrap().get(&key) {
            Some(s) => s.types.max(1),
            None => return (2, HashMap::new()),
        };
        let name = if app_id.is_empty() { "A program".to_string() } else { app_id.to_string() };
        let Some(choice) = choose(conn, handle.as_str(), &name, types).await else { return (1, HashMap::new()) };
        let (size_choice, source_type) = (choice.clone(), if matches!(choice, Choice::Screen) { 1u32 } else { 2 });
        let started = blocking::unblock(move || -> Result<Running, String> {
            let size = grab::probe(&size_choice)?;
            let stream = Arc::new(PwStream::start(size.0, size.1, "Memrium screen share")?);
            let stop = Arc::new(AtomicBool::new(false));
            let (s2, st2, c2) = (stream.clone(), stop.clone(), size_choice.clone());
            let capture = std::thread::spawn(move || {
                if let Err(e) = grab::run(c2, &st2, |f| s2.push(f)) {
                    eprintln!("memrium_portal: screen share ended: {e}");
                }
            });
            Ok(Running { stop, stream, capture: Some(capture) })
        })
        .await;
        let running = match started {
            Ok(r) => r,
            Err(e) => {
                eprintln!("memrium_portal: cannot share: {e}");
                return (2, HashMap::new());
            }
        };
        let node = running.stream.node_id;
        match self.sessions.lock().unwrap().get_mut(&key) {
            Some(s) => s.running = Some(running),
            None => {
                // closed while it was starting
                running.stop.store(true, Ordering::SeqCst);
                running.stream.stop();
                return (1, HashMap::new());
            }
        }
        let mut props = HashMap::new();
        props.insert("source_type".to_string(), owned(source_type));
        let info = StreamInfo { node, props };
        let mut out = HashMap::new();
        out.insert("streams".to_string(), owned(vec![info]));
        (0, out)
    }
}
