//! GPU rendering for the editable mask Paint stack.
//!
//! Unlike the document compositor's embedded mask-stack passes, this entry point returns the
//! stack coverage itself. Shell ownership is supplied as an integer atlas, so Blur can isolate
//! UV islands without sampling neighbouring shells. The caller owns the final texture and can
//! bind it directly as a material map; this path never reads the result back to the CPU.

use photocraft_doc::mask_paint::{EffectKind, Stack};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Surface, Tile};
use std::{collections::BTreeMap, sync::Arc};
use wgpu::util::DeviceExt;

const SHADER: &str = include_str!("mask_stack_gpu.wgsl");
const MAX_MASK_FILL_MAP_BYTES: u64 = 256 * 1024 * 1024;

/// Reusable pipelines for rendering editable mask stacks to a caller-owned GPU target.
pub struct MaskStackRenderer {
    pipeline: wgpu::RenderPipeline,
    dab_pipeline: wgpu::ComputePipeline,
    params_layout: wgpu::BindGroupLayout,
    textures_layout: wgpu::BindGroupLayout,
    kernel_weights: wgpu::Buffer,
    dummy_coverage: wgpu::Buffer,
    cache: Option<RenderCache>,
    brush_preview: Option<MaskBrushPreview>,
}

/// Round-dab inputs emitted by the CPU's authoritative DabGenerator. `radius` is the effective
/// radius after applying dab roundness. Other dab shapes and small-tip supersampling stay on CPU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskRoundDab {
    pub center: [f32; 2],
    pub radius: f32,
    pub alpha: f32,
    pub opacity: f32,
}

/// The per-stroke values needed to preview a supported native grayscale mask brush.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskBrushSettings {
    pub hardness: f32,
    pub stroke_opacity: f32,
    pub color: [f32; 4],
}

/// One dirty canonical Paint patch read back when the gesture ends. Pixels are packed row-major
/// straight RGBA8; the patch origin is `rect.(x0,y0)` and row stride is `rect.width() * 4`.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskBrushPatch {
    pub rect: Rect,
    pub pixels_rgba8: Vec<u8>,
}

struct MaskBrushPreview {
    owner_key: u64,
    size: [u32; 2],
    base: CachedSurface,
    coverage: wgpu::Buffer,
    output: Arc<wgpu::Texture>,
    dummy_shell: wgpu::Texture,
    dummy: wgpu::Texture,
    settings: MaskBrushSettings,
    dirty: Vec<Rect>,
    stroke_dirty: Vec<Rect>,
}

struct CachedSurface {
    texture: wgpu::Texture,
    format: photocraft_color::PixelFormat,
    default_pixel: Vec<f32>,
    tiles: BTreeMap<TileCoord, Arc<Tile>>,
}

struct RenderCache {
    owner_key: u64,
    size: [u32; 2],
    base: CachedSurface,
    paints: Vec<Option<CachedSurface>>,
    fills: Vec<Option<CachedFill>>,
    shell_key: Option<u64>,
    shell: Option<wgpu::Texture>,
    dummy_shell: wgpu::Texture,
    scratch: Vec<wgpu::Texture>,
    dummy: wgpu::Texture,
    stack_key: Option<u64>,
    blur_checkpoint: Option<wgpu::Texture>,
    blur_checkpoint_after: Option<usize>,
    blur_checkpoint_key: Option<u64>,
}

struct CachedFill {
    texture: wgpu::Texture,
    size: [u32; 2],
    format: photocraft_color::PixelFormat,
    default_pixel: Vec<f32>,
    tiles: BTreeMap<TileCoord, Arc<Tile>>,
}

impl RenderCache {
    fn configure_blur_resources(&mut self, device: &wgpu::Device, size: [u32; 2], enabled: bool) {
        if enabled {
            while self.scratch.len() < 4 {
                self.scratch.push(rgba16_texture(device, size, "pc_mask_stack_ping_pong"));
            }
            if self.shell.is_none() {
                self.shell = Some(rgba32u_texture(device, size, "pc_mask_shell_ids"));
                self.shell_key = None;
            }
            if self.blur_checkpoint.is_none() {
                self.blur_checkpoint = Some(rgba16_texture(device, size, "pc_mask_stack_blur_checkpoint"));
            }
        } else {
            self.scratch.truncate(2);
            self.shell = None;
            self.shell_key = None;
            self.blur_checkpoint = None;
            self.blur_checkpoint_after = None;
            self.blur_checkpoint_key = None;
        }
    }
}

impl MaskStackRenderer {
    /// Validate a stack without allocating textures or uploading source pixels.
    pub fn validate(stack: &Stack) -> Result<(), String> {
        if stack.base.channels() == 0 {
            return Err("Mask base surface has no channels".into());
        }
        for layer in &stack.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
                return Err("Mask-stack layer has invalid opacity".into());
            }
            mask_blend_code(layer.blend).ok_or_else(|| "Mask-stack blend mode is unsupported by the GPU preview".to_string())?;
            match &layer.kind {
                EffectKind::Paint if layer.surface.channels() != 4 => return Err("Mask Paint surface must have four channels".into()),
                EffectKind::Fill { value, rotation, .. } => {
                    if !value.is_finite() || !(0.0..=1.0).contains(value) || !rotation.is_finite() {
                        return Err("Mask Fill settings are invalid".into());
                    }
                    if let Some(map) = &layer.fill_map {
                        let [width, height] = layer.fill_map_size;
                        if width == 0 || height == 0 || map.channels() == 0 {
                            return Err("Mask Fill image has an invalid size or format".into());
                        }
                    }
                }
                EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                    if !input_black.is_finite()
                        || !gamma.is_finite()
                        || !input_white.is_finite()
                        || !output_black.is_finite()
                        || !output_white.is_finite()
                        || !(0.0..=1.0).contains(input_black)
                        || !(0.01..=100.0).contains(gamma)
                        || !(0.0..=1.0).contains(input_white)
                        || !(0.0..=1.0).contains(output_black)
                        || !(0.0..=1.0).contains(output_white)
                        || input_black >= input_white
                        || output_black > output_white
                    {
                        return Err("Mask Levels settings are invalid".into());
                    }
                }
                EffectKind::Blur { radius } if *radius > 64 => return Err("Mask Blur radius exceeds the GPU preview limit of 64".into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// Drop all per-stack source and scratch textures.
    pub fn clear_cache(&mut self) {
        self.cache = None;
        self.brush_preview = None;
    }

    /// Start a temporary GPU preview stroke over an existing four-channel Paint surface. This
    /// preview is separate from the canonical Surface; callers commit the final dirty patches via
    /// the existing CPU command/Undo path, or discard it with `cancel_mask_brush_preview`.
    pub fn begin_round_mask_brush_preview(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        owner_key: u64,
        size: [u32; 2],
        base_paint: &Surface,
        settings: MaskBrushSettings,
    ) -> Result<(), String> {
        if size.contains(&0)
            || size.iter().any(|side| *side > device.limits().max_texture_dimension_2d)
            || !rgba8_compatible(base_paint)
            || !settings.hardness.is_finite()
            || !(0.0..=1.0).contains(&settings.hardness)
            || !settings.stroke_opacity.is_finite()
            || !(0.0..=1.0).contains(&settings.stroke_opacity)
            || settings.color.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("GPU mask brush settings or source surface are invalid".into());
        }
        let coverage_bytes =
            u64::from(size[0]).checked_mul(u64::from(size[1])).and_then(|pixels| pixels.checked_mul(4)).ok_or("GPU mask brush coverage size overflow")?;
        if coverage_bytes > u64::from(device.limits().max_storage_buffer_binding_size) || coverage_bytes > device.limits().max_buffer_size {
            return Err("GPU mask brush coverage exceeds the storage-buffer limit".into());
        }
        let base = create_cached_surface(device, queue, size, base_paint, "pc_mask_brush_base")?;
        let coverage = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_mask_brush_coverage"),
            size: coverage_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output = Arc::new(rgba16_texture(device, size, "pc_mask_brush_preview"));
        let dummy_shell = rgba32u_texture(device, [1, 1], "pc_mask_brush_dummy_shell");
        let dummy = rgba16_texture(device, [1, 1], "pc_mask_brush_dummy");
        let preview = MaskBrushPreview { owner_key, size, base, coverage, output, dummy_shell, dummy, settings, dirty: Vec::new(), stroke_dirty: Vec::new() };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mask_brush_begin") });
        encoder.clear_buffer(&preview.coverage, 0, None);
        let base_view = preview.base.texture.create_view(&Default::default());
        let dummy_view = preview.dummy.create_view(&Default::default());
        let shell_view = preview.dummy_shell.create_view(&Default::default());
        let output_view = preview.output.create_view(&Default::default());
        Self::draw_stage(
            &self.pipeline,
            &self.params_layout,
            &self.textures_layout,
            device,
            &mut encoder,
            &base_view,
            &dummy_view,
            &shell_view,
            &output_view,
            size,
            brush_composite_params(size, settings),
            None,
            &self.kernel_weights,
            &preview.coverage,
        )?;
        queue.submit([encoder.finish()]);
        self.brush_preview = Some(preview);
        Ok(())
    }

