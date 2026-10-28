//! The icon renderer on the graphics card. `render` does what `Renderer::render` does on the processor, step for step (the
//! passes are in glass.wgsl): the layers are still rasterised from their SVG here, but placing them, the glass and putting
//! them together are compute passes, all of one icon in one submission, and only the finished picture comes back.
//!
//! Only a dedicated card is used, and only when the GPU preset (gpu.toml) would start a program on it now: an integrated one
//! is slower at this than the processor's cores (Intel UHD of a 12450HX: 42 s for the whole theme, the processor 27 s, an
//! RTX 3050 6 s). The decision is made once, when a program first draws an icon. `MEMRIUM_ICONS_CPU=1` keeps the drawing on
//! the processor.

use super::*;
use std::sync::OnceLock;

/// The parameters of one pass, as glass.wgsl's `Params`.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    m: u32,
    dst: u32,
    src: u32,
    src2: u32,
    src3: u32,
    src4: u32,
    data_off: u32,
    mask_off: u32,
    has_mask: u32,
    pk: u32,
    blend: u32,
    r: u32,
    flag: u32,
    dy: u32,
    at_x: i32,
    at_y: i32,
    _pad: [u32; 3],
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
    top: [f32; 3],
    plate_top: f32,
    bottom: [f32; 3],
    plate_span: f32,
    fade: f32,
    edge_sharp: f32,
    _fpad: [f32; 2],
}

#[derive(Clone, Copy)]
enum Pass {
    Zero,
    Copy,
    Place,
    Plate,
    Transform,
    MaskMul,
    Blur,
    BlurRow,
    Hill,
    BevelSlope,
    Glow,
    Shadow,
    Frost,
    Refract,
    InnerBend,
    PutLayer,
    Bevel,
    Specular,
    Pack,
}

const ENTRIES: [&str; 19] = ["zero", "copy", "place", "plate", "transform", "mask_mul", "blur", "blur_row", "hill", "bevel_slope", "glow", "shadow", "frost", "bend_behind", "inner_bend", "put_layer", "bevel", "specular", "pack"];

// the planes of a render: the layer being put on, two for the blur, the slope of the glass, the slope of the bevel, two for
// what is blurred, then the pictures being drawn on (the icon, and one for each group drawn on its own inside another)
const LAYER: u32 = 0;
const T0: u32 = 1;
const T1: u32 = 2;
const NRM: u32 = 3;
const BEV: u32 = 4;
const B1: u32 = 5;
const B2: u32 = 6;
const ACC: u32 = 7;

struct Gpu {
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
    /// the step between the parameters of two passes in the uniform buffer
    align: u64,
    max_buffer: u64,
}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        if std::env::var_os("MEMRIUM_ICONS_CPU").is_some() {
            return None;
        }
        let made = Gpu::new();
        match &made {
            Ok(g) => eprintln!("[icons] drawing on {}", g.name),
            Err(e) => eprintln!("[icons] drawing on the processor: {e}"),
        }
        made.ok()
    })
    .as_ref()
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        // the dedicated card, when the preset would start a program there now (gpu.toml: on mains power by default)
        if !crate::gpu::wants_discrete(crate::gpu::load_mode(), None, crate::gpu::on_ac()) {
            return Err("the GPU preset keeps the dedicated card off now".into());
        }
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        if info.device_type != wgpu::DeviceType::DiscreteGpu {
            return Err(format!("no dedicated graphics card ({} is slower at this than the processor)", info.name));
        }
        let limits = adapter.limits();
        let required = wgpu::Limits { max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size, max_buffer_size: limits.max_buffer_size, ..wgpu::Limits::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("icons"), required_limits: required.clone(), ..Default::default() })).map_err(|e| e.to_string())?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("icon glass"), source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()) });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("icon glass"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) }, count: None },
                storage(1, false),
                storage(2, false),
                storage(3, true),
                storage(4, false),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("icon glass"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipelines = ENTRIES.iter().map(|e| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(&pl), module: &module, entry_point: Some(e), compilation_options: Default::default(), cache: None })).collect();
        let align = (device.limits().min_uniform_buffer_offset_alignment as u64).max(std::mem::size_of::<Params>() as u64).next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
        Ok(Gpu { name: info.name, max_buffer: (required.max_storage_buffer_binding_size as u64).min(required.max_buffer_size), device, queue, layout, pipelines, align })
    }
}

