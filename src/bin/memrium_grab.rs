//! Screenshot tool for any compositor that speaks wlr-screencopy (this one does): `memrium_grab out.png`.
//! Also the test for the screencopy implementation.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use wayland_client::{
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry, wl_shm, wl_shm_pool::WlShmPool},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

#[derive(Default)]
struct App {
    shm: Option<wl_shm::WlShm>,
    output: Option<WlOutput>,
    manager: Option<ZwlrScreencopyManagerV1>,
    /// (format, width, height, stride) the compositor wants
    wanted: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_output" if state.output.is_none() => state.output = Some(registry.bind(name, version.min(3), qh, ())),
                "zwlr_screencopy_manager_v1" => state.manager = Some(registry.bind(name, version.min(3), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for App {
    fn event(state: &mut Self, _: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer { format: WEnum::Value(f), width, height, stride } => state.wanted = Some((f, width, height, stride)),
            zwlr_screencopy_frame_v1::Event::BufferDone => state.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.failed = true,
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(impl Dispatch<$t, ()> for App { fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} })*};
}
ignore!(wl_shm::WlShm, WlOutput, WlShmPool, WlBuffer, ZwlrScreencopyManagerV1);

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "grab.png".into());
    let conn = Connection::connect_to_env().expect("no Wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut app = App::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut app).unwrap();
    let (Some(shm), Some(output), Some(manager)) = (app.shm.clone(), app.output.clone(), app.manager.clone()) else {
        eprintln!("the compositor does not offer wlr-screencopy");
        std::process::exit(1);
    };
    let frame = manager.capture_output(0, &output, &qh, ());
    while !app.buffer_done && app.wanted.is_none() {
        queue.blocking_dispatch(&mut app).unwrap();
    }
    let (format, w, h, stride) = app.wanted.expect("no buffer description");
    // SAFETY: memfd_create returns a fresh descriptor that we own
    let fd = unsafe { OwnedFd::from_raw_fd(libc::memfd_create(c"grab".as_ptr(), 0)) };
    let size = (stride * h) as usize;
    assert!(unsafe { libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd), size as i64) } == 0);
    let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &qh, ());
    frame.copy(&buffer);
    while !app.ready && !app.failed {
        queue.blocking_dispatch(&mut app).unwrap();
    }
    if app.failed {
        eprintln!("the compositor could not copy the screen");
        std::process::exit(1);
    }
    // SAFETY: the memfd is `size` bytes long and nothing else writes to it any more
    let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, std::os::fd::AsRawFd::as_raw_fd(&fd), 0) };
    let bytes = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for px in bytes[y * stride as usize..][..w as usize * 4].chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).expect("could not write the image");
    println!("{path}: {w}x{h}");
}
