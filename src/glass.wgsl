// Jelly Glass: refraction + dispersion + blur + rim light, evaluated per panel.
// `bg` is the already-rendered scene beneath the panel (wallpaper / windows).

struct Panel {
    // xy = centre (px), zw = half size (px)
    rect: vec4<f32>,
    // x = corner radius, y = kind (0 glass, 1 solid, 2 white glass), z = bezel width, w = refraction strength
    params: vec4<f32>,
    // rgb = tint, a = tint strength (glass) / opacity (solid)
    tint: vec4<f32>,
    // x = blur radius px, y = dispersion, z = rim strength, w = gradient mix (solid)
    fx: vec4<f32>,
    // second colour for solid gradients
    color2: vec4<f32>,
    // x = shadow opacity, y = shadow blur px, z = shadow y-offset
    extra: vec4<f32>,
    // clip box (centre, half size; z = 0 means none): nothing of this panel is drawn outside it
    clip: vec4<f32>,
};

struct Globals {
    res: vec2<f32>,
    count: u32,
    scale: f32,
    // x = 1 when the wallpaper texture is a real image
    flags: vec4<u32>,
    // binned draws: x, y = tile grid origin (px), z = tile size (px), w = tiles per row (0 = not binned)
    grid: vec4<f32>,
    // x = opacity of the whole draw
    fade: vec4<f32>,
    // genie warp: window rect (x, y, w, h); (progress, dock x, dock y, icon width)
    warp: vec4<f32>,
    genie: vec4<f32>,
    panels: array<Panel, 576>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var bg: texture_2d<f32>;
@group(0) @binding(2) var bgs: sampler;
// Lucide icon atlas: 16x16 cells of 64px (mip chain rasterised per level), white with alpha
@group(0) @binding(3) var atlas: texture_2d<f32>;

// UI text atlas (R8 coverage): shelf-packed rasterised lines
@group(0) @binding(4) var text_atlas: texture_2d<f32>;

// the client surface being drawn by `fs_surface` (a placeholder in every other pass)
@group(0) @binding(5) var surf: texture_2d<f32>;

// application icons (premultiplied RGBA): 16x16 cells of 128px, 3 mips
@group(0) @binding(6) var app_atlas: texture_2d<f32>;

// per-draw tile table: [tiles_x * tiles_y + 1 offsets][panel indices], see Renderer::write_slot
@group(0) @binding(7) var<storage, read> tiles: array<u32>;

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) px: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4(p[i], 0.0, 1.0);
    o.px = (vec2(p[i].x, -p[i].y) * 0.5 + 0.5) * g.res;
    return o;
}

// ---------- procedural wallpaper (macOS-26-ish blue swirl) ----------
fn wallpaper(uv: vec2<f32>) -> vec3<f32> {
    let a = g.res.x / g.res.y;
    var p = (uv - vec2(0.5, 0.5)) * vec2(a, 1.0);
    let c = vec2(1.15, 0.9);
    let d = p - c;
    let r = length(d);
    let ang = atan2(d.y, d.x);
    let band = sin(r * 11.0 - ang * 1.5 + 0.8 * sin(ang * 3.0 + r * 4.0)) * 0.5 + 0.5;
    let band2 = sin(r * 23.0 + ang * 2.0) * 0.5 + 0.5;
    let deep = vec3(0.02, 0.10, 0.62);
    let mid  = vec3(0.08, 0.42, 0.95);
    let sky  = vec3(0.45, 0.78, 0.98);
    let warm = vec3(0.98, 0.62, 0.30);
    var col = mix(deep, mid, smoothstep(0.0, 1.0, band));
    col = mix(col, sky, smoothstep(0.55, 1.0, band2) * 0.55);
    // bright sweeping arc, top-left to right
    let arc = smoothstep(0.085, 0.0, abs(r - 1.05));
    col = mix(col, vec3(0.75, 0.9, 1.0), arc * 0.8);
    // warm glow bottom-left like the Dock reference
    let w = smoothstep(0.9, 0.0, length((uv - vec2(0.12, 1.05)) * vec2(a, 1.0)));
    col = mix(col, warm, w * 0.7);
    return col;
}

// ---------- SDF ----------
fn sd_round_rect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// A rounded rectangle whose corners are superellipses (Apple's "continuous" corners, a squircle) when r < 0.
fn sd_shape(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    if (r >= 0.0) { return sd_round_rect(p, b, r); }
    let rr = -r;
    let q = abs(p) - b + vec2(rr);
    let qp = max(q, vec2(0.0));
    // exponent 3: softer than the icon squircle (5), which makes a pill's ends look boxy
    let l = pow(pow(qp.x, 3.0) + pow(qp.y, 3.0), 0.3333333);
    return l + min(max(q.x, q.y), 0.0) - rr;
}

fn sdf_grad(p: vec2<f32>, b: vec2<f32>, r: f32) -> vec2<f32> {
    let e = 0.75;
    let dx = sd_shape(p + vec2(e, 0.0), b, r) - sd_shape(p - vec2(e, 0.0), b, r);
    let dy = sd_shape(p + vec2(0.0, e), b, r) - sd_shape(p - vec2(0.0, e), b, r);
    let v = vec2(dx, dy);
    let l = length(v);
    return select(vec2(0.0, -1.0), v / l, l > 1e-5);
}

fn sample_bg(px: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(bg, bgs, px / g.res, 0.0).rgb;
}

// Glass keeps two numbers in the fraction of its kind: edge sharpness (3 bits) and transparency (6 bits), see `glass_code`.
fn glass_idx(py: f32) -> u32 { return min(u32(max(py, 0.0) / 0.49 * 512.0 + 0.5), 511u); }

// 16-tap golden-angle disc blur
fn blur_bg(px: vec2<f32>, radius: f32) -> vec3<f32> {
    if (radius < 0.5) { return sample_bg(px); }
    // the backdrop has a mip chain (box-filtered pyramid): pick the level whose texels are about
    // as wide as the blur, and smooth the blockiness of that level with a few offset taps
    let levels = f32(textureNumLevels(bg));
    let lod = clamp(log2(max(radius * g.scale * 0.85, 1.0)), 0.0, levels - 1.0);
    let uv = px / g.res;
    let texel = vec2(exp2(lod)) / (g.res * g.scale);
    var acc = textureSampleLevel(bg, bgs, uv, lod).rgb * 0.4;
    acc += textureSampleLevel(bg, bgs, uv + vec2(0.8, 0.8) * texel, lod).rgb * 0.15;
    acc += textureSampleLevel(bg, bgs, uv + vec2(-0.8, 0.8) * texel, lod).rgb * 0.15;
    acc += textureSampleLevel(bg, bgs, uv + vec2(0.8, -0.8) * texel, lod).rgb * 0.15;
    acc += textureSampleLevel(bg, bgs, uv + vec2(-0.8, -0.8) * texel, lod).rgb * 0.15;
    return acc;
}

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3(0.299, 0.587, 0.114)); }

