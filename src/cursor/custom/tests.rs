use super::super::{draw, paint, pieces as shape_pieces, pose_of, preview_image, preview_key, render_with, CursorImage, CursorLook, Fit, Morph, CURSOR_MAX, DESIGN, PREVIEW_SHAPES};
use super::*;
use crate::cursor::ARROWS;

const ACCENT: [f32; 3] = [0.1, 0.5, 1.0];
const ALL: [Shape; 15] = [
    Shape::Default,
    Shape::Copy,
    Shape::Progress,
    Shape::Pointer,
    Shape::Grab,
    Shape::Grabbing,
    Shape::Text,
    Shape::Move,
    Shape::ResizeEw,
    Shape::ResizeNs,
    Shape::ResizeNesw,
    Shape::ResizeNwse,
    Shape::Wait,
    Shape::Crosshair,
    Shape::NotAllowed,
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u32 << 24) as f32
    }
}

/// Every slider alone at each end from every preset, all at one end, random ends and random places.
fn samples<G: Gen>() -> Vec<G> {
    let mut out = Vec::new();
    let mut rng = Rng(0x5EED_1234_9ABC);
    for (_, set) in G::STARTS {
        let mut p = CursorPrefs::default();
        set(&mut p);
        let base = G::from_preset(&p);
        out.push(base.clone());
        for d in G::DIALS {
            for v in [d.lo, d.hi] {
                let mut s = base.clone();
                *(d.get)(&mut s) = v;
                out.push(s);
            }
        }
    }
    for kind in 0..700 {
        let mut s = G::default();
        for d in G::DIALS {
            *(d.get)(&mut s) = match kind {
                0 => d.lo,
                1 => d.hi,
                2..=280 => if rng.next() < 0.5 { d.lo } else { d.hi },
                _ => d.lo + (d.hi - d.lo) * rng.next(),
            };
        }
        out.push(s.clamped());
    }
    out
}

fn with_sample(i: usize, all: &Samples) -> CursorPrefs {
    let mut p = CursorPrefs::default();
    let c = &mut p.custom;
    c.arrow = Made { on: true, shape: all.0[i % all.0.len()].clone(), last: None };
    c.hand = Made { on: true, shape: all.1[i % all.1.len()].clone(), last: None };
    c.beam = Made { on: true, shape: all.2[i % all.2.len()].clone(), last: None };
    c.resize = Made { on: true, shape: all.3[i % all.3.len()].clone(), last: None };
    c.busy = Made { on: true, shape: all.4[i % all.4.len()].clone(), last: None };
    c.signs = Made { on: true, shape: all.5[i % all.5.len()].clone(), last: None };
    p.busy = if i % 3 == 0 { BusyStyle::Hourglass } else { BusyStyle::Ring };
    p
}

type Samples = (Vec<ArrowGen>, Vec<HandGen>, Vec<BeamGen>, Vec<ResizeGen>, Vec<BusyGen>, Vec<SignGen>);

fn all_samples() -> (Samples, usize) {
    let s: Samples = (samples(), samples(), samples(), samples(), samples(), samples());
    let n = [s.0.len(), s.1.len(), s.2.len(), s.3.len(), s.4.len(), s.5.len()].into_iter().max().unwrap();
    (s, n)
}

#[test]
fn every_built_in_arrow_is_a_setting_of_the_sliders() {
    for (style, name) in ARROWS {
        let g = ArrowGen::of(style);
        assert_eq!(g, g.clamped(), "{name}: out of a slider's range");
        let mut got = g.corners(1.0);
        let mut want: Vec<P> = arrow_points(style, 1.0, 0.0, 0.0).into_iter().map(|p| sub(p, (2.0, 2.0))).collect();
        if style == ArrowStyle::Navigator {
            want[1..].reverse();
        }
        got.truncate(8);
        assert_eq!(got.len(), want.len(), "{name}: {got:?}");
        for (a, b) in got.iter().zip(&want) {
            assert!(len(sub(*a, *b)) < 0.002, "{name}: {got:?} instead of {want:?}");
        }
    }
}

