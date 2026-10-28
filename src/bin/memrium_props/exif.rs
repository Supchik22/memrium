//! A small EXIF reader: JPEG (APP1), TIFF and WebP carry the same TIFF-structured block of camera data.

struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(o..o + 2)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The entries of the directory at `off`: (tag, type, count, where the value starts).
    fn ifd(&self, off: usize) -> Vec<(u16, u16, usize, usize)> {
        let mut v = Vec::new();
        let Some(n) = self.u16(off) else { return v };
        for i in 0..n as usize {
            let e = off + 2 + i * 12;
            let (Some(tag), Some(ty), Some(count)) = (self.u16(e), self.u16(e + 2), self.u32(e + 4)) else { break };
            let size = match ty {
                1 | 2 | 7 => 1,
                3 => 2,
                4 | 9 => 4,
                5 | 10 => 8,
                _ => 1,
            } * count as usize;
            let at = if size <= 4 { e + 8 } else { self.u32(e + 8).unwrap_or(0) as usize };
            v.push((tag, ty, count as usize, at));
        }
        v
    }

    fn text(&self, ty: u16, count: usize, at: usize) -> Option<String> {
        if ty != 2 && ty != 7 {
            return None;
        }
        let b = self.d.get(at..at + count)?;
        let s = String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        (!s.is_empty()).then_some(s)
    }

    fn num(&self, ty: u16, at: usize, i: usize) -> Option<f64> {
        match ty {
            3 => self.u16(at + i * 2).map(|v| v as f64),
            4 => self.u32(at + i * 4).map(|v| v as f64),
            9 => self.u32(at + i * 4).map(|v| v as i32 as f64),
            5 | 10 => {
                let (a, b) = (self.u32(at + i * 8)?, self.u32(at + i * 8 + 4)?);
                if b == 0 {
                    return None;
                }
                Some(if ty == 10 { a as i32 as f64 / b as i32 as f64 } else { a as f64 / b as f64 })
            }
            _ => None,
        }
    }
}

fn find_tiff(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(&[0xFF, 0xD8]) {
        let mut p = 2;
        while p + 4 <= data.len() && data[p] == 0xFF {
            let marker = data[p + 1];
            if marker == 0xDA {
                break;
            }
            let len = u16::from_be_bytes([data[p + 2], data[p + 3]]) as usize;
            let body = data.get(p + 4..p + 2 + len)?;
            if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
                return Some(&body[6..]);
            }
            p += 2 + len;
        }
        None
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        Some(data)
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        let mut p = 12;
        while p + 8 <= data.len() {
            let len = u32::from_le_bytes(data[p + 4..p + 8].try_into().ok()?) as usize;
            if &data[p..p + 4] == b"EXIF" {
                let body = data.get(p + 8..p + 8 + len)?;
                return Some(body.strip_prefix(b"Exif\0\0").unwrap_or(body));
            }
            p += 8 + len + (len & 1);
        }
        None
    } else {
        None
    }
}

fn shutter(s: f64) -> String {
    if s >= 1.0 {
        format!("{s:.1} s")
    } else if s > 0.0 {
        format!("1/{} s", (1.0 / s).round() as u32)
    } else {
        String::new()
    }
}

fn degrees(t: &Tiff, ty: u16, at: usize, reference: &str) -> Option<String> {
    let (d, m, s) = (t.num(ty, at, 0)?, t.num(ty, at, 1)?, t.num(ty, at, 2)?);
    Some(format!("{}° {}′ {:.1}″ {reference}", d as u32, m as u32, s))
}