// Adaptive ink: black on bright backdrops, white on dark ones. fx.x = 1 + strength of the glass tint
// under the element, fx.y = that tint's luminance; the backdrop itself is sampled (blurred) at the centre.
fn auto_ink(pn: Panel, base: vec3<f32>, under: vec4<f32>) -> vec3<f32> {
    if (pn.fx.x < 0.5) { return base; }
    // one decision per element (sampled at its centre), never a gradient inside a glyph:
    // the backdrop, tinted by the glass it sits on, then shaded the way the glass itself darkens
    var eff = mix(luma(blur_bg(pn.rect.xy, 22.0)), pn.fx.y, clamp(pn.fx.x - 1.0, 0.0, 1.0));
    eff = eff * (1.0 - 0.46 * select(bitcast<f32>(g.flags.z), 0.0, pn.fx.z > 0.5) * smoothstep(0.38, 0.85, eff));
    return select(vec3(1.0), vec3(0.06, 0.08, 0.13), eff > 0.6);
}

fn saturate_col(c: vec3<f32>, s: f32) -> vec3<f32> {
    return mix(vec3(luma(c)), c, s);
}

// Convex-lens height profile: 0 at the outer edge, 1 at the inner bezel limit.
fn lens(t: f32) -> f32 {
    let u = clamp(t, 0.0, 1.0);
    return sqrt(1.0 - (1.0 - u) * (1.0 - u));
}

fn shade_glass(px: vec2<f32>, pn: Panel, sd: f32, n: vec2<f32>, base: vec3<f32>) -> vec3<f32> {
    let bezel = abs(pn.params.z);
    let depth = -sd;                       // px inside the shape
    let t = depth / max(bezel, 1.0);       // 0 at edge .. 1 at bezel limit
    // Refraction offset: strong near the edge, fades to zero inside.
    let k = (1.0 - lens(t));
    let k2 = select(0.0, k * k, t < 1.0);
    let disp = -n * k2 * pn.params.w;      // sample from further inside => edge bends scene inward
    let dsp = pn.fx.y;

    let br = pn.fx.x;
    var col: vec3<f32>;
    if (k2 <= 0.0001 || dsp < 0.001) {
        // no refraction here (or no dispersion): one blurred sample instead of three
        col = blur_bg(px + disp, br);
    } else {
        col.r = blur_bg(px + disp * (1.0 + dsp), br).r;
        col.g = blur_bg(px + disp, br).g;
        col.b = blur_bg(px + disp * (1.0 - dsp), br).b;
    }

    col = saturate_col(col, select(pn.extra.w, 1.15, pn.extra.w <= 0.0));
    // tint + slight lift
    col = mix(col, pn.tint.rgb, pn.tint.a);
    col = col * 1.0 + 0.015;
    // adaptive shade: over a bright backdrop the glass darkens, so white content stays readable
    // (only for panels marked adaptive: Dock, Quick Settings, menu bar; not tooltips). The shade is a deeper,
    // slightly cooler and more saturated version of the colour, not a plain grey multiply.
    if (pn.params.z < 0.0) {
        let k = bitcast<f32>(g.flags.z) * smoothstep(0.38, 0.85, luma(col));
        let shade = col * mix(vec3(1.0), vec3(0.50, 0.57, 0.66), k);
        col = mix(vec3(luma(shade)), shade, 1.0 + 0.25 * k);
    }

    // Rim lighting: bright at top-left and bottom-right, fading inward (Apple-style). Deep inside the shape the
    // terms are far below what an 8-bit picture can show: skipped (a big panel is mostly deep inside).
    if (depth < 40.0) {
        let L = normalize(vec2(-0.6, -0.8));
        let nd = dot(n, L);
        // edge sharpness rides in the fractional part of the kind: 0 = soft, 1 = a crisp hairline
        let sharp = select(0.0, f32(glass_idx(pn.params.y) >> 6u) / 7.0, u32(pn.params.y + 0.5) == 0u);
        let edge = exp(-depth / mix(1.6, 0.55, sharp));          // thin specular line
        let glow = exp(-depth / 7.0) * (1.0 - 0.7 * sharp);      // soft inner glow
        let spec = (pow(max(nd, 0.0), 1.6) + pow(max(-nd, 0.0), 2.4) * 0.65);
        // rim fades on bright backgrounds so it never blows out to white
        let rim_k = mix(1.0, 0.3, smoothstep(0.35, 0.8, luma(col)));
        col += vec3(1.0) * rim_k * (edge * (0.15 + 0.55 * spec) * (0.55 + 1.3 * sharp) + glow * (0.03 + 0.10 * spec)) * pn.fx.z;
        // darker inner-edge shadow on the unlit sides for depth
        col -= vec3(0.05) * glow * (1.0 - spec) * pn.fx.z;
    }

    // glossy sheen (Aero): a lighter upper half and a diagonal streak, darker lower half
    if (pn.fx.w > 0.0) {
        let size = 2.0 * pn.rect.zw;
        let rel = px - (pn.rect.xy - pn.rect.zw);
        let qy = rel.y / size.y;
        let top = 1.0 - smoothstep(0.0, 0.55, qy);
        let dd = rel.x - rel.y * 1.2;
        let streak = smoothstep(0.07 * size.x, 0.0, abs(dd - 0.30 * size.x)) * (1.0 - qy);
        col += vec3(1.0) * pn.fx.w * (0.10 * top + 0.12 * streak);
        col -= vec3(0.04) * pn.fx.w * smoothstep(0.5, 1.0, qy);
    }
    // 1px inner border in the theme's border colour
    let band = 1.0 - smoothstep(0.0, 1.5, depth);
    col = mix(col, pn.color2.rgb, pn.color2.a * band);
    return col;
}

