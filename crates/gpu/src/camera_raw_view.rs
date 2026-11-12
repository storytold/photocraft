//! RAW Develop's preview shown from the GPU (#6916): the developed frame converted for the dialog
//! (as `CanvasDisplay::to_rgba8` and egui's `Color32::from_rgba_unmultiplied` do on the CPU) into a
//! texture egui can draw, and its histogram counted (as `RgbHistogram::from_rgba` does), in the
//! develop's command buffer. Only the histogram's 4 KB come back; the developed pixels stay on the
//! GPU in [`CameraRawFrame::pixels`] until something reads them ([`read_camera_raw_pixels`]).
//!
//! Two slots take turns, so the frame on screen is never the one being developed.

use photocraft_algo::histogram::{BINS, RgbHistogram};

use crate::Unsupported;

pub const CAMERA_RAW_VIEW_SHADER: &str = include_str!("camera_raw_view.wgsl");

/// Histogram columns: the bins, each channel's flags, then samples, transparent and invalid.
const COLUMNS: u32 = BINS as u32 + 4;
/// Points a pixel draws (`vs_count`).
const POINTS: u32 = 7;
/// Pixels a histogram row counts: an `Rgba16Float` texel holds every whole number up to 2048.
const ROW_PIXELS: u32 = 2048;
/// Texels a row of the matrix/shaper's curves texture.
const CURVE_WIDTH: usize = 1024;
const UNIFORM_SIZE: u64 = 80;
const DEVELOPED: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// How the dialog shows a frame: `CanvasDisplay` as plain data.
#[derive(Clone, Copy, Debug)]
pub struct CameraRawDisplay<'a> {
    /// The display texture holds `srgb_encode(value)` (a linear document's composite).
    pub encode_srgb: bool,
    /// The display transform after the 8-bit codes, when it is a matrix/shaper (`None` when the
    /// transform is the identity).
    pub shaper: Option<CameraRawShaper<'a>>,
    /// Changes whenever anything above does; the curves upload only then.
    pub key: u64,
}

/// `photocraft_cms::GpuMatrixShaper`'s parts.
#[derive(Clone, Copy, Debug)]
pub struct CameraRawShaper<'a> {
    pub matrix: [[f32; 4]; 3],
    pub input: &'a [[f32; 4]],
    pub output: &'a [[f32; 4]],
}

/// A developed frame on the GPU.
#[derive(Clone, Debug)]
pub struct CameraRawFrame {
    /// Which of the two slots holds it.
    pub slot: usize,
    /// Changes when the slot's textures are made again (a new size).
    pub generation: u64,
    pub size: [u32; 2],
    pub histogram: RgbHistogram,
    /// The developed pixels (straight `Rgba32Float`), for [`read_camera_raw_pixels`].
    pub pixels: wgpu::Texture,
    /// The display texture's `Rgba8UnormSrgb` view: premultiplied sRGB bytes, as egui's own
    /// textures hold them.
    pub shown: wgpu::TextureView,
    /// The display texture itself (`Rgba8Unorm`, read back by tests).
    pub display: wgpu::Texture,
}

struct Slot {
    size: [u32; 2],
    generation: u64,
    pixels: wgpu::Texture,
    display: wgpu::Texture,
    target: wgpu::TextureView,
    shown: wgpu::TextureView,
}

struct Counts {
    rows: u32,
    view: wgpu::TextureView,
}

pub(crate) struct ViewPasses {
    layout: wgpu::BindGroupLayout,
    display: wgpu::RenderPipeline,
    points: wgpu::RenderPipeline,
    sum: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    edges: wgpu::Texture,
    edges_view: wgpu::TextureView,
    /// Whether `edges` holds its values (they upload with the first frame: building has no queue).
    edges_ready: bool,
    curves: Option<(u64, wgpu::TextureView)>,
    dummy: wgpu::TextureView,
    slots: [Option<Slot>; 2],
    generation: u64,
    counts: Option<Counts>,
    sums: wgpu::Texture,
    sums_view: wgpu::TextureView,
    sums_readback: wgpu::Buffer,
}

