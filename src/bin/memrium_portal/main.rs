//! `memrium_portal`: the xdg-desktop-portal backend of the desktop. It answers `org.freedesktop.impl.portal.FileChooser`
//! (what every GTK, Qt and Electron program's "Open" and "Save" go through) by starting `memrium_files` as the dialog.
//! It is started by the bus when the portal first asks (see `session/portal/`).

mod grab;
mod screencast;
mod stream;

use std::collections::HashMap;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

const BUS: &str = "org.freedesktop.impl.portal.desktop.memrium";
const PATH: &str = "/org/freedesktop/portal/desktop";

type Options = HashMap<String, OwnedValue>;
type Reply = (u32, HashMap<String, OwnedValue>);

/// The request object the portal may `Close` while the dialog is open: closing the dialog ends the request.
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

struct FileChooser;

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(&self, #[zbus(connection)] conn: &Connection, handle: ObjectPath<'_>, _app_id: &str, _parent_window: &str, title: &str, options: Options) -> Reply {
        let mut args = vec!["--choose".to_string(), if flag(&options, "directory") { "dir" } else { "open" }.into(), "--title".into(), title.into()];
        if flag(&options, "multiple") {
            args.push("--multiple".into());
        }
        common(&mut args, &options);
        if let Some(folder) = options.get("current_folder").and_then(path_of) {
            args.extend(["--folder".into(), folder.to_string_lossy().into_owned()]);
        }
        let Some(answer) = ask(conn, handle.as_str(), args).await else { return (1, HashMap::new()) };
        let mut out = results(&answer, &options);
        out.insert("uris".into(), strings(answer.paths.iter().map(|p| uri(p)).collect()));
        (0, out)
    }

    async fn save_file(&self, #[zbus(connection)] conn: &Connection, handle: ObjectPath<'_>, _app_id: &str, _parent_window: &str, title: &str, options: Options) -> Reply {
        let mut args = vec!["--choose".to_string(), "save".into(), "--title".into(), title.into()];
        common(&mut args, &options);
        // the program names the file, or points at one (`current_file`) whose folder and name are the start
        let current = options.get("current_file").and_then(path_of);
        let name = options.get("current_name").and_then(string_of).or_else(|| current.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()));
        let folder = options.get("current_folder").and_then(path_of).or_else(|| current.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf()));
        if let Some(name) = name {
            args.extend(["--name".into(), name]);
        }
        if let Some(folder) = folder {
            args.extend(["--folder".into(), folder.to_string_lossy().into_owned()]);
        }
        let Some(answer) = ask(conn, handle.as_str(), args).await else { return (1, HashMap::new()) };
        let mut out = results(&answer, &options);
        out.insert("uris".into(), strings(answer.paths.iter().map(|p| uri(p)).collect()));
        (0, out)
    }

    /// Several files with given names go into one folder: the dialog picks the folder.
    async fn save_files(&self, #[zbus(connection)] conn: &Connection, handle: ObjectPath<'_>, _app_id: &str, _parent_window: &str, title: &str, options: Options) -> Reply {
        let mut args = vec!["--choose".to_string(), "dir".into(), "--title".into(), title.into(), "--accept".into(), "Save".into()];
        if let Some(folder) = options.get("current_folder").and_then(path_of) {
            args.extend(["--folder".into(), folder.to_string_lossy().into_owned()]);
        }
        let Some(answer) = ask(conn, handle.as_str(), args).await else { return (1, HashMap::new()) };
        let Some(dir) = answer.paths.first() else { return (1, HashMap::new()) };
        let names: Vec<PathBuf> = match options.get("files").map(|v| &**v) {
            Some(Value::Array(a)) => a.iter().filter_map(|f| bytes(f).map(|b| PathBuf::from(std::ffi::OsString::from_vec(b)))).collect(),
            _ => Vec::new(),
        };
        let mut out = HashMap::new();
        out.insert("uris".into(), strings(names.iter().map(|n| uri(&dir.join(n.file_name().unwrap_or_default()))).collect()));
        (0, out)
    }
}

struct Answer {
    paths: Vec<PathBuf>,
    filter: usize,
}

/// The flags of the filters, the label of the button and the like, which open and save share.
fn common(args: &mut Vec<String>, options: &Options) {
    if let Some(label) = options.get("accept_label").and_then(string_of) {
        args.extend(["--accept".into(), label.replace('_', "")]);
    }
    for f in filters(options) {
        args.extend(["--filter".into(), f.0]);
    }
    let all = filters(options);
    let current = options.get("current_filter").and_then(|v| match &**v {
        Value::Structure(s) => s.fields().first().and_then(|n| if let Value::Str(n) = n { Some(n.to_string()) } else { None }),
        _ => None,
    });
    if let Some(at) = current.and_then(|c| all.iter().position(|f| f.2 == c)) {
        args.extend(["--current-filter".into(), at.to_string()]);
    }
}

