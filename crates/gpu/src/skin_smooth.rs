//! Bounded Skin Smooth v2 compute pass. The host supplies padded bands and blends selections.

use photocraft_raster::Interrupt;

use crate::{DeviceHealth, Unsupported};

/// Skin Smooth controls in the plug-in's original percent units.
#[derive(Clone, Debug)]
pub struct SkinSmoothParams {
    pub fine: f32,
    pub medium: f32,
    pub large: f32,
    pub detail: f32,
    pub brightness: f64,
    pub warmth: f32,
    pub redness: f32,
    pub tolerance: f32,
    pub skin_only: bool,
    pub positive: Vec<[f32; 3]>,
    pub negative: Vec<[f32; 3]>,
}

impl SkinSmoothParams {
    /// Checks finite control ranges and bounded normalized colour samples before GPU uniform upload.
    fn valid(&self) -> bool {
        [self.fine, self.medium, self.large, self.detail, self.redness, self.tolerance].iter().all(|v| v.is_finite() && (0.0..=100.0).contains(v))
            && self.brightness.is_finite()
            && (-100.0..=100.0).contains(&self.brightness)
            && self.warmth.is_finite()
            && (-100.0..=100.0).contains(&self.warmth)
            && photocraft_algo::skin_smooth_selection::valid_samples(&self.positive)
            && photocraft_algo::skin_smooth_selection::valid_samples(&self.negative)
    }
}

const UNIFORM_BYTES: u64 = 64 + 32 * 16;
const MAX_PIXELS: usize = 1_048_576;

struct Buffers {
    capacity: usize,
    source: wgpu::Buffer,
    result: wgpu::Buffer,
    readback: wgpu::Buffer,
    groups: Vec<wgpu::BindGroup>,
    uploaded: Vec<[f32; 4]>,
}

/// Created on a worker at first use, then reused across preview changes and final bands.
pub struct SkinSmooth {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
    uniforms: Vec<wgpu::Buffer>,
    buffers: Option<Buffers>,
}