/// The buffers one renderer draws pictures of one size in (each thread has its own renderer, so its own buffers).
pub struct Ctx {
    n: u32,
    planes_count: u32,
    planes: wgpu::Buffer,
    peaks: wgpu::Buffer,
    data: wgpu::Buffer,
    params: wgpu::Buffer,
    out: wgpu::Buffer,
    read: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl Ctx {
    fn new(g: &Gpu, n: u32, planes_count: u32, data_len: u64, ops: u64, peaks: u64) -> Ctx {
        let d = &g.device;
        let buf = |label, size: u64, usage| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false });
        let px = n as u64 * n as u64;
        let planes = buf("icon planes", px * 16 * planes_count as u64, wgpu::BufferUsages::STORAGE);
        let peaks = buf("icon peaks", peaks.next_power_of_two() * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let data = buf("icon data", data_len.next_power_of_two(), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let params = buf("icon params", ops.next_power_of_two() * g.align, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let out = buf("icon out", px * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let read = buf("icon read", px * 4, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("icon glass"),
            layout: &g.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &params, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) }) },
                wgpu::BindGroupEntry { binding: 1, resource: planes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: peaks.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: out.as_entire_binding() },
            ],
        });
        Ctx { n, planes_count, planes, peaks, data, params, out, read, bind }
    }

    fn fits(&self, g: &Gpu, planes_count: u32, data_len: u64, ops: u64, peaks: u64) -> bool {
        self.planes_count >= planes_count && self.data.size() >= data_len && self.params.size() >= ops * g.align && self.peaks.size() >= peaks * 4
    }
}

/// The passes of one render, written down before anything is sent to the card.
struct Job {
    n: u32,
    ops: Vec<(Pass, Params)>,
    /// rasters (RGBA, 8 bits a channel) and the mask (f32), in u32 words
    data: Vec<u8>,
    peaks: u32,
    mask_off: Option<u32>,
}

impl Job {
    fn base(&self) -> Params {
        Params { n: self.n, has_mask: 0, mask_off: self.mask_off.unwrap_or(0), ..Params::default() }
    }

    fn op(&mut self, pass: Pass, p: Params) {
        self.ops.push((pass, p));
    }

    fn zero(&mut self, dst: u32) {
        let p = Params { dst, ..self.base() };
        self.op(Pass::Zero, p);
    }

    fn copy(&mut self, src: u32, dst: u32) {
        let p = Params { src, dst, ..self.base() };
        self.op(Pass::Copy, p);
    }

    fn words(&self) -> u32 {
        (self.data.len() / 4) as u32
    }

    /// `img` (straight alpha) on plane `dst`, its top left corner at `at`.
    fn place(&mut self, img: &RgbaImage, at: [i32; 2], dst: u32) {
        let off = self.words();
        self.data.extend_from_slice(img.as_raw());
        let p = Params { dst, m: img.width(), data_off: off, at_x: at[0], at_y: at[1], ..self.base() };
        self.op(Pass::Place, p);
    }

