// Supersampling: the frame was drawn `ratio` times larger each way than the screen; this shrinks it onto the target.
// Each output pixel averages a 4 x 4 grid of bilinear taps spread over the square of source pixels it covers, so lines,
// text and edges come out smooth without the shimmer of picking one pixel out of several.
struct U {
    src: vec2<f32>,
    ratio: f32,
    pad: f32,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src_t: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4(p * 2.0 - vec2(1.0), 0.0, 1.0);
    o.pos.y = -o.pos.y;
    o.uv = p;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / u.src;
    var acc = vec4<f32>(0.0);
    for (var j = 0; j < 4; j = j + 1) {
        for (var i = 0; i < 4; i = i + 1) {
            let off = ((vec2(f32(i), f32(j)) + vec2(0.5)) / 4.0 - vec2(0.5)) * u.ratio * texel;
            acc += textureSampleLevel(src_t, smp, in.uv + off, 0.0);
        }
    }
    return acc / 16.0;
}
