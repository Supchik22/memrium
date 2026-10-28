// The icon renderer on the graphics card: every step of `composite` in iconrender.rs as a compute pass, doing the same sums
// pixel by pixel. All the pictures of one render are planes (premultiplied r, g, b, a) of one buffer; a pass reads and writes
// the planes its parameters name.

struct Params {
    n: u32,
    // the side of the raster `place` reads
    m: u32,
    dst: u32,
    src: u32,
    src2: u32,
    src3: u32,
    src4: u32,
    // where the raster and the mask are in `data` (in u32 words)
    data_off: u32,
    mask_off: u32,
    has_mask: u32,
    // the first of the two peaks of this layer: the slope of the glass, the slope of the bevel
    pk: u32,
    blend: u32,
    // blur: the radius of the box; bevel: bit 0 the bevel, bit 1 the inner shade; shadow: chromatic
    r: u32,
    flag: u32,
    dy: u32,
    at_x: i32,
    at_y: i32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    opacity: f32,
    see_through: f32,
    glow: f32,
    shadow_amount: f32,
    bevel: f32,
    inner_shadow: f32,
    specular: f32,
    bend_out: f32,
    bend_in: f32,
    dispersion: f32,
    lx: f32,
    ly: f32,
    sharp01: f32,
    edge_line: f32,
    soft_light: f32,
    rim: f32,
    sheen: f32,
    scale: f32,
    off_x: f32,
    off_y: f32,
    top_r: f32,
    top_g: f32,
    top_b: f32,
    plate_top: f32,
    bot_r: f32,
    bot_g: f32,
    bot_b: f32,
    plate_span: f32,
    fade: f32,
    edge_sharp: f32,
    _f1: f32,
    _f2: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> planes: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> peaks: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read> data: array<u32>;
@group(0) @binding(4) var<storage, read_write> outp: array<u32>;

fn at(k: u32, i: u32) -> u32 {
    return k * p.n * p.n + i;
}

fn mask(i: u32) -> f32 {
    if p.has_mask == 0u {
        return 1.0;
    }
    return bitcast<f32>(data[p.mask_off + i]);
}

fn ease(t: f32) -> f32 {
    let u = clamp(t, 0.0, 1.0);
    return u * u * (3.0 - 2.0 * u);
}

// (pow of 0 is not defined in WGSL)
fn spow(x: f32, y: f32) -> f32 {
    return select(0.0, pow(x, y), x > 0.0);
}

// Buf::sample: bilinear, clamped to the picture
fn bilinear(k: u32, xf: f32, yf: f32) -> vec4<f32> {
    let n = p.n;
    let nf = f32(n);
    let x = clamp(xf, 0.0, nf - 1.001);
    let y = clamp(yf, 0.0, nf - 1.001);
    let x0 = u32(x);
    let y0 = u32(y);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let b = k * n * n;
    return planes[b + y0 * n + x0] * (1.0 - fx) * (1.0 - fy) + planes[b + y0 * n + x0 + 1u] * fx * (1.0 - fy) + planes[b + (y0 + 1u) * n + x0] * (1.0 - fx) * fy + planes[b + (y0 + 1u) * n + x0 + 1u] * fx * fy;
}

fn blend_channel(mode: u32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case 1u: { return cb * cs; }
        case 2u: { return cb + cs - cb * cs; }
        case 3u: {
            if cb <= 0.5 {
                return 2.0 * cb * cs;
            }
            return 1.0 - 2.0 * (1.0 - cb) * (1.0 - cs);
        }
        case 4u: { return min(cb, cs); }
        case 5u: { return max(cb, cs); }
        case 6u: { return min(cb + cs, 1.0); }
        default: { return cs; }
    }
}

// `put` of iconrender.rs: colour `cs` of alpha `a` over pixel `i` of plane `k`
fn put(k: u32, i: u32, cs: vec3<f32>, a: f32, mode: u32) {
    if a <= 0.0 {
        return;
    }
    let j = at(k, i);
    let d = planes[j];
    let ab = d.w;
    var o = d;
    for (var c = 0u; c < 3u; c++) {
        let pm = d[c];
        let cb = select(0.0, clamp(pm / ab, 0.0, 1.0), ab > 0.0);
        o[c] = (1.0 - ab) * a * cs[c] + (1.0 - a) * pm + ab * a * blend_channel(mode, cb, cs[c]);
    }
    o.w = a + ab - a * ab;
    planes[j] = o;
}