    /// `blur` of iconrender.rs: three box passes across and down, from plane `src` into `dst` (neither T0 nor T1).
    fn blur(&mut self, src: u32, dst: u32, sigma: f32) {
        if sigma < 0.3 {
            self.copy(src, dst);
            return;
        }
        let bw = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().round() as u32).max(1) | 1;
        let r = bw / 2;
        // (each pass turns the picture over: across, then down)
        if self.n <= 512 {
            for (s, d) in [(src, T0), (T0, dst)] {
                let p = Params { src: s, dst: d, r, ..self.base() };
                self.op(Pass::BlurRow, p);
            }
            return;
        }
        // (bigger: one box at a time, three times)
        for (s, d) in [(src, T0), (T0, T1), (T1, T0), (T0, T1), (T1, T0), (T0, dst)] {
            let p = Params { src: s, dst: d, r, ..self.base() };
            self.op(Pass::Blur, p);
        }
    }

    /// `composite` of iconrender.rs: the layer in plane LAYER put on plane `acc`, with its glass.
    fn composite(&mut self, acc: u32, p: &Props, masked: bool, tune: &GlassTune) {
        let n = self.n as usize;
        let nf = n as f32;
        let pk = self.peaks;
        self.peaks += 2;
        let g = p.glass;
        let glass = g.enabled;
        let has_mask = (masked && self.mask_off.is_some()) as u32;
        let (lx, ly) = tune.light_dir();
        let sharp = tune.sharp.clamp(0.25, 2.5);
        let sharp01 = ((sharp - 0.25) / 2.25).clamp(0.0, 1.0);
        let unit = nf / 128.0;
        let base = Params {
            dst: acc,
            src: LAYER,
            has_mask,
            pk,
            blend: p.blend.code() as u32,
            opacity: p.opacity,
            see_through: if glass { 1.0 - 0.85 * g.translucency } else { 1.0 },
            glow: g.glow,
            shadow_amount: g.shadow_amount,
            bevel: g.bevel,
            inner_shadow: g.inner_shadow,
            specular: g.specular,
            bend_out: nf * 0.11 * g.refraction,
            bend_in: nf * 0.05 * g.refraction * tune.inner_bend,
            dispersion: tune.dispersion,
            lx,
            ly,
            sharp01,
            edge_line: tune.edge_line,
            soft_light: tune.soft_light,
            edge_sharp: tune.edge_sharp,
            rim: g.specular * tune.rim * 1.3 * p.opacity,
            sheen: g.specular * 0.05 * tune.sheen * p.opacity,
            fade: nf * 0.035,
            ..self.base()
        };
        if has_mask != 0 {
            self.op(Pass::MaskMul, Params { dst: LAYER, ..base });
        }
        let slope = glass && (g.specular > 0.0 || g.refraction > 0.0 || g.inner_shadow > 0.0);
        if slope {
            let bezel = (0.09 * nf * tune.lens_width.clamp(0.2, 3.0)).max(2.0);
            self.blur(LAYER, B1, bezel * 0.45);
            self.op(Pass::Hill, Params { src: B1, dst: NRM, ..base });
        }
        if glass && g.bevel > 0.0 {
            self.blur(LAYER, B1, nf * (0.008 + 0.06 * g.bevel_width.clamp(0.0, 1.0)) / sharp);
            self.op(Pass::BevelSlope, Params { src: B1, dst: BEV, ..base });
        }
        if glass && g.glow > 0.0 {
            self.blur(LAYER, B1, nf * 0.05);
            self.op(Pass::Glow, Params { src: B1, ..base });
        }
        if glass && g.shadow != ShadowKind::None && g.shadow_amount > 0.0 {
            self.blur(LAYER, B1, nf * 0.03 * tune.shadow_soft);
            let dy = (nf * 0.025 * tune.shadow_dist).round() as u32;
            self.op(Pass::Shadow, Params { src: B1, dy, flag: (g.shadow == ShadowKind::Chromatic) as u32, ..base });
        }
        if glass && g.blur > 0.0 {
            self.blur(acc, B1, nf * 0.045 * g.blur);
            self.op(Pass::Frost, Params { src: B1, src2: LAYER, ..base });
        }
        if glass && g.refraction > 0.0 && slope {
            self.copy(acc, B1);
            self.op(Pass::Refract, Params { src: B1, src2: LAYER, src3: NRM, ..base });
            self.copy(LAYER, B1);
            self.op(Pass::InnerBend, Params { src: B1, src3: NRM, dst: LAYER, ..base });
        }
        self.op(Pass::PutLayer, base);
        if glass && (g.bevel > 0.0 || g.inner_shadow > 0.0) {
            if g.inner_shadow > 0.0 {
                self.blur(LAYER, B1, 0.04 * nf * tune.lens_width.clamp(0.2, 3.0));
            }
            let flag = (g.bevel > 0.0) as u32 | (((g.inner_shadow > 0.0) as u32) << 1);
            self.op(Pass::Bevel, Params { src2: BEV, src3: B1, flag, ..base });
        }
        if glass && g.specular > 0.0 && slope {
            self.blur(LAYER, B1, 10.0 * unit.max(0.25));
            self.blur(LAYER, B2, ((1.6 + (0.55 - 1.6) * sharp01) * unit * tune.edge_width).max(0.6));
            self.op(Pass::Specular, Params { src2: NRM, src3: B1, src4: B2, ..base });
        }
    }
}

