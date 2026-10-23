//! `wlr-screencopy`: other programs may ask for the contents of the screen. This is what screenshot tools, OBS and
//! the screen-sharing portal (xdg-desktop-portal-wlr, which feeds PipeWire) use.
//!
//! Frames are delivered through shared memory. A plain `copy` is served on the next tick; `copy_with_damage` waits
//! until something on screen actually changed, so a shared screen that is not moving costs nothing.

use crate::state::State;
use smithay::{
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    wayland::shm::with_buffer_contents_mut,
};

/// What a frame object was created for.
pub struct FrameInfo {
    /// region in device pixels: x, y, w, h
    rect: [u32; 4],
}

/// A `copy` request waiting for its turn.
pub struct PendingCopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    rect: [u32; 4],
    /// wait for a change on screen first
    on_damage: bool,
    /// `frames_drawn` when the request came in
    since: u64,
}

impl State {
    pub fn init_screencopy(&mut self) {
        let dh = self.dh.clone();
        dh.create_global::<State, ZwlrScreencopyManagerV1, _>(3, ());
    }

    /// Serve the copy requests that are due. Called every tick; cheap when there are none.
    pub fn serve_screencopy(&mut self) {
        if self.screencopies.is_empty() {
            return;
        }
        let drawn = self.frames_drawn;
        let due = self.screencopies.iter().any(|p| !p.on_damage || drawn != p.since);
        if !due {
            return;
        }
        let (pw, ph, rgba) = self.capture_rgba(None);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let after = self.frames_drawn;
        let mut keep = Vec::new();
        for p in std::mem::take(&mut self.screencopies) {
            if p.on_damage && drawn == p.since {
                keep.push(p);
                continue;
            }
            let [rx, ry, rw, rh] = p.rect;
            let (rx, ry) = (rx.min(pw.saturating_sub(1)), ry.min(ph.saturating_sub(1)));
            let (rw, rh) = (rw.min(pw - rx), rh.min(ph - ry));
            let written = with_buffer_contents_mut(&p.buffer, |ptr, len, data| {
                let (w, h, stride) = (data.width as u32, data.height as u32, data.stride as usize);
                let ok_format = matches!(data.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);
                if !ok_format || w != rw || h != rh || stride < w as usize * 4 || data.offset as usize + stride * h as usize > len {
                    return false;
                }
                for y in 0..h as usize {
                    // SAFETY: the row lies inside the buffer (checked above) and the client mapped it writable
                    let row = unsafe { std::slice::from_raw_parts_mut(ptr.add(data.offset as usize + y * stride), w as usize * 4) };
                    let src = &rgba[(((ry as usize + y) * pw as usize + rx as usize) * 4)..][..w as usize * 4];
                    for (d, s) in row.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                    }
                }
                true
            });
            if matches!(written, Ok(true)) {
                p.frame.flags(Flags::empty());
                let secs = now.as_secs();
                p.frame.ready((secs >> 32) as u32, secs as u32, now.subsec_nanos());
            } else {
                p.frame.failed();
            }
        }
        self.screencopies = keep;
        // the capture itself drew a frame; waiting copies must not take that as a change
        for p in &mut self.screencopies {
            p.since = after;
        }
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ZwlrScreencopyManagerV1>, _: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &ZwlrScreencopyManagerV1, request: zwlr_screencopy_manager_v1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        let (pw, ph) = state.renderer.pixel_size();
        let (frame, rect) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, .. } => (frame, [0, 0, pw, ph]),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { frame, x, y, width, height, .. } => {
                // the region is given in logical pixels
                let (kx, ky) = (pw as f32 / state.size[0], ph as f32 / state.size[1]);
                let (x0, y0) = ((x.max(0) as f32 * kx).round() as u32, (y.max(0) as f32 * ky).round() as u32);
                let (w, h) = ((width.max(1) as f32 * kx).round() as u32, (height.max(1) as f32 * ky).round() as u32);
                (frame, [x0.min(pw - 1), y0.min(ph - 1), w.min(pw - x0.min(pw - 1)).max(1), h.min(ph - y0.min(ph - 1)).max(1)])
            }
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => return,
        };
        let frame = data_init.init(frame, FrameInfo { rect });
        frame.buffer(wl_shm::Format::Xrgb8888, rect[2], rect[3], rect[2] * 4);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameInfo> for State {
    fn request(state: &mut Self, _: &Client, frame: &ZwlrScreencopyFrameV1, request: zwlr_screencopy_frame_v1::Request, info: &FrameInfo, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        let (buffer, on_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => {
                state.screencopies.retain(|p| &p.frame != frame);
                return;
            }
            _ => return,
        };
        let since = state.frames_drawn;
        state.screencopies.push(PendingCopy { frame: frame.clone(), buffer, rect: info.rect, on_damage, since });
    }
}
