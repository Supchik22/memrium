//! The screenshot editor: after an area is chosen (Print, or Shift+Print) its picture is taken and shown where it was, with a
//! small bar to draw on it (pen, rectangle), to blur parts of it, to take back the last stroke and to save it or drop it.
//! Also the shot of one window with a see-through background (Alt+Print).

use crate::state::{State, Sw, Win, BTN_LEFT};
use image::RgbaImage;
use memrium_de::renderer::{Panel, SurfaceTex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Rect,
    Blur,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tb {
    Tool(Tool),
    Color(usize),
    Width(usize),
    Undo,
    Save,
    Cancel,
}

pub const COLORS: [[u8; 3]; 6] = [[239, 68, 68], [250, 204, 21], [34, 197, 94], [59, 130, 246], [255, 255, 255], [20, 20, 24]];
/// stroke widths, logical px
pub const WIDTHS: [f32; 3] = [3.0, 6.0, 11.0];

const BTN: f32 = 34.0;
const BAR_H: f32 = 46.0;

enum Op {
    Pen { pts: Vec<[f32; 2]>, color: [u8; 3], w: f32 },
    Rect { a: [f32; 2], b: [f32; 2], color: [u8; 3], w: f32 },
    Blur { a: [f32; 2], b: [f32; 2] },
}

pub struct ShotEdit {
    /// where the picture was taken, logical desktop px
    rect: [f32; 4],
    base: RgbaImage,
    work: RgbaImage,
    tex: SurfaceTex,
    pub tool: Tool,
    color: usize,
    width: usize,
    ops: Vec<Op>,
    cur: Option<Op>,
}

impl ShotEdit {
    pub fn tex(&self) -> &SurfaceTex {
        &self.tex
    }

    pub fn rect(&self) -> [f32; 4] {
        self.rect
    }

    /// image px per logical px
    fn k(&self) -> (f32, f32) {
        (self.work.width() as f32 / self.rect[2].max(1.0), self.work.height() as f32 / self.rect[3].max(1.0))
    }

    /// a logical desktop point as a point of the picture
    fn to_img(&self, p: [f32; 2]) -> [f32; 2] {
        let (kx, ky) = self.k();
        [(p[0] - self.rect[0]) * kx, (p[1] - self.rect[1]) * ky]
    }

    fn inside(&self, p: [f32; 2]) -> bool {
        p[0] >= self.rect[0] && p[0] <= self.rect[0] + self.rect[2] && p[1] >= self.rect[1] && p[1] <= self.rect[1] + self.rect[3]
    }
}

/// Paint `c` over a pixel with `cover` (0 .. 1) of it.
fn blend(img: &mut RgbaImage, x: i32, y: i32, c: [u8; 3], cover: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 || cover <= 0.0 {
        return;
    }
    let px = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        px[i] = (px[i] as f32 * (1.0 - cover) + c[i] as f32 * cover).round() as u8;
    }
    px[3] = 255;
}

fn disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, c: [u8; 3]) {
    let (x0, x1, y0, y1) = ((cx - r - 1.0).floor() as i32, (cx + r + 1.0).ceil() as i32, (cy - r - 1.0).floor() as i32, (cy + r + 1.0).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            blend(img, x, y, c, (r + 0.5 - d).clamp(0.0, 1.0));
        }
    }
}

fn line(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], w: f32, c: [u8; 3]) {
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let step = (w * 0.25).max(0.5);
    let n = (len / step).ceil().max(1.0) as u32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        disc(img, a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, w / 2.0, c);
    }
}

fn rect_outline(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], w: f32, c: [u8; 3]) {
    let (x0, y0, x1, y1) = (a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1]));
    line(img, [x0, y0], [x1, y0], w, c);
    line(img, [x1, y0], [x1, y1], w, c);
    line(img, [x1, y1], [x0, y1], w, c);
    line(img, [x0, y1], [x0, y0], w, c);
}

/// Blur the part of the picture inside `a`..`b` so that it cannot be read.
fn blur_rect(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], sigma: f32) {
    let (x0, y0) = (a[0].min(b[0]).max(0.0) as u32, a[1].min(b[1]).max(0.0) as u32);
    let (x1, y1) = ((a[0].max(b[0]).max(0.0) as u32).min(img.width()), (a[1].max(b[1]).max(0.0) as u32).min(img.height()));
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return;
    }
    let part = image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image();
    // blurring only the part would leave its edge sharp against clear neighbours, but it is the part that must not be read
    let blurred = image::imageops::blur(&part, sigma);
    image::imageops::replace(img, &blurred, x0 as i64, y0 as i64);
}