    /// Add authoritative CPU-generated round dabs without readback. Returns only the clipped
    /// output regions whose GPU preview changed. Rejecting unsupported small/invalid dabs lets the
    /// caller switch the entire gesture to the existing CPU path before mutating canonical data.
    pub fn push_round_mask_brush_dabs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, dabs: &[MaskRoundDab]) -> Result<Vec<Rect>, String> {
        let preview = self.brush_preview.as_mut().ok_or("No active GPU mask brush preview")?;
        let mut encoded_dabs = Vec::with_capacity(dabs.len());
        for dab in dabs {
            let (rect, core, shoulder, radius) = round_dab_parameters(*dab, preview.settings.hardness, preview.size)?;
            if rect.is_empty() {
                continue;
            }
            encoded_dabs.push((*dab, rect, core, shoulder, radius));
        }
        if encoded_dabs.is_empty() {
            return Ok(Vec::new());
        }
        let mut damage = Vec::new();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mask_brush_dabs") });
        let base_view = preview.base.texture.create_view(&Default::default());
        let dummy_view = preview.dummy.create_view(&Default::default());
        let shell_view = preview.dummy_shell.create_view(&Default::default());
        for (dab, rect, core, shoulder, radius) in encoded_dabs {
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pc_mask_brush_dab_params"),
                contents: &dab_params(preview.size, dab, rect, radius, core, shoulder),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let params_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pc_mask_brush_dab_params"),
                layout: &self.params_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
            });
            let textures_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pc_mask_brush_dab_inputs"),
                layout: &self.textures_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&base_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dummy_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&shell_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: self.kernel_weights.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: preview.coverage.as_entire_binding() },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_mask_brush_dab"), timestamp_writes: None });
            pass.set_pipeline(&self.dab_pipeline);
            pass.set_bind_group(0, &params_group, &[]);
            pass.set_bind_group(1, &textures_group, &[]);
            pass.dispatch_workgroups(rect.width().div_ceil(8), rect.height().div_ceil(8), 1);
            drop(pass);
            add_regions(&mut damage, rect);
        }
        let output_view = preview.output.create_view(&Default::default());
        Self::draw_stage(
            &self.pipeline,
            &self.params_layout,
            &self.textures_layout,
            device,
            &mut encoder,
            &base_view,
            &dummy_view,
            &shell_view,
            &output_view,
            preview.size,
            brush_composite_params(preview.size, preview.settings),
            Some(&damage),
            &self.kernel_weights,
            &preview.coverage,
        )?;
        queue.submit([encoder.finish()]);
        for rect in &damage {
            add_regions(&mut preview.dirty, *rect);
            add_regions(&mut preview.stroke_dirty, *rect);
        }
        Ok(damage)
    }

    /// The transient Rgba16Float Paint map to pass to `render_cached_with_paint_override`.
    pub fn mask_brush_preview_texture(&self) -> Option<&wgpu::Texture> {
        self.brush_preview.as_ref().map(|preview| preview.output.as_ref())
    }

    /// Drain accumulated dirty regions for GPU stack recomposition or one final CPU readback.
    pub fn take_mask_brush_dirty_regions(&mut self) -> Vec<Rect> {
        self.brush_preview.as_mut().map_or_else(Vec::new, |preview| std::mem::take(&mut preview.dirty))
    }

    /// Perform the single blocking release readback for a stroke. The caller applies each patch
    /// to the canonical Paint Surface and submits the existing undoable document command.
    pub fn read_mask_brush_patches(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Vec<MaskBrushPatch>, String> {
        let preview = self.brush_preview.as_ref().ok_or("No active GPU mask brush preview")?;
        if preview.stroke_dirty.is_empty() {
            return Ok(Vec::new());
        }
        let texture = &preview.output;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mask_brush_release_readback") });
        let mut buffers = Vec::with_capacity(preview.stroke_dirty.len());
        for rect in &preview.stroke_dirty {
            let row = (rect.width() * 8).div_ceil(256) * 256;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_mask_brush_release_patch"),
                size: u64::from(row) * u64::from(rect.height()),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rect.height()) },
                },
                wgpu::Extent3d { width: rect.width(), height: rect.height(), depth_or_array_layers: 1 },
            );
            buffers.push((buffer, row, *rect));
        }
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        for (buffer, _, _) in &buffers {
            let send = send.clone();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        }
        drop(send);
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        for _ in 0..buffers.len() {
            receive.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        }
        let mut patches = Vec::with_capacity(buffers.len());
        for (buffer, row, rect) in &buffers {
            let mapped = buffer.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
            let mut pixels_rgba8 = Vec::with_capacity(rect.width() as usize * rect.height() as usize * 4);
            for y in 0..rect.height() as usize {
                for x in 0..rect.width() as usize {
                    let offset = y * *row as usize + x * 8;
                    for channel in 0..4 {
                        let byte_offset = offset + channel * 2;
                        let value = half::f16::from_le_bytes([mapped[byte_offset], mapped[byte_offset + 1]]).to_f32();
                        pixels_rgba8.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
                    }
                }
            }
            drop(mapped);
            buffer.unmap();
            patches.push(MaskBrushPatch { rect: *rect, pixels_rgba8 });
        }
        Ok(patches)
    }

    /// Discard transient GPU dab coverage and preview pixels. Canonical Surface and Undo remain
    /// owned by the caller throughout the gesture.
    pub fn cancel_mask_brush_preview(&mut self) {
        let Some(preview) = self.brush_preview.take() else { return };
        if let Some(cache) = self.cache.as_mut()
            && cache.owner_key == preview.owner_key
        {
            // The output target may still contain this transient preview. Force the next render
            // to rebuild the stack from canonical surfaces before reusing that target.
            cache.stack_key = None;
        }
    }

    /// Stable key for stack structure/settings; pixel content and document revisions are excluded.
    /// Callers can use it to avoid retrying an unsupported GPU configuration on every paint update.
    pub fn configuration_key(stack: &Stack) -> u64 {
        stack_config_key(stack)
    }

    /// Build the mask-stack pipeline on the supplied device.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_mask_stack_gpu"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_mask_stack_params"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(80),
                },
                count: None,
            }],
        });
        let float_texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let textures_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_mask_stack_inputs"),
            entries: &[
                float_texture(0),
                float_texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_mask_stack_gpu"),
            bind_group_layouts: &[Some(&params_layout), Some(&textures_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pc_mask_stack_gpu"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let dab_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pc_mask_round_dab_gpu"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("cs_dab"),
            compilation_options: Default::default(),
            cache: None,
        });
        let kernel_values = gaussian_kernel_weights();
        let kernel_weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pc_mask_blur_kernel_weights"),
            contents: bytemuck::cast_slice(&kernel_values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let dummy_coverage = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_mask_brush_dummy_coverage"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Self { pipeline, dab_pipeline, params_layout, textures_layout, kernel_weights, dummy_coverage, cache: None, brush_preview: None }
    }

    /// Render `stack` into `target` at `size`, using the row-major integer shell map.
    ///
    /// `shell_ids` contains zero for off-shell texels and `shell_id + 1` for occupied texels.
    /// Blur is shell-aware: it renormalizes samples within one shell and lets occupied pixels
    /// contribute outward without allowing off-shell pixels to bleed back into a shell.
    /// Unsupported historical Fill map variants return an error so the caller can retain its
    /// canonical CPU preview. The target must be an `Rgba16Float` texture with matching extent
    /// and `RENDER_ATTACHMENT | COPY_DST` usage.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stack: &Stack,
        size: [u32; 2],
        shell_ids: &[u32],
        target: &wgpu::Texture,
    ) -> Result<(), String> {
        self.render_cached(device, queue, stack, size, shell_ids, 0, 0, target)
    }

    /// Reuse same-size input and intermediate textures. `owner_key` identifies a mask stack;
    /// `shell_key` identifies the topology/material ownership map. The caller must keep the
    /// target texture stable while reusing an owner key, because unchanged frames are skipped.
    pub fn render_cached(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stack: &Stack,
        size: [u32; 2],
        shell_ids: &[u32],
        owner_key: u64,
        shell_key: u64,
        target: &wgpu::Texture,
    ) -> Result<(), String> {
        self.render_cached_internal(device, queue, stack, size, shell_ids, owner_key, shell_key, target, None)
    }

    /// Recompose the stack with a transient live GPU Paint map. The brush renderer must have been
    /// begun for the same owner/size; `dirty_regions` are the current dab updates since last draw.
    pub fn render_cached_with_brush_preview(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stack: &Stack,
        size: [u32; 2],
        shell_ids: &[u32],
        shell_key: u64,
        paint_layer_index: usize,
        dirty_regions: &[Rect],
        target: &wgpu::Texture,
    ) -> Result<(), String> {
        let (owner_key, output) = {
            let preview = self.brush_preview.as_ref().ok_or("No active GPU mask brush preview")?;
            if preview.size != size || stack.layers.get(paint_layer_index).is_none_or(|layer| !matches!(&layer.kind, EffectKind::Paint)) {
                return Err("GPU mask brush preview does not match this stack/layer".into());
            }
            (preview.owner_key, Arc::clone(&preview.output))
        };
        self.render_cached_internal(
            device,
            queue,
            stack,
            size,
            shell_ids,
            owner_key,
            shell_key,
            target,
            Some((paint_layer_index, output.as_ref(), dirty_regions)),
        )
    }

    fn render_cached_internal(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stack: &Stack,
        size: [u32; 2],
        shell_ids: &[u32],
        owner_key: u64,
        shell_key: u64,
        target: &wgpu::Texture,
        brush_override: Option<(usize, &wgpu::Texture, &[Rect])>,
    ) -> Result<(), String> {
        let count = usize::try_from(size[0]).ok().and_then(|w| usize::try_from(size[1]).ok().and_then(|h| w.checked_mul(h)));
        if size.contains(&0)
            || size.iter().any(|side| *side > device.limits().max_texture_dimension_2d)
            || count != Some(shell_ids.len())
            || target.size().width != size[0]
            || target.size().height != size[1]
            || target.format() != wgpu::TextureFormat::Rgba16Float
        {
            return Err("Mask-stack target or shell map has an invalid size/format".into());
        }
        Self::validate(stack)?;
        for layer in &stack.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if matches!(&layer.kind, EffectKind::Fill { .. })
                && let Some([width, height]) = layer.fill_map.as_ref().map(|_| layer.fill_map_size)
            {
                let bytes = u64::from(width).checked_mul(u64::from(height)).and_then(|pixels| pixels.checked_mul(4));
                if width == 0
                    || height == 0
                    || width > device.limits().max_texture_dimension_2d
                    || height > device.limits().max_texture_dimension_2d
                    || bytes.is_none_or(|bytes| bytes > MAX_MASK_FILL_MAP_BYTES)
                {
                    return Err("Mask Fill image exceeds the GPU preview allocation limit".into());
                }
            }
        }
        let has_blur =
            stack.layers.iter().any(|layer| layer.visible && layer.opacity > 0.0 && matches!(&layer.kind, EffectKind::Blur { radius } if *radius > 0));
        if self.cache.as_ref().is_none_or(|cache| cache.owner_key != owner_key || cache.size != size) {
            self.cache = Some(create_render_cache(device, size, owner_key));
        }
        let cache = self.cache.as_mut().ok_or("Mask-stack GPU cache initialization failed")?;
        cache.configure_blur_resources(device, size, has_blur);
        let stack_key = stack_config_key(stack);
        let shell_changed = has_blur && cache.shell_key != Some(shell_key);
        if shell_changed {
            cache.shell = Some(upload_shell_ids(device, queue, size, shell_ids));
            cache.shell_key = Some(shell_key);
        }
        let mut layers = Vec::new();
        let mut changed_regions = Vec::new();
        let mut changed_layer_index = None;
        let mut paint_index = 0usize;
        let mut fill_index = 0usize;
        for (layer_index, layer) in stack.layers.iter().enumerate() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
                return Err("Mask-stack layer has invalid opacity".into());
            }
            let mode = mask_blend_code(layer.blend).ok_or_else(|| "Mask-stack blend mode is unsupported by the GPU preview".to_string())?;
            match &layer.kind {
                EffectKind::Paint => {
                    if layer.surface.channels() != 4 {
                        return Err("Mask Paint surface must have four channels".into());
                    }
                    while cache.paints.len() <= paint_index {
                        cache.paints.push(None);
                    }
                    let paint = &mut cache.paints[paint_index];
                    if paint.is_none() {
                        *paint = Some(create_cached_surface(device, queue, size, &layer.surface, "pc_mask_paint_input")?);
                        changed_regions = vec![full_bounds(size)];
                        changed_layer_index = Some(layer_index);
                    } else if let Some(paint) = paint {
                        let dirty = sync_cached_surface(device, queue, size, &layer.surface, paint, true)?;
                        if !dirty.is_empty() {
                            for rect in dirty {
                                add_regions(&mut changed_regions, rect);
                            }
                            changed_layer_index = Some(changed_layer_index.map_or(layer_index, |current: usize| current.min(layer_index)));
                        }
                    }
                    layers.push((layer_index, layer.opacity, mode, MaskLayer::Paint(paint_index)));
                    paint_index += 1;
                }
                EffectKind::Fill { value, .. } => {
                    if !value.is_finite() || !(0.0..=1.0).contains(value) {
                        return Err("Mask Fill value is invalid".into());
                    }
                    let mut map_index = None;
                    if let Some(map) = &layer.fill_map {
                        let [map_width, map_height] = layer.fill_map_size;
                        if map_width == 0
                            || map_height == 0
                            || map_width > device.limits().max_texture_dimension_2d
                            || map_height > device.limits().max_texture_dimension_2d
                        {
                            return Err("Mask Fill image has an invalid size".into());
                        }
                        while cache.fills.len() <= fill_index {
                            cache.fills.push(None);
                        }
                        let cached = &mut cache.fills[fill_index];
                        if cached.as_ref().is_none_or(|cached| cached.size != [map_width, map_height]) {
                            *cached = Some(create_cached_fill(device, queue, [map_width, map_height], map)?);
                            changed_regions = vec![full_bounds(size)];
                            changed_layer_index = Some(layer_index);
                        } else if let Some(cached) = cached {
                            let dirty = sync_cached_fill(queue, map, cached)?;
                            if !dirty.is_empty() {
                                // A Fill image change can affect the full destination bounds.
                                changed_regions = vec![full_bounds(size)];
                                changed_layer_index = Some(changed_layer_index.map_or(layer_index, |current: usize| current.min(layer_index)));
                            }
                        }
                        map_index = Some(fill_index);
                        fill_index += 1;
                    }
                    let (rotation, repeat_u, repeat_v, mirror_u, mirror_v) = match &layer.kind {
                        EffectKind::Fill { rotation, repeat_u, repeat_v, mirror_u, mirror_v, .. } => (*rotation, *repeat_u, *repeat_v, *mirror_u, *mirror_v),
                        _ => unreachable!(),
                    };
                    let bounds = layer.fill_bounds.unwrap_or([0; 4]);
                    layers.push((
                        layer_index,
                        layer.opacity,
                        mode,
                        MaskLayer::Fill {
                            value: *value,
                            bounds,
                            has_bounds: layer.fill_bounds.is_some(),
                            rotation,
                            repeat_u,
                            repeat_v,
                            mirror_u,
                            mirror_v,
                            map_index,
                        },
                    ));
                }
                EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                    if !input_black.is_finite()
                        || !gamma.is_finite()
                        || !input_white.is_finite()
                        || !output_black.is_finite()
                        || !output_white.is_finite()
                        || !(0.0..=1.0).contains(input_black)
                        || !(0.01..=100.0).contains(gamma)
                        || !(0.0..=1.0).contains(input_white)
                        || !(0.0..=1.0).contains(output_black)
                        || !(0.0..=1.0).contains(output_white)
                        || input_black >= input_white
                        || output_black > output_white
                    {
                        return Err("Mask Levels settings are invalid".into());
                    }
                    layers.push((layer_index, layer.opacity, mode, MaskLayer::Levels([*input_black, *gamma, *input_white, *output_black, *output_white])));
                }
                EffectKind::Blur { radius } => {
                    if *radius > 64 {
                        return Err("Mask Blur radius exceeds the GPU preview limit of 64".into());
                    }
                    layers.push((layer_index, layer.opacity, mode, MaskLayer::Blur(*radius)));
                }
            }
        }
        cache.paints.truncate(paint_index);
        cache.fills.truncate(fill_index);

        if let Some((layer_index, texture, override_damage)) = brush_override {
            if texture.size().width != size[0]
                || texture.size().height != size[1]
                || !matches!(texture.format(), wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba8Unorm)
                || stack.layers.get(layer_index).is_none_or(|layer| !layer.visible || layer.opacity <= 0.0 || !matches!(&layer.kind, EffectKind::Paint))
            {
                return Err("Live GPU Paint override is incompatible with the selected mask layer".into());
            }
            for rect in override_damage {
                add_regions(&mut changed_regions, rect.intersect(&full_bounds(size)));
            }
            if !override_damage.is_empty() {
                changed_layer_index = Some(changed_layer_index.map_or(layer_index, |current: usize| current.min(layer_index)));
            }
        }

        let channels = stack.base.channels();
        if channels == 0 {
            return Err("Mask base surface has no channels".into());
        }
        let base_dirty = sync_cached_surface(device, queue, size, &stack.base, &mut cache.base, false)?;
        let base_changed = !base_dirty.is_empty();
        if base_changed {
            for rect in base_dirty {
                add_regions(&mut changed_regions, rect);
            }
            // The base precedes every effect layer, including a Blur at stack index zero.
            changed_layer_index = Some(usize::MAX);
        }
        let checkpoint_blur_index = stack
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                (layer.visible && layer.opacity > 0.0 && matches!(&layer.kind, EffectKind::Blur { radius } if *radius > 0)).then_some(index)
            })
            .next();
        let config_changed = cache.stack_key != Some(stack_key);
        // `render_cached` keeps the caller's material target stable for the same owner key.
        // Avoid replaying the entire GPU stack when neither inputs nor configuration changed.
        if !config_changed && !base_changed && !shell_changed && changed_regions.is_empty() {
            return Ok(());
        }
        let can_resume_blur_prefix = !config_changed
            && !base_changed
            && !shell_changed
            && cache.blur_checkpoint_key == Some(stack_key)
            && cache.blur_checkpoint_after.is_some_and(|checkpoint| changed_layer_index.is_some_and(|changed| changed > checkpoint))
            && !regions_are_full(&changed_regions, full_bounds(size));
        let can_update_local_stack =
            !config_changed && !shell_changed && changed_layer_index.is_some() && !regions_are_full(&changed_regions, full_bounds(size));
        let local_stack_regions = if can_update_local_stack {
            let changed_at = changed_layer_index.ok_or("Missing changed mask layer")?;
            Some(stack_update_regions(stack, changed_at, &changed_regions, size, can_resume_blur_prefix.then_some(cache.blur_checkpoint_after).flatten()))
        } else {
            None
        };
        let full_render = !(can_resume_blur_prefix || can_update_local_stack);
        let damage = if full_render {
            None
        } else if let Some((final_regions, _, _)) = &local_stack_regions {
            Some(final_regions.clone())
        } else {
            Some(changed_regions.clone())
        };
        if !full_render && damage.as_ref().is_none_or(Vec::is_empty) {
            return Ok(());
        }
        let shell_view = cache.shell.as_ref().unwrap_or(&cache.dummy_shell).create_view(&Default::default());
        let scratch_views: Vec<_> = cache.scratch.iter().map(|texture| texture.create_view(&Default::default())).collect();
        let dummy_view = cache.dummy.create_view(&Default::default());
        // Start from the base, or from the cached output after the final lower Blur when
        // only a pointwise suffix changed.
        let mut current = 0usize;
        let initial_texture = if can_resume_blur_prefix { cache.blur_checkpoint.as_ref().ok_or("Missing mask Blur checkpoint")? } else { &cache.base.texture };
        let base_damage = local_stack_regions.as_ref().map(|(_, base_region, _)| base_region.as_slice()).or(damage.as_deref());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mask_stack_render") });
        for rect in base_damage.unwrap_or(&[full_bounds(size)]) {
            copy_region(&mut encoder, initial_texture, &cache.scratch[0], *rect);
        }
        for (layer_index, opacity, mode, layer) in layers {
            if can_resume_blur_prefix && cache.blur_checkpoint_after.is_some_and(|checkpoint| layer_index <= checkpoint) {
                continue;
            }
            let layer_damage =
                local_stack_regions.as_ref().map(|(_, _, regions)| regions.get(layer_index).map(Vec::as_slice).unwrap_or(&[])).or(damage.as_deref());
            match layer {
                MaskLayer::Paint(index) => {
                    let paint_view = brush_override
                        .filter(|(override_layer, _, _)| *override_layer == layer_index)
                        .map(|(_, texture, _)| texture.create_view(&Default::default()))
                        .or_else(|| cache.paints.get(index).and_then(Option::as_ref).map(|surface| surface.texture.create_view(&Default::default())))
                        .ok_or("Missing cached Paint texture")?;
                    let dst = next_slot(&[current]);
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[current],
                        &paint_view,
                        &shell_view,
                        &scratch_views[dst],
                        size,
                        params(size, 0, 0, opacity, mode, [0.0; 6]),
                        layer_damage,
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    current = dst;
                }
                MaskLayer::Fill { value, bounds, has_bounds, rotation, repeat_u, repeat_v, mirror_u, mirror_v, map_index } => {
                    let dst = next_slot(&[current]);
                    let fill_view = map_index
                        .and_then(|index| cache.fills.get(index).and_then(Option::as_ref))
                        .map(|cached| cached.texture.create_view(&Default::default()));
                    let fill_view = fill_view.as_ref().unwrap_or(&dummy_view);
                    let fill_size = map_index.and_then(|index| cache.fills.get(index).and_then(Option::as_ref)).map_or([0, 0], |cached| cached.size);
                    let flags = u32::from(repeat_u && map_index.is_some())
                        | (u32::from(repeat_v && map_index.is_some()) << 1)
                        | (u32::from(mirror_u) << 2)
                        | (u32::from(mirror_v) << 3)
                        | (u32::from(has_bounds) << 4)
                        | (u32::from(map_index.is_some()) << 5);
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[current],
                        fill_view,
                        &shell_view,
                        &scratch_views[dst],
                        size,
                        fill_params(size, opacity, mode, value, bounds, rotation, fill_size, flags),
                        layer_damage,
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    current = dst;
                }
                MaskLayer::Levels(levels) => {
                    let dst = next_slot(&[current]);
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[current],
                        &dummy_view,
                        &shell_view,
                        &scratch_views[dst],
                        size,
                        params(size, 2, 0, opacity, mode, [0.0, levels[0], levels[1], levels[2], levels[3], levels[4]]),
                        layer_damage,
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    current = dst;
                }
                MaskLayer::Blur(radius) if radius > 0 => {
                    let original = current;
                    let horizontal = next_slot(&[original]);
                    let blur_region = layer_damage;
                    let horizontal_damage = blur_region.map(|regions| expanded_regions(regions, 0, i32::from(radius), full_bounds(size)));
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[original],
                        &dummy_view,
                        &shell_view,
                        &scratch_views[horizontal],
                        size,
                        params(size, 3, u32::from(radius), opacity, mode, [0.0; 6]),
                        horizontal_damage.as_deref(),
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    let vertical = next_slot(&[original, horizontal]);
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[horizontal],
                        &dummy_view,
                        &shell_view,
                        &scratch_views[vertical],
                        size,
                        params(size, 4, u32::from(radius), opacity, mode, [0.0; 6]),
                        blur_region,
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    let mixed = next_slot(&[original, horizontal, vertical]);
                    Self::draw_stage(
                        &self.pipeline,
                        &self.params_layout,
                        &self.textures_layout,
                        device,
                        &mut encoder,
                        &scratch_views[vertical],
                        &scratch_views[original],
                        &shell_view,
                        &scratch_views[mixed],
                        size,
                        params(size, 5, 0, opacity, mode, [0.0; 6]),
                        blur_region,
                        &self.kernel_weights,
                        &self.dummy_coverage,
                    )?;
                    current = mixed;
                }
                MaskLayer::Blur(_) => {}
            }
            if !can_resume_blur_prefix && checkpoint_blur_index == Some(layer_index) {
                for rect in layer_damage.unwrap_or(&[full_bounds(size)]) {
                    copy_region(&mut encoder, &cache.scratch[current], cache.blur_checkpoint.as_ref().ok_or("Missing mask Blur checkpoint")?, *rect);
                }
                cache.blur_checkpoint_after = Some(layer_index);
                cache.blur_checkpoint_key = Some(stack_key);
            }
        }
        for rect in damage.as_deref().unwrap_or(&[full_bounds(size)]) {
            copy_region(&mut encoder, &cache.scratch[current], target, *rect);
        }
        cache.stack_key = Some(stack_key);
        queue.submit([encoder.finish()]);
        Ok(())
    }

    fn draw_stage(
        pipeline: &wgpu::RenderPipeline,
        params_layout: &wgpu::BindGroupLayout,
        textures_layout: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        auxiliary: &wgpu::TextureView,
        shells: &wgpu::TextureView,
        target: &wgpu::TextureView,
        size: [u32; 2],
        params: [u8; 80],
        damage: Option<&[Rect]>,
        kernel_weights: &wgpu::Buffer,
        coverage_buffer: &wgpu::Buffer,
    ) -> Result<(), String> {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pc_mask_stack_params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let params_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_mask_stack_params"),
            layout: params_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let textures_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_mask_stack_inputs"),
            layout: textures_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(auxiliary) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(shells) },
                wgpu::BindGroupEntry { binding: 3, resource: kernel_weights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: coverage_buffer.as_entire_binding() },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pc_mask_stack_stage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: if damage.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &params_group, &[]);
            pass.set_bind_group(1, &textures_group, &[]);
            if let Some(regions) = damage {
                for rect in regions {
                    pass.set_scissor_rect(rect.x0 as u32, rect.y0 as u32, rect.width(), rect.height());
                    pass.draw(0..3, 0..1);
                }
            } else {
                pass.draw(0..3, 0..1);
            }
        }
        let _ = size;
        Ok(())
    }
}

