//! Camera Raw's per-pixel develop on the GPU, for the dialog's interactive preview (#6916).
//!
//! One fragment pass (`camera_raw.wgsl`) runs the stages that read only their own pixel: white
//! balance, Exposure, Contrast without Texture or Clarity, Whites and Blacks, Vibrance and
//! Saturation (with their measured colour correction), the tone curves, the Color Mixer, Color Grading, the vignette and grain. Their
//! constants and tables come from [`photocraft_algo::camera_raw::per_pixel`], which the CPU
//! develop builds its own from, so both evaluate the same maths; the curves and the Whites and
//! Blacks rows are rows of one `R32Float` texture. Settings with a spatial stage (Dehaze, Shadows
//! and Highlights, Texture, Clarity, Detail) have no [`PerPixel`] and stay on the CPU.
//!
//! The pipeline is built on demand ([`CameraRawDevelop::new`]), not with the compositor (#6419).
//! It renders into `Rgba32Float`; [`CameraRawDevelop::supported`] says whether the adapter can.

use photocraft_algo::camera_raw::{
    BalanceStep, CURVE_LUT, FIX_GRID, MIXER_GRID, MIXER_KINDS, PRESENCE_GRID, PRESENCE_KINDS, PerPixel, RowOn, TONE_STEPS, VignetteStyle,
};

use crate::Unsupported;

/// The pass's WGSL source.
pub const CAMERA_RAW_SHADER: &str = include_str!("camera_raw.wgsl");

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// `Params` in the shader: 20 rows of 16 bytes.
const PARAMS_SIZE: u64 = 20 * 16;
/// Rows of the table texture: parametric, master, red, green, blue curves; Whites, Blacks;
/// Exposure, Contrast; then from [`WHITES_FIX_ROW`] Whites down's correction grid, from
/// [`MIXER_ROW`] the Color Mixer tables, from [`PRESENCE_ROW`] Saturation's and Vibrance's, and
/// from [`BALANCE_ROW`] the two white-balance steps.
const LUT_ROWS: u32 = BALANCE_ROW + 2 * BALANCE_ROWS;
const LUT_WIDTH: u32 = CURVE_LUT as u32;
/// Values of Whites down's correction grid (RGB per node) and the rows they take.
const FIX_VALUES: usize = FIX_GRID * FIX_GRID * FIX_GRID * 3;
const FIX_ROWS: u32 = FIX_VALUES.div_ceil(CURVE_LUT) as u32;
/// The first row of Whites down's correction grid (`WHITES_FIX_ROW` in the shader).
const WHITES_FIX_ROW: u32 = 9;
/// The first row of the Color Mixer tables (`MIXER_ROW` in the shader), their values (Luminance,
/// Saturation and Hue tables of hue shift, saturation and log-value change per node) and the
/// rows they take.
const MIXER_ROW: u32 = WHITES_FIX_ROW + FIX_ROWS;
const MIXER_VALUES: usize = MIXER_KINDS * MIXER_GRID[0] * MIXER_GRID[1] * MIXER_GRID[2] * 3;
const MIXER_ROWS: u32 = MIXER_VALUES.div_ceil(CURVE_LUT) as u32;
/// The first row of Saturation's and Vibrance's tables (`PRESENCE_ROW` in the shader), their
/// values and the rows they take.
const PRESENCE_ROW: u32 = MIXER_ROW + MIXER_ROWS;
const PRESENCE_VALUES: usize = PRESENCE_KINDS * PRESENCE_GRID[0] * PRESENCE_GRID[1] * PRESENCE_GRID[2] * 3;
const PRESENCE_ROWS: u32 = PRESENCE_VALUES.div_ceil(CURVE_LUT) as u32;
/// The first row of the white-balance steps (`BALANCE_ROW` in the shader): per step (Temperature,
/// then Tint) the red, green and blue curves, then its correction grid.
const BALANCE_ROW: u32 = PRESENCE_ROW + PRESENCE_ROWS;
const BALANCE_ROWS: u32 = 3 + FIX_ROWS;
const BYTES_PER_PIXEL: u32 = 16;