fn shade_solid(px: vec2<f32>, pn: Panel, sd: f32, n: vec2<f32>) -> vec3<f32> {
    let h = pn.rect.w;
    let y = clamp((px.y - (pn.rect.y - h)) / (2.0 * h), 0.0, 1.0);
    var col = mix(pn.tint.rgb, pn.color2.rgb, smoothstep(0.0, 1.0, y * pn.fx.w + (1.0 - pn.fx.w) * 0.5));
    // subtle top gloss and edge hairline
    col += vec3(0.08) * smoothstep(0.35, 0.0, y) * pn.fx.z;
    let depth = -sd;
    if (depth < 12.0) {
        col += vec3(0.20) * exp(-depth / 1.2) * 0.35 * pn.fx.z;
    }
    return col;
}

// premultiplied "over": add colour `c` with alpha `a` on top of `acc`
fn over_rgb(acc: vec4<f32>, c: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4(c * a + acc.rgb * (1.0 - a), a + acc.a * (1.0 - a));
}

// darken what is already there by `s` (drop shadows)
fn dim_acc(acc: vec4<f32>, s: f32) -> vec4<f32> {
    return vec4(acc.rgb * (1.0 - s), s + acc.a * (1.0 - s));
}

// ---- paper: sheets with torn, uneven edges, fibres and creases (kind 12)
fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2(1.0, 0.0));
    let c = hash21(i + vec2(0.0, 1.0));
    let d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var o = 0; o < 4; o++) {
        s += a * vnoise(q);
        q = q * 2.03 + vec2(17.1, 9.2);
        a *= 0.5;
    }
    return s;
}

// every sheet has its own tear: a seed from its size and its place in the list
fn paper_seed(pn: Panel, idx: f32) -> vec2<f32> {
    return vec2(hash21(pn.rect.zw + vec2(idx)) * 97.0, hash21(pn.rect.zw + vec2(idx * 1.7, 3.7)) * 53.0);
}

// the sheet's outline: the rounded rectangle, wandering by `fx.x` px with a ragged fringe on top
fn torn_sd(local: vec2<f32>, pn: Panel, seed: vec2<f32>) -> f32 {
    let base = sd_shape(local, pn.rect.zw, pn.params.x);
    let amp = pn.fx.x;
    let big = fbm(local * 0.045 + seed) - 0.47;
    let small = vnoise(local * 0.55 + seed * 1.7) - 0.5;
    return base + big * amp * 1.6 + small * amp * 0.28;
}

fn paper_panel(i: u32, px: vec2<f32>, acc_in: vec4<f32>) -> vec4<f32> {
    let pn = g.panels[i];
    let local = px - pn.rect.xy;
    let seed = paper_seed(pn, f32(i));
    var acc = acc_in;
    // a soft shadow cast by the torn shape itself
    let bl = max(pn.extra.y, 1.0);
    if (pn.extra.x > 0.0) {
        let sds = max(torn_sd(local - vec2(1.5, pn.extra.z), pn, seed), 0.0);
        if (sds < bl * 1.6) {
            acc = dim_acc(acc, pn.extra.x * exp(-2.0 * sds * sds / (bl * bl)));
        }
    }
    let sd = torn_sd(local, pn, seed);
    if (sd * g.scale > 1.0) { return acc; }
    var a = clamp(0.5 - sd * g.scale, 0.0, 1.0);
    // the paper: its colour, grain and long fibres
    var col = pn.tint.rgb;
    let grain = vnoise(px * 1.1) * 0.5 + vnoise(px * 0.37 + seed) * 0.5;
    let fib = vnoise(vec2(px.x * 0.11, px.y * 1.7) + seed) * vnoise(vec2(px.x * 1.9, px.y * 0.09) + seed * 2.0);
    col *= 1.0 + (grain - 0.5) * 0.14 * pn.fx.y + (fib - 0.25) * 0.20 * pn.fx.y;
    // creases: a slowly changing height, lit from the top left
    let hq = px * 0.018 + seed;
    let e = 0.06;
    let gx = fbm(hq + vec2(e, 0.0)) - fbm(hq - vec2(e, 0.0));
    let gy = fbm(hq + vec2(0.0, e)) - fbm(hq - vec2(0.0, e));
    col *= 1.0 + (gx * -0.6 + gy * -0.8) * 1.4 * pn.fx.z;
    // where the sheet was torn the pale core of the paper shows, in a band that wanders
    let depth = -sd;
    let band = 1.0 - smoothstep(0.0, 3.0 + 3.0 * vnoise(local * 0.9 + seed), depth);
    col = mix(col, vec3(0.97, 0.96, 0.93), band * 0.85);
    // a little darker towards the bottom of the sheet
    col *= 1.0 - 0.05 * clamp(local.y / (2.0 * pn.rect.w) + 0.5, 0.0, 1.0);
    // a ragged fringe of fibres right at the edge
    let rough = vnoise(local * 1.3 + seed * 3.0);
    if (sd > -1.0) { a *= smoothstep(0.1, 0.9, rough + (-sd) * 0.8); }
    return over_rgb(acc, col, a);
}