impl ShotEdit {
    fn apply(&self, img: &mut RgbaImage, op: &Op) {
        let (kx, _) = self.k();
        match op {
            Op::Pen { pts, color, w } => {
                if let [only] = pts.as_slice() {
                    disc(img, only[0], only[1], w * kx / 2.0, *color);
                }
                for seg in pts.windows(2) {
                    line(img, seg[0], seg[1], w * kx, *color);
                }
            }
            Op::Rect { a, b, color, w } => rect_outline(img, *a, *b, w * kx, *color),
            Op::Blur { a, b } => blur_rect(img, *a, *b, (9.0 * kx).max(4.0)),
        }
    }

    /// The picture as it is with the finished strokes on it.
    fn rebuild(&mut self) {
        let mut img = self.base.clone();
        let ops = std::mem::take(&mut self.ops);
        for op in &ops {
            self.apply(&mut img, op);
        }
        self.ops = ops;
        self.work = img;
    }
}

impl State {
    /// Take the picture of `logical` and open the editor on it.
    pub fn start_shot_edit(&mut self, logical: [f32; 4]) {
        let [wx0, wy0, wx1, wy1] = self.world;
        let x0 = logical[0].max(wx0);
        let y0 = logical[1].max(wy0);
        let r = [x0, y0, (logical[0] + logical[2]).min(wx1) - x0, (logical[1] + logical[3]).min(wy1) - y0];
        if r[2] < 4.0 || r[3] < 4.0 {
            return;
        }
        let (w, h, rgba) = self.capture_rgba(Some(r));
        let Some(base) = RgbaImage::from_raw(w, h, rgba) else { return };
        let tex = self.renderer.create_surface_tex(w, h, true);
        let edit = ShotEdit { rect: r, work: base.clone(), base, tex, tool: Tool::Pen, color: 0, width: 1, ops: Vec::new(), cur: None };
        self.upload_rows(&edit, 0..h);
        self.shot_edit = Some(edit);
    }