struct Image {
    key: Option<u64>,
    size: [u32; 2],
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Image {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2], usage: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Image { key: None, size, texture, view }
    }
}

/// The Camera Raw preview pass and its textures (reused while the size stays).
pub struct CameraRawDevelop {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    luts: wgpu::Texture,
    luts_view: wgpu::TextureView,
    source: Option<Image>,
    target: Option<Image>,
    #[cfg(not(target_arch = "wasm32"))]
    readback: Option<wgpu::Buffer>,
    /// The spatial stages' passes, built the first time settings need them.
    spatial: Option<crate::camera_raw_spatial::Spatial>,
}

impl CameraRawDevelop {
    /// Whether `adapter` can render the pass's `Rgba32Float` output.
    pub fn supported(adapter: &wgpu::Adapter) -> bool {
        adapter.get_texture_format_features(FORMAT).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    }

    /// Builds the pipeline. A shader or pipeline the driver can't build is an `Err`, so the
    /// caller keeps the CPU develop (where errors only arrive asynchronously, on WebGPU, creation
    /// is assumed to have worked).
    pub fn new(device: &wgpu::Device) -> Result<Self, Unsupported> {
        match crate::first_error(device, || Self::build(device)) {
            (dev, None) => Ok(dev),
            (_, Some(e)) => Err(Unsupported(format!("couldn't build the RAW Develop pipeline: {e}"))),
        }
    }

    fn build(device: &wgpu::Device) -> Self {
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_camera_raw"), source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                crate::tex_entry(1),
                crate::tex_entry(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pc_camera_raw"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_params"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let luts = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pc_camera_raw_luts"),
            size: wgpu::Extent3d { width: LUT_WIDTH, height: LUT_ROWS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let luts_view = luts.create_view(&Default::default());
        CameraRawDevelop {
            pipeline,
            layout,
            params,
            luts,
            luts_view,
            source: None,
            target: None,
            #[cfg(not(target_arch = "wasm32"))]
            readback: None,
            spatial: None,
        }
    }

    /// Records the develop of `source` (`w × h` straight RGBA, display-encoded) with `stages`
    /// into `encoder` and returns the `Rgba32Float` texture it renders. The source is uploaded
    /// unless the last one had the same `key` and size (`None` always uploads).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<&wgpu::Texture, Unsupported> {
        self.upload(device, queue, source, key, size)?;
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            self.target = Some(Image::new(device, "pc_camera_raw_target", size, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC));
        }
        let (Some(src), Some(target)) = (self.source.as_ref(), self.target.as_ref()) else { return Err(Unsupported("no RAW Develop target".into())) };
        record_pass(self, device, queue, encoder, &src.view, &target.view, size, stages);
        Ok(&target.texture)
    }