enum MaskLayer {
    Paint(usize),
    Fill {
        value: f32,
        bounds: [i32; 4],
        has_bounds: bool,
        rotation: f32,
        repeat_u: bool,
        repeat_v: bool,
        mirror_u: bool,
        mirror_v: bool,
        map_index: Option<usize>,
    },
    Levels([f32; 5]),
    Blur(u8),
}

fn create_render_cache(device: &wgpu::Device, size: [u32; 2], owner_key: u64) -> RenderCache {
    RenderCache {
        owner_key,
        size,
        base: empty_cached_surface(device, size, "pc_mask_base_input"),
        paints: Vec::new(),
        fills: Vec::new(),
        shell_key: None,
        shell: None,
        dummy_shell: rgba32u_texture(device, [1, 1], "pc_mask_stack_dummy_shell"),
        scratch: (0..2).map(|_| rgba16_texture(device, size, "pc_mask_stack_ping_pong")).collect(),
        dummy: rgba16_texture(device, [1, 1], "pc_mask_stack_dummy"),
        stack_key: None,
        blur_checkpoint: None,
        blur_checkpoint_after: None,
        blur_checkpoint_key: None,
    }
}

fn create_cached_fill(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], surface: &Surface) -> Result<CachedFill, String> {
    let texture = r32_texture(device, size, "pc_mask_fill_map");
    let mut cached = CachedFill { texture, size, format: surface_format_placeholder(), default_pixel: Vec::new(), tiles: BTreeMap::new() };
    sync_cached_fill(queue, surface, &mut cached)?;
    Ok(cached)
}