    fn upload_rows(&self, e: &ShotEdit, rows: std::ops::Range<u32>) {
        let rows = rows.start.min(e.work.height())..rows.end.min(e.work.height());
        if rows.is_empty() {
            return;
        }
        let stride = e.work.width() as usize * 4;
        let mut bgra = Vec::with_capacity(stride * rows.len());
        for y in rows.clone() {
            for px in e.work.as_raw()[y as usize * stride..(y as usize + 1) * stride].chunks_exact(4) {
                bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        self.renderer.upload_surface_rows(&e.tex, &bgra, stride as u32, rows);
    }

    /// The bar's frame and its buttons (logical px), placed under the picture, or above it when there is no room.
    fn shot_bar(&self) -> Option<([f32; 4], Vec<(Tb, [f32; 4])>)> {
        let e = self.shot_edit.as_ref()?;
        let mut items: Vec<(Tb, f32)> = vec![(Tb::Tool(Tool::Pen), 0.0), (Tb::Tool(Tool::Rect), 0.0), (Tb::Tool(Tool::Blur), 0.0)];
        for i in 0..COLORS.len() {
            items.push((Tb::Color(i), if i == 0 { 12.0 } else { 0.0 }));
        }
        for i in 0..WIDTHS.len() {
            items.push((Tb::Width(i), if i == 0 { 12.0 } else { 0.0 }));
        }
        items.push((Tb::Undo, 12.0));
        items.push((Tb::Save, 12.0));
        items.push((Tb::Cancel, 0.0));
        let w = 20.0 + items.iter().map(|(_, gap)| BTN + gap).sum::<f32>();
        let [wx0, wy0, wx1, wy1] = self.world;
        let r = e.rect;
        let x = (r[0] + r[2] / 2.0 - w / 2.0).clamp(wx0 + 8.0, (wx1 - w - 8.0).max(wx0 + 8.0));
        let mut y = r[1] + r[3] + 12.0;
        if y + BAR_H > wy1 - 8.0 {
            y = r[1] - BAR_H - 12.0;
        }
        if y < wy0 + 8.0 {
            y = (r[1] + r[3] - BAR_H - 12.0).max(wy0 + 8.0);
        }
        let mut cx = x + 10.0;
        let mut out = Vec::new();
        for (tb, gap) in items {
            cx += gap;
            out.push((tb, [cx, y + (BAR_H - BTN) / 2.0, BTN, BTN]));
            cx += BTN;
        }
        Some(([x, y, w, BAR_H], out))
    }

    fn shot_bar_hit(&self, p: [f32; 2]) -> Option<Tb> {
        let (frame, btns) = self.shot_bar()?;
        if !Self::in_rect(p, frame) {
            return None;
        }
        btns.into_iter().find(|(_, r)| Self::in_rect(p, *r)).map(|(t, _)| t)
    }

    /// The pointer went down or up with the editor open.
    pub fn shot_edit_button(&mut self, button: u32, pressed: bool) {
        if button != BTN_LEFT {
            return;
        }
        let p = self.mouse;
        if !pressed {
            self.shot_edit_finish_stroke();
            return;
        }
        if let Some(tb) = self.shot_bar_hit(p) {
            self.shot_bar_action(tb);
            return;
        }
        let on_bar = self.shot_bar().is_some_and(|(f, _)| p[0] >= f[0] && p[0] <= f[0] + f[2] && p[1] >= f[1] && p[1] <= f[1] + f[3]);
        let Some(e) = self.shot_edit.as_mut() else { return };
        if !e.inside(p) {
            // a press beside the picture starts a new selection (the drawing so far is dropped)
            if !on_bar {
                self.shot_edit = None;
                self.shot_sel = Some(crate::state::ShotSel { start: Some(p), record: false });
            }
            return;
        }
        let q = e.to_img(p);
        let (color, w) = (COLORS[e.color], WIDTHS[e.width]);
        e.cur = Some(match e.tool {
            Tool::Pen => Op::Pen { pts: vec![q], color, w },
            Tool::Rect => Op::Rect { a: q, b: q, color, w },
            Tool::Blur => Op::Blur { a: q, b: q },
        });
        self.shot_edit_motion();
    }

    pub fn shot_edit_motion(&mut self) {
        let p = self.mouse;
        let Some(mut e) = self.shot_edit.take() else { return };
        if e.cur.is_some() {
            let q = e.to_img([p[0].clamp(e.rect[0], e.rect[0] + e.rect[2]), p[1].clamp(e.rect[1], e.rect[1] + e.rect[3])]);
            let (kx, _) = e.k();
            let mut rows = None;
            match e.cur.as_mut() {
                Some(Op::Pen { pts, color, w }) => {
                    let last = *pts.last().unwrap_or(&q);
                    if pts.len() == 1 || (last[0] - q[0]).abs() + (last[1] - q[1]).abs() >= 1.0 {
                        pts.push(q);
                        let (color, w) = (*color, *w * kx);
                        line(&mut e.work, last, q, w, color);
                        if pts.len() == 2 {
                            disc(&mut e.work, last[0], last[1], w / 2.0, color);
                        }
                        let reach = w + 2.0;
                        rows = Some((last[1].min(q[1]) - reach).max(0.0) as u32..(last[1].max(q[1]) + reach).ceil() as u32 + 1);
                    }
                }
                Some(Op::Rect { b, .. }) | Some(Op::Blur { b, .. }) => *b = q,
                None => {}
            }
            if let Some(rows) = rows {
                self.upload_rows(&e, rows);
            }
        }
        self.shot_edit = Some(e);
    }

    fn shot_edit_finish_stroke(&mut self) {
        let Some(mut e) = self.shot_edit.take() else { return };
        if let Some(op) = e.cur.take() {
            match &op {
                // a pen stroke is on the picture already
                Op::Pen { .. } => {}
                _ => {
                    let mut img = std::mem::take(&mut e.work);
                    e.apply(&mut img, &op);
                    e.work = img;
                }
            }
            e.ops.push(op);
            let h = e.work.height();
            self.upload_rows(&e, 0..h);
        }
        self.shot_edit = Some(e);
    }

    fn shot_bar_action(&mut self, tb: Tb) {
        match tb {
            Tb::Save => self.shot_edit_save(),
            Tb::Cancel => self.shot_edit = None,
            Tb::Undo => self.shot_edit_undo(),
            Tb::Tool(t) => {
                if let Some(e) = self.shot_edit.as_mut() {
                    e.tool = t;
                }
            }
            Tb::Color(i) => {
                if let Some(e) = self.shot_edit.as_mut() {
                    e.color = i;
                    if e.tool == Tool::Blur {
                        e.tool = Tool::Pen;
                    }
                }
            }
            Tb::Width(i) => {
                if let Some(e) = self.shot_edit.as_mut() {
                    e.width = i;
                }
            }
        }
    }

    pub fn shot_edit_undo(&mut self) {
        let Some(mut e) = self.shot_edit.take() else { return };
        e.cur = None;
        if e.ops.pop().is_some() {
            e.rebuild();
            let h = e.work.height();
            self.upload_rows(&e, 0..h);
        }
        self.shot_edit = Some(e);
    }

    pub fn shot_edit_save(&mut self) {
        let Some(e) = self.shot_edit.take() else { return };
        let (w, h) = e.work.dimensions();
        self.finish_shot(e.work.into_raw(), w, h, false);
    }

    /// A key with the editor open. Every key belongs to it.
    pub fn shot_edit_key(&mut self, ctrl: bool, sym: smithay::input::keyboard::Keysym) {
        use smithay::input::keyboard::Keysym;
        match sym {
            Keysym::Escape => self.shot_edit = None,
            Keysym::Return | Keysym::KP_Enter => self.shot_edit_save(),
            Keysym::z | Keysym::Z if ctrl => self.shot_edit_undo(),
            _ => {}
        }
    }

    /// The editor's drawing: the picture's frame, what is being drawn, and the bar.
    pub fn shot_edit_overlay(&self, shell: &mut Vec<Panel>) {
        let Some(e) = self.shot_edit.as_ref() else { return };
        let [wx0, wy0, wx1, wy1] = self.world;
        let sw = wx1 - wx0;
        let r = e.rect;
        let dim = |x: f32, y: f32, w: f32, h: f32| Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(0.38);
        shell.push(dim(wx0, wy0, sw, r[1] - wy0));
        shell.push(dim(wx0, r[1] + r[3], sw, wy1 - r[1] - r[3]));
        shell.push(dim(wx0, r[1], r[0] - wx0, r[3]));
        shell.push(dim(r[0] + r[2], r[1], wx1 - r[0] - r[2], r[3]));
        let accent = self.theme.accent;
        let edge = |shell: &mut Vec<Panel>, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: [f32; 3]| {
            for (x, y, w, h) in [(x0, y0, x1 - x0, t), (x0, y1 - t, x1 - x0, t), (x0, y0, t, y1 - y0), (x1 - t, y0, t, y1 - y0)] {
                shell.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 0.0, c, c).flat());
            }
        };
        edge(shell, r[0], r[1], r[0] + r[2], r[1] + r[3], 1.5, accent);
        // the rectangle or the blurred part being dragged out
        let (kx, ky) = e.k();
        match &e.cur {
            Some(Op::Rect { a, b, color, .. }) => {
                let c = [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0];
                edge(shell, r[0] + a[0].min(b[0]) / kx, r[1] + a[1].min(b[1]) / ky, r[0] + a[0].max(b[0]) / kx, r[1] + a[1].max(b[1]) / ky, 2.0, c);
            }
            Some(Op::Blur { a, b }) => {
                let (x0, y0, x1, y1) = (r[0] + a[0].min(b[0]) / kx, r[1] + a[1].min(b[1]) / ky, r[0] + a[0].max(b[0]) / kx, r[1] + a[1].max(b[1]) / ky);
                shell.push(Panel::solid((x0 + x1) / 2.0, (y0 + y1) / 2.0, x1 - x0, y1 - y0, 0.0, [1.0; 3], [1.0; 3]).flat().alpha(0.22));
                edge(shell, x0, y0, x1, y1, 1.5, [1.0; 3]);
            }
            _ => {}
        }
        // the bar
        let Some((frame, btns)) = self.shot_bar() else { return };
        let mut g = self.theme.glass.panel(frame[0] + frame[2] / 2.0, frame[1] + frame[3] / 2.0, frame[2], frame[3], frame[3] / 2.0).tinted([0.08, 0.10, 0.16, 0.72]);
        g.extra[0] = 0.25;
        shell.push(g);
        for (tb, b) in btns {
            let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
            let hover = Self::in_rect(self.mouse, b);
            let on = match tb {
                Tb::Tool(t) => t == e.tool,
                Tb::Color(i) => i == e.color,
                Tb::Width(i) => i == e.width,
                _ => false,
            };
            if on || hover {
                shell.push(Panel::solid(cx, cy, b[2] - 2.0, b[3] - 2.0, (b[2] - 2.0) / 2.0, [1.0; 3], [1.0; 3]).flat().alpha(if on { 0.22 } else { 0.12 }));
            }
            match tb {
                Tb::Tool(t) => shell.push(Panel::glyph(cx, cy, 17.0, match t { Tool::Pen => "pencil", Tool::Rect => "square", Tool::Blur => "droplet" }, [1.0; 3], 0.95)),
                Tb::Color(i) => {
                    let c = COLORS[i];
                    let c = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0];
                    if on {
                        shell.push(Panel::solid(cx, cy, 24.0, 24.0, 12.0, [1.0; 3], [1.0; 3]).flat());
                    }
                    shell.push(Panel::solid(cx, cy, 18.0, 18.0, 9.0, c, c).flat());
                }
                Tb::Width(i) => {
                    let d = 4.0 + WIDTHS[i] * 0.9;
                    shell.push(Panel::solid(cx, cy, d, d, d / 2.0, [1.0; 3], [1.0; 3]).flat().alpha(0.9));
                }
                Tb::Undo => shell.push(Panel::glyph(cx, cy, 17.0, "undo-2", [1.0; 3], if e.ops.is_empty() { 0.35 } else { 0.95 })),
                Tb::Save => shell.push(Panel::glyph(cx, cy, 18.0, "check", [0.55, 0.95, 0.6], 1.0)),
                Tb::Cancel => shell.push(Panel::glyph(cx, cy, 18.0, "x", [1.0, 0.55, 0.55], 1.0)),
            }
        }
    }
}

