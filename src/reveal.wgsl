// Circular reveal between two themes: the old frame is drawn over the new one everywhere outside a growing circle.
struct U {
    center: vec2<f32>,
    radius: f32,
    soft: f32,
    res: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var old_t: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct VOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4(p * 2.0 - vec2(1.0), 0.0, 1.0);
    o.pos.y = -o.pos.y;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if (u.pad.x > 0.5) {
        // fade: radius carries the progress 0 .. 1
        let o = textureSampleLevel(old_t, smp, in.pos.xy / u.res, 0.0).rgb;
        let k = 1.0 - u.radius;
        return vec4(o, k * k * (3.0 - 2.0 * k));
    }
    let d = length(in.pos.xy - u.center);
    let old = textureSampleLevel(old_t, smp, in.pos.xy / u.res, 0.0).rgb;
    let a = clamp((d - u.radius) / u.soft + 0.5, 0.0, 1.0);
    // a bright rim travelling with the edge
    let rim = exp(-pow((d - u.radius) / (u.soft * 2.2), 2.0)) * 0.30 * u.pad.y;
    let col = mix(old, vec3(1.0), rim);
    return vec4(col, max(a, rim * 0.8));
}
