//! Camera Raw's spatial stages in the GPU preview (#6916): Dehaze, Highlights and Shadows, and the
//! tones Whites up adapts to after them.
//!
//! The stages' analyses run on the CPU, with the same code the CPU develop runs
//! (`photocraft_algo::camera_raw`), on reduced copies the GPU makes and reads back: Dehaze's
//! transmission and atmospheric light (a copy at most 768 pixels across), negative Dehaze's haze
//! colour (256), Highlights' and Shadows' moves (384) and the tones (384 cells across). The GPU
//! does the full-size work: the per-pixel stages before and after (`camera_raw.wgsl`), the
//! reductions, and applying each analysis per pixel (`camera_raw_spatial.wgsl`). The readbacks
//! block, so this runs on native targets only.

use photocraft_algo::camera_raw::{
    self, CameraRaw, DehazePlan, DevelopInputs,
    local_tone::{self, ToneMove},
};

use crate::Unsupported;
use crate::camera_raw::CameraRawDevelop;

/// The spatial passes' WGSL source.
pub const CAMERA_RAW_SPATIAL_SHADER: &str = include_str!("camera_raw_spatial.wgsl");

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// `S` in the shader: seven rows of 16 bytes.
const S_SIZE: u64 = 7 * 16;
const HL_ON: u32 = 1;
const HL_EXACT: u32 = 2;
const SH_ON: u32 = 4;
const SH_EXACT: u32 = 8;

/// The shader's entry points.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    ReduceDehaze,
    ReduceHaze,
    ReduceLuma,
    ReduceLumaAfter,
    ReduceCells,
    ApplyDehaze,
    ApplyHaze,
    ApplyTone,
}

const PASSES: [(Pass, &str); 8] = [
    (Pass::ReduceDehaze, "fs_reduce_dehaze"),
    (Pass::ReduceHaze, "fs_reduce_haze"),
    (Pass::ReduceLuma, "fs_reduce_luma"),
    (Pass::ReduceLumaAfter, "fs_reduce_luma_after"),
    (Pass::ReduceCells, "fs_reduce_cells"),
    (Pass::ApplyDehaze, "fs_apply_dehaze"),
    (Pass::ApplyHaze, "fs_apply_haze"),
    (Pass::ApplyTone, "fs_apply_tone"),
];

/// The spatial passes' pipelines.
pub(crate) struct Spatial {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<(Pass, wgpu::RenderPipeline)>,
    /// A 1 × 1 texture bound where a pass reads no grid.
    empty: (wgpu::Texture, wgpu::TextureView),
}

/// One texture and its view.
struct Tex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

impl Tex {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Tex { texture, view, size }
    }

    /// A texture holding `data` (`size`, RGBA per texel).
    fn with(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, size: [u32; 2], data: &[[f32; 4]]) -> Self {
        let t = Tex::new(device, label, size);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * 16), rows_per_image: Some(size[1]) },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        t
    }
}

/// The settings uniform of one pass.
#[derive(Clone, Copy, Default)]
struct Uniform {
    sizes: [u32; 4],
    grid: [u32; 4],
    grid2: [u32; 4],
    k: [f32; 4],
    air: [f32; 4],
    haze: [f32; 4],
    lum: [f32; 4],
}

impl Uniform {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(S_SIZE as usize);
        for v in [self.sizes, self.grid, self.grid2] {
            out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
        }
        for v in [self.k, self.air, self.haze, self.lum] {
            out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
        }
        out
    }
}

impl Spatial {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pc_camera_raw_spatial"),
            source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_SPATIAL_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw_spatial"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(S_SIZE),
                    },
                    count: None,
                },
                crate::tex_entry(1),
                crate::tex_entry(2),
                crate::tex_entry(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw_spatial"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = PASSES
            .iter()
            .map(|(pass, entry)| {
                let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                (*pass, p)
            })
            .collect();
        let empty = Tex::new(device, "pc_camera_raw_spatial_empty", [1, 1]);
        Spatial { layout, pipelines, empty: (empty.texture, empty.view) }
    }

    /// Records `pass` from `src` (and the grids) into `target` with the settings `u`.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pass: Pass,
        u: &Uniform,
        src: &wgpu::TextureView,
        grid_a: Option<&wgpu::TextureView>,
        grid_b: Option<&wgpu::TextureView>,
        target: &wgpu::TextureView,
    ) {
        let Some(pipeline) = self.pipelines.iter().find(|(p, _)| *p == pass).map(|(_, p)| p) else { return };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_spatial_settings"),
            size: S_SIZE,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        if let Ok(mut view) = buffer.slice(..).get_mapped_range_mut() {
            view.copy_from_slice(&u.bytes());
        }
        buffer.unmap();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_camera_raw_spatial"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(grid_a.unwrap_or(&self.empty.1)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(grid_b.unwrap_or(&self.empty.1)) },
            ],
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pc_camera_raw_spatial"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }
}