/// How different two pictures are with their hot spots on one pixel: the largest difference of alpha, and the mean.
fn difference(a: &CursorImage, b: &CursorImage) -> (u32, f32) {
    let at = |i: &CursorImage, x: i32, y: i32| {
        let (x, y) = (x + i.hot.0, y + i.hot.1);
        if x < 0 || y < 0 || x >= i.size as i32 || y >= i.size as i32 { 0 } else { i.argb[(y * i.size as i32 + x) as usize] >> 24 }
    };
    let (mut most, mut sum, mut seen) = (0, 0, 0);
    for y in -40..40 {
        for x in -40..40 {
            let (p, q) = (at(a, x, y), at(b, x, y));
            if p + q > 0 {
                most = most.max(p.abs_diff(q));
                sum += p.abs_diff(q);
                seen += 1;
            }
        }
    }
    (most, sum as f32 / seen.max(1) as f32)
}

#[test]
fn custom_starts_as_the_preset_looks() {
    let check = |preset: &CursorPrefs, custom: &CursorPrefs, shapes: &[Shape]| {
        for &s in shapes {
            let (a, b) = (render_with(s, 32, preset, ACCENT, 0.25), render_with(s, 32, custom, ACCENT, 0.25));
            let (most, mean) = difference(&a, &b);
            assert!(mean < 3.0, "{s:?} of {:?} {:?} {:?} {:?}: differs by {mean} (at most {most})", preset.arrow, preset.beam, preset.resize, preset.busy);
        }
    };
    // (an outline thin enough that the built-in is cut nowhere, and whole-pixel points)
    let plain = CursorPrefs { outline: 1.0, ..CursorPrefs::default() };
    for (arrow, _) in ARROWS {
        let p = CursorPrefs { arrow, ..plain.clone() };
        let mut c = p.clone();
        c.custom.arrow.choose(&p);
        check(&p, &c, &[Shape::Default]);
    }
    let mut c = plain.clone();
    c.custom.hand.choose(&plain);
    c.custom.signs.choose(&plain);
    c.custom.busy.choose(&plain);
    check(&plain, &c, &[Shape::Pointer, Shape::Grab, Shape::Grabbing, Shape::Crosshair, Shape::NotAllowed, Shape::Wait]);
    for (beam, _) in crate::cursor::BEAMS {
        let p = CursorPrefs { beam, ..plain.clone() };
        let mut c = p.clone();
        c.custom.beam.choose(&p);
        check(&p, &c, &[Shape::Text]);
    }
    for (resize, _) in crate::cursor::RESIZES {
        let p = CursorPrefs { resize, ..plain.clone() };
        let mut c = p.clone();
        c.custom.resize.choose(&p);
        check(&p, &c, &[Shape::ResizeEw, Shape::ResizeNs, Shape::ResizeNesw, Shape::ResizeNwse]);
    }
}

#[test]
fn no_slider_setting_makes_an_outline_cross_itself() {
    for (i, g) in samples::<ArrowGen>().iter().enumerate() {
        let flat = flatten(&g.outline());
        assert!(simple(&flat), "sample {i} crosses itself: {g:?}");
        let area: f32 = (0..flat.len()).map(|j| cross(flat[j], flat[(j + 1) % flat.len()])).sum::<f32>() / 2.0;
        assert!(area.abs() > 1.0, "sample {i} is nothing: {g:?}");
        assert!(g.path(1.0, (2.0, 2.0)).is_some(), "sample {i}");
    }
    for g in samples::<ResizeGen>() {
        let half = g.head_width / 2.0;
        // a head's back stays behind its tip
        assert!(g.head * (1.0 - g.barb) > 0.5 && half > 1.0, "{g:?}");
    }
}

