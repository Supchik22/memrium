//! The Wayland side of screen sharing: lists the windows (`ext-foreign-toplevel-list`) and takes pictures of one of
//! them, or of the screen (`ext-image-copy-capture`).

use crate::stream::Frame;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry::WlRegistry, wl_shm::{self, WlShm}, wl_shm_pool::WlShmPool},
    event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
        ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

/// A window as the compositor lists it.
#[derive(Clone, Debug)]
pub struct Toplevel {
    pub identifier: String,
    pub title: String,
    pub app_id: String,
}

/// What to take pictures of.
#[derive(Clone, Debug)]
pub enum Choice {
    Screen,
    Window(String),
}

struct Entry {
    handle: ExtForeignToplevelHandleV1,
    info: Toplevel,
    closed: bool,
}

enum FrameEnd {
    Waiting,
    Ready,
    Failed(WEnum<FailureReason>),
}

struct Cap {
    toplevels: Vec<Entry>,
    size: (u32, u32),
    formats: Vec<u32>,
    session_done: bool,
    stopped: bool,
    frame: FrameEnd,
}

type Bound = (WlShm, Option<ExtForeignToplevelListV1>, Option<ExtForeignToplevelImageCaptureSourceManagerV1>, Option<ExtOutputImageCaptureSourceManagerV1>, Option<ExtImageCopyCaptureManagerV1>, Option<WlOutput>);

impl Dispatch<WlRegistry, GlobalListContents> for Cap {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

delegate_noop!(Cap: ignore WlShm);
delegate_noop!(Cap: ignore WlOutput);
delegate_noop!(Cap: ignore WlShmPool);
delegate_noop!(Cap: ignore WlBuffer);
delegate_noop!(Cap: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Cap: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Cap: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(Cap: ignore ExtImageCaptureSourceV1);

impl Dispatch<ExtForeignToplevelListV1, ()> for Cap {
    fn event(state: &mut Self, _: &ExtForeignToplevelListV1, event: ext_foreign_toplevel_list_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push(Entry { handle: toplevel, info: Toplevel { identifier: String::new(), title: String::new(), app_id: String::new() }, closed: false });
        }
    }

    event_created_child!(Cap, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Cap {
    fn event(state: &mut Self, handle: &ExtForeignToplevelHandleV1, event: ext_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(e) = state.toplevels.iter_mut().find(|e| &e.handle == handle) else { return };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Closed => e.closed = true,
            ext_foreign_toplevel_handle_v1::Event::Title { title } => e.info.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => e.info.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => e.info.identifier = identifier,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Cap {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureSessionV1, event: ext_image_copy_capture_session_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => state.size = (width, height),
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                if let WEnum::Value(f) = format {
                    state.formats.push(f as u32);
                }
            }
            ext_image_copy_capture_session_v1::Event::Done => state.session_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Cap {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureFrameV1, event: ext_image_copy_capture_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame = FrameEnd::Ready,
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => state.frame = FrameEnd::Failed(reason),
            _ => {}
        }
    }
}

fn connect() -> Result<(Connection, EventQueue<Cap>, Cap, Bound), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<Cap>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(|_| "the compositor has no wl_shm".to_string())?;
    let list = globals.bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ()).ok();
    let tl_src = globals.bind::<ExtForeignToplevelImageCaptureSourceManagerV1, _, _>(&qh, 1..=1, ()).ok();
    let out_src = globals.bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(&qh, 1..=1, ()).ok();
    let copy = globals.bind::<ExtImageCopyCaptureManagerV1, _, _>(&qh, 1..=1, ()).ok();
    let output = globals.bind::<WlOutput, _, _>(&qh, 1..=1, ()).ok();
    let mut cap = Cap { toplevels: Vec::new(), size: (0, 0), formats: Vec::new(), session_done: false, stopped: false, frame: FrameEnd::Waiting };
    queue.roundtrip(&mut cap).map_err(|e| e.to_string())?;
    // the windows told themselves in the first round; their details come in the second
    queue.roundtrip(&mut cap).map_err(|e| e.to_string())?;
    Ok((conn, queue, cap, (shm, list, tl_src, out_src, copy, output)))
}

/// The windows the compositor lists now.
pub fn list_toplevels() -> Vec<Toplevel> {
    match connect() {
        Ok((_, _, cap, _)) => cap.toplevels.into_iter().filter(|e| !e.closed && !e.info.identifier.is_empty()).map(|e| e.info).collect(),
        Err(e) => {
            eprintln!("memrium_portal: cannot list windows: {e}");
            Vec::new()
        }
    }
}

/// The size a first picture of `choice` will have.
pub fn probe(choice: &Choice) -> Result<(u32, u32), String> {
    let (_conn, mut queue, mut cap, bound) = connect()?;
    let qh = queue.handle();
    let (_session, _source) = open_session(&mut queue, &mut cap, &bound, &qh, choice)?;
    Ok(cap.size)
}

fn open_session(queue: &mut EventQueue<Cap>, cap: &mut Cap, bound: &Bound, qh: &QueueHandle<Cap>, choice: &Choice) -> Result<(ExtImageCopyCaptureSessionV1, ExtImageCaptureSourceV1), String> {
    let (_, _, tl_src, out_src, copy, output) = bound;
    let copy = copy.as_ref().ok_or("the compositor cannot copy images (ext-image-copy-capture)")?;
    let source = match choice {
        Choice::Screen => out_src.as_ref().zip(output.as_ref()).ok_or("the compositor cannot share the screen")?.0.create_source(output.as_ref().unwrap(), qh, ()),
        Choice::Window(id) => {
            let tl_src = tl_src.as_ref().ok_or("the compositor cannot share windows")?;
            let e = cap.toplevels.iter().find(|e| &e.info.identifier == id && !e.closed).ok_or("that window is gone")?;
            tl_src.create_source(&e.handle, qh, ())
        }
    };
    cap.session_done = false;
    cap.formats.clear();
    let session = copy.create_session(&source, ext_image_copy_capture_manager_v1_options(), qh, ());
    while !cap.session_done && !cap.stopped {
        queue.blocking_dispatch(cap).map_err(|e| e.to_string())?;
    }
    if cap.stopped {
        return Err("that window cannot be shared".into());
    }
    Ok((session, source))
}

fn ext_image_copy_capture_manager_v1_options() -> wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::Options {
    wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::Options::PaintCursors
}

struct Shm {
    pool: WlShmPool,
    buffer: WlBuffer,
    map: *mut u8,
    len: usize,
}

impl Drop for Shm {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
        unsafe { libc::munmap(self.map as *mut _, self.len) };
    }
}