/// A value's plain 8-bit code on the CPU (`Buffer::to_rgba8`, the histogram's bins).
fn plain_code(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

/// A value's sRGB-encoded 8-bit code on the CPU (`display_color`'s encode table).
fn srgb_code(v: f32) -> u32 {
    let i = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
    u32::from((photocraft_color::convert::linear_to_srgb(i as f32 / 65535.0) * 255.0 + 0.5) as u8)
}

/// For `k` in 1 to 255, the smallest `f32` in 0..=1 whose `code` is at least `k` (index 0 is
/// unused). `code` rises with the value, so a value's code is how many edges it reaches.
fn edges(code: impl Fn(f32) -> u32) -> [f32; 256] {
    let mut out = [0.0f32; 256];
    for (k, edge) in out.iter_mut().enumerate().skip(1) {
        // Non-negative floats order like their bits.
        let (mut lo, mut hi) = (0u32, 1.0f32.to_bits());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if code(f32::from_bits(mid)) >= k as u32 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        *edge = f32::from_bits(lo);
    }
    out
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    views: &[wgpu::TextureFormat],
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: views,
    })
}

fn entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn pass(encoder: &mut wgpu::CommandEncoder, label: &str, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, target: &wgpu::TextureView, vertices: u32) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
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
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..vertices, 0..1);
}