/// `a(sa(us))` as (the `--filter` argument, the filter as the portal sent it, its name).
fn filters(options: &Options) -> Vec<(String, Value<'static>, String)> {
    let Some(Value::Array(list)) = options.get("filters").map(|v| &**v) else { return Vec::new() };
    list.iter()
        .filter_map(|f| {
            let Value::Structure(s) = f else { return None };
            let [Value::Str(name), Value::Array(rules)] = s.fields() else { return None };
            let mut arg = name.to_string();
            for r in rules.iter() {
                if let Value::Structure(r) = r {
                    if let [Value::U32(kind), Value::Str(pat)] = r.fields() {
                        arg.push('\x1f');
                        arg.push_str(if *kind == 1 { "m:" } else { "g:" });
                        arg.push_str(pat);
                    }
                }
            }
            Some((arg, f.try_clone().ok()?.try_to_owned().ok().map(|o| (*o).try_clone().ok()).flatten()?, name.to_string()))
        })
        .collect()
}

/// What the portal wants back besides the files: the filter that was current (programs add its extension when saving).
fn results(answer: &Answer, options: &Options) -> HashMap<String, OwnedValue> {
    let mut out = HashMap::new();
    if let Some((_, value, _)) = filters(options).into_iter().nth(answer.filter) {
        if let Ok(v) = OwnedValue::try_from(value) {
            out.insert("current_filter".into(), v);
        }
    }
    out
}

/// Starts the dialog and waits for it. `None` when the user cancelled it, it could not start, or the request was closed.
async fn ask(conn: &Connection, handle: &str, args: Vec<String>) -> Option<Answer> {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("memrium_files"))).filter(|p| p.exists()).unwrap_or_else(|| "memrium_files".into());
    let (pid, closed) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
    conn.object_server().at(handle, Request { pid: pid.clone(), closed: closed.clone() }).await.ok()?;
    let child = Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn();
    let out = match child {
        Ok(child) => {
            pid.store(child.id(), Ordering::SeqCst);
            // `Close` may have come before the pid was known
            if closed.load(Ordering::SeqCst) {
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            }
            blocking::unblock(move || child.wait_with_output()).await.ok()
        }
        Err(e) => {
            eprintln!("memrium_portal: cannot start memrium_files: {e}");
            None
        }
    };
    let _ = conn.object_server().remove::<Request, _>(handle).await;
    if closed.load(Ordering::SeqCst) {
        return None;
    }
    let text = String::from_utf8(out?.stdout).ok()?;
    let json: serde_json::Value = text.lines().rev().find(|l| l.starts_with('{')).and_then(|l| serde_json::from_str(l).ok())?;
    let paths: Vec<PathBuf> = json["paths"].as_array()?.iter().filter_map(|p| p.as_str().map(PathBuf::from)).collect();
    (!paths.is_empty()).then(|| Answer { paths, filter: json["filter"].as_u64().unwrap_or(0) as usize })
}

fn flag(options: &Options, key: &str) -> bool {
    options.get(key).is_some_and(|v| matches!(**v, Value::Bool(true)))
}

fn string_of(v: &OwnedValue) -> Option<String> {
    if let Value::Str(s) = &**v {
        Some(s.to_string())
    } else {
        None
    }
}

fn bytes(v: &Value<'_>) -> Option<Vec<u8>> {
    let Value::Array(a) = v else { return None };
    let mut out: Vec<u8> = a.iter().map(|x| if let Value::U8(b) = x { Some(*b) } else { None }).collect::<Option<_>>()?;
    // a path comes as a nul-terminated byte string
    while out.last() == Some(&0) {
        out.pop();
    }
    Some(out)
}

fn path_of(v: &OwnedValue) -> Option<PathBuf> {
    bytes(v).filter(|b| !b.is_empty()).map(|b| PathBuf::from(std::ffi::OsString::from_vec(b)))
}

fn strings(v: Vec<String>) -> OwnedValue {
    OwnedValue::try_from(Value::from(v)).expect("strings are a plain value")
}

/// `file://` + the path with everything but the unreserved characters and `/` percent-encoded.
fn uri(p: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `memrium_portal --grab-test OUT.png [window]`: take one picture of the screen (or of the first window) the way a share
/// does, to see that the compositor side works without a portal client.
fn grab_test(out: &str, window: bool) {
    let list = grab::list_toplevels();
    for t in &list {
        println!("window {} \"{}\" ({})", t.identifier, t.title, t.app_id);
    }
    let choice = match (window, list.first()) {
        (true, Some(t)) => grab::Choice::Window(t.identifier.clone()),
        (true, None) => return eprintln!("no window to take"),
        _ => grab::Choice::Screen,
    };
    let stop = AtomicBool::new(false);
    let mut done = false;
    let result = grab::run(choice, &stop, |f| {
        if !done {
            done = true;
            let rgba: Vec<u8> = f.bgrx.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            if let Some(img) = image::RgbaImage::from_raw(f.w, f.h, rgba) {
                let _ = img.save(out);
            }
            println!("picture {}x{} -> {out}", f.w, f.h);
            stop.store(true, Ordering::SeqCst);
        }
    });
    if let Err(e) = result {
        eprintln!("grab failed: {e}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--grab-test") {
        return grab_test(args.get(2).map(String::as_str).unwrap_or("/tmp/grab.png"), args.get(3).map(String::as_str) == Some("window"));
    }
    let result: zbus::Result<()> = zbus::block_on(async {
        let _conn = zbus::connection::Builder::session()?.name(BUS)?.serve_at(PATH, FileChooser)?.serve_at(PATH, screencast::ScreenCast::new())?.build().await?;
        std::future::pending::<()>().await;
        Ok(())
    });
    if let Err(e) = result {
        eprintln!("memrium_portal: {e}");
        std::process::exit(1);
    }
}
