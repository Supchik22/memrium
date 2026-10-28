//! `ext-foreign-toplevel-list-v1`, `ext-image-capture-source-v1` and `ext-image-copy-capture-v1`: a program may list the
//! windows and ask for the picture of one of them (or of the screen). The screen-sharing portal of the desktop uses this
//! to offer "share a window".
//!
//! A window's picture is cut out of the finished frame, so it shows what is on screen at its place (a window above it
//! is in it too). Frames are delivered through shared memory, and only when something on screen changed.

use crate::state::{State, Sw};
use smithay::{
    reexports::{
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_foreign_toplevel_image_capture_source_manager_v1::{self, ExtForeignToplevelImageCaptureSourceManagerV1},
                ext_image_capture_source_v1::ExtImageCaptureSourceV1,
                ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
                ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
                ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
                ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
            },
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output::Transform, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    wayland::{
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState},
        shm::with_buffer_contents_mut,
    },
};
use std::{collections::HashMap, sync::Mutex};

/// What a capture source points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// the whole screen
    Screen,
    /// a Wayland / X11 window, by `Win::id`
    Win(u64),
    /// one of the shell's own windows (Settings, Files)
    Shell(Sw),
    /// a window that is gone
    Gone,
}

/// User data of a capture source.
pub struct Source(Target);

struct SessionInner {
    /// the size told to the program (device pixels)
    size: (u32, u32),
    /// `frames_drawn` after the last frame given out; 0 until the first one
    last_gen: u64,
    /// a frame object exists and has not been consumed
    frame_live: bool,
    stopped: bool,
}

pub struct SessionData {
    target: Target,
    inner: Mutex<SessionInner>,
}

#[derive(Default)]
struct FrameInner {
    buffer: Option<WlBuffer>,
    captured: bool,
}

pub struct FrameData {
    session: ExtImageCopyCaptureSessionV1,
    inner: Mutex<FrameInner>,
}

/// A frame whose `capture` request waits for its turn.
struct PendingFrame {
    frame: ExtImageCopyCaptureFrameV1,
    session: ExtImageCopyCaptureSessionV1,
}

struct Listed {
    handle: ForeignToplevelHandle,
    title: String,
    app_id: String,
}

pub struct ExtCapture {
    pub list: ForeignToplevelListState,
    windows: HashMap<u64, Listed>,
    shell: HashMap<u8, Listed>,
    /// toplevel identifier -> what it stands for
    targets: HashMap<String, Target>,
    pending: Vec<PendingFrame>,
}

impl ExtCapture {
    pub fn new(dh: &DisplayHandle) -> ExtCapture {
        dh.create_global::<State, ExtOutputImageCaptureSourceManagerV1, _>(1, ());
        dh.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, _>(1, ());
        dh.create_global::<State, ExtImageCopyCaptureManagerV1, _>(1, ());
        ExtCapture { list: ForeignToplevelListState::new::<State>(dh), windows: HashMap::new(), shell: HashMap::new(), targets: HashMap::new(), pending: Vec::new() }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.ext.list
    }
}
smithay::delegate_foreign_toplevel_list!(State);

fn shell_key(k: Sw) -> u8 {
    match k {
        Sw::Settings => 0,
        Sw::Files => 1,
    }
}

impl State {
    /// Keep the list of windows the programs see in step with the windows there are. Called every tick.
    pub fn sync_toplevels(&mut self) {
        let mut now: Vec<(u64, String, String)> = Vec::new();
        for w in self.wins.iter().filter(|w| w.mapped && !w.or) {
            let title = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
            now.push((w.id, title, w.app_id.clone()));
        }
        let gone: Vec<u64> = self.ext.windows.keys().copied().filter(|id| !now.iter().any(|n| n.0 == *id)).collect();
        for id in gone {
            if let Some(l) = self.ext.windows.remove(&id) {
                self.ext.targets.remove(&l.handle.identifier());
                l.handle.send_closed();
            }
        }
        for (id, title, app_id) in now {
            match self.ext.windows.get_mut(&id) {
                Some(l) => {
                    if l.title != title || l.app_id != app_id {
                        l.handle.send_title(&title);
                        l.handle.send_app_id(&app_id);
                        l.handle.send_done();
                        l.title = title;
                        l.app_id = app_id;
                    }
                }
                None => {
                    let handle = self.ext.list.new_toplevel::<State>(title.clone(), app_id.clone());
                    handle.send_done();
                    self.ext.targets.insert(handle.identifier(), Target::Win(id));
                    self.ext.windows.insert(id, Listed { handle, title, app_id });
                }
            }
        }
        for (k, title, app_id) in [(Sw::Settings, "Settings", "memrium_settings"), (Sw::Files, "Files", "memrium_files")] {
            let open = self.is_open(k);
            let key = shell_key(k);
            match (open, self.ext.shell.contains_key(&key)) {
                (true, false) => {
                    let handle = self.ext.list.new_toplevel::<State>(title, app_id);
                    handle.send_done();
                    self.ext.targets.insert(handle.identifier(), Target::Shell(k));
                    self.ext.shell.insert(key, Listed { handle, title: title.into(), app_id: app_id.into() });
                }
                (false, true) => {
                    if let Some(l) = self.ext.shell.remove(&key) {
                        self.ext.targets.remove(&l.handle.identifier());
                        l.handle.send_closed();
                    }
                }
                _ => {}
            }
        }
    }