fn empty_cached_surface(device: &wgpu::Device, size: [u32; 2], label: &str) -> CachedSurface {
    CachedSurface { texture: rgba16_texture(device, size, label), format: surface_format_placeholder(), default_pixel: Vec::new(), tiles: BTreeMap::new() }
}

fn r32_texture(device: &wgpu::Device, size: [u32; 2], label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn sync_cached_fill(queue: &wgpu::Queue, surface: &Surface, cached: &mut CachedFill) -> Result<Vec<Rect>, String> {
    let default_pixel = surface.default_pixel();
    let bounds = full_bounds(cached.size);
    let full = cached.tiles.is_empty() && cached.default_pixel.is_empty() || cached.format != surface.format() || cached.default_pixel != default_pixel;
    let mut dirty = Vec::new();
    if full {
        write_fill_region(queue, &cached.texture, surface, bounds)?;
        dirty.push(bounds);
        cached.tiles = surface.tiles().map(|(coord, tile)| (*coord, tile.clone())).collect();
    } else {
        let removed: Vec<_> = cached.tiles.keys().copied().filter(|coord| surface.tile(*coord).is_none()).collect();
        for coord in removed {
            let rect = coord.rect().intersect(&bounds);
            if !rect.is_empty() {
                write_fill_region(queue, &cached.texture, surface, rect)?;
                add_regions(&mut dirty, rect);
            }
            cached.tiles.remove(&coord);
        }
        for (coord, tile) in surface.tiles() {
            if cached.tiles.get(coord).is_none_or(|old| !Arc::ptr_eq(old, tile)) {
                let rect = coord.rect().intersect(&bounds);
                if !rect.is_empty() {
                    write_fill_region(queue, &cached.texture, surface, rect)?;
                    add_regions(&mut dirty, rect);
                }
                cached.tiles.insert(*coord, tile.clone());
            }
        }
    }
    cached.format = surface.format();
    cached.default_pixel = default_pixel;
    Ok(dirty)
}

fn write_fill_region(queue: &wgpu::Queue, texture: &wgpu::Texture, surface: &Surface, rect: Rect) -> Result<(), String> {
    let channels = surface.channels();
    if channels == 0 {
        return Err("Mask Fill image has no channels".into());
    }
    let values = surface.try_read_region(rect).map_err(|error| error.to_string())?;
    let bytes: Vec<u8> = values.chunks_exact(channels).flat_map(|pixel| pixel[0].to_le_bytes()).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rect.width() as u32 * 4), rows_per_image: Some(rect.height() as u32) },
        wgpu::Extent3d { width: rect.width() as u32, height: rect.height() as u32, depth_or_array_layers: 1 },
    );
    Ok(())
}

