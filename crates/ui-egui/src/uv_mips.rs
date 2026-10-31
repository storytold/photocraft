//! GPU mip generation for full-resolution model maps, restricted to changed texels.
use eframe::{egui_wgpu::RenderState, wgpu};
pub(crate) struct Mips {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    color: wgpu::RenderPipeline,
    data: wgpu::RenderPipeline,
    separate: bool,
}
impl Mips {
    pub fn new(rs: &RenderState) -> Self {
        let device = &rs.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv mip"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("uv mip"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("uv mip"), source: wgpu::ShaderSource::Wgsl(crate::gpu_canvas::MIP_WGSL.into()) });
        let pipeline = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("uv mip"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let color = pipeline(wgpu::TextureFormat::Rgba8Unorm);
        let data = pipeline(wgpu::TextureFormat::Rgba16Float);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let info = rs.adapter.get_info();
        let separate = crate::gpu_canvas::separate_mip_targets(info.backend, info.vendor, std::env::var("PHOTOCRAFT_SEPARATE_MIPS").ok().as_deref());
        Self { layout, sampler, color, data, separate }
    }
    pub fn regenerate(&self, rs: &RenderState, texture: &wgpu::Texture, size: [u32; 2], levels: &[wgpu::TextureView], region: [u32; 4]) {
        if levels.len() < 2 {
            return;
        }
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("uv mip update") });
        let [mut x, mut y, w, h] = region;
        let (mut x1, mut y1) = (x.saturating_add(w), y.saturating_add(h));
        for level in 1..levels.len() {
            let Some(previous) = levels.get(level - 1) else { continue };
            let Some(target) = levels.get(level) else { continue };
            let width = (size[0] >> level).max(1);
            let height = (size[1] >> level).max(1);
            x = (x / 2).saturating_sub(1).min(width - 1);
            y = (y / 2).saturating_sub(1).min(height - 1);
            x1 = x1.div_ceil(2).saturating_add(1).min(width);
            y1 = y1.div_ceil(2).saturating_add(1).min(height);
            let group = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uv mip"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(previous) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            let scratch = self.separate.then(|| {
                rs.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("uv mip scratch"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: texture.format(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            });
            let scratch_view = scratch.as_ref().map(|t| t.create_view(&Default::default()));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("uv mip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: scratch_view.as_ref().unwrap_or(target),
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(if texture.format() == wgpu::TextureFormat::Rgba16Float { &self.data } else { &self.color });
                pass.set_bind_group(0, &group, &[]);
                pass.set_scissor_rect(x, y, (x1 - x).max(1), (y1 - y).max(1));
                pass.draw(0..3, 0..1);
            }
            if let Some(scratch) = &scratch {
                let origin = wgpu::Origin3d { x, y, z: 0 };
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { texture: scratch, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyTextureInfo { texture, mip_level: level as u32, origin, aspect: wgpu::TextureAspect::All },
                    wgpu::Extent3d { width: x1 - x, height: y1 - y, depth_or_array_layers: 1 },
                );
            }
        }
        rs.queue.submit([encoder.finish()]);
    }
}
