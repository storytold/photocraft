//! Texture and Clarity in Camera Raw's GPU preview (#6916): `camera_raw_detail.wgsl`'s compute
//! passes, run by [`Detail::run`] between Highlights/Shadows and Whites/Blacks. The model is
//! `photocraft_algo::camera_raw::local_detail`'s ([`DetailPlan`]); opaque pixels in sRGB's
//! primaries only (the spatial develop leaves the rest to the CPU).

use photocraft_algo::camera_raw::DetailPlan;

/// The passes' WGSL source.
pub const CAMERA_RAW_DETAIL_SHADER: &str = include_str!("camera_raw_detail.wgsl");

/// `D` in the shader: three rows of 16 bytes.
const D_SIZE: u64 = 3 * 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Planes,
    Copy,
    Rows,
    Cols,
    Moves,
    Apply,
}

const PASSES: [(Pass, &str); 6] = [
    (Pass::Planes, "cs_planes"),
    (Pass::Copy, "cs_copy"),
    (Pass::Rows, "cs_box_rows"),
    (Pass::Cols, "cs_box_cols"),
    (Pass::Moves, "cs_moves"),
    (Pass::Apply, "cs_apply"),
];

/// The compute pipelines.
pub(crate) struct Detail {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<(Pass, wgpu::ComputePipeline)>,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

/// The per-pixel buffers of one run.
struct Buffers {
    planes: wgpu::Buffer,
    knots: wgpu::Buffer,
    moves: wgpu::Buffer,
    blur: [wgpu::Buffer; 2],
}

impl Detail {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pc_camera_raw_detail"),
            source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_DETAIL_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw_detail"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(D_SIZE),
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, true),
                storage(6, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw_detail"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = PASSES
            .iter()
            .map(|(pass, entry)| {
                let p = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                });
                (*pass, p)
            })
            .collect();
        Detail { layout, pipelines }
    }

    /// Records Texture's or Clarity's passes (`plan`) from `src` into `dst` (both `size`;
    /// `dst` a storage texture) for a document with transfer curve `gamma` (0: sRGB's).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        plan: &DetailPlan,
        gamma: f32,
        src: &wgpu::TextureView,
        dst: &wgpu::TextureView,
        size: [u32; 2],
    ) {
        let [w, h] = size;
        let n = u64::from(w) * u64::from(h) * 16;
        let buffer = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: n.max(16), usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
        };
        let bufs = Buffers {
            planes: buffer("pc_camera_raw_detail_planes"),
            knots: buffer("pc_camera_raw_detail_knots"),
            moves: buffer("pc_camera_raw_detail_moves"),
            blur: [buffer("pc_camera_raw_detail_blur_a"), buffer("pc_camera_raw_detail_blur_b")],
        };
        let coeffs = init_buffer(
            device,
            "pc_camera_raw_detail_coeffs",
            wgpu::BufferUsages::STORAGE,
            &plan.coeffs.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
        );
        let scales = plan.radii.len() as u32;
        let amps = plan.amps.len().min(2) as u32;
        let amp = |i: usize| plan.amps.get(i).copied().unwrap_or(0.0);
        let uniform = |radius: u32, scale: u32| {
            let mut b = Vec::with_capacity(D_SIZE as usize);
            for v in [w, h, radius, scale, scales, amps, u32::from(plan.colour), 0] {
                b.extend(v.to_le_bytes());
            }
            for v in [gamma, amp(0), amp(1), 0.0] {
                b.extend(v.to_le_bytes());
            }
            init_buffer(device, "pc_camera_raw_detail_settings", wgpu::BufferUsages::UNIFORM, &b)
        };
        let full = [w.div_ceil(8), h.div_ceil(8)];
        self.dispatch(device, encoder, Pass::Planes, &uniform(0, 0), &coeffs, &bufs, 0, src, dst, full);
        for (si, radii) in plan.radii.iter().enumerate() {
            // A scale too small to blur adds nothing (its detail is 0).
            let Some(radii) = radii else { continue };
            self.dispatch(device, encoder, Pass::Copy, &uniform(0, 0), &coeffs, &bufs, 1, src, dst, full);
            // `from` is the buffer the next box pass reads.
            let mut from = 0usize;
            for (pass, lines) in [(Pass::Rows, h), (Pass::Cols, w)] {
                for &r in radii.iter() {
                    if r == 0 {
                        continue;
                    }
                    self.dispatch(
                        device,
                        encoder,
                        pass,
                        &uniform(r as u32, 0),
                        &coeffs,
                        &bufs,
                        if from == 0 { 0 } else { 2 },
                        src,
                        dst,
                        [lines.div_ceil(64), 1],
                    );
                    from = 1 - from;
                }
            }
            self.dispatch(device, encoder, Pass::Moves, &uniform(0, si as u32), &coeffs, &bufs, if from == 0 { 3 } else { 4 }, src, dst, full);
        }
        self.dispatch(device, encoder, Pass::Apply, &uniform(0, 0), &coeffs, &bufs, 0, src, dst, full);
    }

    /// Records one dispatch. `route` picks the blur buffers: 0 reads A and writes B, 2 reads B and
    /// writes A, 1 writes A (the copy), 3 reads A, 4 reads B.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pass: Pass,
        uniform: &wgpu::Buffer,
        coeffs: &wgpu::Buffer,
        bufs: &Buffers,
        route: u8,
        src: &wgpu::TextureView,
        dst: &wgpu::TextureView,
        groups: [u32; 2],
    ) {
        let Some(pipeline) = self.pipelines.iter().find(|(p, _)| *p == pass).map(|(_, p)| p) else { return };
        let [a, b] = &bufs.blur;
        let (input, output) = match route {
            2 | 4 => (b, a),
            1 => (b, a),
            _ => (a, b),
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_camera_raw_detail"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: coeffs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: bufs.planes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: bufs.knots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: bufs.moves.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: input.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(dst) },
            ],
        });
        let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_camera_raw_detail"), timestamp_writes: None });
        cp.set_pipeline(pipeline);
        cp.set_bind_group(0, &group, &[]);
        cp.dispatch_workgroups(groups[0].max(1), groups[1].max(1), 1);
    }
}

/// A buffer holding `data` (at least 16 bytes).
fn init_buffer(device: &wgpu::Device, label: &str, usage: wgpu::BufferUsages, data: &[u8]) -> wgpu::Buffer {
    let size = (data.len() as u64).max(16).div_ceil(4) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: true });
    let mut padded = data.to_vec();
    padded.resize(size as usize, 0);
    if let Ok(mut view) = buffer.slice(..).get_mapped_range_mut() {
        view.copy_from_slice(&padded);
    }
    buffer.unmap();
    buffer
}