fn peak(k: u32, least: f32) -> f32 {
    return max(bitcast<f32>(atomicLoad(&peaks[k])), least);
}

@compute @workgroup_size(8, 8)
fn zero(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    planes[at(p.dst, g.y * p.n + g.x)] = vec4<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn copy(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    planes[at(p.dst, i)] = planes[at(p.src, i)];
}

// place_at: the raster (straight alpha, m x m, in `data`) premultiplied, with its top left corner at (at_x, at_y)
@compute @workgroup_size(8, 8)
fn place(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let sx = i32(g.x) - p.at_x;
    let sy = i32(g.y) - p.at_y;
    let m = i32(p.m);
    var v = vec4<f32>(0.0);
    if sx >= 0 && sy >= 0 && sx < m && sy < m {
        let c = unpack4x8unorm(data[p.data_off + u32(sy * m + sx)]);
        v = vec4<f32>(c.rgb * c.a, c.a);
    }
    planes[at(p.dst, i)] = v;
}

// the plate: a gradient from top to bottom, cut by the mask
@compute @workgroup_size(8, 8)
fn plate(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let t = clamp((f32(g.y) + 0.5 - p.plate_top) / p.plate_span, 0.0, 1.0);
    let c = mix(vec3<f32>(p.top_r, p.top_g, p.top_b), vec3<f32>(p.bot_r, p.bot_g, p.bot_b), t);
    let a = mask(i);
    planes[at(p.dst, i)] = vec4<f32>(c * a, a);
}

// transform: scaled by `scale` about the centre and moved by (off_x, off_y)
@compute @workgroup_size(8, 8)
fn transform(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let nf = f32(p.n);
    let c = nf / 2.0;
    let s = max(p.scale, 0.01);
    let sx = (f32(g.x) + 0.5 - c - p.off_x) / s + c - 0.5;
    let sy = (f32(g.y) + 0.5 - c - p.off_y) / s + c - 0.5;
    var v = vec4<f32>(0.0);
    if !(sx < -1.0 || sy < -1.0 || sx > nf || sy > nf) {
        v = bilinear(p.src, sx, sy);
    }
    planes[at(p.dst, i)] = v;
}

@compute @workgroup_size(8, 8)
fn mask_mul(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    planes[at(p.dst, i)] *= mask(i);
}

// one box pass of `blur` along the rows: the mean over the box, what lies outside the picture counting as nothing. The result
// is written turned over (rows become columns), so the next pass, the same again, goes down the columns and turns it back.
// A workgroup reads its piece of the row once into `tile`, with the box's reach on either side.
const TILE_REACH: u32 = 256u;
var<workgroup> tile: array<vec4<f32>, 576>;

@compute @workgroup_size(64, 1, 1)
fn blur(@builtin(local_invocation_id) l: vec3<u32>, @builtin(workgroup_id) w: vec3<u32>) {
    let n = p.n;
    let r = p.r;
    let y = w.y;
    let x0 = w.x * 64u;
    let row = p.src * n * n + y * n;
    if r <= TILE_REACH {
        for (var k = l.x; k < 64u + 2u * r; k += 64u) {
            let xi = i32(x0 + k) - i32(r);
            var v = vec4<f32>(0.0);
            if xi >= 0 && xi < i32(n) {
                v = planes[row + u32(xi)];
            }
            tile[k] = v;
        }
    }
    workgroupBarrier();
    let x = x0 + l.x;
    if x >= n || y >= n { return; }
    var sum = vec4<f32>(0.0);
    if r <= TILE_REACH {
        for (var j = 0u; j <= 2u * r; j++) {
            sum += tile[l.x + j];
        }
    } else {
        let lo = max(x, r) - r;
        let hi = min(x + r, n - 1u);
        for (var j = lo; j <= hi; j++) {
            sum += planes[row + j];
        }
    }
    planes[at(p.dst, x * n + y)] = max(sum / f32(2u * r + 1u), vec4<f32>(0.0));
}

// the whole `blur` across the rows in one pass, for pictures of up to 512 pixels: a workgroup takes one row into `row_a`
// and runs the three box passes there (the passes across and down commute, so all three across come first), then writes the
// row turned over as a column; the same again goes down the columns.
var<workgroup> row_a: array<vec4<f32>, 512>;
var<workgroup> row_b: array<vec4<f32>, 512>;

@compute @workgroup_size(256, 1, 1)
fn blur_row(@builtin(local_invocation_id) l: vec3<u32>, @builtin(workgroup_id) w: vec3<u32>) {
    let n = p.n;
    let r = p.r;
    let y = w.x;
    let norm = 1.0 / f32(2u * r + 1u);
    let row = p.src * n * n + y * n;
    for (var x = l.x; x < n; x += 256u) {
        row_a[x] = planes[row + x];
    }
    workgroupBarrier();
    for (var pass_ = 0u; pass_ < 3u; pass_++) {
        for (var x = l.x; x < n; x += 256u) {
            let lo = max(x, r) - r;
            let hi = min(x + r, n - 1u);
            var sum = vec4<f32>(0.0);
            if pass_ == 1u {
                for (var j = lo; j <= hi; j++) {
                    sum += row_b[j];
                }
                row_a[x] = max(sum * norm, vec4<f32>(0.0));
            } else {
                for (var j = lo; j <= hi; j++) {
                    sum += row_a[j];
                }
                row_b[x] = max(sum * norm, vec4<f32>(0.0));
            }
        }
        workgroupBarrier();
    }
    for (var x = l.x; x < n; x += 256u) {
        planes[at(p.dst, x * n + y)] = row_b[x];
    }
}

// the slope of the blurred shape (in .w of src): which way the edge faces and how steep the glass is there
@compute @workgroup_size(8, 8)
fn hill(@builtin(global_invocation_id) g: vec3<u32>) {
    let n = p.n;
    if g.x >= n || g.y >= n { return; }
    let b = p.src * n * n;
    let x = g.x;
    let y = g.y;
    let gx = planes[b + y * n + min(x + 1u, n - 1u)].w - planes[b + y * n + max(x, 1u) - 1u].w;
    let gy = planes[b + min(y + 1u, n - 1u) * n + x].w - planes[b + (max(y, 1u) - 1u) * n + x].w;
    let l = sqrt(gx * gx + gy * gy);
    var nrm = vec2<f32>(0.0, -1.0);
    if l > 1e-7 {
        nrm = vec2<f32>(-gx / l, -gy / l);
    }
    planes[at(p.dst, y * n + x)] = vec4<f32>(nrm, l, 0.0);
    if l > 0.0 {
        atomicMax(&peaks[p.pk], bitcast<u32>(l));
    }
}

// the slope of the rounded edge of a bevel
@compute @workgroup_size(8, 8)
fn bevel_slope(@builtin(global_invocation_id) g: vec3<u32>) {
    let n = p.n;
    if g.x >= n || g.y >= n { return; }
    let b = p.src * n * n;
    let x = g.x;
    let y = g.y;
    let gx = (planes[b + y * n + min(x + 1u, n - 1u)].w - planes[b + y * n + max(x, 1u) - 1u].w) * 0.5;
    let gy = (planes[b + min(y + 1u, n - 1u) * n + x].w - planes[b + (max(y, 1u) - 1u) * n + x].w) * 0.5;
    planes[at(p.dst, y * n + x)] = vec4<f32>(gx, gy, 0.0, 0.0);
    let l = sqrt(gx * gx + gy * gy);
    if l > 0.0 {
        atomicMax(&peaks[p.pk + 1u], bitcast<u32>(l));
    }
}

// the layer's own colour glowing round it (src: the layer blurred)
@compute @workgroup_size(8, 8)
fn glow(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let b = planes[at(p.src, i)];
    let sv = b.w;
    let a = clamp(sv * p.glow * 0.9, 0.0, 1.0) * mask(i);
    if a <= 0.0 { return; }
    let sa = max(sv, 1e-4);
    put(p.dst, i, clamp(b.rgb / sa, vec3<f32>(0.0), vec3<f32>(1.0)), a, 0u);
}

// a shadow under the layer (src: the layer blurred), `dy` pixels down
@compute @workgroup_size(8, 8)
fn shadow(@builtin(global_invocation_id) g: vec3<u32>) {
    let n = p.n;
    if g.x >= n || g.y >= n || g.y < p.dy { return; }
    let i = g.y * n + g.x;
    let b = planes[at(p.src, (g.y - p.dy) * n + g.x)];
    let sv = b.w;
    var a = max((sv - 0.01) * 1.0101, 0.0) * p.shadow_amount * 0.5;
    a *= ease(f32(min(min(g.x, n - 1u - g.x), min(g.y, n - 1u - g.y))) / p.fade);
    a *= mask(i);
    if a <= 0.0 { return; }
    var c = vec3<f32>(0.03, 0.06, 0.14);
    if p.flag != 0u {
        let base = b.rgb / max(sv, 1e-4);
        let l = 0.3 * base.r + 0.59 * base.g + 0.11 * base.b;
        c = clamp((vec3<f32>(l) + (base - vec3<f32>(l)) * 1.4) * 0.7, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    put(p.dst, i, c, a, 0u);
}

// what is behind the glass frosted (src: it blurred; src2: the layer)
@compute @workgroup_size(8, 8)
fn frost(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let w = clamp(planes[at(p.src2, i)].w, 0.0, 1.0);
    if w > 0.0 {
        let j = at(p.dst, i);
        planes[j] = mix(planes[j], planes[at(p.src, i)], w);
    }
}

// what is behind the glass bent at its edge (src: a copy of it; src2: the layer; src3: the slope)
@compute @workgroup_size(8, 8)
fn bend_behind(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let w = clamp(planes[at(p.src2, i)].w, 0.0, 1.0);
    if w <= 0.0 { return; }
    let h = planes[at(p.src3, i)];
    let s = h.z / peak(p.pk, 1e-6);
    let k2 = s * s;
    if k2 <= 0.0 { return; }
    let d = -h.xy * p.bend_out * k2;
    let x = f32(g.x);
    let y = f32(g.y);
    let vr = bilinear(p.src, x + d.x * (1.0 + 0.1 * p.dispersion), y + d.y * (1.0 + 0.1 * p.dispersion));
    let vg = bilinear(p.src, x + d.x, y + d.y);
    let vb = bilinear(p.src, x + d.x * (1.0 - 0.1 * p.dispersion), y + d.y * (1.0 - 0.1 * p.dispersion));
    let j = at(p.dst, i);
    planes[j] = mix(planes[j], vec4<f32>(vr.x, vg.y, vb.z, vg.w), w);
}

// the glass bends its own picture at the edges (src: a copy of the layer; src3: the slope; dst: the layer)
@compute @workgroup_size(8, 8)
fn inner_bend(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let own = planes[at(p.src, i)];
    let la = own.w;
    if la <= 0.0 { return; }
    let h = planes[at(p.src3, i)];
    let s = h.z / peak(p.pk, 1e-6);
    let k2 = s * s;
    if k2 <= 0.0 { return; }
    let v = bilinear(p.src, f32(g.x) - h.x * p.bend_in * k2, f32(g.y) - h.y * p.bend_in * k2);
    let w = ease((v.w / max(la, 1e-4) - 0.3) / 0.6);
    if v.w > 1e-4 && w > 0.0 {
        let j = at(p.dst, i);
        planes[j] = vec4<f32>(mix(own.rgb, v.rgb / v.w * la, w), planes[j].w);
    }
}

// the layer itself (src) put on
@compute @workgroup_size(8, 8)
fn put_layer(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let l = planes[at(p.src, i)];
    if l.w <= 0.0 { return; }
    put(p.dst, i, l.rgb / l.w, clamp(l.w * p.opacity * p.see_through, 0.0, 1.0), p.blend);
}

// the bevel and the shade inside along the edge (src: the layer; src2: the bevel's slope; src3: the layer blurred)
@compute @workgroup_size(8, 8)
fn bevel(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let a = planes[at(p.src, i)].w;
    if a <= 0.0 { return; }
    let j = at(p.dst, i);
    var acc = planes[j];
    let da = acc.w;
    if (p.flag & 1u) != 0u {
        let bp = peak(p.pk + 1u, 1e-4);
        let s = planes[at(p.src2, i)].xy / bp;
        let edge = clamp(length(s), 0.0, 1.0);
        let facing = -(s.x * p.lx + s.y * p.ly);
        let v = clamp(facing, -1.0, 1.0) * spow(edge, 0.7) * p.bevel * ease(a) * p.opacity;
        for (var k = 0u; k < 3u; k++) {
            let c = acc[k];
            acc[k] = select(max(c * (1.0 + v * 0.7), 0.0), mix(c, da, min(v * 0.6, 0.8)), v > 0.0);
        }
    }
    if (p.flag & 2u) != 0u {
        let edge = spow(clamp(2.0 * (1.0 - planes[at(p.src3, i)].w), 0.0, 1.0), 1.5);
        let v = clamp(edge * p.inner_shadow * ease(a) * p.opacity * 0.55, 0.0, 0.8);
        acc = vec4<f32>(acc.rgb * (1.0 - v), acc.w);
    }
    planes[j] = acc;
}

// light on the edges and the sheen across the glass (src: the layer; src2: the slope; src3, src4: the layer blurred wide
// and a little)
@compute @workgroup_size(8, 8)
fn specular(@builtin(global_invocation_id) g: vec3<u32>) {
    let n = p.n;
    if g.x >= n || g.y >= n { return; }
    let i = g.y * n + g.x;
    let a = planes[at(p.src, i)].w;
    if a <= 0.0 { return; }
    let nrm = planes[at(p.src2, i)].xy;
    let nd = nrm.x * p.lx + nrm.y * p.ly;
    let edge = clamp((2.0 * (1.0 - planes[at(p.src4, i)].w) - 0.5) * p.edge_sharp + 0.5, 0.0, 1.0);
    let halo = clamp(2.0 * (1.0 - planes[at(p.src3, i)].w), 0.0, 1.0) * (1.0 - 0.7 * p.sharp01);
    let spec = spow(max(nd, 0.0), 1.6) + spow(max(-nd, 0.0), 2.4) * 0.65;
    let j = at(p.dst, i);
    let acc = planes[j];
    let da = acc.w;
    var lum = 0.0;
    if da > 1e-4 {
        lum = (0.299 * acc.r + 0.587 * acc.g + 0.114 * acc.b) / da;
    }
    let rim_k = 1.0 + (0.3 - 1.0) * ease((lum - 0.35) / 0.45);
    let light = rim_k * (edge * (0.15 + 0.55 * spec) * (0.55 + 1.3 * p.sharp01) * p.edge_line + halo * (0.055 + 0.075 * spec) * p.soft_light) * p.rim;
    let dark = min(0.05 * halo * (1.0 - min(spec, 1.0)) * p.rim, 0.12);
    let nf = f32(n);
    let px = f32(g.x) / nf - 0.5;
    let py = f32(g.y) / nf - 0.5;
    let sh = clamp(0.5 - (px * p.lx + py * p.ly) * 0.9, 0.0, 1.0);
    let sheen = sh * sh * p.sheen;
    let lit = clamp(acc.rgb + vec3<f32>(da * (light + sheen) - da * dark), vec3<f32>(0.0), vec3<f32>(da));
    planes[j] = vec4<f32>(mix(acc.rgb, lit, min(a, 1.0)), da);
}

// Buf::to_image: straight alpha, 8 bits a channel
@compute @workgroup_size(8, 8)
fn pack(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.n || g.y >= p.n { return; }
    let i = g.y * p.n + g.x;
    let v = planes[at(p.src, i)];
    let a = clamp(v.w, 0.0, 1.0);
    var c = vec3<u32>(0u);
    if a > 0.0 {
        c = vec3<u32>(floor(clamp(v.rgb / a, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + 0.5));
    }
    let al = u32(floor(a * 255.0 + 0.5));
    outp[i] = c.x | (c.y << 8u) | (c.z << 16u) | (al << 24u);
}
