//! Reduced Move output sampled from immutable native bytes, including offsets between proxy
//! samples. Byte planes keep 8/16/32-bit sources compact without requiring normalized-16 formats.

use super::{
    Arc, ChannelView, Document, FLOAT, LayerId, OUTPUT, Pipelines, Preview, RenderState, SampleType, TransformSession, UNIFORM_BYTES, Validated, bytes,
    render_pass, shader_source as transform_shader_source, source, supports_channels, validated, wgpu,
};

fn write_bytes(rs: &RenderState, texture: &wgpu::Texture, width: u32, y: u32, rows: u32, plane: u32, data: &[u8]) -> Option<()> {
    rs.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y, z: plane }, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width.checked_mul(4)?), rows_per_image: Some(rows) },
        wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
    );
    Some(())
}

pub(super) fn input_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("native channel Move inputs"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(UNIFORM_BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

pub(super) fn shader_source(multiple: bool) -> Option<String> {
    Some(
        transform_shader_source(multiple)?
            .into_owned()
            .replace("var source: texture_2d<f32>;", "var source: texture_2d_array<u32>;")
            .replace("fn source_at(at: vec2<i32>) -> vec4<f32> { return textureLoad(source, at, 0); }", include_str!("channel_move_native.wgsl")),
    )
}

fn pipeline(device: &wgpu::Device, inputs: &wgpu::BindGroupLayout, multiple: bool) -> Option<wgpu::RenderPipeline> {
    let source = shader_source(multiple)?;
    let shader =
        device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("native channel Move"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("native channel Move"),
        bind_group_layouts: &[Some(inputs)],
        immediate_size: 0,
    });
    Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("native channel Move"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_native_move"),
            targets: &[Some(wgpu::ColorTargetState { format: OUTPUT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    }))
}

impl Preview {
    pub(super) fn capture_native_move(gpu: &crate::gpu_canvas::GpuCanvas, doc: &Document, layer: Option<LayerId>, fill: [f32; 4], factor: u32) -> Option<Self> {
        let (l, beneath) = source(doc, layer)?;
        let surf = l.surface()?;
        let bounds = photocraft_compose::bounds::content_bounds(surf);
        if doc.selection.is_some() || !doc.bounds().contains_rect(&bounds) || !gpu.health().is_ok() || fill.iter().any(|v| !v.is_finite() || v.abs() > 65504.0)
        {
            return None;
        }
        let native_size = [doc.size.width, doc.size.height];
        let size = native_size.map(|n| n.div_ceil(factor));
        let rs = gpu.render_state();
        // Retain the shared channel pipeline's device eligibility. Unsupported adapters
        // use the CPU oracle for both full-size and reduced Move previews.
        if !supports_channels(rs) || factor > i32::MAX as u32 {
            return None;
        }
        let sample_bytes = doc.depth.bytes();
        let limits = rs.device.limits();
        let cost = doc
            .size
            .area()
            .checked_mul(4 * sample_bytes as u64)?
            .checked_add(u64::from(size[0]).checked_mul(u64::from(size[1]))?.checked_mul(4)?)?
            .checked_add(512)?;
        if size.contains(&0)
            || native_size.into_iter().any(|n| n > limits.max_texture_dimension_2d)
            || sample_bytes as u32 > limits.max_texture_array_layers
            || cost > gpu.memory_budget().unwrap_or(256 * 1024 * 1024).min(512 * 1024 * 1024)
        {
            return None;
        }
        let pipes = {
            let mut renderer = rs.renderer.write();
            if let Some(p) = renderer.callback_resources.get::<Arc<Validated<Pipelines>>>() {
                p.clone()
            } else {
                let p = Arc::new(validated(&rs.device, || Pipelines::build(&rs.device)));
                renderer.callback_resources.insert(p.clone());
                p
            }
        };
        let layouts = pipes.get()?;
        let texture = |label, size: [u32; 3], format, usage| {
            rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rgb = texture(
            "native Move bytes",
            [native_size[0], native_size[1], sample_bytes as u32],
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let one = match doc.depth {
            SampleType::U8 => vec![255],
            SampleType::U16 => u16::MAX.to_le_bytes().to_vec(),
            SampleType::F32 => 1.0f32.to_le_bytes().to_vec(),
        };
        let fmt = surf.format();
        for y in (0..native_size[1]).step_by(32) {
            let rows = (native_size[1] - y).min(32);
            let raw = surf.try_to_interleaved(photocraft_geom::Rect::from_xywh(0, y as i32, native_size[0], rows)).ok()?;
            if sample_bytes == 1 && fmt.alpha {
                // The common RGB8 source is already packed exactly as the texture expects.
                // Avoid a float conversion or a per-component copy at first capture.
                if beneath && raw.as_chunks::<4>().0.iter().any(|pixel| pixel.get(3) != Some(&255)) {
                    return None;
                }
                write_bytes(rs, &rgb, native_size[0], y, rows, 0, &raw)?;
                continue;
            }
            let len = (native_size[0] as usize).checked_mul(rows as usize)?.checked_mul(4)?;
            let mut band = Vec::new();
            band.try_reserve_exact(len).ok()?;
            band.resize(len, 0);
            for byte in 0..sample_bytes {
                for (i, pixel) in raw.chunks_exact(fmt.bytes_per_pixel()).enumerate() {
                    if byte == 0 {
                        if fmt.sample == SampleType::F32
                            && pixel.as_chunks::<4>().0.iter().any(|p| {
                                let v = f32::from_le_bytes(*p);
                                !v.is_finite() || v.abs() > 65504.0
                            })
                        {
                            return None;
                        }
                        if fmt.alpha {
                            let a = pixel.get(3 * sample_bytes..)?;
                            if beneath && a != one.as_slice() {
                                return None;
                            }
                            if fmt.sample == SampleType::F32 {
                                let a = f32::from_le_bytes(a.try_into().ok()?);
                                if !(0.0..=1.0).contains(&a) {
                                    return None;
                                }
                            }
                        }
                    }
                    for c in 0..4 {
                        *band.get_mut(i.checked_mul(4)?.checked_add(c)?)? =
                            if c == 3 && !fmt.alpha { *one.get(byte)? } else { *pixel.get(c * sample_bytes + byte)? };
                    }
                }
                write_bytes(rs, &rgb, native_size[0], y, rows, byte as u32, &band)?;
            }
        }
        let selection = texture("unused Move selection", [1, 1, 1], wgpu::TextureFormat::R32Float, wgpu::TextureUsages::TEXTURE_BINDING);
        let moved_texture = texture("unused Move mesh", [1, 1, 1], FLOAT, wgpu::TextureUsages::TEXTURE_BINDING);
        let moved_view = moved_texture.create_view(&Default::default());
        let output = texture(
            "native Move display",
            [size[0], size[1], 1],
            OUTPUT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let output_view = output.create_view(&Default::default());
        let uniform = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native Move parameters"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let inputs = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("native Move source"),
            layout: &layouts.native_inputs,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &rgb.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }),
                    ),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&selection.create_view(&Default::default())) },
            ],
        });
        let moved = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("unused Move result"),
            layout: &layouts.moved,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&moved_view) }],
        });
        let id = rs.renderer.write().register_native_texture(&rs.device, &output_view, wgpu::FilterMode::Linear);
        let buffer = |usage| {
            rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("unused Move mesh"),
                size: 64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let steps = match doc.depth {
            SampleType::U8 => 255.0,
            SampleType::U16 => 65535.0,
            SampleType::F32 => 0.0,
        };
        let fill = fill.map(|v| if steps > 0.0 { (v.clamp(0.0, 1.0) * steps + 0.5).floor() / steps } else { v });
        Some(Self {
            rs: rs.clone(),
            pipes,
            uniform,
            inputs,
            moved,
            moved_view,
            output_view,
            _output: output,
            id,
            size,
            opacity: l.opacity * l.fill_opacity,
            steps,
            fill,
            content: [bounds; 3],
            lifted: [photocraft_geom::Rect::EMPTY; 3],
            selection: None,
            selected: false,
            last: None,
            vertices: buffer(wgpu::BufferUsages::VERTEX),
            indices: buffer(wgpu::BufferUsages::INDEX),
            native: Some([native_size[0], native_size[1], factor]),
        })
    }

    pub(super) fn texture_native_move(
        &mut self,
        t: &TransformSession,
        view: &ChannelView,
        in_color: bool,
        encode: bool,
        factor: u32,
        bounds: photocraft_geom::Rect,
    ) -> Option<egui::TextureId> {
        let [width, height, k] = self.native?;
        if k != factor
            || t.warp.is_some()
            || bounds.is_empty()
            || !photocraft_geom::Rect::from_xywh(0, 0, width, height).contains_rect(&bounds)
            || t.rect != [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(f64::from)
        {
            return None;
        }
        let corners = [[t.rect[0], t.rect[1]], [t.rect[2], t.rect[1]], [t.rect[2], t.rect[3]], [t.rect[0], t.rect[3]]];
        let [dx, dy] = [t.quad[0][0] - t.rect[0], t.quad[0][1] - t.rect[1]];
        if [dx, dy].iter().any(|v| !v.is_finite() || v.abs() > 1e7 || *v != v.round()) || corners.iter().zip(t.quad).any(|([x, y], p)| p != [x + dx, y + dy]) {
            return None;
        }
        if self.last.as_ref().is_some_and(|(old, v, c, e, b)| old == t && v == view && *c == in_color && *e == encode && *b == Some(bounds)) {
            return Some(self.id);
        }
        let colors = photocraft_engine::color_channels::explicit(&serde_json::json!({"target":t.target}), 3).ok()??;
        let c = colors.indices().next()?;
        let mask = colors.indices().fold(0u32, |m, i| m | (1 << i));
        let pipes = self.pipes.get()?;
        let slot = if colors.len() > 1 { &pipes.native_multiple } else { &pipes.native_scalar };
        let pass = slot.get_or_init(|| validated(&self.rs.device, || pipeline(&self.rs.device, &pipes.native_inputs, colors.len() > 1))).get()?.as_ref()?;
        let mut data = [0.0f32; 36];
        data[3] = factor as f32;
        data[4] = dx as f32;
        data[5] = dy as f32;
        data[12..16].copy_from_slice(&[
            width as f32,
            height as f32,
            if colors.len() == 1 { c as f32 } else { mask as f32 },
            if colors.len() == 1 { *self.fill.get(c)? } else { 0.0 },
        ]);
        data[16..20].copy_from_slice(&[bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(|v| v as f32));
        let visible = (0..3).fold(0u32, |v, c| v | (u32::from(view.color_visible(c)) << c));
        data[20..24].copy_from_slice(&[visible as f32, f32::from(in_color), self.opacity, f32::from(encode)]);
        data[24..28].copy_from_slice(&[0.0, 0.0, self.steps, 2.0]);
        data[28..32].copy_from_slice(&[bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(|v| v as f32));
        data[32..36].copy_from_slice(&self.fill);
        self.rs.queue.write_buffer(&self.uniform, 0, &bytes(&data));
        let mut encoder = self.rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("native channel Move") });
        {
            let mut draw = render_pass(&mut encoder, &self.output_view);
            draw.set_pipeline(pass);
            draw.set_bind_group(0, &self.inputs, &[]);
            draw.draw(0..3, 0..1);
        }
        self.rs.queue.submit(Some(encoder.finish()));
        self.last = Some((t.clone(), view.clone(), in_color, encode, Some(bounds)));
        Some(self.id)
    }
}
