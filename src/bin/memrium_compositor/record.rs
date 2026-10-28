//! Recording a demo: the headless shell is stepped on a virtual clock (a fixed time per frame, so the video does not depend on how
//! fast the machine renders), and every frame is written as two small pictures for the video editor: the whole screen, and a
//! "camera" view that follows the pointer and comes closer when it stops. `MEMRIUM_RECORD=<dir>`, `MEMRIUM_RECORD_FPS=30`,
//! `MEMRIUM_RECORD_SECS=40`, view size `MEMRIUM_RECORD_VIEW=1000x900`.

use image::imageops::{self, FilterType};
use std::io::Write;

pub struct Recorder {
    dir: std::path::PathBuf,
    pub fps: f32,
    pub secs: f32,
    view: (u32, u32),
    log: std::fs::File,
    frame: u32,
    /// camera centre and zoom (logical px), and their speeds
    c: [f32; 2],
    vc: [f32; 2],
    z: f32,
    vz: f32,
    last_mouse: [f32; 2],
    speed: f32,
    /// seconds since the last click
    since_click: f32,
    /// a zoom the script asked for (0 = decide by itself)
    pub zoom_override: f32,
    pub down: bool,
    pub clicked: bool,
    /// the camera looks here instead of at the pointer
    pub focus: Option<[f32; 2]>,
    /// the shortcut pressed at this frame (shown as a key cap in the video)
    pub keys: Option<String>,
    started: bool,
}

impl Recorder {
    pub fn from_env() -> Option<Recorder> {
        let dir = std::path::PathBuf::from(std::env::var_os("MEMRIUM_RECORD")?);
        std::fs::create_dir_all(&dir).ok()?;
        let fps = std::env::var("MEMRIUM_RECORD_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
        let secs = std::env::var("MEMRIUM_RECORD_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
        let view = std::env::var("MEMRIUM_RECORD_VIEW").ok().and_then(|v| v.split_once('x').map(|(w, h)| (w.parse().unwrap_or(1000u32), h.parse().unwrap_or(900u32)))).unwrap_or((1000, 900));
        let log = std::fs::File::create(dir.join("track.jsonl")).ok()?;
        Some(Recorder { dir, fps, secs, view, log, frame: 0, c: [0.0; 2], vc: [0.0; 2], z: 1.0, vz: 0.0, last_mouse: [0.0; 2], speed: 0.0, since_click: 9.0, zoom_override: 0.0, down: false, clicked: false, focus: None, keys: None, started: false })
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.fps
    }

    pub fn done(&self) -> bool {
        self.frame as f32 >= self.secs * self.fps
    }

    /// Write frame `n` from the full-screen RGBA picture `px` (`w` x `h` device pixels; `scale` device px per logical px).
    pub fn push(&mut self, px: Vec<u8>, w: u32, h: u32, scale: f32, mouse: [f32; 2]) {
        let dt = self.dt();
        let (lw, lh) = (w as f32 / scale, h as f32 / scale);
        let aspect = self.view.0 as f32 / self.view.1 as f32;
        if !self.started {
            self.started = true;
            self.c = mouse;
            self.z = 1.4;
            self.last_mouse = mouse;
        }
        // how fast the pointer moves (smoothed), and the time since a click
        let v = ((mouse[0] - self.last_mouse[0]).hypot(mouse[1] - self.last_mouse[1])) / dt;
        self.speed += (v - self.speed) * (1.0 - (-dt * 8.0).exp());
        self.last_mouse = mouse;
        self.since_click = if self.clicked { 0.0 } else { self.since_click + dt };
        // closer when it rests or has just clicked, further out while it travels
        let rest = 1.0 - (self.speed / 900.0).clamp(0.0, 1.0);
        let look = self.focus.unwrap_or(mouse);
        let mut zt = 1.55 + 0.75 * rest;
        if self.since_click < 0.9 {
            zt = zt.max(2.2);
        }
        if self.zoom_override > 0.0 {
            zt = self.zoom_override;
        }
        // the part of the screen shown: as tall as the screen at zoom 1 (and as wide as the view's shape asks)
        let (base_h, base_w) = (lh.min(lw / aspect), lh.min(lw / aspect) * aspect);
        // critically damped springs for the centre and the zoom (in small steps, so a long frame time cannot make them blow up)
        let (wc, wz) = (7.0f32, 4.5f32);
        let n = ((dt * 60.0).ceil() as usize).max(1);
        let sh = dt / n as f32;
        for _ in 0..n {
            for i in 0..2 {
                let a = wc * wc * (look[i] - self.c[i]) - 2.0 * wc * self.vc[i];
                self.vc[i] += a * sh;
                self.c[i] += self.vc[i] * sh;
            }
            let a = wz * wz * (zt - self.z) - 2.0 * wz * self.vz;
            self.vz += a * sh;
            self.z += self.vz * sh;
        }
        self.z = self.z.clamp(1.0, 4.0);
        let (rw, rh) = (base_w / self.z, base_h / self.z);
        let rx = (self.c[0] - rw / 2.0).clamp(0.0, (lw - rw).max(0.0));
        let ry = (self.c[1] - rh / 2.0).clamp(0.0, (lh - rh).max(0.0));

        let full = image::RgbaImage::from_raw(w, h, px).expect("frame size");
        let crop = imageops::crop_imm(&full, (rx * scale) as u32, (ry * scale) as u32, (rw * scale) as u32, (rh * scale) as u32).to_image();
        let view = imageops::resize(&crop, self.view.0, self.view.1, FilterType::Lanczos3);
        let ov_w = 1000u32;
        let ov = imageops::resize(&full, ov_w, ov_w * h / w, FilterType::Triangle);
        let save = |img: &image::RgbaImage, name: String| {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let mut f = std::fs::File::create(self.dir.join(name)).unwrap();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 93).encode_image(&rgb).unwrap();
        };
        save(&view, format!("v{:05}.jpg", self.frame));
        save(&ov, format!("o{:05}.jpg", self.frame));
        // where the pointer is inside the camera view, and the view's place on the screen (shares of the screen)
        let (px_, py_) = ((mouse[0] - rx) / rw * self.view.0 as f32, (mouse[1] - ry) / rh * self.view.1 as f32);
        let _ = writeln!(self.log, "{{\"f\":{},\"mx\":{:.1},\"my\":{:.1},\"cx\":{:.4},\"cy\":{:.4},\"cw\":{:.4},\"ch\":{:.4},\"ox\":{:.4},\"oy\":{:.4},\"down\":{},\"click\":{},\"z\":{:.3},\"key\":{}}}", self.frame, px_, py_, rx / lw, ry / lh, rw / lw, rh / lh, mouse[0] / lw, mouse[1] / lh, self.down, self.clicked, self.z, self.keys.take().map_or("null".to_string(), |k| format!("\"{k}\"")));
        self.clicked = false;
        self.frame += 1;
    }
}