// ---- wood: boards with grain, rings, knots and a varnished bevel (kind 13)
// fx.x = ring density, fx.y = grain strength, fx.z = bevel light, fx.w = varnish sheen; tint = the wood's colour
fn shade_wood(i: u32, px: vec2<f32>, pn: Panel, sd: f32, n: vec2<f32>) -> vec3<f32> {
    let seed = vec2(hash21(pn.rect.zw + vec2(f32(i))) * 91.0, hash21(pn.rect.zw + vec2(f32(i) * 1.3, 5.1)) * 47.0);
    let q = px + seed * 11.0;
    // Long, calm grain: the growth rings run along the board, bent a little by slow noise. (Strong bending looks
    // like marble, not wood.)
    var warp = fbm(vec2(q.x * 0.0035, q.y * 0.020)) * 1.7 + 0.35 * fbm(vec2(q.x * 0.012, q.y * 0.08));
    // a knot, rarely: the rings bend round it
    let kc = pn.rect.xy + (vec2(hash21(seed), hash21(seed + 7.7)) - 0.5) * 1.2 * pn.rect.zw;
    let kd = length((px - kc) * vec2(0.55, 1.0));
    let knot = select(0.0, 1.0, hash21(seed + 3.3) > 0.78);
    warp += knot * 1.4 * exp(-kd * kd / 500.0);
    let rings = pow(0.5 + 0.5 * sin(q.y * 0.11 * pn.fx.x + warp * 5.0), 1.7);
    let dark = pn.tint.rgb * vec3(0.66, 0.57, 0.50);
    let light = min(pn.tint.rgb * 1.06 + vec3(0.02), vec3(1.0));
    var col = mix(light, dark, rings * 0.34 * pn.fx.y);
    // thin fibres along the grain, and slow variation in tone from one stretch of board to the next
    let fine = vnoise(vec2(q.x * 0.018, q.y * 1.7));
    col *= 1.0 + (fine - 0.5) * 0.13 * pn.fx.y;
    col *= 0.93 + 0.14 * fbm(px * 0.0045 + seed);
    // pores: short dark dashes
    col *= 1.0 - 0.07 * pn.fx.y * smoothstep(0.70, 0.98, vnoise(vec2(q.x * 0.09, q.y * 2.6)));
    // the knot itself is darker, with a darker ring round it
    col *= 1.0 - knot * (0.30 * exp(-kd * kd / 70.0) + 0.10 * exp(-kd * kd / 400.0));
    // a bevelled edge: bright on the lit side, dark opposite, and a thin dark outline
    let depth = -sd;
    let L = normalize(vec2(-0.6, -0.8));
    let nd = dot(n, L);
    let edge = exp(-depth / 2.2);
    col += vec3(1.0) * edge * (max(nd, 0.0) * 0.34 + 0.04) * pn.fx.z;
    col -= vec3(0.30) * exp(-depth / 3.5) * max(-nd, 0.0) * pn.fx.z;
    col *= 1.0 - 0.16 * (1.0 - smoothstep(0.0, 1.2, depth));
    // varnish: a soft diagonal sheen over the whole board
    let size = 2.0 * pn.rect.zw;
    let rel = px - (pn.rect.xy - pn.rect.zw);
    let dd = rel.x / size.x - rel.y / size.y * 0.9;
    col += vec3(1.0) * pn.fx.w * (0.10 * smoothstep(0.30, 0.0, abs(dd - 0.35)) + 0.04 * (1.0 - rel.y / size.y));
    return col;
}

// ---- clay: soft, opaque, puffy surfaces (kind 14)
// params.z = how wide the rounded rim is (px), fx.y = surface texture, fx.z = highlight, fx.w = glaze
fn shade_clay(i: u32, px: vec2<f32>, pn: Panel, sd: f32, n: vec2<f32>) -> vec3<f32> {
    let seed = vec2(hash21(pn.rect.zw + vec2(f32(i))) * 83.0, hash21(pn.rect.zw + vec2(f32(i) * 1.9, 2.3)) * 61.0);
    let bz = max(abs(pn.params.z), 6.0);
    let depth = -sd;
    let t = clamp(depth / bz, 0.0, 1.0);
    // a pillow: the surface tilts outwards at the rim and is level in the middle
    let slope = (1.0 - t) * (1.0 - t);
    let n3 = normalize(vec3(n * slope * 1.1, 1.0));
    let L3 = normalize(vec3(-0.5, -0.7, 0.62));
    let flat_lit = L3.z;
    let lit = dot(n3, L3);
    let base = pn.tint.rgb;
    var col = base * (1.0 + 0.95 * (lit - flat_lit));
    // inner shadow on the far side, tinted by the clay itself
    let L2 = normalize(vec2(-0.6, -0.8));
    let nd = dot(n, L2);
    let shade_k = exp(-depth / (bz * 0.55)) * max(-nd, 0.0);
    col = mix(col, base * vec3(0.82, 0.74, 0.78), shade_k * 0.50);
    // soft light on the lit rim
    let hl = exp(-depth / (bz * 0.40)) * max(nd, 0.0);
    col += vec3(1.0) * hl * 0.20 * pn.fx.z;
    // a glaze: the top of the piece catches a little more light
    let ty = clamp((px.y - (pn.rect.y - pn.rect.w)) / (2.0 * pn.rect.w), 0.0, 1.0);
    col += vec3(1.0) * pn.fx.w * 0.05 * (1.0 - smoothstep(0.0, 0.7, ty));
    // fingertip-soft unevenness of the surface and slow differences in tone
    let tex = vnoise(px * 0.9 + seed) * 0.6 + vnoise(px * 2.6 + seed * 2.0) * 0.4;
    col *= 1.0 + (tex - 0.5) * 0.07 * pn.fx.y + (fbm(px * 0.012 + seed) - 0.46) * 0.10 * pn.fx.y;
    return col;
}

// Composite the panels over `start` (premultiplied). Shell: start = opaque backdrop; window frames: start = transparent.
// Specialised pipelines: a draw whose panels hold no paper, wood, clay, vinyl or glass lettering is compiled without them
// (those branches make the shader's registers so many that fewer pixels run at once, even when never taken).
override EXOTIC: bool = true;