    /// Where a target is on screen (logical, `None` for the whole screen). `Err` when it cannot be seen now: it is
    /// gone (the session ends) or hidden (the program waits).
    fn target_rect(&self, t: Target) -> Result<Option<[f32; 4]>, bool> {
        match t {
            Target::Screen => Ok(None),
            Target::Gone => Err(true),
            Target::Shell(k) => {
                if !self.is_open(k) {
                    Err(true)
                } else if !self.sw_visible(k) {
                    Err(false)
                } else {
                    Ok(Some(self.sw_frame(k)))
                }
            }
            Target::Win(id) => match self.wins.iter().find(|w| w.id == id) {
                None => Err(true),
                Some(w) if !w.mapped || w.minimized => Err(false),
                Some(w) => Ok(Some(Self::frame_rect(w))),
            },
        }
    }

    /// Hand out the frames that are due. Called every tick; cheap when none was asked for.
    pub fn serve_ext_capture(&mut self) {
        if self.ext.pending.is_empty() {
            return;
        }
        let drawn = self.frames_drawn;
        let mut keep = Vec::new();
        let mut cache: Vec<(Target, u32, u32, Vec<u8>)> = Vec::new();
        for p in std::mem::take(&mut self.ext.pending) {
            let (Some(sd), Some(fd)) = (p.session.data::<SessionData>(), p.frame.data::<FrameData>()) else { continue };
            if !p.frame.is_alive() {
                continue;
            }
            let rect = match self.target_rect(sd.target) {
                Ok(r) => r,
                Err(true) => {
                    let mut s = sd.inner.lock().unwrap();
                    if !s.stopped {
                        s.stopped = true;
                        p.session.stopped();
                    }
                    p.frame.failed(FailureReason::Stopped);
                    continue;
                }
                Err(false) => {
                    keep.push(p);
                    continue;
                }
            };
            let [x0, y0, x1, y1] = self.crop_px(rect);
            let size = (x1 - x0, y1 - y0);
            {
                let mut s = sd.inner.lock().unwrap();
                if s.size != size {
                    // the window changed its size: the program learns the new one and asks again
                    s.size = size;
                    s.frame_live = false;
                    p.session.buffer_size(size.0, size.1);
                    p.session.done();
                    p.frame.failed(FailureReason::BufferConstraints);
                    continue;
                }
                // nothing new on screen since the last frame: wait
                if s.last_gen != 0 && s.last_gen == drawn {
                    drop(s);
                    keep.push(p);
                    continue;
                }
            }
            let buffer = fd.inner.lock().unwrap().buffer.clone();
            let Some(buffer) = buffer else {
                p.frame.failed(FailureReason::Unknown);
                continue;
            };
            if !cache.iter().any(|c| c.0 == sd.target) {
                let (w, h, px) = self.capture_rgba(rect);
                cache.push((sd.target, w, h, px));
            }
            let (_, pw, ph, rgba) = cache.iter().find(|c| c.0 == sd.target).unwrap();
            let written = with_buffer_contents_mut(&buffer, |ptr, len, data| {
                let (w, h, stride) = (data.width as u32, data.height as u32, data.stride as usize);
                let ok_format = matches!(data.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);
                if !ok_format || w != *pw || h != *ph || stride < w as usize * 4 || data.offset as usize + stride * h as usize > len {
                    return false;
                }
                for y in 0..h as usize {
                    // SAFETY: the row lies inside the buffer (checked above) and the client mapped it writable
                    let row = unsafe { std::slice::from_raw_parts_mut(ptr.add(data.offset as usize + y * stride), w as usize * 4) };
                    let src = &rgba[y * w as usize * 4..][..w as usize * 4];
                    for (d, s) in row.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                    }
                }
                true
            });
            if matches!(written, Ok(true)) {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                let secs = now.as_secs();
                p.frame.transform(Transform::Normal);
                p.frame.damage(0, 0, size.0 as i32, size.1 as i32);
                p.frame.presentation_time((secs >> 32) as u32, secs as u32, now.subsec_nanos());
                p.frame.ready();
                let mut s = sd.inner.lock().unwrap();
                s.frame_live = false;
                // the capture drew a frame itself; it is not a change
                s.last_gen = self.frames_drawn;
            } else {
                sd.inner.lock().unwrap().frame_live = false;
                p.frame.failed(FailureReason::BufferConstraints);
            }
        }
        // the sessions that waited must not take the capture's own frame as news
        let after = self.frames_drawn;
        for p in &keep {
            if let Some(sd) = p.session.data::<SessionData>() {
                let mut s = sd.inner.lock().unwrap();
                if s.last_gen == drawn {
                    s.last_gen = after;
                }
            }
        }
        self.ext.pending.extend(keep);
    }
}