impl ViewPasses {
    pub(crate) fn build(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pc_camera_raw_view"),
            source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_VIEW_SHADER.into()),
        });
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw_view"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: both,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                },
                entry(1, both),
                entry(2, both),
                entry(3, wgpu::ShaderStages::FRAGMENT),
                entry(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw_view"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |vs: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, topology: wgpu::PrimitiveTopology| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), buffers: &[], compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let display = pipeline("vs", "fs_display", wgpu::TextureFormat::Rgba8Unorm, None, wgpu::PrimitiveTopology::TriangleList);
        let points = pipeline(
            "vs_count",
            "fs_point",
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState { color: add, alpha: add }),
            wgpu::PrimitiveTopology::PointList,
        );
        let sum = pipeline("vs", "fs_sum", wgpu::TextureFormat::Rgba32Uint, None, wgpu::PrimitiveTopology::TriangleList);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_view"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edges_texture = texture(
            device,
            "pc_camera_raw_view_edges",
            [256, 2],
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            &[],
        );
        let dummy = texture(device, "pc_camera_raw_view_dummy", [1, 1], DEVELOPED, wgpu::TextureUsages::TEXTURE_BINDING, &[]).create_view(&Default::default());
        let sums = texture(
            device,
            "pc_camera_raw_view_sums",
            [COLUMNS, 1],
            wgpu::TextureFormat::Rgba32Uint,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            &[],
        );
        let sums_view = sums.create_view(&Default::default());
        ViewPasses {
            layout,
            display,
            points,
            sum,
            uniform,
            edges_view: edges_texture.create_view(&Default::default()),
            edges: edges_texture,
            edges_ready: false,
            curves: None,
            dummy,
            slots: [None, None],
            generation: 0,
            counts: None,
            sums,
            sums_view,
            sums_readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_camera_raw_view_sums"),
                size: u64::from(SUMS_ROW),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }

    fn group(&self, device: &wgpu::Device, developed: &wgpu::TextureView, curves: &wgpu::TextureView, counts: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_camera_raw_view"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(developed) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.edges_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(curves) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(counts) },
            ],
        })
    }

    /// Records the display conversion and the histogram of `developed` (`size`, `Rgba32Float`)
    /// into slot `slot`, and the histogram's sums into the readback buffer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        developed: &wgpu::Texture,
        size: [u32; 2],
        display: &CameraRawDisplay,
        slot: usize,
    ) -> Result<(), Unsupported> {
        let [w, h] = size;
        let pixels = u64::from(w) * u64::from(h);
        let rows = u32::try_from(pixels.div_ceil(u64::from(ROW_PIXELS))).unwrap_or(u32::MAX).max(1);
        let vertices =
            u32::try_from(pixels.saturating_mul(u64::from(POINTS))).map_err(|_| Unsupported("the RAW Develop preview is too large to count".into()))?;
        if rows > device.limits().max_texture_dimension_2d || w == 0 || h == 0 {
            return Err(Unsupported(format!("a {w}×{h} RAW Develop preview is too large to count on the GPU")));
        }
        if !self.edges_ready {
            let mut bytes = Vec::with_capacity(256 * 2 * 4);
            for v in edges(plain_code).iter().chain(&edges(srgb_code)) {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.edges, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(2) },
                wgpu::Extent3d { width: 256, height: 2, depth_or_array_layers: 1 },
            );
            self.edges_ready = true;
        }
        let mut flags = u32::from(display.encode_srgb);
        let mut curves = [0u32; 2];
        let mut matrix = [[0.0f32; 4]; 3];
        if let Some(s) = &display.shaper {
            let (inputs, outputs) = (s.input.len(), s.output.len());
            if inputs < 2 || outputs < 2 || !s.input.iter().chain(s.output).flatten().chain(s.matrix.iter().flatten()).all(|v| v.is_finite()) {
                return Err(Unsupported("the display transform's tables aren't usable on the GPU".into()));
            }
            if self.curves.as_ref().is_none_or(|(key, _)| *key != display.key) {
                let count = inputs.saturating_add(outputs);
                let height = count.div_ceil(CURVE_WIDTH);
                let height32 = u32::try_from(height).map_err(|_| Unsupported("display curves too long".into()))?;
                let t = texture(
                    device,
                    "pc_camera_raw_view_curves",
                    [CURVE_WIDTH as u32, height32],
                    DEVELOPED,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    &[],
                );
                let mut bytes = Vec::with_capacity(CURVE_WIDTH * height * 16);
                for v in s.input.iter().chain(s.output).flatten() {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                bytes.resize(CURVE_WIDTH * height * 16, 0);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(CURVE_WIDTH as u32 * 16), rows_per_image: Some(height32) },
                    wgpu::Extent3d { width: CURVE_WIDTH as u32, height: height32, depth_or_array_layers: 1 },
                );
                self.curves = Some((display.key, t.create_view(&Default::default())));
            }
            flags |= 2;
            curves = [u32::try_from(inputs).unwrap_or(u32::MAX), u32::try_from(outputs).unwrap_or(u32::MAX)];
            matrix = s.matrix;
        }
        let mut uniform = Vec::with_capacity(UNIFORM_SIZE as usize);
        for v in [w, h, flags, rows, curves[0], curves[1], 0, 0] {
            uniform.extend_from_slice(&v.to_le_bytes());
        }
        for v in matrix.iter().flatten() {
            uniform.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &uniform);
        // The slot's textures, made again for a new size.
        let index = slot.min(1);
        if self.slots.get(index).and_then(Option::as_ref).is_none_or(|s| s.size != size) {
            self.generation = self.generation.wrapping_add(1);
            let pixels = texture(
                device,
                "pc_camera_raw_view_pixels",
                size,
                DEVELOPED,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                &[],
            );
            let display = texture(
                device,
                "pc_camera_raw_view_display",
                size,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                &[wgpu::TextureFormat::Rgba8UnormSrgb],
            );
            let target = display.create_view(&Default::default());
            let shown = display.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8UnormSrgb), ..Default::default() });
            if let Some(s) = self.slots.get_mut(index) {
                *s = Some(Slot { size, generation: self.generation, pixels, display, target, shown });
            }
        }
        if self.counts.as_ref().is_none_or(|c| c.rows != rows) {
            let texture = texture(
                device,
                "pc_camera_raw_view_counts",
                [COLUMNS, rows],
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                &[],
            );
            self.counts = Some(Counts { rows, view: texture.create_view(&Default::default()) });
        }
        let (Some(s), Some(counts)) = (self.slots.get(index).and_then(Option::as_ref), self.counts.as_ref()) else {
            return Err(Unsupported("no RAW Develop preview slot".into()));
        };
        let developed_view = developed.create_view(&Default::default());
        let curves_view = self.curves.as_ref().filter(|_| flags & 2 != 0).map_or(&self.dummy, |(_, v)| v);
        // The counts target can't be read while it is drawn into: the display and points passes
        // bind a dummy there, the sum pass the counts.
        let group = self.group(device, &developed_view, curves_view, &self.dummy);
        pass(encoder, "pc_camera_raw_view_display", &self.display, &group, &s.target, 3);
        pass(encoder, "pc_camera_raw_view_points", &self.points, &group, &counts.view, vertices);
        let sum_group = self.group(device, &developed_view, curves_view, &counts.view);
        pass(encoder, "pc_camera_raw_view_sum", &self.sum, &sum_group, &self.sums_view, 3);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: developed, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &s.pixels, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.sums, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.sums_readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SUMS_ROW), rows_per_image: Some(1) },
            },
            wgpu::Extent3d { width: COLUMNS, height: 1, depth_or_array_layers: 1 },
        );
        Ok(())
    }

    /// The frame in slot `slot`, its histogram read from the mapped sums (after the submit).
    pub(crate) fn frame(&self, slot: usize) -> Result<CameraRawFrame, Unsupported> {
        let s = self.slots.get(slot.min(1)).and_then(Option::as_ref).ok_or_else(|| Unsupported("no RAW Develop preview slot".into()))?;
        let histogram = {
            let data = self.sums_readback.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU histogram readback failed: {e:?}")))?;
            let sums: Vec<[u32; 4]> = bytemuck::pod_collect_to_vec(data.get(..COLUMNS as usize * 16).ok_or_else(|| Unsupported("short GPU histogram".into()))?);
            histogram(&sums)
        };
        self.sums_readback.unmap();
        Ok(CameraRawFrame {
            slot: slot.min(1),
            generation: s.generation,
            size: s.size,
            histogram: histogram.ok_or_else(|| Unsupported("short GPU histogram".into()))?,
            pixels: s.pixels.clone(),
            shown: s.shown.clone(),
            display: s.display.clone(),
        })
    }

    pub(crate) fn sums_buffer(&self) -> &wgpu::Buffer {
        &self.sums_readback
    }

    /// Forgets the slots and uploads, so the next frame makes them again (after a GPU error).
    pub(crate) fn forget(&mut self) {
        self.slots = [None, None];
        self.curves = None;
        self.edges_ready = false;
    }
}