fn make_shm(shm: &WlShm, qh: &QueueHandle<Cap>, size: (u32, u32), format: wl_shm::Format) -> Result<Shm, String> {
    let (w, h) = size;
    let len = (w * h * 4) as usize;
    let fd = unsafe { libc::memfd_create(c"memrium-share".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 || unsafe { libc::ftruncate(fd, len as i64) } != 0 {
        return Err("no shared memory".into());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let map = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
    if map == libc::MAP_FAILED {
        return Err("cannot map shared memory".into());
    }
    let pool = shm.create_pool(fd.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, format, qh, ());
    Ok(Shm { pool, buffer, map: map as *mut u8, len })
}

/// Take pictures of `choice` and hand each to `on_frame`, until `stop` is set or the window is gone.
pub fn run(choice: Choice, stop: &AtomicBool, mut on_frame: impl FnMut(Frame)) -> Result<(), String> {
    let (conn, mut queue, mut cap, bound) = connect()?;
    let qh = queue.handle();
    let (session, _source) = open_session(&mut queue, &mut cap, &bound, &qh, &choice)?;
    let mut shm: Option<Shm> = None;
    let mut have = (0, 0);
    while !stop.load(Ordering::SeqCst) && !cap.stopped {
        if shm.is_none() || have != cap.size {
            let format = if cap.formats.contains(&(wl_shm::Format::Xrgb8888 as u32)) { wl_shm::Format::Xrgb8888 } else { wl_shm::Format::Argb8888 };
            shm = Some(make_shm(&bound.0, &qh, cap.size, format)?);
            have = cap.size;
        }
        let Some(buf) = shm.as_ref() else { break };
        cap.frame = FrameEnd::Waiting;
        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buf.buffer);
        frame.capture();
        conn.flush().map_err(|e| e.to_string())?;
        // wait for the picture, looking at `stop` now and then
        while matches!(cap.frame, FrameEnd::Waiting) && !stop.load(Ordering::SeqCst) && !cap.stopped {
            pump(&conn, &mut queue, &mut cap)?;
        }
        match std::mem::replace(&mut cap.frame, FrameEnd::Waiting) {
            FrameEnd::Ready => {
                let mut bgrx = vec![0u8; buf.len];
                unsafe { std::ptr::copy_nonoverlapping(buf.map, bgrx.as_mut_ptr(), buf.len) };
                on_frame(Frame { w: have.0, h: have.1, bgrx });
            }
            FrameEnd::Failed(reason) => {
                if matches!(reason, WEnum::Value(FailureReason::BufferConstraints)) {
                    // the size changed: the session told the new one
                    while !cap.session_done && !cap.stopped && !stop.load(Ordering::SeqCst) {
                        pump(&conn, &mut queue, &mut cap)?;
                    }
                    cap.session_done = false;
                } else if matches!(reason, WEnum::Value(FailureReason::Stopped)) {
                    break;
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            FrameEnd::Waiting => {}
        }
        frame.destroy();
    }
    session.destroy();
    let _ = conn.flush();
    Ok(())
}

/// Read what the compositor sent, waiting at most a moment.
fn pump(conn: &Connection, queue: &mut EventQueue<Cap>, cap: &mut Cap) -> Result<(), String> {
    queue.dispatch_pending(cap).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    if let Some(guard) = conn.prepare_read() {
        let fd = guard.connection_fd().as_raw_fd();
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let n = unsafe { libc::poll(&mut p, 1, 200) };
        if n > 0 {
            guard.read().map_err(|e| e.to_string())?;
        }
    }
    queue.dispatch_pending(cap).map_err(|e| e.to_string())?;
    Ok(())
}