fn apply_panel_raw(i: u32, px: vec2<f32>, acc_in: vec4<f32>) -> vec4<f32> {
    if (EXOTIC && u32(g.panels[i].params.y + 0.5) == 12u) {
        // (the reject below would need the shadow numbers: the paper does its own)
        let q = abs(px - g.panels[i].rect.xy) - g.panels[i].rect.zw;
        if (max(q.x, q.y) > max(g.panels[i].extra.y * 1.6 + abs(g.panels[i].extra.z), 2.0) + g.panels[i].fx.x * 2.0) { return acc_in; }
        return paper_panel(i, px, acc_in);
    }
    var acc = acc_in;
    // cheap reject first (only rect + shadow reach are read): skip panels far from this pixel
    let prect = g.panels[i].rect;
    let pex = g.panels[i].extra;
    let local = px - prect.xy;
    // glass lettering keeps its atlas position in `extra`, which says nothing about shadows
    let reach = select(max(pex.y * 1.6 + abs(pex.z), 2.0), 2.0, u32(g.panels[i].params.y + 0.5) >= 9u);
    let qq = abs(local) - prect.zw;
    if (max(qq.x, qq.y) > reach) { return acc; }
    let pn = g.panels[i];
    var sd = sd_shape(local, pn.rect.zw, pn.params.x);
    let kind = u32(pn.params.y + 0.5);
    if (EXOTIC && (kind == 9u || kind == 10u)) {
        return glass_glyph(px, pn, acc, kind == 10u);
    }
    if (EXOTIC && kind == 15u) {
        // A vinyl record in one panel: grooves, light wedges and a label that turn, optionally cut to a rounded rect.
        // rect = the disc (centre, radius, radius), tint = the vinyl's colour and opacity, params.z = label atlas cell
        // (negative: a plain label), fx.x = turn angle, fx.y = label radius (share of the disc), fx.z = sheen strength,
        // color2 = clip rect (centre, half size; z = 0 means none), extra.w = clip corner radius.
        var clip = 1.0;
        if (pn.color2.z > 0.0) {
            clip = clamp(0.5 - sd_round_rect(px - pn.color2.xy, pn.color2.zw, pn.extra.w) * g.scale, 0.0, 1.0);
        }
        let rad = pn.rect.z;
        let u = local / rad;
        let rr = length(u);
        let cover = clamp(0.5 - (rr - 1.0) * rad * g.scale, 0.0, 1.0) * clip;
        if (cover <= 0.0) { return acc; }
        let tone = pn.tint.rgb;
        let rpx = rr * rad;
        let lab = pn.fx.y;
        // fine rings that catch the light a little differently, a smooth ring by the label and a lighter rim
        var v = 0.10 + 0.07 * sin(rpx * 2.1) + 0.04 * sin(rpx * 5.7 + 1.3) + 0.03 * sin(rpx * 0.9);
        v = select(v, 0.07, rr < lab * 1.55);
        v = v + 0.05 * clamp((rr - 0.96) / 0.04, 0.0, 1.0);
        var col = tone * v;
        // two bright wedges that turn with the record, and a rim of light that stays where the light is
        let th = atan2(u.y, u.x) - pn.fx.x;
        let wedge = pow(max(cos(2.0 * (th - 0.6)), 0.0), 5.0);
        let radial = clamp((rr - lab * 1.2) / 0.1, 0.0, 1.0) * (1.0 - clamp((rr - 0.93) / 0.07, 0.0, 1.0));
        col = col + (vec3(0.65) + 0.35 * tone) * wedge * radial * 0.55 * pn.fx.z;
        col = col + vec3(0.05) * clamp(1.0 - length(u - vec2(-0.38, -0.38)) / 0.9, 0.0, 1.0);
        // the label, turning
        let lm = clamp((lab - rr) * rad * g.scale + 0.5, 0.0, 1.0);
        if (lm > 0.0) {
            let ul = u / lab;
            let ca = cos(pn.fx.x);
            let sa = sin(pn.fx.x);
            let ur = vec2(ca * ul.x - sa * ul.y, sa * ul.x + ca * ul.y);
            var lc = tone * (1.0 - 0.35 * length(ul));
            if (pn.params.z >= 0.0) {
                let id = u32(pn.params.z + 0.5);
                let cell = vec2(f32(id % 24u), f32(id / 24u));
                let uv = (cell + clamp(ur * 0.5 + vec2(0.5), vec2(0.004), vec2(0.996))) / 24.0;
                let lod = clamp(log2(128.0 / max(2.0 * lab * rad * g.scale, 1.0)), 0.0, 2.0);
                let t = textureSampleLevel(app_atlas, bgs, uv, lod);
                lc = t.rgb / max(t.a, 0.0001);
            } else {
                // a plain label: light spokes and a dot, so the turning shows
                let spoke = max(cos(3.0 * atan2(ur.y, ur.x)) - 0.55, 0.0) * 1.6 * (1.0 - length(ul));
                lc = min(lc + vec3(spoke * 0.35), vec3(1.0));
                lc = mix(lc, vec3(1.0), clamp(0.5 - (length(ur - vec2(0.58, 0.0)) - 0.12) * lab * rad * g.scale, 0.0, 1.0));
            }
            col = mix(col, lc, lm);
        }
        // the spindle
        col = mix(col, vec3(0.80, 0.81, 0.85), clamp(0.5 - (rpx - 4.0) * g.scale, 0.0, 1.0));
        return over_rgb(acc, col, cover * pn.tint.a);
    }
    if (kind == 7u) {
        // application icon from the atlas (premultiplied), drawn at tint.a opacity.
        // params.w = shape mask (1 squircle, 2 circle, 3 rounded), fx.x = padding, fx.y > 0.5 = one-colour tint (color2.rgb)
        let u = local / pn.rect.zw;
        let uu = u / max(1.0 - pn.fx.x, 0.2);
        if (abs(uu.x) <= 1.0 && abs(uu.y) <= 1.0) {
            let id = u32(pn.params.z + 0.5);
            let cell = vec2(f32(id % 24u), f32(id / 24u));
            // fx.z turns the picture (a record label); the mask below stays put
            let ca = cos(pn.fx.z);
            let sa = sin(pn.fx.z);
            let ur = vec2(ca * uu.x - sa * uu.y, sa * uu.x + ca * uu.y);
            let uv = (cell + clamp(ur * 0.5 + vec2(0.5), vec2(0.004), vec2(0.996))) / 24.0;
            let lod = clamp(log2(128.0 / max(2.0 * pn.rect.z * g.scale, 1.0)), 0.0, 2.0);
            var t = textureSampleLevel(app_atlas, bgs, uv, lod);
            if (pn.fx.y > 0.5) {
                let lum = luma(t.rgb / max(t.a, 0.0001));
                t = vec4(pn.color2.rgb * lum * t.a, t.a);
            }
            let shape = u32(pn.params.w + 0.5);
            if (shape != 0u) {
                var msd = 0.0;
                if (shape == 1u) {
                    msd = (pow(pow(abs(u.x), 5.0) + pow(abs(u.y), 5.0), 0.2) - 1.0) * pn.rect.z;
                } else if (shape == 2u) {
                    msd = (length(u) - 1.0) * pn.rect.z;
                } else {
                    msd = sd_round_rect(local, pn.rect.zw, pn.rect.z * 0.44);
                }
                t = t * clamp(0.5 - msd * g.scale, 0.0, 1.0);
            }
            // a picture cut to a rounded rectangle (color2 = centre + half size, extra.w = radius), when asked to
            if (pn.fx.y <= 0.5 && pn.color2.z > 0.0) {
                t = t * clamp(0.5 - sd_round_rect(px - pn.color2.xy, pn.color2.zw, pn.extra.w) * g.scale, 0.0, 1.0);
            }
            let o = pn.tint.a;
            acc = vec4(t.rgb * o + acc.rgb * (1.0 - t.a * o), t.a * o + acc.a * (1.0 - t.a * o));
        }
        return acc;
    }
    if (kind == 6u) {
        // UI text: 1:1 texel lookup in the text atlas
        let u = local / (2.0 * pn.rect.zw) + vec2(0.5);
        if (u.x >= 0.0 && u.x <= 1.0 && u.y >= 0.0 && u.y <= 1.0) {
            let ext = select(2.0 * pn.rect.zw, 2.0 * pn.color2.xy, pn.color2.x > 0.0);
            let texel = vec2(pn.params.z, pn.params.w) + u * (ext * g.scale);
            let a = textureSampleLevel(text_atlas, bgs, texel / vec2(2048.0, 1024.0), 0.0).r;
            var ink = pn.tint.rgb;
            if (a > 0.01) { ink = auto_ink(pn, ink, acc); }
            acc = over_rgb(acc, ink, a * pn.tint.a);
        }
        return acc;
    }
    if (kind == 5u) {
        // Lucide glyph: tinted, with the atlas alpha as coverage
        let u = local / (2.0 * pn.rect.zw) + vec2(0.5);
        if (u.x >= 0.0 && u.x <= 1.0 && u.y >= 0.0 && u.y <= 1.0) {
            let id = u32(pn.params.z + 0.5);
            let cell = vec2(f32(id % 16u), f32(id / 16u));
            let uv = (cell + clamp(u, vec2(0.003), vec2(0.997))) / 16.0;
            let lod = clamp(log2(64.0 / max(2.0 * pn.rect.z * g.scale, 1.0)), 0.0, 3.0);
            let a = textureSampleLevel(atlas, bgs, uv, lod).a;
            var ink = pn.tint.rgb;
            if (a > 0.01) { ink = auto_ink(pn, ink, acc); }
            acc = over_rgb(acc, ink, a * pn.tint.a);
        }
        return acc;
    }
    if (kind == 8u) {
        // blur + tint that fade out away from the screen edge (params.w = +1 strongest at the top, -1 at the bottom)
        let h2 = 2.0 * pn.rect.w;
        var ty = (px.y - (pn.rect.y - pn.rect.w)) / h2;
        if (pn.params.w < 0.0) { ty = 1.0 - ty; }
        if (ty < 0.0 || ty > 1.0 || abs(local.x) > pn.rect.z) { return acc; }
        let m = pow(1.0 - ty, 1.7);
        let blurred = blur_bg(px, pn.fx.x * m + 0.01);
        let col = mix(blurred, pn.tint.rgb, pn.tint.a * m);
        return over_rgb(acc, col, clamp(m * 1.6, 0.0, 1.0));
    }
    // soft drop shadow (gaussian falloff outside the shape)
    let bl = pn.extra.y;
    // (inside the shape itself the shadow is not drawn: `outside` below is 0 there)
    if (pn.extra.x > 0.0 && bl > 0.0 && sd * g.scale > -0.5) {
        let sds = max(sd_shape(local - vec2(0.0, pn.extra.z), pn.rect.zw, pn.params.x), 0.0);
        if (sds < bl * 1.6) {
            // only outside the shape itself, so translucent windows are not darkened by their own shadow
            let outside = clamp(0.5 + sd * g.scale, 0.0, 1.0);
            acc = dim_acc(acc, pn.extra.x * exp(-2.0 * sds * sds / (bl * bl)) * outside);
        }
    }
    if (kind == 3u) {
        // Apple-style continuous-corner "squircle" (superellipse, n = 5)
        let u = local / pn.rect.zw;
        sd = (pow(pow(abs(u.x), 5.0) + pow(abs(u.y), 5.0), 0.2) - 1.0) * pn.rect.z;
    }
    if (sd * g.scale > 1.0) { return acc; }
    let cov = clamp(0.5 - sd * g.scale, 0.0, 1.0);
    // the normal only matters near the edge (refraction, rim light): deep inside it is skipped
    var n = vec2(0.0, -1.0);
    if (-sd < 56.0) { n = sdf_grad(local, pn.rect.zw, pn.params.x); }
    var p: vec3<f32>;
    if (kind == 0u) {
        // soft drop shadow handled outside; glass samples the current framebuffer-bg
        p = shade_glass(px, pn, sd, n, acc.rgb);
    } else if (kind == 1u) {
        p = auto_ink(pn, shade_solid(px, pn, sd, n), acc);
    } else if (EXOTIC && kind == 13u) {
        p = shade_wood(i, px, pn, sd, n);
    } else if (EXOTIC && kind == 14u) {
        p = shade_clay(i, px, pn, sd, n);
    } else if (kind == 3u) {
        p = shade_solid(px, pn, sd, n);
        let aa = 1.0 / (pn.rect.z * g.scale);
        p = icon_paint(u32(pn.params.z + 0.5), local / pn.rect.zw, aa, p);
        // soft inner edge so the icon reads as a glossy tile
        p += vec3(0.5) * exp(-(-sd) / 0.9) * 0.18;
    } else {
        // opaque white "active" glass (Quick Settings toggle)
        p = mix(vec3(1.0), vec3(0.93, 0.95, 1.0), smoothstep(-6.0, 0.0, sd));
    }
    acc = over_rgb(acc, p, cov * select(1.0, pn.tint.a, kind == 1u));
    return acc;
}