/// Submits `encoder`, waits, and reads `tex` back (RGBA per texel, row-major).
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    health: Option<&crate::DeviceHealth>,
    mut encoder: wgpu::CommandEncoder,
    tex: &Tex,
) -> Result<Vec<[f32; 4]>, Unsupported> {
    let [w, h] = tex.size;
    let row = (w * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pc_camera_raw_spatial_readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &tex.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let index = queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    crate::camera_raw::wait(device, health, index)?;
    let mut out = Vec::with_capacity(w as usize * h as usize);
    {
        let data = buffer.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU readback failed: {e:?}")))?;
        for y in 0..h as usize {
            let start = y * row as usize;
            let line = data.get(start..start + w as usize * 16).ok_or_else(|| Unsupported("short GPU readback".into()))?;
            out.extend(bytemuck::pod_collect_to_vec::<u8, [f32; 4]>(line));
        }
    }
    buffer.unmap();
    Ok(out)
}

/// The settings split around the spatial stages: those before (white balance, Exposure,
/// Contrast, Calibration) and those after (Whites and Blacks onward); the spatial stages
/// themselves are in neither.
fn split(p: &CameraRaw) -> (CameraRaw, CameraRaw) {
    let mut pre = CameraRaw::default();
    pre.temperature = p.temperature;
    pre.tint = p.tint;
    pre.exposure = p.exposure;
    pre.contrast = p.contrast;
    pre.shadow_tint = p.shadow_tint;
    pre.calibration_hue = p.calibration_hue;
    pre.calibration_sat = p.calibration_sat;
    let mut post = p.clone();
    post.temperature = 0.0;
    post.tint = 0.0;
    post.exposure = 0.0;
    post.contrast = 0.0;
    post.shadow_tint = 0.0;
    post.calibration_hue = [0.0; 3];
    post.calibration_sat = [0.0; 3];
    post.dehaze = 0.0;
    post.highlights = 0.0;
    post.shadows = 0.0;
    post.texture = 0.0;
    post.clarity = 0.0;
    (pre, post)
}

/// Whether the spatial develop handles `p`: it has a spatial stage other than Detail (sharpening,
/// noise reduction), which stays on the CPU. Texture and Clarity develop on the GPU for opaque
/// pixels in sRGB's primaries; elsewhere the develop answers `None` and the CPU develops.
pub fn spatial_supported(p: &CameraRaw) -> bool {
    (p.dehaze != 0.0 || p.highlights != 0.0 || p.shadows != 0.0 || p.texture != 0.0 || p.clarity != 0.0)
        && p.sharpen_amount <= 0.0
        && p.noise_luminance <= 0.0
        && p.noise_color <= 0.0
}

impl CameraRawDevelop {
    /// Develops `source` (`size`, straight RGBA, display-encoded) with settings `p` that have
    /// spatial stages ([`spatial_supported`]) and reads the result back. `Ok(None)` when these
    /// settings must develop on the CPU. Blocking; native only.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn develop_spatial(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        p: &CameraRaw,
        inputs: &DevelopInputs,
        float: bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Unsupported> {
        if let Some(f) = health.and_then(crate::DeviceHealth::fault) {
            return Err(Unsupported(f.to_string()));
        }
        let (result, error) = crate::first_error(device, || self.spatial_inner(device, queue, health, source, key, size, p, inputs, float));
        match error {
            Some(e) => {
                self.forget_source();
                Err(Unsupported(format!("GPU RAW Develop failed: {e}")))
            }
            None => result,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    fn spatial_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        p: &CameraRaw,
        inputs: &DevelopInputs,
        float: bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Unsupported> {
        if !spatial_supported(p) || p.is_identity() {
            return Ok(None);
        }
        let [w, h] = size;
        let (wu, hu) = (w as usize, h as usize);
        if wu.saturating_mul(hu) != source.len() || wu == 0 || hu == 0 {
            return Ok(None);
        }
        let detail = p.texture != 0.0 || p.clarity != 0.0;
        // Texture's and Clarity's GPU passes read opaque pixels in sRGB's primaries (the CPU
        // blurs alpha-weighted planes elsewhere).
        if detail && (!inputs.srgb_primaries() || source.iter().any(|q| q[3].partial_cmp(&1.0).is_none_or(|o| o.is_lt()))) {
            return Ok(None);
        }
        let (pre, post) = split(p);
        // Before: the per-pixel stages up to Calibration, unclipped (develop clips only at the end).
        let points = camera_raw::white_black_points(source, wu, hu, &pre, inputs);
        let Some(pre_stages) = camera_raw::per_pixel(&pre, inputs, wu, hu, true, points) else { return Ok(None) };
        self.upload(device, queue, source, key, size)?;
        self.ensure_spatial(device);
        if detail {
            self.ensure_detail(device);
        }
        let Some(spatial) = self.spatial_passes() else { return Ok(None) };
        let mut cur = Tex::new(device, "pc_camera_raw_work_a", size);
        let mut next = Tex::new(device, "pc_camera_raw_work_b", size);
        {
            let Some(src) = self.source_view() else { return Err(Unsupported("no RAW Develop source".into())) };
            let mut pre_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_pre") });
            self.record(device, queue, &mut pre_encoder, src, &cur.view, size, &pre_stages);
            // Submitted on its own: the after-pass rewrites the same settings and tables.
            queue.submit([pre_encoder.finish()]);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_spatial") });
        let gamma = inputs.encoding_gamma;
        let base = Uniform { sizes: [w, h, w, h], k: [gamma, 0.0, 0.0, 0.0], ..Default::default() };
        // Dehaze.
        if p.dehaze > 0.0 {
            let (k, sw, sh) = camera_raw::dehaze_reduction(wu, hu);
            let small = Tex::new(device, "pc_camera_raw_dehaze_small", [sw as u32, sh as u32]);
            let u = Uniform { sizes: [w, h, sw as u32, sh as u32], grid: [k as u32, 0, 0, 0], ..base };
            spatial.record(device, &mut encoder, Pass::ReduceDehaze, &u, &cur.view, None, None, &small.view);
            let back = read_back(device, queue, health, encoder, &small)?;
            let rgb: Vec<[f32; 3]> = back.iter().map(|q| [q[0], q[1], q[2]]).collect();
            encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_dehaze") });
            if let Some(DehazePlan { t, w: tw, h: th, air }) = camera_raw::dehaze_plan(&rgb, sw, sh) {
                let data: Vec<[f32; 4]> = t.iter().map(|v| [*v, 0.0, 0.0, 0.0]).collect();
                let grid = Tex::with(device, queue, "pc_camera_raw_dehaze_t", [tw as u32, th as u32], &data);
                let u =
                    Uniform { grid: [k as u32, 0, tw as u32, th as u32], k: [gamma, p.dehaze / 100.0, 0.0, 0.0], air: [air[0], air[1], air[2], 0.0], ..base };
                spatial.record(device, &mut encoder, Pass::ApplyDehaze, &u, &cur.view, Some(&grid.view), None, &next.view);
                std::mem::swap(&mut cur, &mut next);
            }
        } else if p.dehaze < 0.0
            && let Some(kq) = camera_raw::add_haze_weights(p.dehaze)
        {
            let (s, sw, sh) = camera_raw::add_haze_reduction(wu, hu);
            let small = Tex::new(device, "pc_camera_raw_haze_small", [sw as u32, sh as u32]);
            let u = Uniform { sizes: [w, h, sw as u32, sh as u32], grid: [s as u32, 0, 0, 0], ..base };
            spatial.record(device, &mut encoder, Pass::ReduceHaze, &u, &cur.view, None, None, &small.view);
            let back = read_back(device, queue, health, encoder, &small)?;
            encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_haze") });
            let sums: Vec<f32> = back.iter().flat_map(|q| [q[0], q[1], q[2]]).collect();
            let counts: Vec<f32> = back.iter().map(|q| q[3]).collect();
            if let Some(air) = camera_raw::add_haze_air(sums, counts, sw, sh) {
                let u = Uniform { air: [air[0], air[1], air[2], 0.0], haze: [kq[0], kq[1], kq[2], 0.0], ..base };
                spatial.record(device, &mut encoder, Pass::ApplyHaze, &u, &cur.view, None, None, &next.view);
                std::mem::swap(&mut cur, &mut next);
            }
        }
        // Highlights, then Shadows on its result.
        if (p.highlights != 0.0 || p.shadows != 0.0) && wu >= 2 && hu >= 2 {
            let (hl, sh) = (p.highlights.clamp(-100.0, 100.0) / 100.0, p.shadows.clamp(-100.0, 100.0) / 100.0);
            let f = local_tone::reduction(wu, hu);
            let (sw, shh) = (wu.div_ceil(f), hu.div_ceil(f));
            let small = Tex::new(device, "pc_camera_raw_tone_small", [sw as u32, shh as u32]);
            let reduce = Uniform { sizes: [w, h, sw as u32, shh as u32], grid: [f as u32, 0, 0, 0], ..base };
            let mut hl_move: Option<(ToneMove, Tex)> = None;
            if hl != 0.0 {
                spatial.record(device, &mut encoder, Pass::ReduceLuma, &reduce, &cur.view, None, None, &small.view);
                let back = read_back(device, queue, health, encoder, &small)?;
                encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_tone") });
                let lum: Vec<f32> = back.iter().map(|q| q[0]).collect();
                hl_move = local_tone::tone_move(&lum, sw, shh, f, hl, true).map(|m| {
                    let t = move_texture(device, queue, &m);
                    (m, t)
                });
            }
            let hl_flags = hl_move.as_ref().map_or(0, |(m, _)| HL_ON | if m.exact { HL_EXACT } else { 0 });
            let hl_grid = hl_move.as_ref().map_or([0, 0], |(m, _)| [m.w as u32, m.h as u32]);
            let mut sh_move: Option<(ToneMove, Tex)> = None;
            if sh != 0.0 {
                let u = Uniform { grid: [f as u32, hl_flags, hl_grid[0], hl_grid[1]], ..reduce };
                let pass = if hl_move.is_some() { Pass::ReduceLumaAfter } else { Pass::ReduceLuma };
                spatial.record(device, &mut encoder, pass, &u, &cur.view, hl_move.as_ref().map(|(_, t)| &t.view), None, &small.view);
                let back = read_back(device, queue, health, encoder, &small)?;
                encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_tone") });
                let lum: Vec<f32> = back.iter().map(|q| q[0]).collect();
                sh_move = local_tone::tone_move(&lum, sw, shh, f, sh, false).map(|m| {
                    let t = move_texture(device, queue, &m);
                    (m, t)
                });
            }
            if hl_move.is_some() || sh_move.is_some() {
                let sh_flags = sh_move.as_ref().map_or(0, |(m, _)| SH_ON | if m.exact { SH_EXACT } else { 0 });
                let sh_grid = sh_move.as_ref().map_or([0, 0], |(m, _)| [m.w as u32, m.h as u32]);
                let u = Uniform {
                    grid: [0, hl_flags | sh_flags, hl_grid[0], hl_grid[1]],
                    grid2: [sh_grid[0], sh_grid[1], 0, 0],
                    k: [gamma, 0.0, local_tone::colour_exponent(true, hl), local_tone::colour_exponent(false, sh)],
                    ..base
                };
                spatial.record(
                    device,
                    &mut encoder,
                    Pass::ApplyTone,
                    &u,
                    &cur.view,
                    hl_move.as_ref().map(|(_, t)| &t.view),
                    sh_move.as_ref().map(|(_, t)| &t.view),
                    &next.view,
                );
                std::mem::swap(&mut cur, &mut next);
            }
        }
        // Texture, then Clarity on its result.
        for (texture, v) in [(true, p.texture), (false, p.clarity)] {
            let (Some(plan), Some(passes)) = (camera_raw::detail_plan(texture, v, wu, hu), self.detail_passes()) else { continue };
            passes.run(device, &mut encoder, &plan, gamma, &cur.view, &next.view, size);
            std::mem::swap(&mut cur, &mut next);
        }
        // Whites up adapts to the tones after the spatial stages.
        let mut tones_w = [0.0f32; 4];
        if post.whites > 0.0 {
            let f = camera_raw::tone_cell(wu, hu);
            let (sw, shh) = (wu.div_ceil(f), hu.div_ceil(f));
            let cells = Tex::new(device, "pc_camera_raw_cells", [sw as u32, shh as u32]);
            let lw = inputs.luminance_weights();
            let u = Uniform { sizes: [w, h, sw as u32, shh as u32], grid: [f as u32, 0, 0, 0], lum: [lw[0], lw[1], lw[2], 0.0], ..base };
            spatial.record(device, &mut encoder, Pass::ReduceCells, &u, &cur.view, None, None, &cells.view);
            let back = read_back(device, queue, health, encoder, &cells)?;
            encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_post") });
            tones_w = camera_raw::tones_from_cells(back.iter().filter(|q| q[1] > 0.0).map(|q| q[0] / q[1]));
        }
        // After: Whites and Blacks onward, clipped at the end as develop clips.
        let Some(mut post_stages) = camera_raw::per_pixel(&post, inputs, wu, hu, float, Some([[0.0; 4], tones_w])) else { return Ok(None) };
        post_stages.clamp = !float;
        let target = Tex::new(device, "pc_camera_raw_spatial_target", size);
        self.record(device, queue, &mut encoder, &cur.view, &target.view, size, &post_stages);
        read_back(device, queue, health, encoder, &target).map(Some)
    }
}

/// A tone move's grid as a texture: `a` and `b` in red and green.
fn move_texture(device: &wgpu::Device, queue: &wgpu::Queue, m: &ToneMove) -> Tex {
    let data: Vec<[f32; 4]> = m.a.iter().zip(&m.b).map(|(a, b)| [*a, *b, 0.0, 0.0]).collect();
    Tex::with(device, queue, "pc_camera_raw_tone_move", [m.w as u32, m.h as u32], &data)
}