// ------------------------------------------------------------------------ a window with a see-through background

impl State {
    /// The picture of one window (the front one, or a shell window) over nothing: its own pixels and its shadow, and
    /// see-through everywhere else. Made from two pictures of the window alone, one over black and one over white: what
    /// a pixel shows of the background is how much it differs between them.
    pub fn shot_window_alpha(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        // which window: a shell window in front, else the front client window
        let shell = self.focused_shell();
        let client = if shell.is_some() { None } else { self.wins.iter().rposition(|w| !w.minimized && w.mapped && !w.or) };
        let frame = match (shell, client) {
            (Some(k), _) => self.sw_frame(k),
            (None, Some(i)) => Self::frame_rect(&self.wins[i]),
            _ => return None,
        };
        // the shadow reaches a little beyond the frame
        let m = 48.0;
        let [wx0, wy0, wx1, wy1] = self.world;
        let x0 = (frame[0] - m).max(wx0);
        let y0 = (frame[1] - m).max(wy0);
        let region = [x0, y0, (frame[0] + frame[2] + m).min(wx1) - x0, (frame[1] + frame[3] + m).min(wy1) - y0];

        // show only that window: the others are put aside for the two pictures
        let saved_wins: Vec<Win> = std::mem::take(&mut self.wins);
        let saved_z: Vec<Sw> = std::mem::take(&mut self.z);
        let mut kept: Vec<Win> = Vec::new();
        let mut rest: Vec<(usize, Win)> = Vec::new();
        for (i, w) in saved_wins.into_iter().enumerate() {
            if Some(i) == client {
                kept.push(w);
            } else {
                rest.push((i, w));
            }
        }
        self.wins = kept;
        if let Some(k) = shell {
            self.z = vec![k];
        }
        self.capture_alone = true;
        self.renderer.solid_bg = Some([0.0; 3]);
        let on_black = self.capture_rgba(Some(region));
        self.renderer.solid_bg = Some([1.0; 3]);
        let on_white = self.capture_rgba(Some(region));
        self.renderer.solid_bg = None;
        self.capture_alone = false;
        // everything back as it was
        let mut all = std::mem::take(&mut self.wins);
        for (i, w) in rest {
            let at = i.min(all.len());
            all.insert(at, w);
        }
        self.wins = all;
        self.z = saved_z;
        self.invalidate();

        let (w, h, black) = on_black;
        let (w2, h2, white) = on_white;
        if (w, h) != (w2, h2) {
            return None;
        }
        let mut out = Vec::with_capacity(black.len());
        for (b, wt) in black.chunks_exact(4).zip(white.chunks_exact(4)) {
            // the average over the three colours of how much of the white came through
            let seen: f32 = (0..3).map(|i| (wt[i] as f32 - b[i] as f32).max(0.0)).sum::<f32>() / (3.0 * 255.0);
            let a = (1.0 - seen).clamp(0.0, 1.0);
            if a < 1.0 / 255.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[(b[0] as f32 / a).min(255.0) as u8, (b[1] as f32 / a).min(255.0) as u8, (b[2] as f32 / a).min(255.0) as u8, (a * 255.0).round() as u8]);
            }
        }
        Some((w, h, out))
    }
}