fn surface_format_placeholder() -> photocraft_color::PixelFormat {
    photocraft_color::PixelFormat { mode: photocraft_color::ColorMode::Grayscale, sample: photocraft_color::SampleType::F32, alpha: true }
}

fn rgba8_compatible(surface: &Surface) -> bool {
    use photocraft_color::{ColorMode, SampleType};
    let format = surface.format();
    format.mode == ColorMode::Rgb && format.sample == SampleType::U8 && format.alpha && format.channels() == 4
}

fn create_cached_surface(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], surface: &Surface, label: &str) -> Result<CachedSurface, String> {
    let mut cached = empty_cached_surface(device, size, label);
    sync_cached_surface(device, queue, size, surface, &mut cached, true)?;
    Ok(cached)
}

fn sync_cached_surface(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: [u32; 2],
    surface: &Surface,
    cached: &mut CachedSurface,
    compact_paint: bool,
) -> Result<Vec<Rect>, String> {
    let default_pixel = surface.default_pixel();
    let full = cached.tiles.is_empty() && cached.default_pixel.is_empty() || cached.format != surface.format() || cached.default_pixel != default_pixel;
    let bounds = full_bounds(size);
    let requested_format = if compact_paint && rgba8_compatible(surface) { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::Rgba16Float };
    if cached.texture.format() != requested_format {
        cached.texture = if requested_format == wgpu::TextureFormat::Rgba8Unorm {
            rgba8_texture(device, size, "pc_mask_paint_input_rgba8")
        } else {
            rgba16_texture(device, size, "pc_mask_surface_input")
        };
        cached.tiles.clear();
        cached.default_pixel.clear();
    }
    let mut dirty = Vec::new();
    if full {
        write_surface_region(queue, &cached.texture, surface, bounds)?;
        dirty.push(bounds);
        cached.tiles = surface.tiles().map(|(coord, tile)| (*coord, tile.clone())).collect();
    } else {
        let removed: Vec<_> = cached.tiles.keys().copied().filter(|coord| surface.tile(*coord).is_none()).collect();
        for coord in removed {
            let rect = coord.rect().intersect(&bounds);
            if !rect.is_empty() {
                write_surface_region(queue, &cached.texture, surface, rect)?;
                add_regions(&mut dirty, rect);
            }
            cached.tiles.remove(&coord);
        }
        for (coord, tile) in surface.tiles() {
            if cached.tiles.get(coord).is_none_or(|old| !Arc::ptr_eq(old, tile)) {
                let rect = coord.rect().intersect(&bounds);
                if !rect.is_empty() {
                    write_surface_region(queue, &cached.texture, surface, rect)?;
                    add_regions(&mut dirty, rect);
                }
                cached.tiles.insert(*coord, tile.clone());
            }
        }
    }
    cached.format = surface.format();
    cached.default_pixel = default_pixel;
    Ok(dirty)
}