#[test]
fn every_hand_holds_together_with_the_point_on_the_fingertip() {
    for g in samples::<HandGen>() {
        for (base, pointing) in [(&super::super::HAND_POINTER, true), (&super::super::HAND_GRAB, false), (&super::super::HAND_GRABBING, false)] {
            let (b, hot) = g.hand(base, pointing);
            let palm = b[4];
            for (i, f) in b.iter().enumerate().filter(|(i, f)| *i != 4 && f[2] >= 0.5) {
                let apart = f[0] > palm[0] + palm[2] || f[0] + f[2] < palm[0] || f[1] > palm[1] + palm[3] || f[1] + f[3] < palm[1];
                assert!(!apart, "part {i} left the palm: {g:?}");
            }
            if pointing {
                assert_eq!(hot.1, b[0][1]);
                assert!((hot.0 - b[0][0] - b[0][2] / 2.0).abs() <= b[0][2] * 0.07, "off the fingertip: {g:?}");
                assert!(b[0][1] < palm[1], "the index finger is inside the palm: {g:?}");
            }
        }
    }
}

#[test]
fn nothing_is_cut_at_any_slider_setting() {
    let (all, n) = all_samples();
    for i in 0..n {
        let mut p = with_sample(i, &all);
        p.outline = [0.0, 1.7, 3.0][i % 3];
        if i % 5 == 0 {
            p.look = CursorLook::Hollow;
        }
        let reach = p.outline.max(if i % 5 == 0 { 1.8 } else { 0.0 });
        for shape in ALL {
            let (v, hot, turn) = shape_pieces(shape, 0.37, &p);
            for box_px in [12u32, 16, 29, 47, 64] {
                let k = box_px as f32 / DESIGN;
                let fit = Fit::new(&v, hot, k, box_px, CURSOR_MAX, reach, 0.0);
                assert!(fit.size <= CURSOR_MAX && fit.size >= box_px, "{shape:?} {box_px}");
                assert!(fit.hot.0.fract() == 0.0 && fit.hot.1.fract() == 0.0);
                let ks = k * fit.shrink;
                let rot = Transform::from_rotate_at(turn, 16.0, 16.0);
                for piece in &v {
                    let (path, out) = match piece {
                        Piece::Fill(path) | Piece::Accent(path) => (path, 0.0),
                        Piece::Line(path, w) => (path, w / 2.0),
                        Piece::Room(_) => continue,
                    };
                    let b = path.clone().transform(rot).unwrap().bounds();
                    let out = out * ks + reach * k;
                    let (l, r) = (fit.hot.0 + (b.left() - hot.0) * ks - out, fit.hot.0 + (b.right() - hot.0) * ks + out);
                    let (t, d) = (fit.hot.1 + (b.top() - hot.1) * ks - out, fit.hot.1 + (b.bottom() - hot.1) * ks + out);
                    let max = fit.size as f32 + 0.02;
                    assert!(l >= -0.02 && t >= -0.02 && r <= max && d <= max, "{shape:?} sample {i} in {box_px}: {l} {t} {r} {d} of {max}, shrink {}", fit.shrink);
                }
            }
        }
    }
}

#[test]
fn the_drawn_picture_has_the_point_on_the_tip() {
    let (all, n) = all_samples();
    for i in 0..n {
        let mut p = with_sample(i, &all);
        p.shadow = if i % 2 == 0 { 0.0 } else { 0.8 };
        for box_px in [16u32, 29, 64] {
            for shape in [Shape::Default, Shape::Pointer, Shape::Copy, Shape::Progress] {
                let img = render_with(shape, box_px, &p, ACCENT, 0.2);
                assert!(img.size <= 64 && img.argb.len() == (img.size * img.size) as usize);
                let (hx, hy) = img.hot;
                assert!(hx >= 0 && hy >= 0 && hx < img.size as i32 && hy < img.size as i32, "{shape:?} sample {i}: {:?} of {}", img.hot, img.size);
                // the point is the corner of four pixels: the outline round the tip paints at least one of them
                let painted = [(0, 0), (-1, 0), (0, -1), (-1, -1)].iter().any(|(dx, dy)| {
                    let (x, y) = ((hx + dx).max(0), (hy + dy).max(0));
                    img.argb[(y * img.size as i32 + x) as usize] >> 24 > 60
                });
                assert!(painted, "{shape:?} sample {i} in {box_px}: nothing at the point");
            }
        }
    }
}