/// What the picture's camera recorded, as (label, value) rows; empty when there is no EXIF block.
pub fn read(data: &[u8]) -> Vec<(String, String)> {
    let Some(block) = find_tiff(data) else { return Vec::new() };
    let le = match block.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Vec::new(),
    };
    let t = Tiff { d: block, le };
    let Some(first) = t.u32(4) else { return Vec::new() };
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut exif_ptr = None;
    let mut gps_ptr = None;
    let mut make = None;
    let mut model = None;
    for (tag, ty, count, at) in t.ifd(first as usize) {
        match tag {
            0x010F => make = t.text(ty, count, at),
            0x0110 => model = t.text(ty, count, at),
            0x0131 => rows.extend(t.text(ty, count, at).map(|v| ("Software".to_string(), v))),
            0x0132 => rows.extend(t.text(ty, count, at).map(|v| ("Date".to_string(), v))),
            0x013B => rows.extend(t.text(ty, count, at).map(|v| ("Artist".to_string(), v))),
            0x8298 => rows.extend(t.text(ty, count, at).map(|v| ("Copyright".to_string(), v))),
            0x0112 => {
                if let Some(o) = t.num(ty, at, 0) {
                    let name = ["Normal", "Mirrored", "Rotated 180°", "Mirrored vertically", "Mirrored, rotated 90°", "Rotated 90°", "Mirrored, rotated 270°", "Rotated 270°"];
                    rows.push(("Orientation".into(), name.get((o as usize).wrapping_sub(1)).copied().unwrap_or("Unknown").into()));
                }
            }
            0x8769 => exif_ptr = t.u32(at),
            0x8825 => gps_ptr = t.u32(at),
            _ => {}
        }
    }
    let camera = match (make, model) {
        (Some(a), Some(b)) if b.starts_with(&a) => Some(b),
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    };
    if let Some(c) = camera {
        rows.insert(0, ("Camera".into(), c));
    }
    if let Some(p) = exif_ptr {
        let mut flash = None;
        for (tag, ty, count, at) in t.ifd(p as usize) {
            let n = |i| t.num(ty, at, i);
            let label_row = |l: &str, v: String| (l.to_string(), v);
            match tag {
                0xA434 => rows.extend(t.text(ty, count, at).map(|v| label_row("Lens", v))),
                0x9003 => rows.extend(t.text(ty, count, at).map(|v| label_row("Taken", v))),
                0x829A => rows.extend(n(0).map(|v| label_row("Exposure", shutter(v)))),
                0x829D => rows.extend(n(0).map(|v| label_row("Aperture", format!("f/{v:.1}")))),
                0x8827 => rows.extend(n(0).map(|v| label_row("ISO", format!("{}", v as u32)))),
                0x920A => rows.extend(n(0).map(|v| label_row("Focal length", format!("{v:.0} mm")))),
                0xA405 => rows.extend(n(0).filter(|v| *v > 0.0).map(|v| label_row("35 mm equivalent", format!("{v:.0} mm")))),
                0x9204 => rows.extend(n(0).map(|v| label_row("Exposure bias", format!("{v:+.1} EV")))),
                0x9209 => flash = n(0).map(|v| v as u32 & 1 == 1),
                _ => {}
            }
        }
        if let Some(f) = flash {
            rows.push(("Flash".into(), if f { "Fired" } else { "Did not fire" }.into()));
        }
    }
    if let Some(p) = gps_ptr {
        let (mut lat, mut lon, mut lat_ref, mut lon_ref, mut alt) = (None, None, String::new(), String::new(), None);
        for (tag, ty, count, at) in t.ifd(p as usize) {
            match tag {
                1 => lat_ref = t.text(ty, count, at).unwrap_or_default(),
                2 => lat = Some((ty, at)),
                3 => lon_ref = t.text(ty, count, at).unwrap_or_default(),
                4 => lon = Some((ty, at)),
                6 => alt = t.num(ty, at, 0),
                _ => {}
            }
        }
        if let (Some((ty1, a1)), Some((ty2, a2))) = (lat, lon) {
            if let (Some(a), Some(b)) = (degrees(&t, ty1, a1, &lat_ref), degrees(&t, ty2, a2, &lon_ref)) {
                rows.push(("Location".into(), format!("{a}, {b}")));
            }
        }
        if let Some(a) = alt {
            rows.push(("Altitude".into(), format!("{a:.0} m")));
        }
    }
    rows
}