// ------------------------------------------------------------------ capture sources

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ExtOutputImageCaptureSourceManagerV1>, _: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &ExtOutputImageCaptureSourceManagerV1, request: ext_output_image_capture_source_manager_v1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource { source, .. } = request {
            // one output: the whole screen
            data_init.init(source, Source(Target::Screen));
        }
    }
}

impl GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>, _: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &ExtForeignToplevelImageCaptureSourceManagerV1, request: ext_foreign_toplevel_image_capture_source_manager_v1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource { source, toplevel_handle } = request {
            let target = ForeignToplevelHandle::from_resource(&toplevel_handle)
                .and_then(|h| state.ext.targets.get(&h.identifier()).copied())
                .unwrap_or(Target::Gone);
            data_init.init(source, Source(target));
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, Source> for State {
    fn request(_: &mut Self, _: &Client, _: &ExtImageCaptureSourceV1, _: <ExtImageCaptureSourceV1 as Resource>::Request, _: &Source, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
}

// ------------------------------------------------------------------ copy capture

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ExtImageCopyCaptureManagerV1>, _: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl State {
    /// A new session on `target`: tell the program the buffer it must bring.
    fn open_session(&self, session: New<ExtImageCopyCaptureSessionV1>, target: Target, data_init: &mut DataInit<'_, Self>) {
        let size = match self.target_rect(target) {
            Ok(r) => {
                let [x0, y0, x1, y1] = self.crop_px(r);
                (x1 - x0, y1 - y0)
            }
            // not on screen right now: a size to start with, corrected when it shows
            Err(_) => (1, 1),
        };
        let s = data_init.init(session, SessionData { target, inner: Mutex::new(SessionInner { size, last_gen: 0, frame_live: false, stopped: false }) });
        s.buffer_size(size.0, size.1);
        s.shm_format(wl_shm::Format::Xrgb8888);
        s.shm_format(wl_shm::Format::Argb8888);
        s.done();
        if target == Target::Gone {
            s.stopped();
        }
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &ExtImageCopyCaptureManagerV1, request: ext_image_copy_capture_manager_v1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession { session, source, .. } => {
                let target = source.data::<Source>().map(|s| s.0).unwrap_or(Target::Gone);
                state.open_session(session, target, data_init);
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession { session, .. } => {
                // the cursor is part of the picture; there is no separate cursor to follow
                data_init.init(session, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &ExtImageCopyCaptureCursorSessionV1, request: ext_image_copy_capture_cursor_session_v1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = request {
            state.open_session(session, Target::Gone, data_init);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, SessionData> for State {
    fn request(state: &mut Self, _: &Client, session: &ExtImageCopyCaptureSessionV1, request: ext_image_copy_capture_session_v1::Request, data: &SessionData, _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        match request {
            ext_image_copy_capture_session_v1::Request::CreateFrame { frame } => {
                let mut s = data.inner.lock().unwrap();
                if s.frame_live {
                    session.post_error(ext_image_copy_capture_session_v1::Error::DuplicateFrame, "a frame of this session is still waiting");
                    return;
                }
                s.frame_live = true;
                data_init.init(frame, FrameData { session: session.clone(), inner: Mutex::new(FrameInner::default()) });
            }
            ext_image_copy_capture_session_v1::Request::Destroy => {
                state.ext.pending.retain(|p| &p.session != session);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for State {
    fn request(state: &mut Self, _: &Client, frame: &ExtImageCopyCaptureFrameV1, request: ext_image_copy_capture_frame_v1::Request, data: &FrameData, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        match request {
            ext_image_copy_capture_frame_v1::Request::AttachBuffer { buffer } => {
                let mut f = data.inner.lock().unwrap();
                if f.captured {
                    frame.post_error(ext_image_copy_capture_frame_v1::Error::AlreadyCaptured, "capture was already requested");
                    return;
                }
                f.buffer = Some(buffer);
            }
            ext_image_copy_capture_frame_v1::Request::Capture => {
                let mut f = data.inner.lock().unwrap();
                if f.captured {
                    frame.post_error(ext_image_copy_capture_frame_v1::Error::AlreadyCaptured, "capture was already requested");
                    return;
                }
                if f.buffer.is_none() {
                    frame.post_error(ext_image_copy_capture_frame_v1::Error::NoBuffer, "capture without a buffer");
                    return;
                }
                f.captured = true;
                drop(f);
                state.ext.pending.push(PendingFrame { frame: frame.clone(), session: data.session.clone() });
            }
            ext_image_copy_capture_frame_v1::Request::Destroy => {
                if let Some(sd) = data.session.data::<SessionData>() {
                    sd.inner.lock().unwrap().frame_live = false;
                }
                state.ext.pending.retain(|p| &p.frame != frame);
            }
            _ => {}
        }
    }
}
