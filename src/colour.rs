//! Colour helpers shared by the icon tools: HSL / HSV conversions and hex codes.

/// Hue in degrees, saturation and lightness 0..1 to RGB 0..1.
pub fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// RGB 0..1 to hue (degrees), saturation and value 0..1.
pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 1e-6 {
        0.0
    } else if mx == c[0] {
        60.0 * (((c[1] - c[2]) / d).rem_euclid(6.0))
    } else if mx == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

/// Hue (degrees), saturation and value 0..1 to RGB 0..1.
pub fn hsv_to_rgb(h: [f32; 3]) -> [f32; 3] {
    let c = h[2] * h[1];
    let hp = h[0].rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = h[2] - c;
    [r + m, g + m, b + m]
}

/// "#rrggbb", "rrggbb" or "#rgb" as RGB 0..1.
pub fn parse_hex(text: &str) -> Option<[f32; 3]> {
    let t = text.trim().trim_start_matches('#');
    let v = match t.len() {
        3 => t.chars().map(|c| c.to_digit(16).map(|d| d * 17)).collect::<Option<Vec<u32>>>()?,
        6 => (0..3).map(|i| u32::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()).collect::<Option<Vec<u32>>>()?,
        _ => return None,
    };
    Some([v[0] as f32 / 255.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0])
}

pub fn to_hex(c: [f32; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", (c[0].clamp(0.0, 1.0) * 255.0).round() as u8, (c[1].clamp(0.0, 1.0) * 255.0).round() as u8, (c[2].clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip_through_hsv_and_hex() {
        for c in [[0.8, 0.2, 0.1], [0.1, 0.9, 0.4], [0.3, 0.3, 0.95], [0.5, 0.5, 0.5]] {
            let back = hsv_to_rgb(rgb_to_hsv(c));
            assert!((0..3).all(|i| (back[i] - c[i]).abs() < 1e-4), "{c:?} -> {back:?}");
        }
        assert_eq!(to_hex([1.0, 0.5, 0.0]), "#ff8000");
        assert_eq!(parse_hex("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0]));
        assert_eq!(parse_hex("f00"), Some([1.0, 0.0, 0.0]));
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(hsl(0.0, 1.0, 0.5), [1.0, 0.0, 0.0]);
    }
}