/// How many pictures a document can need drawn on at once: the icon, and one more for each level of groups.
fn depth(nodes: &[Node]) -> u32 {
    nodes.iter().map(|n| if let Kind::Group(g) = &n.kind { 1 + depth(&g.children) } else { 0 }).max().unwrap_or(0)
}

/// `draw_node` of iconrender.rs, written into `job`; `level` is the plane drawn on (above ACC).
fn draw_node(r: &mut Renderer, job: &mut Job, node: &Node, doc: &IconDoc, app: Appearance, level: u32, place: Place, masked: bool) {
    let mut p = effective_props(node, app);
    if !p.visible {
        return;
    }
    p.glass = if r.flat { primitive_glass(p.glass) } else { r.tune.apply(p.glass) };
    let nf = job.n as f32;
    match &node.kind {
        Kind::Layer(layer) => {
            let k = p.scale * place.scale;
            let m = (nf * k).round().clamp(8.0, 2.0 * nf.max(512.0)) as u32;
            let img = r.raster(layer, &p, doc, app, m);
            let off = [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]];
            job.place(&img, [(nf / 2.0 + off[0] - m as f32 / 2.0).round() as i32, (nf / 2.0 + off[1] - m as f32 / 2.0).round() as i32], LAYER);
            let tune = r.tune;
            job.composite(ACC + level, &p, masked, &tune);
        }
        Kind::Group(g) => {
            let plain = (p.opacity - 1.0).abs() < 1e-4 && p.blend == Blend::Normal && (p.scale - 1.0).abs() < 1e-4 && p.offset == [0.0, 0.0];
            let combined = p.glass_mode == GlassMode::Combined && p.glass.enabled;
            if plain && !combined {
                for c in &g.children {
                    draw_node(r, job, c, doc, app, level, place, masked);
                }
                return;
            }
            let inner = ACC + level + 1;
            job.zero(inner);
            for c in &g.children {
                draw_node(r, job, c, doc, app, level + 1, Place { scale: 1.0, off: [0.0, 0.0] }, false);
            }
            let s = p.scale * place.scale;
            let off = [p.offset[0] * nf * place.scale + place.off[0], p.offset[1] * nf * place.scale + place.off[1]];
            if (s - 1.0).abs() < 1e-4 && off[0].abs() < 1e-3 && off[1].abs() < 1e-3 {
                job.copy(inner, LAYER);
            } else {
                let tp = Params { src: inner, dst: LAYER, scale: s, off_x: off[0], off_y: off[1], ..job.base() };
                job.op(Pass::Transform, tp);
            }
            let own = if combined { p } else { Props { glass: Glass { enabled: false, ..p.glass }, ..p } };
            let tune = r.tune;
            job.composite(ACC + level, &own, masked, &tune);
        }
    }
}

/// `Renderer::render` on the graphics card; None when there is none (or the picture is too big for it), and the processor
/// draws it.
pub(super) fn render(r: &mut Renderer, doc: &IconDoc, app: Appearance, n: u32) -> Option<RgbaImage> {
    let g = gpu()?;
    let planes_count = ACC + 1 + depth(&doc.nodes);
    let px = n as u64 * n as u64;
    if px * 16 * planes_count as u64 > g.max_buffer {
        return None;
    }
    let nn = n as usize;
    let mut job = Job { n, ops: Vec::new(), data: Vec::new(), peaks: 0, mask_off: None };
    let fit = plate_fit();
    let bg = r.plate_for(doc, app);
    if bg.shape != Shape::None {
        let key = (bg.shape.code(), n, r.tune.corner.to_bits());
        let mask = match r.masks.get(&key) {
            Some(m) => m.clone(),
            None => {
                let m = Arc::new(shape_mask(bg.shape, nn, n as f32 * fit / 2.0, r.tune.corner));
                r.masks.insert(key, m.clone());
                m
            }
        };
        job.mask_off = Some(0);
        job.data.extend_from_slice(bytemuck::cast_slice(&mask[..]));
    }
    match &r.backdrop {
        Some(bd) if !r.bare => {
            let img = if bd.width() == n { (**bd).clone() } else { image::imageops::resize(&**bd, n, n, image::imageops::FilterType::Triangle) };
            job.place(&img, [0, 0], ACC);
        }
        _ => job.zero(ACC),
    }
    // the plate (an empty one changes nothing, so it is not put on)
    let (ptop, pbottom) = match bg.fill {
        FillKind::Solid => (bg.top, bg.top),
        FillKind::System if app.is_dark() => (DARK_BASE_TOP, DARK_BASE_BOTTOM),
        FillKind::System => ([1.0, 1.0, 1.0], [0.90, 0.91, 0.94]),
        FillKind::Auto => (bg.top.map(|v| v + (1.0 - v) * 0.22), bg.top.map(|v| v * 0.92)),
        _ => (bg.top, bg.bottom),
    };
    if bg.fill != FillKind::None && !r.bare {
        let pp = Params { dst: LAYER, has_mask: job.mask_off.is_some() as u32, top: ptop, bottom: pbottom, plate_top: n as f32 * (1.0 - fit) / 2.0, plate_span: n as f32 * fit, ..job.base() };
        job.op(Pass::Plate, pp);
        let plate_glass = if r.flat { primitive_glass(bg.glass) } else { r.tune.apply(bg.glass) };
        let tune = r.tune;
        job.composite(ACC, &Props { glass: plate_glass, ..Props::default() }, false, &tune);
    }
    for node in &doc.nodes {
        draw_node(r, &mut job, node, doc, app, 0, Place { scale: fit, off: [0.0, 0.0] }, true);
    }
    let pp = Params { src: ACC, ..job.base() };
    job.op(Pass::Pack, pp);
    run(g, r, job, planes_count)
}