// A panel's own opacity (`Panel::faded`): everything it did (shadow, refraction, tint, rim) is mixed with what was there
// before, so glass fades in and out as one piece instead of keeping its blur while only the tint thins out. How
// transparent the panel is rides in the fractional part of `params.y` (the kind): kind + 0.49 * transparency. Every
// kind is read with `u32(params.y + 0.5)`, which ignores a fraction below one half, so nothing else changes.
fn apply_panel(i: u32, px: vec2<f32>, acc_in: vec4<f32>) -> vec4<f32> {
    var keep = 1.0;
    let cl = g.panels[i].clip;
    if (cl.z > 0.0) {
        let cq = abs(px - cl.xy) - cl.zw;
        keep = clamp(0.5 - max(cq.x, cq.y) * g.scale, 0.0, 1.0);
        if (keep <= 0.0) { return acc_in; }
    }
    let out = mix(acc_in, apply_panel_raw(i, px, acc_in), keep);
    let py = g.panels[i].params.y;
    var t = clamp((py - floor(py + 0.001)) / 0.49, 0.0, 1.0);
    // glass: the fraction holds the edge sharpness (3 bits) and the transparency (6 bits)
    if (u32(py + 0.5) == 0u) { t = f32(glass_idx(py) & 63u) / 63.0; }
    if (t > 0.002) { return mix(acc_in, out, 1.0 - t); }
    return out;
}