    /// Uploads `source` as the pass's source texture, unless the last one had the same `key` and
    /// size (`None` always uploads).
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
    ) -> Result<(), Unsupported> {
        check_size(device, source, size)?;
        let [w, h] = size;
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let src = match self.source.take() {
            Some(s) if s.size == size => s,
            _ => Image::new(device, "pc_camera_raw_source", size, usage),
        };
        let src = self.source.insert(src);
        if key.is_none() || src.key != key {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &src.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(source),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * BYTES_PER_PIXEL), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            src.key = key;
        }
        Ok(())
    }

    /// Builds the spatial stages' passes if they aren't yet.
    pub(crate) fn ensure_spatial(&mut self, device: &wgpu::Device) {
        if self.spatial.is_none() {
            self.spatial = Some(crate::camera_raw_spatial::Spatial::new(device));
        }
    }

    /// The spatial stages' passes, once built.
    pub(crate) fn spatial_passes(&self) -> Option<&crate::camera_raw_spatial::Spatial> {
        self.spatial.as_ref()
    }

    /// Drops the cached source, so a half-done upload isn't taken for it next time.
    pub(crate) fn forget_source(&mut self) {
        self.source = None;
    }

    /// The uploaded source's view.
    pub(crate) fn source_view(&self) -> Option<&wgpu::TextureView> {
        self.source.as_ref().map(|s| &s.view)
    }

    /// Records the per-pixel pass with `stages` from `input` into `target` (both `size`). The
    /// settings and tables are written to the queue now, so a pass recorded later in the same
    /// submission would overwrite them: each submission records one per-pixel pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        target: &wgpu::TextureView,
        size: [u32; 2],
        stages: &PerPixel,
    ) {
        record_pass(self, device, queue, encoder, input, target, size, stages);
    }

    /// Develops `source` and reads the result back (blocking; native only, the web has no
    /// blocking wait). GPU errors and a lost device are `Err`; `health`, when the device is
    /// watched, records a loss.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn develop(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<Vec<[f32; 4]>, Unsupported> {
        if let Some(f) = health.and_then(crate::DeviceHealth::fault) {
            return Err(Unsupported(f.to_string()));
        }
        let (result, error) = crate::first_error(device, || self.develop_inner(device, queue, health, source, key, size, stages));
        match error {
            Some(e) => {
                // A half-done upload must not be taken for the cached source next time.
                self.source = None;
                Err(Unsupported(format!("GPU RAW Develop failed: {e}")))
            }
            None => result,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    fn develop_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<Vec<[f32; 4]>, Unsupported> {
        check_size(device, source, size)?;
        let [w, h] = size;
        let row = (w * BYTES_PER_PIXEL).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes = u64::from(row) * u64::from(h);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw") });
        self.encode(device, queue, &mut encoder, source, key, size, stages)?;
        let Some(target) = self.target.as_ref().map(|t| &t.texture) else { return Err(Unsupported("no RAW Develop target".into())) };
        let buffer = match self.readback.take() {
            Some(b) if b.size() == bytes => b,
            _ => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_camera_raw_readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let index = queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        wait(device, health, index)?;
        let mut out = Vec::with_capacity(source.len());
        {
            let data = buffer.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU readback failed: {e:?}")))?;
            let width = w as usize * BYTES_PER_PIXEL as usize;
            for y in 0..h as usize {
                let start = y * row as usize;
                let line = data.get(start..start + width).ok_or_else(|| Unsupported("short GPU readback".into()))?;
                out.extend(bytemuck::pod_collect_to_vec::<u8, [f32; 4]>(line));
            }
        }
        buffer.unmap();
        self.readback = Some(buffer);
        Ok(out)
    }
}

/// The per-pixel pass with `stages` from `input` into `target` (see [`CameraRawDevelop::record`]).
#[allow(clippy::too_many_arguments)]
fn record_pass(
    dev: &CameraRawDevelop,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    input: &wgpu::TextureView,
    target: &wgpu::TextureView,
    size: [u32; 2],
    stages: &PerPixel,
) {
    queue.write_buffer(&dev.params, 0, &params(size, stages));
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &dev.luts, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytemuck::cast_slice(&tables(stages)),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(LUT_WIDTH * 4), rows_per_image: Some(LUT_ROWS) },
        wgpu::Extent3d { width: LUT_WIDTH, height: LUT_ROWS, depth_or_array_layers: 1 },
    );
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_camera_raw"),
        layout: &dev.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: dev.params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(input) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&dev.luts_view) },
        ],
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pc_camera_raw"),
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
    pass.set_pipeline(&dev.pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}

/// Waits for submission `index`; a lost device (recorded in `health`, when watched) or a failed
/// wait is an `Err`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn wait(device: &wgpu::Device, health: Option<&crate::DeviceHealth>, index: wgpu::SubmissionIndex) -> Result<(), Unsupported> {
    match health {
        Some(hl) => {
            if !hl.wait(device, Some(index)) {
                return Err(Unsupported(hl.fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string())));
            }
        }
        None => {
            device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).map_err(|e| Unsupported(format!("GPU wait failed: {e}")))?;
        }
    }
    Ok(())
}

fn check_size(device: &wgpu::Device, source: &[[f32; 4]], [w, h]: [u32; 2]) -> Result<(), Unsupported> {
    let max = device.limits().max_texture_dimension_2d;
    if w == 0 || h == 0 || w > max || h > max {
        return Err(Unsupported(format!("a {w}×{h} RAW Develop preview doesn't fit the GPU's {max} px textures")));
    }
    if source.len() as u64 != u64::from(w) * u64::from(h) {
        return Err(Unsupported(format!("{} RAW Develop preview pixels for {w}×{h}", source.len())));
    }
    Ok(())
}