#[test]
fn a_turning_or_turned_shape_keeps_its_place() {
    let (all, _) = all_samples();
    for i in 0..120 {
        let p = with_sample(i * 5, &all);
        for shape in [Shape::Wait, Shape::Progress] {
            let first = render_with(shape, 29, &p, ACCENT, 0.0);
            for step in 1..16 {
                let img = render_with(shape, 29, &p, ACCENT, step as f32 / 16.0);
                assert_eq!((img.size, img.hot), (first.size, first.hot), "{shape:?} sample {i}, step {step}");
            }
        }
        let at = |s| {
            let img = render_with(s, 29, &p, ACCENT, 0.0);
            (img.size, img.hot)
        };
        for s in [Shape::ResizeNs, Shape::ResizeNesw, Shape::ResizeNwse] {
            assert_eq!(at(s), at(Shape::ResizeEw), "{s:?} sample {i}");
        }
    }
}

#[test]
fn a_morph_between_custom_shapes_begins_and_ends_on_their_pictures() {
    let (all, _) = all_samples();
    for i in 0..100 {
        let mut p = with_sample(i * 7, &all);
        p.busy = if i % 2 == 0 { BusyStyle::Ring } else { BusyStyle::Hourglass };
        p.custom.arrow.on = i % 4 != 1;
        p.custom.busy.on = i % 4 != 2;
        for shape in [Shape::Default, Shape::Copy, Shape::Progress, Shape::Pointer, Shape::Grab, Shape::Grabbing, Shape::ResizeEw, Shape::ResizeNwse] {
            if !is_custom(shape, &p) {
                continue;
            }
            let pose = pose_of(shape, &p).unwrap();
            let (v, hot, turn) = pose.pieces(0.25, &p);
            let posed = paint(Shape::Default, &v, hot, turn, pose.place(&p), 29, CURSOR_MAX, &p, ACCENT, 0.25);
            let own = render_with(shape, 29, &p, ACCENT, 0.25);
            assert_eq!((posed.size, posed.hot), (own.size, own.hot), "{shape:?} sample {i}");
            assert!(posed.argb == own.argb, "{shape:?} sample {i}: the morph's end is another picture");
        }
        let t0 = std::time::Instant::now();
        for (a, b) in [(Shape::Default, Shape::Progress), (Shape::Grab, Shape::Pointer), (Shape::ResizeEw, Shape::ResizeNwse), (Shape::Default, Shape::Copy)] {
            let mut m = Morph::default();
            m.retarget(a, true, &p, t0);
            m.frame(t0, &p, 29, ACCENT, 0.0, || render_with(a, 29, &p, ACCENT, 0.0));
            m.retarget(b, true, &p, t0);
            for step in 1..=10 {
                let t = t0 + std::time::Duration::from_millis(step * 20);
                let img = m.frame(t, &p, 29, ACCENT, 0.0, || render_with(b, 29, &p, ACCENT, 0.0));
                assert!(img.size <= 64 && img.hot.0 >= 0 && img.hot.0 < img.size as i32 && img.hot.1 >= 0 && img.hot.1 < img.size as i32);
            }
            let end = m.frame(t0 + std::time::Duration::from_secs(1), &p, 29, ACCENT, 0.0, || render_with(b, 29, &p, ACCENT, 0.0));
            assert!(*end == render_with(b, 29, &p, ACCENT, 0.0));
        }
    }
}

#[test]
fn a_morph_begins_where_the_hand_and_the_resize_arrow_are() {
    let mut p = CursorPrefs::default();
    p.custom.hand.choose(&p.clone());
    p.custom.resize.choose(&p.clone());
    for (a, b) in [(Shape::Pointer, Shape::Grabbing), (Shape::ResizeEw, Shape::ResizeNs)] {
        let (from, to) = (pose_of(a, &p).unwrap(), pose_of(b, &p).unwrap());
        assert_eq!(from.lerp(&to, 0.0), Some(from));
        assert!(from.lerp(&to, 0.5).is_some());
    }
}