// Coverage of the lettering at an offset `q` (logical px) from the centre of its glyph box: 0 outside the box.
// The box is the panel's rect minus the padding in extra.x; the glyph bitmap sits at (extra.y, extra.z) in the text atlas.
fn glyph_cov(pn: Panel, q: vec2<f32>) -> f32 {
    let half = pn.rect.zw - vec2(pn.extra.x);
    let u = q / (2.0 * half) + vec2(0.5);
    if (u.x < 0.0 || u.x > 1.0 || u.y < 0.0 || u.y > 1.0) { return 0.0; }
    let texel = vec2(pn.extra.y, pn.extra.z) + u * (2.0 * half * g.scale);
    return textureSampleLevel(text_atlas, bgs, texel / vec2(2048.0, 1024.0), 0.0).r;
}

// Jelly Glass lettering: the glyph is a slab of glass. Rings of coverage samples around the pixel say how far the
// edge is (the bevel) and which way it faces (the normal); the usual glass shading does the rest.
fn glass_glyph(px: vec2<f32>, pn: Panel, acc: vec4<f32>, liquid: bool) -> vec4<f32> {
    let local = px - pn.rect.xy;
    let c0 = glyph_cov(pn, local);
    // The coverage, smoothed with a Gaussian: its value says how deep inside the letter we are, its slope which way
    // the edge faces. A grid of weighted taps (not a ring: a ring has only a few directions and the bevel turns
    // into facets).
    let bz = max(abs(pn.params.z), 4.0);
    let sg = bz * 0.55;
    let h = sg * 0.5;
    var gsum = vec2(0.0);
    var bsum = 0.0;
    var wsum = 0.0;
    for (var iy = -4; iy <= 4; iy++) {
        for (var ix = -4; ix <= 4; ix++) {
            let o = vec2(f32(ix), f32(iy)) * h;
            let w = exp(-dot(o, o) / (2.0 * sg * sg));
            let c = glyph_cov(pn, local + o);
            gsum += o * c * w;
            bsum += c * w;
            wsum += w;
        }
    }
    if (c0 <= 0.003 && bsum <= 0.001) { return acc; }
    let b = bsum / wsum;
    var n = vec2(0.0, -1.0);
    if (length(gsum) > 0.0001) { n = -normalize(gsum); }
    // 0 at the edge (half covered), 1 once well inside, rounded off so the bevel is a soft slope
    let t = clamp((b - 0.5) / 0.42, 0.0, 1.0);
    let inside = t * t * (3.0 - 2.0 * t);
    let depth = inside * bz;
    var p = shade_glass(px, pn, -depth, n, acc.rgb);
    // whatever panels underneath already did to the backdrop (the lock screen's veil) carries through the slab
    // (only where the layer already holds the picture: a window-layer frame starts out transparent)
    if (acc.a > 0.98) {
        p += acc.rgb - sample_bg(px);
    }
    if (liquid) {
        // Jelly Glass: smooth and luminous. No outline; the slab is lighter towards the top, glows softly from the
        // edge inwards and has a thin bright rim on the lit side.
        let half = pn.rect.zw - vec2(pn.extra.x);
        let ty = clamp(local.y / (2.0 * half.y) + 0.5, 0.0, 1.0);
        let Ll = normalize(vec2(-0.6, -0.8));
        let ndl = dot(n, Ll);
        let glow = (1.0 - smoothstep(0.0, 0.6, inside));
        p += vec3(1.0) * (0.10 * (1.0 - ty) + 0.14 * glow * (0.5 + 0.5 * max(ndl, 0.0)));
        p += vec3(1.0) * pow(max(ndl, 0.0), 2.0) * (1.0 - smoothstep(0.0, 0.18, inside)) * 0.35 * pn.fx.z;
        p -= vec3(0.05) * ty * inside;
        return over_rgb(acc, p, clamp(c0, 0.0, 1.0));
    }
    // a faint dark outline, and a soft bright band just inside it on the lit side: the slab's thickness
    let L = normalize(vec2(-0.6, -0.8));
    let nd = dot(n, L);
    let band = smoothstep(0.0, 0.2, inside) * (1.0 - smoothstep(0.2, 0.7, inside));
    p += vec3(1.0) * band * (0.26 * max(nd, 0.0) + 0.08) * pn.fx.z;
    p -= vec3(0.08) * (1.0 - smoothstep(0.0, 0.12, inside)) * pn.fx.z;
    return over_rgb(acc, p, clamp(c0, 0.0, 1.0));
}

fn compose(px: vec2<f32>, start: vec4<f32>) -> vec4<f32> {
    var acc = start;
    if (g.grid.w > 0.5) {
        // binned: this pixel's tile lists the panels that can touch it (built on the CPU per draw)
        let tiles_x = u32(g.grid.w);
        let tiles_y = g.flags.w;
        let tp = (px - g.grid.xy) / g.grid.z;
        let tx = u32(clamp(floor(tp.x), 0.0, f32(tiles_x) - 1.0));
        let ty = u32(clamp(floor(tp.y), 0.0, f32(tiles_y) - 1.0));
        let t = ty * tiles_x + tx;
        let first = tiles[t];
        let last = tiles[t + 1u];
        let base = tiles_x * tiles_y + 1u;
        for (var j = first; j < last; j = j + 1u) {
            acc = apply_panel(tiles[base + j], px, acc);
        }
    } else {
        for (var i = 0u; i < g.count; i = i + 1u) {
            acc = apply_panel(i, px, acc);
        }
    }
    return acc;
}


// Mip-chain downsample: one pass per level; `bg` is the previous level (a single-mip view), g.res = this level's size.
@fragment
fn fs_down(in: VOut) -> @location(0) vec4<f32> {
    let uv = in.px / g.res;
    let t = 1.0 / (2.0 * g.res);
    var c = textureSampleLevel(bg, bgs, uv, 0.0).rgb * 4.0;
    c += textureSampleLevel(bg, bgs, uv + vec2(t.x, t.y), 0.0).rgb;
    c += textureSampleLevel(bg, bgs, uv + vec2(-t.x, t.y), 0.0).rgb;
    c += textureSampleLevel(bg, bgs, uv + vec2(t.x, -t.y), 0.0).rgb;
    c += textureSampleLevel(bg, bgs, uv + vec2(-t.x, -t.y), 0.0).rgb;
    return vec4(c / 8.0, 1.0);
}

