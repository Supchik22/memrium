//! A bare X11 window that writes the pointer positions it is told (window coordinates, X pixels) to the file named by
//! `PROBE_OUT`, for checking how the compositor maps the pointer into an X11 window at a fractional scale:
//! `MEMRIUM_HEADLESS_SCALE=1.25 MEMRIUM_TEST_INPUT="..." memrium_compositor --headless --exec x11_probe`
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::COPY_DEPTH_FROM_PARENT;

fn main() {
    // PROBE_DISPLAY (e.g. ":1") is required: never fall back to the display of whatever session the shell runs in. The test
    // compositor's X server may start after this program does, so it is waited for.
    let display = std::env::var("PROBE_DISPLAY").expect("PROBE_DISPLAY, the X display of the compositor under test");
    let started = std::time::Instant::now();
    let (conn, screen) = loop {
        match RustConnection::connect(Some(&display)) {
            Ok(c) => break c,
            Err(e) if started.elapsed().as_secs() > 20 => panic!("X display {display}: {e}"),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    };
    let root = conn.setup().roots[screen].root;
    let win = conn.generate_id().unwrap();
    let (w, h) = (800u16, 500u16);
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        w,
        h,
        0,
        WindowClass::INPUT_OUTPUT,0,
        &CreateWindowAux::new().event_mask(EventMask::POINTER_MOTION | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION).background_pixel(0x203040),
    )
    .unwrap();
    // PROBE_CURSOR=1: the window asks for a cursor of its own (a glyph of the cursor font, as a game's toolkit would)
    if std::env::var_os("PROBE_CURSOR").is_some() {
        let font = conn.generate_id().unwrap();
        conn.open_font(font, b"cursor").unwrap();
        let cur = conn.generate_id().unwrap();
        // 34 = XC_crosshair
        conn.create_glyph_cursor(cur, font, font, 34, 35, 0, 0, 0, 0xffff, 0xffff, 0xffff).unwrap();
        conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().cursor(cur)).unwrap();
    }
    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    let start = std::time::Instant::now();
    let out = std::env::var("PROBE_OUT").unwrap_or_else(|_| "probe.txt".into());
    let mut log = std::fs::File::create(&out).unwrap();
    use std::io::Write;
    while start.elapsed().as_secs() < 12 {
        match conn.poll_for_event() {
            Ok(Some(Event::MotionNotify(m))) => { let _ = write!(log, "{:>6} motion {} {} state {}\n", start.elapsed().as_millis(), m.event_x, m.event_y, u16::from(m.state)).and_then(|_| log.flush()); }
            Ok(Some(Event::ButtonPress(b))) => { let _ = write!(log, "{:>6} down {} {} button {}\n", start.elapsed().as_millis(), b.event_x, b.event_y, b.detail).and_then(|_| log.flush()); }
            Ok(Some(Event::ButtonRelease(b))) => { let _ = write!(log, "{:>6} up {} {} button {}\n", start.elapsed().as_millis(), b.event_x, b.event_y, b.detail).and_then(|_| log.flush()); }
            Ok(Some(_)) => {}
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(_) => break,
        }
    }
}