#[test]
fn the_settings_load_and_an_unused_table_is_not_written() {
    let plain = toml::to_string(&CursorPrefs::default()).unwrap();
    assert!(!plain.contains("custom"), "{plain}");
    let mut p = CursorPrefs { arrow: ArrowStyle::Navigator, ..CursorPrefs::default() };
    p.custom.arrow.choose(&p.clone());
    p.custom.arrow.shape.length = 25.5;
    let written = toml::to_string(&p).unwrap();
    assert_eq!(toml::from_str::<CursorPrefs>(&written).unwrap(), p);
    assert!(!written.contains("custom.hand") && !written.contains("custom.beam"), "{written}");
    // what an older build reads: the style it was started from
    assert!(written.contains("arrow = \"navigator\""), "{written}");
    // a table with one number in it, and numbers out of range
    let part: CursorPrefs = toml::from_str("[custom.arrow]\non = true\n[custom.arrow.shape]\nlength = 900.0\nwidth = nan\n").unwrap();
    let s = part.custom.arrow.shape.clamped();
    assert_eq!((s.length, s.width), (30.0, ArrowGen::default().width));
    assert!(render_with(Shape::Default, 28, &part, ACCENT, 0.0).size <= 64);
    // and through the whole input section, as the config saves it
    let mut input = crate::input::InputPrefs::default();
    input.cursor = p.clone();
    let back: crate::input::InputPrefs = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
    assert_eq!(back.cursor, p);
}