// Shell pass: panels over the opaque backdrop (the scene texture), result is opaque.
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let r = compose(in.px, vec4(sample_bg(in.px), 1.0));
    return vec4(r.rgb, 1.0);
}

// The scene as it is, where no shell panel is: `fs` with no panels gives the same, at a fraction of the cost.
@fragment
fn fs_copy(in: VOut) -> @location(0) vec4<f32> {
    return vec4(sample_bg(in.px), 1.0);
}

// Genie (minimise to the Dock): maps a destination pixel back to the window pixel that lands there.
// Lines near the bottom of the window reach the icon first; each line is squeezed towards the icon's width.
// Returns (source x, source y, valid).
fn genie_ease(v: f32, t: f32) -> f32 {
    let lag = (1.0 - clamp(v, 0.0, 1.0)) * 0.5;
    return smoothstep(0.0, 1.0, clamp((t - lag) / 0.5, 0.0, 1.0));
}

fn genie_y(v: f32, t: f32) -> f32 {
    let ys = g.warp.y + v * g.warp.w;
    return ys + (g.genie.z - ys) * genie_ease(v, t);
}

fn unwarp(px: vec2<f32>) -> vec3<f32> {
    let t = g.genie.x;
    if (t < -0.5) {
        // jelly: points far from where the window is held lag behind by `off`; found by iterating the forward map
        let off = g.genie.yz;
        let held = vec2(g.warp.x + g.genie.w * g.warp.z, g.warp.y);
        let far = max(length(vec2(g.warp.z, g.warp.w)), 1.0);
        var q = px;
        for (var i = 0; i < 4; i = i + 1) {
            let d = clamp(length(q - held) / far, 0.0, 1.0);
            let wgt = d * d * (3.0 - 2.0 * d);
            q = px - off * wgt;
        }
        return vec3(q, 1.0);
    }
    if (t <= 0.0001) { return vec3(px, 1.0); }
    let margin = 100.0 / max(g.warp.w, 1.0);
    var lo = -margin;
    var hi = 1.0 + margin;
    for (var i = 0; i < 14; i = i + 1) {
        let mid = (lo + hi) * 0.5;
        if (genie_y(mid, t) < px.y) { lo = mid; } else { hi = mid; }
    }
    let v = (lo + hi) * 0.5;
    if (abs(genie_y(v, t) - px.y) > 2.0 || v < -margin + 0.001 || v > 1.0 + margin - 0.001) {
        return vec3(0.0, 0.0, 0.0);
    }
    let e = genie_ease(v, t);
    let cx0 = g.warp.x + g.warp.z * 0.5;
    let cx = cx0 + (g.genie.y - cx0) * e;
    let s = mix(1.0, g.genie.w / max(g.warp.z, 1.0), e);
    let sx = cx0 + (px.x - cx) / max(s, 0.001);
    return vec3(sx, g.warp.y + v * g.warp.w, 1.0);
}

// Window frame pass: panels over transparent, premultiplied output, blended onto the scene by the pipeline.
@fragment
fn fs_frame(in: VOut) -> @location(0) vec4<f32> {
    let q = unwarp(in.px);
    if (q.z < 0.5) { return vec4(0.0); }
    return compose(q.xy, vec4(0.0)) * g.fade.x;
}

// One client surface (window, subsurface or popup). panels[0] describes it:
//   rect = quad (centre, half size, logical px), params.x = clip corner radius,
//   params.y bit0 = opaque (ignore alpha), bit1 = frosted backdrop behind translucent pixels, params.z = veil (0 = none),
//   extra.x = 1 when clipped to the rounded rect in color2 (centre, half size)
@fragment
fn fs_surface(in: VOut) -> @location(0) vec4<f32> {
    let pn = g.panels[0];
    let q3 = unwarp(in.px);
    if (q3.z < 0.5) { return vec4(0.0); }
    let q = q3.xy;
    let u = (q - pn.rect.xy) / (2.0 * pn.rect.zw) + vec2(0.5);
    if (u.x < 0.0 || u.x > 1.0 || u.y < 0.0 || u.y > 1.0) {
        return vec4(0.0);
    }
    let flags = u32(pn.params.y + 0.5);
    var s = textureSampleLevel(surf, bgs, u, 0.0);
    if ((flags & 1u) != 0u) { s.a = 1.0; }
    // a rule asks for only part of the picture (params.z, 0 = all of it): the glass behind shows through the rest
    if (pn.params.z > 0.0) { s = s * pn.params.z; }
    var cov = 1.0;
    if (pn.extra.x > 0.5) {
        let sd = sd_round_rect(q - pn.color2.xy, pn.color2.zw, pn.params.x);
        cov = clamp(0.5 - sd * g.scale, 0.0, 1.0);
    }
    var rgb = s.rgb;
    var a = s.a;
    if ((flags & 2u) != 0u && a < 1.0) {
        // translucent client: frosted glass behind it (blurred wallpaper), like macOS vibrancy. Only where the program
        // painted something: the empty margin round a popup (room for its shadow) stays clear instead of becoming a
        // coloured card, and a faint shadow gets only a faint frost
        let behind = saturate_col(blur_bg(q, 18.0), 1.2) * 0.97 + vec3(0.01);
        let k = smoothstep(0.03, 0.55, a);
        rgb = s.rgb + behind * (1.0 - a) * k;
        a = a + (1.0 - a) * k;
    }
    return vec4(rgb * cov, a * cov) * g.fade.x;
}

// Wallpaper bake pass: the bound texture is the source image (cover-fit), or procedural fallback.
@fragment
fn fs_wall(in: VOut) -> @location(0) vec4<f32> {
    let uv0 = in.px / g.res;
    if (g.flags.x == 0u) {
        return vec4(wallpaper(uv0), 1.0);
    }
    let d = vec2<f32>(textureDimensions(bg));
    let a = g.res.x / g.res.y;
    let ia = d.x / d.y;
    var sc = vec2(1.0, ia / a);
    if (a < ia) { sc = vec2(a / ia, 1.0); }
    let uv = (uv0 - vec2(0.5)) * sc + vec2(0.5);
    return vec4(textureSampleLevel(bg, bgs, uv, 0.0).rgb, 1.0);
}
