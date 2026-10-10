//! Bounded compute coverage backend. CPU native pixels remain authoritative, including undo
//! and saves. Unsupported tips and batches too small to amortize readback stay on the CPU.

use crate::DeviceHealth;
use photocraft_paint::render::{CoverageAccelerator, RasterTile, grid_center};
use photocraft_paint::{BrushSettings, Dab, TipShape};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicU64, Ordering},
};

const SHADER: &str = include_str!("brush.wgsl");
const PIXELS: usize = 4096;
const MAX_TILES: usize = 1024;

fn read_float(bytes: &[u8]) -> f32 {
    match bytes {
        [a, b, c, d] => f32::from_le_bytes([*a, *b, *c, *d]),
        _ => f32::NAN,
    }
}

#[derive(Debug)]
struct Scratch {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    dabs: wgpu::Buffer,
    jobs: wgpu::Buffer,
    coverage: wgpu::Buffer,
    readback: wgpu::Buffer,
    capacity: usize,
}

/// Shares the canvas device; buffers are pooled and bounded independently of canvas size.
#[derive(Debug)]
pub struct Rasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    health: DeviceHealth,
    scratch: Mutex<Option<Scratch>>,
    /// Minimum estimated dab pixels; calibrated against completed CPU/GPU batch timings.
    pub min_work: u64,
    batches: AtomicU64,
}

impl Rasterizer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, health: DeviceHealth) -> Self {
        Self { device, queue, health, scratch: Mutex::new(None), min_work: 8_000_000, batches: AtomicU64::new(0) }
    }
    pub fn batches(&self) -> u64 {
        self.batches.load(Ordering::Relaxed)
    }

    /// Compile the kernel before the first pointer gesture. The tiny dispatch also primes
    /// driver caches; it does not count as a document brush batch.
    pub fn prewarm(&self) {
        let brush = BrushSettings { hardness: 0.0, ..Default::default() };
        let dabs = vec![Dab::round(photocraft_geom::Point::new(32.0, 32.0), 500.0, 0.1); 8];
        let mut tiles = [RasterTile { origin: [0, 0], dabs: (0..8).collect(), coverage: vec![0.0; PIXELS] }];
        if self.rasterize(&brush, false, &dabs, &mut tiles) {
            self.batches.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn buffer(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
    }

    fn create(&self, capacity: usize) -> Scratch {
        let module =
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_brush"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_brush"),
            entries: &std::array::from_fn::<_, 3, _>(|binding| wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: binding != 2 },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_brush"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pc_brush"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("raster"),
            compilation_options: Default::default(),
            cache: None,
        });
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Scratch {
            pipeline,
            layout,
            dabs: self.buffer("pc_brush_dabs", 64 * 64, usage),
            jobs: self.buffer("pc_brush_jobs", (capacity * (4 + 64) * 4) as u64, usage),
            coverage: self.buffer("pc_brush_coverage", (capacity * PIXELS * 4) as u64, usage | wgpu::BufferUsages::COPY_SRC),
            readback: self.buffer("pc_brush_readback", (capacity * PIXELS * 4) as u64, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            capacity,
        }
    }

    fn run(&self, brush: &BrushSettings, dual: bool, dabs: &[Dab], tiles: &mut [RasterTile]) -> Option<()> {
        if !self.supports(brush, dual, dabs, tiles.len()) {
            return None;
        }
        let mut dab_bytes = Vec::with_capacity(dabs.len() * 64);
        for d in dabs {
            let aliased = brush.aliased && !dual;
            let (x, y) = if aliased { grid_center(d.center.x, d.center.y, d.radius * 2.0) } else { (d.center.x as f32, d.center.y as f32) };
            let (sn, cs) = d.angle.sin_cos();
            let (ps, pc) = d.proj_angle.sin_cos();
            let projection = if !dual && d.proj_scale < 0.999 { 1.0 / d.proj_scale.max(0.05) - 1.0 } else { 0.0 };
            let values = [
                x,
                y,
                d.radius,
                d.roundness.max(0.5 / d.radius).min(1.0),
                sn,
                cs,
                if d.flip_x { -1.0 } else { 1.0 },
                if d.flip_y { -1.0 } else { 1.0 },
                d.alpha,
                if dual { 1.0 } else { d.opacity },
                pc,
                ps,
                projection,
                if dual { brush.dual_brush.hardness } else { brush.hardness },
                f32::from(u8::from(brush.wet_edges && !dual)),
                f32::from(u8::from(aliased)),
            ];
            if values.iter().any(|v| !v.is_finite()) {
                return None;
            }
            for value in values {
                dab_bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut jobs = Vec::with_capacity(tiles.len() * 68);
        let mut indices = Vec::new();
        let mut input = Vec::with_capacity(tiles.len() * PIXELS * 4);
        for tile in tiles.iter() {
            if tile.coverage.len() != PIXELS || tile.dabs.len() > 64 || tile.dabs.iter().any(|i| *i >= dabs.len()) {
                return None;
            }
            jobs.extend([tiles.len() as u32 * 4 + indices.len() as u32, tile.dabs.len() as u32, tile.origin[0] as u32, tile.origin[1] as u32]);
            indices.extend(tile.dabs.iter().map(|i| *i as u32));
            for value in &tile.coverage {
                input.extend_from_slice(&value.to_le_bytes());
            }
        }
        jobs.extend(indices);
        let job_bytes: Vec<_> = jobs.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut guard = self.scratch.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(self.create(tiles.len().next_power_of_two().min(MAX_TILES)));
        }
        if let Some(s) = guard.as_mut()
            && s.capacity < tiles.len()
        {
            let capacity = tiles.len().next_power_of_two().min(MAX_TILES);
            let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            s.jobs = self.buffer("pc_brush_jobs", (capacity * 68 * 4) as u64, usage);
            s.coverage = self.buffer("pc_brush_coverage", (capacity * PIXELS * 4) as u64, usage | wgpu::BufferUsages::COPY_SRC);
            s.readback = self.buffer("pc_brush_readback", (capacity * PIXELS * 4) as u64, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            s.capacity = capacity;
        }
        let s = guard.as_ref()?;
        self.queue.write_buffer(&s.dabs, 0, &dab_bytes);
        self.queue.write_buffer(&s.jobs, 0, &job_bytes);
        self.queue.write_buffer(&s.coverage, 0, &input);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_brush"),
            layout: &s.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: s.dabs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: s.jobs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: s.coverage.as_entire_binding() },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_brush") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_brush"), timestamp_writes: None });
            pass.set_pipeline(&s.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(8, 8, tiles.len() as u32);
        }
        let bytes = input.len() as u64;
        encoder.copy_buffer_to_buffer(&s.coverage, 0, &s.readback, 0, bytes);
        let index = self.queue.submit([encoder.finish()]);
        let slice = s.readback.slice(..bytes);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if !self.health.wait(&self.device, Some(index)) {
            return None;
        }
        let data = match slice.get_mapped_range() {
            Ok(data) => data,
            Err(_) => {
                s.readback.unmap();
                return None;
            }
        };
        // Validate before publishing any result: failure leaves the CPU coverage untouched.
        if data.len() != input.len() || data.as_chunks::<4>().0.iter().any(|b| !read_float(b).is_finite()) {
            drop(data);
            s.readback.unmap();
            return None;
        }
        for (tile, bytes) in tiles.iter_mut().zip(data.as_chunks::<{ PIXELS * 4 }>().0.iter()) {
            for (value, bytes) in tile.coverage.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
                *value = read_float(bytes);
            }
        }
        drop(data);
        s.readback.unmap();
        self.batches.fetch_add(1, Ordering::Relaxed);
        Some(())
    }
}