fn run(g: &Gpu, r: &mut Renderer, job: Job, planes_count: u32) -> Option<RgbaImage> {
    let n = job.n;
    let (data_len, ops, peaks) = (job.data.len().max(4) as u64, job.ops.len() as u64, job.peaks.max(2) as u64);
    if !r.gpu.get(&n).is_some_and(|c| c.fits(g, planes_count, data_len, ops, peaks)) {
        // a renderer keeps the buffers of a few sizes (those of the theme), not of every size the editor was zoomed to
        if r.gpu.len() >= 6 {
            r.gpu.clear();
        }
        let old = r.gpu.remove(&n);
        let planes = old.as_ref().map_or(planes_count, |c| c.planes_count.max(planes_count));
        r.gpu.insert(n, Ctx::new(g, n, planes, data_len, ops, peaks));
    }
    let ctx = r.gpu.get(&n)?;
    // one render on the card at a time: the threads draw their layers' SVGs side by side, but several waiting on the card
    // at once spin in the driver and slow everything down
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    debug_assert_eq!(ctx.n, n);
    let mut params = vec![0u8; (ops * g.align) as usize];
    for (k, (_, p)) in job.ops.iter().enumerate() {
        let at = k * g.align as usize;
        params[at..at + std::mem::size_of::<Params>()].copy_from_slice(bytemuck::bytes_of(p));
    }
    g.queue.write_buffer(&ctx.params, 0, &params);
    if !job.data.is_empty() {
        let mut data = job.data;
        data.resize(data.len().next_multiple_of(4), 0);
        g.queue.write_buffer(&ctx.data, 0, &data);
    }
    let mut enc = g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("icon") });
    enc.clear_buffer(&ctx.peaks, 0, None);
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("icon"), timestamp_writes: None });
        let groups = n.div_ceil(8);
        for (k, (which, _)) in job.ops.iter().enumerate() {
            pass.set_pipeline(&g.pipelines[*which as usize]);
            pass.set_bind_group(0, &ctx.bind, &[(k as u64 * g.align) as u32]);
            match which {
                Pass::Blur => pass.dispatch_workgroups(n.div_ceil(64), n, 1),
                Pass::BlurRow => pass.dispatch_workgroups(n, 1, 1),
                _ => pass.dispatch_workgroups(groups, groups, 1),
            }
        }
    }
    let bytes = n as u64 * n as u64 * 4;
    enc.copy_buffer_to_buffer(&ctx.out, 0, &ctx.read, 0, bytes);
    let done = g.queue.submit([enc.finish()]);
    let slice = ctx.read.slice(..bytes);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    let _ = g.device.poll(wgpu::PollType::WaitForSubmissionIndex(done));
    rx.recv().ok()?.ok()?;
    let img = RgbaImage::from_raw(n, n, slice.get_mapped_range().to_vec());
    ctx.read.unmap();
    img
}