impl SkinSmooth {
    /// Builds Skin Smooth pipelines and uniforms, returning Unsupported for insufficient limits or pipeline errors.
    pub fn new(device: &wgpu::Device) -> Result<Self, Unsupported> {
        let limits = device.limits();
        if limits.max_storage_buffers_per_shader_stage < 5 || limits.max_compute_workgroup_size_x < 64 || limits.max_compute_invocations_per_workgroup < 64 {
            return Err(Unsupported("Skin Smooth needs five storage buffers and 64 compute lanes".into()));
        }
        let (pass, error) = crate::paging::first_error(device, || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pc_skin_smooth"),
                source: wgpu::ShaderSource::Wgsl(include_str!("skin_smooth.wgsl").into()),
            });
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
                },
                count: None,
            }];
            for binding in 1..=5 {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: binding == 1 },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                });
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pc_skin_smooth"), entries: &entries });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pc_skin_smooth"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipelines = ["init", "horizontal", "rows", "vertical", "accumulate", "finish"]
                .into_iter()
                .map(|entry| {
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    })
                })
                .collect();
            let uniforms = (0..3)
                .map(|_| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("pc_skin_smooth_params"),
                        size: UNIFORM_BYTES,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                })
                .collect();
            Self { layout, pipelines, uniforms, buffers: None }
        });
        match error {
            Some(e) => Err(Unsupported(format!("Skin Smooth pipeline: {e}"))),
            None => Ok(pass),
        }
    }

    /// Allocates power-of-two working/readback capacity for a count already bounded by filter validation.
    fn allocate(&self, device: &wgpu::Device, count: usize) -> Buffers {
        let capacity = count.next_power_of_two();
        let storage = |label, bytes| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: capacity as u64 * bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let (source, work, accumulated, edges, result) =
            (storage("smooth_source", 16), storage("smooth_work", 16), storage("smooth_accum", 16), storage("smooth_edges", 8), storage("smooth_result", 16));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smooth_readback"),
            size: capacity as u64 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let groups = self
            .uniforms
            .iter()
            .map(|u| {
                let entries: Vec<_> = [u, &source, &work, &accumulated, &edges, &result]
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pc_skin_smooth"), layout: &self.layout, entries: &entries })
            })
            .collect();
        Buffers { capacity, source, result, readback, groups, uploaded: Vec::new() }
    }

    /// Filters normalized straight-alpha SDR RGBA. At most 1M pixels (~88 MiB GPU storage).
    /// `ctl` must include the caller's time budget; readback polls observe it without a fence wait.
    /// An interrupted readback drops its buffers, leaving wgpu to retain in-flight resources.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn filter(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: &DeviceHealth,
        rgba: &[[f32; 4]],
        size: [u32; 2],
        p: SkinSmoothParams,
        ctl: &Interrupt<'_>,
    ) -> Result<Vec<[f32; 4]>, Unsupported> {
        let [w, h] = size;
        let count = u64::from(w) * u64::from(h);
        if w == 0
            || h == 0
            || w > 8192
            || h > 8192
            || count > MAX_PIXELS as u64
            || count as usize != rgba.len()
            || !p.valid()
            || rgba.iter().flatten().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(Unsupported("invalid Skin Smooth band or parameters".into()));
        }
        if ctl.cancelled() || !health.is_ok() {
            return Err(Unsupported("Skin Smooth cancelled or GPU unavailable".into()));
        }
        if p.fine == 0.0 && p.medium == 0.0 && p.large == 0.0 && p.detail == 0.0 && p.brightness == 0.0 && p.warmth == 0.0 && p.redness == 0.0 {
            return Ok(rgba.to_vec());
        }
        let capacity = (count as usize).next_power_of_two();
        if capacity as u64 * 16 > device.limits().max_storage_buffer_binding_size
            || count.div_ceil(64) > u64::from(device.limits().max_compute_workgroups_per_dimension)
            || w.max(h) > device.limits().max_compute_workgroups_per_dimension
        {
            return Err(Unsupported("Skin Smooth band exceeds device limits".into()));
        }
        let levels = [p.fine / 100.0, p.medium / 100.0, p.large / 100.0];
        let detail = p.detail / 100.0;
        let last = levels.iter().rposition(|v| *v != 0.0).or_else(|| (detail != 0.0).then_some(0));
        let gains = [levels[0] - levels[1] - detail * 0.35, levels[1] - levels[2], levels[2]];
        for (i, u) in self.uniforms.iter().enumerate() {
            let mut bytes = Vec::with_capacity(UNIFORM_BYTES as usize);
            for v in [w, h, 0, 0] {
                bytes.extend(v.to_le_bytes());
            }
            let radius = [2.0, 7.0, 20.0].get(i).copied().unwrap_or(20.0);
            for v in [
                (-std::f64::consts::SQRT_2 / radius).exp() as f32,
                gains.get(i).copied().unwrap_or(0.0),
                detail * 0.35 - levels[0],
                0.0,
                2f64.powf(p.brightness / 100.0) as f32,
                p.warmth / 100.0 * 15.0,
                p.redness / 100.0,
                p.tolerance,
            ] {
                bytes.extend(v.to_le_bytes());
            }
            for v in [u32::from(p.skin_only), p.positive.len() as u32, p.negative.len() as u32, 0] {
                bytes.extend(v.to_le_bytes());
            }
            for colors in [&p.positive, &p.negative] {
                for j in 0..16 {
                    let rgb = colors.get(j).copied().unwrap_or([0.0; 3]);
                    let [cb, cr] = photocraft_algo::skin_smooth_selection::chroma(rgb.map(|v| v * 255.0));
                    for v in [cb, cr, 0.0, 0.0] {
                        bytes.extend(v.to_le_bytes());
                    }
                }
            }
            queue.write_buffer(u, 0, &bytes);
        }
        if self.buffers.as_ref().is_none_or(|b| b.capacity < count as usize) {
            self.buffers = Some(self.allocate(device, count as usize));
        }
        let b = self.buffers.as_mut().ok_or_else(|| Unsupported("Skin Smooth buffers unavailable".into()))?;
        let source_bytes = bytemuck::cast_slice(rgba);
        if bytemuck::cast_slice::<[f32; 4], u8>(&b.uploaded) != source_bytes {
            queue.write_buffer(&b.source, 0, source_bytes);
            b.uploaded.clear();
            b.uploaded.extend_from_slice(rgba);
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_skin_smooth") });
        let mut dispatch = |pipeline: usize, group: usize, n: u32| -> Result<(), Unsupported> {
            let groups = if matches!(pipeline, 1 | 3) { n } else { n.div_ceil(64) };
            let pipeline = self.pipelines.get(pipeline).ok_or_else(|| Unsupported("Skin Smooth pipeline missing".into()))?;
            let group = b.groups.get(group).ok_or_else(|| Unsupported("Skin Smooth parameter group missing".into()))?;
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_skin_smooth"), timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
            Ok(())
        };
        dispatch(0, 0, count as u32)?;
        for i in 0..last.map_or(0, |i| i + 1) {
            dispatch(1, i, h)?;
            dispatch(2, i, count as u32)?;
            dispatch(3, i, w)?;
            dispatch(4, i, count as u32)?;
        }
        dispatch(5, 0, count as u32)?;
        enc.copy_buffer_to_buffer(&b.result, 0, &b.readback, 0, count * 16);
        queue.submit([enc.finish()]);
        let slice = b.readback.slice(..count * 16);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        loop {
            if ctl.cancelled() || !health.is_ok() {
                self.buffers = None;
                return Err(Unsupported("Skin Smooth cancelled or GPU unavailable".into()));
            }
            health.poll(device);
            match rx.try_recv() {
                Ok(Ok(())) => break,
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.buffers = None;
                    return Err(Unsupported("Skin Smooth GPU readback failed".into()));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        let data = slice.get_mapped_range().map_err(|e| Unsupported(format!("Skin Smooth readback: {e}")))?;
        let out = bytemuck::try_cast_slice::<u8, [f32; 4]>(&data).map_err(|e| Unsupported(format!("Skin Smooth pixels: {e}")))?.to_vec();
        drop(data);
        b.readback.unmap();
        if ctl.cancelled() {
            return Err(Unsupported("Skin Smooth cancelled".into()));
        }
        Ok(out)
    }
}