fn full_bounds(size: [u32; 2]) -> Rect {
    Rect::new(0, 0, size[0] as i32, size[1] as i32)
}

fn expand_rect(rect: Rect, x: i32, y: i32, bounds: Rect) -> Rect {
    Rect::new(rect.x0.saturating_sub(x), rect.y0.saturating_sub(y), rect.x1.saturating_add(x), rect.y1.saturating_add(y)).intersect(&bounds)
}

/// The final affected region grows only through Blurs above the changed layer. Rebuilding from
/// the base requires intermediate input over the halo of every Blur in the stack.
fn add_regions(regions: &mut Vec<Rect>, rect: Rect) {
    if rect.is_empty() {
        return;
    }
    let mut merged = rect;
    let mut index = 0;
    while index < regions.len() {
        let current = regions[index];
        let overlaps = merged.x0 < current.x1 && current.x0 < merged.x1 && merged.y0 < current.y1 && current.y0 < merged.y1;
        if overlaps {
            merged = merged.union(&current);
            regions.swap_remove(index);
            index = 0;
        } else {
            index += 1;
        }
    }
    regions.push(merged);
}

fn regions_are_full(regions: &[Rect], full: Rect) -> bool {
    regions.len() == 1 && regions[0] == full
}

fn expanded_regions(regions: &[Rect], x: i32, y: i32, bounds: Rect) -> Vec<Rect> {
    let mut expanded = Vec::new();
    for rect in regions {
        add_regions(&mut expanded, expand_rect(*rect, x, y, bounds));
    }
    expanded
}