#[test]
fn choosing_custom_keeps_what_was_built_before() {
    let mut p = CursorPrefs::default();
    p.custom.arrow.choose(&p.clone());
    assert_eq!(p.custom.arrow.last, None);
    p.custom.arrow.shape.length = 27.0;
    let built = p.custom.arrow.shape.clone();
    p.custom.arrow.on = false;
    p.arrow = ArrowStyle::Sharp;
    p.custom.arrow.choose(&p.clone());
    assert_eq!(p.custom.arrow.shape, ArrowGen::of(ArrowStyle::Sharp));
    assert_eq!(p.custom.arrow.last, Some(built.clone()));
    p.custom.arrow.start(built.clone());
    assert_eq!(p.custom.arrow.shape, built);
    for seed in 0..200u64 {
        let r = ArrowGen::random(&p, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        assert_eq!(r, r.clamped());
    }
}

#[test]
fn a_preview_changes_only_with_what_it_shows_and_has_one_cell() {
    let mut p = CursorPrefs::default();
    p.custom.arrow.choose(&p.clone());
    let key = |p: &CursorPrefs, s: Shape| preview_key("arrow-big", s, p, ACCENT);
    let (arrow, beam) = (key(&p, Shape::Default), key(&p, Shape::Text));
    let mut places = std::collections::HashSet::new();
    let mut keys = std::collections::HashSet::new();
    for step in 0..500 {
        let mut q = p.clone();
        q.custom.arrow.shape.length = 8.0 + step as f32 * 0.04;
        q.size = 20.0 + (step % 30) as f32;
        let k = key(&q, Shape::Default);
        places.insert(crate::cursor::preview_slot(&k).to_string());
        assert_eq!(key(&q, Shape::Text), beam, "another shape's preview changed with the arrow");
        keys.insert(k);
    }
    assert_eq!(places.into_iter().collect::<Vec<_>>(), ["cursor:arrow-big"]);
    assert!(keys.len() > 400 && preview_image(&arrow).is_some());
    for s in PREVIEW_SHAPES.into_iter().chain([Shape::Copy, Shape::Grabbing]) {
        let (img, _) = preview_image(&key(&p, s)).unwrap_or_else(|| panic!("no preview of {s:?}"));
        assert_eq!(img.dimensions(), (128, 128));
    }
}

/// `CURSOR_CUSTOM_SHEET=/path.png cargo test --release custom_sheet -- --ignored`: presets as Custom, each slider's
/// two ends, and random ones, to look at.
#[test]
#[ignore]
fn custom_sheet() {
    let Ok(path) = std::env::var("CURSOR_CUSTOM_SHEET") else { return };
    const CELL: u32 = 70;
    const COLS: u32 = 28;
    let mut cells: Vec<CursorImage> = Vec::new();
    let add = |cells: &mut Vec<CursorImage>, p: &CursorPrefs, shapes: &[Shape]| cells.extend(shapes.iter().map(|s| draw(*s, 40, 68, p, ACCENT, 0.3)));
    fn row_end(cells: &mut Vec<CursorImage>) {
        while cells.len() % COLS as usize != 0 {
            cells.push(CursorImage { size: 1, hot: (0, 0), argb: vec![0] });
        }
    }
    for (arrow, _) in ARROWS {
        let mut p = CursorPrefs { arrow, ..CursorPrefs::default() };
        add(&mut cells, &p, &[Shape::Default]);
        p.custom.arrow.choose(&p.clone());
        add(&mut cells, &p, &[Shape::Default]);
    }
    row_end(&mut cells);
    fn ends<G: Gen>(cells: &mut Vec<CursorImage>, get: fn(&mut CursorPrefs) -> &mut Made<G>, shape: Shape) {
        for d in G::DIALS {
            for v in [d.lo, d.hi] {
                let mut p = CursorPrefs::default();
                get(&mut p).on = true;
                *(d.get)(&mut get(&mut p).shape) = v;
                cells.push(draw(shape, 40, 68, &p, ACCENT, 0.3));
            }
        }
        row_end(cells);
    }
    ends::<ArrowGen>(&mut cells, |p| &mut p.custom.arrow, Shape::Default);
    ends::<HandGen>(&mut cells, |p| &mut p.custom.hand, Shape::Pointer);
    ends::<HandGen>(&mut cells, |p| &mut p.custom.hand, Shape::Grab);
    ends::<BeamGen>(&mut cells, |p| &mut p.custom.beam, Shape::Text);
    ends::<ResizeGen>(&mut cells, |p| &mut p.custom.resize, Shape::ResizeNwse);
    ends::<ResizeGen>(&mut cells, |p| &mut p.custom.resize, Shape::Move);
    ends::<BusyGen>(&mut cells, |p| &mut p.custom.busy, Shape::Wait);
    ends::<SignGen>(&mut cells, |p| &mut p.custom.signs, Shape::Crosshair);
    for seed in 1..=8u64 {
        let seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut p = CursorPrefs::default();
        let base = p.clone();
        p.custom.arrow = Made { on: true, shape: ArrowGen::random(&base, seed), last: None };
        p.custom.hand = Made { on: true, shape: HandGen::random(&base, seed + 1), last: None };
        p.custom.beam = Made { on: true, shape: BeamGen::random(&base, seed + 2), last: None };
        p.custom.resize = Made { on: true, shape: ResizeGen::random(&base, seed + 3), last: None };
        p.custom.busy = Made { on: true, shape: BusyGen::random(&base, seed + 4), last: None };
        p.custom.signs = Made { on: true, shape: SignGen::random(&base, seed + 5), last: None };
        add(&mut cells, &p, &ALL[..14]);
    }
    let rows = (cells.len() as u32).div_ceil(COLS);
    let mut img = image::RgbaImage::from_pixel(CELL * COLS, CELL * rows, image::Rgba([122, 130, 146, 255]));
    for (i, c) in cells.iter().enumerate() {
        let (ox, oy) = (i as u32 % COLS * CELL, i as u32 / COLS * CELL);
        for (j, p) in c.argb.iter().enumerate() {
            let (x, y) = (ox + j as u32 % c.size, oy + j as u32 / c.size);
            let (old, keep) = (img.get_pixel(x, y).0, 1.0 - (p >> 24) as f32 / 255.0);
            let over = |sh: u32, o: u8| (((p >> sh) & 255) as f32 + o as f32 * keep).min(255.0) as u8;
            img.put_pixel(x, y, image::Rgba([over(16, old[0]), over(8, old[1]), over(0, old[2]), 255]));
        }
    }
    img.save(path).unwrap();
}
