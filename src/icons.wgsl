// Procedural Apple-style app icons, painted with SDFs in a normalised [-1,1] square (y down).
// The squircle outline and the background gradient come from the panel; this paints the glyph.

fn cov(d: f32, aa: f32) -> f32 {
    return clamp(0.5 - d / aa, 0.0, 1.0);
}

fn rot(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(p) - b;
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0);
}

fn sd_rbox(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    return sd_box(p, b - vec2(r)) - r;
}

fn sd_seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_ellipse(p: vec2<f32>, r: vec2<f32>) -> f32 {
    return (length(p / r) - 1.0) * min(r.x, r.y);
}

fn sd_tri(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let q0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let q1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let q2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(vec2(dot(q0, q0), s * (v0.x * e0.y - v0.y * e0.x)), vec2(dot(q1, q1), s * (v1.x * e1.y - v1.y * e1.x))),
        vec2(dot(q2, q2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

fn icon_paint(id: u32, p: vec2<f32>, aa: f32, base: vec3<f32>) -> vec3<f32> {
    var c = base;
    switch id {
        // ---- Finder: two-tone face
        case 0u: {
            let lh = vec3(0.60, 0.87, 1.0);
            let rh = vec3(0.08, 0.44, 0.97);
            c = mix(lh, rh, smoothstep(-0.015, 0.015, p.x));
            c += vec3(0.10) * (1.0 - (p.y * 0.5 + 0.5)) * 0.6;
            let navy = vec3(0.07, 0.11, 0.30);
            c = mix(c, navy, cov(sd_rbox(p - vec2(-0.40, -0.20), vec2(0.045, 0.17), 0.045), aa));
            c = mix(c, navy, cov(sd_rbox(p - vec2(0.36, -0.20), vec2(0.045, 0.17), 0.045), aa));
            c = mix(c, navy, cov(sd_seg(p, vec2(0.02, -0.22), vec2(-0.06, 0.12)) - 0.022, aa));
            let ring = abs(length(p - vec2(0.0, -0.04)) - 0.62) - 0.038;
            c = mix(c, navy, cov(max(ring, sd_box(p - vec2(0.0, 0.46), vec2(0.5, 0.2))), aa));
        }
        // ---- Safari: compass
        case 1u: {
            let r = length(p);
            let ang = atan2(p.y, p.x);
            let disc = r - 0.74;
            let dc = mix(vec3(0.36, 0.80, 1.0), vec3(0.0, 0.40, 0.94), p.y * 0.5 + 0.5);
            c = mix(c, dc, cov(disc, aa));
            let step = 6.2831853 / 36.0;
            let fa = (fract(ang / step + 0.5) - 0.5) * step * r;
            let tick = max(abs(fa) - 0.007, abs(r - 0.66) - 0.04);
            c = mix(c, vec3(1.0), cov(tick, aa) * 0.9 * cov(disc, aa));
            let q = rot(p, 0.785398);
            let hw = 0.13 * (1.0 - abs(q.x) / 0.52);
            let nd = max(abs(q.y) - hw, abs(q.x) - 0.52);
            let nc = select(vec3(0.97), vec3(1.0, 0.26, 0.25), q.x > 0.0);
            c = mix(c, nc, cov(nd, aa));
        }
        // ---- Messages: speech bubble
        case 2u: {
            let bub = min(sd_ellipse(p - vec2(0.0, -0.10), vec2(0.68, 0.50)), sd_tri(p, vec2(-0.46, 0.22), vec2(-0.66, 0.64), vec2(-0.08, 0.38)));
            c = mix(c, vec3(1.0), cov(bub, aa));
        }
        // ---- Mail: envelope
        case 3u: {
            let env = sd_rbox(p - vec2(0.0, 0.04), vec2(0.66, 0.46), 0.10);
            c = mix(c, vec3(1.0), cov(env, aa));
            let f1 = max(sd_seg(p, vec2(-0.62, -0.36), vec2(0.0, 0.14)) - 0.034, env + 0.05);
            let f2 = max(sd_seg(p, vec2(0.62, -0.36), vec2(0.0, 0.14)) - 0.034, env + 0.05);
            c = mix(c, vec3(0.50, 0.72, 1.0), cov(min(f1, f2), aa));
        }
        // ---- Calendar: red header + digit "2"
        case 4u: {
            c = mix(c, vec3(1.0, 0.27, 0.25), cov(sd_rbox(p - vec2(0.0, -0.58), vec2(0.22, 0.05), 0.05), aa));
            let q = p - vec2(0.0, 0.16);
            let ink = vec3(0.10, 0.10, 0.12);
            // "2": top arc, diagonal stroke, base bar
            var d = max(abs(length(q - vec2(0.0, -0.22)) - 0.20) - 0.05, q.y + 0.12);
            d = min(d, sd_seg(q, vec2(0.19, -0.16), vec2(-0.21, 0.38)) - 0.05);
            d = min(d, sd_rbox(q - vec2(0.0, 0.40), vec2(0.25, 0.05), 0.05));
            c = mix(c, ink, cov(d, aa));
        }
        // ---- Settings: gear
        case 5u: {
            let r = length(p);
            let step = 0.785398;
            let a = atan2(p.y, p.x);
            let fa = (fract(a / step + 0.5) - 0.5) * step;
            let qq = vec2(cos(fa), sin(fa)) * r;
            let gear = min(r - 0.46, sd_rbox(qq - vec2(0.52, 0.0), vec2(0.13, 0.10), 0.03));
            c = mix(c, vec3(0.95, 0.96, 0.98), cov(gear, aa));
            c = mix(c, vec3(0.50, 0.52, 0.58), cov(r - 0.20, aa));
        }
        // ---- Music: note
        case 6u: {
            var d = length((p - vec2(-0.24, 0.32)) / vec2(1.0, 0.78)) - 0.16;
            d = min(d, length((p - vec2(0.28, 0.22)) / vec2(1.0, 0.78)) - 0.16);
            d = min(d, sd_box(p - vec2(-0.095, -0.06), vec2(0.035, 0.40)));
            d = min(d, sd_box(p - vec2(0.405, -0.16), vec2(0.035, 0.38)));
            d = min(d, sd_seg(p, vec2(-0.095, -0.44), vec2(0.405, -0.54)) - 0.06);
            c = mix(c, vec3(1.0), cov(d, aa));
        }
        // ---- generic application: a small window
        case 8u: {
            let win = sd_rbox(p - vec2(0.0, 0.04), vec2(0.62, 0.50), 0.12);
            c = mix(c, vec3(1.0), cov(win, aa) * 0.95);
            c = mix(c, vec3(0.55, 0.62, 0.80), cov(sd_rbox(p - vec2(0.0, -0.30), vec2(0.62, 0.12), 0.0) , aa) * cov(win, aa));
            for (var i = 0; i < 3; i = i + 1) {
                c = mix(c, vec3(1.0), cov(length(p - vec2(-0.44 + f32(i) * 0.17, -0.30)) - 0.05, aa) * 0.9);
            }
        }
        // ---- Photos: rainbow petals
        default: {
            var pet = array<vec3<f32>, 8>(
                vec3(1.0, 0.30, 0.28), vec3(1.0, 0.58, 0.18), vec3(1.0, 0.82, 0.20), vec3(0.45, 0.85, 0.30),
                vec3(0.20, 0.78, 0.62), vec3(0.20, 0.55, 1.0), vec3(0.58, 0.38, 0.95), vec3(0.95, 0.35, 0.65),
            );
            for (var i = 0; i < 8; i = i + 1) {
                let a = f32(i) * 0.785398 - 1.5708;
                let q = rot(p - vec2(cos(a), sin(a)) * 0.27, -a);
                let d = (length(q / vec2(0.34, 0.17)) - 1.0) * 0.17;
                c = mix(c, pet[i], cov(d, aa) * 0.8);
            }
        }
    }
    return c;
}