fn stack_update_regions(
    stack: &Stack,
    changed_at: usize,
    changed: &[Rect],
    size: [u32; 2],
    resume_after: Option<usize>,
) -> (Vec<Rect>, Vec<Rect>, Vec<Vec<Rect>>) {
    let bounds = full_bounds(size);
    let blur_radius = |layer: &photocraft_doc::mask_paint::Paint| {
        if !layer.visible || layer.opacity <= 0.0 {
            0
        } else if let EffectKind::Blur { radius } = &layer.kind {
            i32::from(*radius)
        } else {
            0
        }
    };
    let suffix_halo = stack
        .layers
        .iter()
        .enumerate()
        .filter(|(index, _)| changed_at == usize::MAX || *index > changed_at)
        .map(|(_, layer)| blur_radius(layer))
        .fold(0, i32::saturating_add);
    let total_halo = stack
        .layers
        .iter()
        .enumerate()
        .filter(|(index, _)| resume_after.is_none_or(|checkpoint| *index > checkpoint))
        .map(|(_, layer)| blur_radius(layer))
        .fold(0, i32::saturating_add);
    let mut final_regions = Vec::new();
    let mut base_regions = Vec::new();
    let mut regions = vec![Vec::new(); stack.layers.len()];
    for changed in changed {
        let final_region = expand_rect(*changed, suffix_halo, suffix_halo, bounds);
        add_regions(&mut final_regions, final_region);
        add_regions(&mut base_regions, expand_rect(final_region, total_halo, total_halo, bounds));
        let mut required = vec![final_region];
        for (index, layer) in stack.layers.iter().enumerate().rev() {
            if resume_after.is_some_and(|checkpoint| index <= checkpoint) || !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            for region in &required {
                add_regions(&mut regions[index], *region);
            }
            let radius = blur_radius(layer);
            if radius > 0 {
                required = expanded_regions(&required, radius, radius, bounds);
            }
        }
    }
    (final_regions, base_regions, regions)
}

fn copy_region(encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, target: &wgpu::Texture, rect: Rect) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d { width: rect.width(), height: rect.height(), depth_or_array_layers: 1 },
    );
}

fn stack_config_key(stack: &Stack) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stack.layers.len().hash(&mut hasher);
    stack.base.format().hash(&mut hasher);
    stack.base.channels().hash(&mut hasher);
    for layer in &stack.layers {
        layer.visible.hash(&mut hasher);
        layer.opacity.to_bits().hash(&mut hasher);
        layer.blend.hash(&mut hasher);
        match &layer.kind {
            EffectKind::Paint => {
                0u8.hash(&mut hasher);
                layer.surface.format().hash(&mut hasher);
                layer.surface.channels().hash(&mut hasher);
            }
            EffectKind::Fill { value, rotation, repeat_u, repeat_v, mirror_u, mirror_v } => {
                1u8.hash(&mut hasher);
                value.to_bits().hash(&mut hasher);
                rotation.to_bits().hash(&mut hasher);
                repeat_u.hash(&mut hasher);
                repeat_v.hash(&mut hasher);
                mirror_u.hash(&mut hasher);
                mirror_v.hash(&mut hasher);
                layer.fill_map.is_some().hash(&mut hasher);
                layer.fill_map_size.hash(&mut hasher);
                layer.fill_bounds.hash(&mut hasher);
            }
            EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                2u8.hash(&mut hasher);
                input_black.to_bits().hash(&mut hasher);
                gamma.to_bits().hash(&mut hasher);
                input_white.to_bits().hash(&mut hasher);
                output_black.to_bits().hash(&mut hasher);
                output_white.to_bits().hash(&mut hasher);
            }
            EffectKind::Blur { radius } => {
                3u8.hash(&mut hasher);
                radius.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn write_surface_region(queue: &wgpu::Queue, texture: &wgpu::Texture, surface: &Surface, rect: Rect) -> Result<(), String> {
    use half::f16;
    let channels = surface.channels();
    if channels == 0 {
        return Err("Mask surface has no channels".into());
    }
    let values = surface.try_read_region(rect).map_err(|error| error.to_string())?;
    if texture.format() == wgpu::TextureFormat::Rgba8Unorm {
        if channels != 4 {
            return Err("Compact GPU Paint texture requires four channels".into());
        }
        let bytes: Vec<u8> = values.chunks_exact(4).flat_map(|pixel| pixel.iter().map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rect.width() as u32 * 4), rows_per_image: Some(rect.height() as u32) },
            wgpu::Extent3d { width: rect.width() as u32, height: rect.height() as u32, depth_or_array_layers: 1 },
        );
        return Ok(());
    }
    let mut bytes = Vec::with_capacity(rect.width() as usize * rect.height() as usize * 8);
    for pixel in values.chunks_exact(channels) {
        let alpha = if channels >= 4 { pixel[3] } else { 1.0 };
        for value in [pixel[0], 0.0, 0.0, alpha] {
            bytes.extend_from_slice(&f16::from_f32(value).to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: rect.x0 as u32, y: rect.y0 as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rect.width() as u32 * 8), rows_per_image: Some(rect.height() as u32) },
        wgpu::Extent3d { width: rect.width() as u32, height: rect.height() as u32, depth_or_array_layers: 1 },
    );
    Ok(())
}

