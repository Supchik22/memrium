//! Raw touchpad fingers for Settings' touchpad test. libinput does not tell where each finger is (only pointer motion,
//! scrolling and gestures), so a second descriptor of the same device is read as plain evdev events (multi-touch slots).

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Settings is showing the test: the session reads the pad.
pub static WANTED: AtomicBool = AtomicBool::new(false);
/// What the pad shows now.
pub static SNAP: Mutex<Snap> = Mutex::new(Snap::new());

#[derive(Clone, Debug, Default)]
pub struct Finger {
    pub id: i32,
    /// 0..1 across the pad
    pub x: f32,
    pub y: f32,
    /// 0..1 how hard (when the pad says), else 0.5
    pub press: f32,
}

#[derive(Clone, Debug)]
pub struct Snap {
    /// there is a touchpad with multi-touch to read
    pub present: bool,
    /// width / height of the pad's surface (real millimetres when the pad says them)
    pub aspect: f32,
    pub fingers: Vec<Finger>,
    /// the most fingers at once since the test began
    pub peak: usize,
    /// how many touches there were since the test began
    pub touches: u32,
}

impl Snap {
    pub const fn new() -> Snap {
        Snap { present: false, aspect: 1.5, fingers: Vec::new(), peak: 0, touches: 0 }
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AbsInfo {
    value: i32,
    min: i32,
    max: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

const EV_ABS: u16 = 3;
const EV_SYN: u16 = 0;
const SYN_REPORT: u16 = 0;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_PRESSURE: u16 = 0x3a;

fn eviocgabs(abs: u16) -> libc::c_ulong {
    // _IOR('E', 0x40 + abs, struct input_absinfo)
    (2 << 30) | ((std::mem::size_of::<AbsInfo>() as libc::c_ulong) << 16) | ((b'E' as libc::c_ulong) << 8) | (0x40 + abs as libc::c_ulong)
}

#[derive(Clone, Copy, Default)]
struct Slot {
    id: i32,
    x: i32,
    y: i32,
    p: i32,
}

pub struct Probe {
    fd: OwnedFd,
    // (kept alive with the descriptor)
    _keep: Option<Box<dyn std::any::Any>>,
    x: AbsInfo,
    y: AbsInfo,
    p: AbsInfo,
    slots: Vec<Slot>,
    cur: usize,
    was_wanted: bool,
}

impl Probe {
    /// `fd`: a new descriptor of the touchpad's event device; `keep` is whatever must live as long as it (the seat's handle).
    pub fn new(fd: OwnedFd, keep: Option<Box<dyn std::any::Any>>) -> Option<Probe> {
        let raw = fd.as_raw_fd();
        let get = |abs: u16| -> Option<AbsInfo> {
            let mut a = AbsInfo::default();
            (unsafe { libc::ioctl(raw, eviocgabs(abs) as _, &mut a as *mut AbsInfo) } == 0).then_some(a)
        };
        let (x, y) = (get(ABS_MT_POSITION_X)?, get(ABS_MT_POSITION_Y)?);
        let slot = get(ABS_MT_SLOT)?;
        let p = get(ABS_MT_PRESSURE).unwrap_or_default();
        unsafe {
            let fl = libc::fcntl(raw, libc::F_GETFL);
            libc::fcntl(raw, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let n = (slot.max + 1).clamp(1, 16) as usize;
        let slots = vec![Slot { id: -1, ..Slot::default() }; n];
        // millimetres when the pad gives its resolution, else the raw units
        let (rx, ry) = (x.resolution.max(1) as f32, y.resolution.max(1) as f32);
        let aspect = ((x.max - x.min).max(1) as f32 / rx) / ((y.max - y.min).max(1) as f32 / ry);
        if let Ok(mut s) = SNAP.lock() {
            s.present = true;
            s.aspect = aspect.clamp(0.8, 2.5);
        }
        Some(Probe { fd, _keep: keep, x, y, p, slots, cur: 0, was_wanted: false })
    }

    /// Read what the pad sent since the last call and publish the fingers. Call when the loop woke for input.
    pub fn poll(&mut self, wanted: bool) {
        let mut buf = [0u8; 24 * 64];
        let mut changed = false;
        loop {
            let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            if !wanted {
                continue;
            }
            for ev in buf[..n as usize].chunks_exact(24) {
                let ty = u16::from_ne_bytes([ev[16], ev[17]]);
                let code = u16::from_ne_bytes([ev[18], ev[19]]);
                let val = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                if ty == EV_ABS {
                    match code {
                        ABS_MT_SLOT => self.cur = (val.max(0) as usize).min(self.slots.len() - 1),
                        ABS_MT_TRACKING_ID => {
                            let s = &mut self.slots[self.cur];
                            if val >= 0 && s.id < 0 {
                                if let Ok(mut sn) = SNAP.lock() {
                                    sn.touches += 1;
                                }
                            }
                            s.id = val;
                        }
                        ABS_MT_POSITION_X => self.slots[self.cur].x = val,
                        ABS_MT_POSITION_Y => self.slots[self.cur].y = val,
                        ABS_MT_PRESSURE => self.slots[self.cur].p = val,
                        _ => {}
                    }
                } else if ty == EV_SYN && code == SYN_REPORT {
                    changed = true;
                }
            }
        }
        if !wanted {
            // fingers that lifted while nobody looked must not show as stuck when the test opens
            for s in &mut self.slots {
                s.id = -1;
            }
            if self.was_wanted {
                if let Ok(mut sn) = SNAP.lock() {
                    sn.fingers.clear();
                }
            }
            self.was_wanted = false;
            return;
        }
        if !self.was_wanted {
            self.was_wanted = true;
            if let Ok(mut sn) = SNAP.lock() {
                sn.peak = 0;
                sn.touches = 0;
            }
        }
        if changed {
            let (x, y, p) = (self.x, self.y, self.p);
            let fingers: Vec<Finger> = self
                .slots
                .iter()
                .filter(|s| s.id >= 0)
                .map(|s| Finger {
                    id: s.id,
                    x: ((s.x - x.min) as f32 / (x.max - x.min).max(1) as f32).clamp(0.0, 1.0),
                    y: ((s.y - y.min) as f32 / (y.max - y.min).max(1) as f32).clamp(0.0, 1.0),
                    press: if p.max > p.min { ((s.p - p.min) as f32 / (p.max - p.min) as f32).clamp(0.0, 1.0) } else { 0.5 },
                })
                .collect();
            if let Ok(mut sn) = SNAP.lock() {
                sn.peak = sn.peak.max(fingers.len());
                sn.fingers = fingers;
            }
        }
    }
}