/// Flags of `Params.flags`, in the shader's bit order.
fn flags(s: &PerPixel) -> u32 {
    let wb = s.whites_blacks.as_ref();
    let on = [
        // 0: unused (was the white-balance gains).
        false,
        s.contrast.is_some(),
        wb.is_some_and(|w| w.whites.is_some()),
        wb.is_some_and(|w| w.blacks.is_some()),
        wb.is_some_and(|w| w.whites.as_ref().is_some_and(|r| r.1 == RowOn::Channels)),
        s.presence.as_ref().is_some_and(|t| t.len() * 3 == PRESENCE_VALUES),
        s.parametric.is_some(),
        s.point[0].is_some(),
        s.point[1].is_some(),
        s.point[2].is_some(),
        s.point[3].is_some(),
        s.hsl.as_ref().is_some_and(|t| t.len() * 3 == MIXER_VALUES),
        s.grade.is_some(),
        s.vignette.is_some(),
        s.grain.is_some(),
        s.clamp,
        s.exposure.is_some(),
        wb.is_some_and(|w| w.blacks.as_ref().is_some_and(|r| r.1 == RowOn::Channels)),
        // 18: unused (was Vibrance's and Saturation's correction grid).
        false,
        wb.is_some_and(|w| w.whites.is_some() && w.whites_fix.as_ref().is_some_and(|g| g.len() * 3 == FIX_VALUES)),
        // 20–23: the white-balance steps and their correction grids.
        s.white_balance.first().is_some(),
        s.white_balance.first().is_some_and(fix_fits),
        s.white_balance.get(1).is_some(),
        s.white_balance.get(1).is_some_and(fix_fits),
        // 24, 25: Calibration's primaries and Shadows › Tint.
        s.calibration.is_some_and(|c| c.primaries.is_some()),
        s.calibration.is_some_and(|c| c.tint != 0.0),
    ];
    on.iter().enumerate().fold(0, |f, (bit, on)| if *on { f | 1 << bit } else { f })
}

/// A white-balance step's correction grid that fills its rows.
fn fix_fits(step: &BalanceStep) -> bool {
    step.fix.as_ref().is_some_and(|g| g.len() * 3 == FIX_VALUES)
}

/// The shader's `Params` for a `size` image.
fn params(size: [u32; 2], s: &PerPixel) -> Vec<u8> {
    let grade = s.grade.as_ref();
    let vig = s.vignette.as_ref();
    let grain = s.grain.as_ref();
    let mut out: Vec<u8> = Vec::with_capacity(PARAMS_SIZE as usize);
    out.extend([size[0], size[1], flags(s)].iter().flat_map(|v| v.to_le_bytes()).chain(s.encoding.to_le_bytes()));
    let mut f = |v: [f32; 4]| out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    // Shadows › Tint (formerly the white-balance gains' slot).
    f([s.calibration.map_or(0.0, |c| c.tint), 0.0, 0.0, 0.0]);
    for m in [s.to_prophoto, s.from_prophoto] {
        for r in m {
            f([r[0], r[1], r[2], 0.0]);
        }
    }
    for k in 0..4 {
        let (t, l) = grade.map_or(([0.0; 3], 0.0), |gr| (gr.tints[k], gr.lums[k]));
        f([t[0], t[1], t[2], l]);
    }
    f(grade.map_or([0.0; 4], |gr| [gr.blend, gr.balance, 0.0, 0.0]));
    let style = |v: VignetteStyle| match v {
        VignetteStyle::HighlightPriority => 0.0,
        VignetteStyle::ColorPriority => 1.0,
        VignetteStyle::PaintOverlay => 2.0,
    };
    f(vig.map_or([0.0, 0.0, 0.0, 2.0], |v| [v.amount, v.highlights, style(v.style), v.power]));
    f(vig.map_or([0.0, 0.0, 1.0, 1.0], |v| [v.centre[0], v.centre[1], v.axes[0], v.axes[1]]));
    f(vig.map_or([0.0; 4], |v| [v.edges[0], v.edges[1], 0.0, 0.0]));
    f(grain.map_or([1.0, 0.0, 0.0, 1.0], |gr| [gr.cell, gr.roughness, gr.amount, gr.scale]));
    // The seed and the scale were resolved from the job's `DevelopInputs` when `per_pixel` built the grain.
    let seed = grain.map_or(0, |gr| gr.seed);
    let fine = seed.wrapping_add(7);
    out.extend([seed as u32, (seed >> 32) as u32, fine as u32, (fine >> 32) as u32].iter().flat_map(|v| v.to_le_bytes()));
    let (hue, sat) = s.calibration.and_then(|c| c.primaries).unwrap_or(([0.0; 3], [0.0; 3]));
    for v in [hue, sat] {
        out.extend([v[0], v[1], v[2], 0.0].iter().flat_map(|x| x.to_le_bytes()));
    }
    out
}