impl CoverageAccelerator for Rasterizer {
    fn supports(&self, brush: &BrushSettings, dual: bool, dabs: &[Dab], tiles: usize) -> bool {
        let limits = self.device.limits();
        self.health.is_ok()
            && !dabs.is_empty()
            && dabs.len() <= 64
            && (1..=MAX_TILES).contains(&tiles)
            && limits.max_compute_workgroup_size_x >= 8
            && limits.max_compute_workgroup_size_y >= 8
            && limits.max_compute_invocations_per_workgroup >= 64
            && limits.max_compute_workgroups_per_dimension >= 8.max(tiles as u32)
            && limits.max_bind_groups >= 1
            && limits.max_bindings_per_bind_group >= 3
            && limits.max_storage_buffers_per_shader_stage >= 3
            && limits.max_storage_buffer_binding_size as usize >= tiles.next_power_of_two() * PIXELS * 4
            && limits.max_buffer_size >= (tiles.next_power_of_two() * PIXELS * 4) as u64
            && matches!(if dual { &brush.dual_brush.tip } else { &brush.tip }, TipShape::Round)
            && (dual || (!brush.wet_edges && !brush.noise && !(brush.texture.enabled && brush.texture.each_tip)))
            && dabs.iter().all(|d| {
                d.alpha.is_finite()
                    && (0.0..=1.0).contains(&d.alpha)
                    && d.opacity.is_finite()
                    && (0.0..=1.0).contains(&d.opacity)
                    && d.radius.is_finite()
                    && d.radius > 0.0
                    && d.radius * d.roundness.max(0.5 / d.radius).min(1.0) >= 3.0
            })
            && (self.min_work == 0
                || dabs.iter().map(|d| (4.0 * d.radius * d.radius) as u64).fold(0u64, u64::saturating_add) >= self.min_work.max((tiles * PIXELS * 4) as u64))
    }
    fn rasterize(&self, brush: &BrushSettings, dual: bool, dabs: &[Dab], tiles: &mut [RasterTile]) -> bool {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run(brush, dual, dabs, tiles))) {
            Ok(result) => result.is_some(),
            Err(_) => {
                self.health.mark(crate::Fault::Lost("GPU brush stopped responding".into()));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty()).validate(&module).unwrap();
    }
}