/// Bytes a row of the sums' readback (aligned for the copy).
const SUMS_ROW: u32 = (COLUMNS * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// The histogram of the sums `fs_sum` writes, column by column.
fn histogram(sums: &[[u32; 4]]) -> Option<RgbHistogram> {
    let mut h = RgbHistogram::default();
    for (c, bins) in h.channels.iter_mut().enumerate() {
        for (bin, count) in bins.iter_mut().enumerate() {
            *count = u64::from(*sums.get(bin)?.get(c)?);
        }
        let flags = sums.get(BINS + c)?;
        h.underflow[c] = u64::from(flags[0]);
        h.overflow[c] = u64::from(flags[1]);
        h.shadows[c] = u64::from(flags[2]);
        h.highlights[c] = u64::from(flags[3]);
    }
    let kinds = sums.get(BINS + 3)?;
    (h.samples, h.transparent, h.invalid) = (u64::from(kinds[0]), u64::from(kinds[1]), u64::from(kinds[2]));
    Some(h)
}

/// Reads `texture` (`Rgba32Float`) back, blocking: the developed pixels of a [`CameraRawFrame`]
/// when something needs them. `buffer` keeps the readback buffer between calls (one is made
/// when it is `None` or another size). GPU errors and a lost device are `Err`.
pub fn read_camera_raw_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    health: Option<&crate::DeviceHealth>,
    texture: &wgpu::Texture,
    buffer: &mut Option<wgpu::Buffer>,
) -> Result<Vec<[f32; 4]>, Unsupported> {
    if let Some(f) = health.and_then(crate::DeviceHealth::fault) {
        return Err(Unsupported(f.to_string()));
    }
    if texture.format() != DEVELOPED {
        return Err(Unsupported("not a developed RAW Develop frame".into()));
    }
    let (result, error) = crate::first_error(device, || {
        let size = [texture.width(), texture.height()];
        let reused = buffer.take().filter(|b| b.size() == crate::camera_raw::readback_bytes(size));
        let out = reused.unwrap_or_else(|| crate::camera_raw::readback_buffer(device, size));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw_view_read") });
        crate::camera_raw::copy_out(&mut encoder, texture, &out, size);
        let index = queue.submit([encoder.finish()]);
        let px = crate::camera_raw::read_out(device, health, &out, index, size);
        (px, out)
    });
    match (error, result) {
        (Some(e), _) => Err(Unsupported(format!("GPU RAW Develop readback failed: {e}"))),
        (None, (px, out)) => {
            // A buffer left mapped by a failed read isn't reused.
            if px.is_ok() {
                *buffer = Some(out);
            }
            px
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every value's code is how many edges it reaches, for both tables: on each side of every
    /// edge, at every 16-bit step and at the ends.
    #[test]
    fn a_values_code_is_the_number_of_edges_it_reaches() {
        for (name, code) in [("plain", plain_code as fn(f32) -> u32), ("sRGB", srgb_code)] {
            let e = edges(code);
            let reached = |v: f32| e.iter().skip(1).filter(|edge| v >= **edge).count() as u32;
            let mut values = vec![0.0, -0.0, -1.0, 1.0, 2.0, f32::MIN_POSITIVE, 1.0 - f32::EPSILON];
            for edge in e.iter().skip(1) {
                values.extend([*edge, f32::from_bits(edge.to_bits() - 1), f32::from_bits(edge.to_bits() + 1)]);
            }
            values.extend((0..=65535u32).map(|i| i as f32 / 65535.0));
            for v in values {
                assert_eq!(reached(v), code(v), "{name} {v:e}");
            }
            assert!(e.windows(2).skip(1).all(|w| w[0] < w[1]), "{name} edges rise");
        }
    }

    /// The sums' columns land in the histogram's fields.
    #[test]
    fn the_sums_fill_the_histogram() {
        let mut sums = vec![[0u32; 4]; COLUMNS as usize];
        sums[0] = [1, 2, 3, 0];
        sums[255] = [4, 5, 6, 0];
        sums[256] = [7, 8, 9, 10];
        sums[258] = [11, 12, 13, 14];
        sums[259] = [15, 16, 17, 0];
        let h = histogram(&sums).unwrap();
        assert_eq!([h.channels[0][0], h.channels[1][0], h.channels[2][0]], [1, 2, 3]);
        assert_eq!([h.channels[0][255], h.channels[1][255], h.channels[2][255]], [4, 5, 6]);
        assert_eq!((h.underflow[0], h.overflow[0], h.shadows[0], h.highlights[0]), (7, 8, 9, 10));
        assert_eq!((h.underflow[2], h.overflow[2], h.shadows[2], h.highlights[2]), (11, 12, 13, 14));
        assert_eq!((h.samples, h.transparent, h.invalid), (15, 16, 17));
        assert!(histogram(&sums[..259]).is_none());
    }
}