/// The table texture's rows (see [`LUT_ROWS`]); rows of stages that are off stay zero.
fn tables(s: &PerPixel) -> Vec<f32> {
    let mut t = vec![0.0f32; (LUT_WIDTH * LUT_ROWS) as usize];
    let mut put = |row: usize, v: &[f32]| {
        let start = row * LUT_WIDTH as usize;
        let n = v.len().min(LUT_WIDTH as usize);
        if let (Some(dst), Some(src)) = (t.get_mut(start..start + n), v.get(..n)) {
            dst.copy_from_slice(src);
        }
    };
    if let Some(l) = &s.parametric {
        put(0, l);
    }
    for (k, l) in s.point.iter().enumerate() {
        if let Some(l) = l {
            put(1 + k, l);
        }
    }
    if let Some(r) = &s.exposure {
        put(7, r);
    }
    if let Some(r) = &s.contrast {
        put(8, r);
    }
    if let Some(wb) = &s.whites_blacks {
        if let Some((r, _)) = &wb.whites {
            put(5, r);
        }
        if let Some((r, _)) = &wb.blacks {
            put(6, r);
        }
    }
    let mut balance_fix = [None, None];
    for (k, step) in s.white_balance.iter().take(2).enumerate() {
        let row = (BALANCE_ROW + k as u32 * BALANCE_ROWS) as usize;
        for (c, r) in step.rows.iter().enumerate() {
            put(row + c, r);
        }
        balance_fix[k] = step.fix.as_ref();
    }
    let whites_fix = s.whites_blacks.as_ref().and_then(|w| w.whites_fix.as_ref());
    let grids = [
        (WHITES_FIX_ROW, whites_fix, FIX_VALUES),
        (MIXER_ROW, s.hsl.as_ref(), MIXER_VALUES),
        (PRESENCE_ROW, s.presence.as_ref(), PRESENCE_VALUES),
        (BALANCE_ROW + 3, balance_fix[0], FIX_VALUES),
        (BALANCE_ROW + BALANCE_ROWS + 3, balance_fix[1], FIX_VALUES),
    ];
    for (row, grid, values) in grids {
        if let Some(g) = grid.filter(|g| g.len() * 3 == values) {
            let start = row as usize * LUT_WIDTH as usize;
            if let Some(dst) = t.get_mut(start..start + values) {
                for (d, v) in dst.iter_mut().zip(g.iter().flatten()) {
                    *d = *v;
                }
            }
        }
    }
    t
}

// The Whites and Blacks, Exposure and Contrast rows fit a table row; the shader's grid size is
// the CPU's.
const _: () = assert!(TONE_STEPS <= CURVE_LUT);
const _: () = assert!(FIX_GRID == 17 && WHITES_FIX_ROW == 9);
const _: () = assert!(MIXER_ROW == 24 && MIXER_GRID[0] == 72 && MIXER_GRID[1] == 9 && MIXER_GRID[2] == 9 && MIXER_KINDS == 3);
const _: () = assert!(PRESENCE_ROW == 76 && PRESENCE_GRID[0] == 48 && PRESENCE_GRID[1] == 21 && PRESENCE_GRID[2] == 21 && PRESENCE_KINDS == 2);
const _: () = assert!(BALANCE_ROW == 76 + PRESENCE_ROWS && BALANCE_ROWS == 18);