fn next_slot(used: &[usize]) -> usize {
    (0..4).find(|slot| !used.contains(slot)).unwrap_or(0)
}

fn mask_blend_code(mode: photocraft_color::BlendMode) -> Option<i32> {
    use photocraft_color::BlendMode::*;
    Some(match mode {
        Normal => 1,
        Darken => 3,
        Multiply => 4,
        Lighten => 8,
        Screen => 9,
        LinearDodge => 11,
        Difference => 20,
        Subtract => 22,
        _ => return None,
    })
}

fn params(size: [u32; 2], stage: u32, radius: u32, opacity: f32, blend: i32, values: [f32; 6]) -> [u8; 80] {
    let mut bytes = [0; 80];
    for (index, value) in [size[0], size[1], radius, stage].into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[16..20].copy_from_slice(&opacity.to_le_bytes());
    bytes[20..24].copy_from_slice(&blend.to_le_bytes());
    let floats = values;
    for (index, value) in floats.into_iter().enumerate() {
        let offset = 24 + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod mask_stack_gpu_tests {
    use super::*;

    #[test]
    fn disjoint_damage_regions_stay_separate_until_blur_halos_overlap() {
        let bounds = Rect::new(0, 0, 1024, 256);
        let a = Rect::new(0, 0, 256, 256);
        let b = Rect::new(768, 0, 1024, 256);
        let mut regions = Vec::new();
        add_regions(&mut regions, a);
        add_regions(&mut regions, b);
        assert_eq!(regions.len(), 2);
        let expanded = expanded_regions(&regions, 32, 32, bounds);
        assert_eq!(expanded.len(), 2);
        let touching = expanded_regions(&regions, 257, 0, bounds);
        assert_eq!(touching.len(), 1);
        assert_eq!(touching[0], bounds);
    }
}

fn fill_params(size: [u32; 2], opacity: f32, blend: i32, value: f32, bounds: [i32; 4], rotation: f32, map_size: [u32; 2], flags: u32) -> [u8; 80] {
    let mut bytes = params(size, 6, 0, opacity, blend, [value, 0.0, 0.0, 0.0, 0.0, 0.0]);
    for (index, bound) in bounds.into_iter().enumerate() {
        let offset = 48 + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&bound.to_le_bytes());
    }
    bytes[64..68].copy_from_slice(&map_size[0].to_le_bytes());
    bytes[68..72].copy_from_slice(&map_size[1].to_le_bytes());
    bytes[72..76].copy_from_slice(&flags.to_le_bytes());
    bytes[76..80].copy_from_slice(&rotation.to_le_bytes());
    bytes
}

fn dab_params(size: [u32; 2], dab: MaskRoundDab, rect: Rect, radius: f32, core: f32, shoulder: f32) -> [u8; 80] {
    let mut bytes = params(size, 7, 0, 0.0, 1, [dab.center[0], dab.center[1], radius, dab.alpha, dab.opacity, core]);
    for (index, value) in [rect.x0, rect.y0, rect.x1, rect.y1].into_iter().enumerate() {
        let offset = 48 + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[76..80].copy_from_slice(&shoulder.to_le_bytes());
    bytes
}

fn brush_composite_params(size: [u32; 2], settings: MaskBrushSettings) -> [u8; 80] {
    params(size, 8, 0, settings.stroke_opacity, 1, [settings.color[0], settings.color[1], settings.color[2], settings.color[3], 0.0, 0.0])
}

fn round_dab_parameters(dab: MaskRoundDab, hardness: f32, size: [u32; 2]) -> Result<(Rect, f32, f32, f32), String> {
    if dab.center.iter().any(|value| !value.is_finite())
        || !dab.radius.is_finite()
        || !dab.alpha.is_finite()
        || !(0.0..=1.0).contains(&dab.alpha)
        || !dab.opacity.is_finite()
        || !(0.0..=1.0).contains(&dab.opacity)
        || !hardness.is_finite()
        || !(0.0..=1.0).contains(&hardness)
    {
        return Err("GPU round dab has invalid values".into());
    }
    let radius = dab.radius.max(0.5);
    if radius < 3.0 {
        return Err("GPU round dabs below 3 px require CPU small-tip supersampling".into());
    }
    let h = hardness * 10.0;
    let shape_index = (h.floor() as usize).min(9);
    let t = h - shape_index as f32;
    const TIP_SHAPE: [(f32, f32); 11] = [
        (0.00, 0.664),
        (0.14, 0.58),
        (0.28, 0.50),
        (0.40, 0.44),
        (0.52, 0.36),
        (0.63, 0.29),
        (0.70, 0.24),
        (0.78, 0.19),
        (0.84, 0.15),
        (0.89, 0.11),
        (1.00, 0.0),
    ];
    let (r_a, s_a) = TIP_SHAPE[shape_index];
    let (r_b, s_b) = TIP_SHAPE[shape_index + 1];
    let edge = 1.2f32.min(0.3 * radius).max(0.25);
    let core = ((r_a + (r_b - r_a) * t) * radius).min(radius - edge).max(0.0);
    let shoulder = ((s_a + (s_b - s_a) * t) * radius).max(edge).max(1e-3);
    let reach = core + 2.6 * shoulder;
    let rr = (reach.ceil() as i32).saturating_add(1);
    let cx = dab.center[0].floor() as i32;
    let cy = dab.center[1].floor() as i32;
    let rect = Rect::new(cx.saturating_sub(rr), cy.saturating_sub(rr), cx.saturating_add(rr).saturating_add(1), cy.saturating_add(rr).saturating_add(1))
        .intersect(&full_bounds(size));
    Ok((rect, core, shoulder, radius))
}

fn gaussian_kernel_weights() -> Vec<f32> {
    const STRIDE: usize = 129;
    let mut weights = vec![0.0; 65 * STRIDE];
    for radius in 0..=64usize {
        let sigma = (radius as f32 / 3.0).max(0.5);
        let mut sum = 0.0;
        for delta in -(radius as i32)..=radius as i32 {
            let d = delta as f32 / sigma;
            let weight = (-0.5 * d * d).exp();
            weights[radius * STRIDE + (delta + 64) as usize] = weight;
            sum += weight;
        }
        for weight in &mut weights[radius * STRIDE..(radius + 1) * STRIDE] {
            *weight /= sum;
        }
    }
    weights
}

fn rgba16_texture(device: &wgpu::Device, size: [u32; 2], label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn rgba8_texture(device: &wgpu::Device, size: [u32; 2], label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn upload_shell_ids(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], ids: &[u32]) -> wgpu::Texture {
    let texture = rgba32u_texture(device, size, "pc_mask_shell_ids");
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytemuck::cast_slice(ids),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * 4), rows_per_image: Some(size[1]) },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    texture
}

fn rgba32u_texture(device: &wgpu::Device, size: [u32; 2], label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
