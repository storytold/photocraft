//! Dab stamping into a GPU-owned layer mask (the preview-only prototype of #17133).
//!
//! A live brush stroke shows as a colour layer through a mask. Here the mask never exists on the
//! CPU: [`Compositor::begin_external_mask`] gives a layer a mask the compositor owns, one texture
//! per page cell the stroke reaches (created as [`Compositor::stamp_dabs`] meets the cell, zeroed),
//! and `stamp_dabs` draws each dab into it as one instanced quad with a fragment shader that
//! evaluates the CPU rasteriser's round tip (`stamp.wgsl`). The planner binds the mask like any
//! other layer mask and [`Compositor::sync`] leaves it alone: it never uploads to it, and a layer
//! whose document mask surface is empty still takes the external texture.
//!
//! The texture holds the stroke buffer's raw coverage `c`: the blend (SrcAlpha, OneMinusSrcAlpha
//! with colour = the dab's opacity ceiling and alpha = flow × tip value `v`) is
//! `c ← c·(1 − v) + ceil·v`, which is `CoverageMap::accumulate` when `ceil ≥ c`. With a constant
//! ceiling (a plain brush) that always holds. The layer's own opacity then carries the brush
//! opacity, as the stroke's mask `min(c · opacity, 1)` does for Normal mode.
//!
//! Fragment passes only: nothing here needs compute shaders or storage buffers, so the web build
//! runs it on WebGL2.
use std::collections::BTreeMap;

use super::consts::STRIDE;
use super::paging::{cell_rect, first_error};
use super::textures::{Cell, Resident, TexKind};
use super::{Compositor, DocId, HashMap, LayerId, PixelFormat, Rect, Role, Unsupported};

/// The precision an external mask is stored in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskPrecision {
    /// R32F: needs the device's `FLOAT32_BLENDABLE` feature.
    Full,
    /// R16F: about 3 decimal digits, blendable everywhere.
    Half,
}

impl MaskPrecision {
    fn kind(self) -> TexKind {
        match self {
            MaskPrecision::Full => TexKind::R32F,
            MaskPrecision::Half => TexKind::R16F,
        }
    }
}

/// One dab as the stamping shader takes it: `photocraft_paint::TipGeometry` field for field (see
/// its documentation for what each number means).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuDab {
    /// The pixels the dab can touch.
    pub rect: Rect,
    pub cx: f32,
    pub cy: f32,
    pub ro: f32,
    pub core: f32,
    pub shoulder: f32,
    pub reach2: f32,
    pub small_reach2: f32,
    pub cs: f32,
    pub sn: f32,
    pub fx: f32,
    pub fy: f32,
    pub proj: [f32; 3],
    pub small: bool,
    pub flow: f32,
    pub ceil: f32,
}

/// Floats in one dab's instance record (six `vec4`s, see `stamp.wgsl`).
const INSTANCE_FLOATS: usize = 24;
const INSTANCE_BYTES: u64 = (INSTANCE_FLOATS * 4) as u64;

impl GpuDab {
    fn instance(&self) -> [f32; INSTANCE_FLOATS] {
        let r = self.rect;
        [
            r.x0 as f32,
            r.y0 as f32,
            r.x1 as f32,
            r.y1 as f32,
            self.cx,
            self.cy,
            self.ro,
            self.flow,
            self.core,
            self.shoulder,
            self.reach2,
            self.small_reach2,
            self.cs,
            self.sn,
            self.fx,
            self.fy,
            self.proj[0],
            self.proj[1],
            self.proj[2],
            if self.small { 1.0 } else { 0.0 },
            self.ceil,
            0.0,
            0.0,
            0.0,
        ]
    }
}

/// What a [`Compositor::stamp_dabs`] call recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StampStats {
    /// Dab quads drawn (a dab over several cells counts once per cell).
    pub quads: usize,
    /// Render passes recorded (one per cell the dabs reach).
    pub passes: usize,
    /// Cell textures created by this call.
    pub cells_created: usize,
}

/// A layer's external mask.
#[derive(Clone, Copy, Debug)]
pub(crate) struct External {
    doc: DocId,
    /// The canvas rounded out to tiles: cells are created over it only.
    canvas: Rect,
    kind: TexKind,
}

/// The stamping shader and the pipelines built from it, one per mask format.
pub(crate) struct Stamper {
    module: wgpu::ShaderModule,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

/// The stamping shader's WGSL source.
pub const STAMP_SHADER: &str = include_str!("stamp.wgsl");

impl Stamper {
    fn new(device: &wgpu::Device) -> Self {
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_stamp"), source: wgpu::ShaderSource::Wgsl(STAMP_SHADER.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_stamp_cell"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(16),
                },
                count: None,
            }],
        });
        let layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pc_stamp"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        Stamper { module, bgl, layout, pipelines: HashMap::new() }
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> Result<wgpu::RenderPipeline, Unsupported> {
        if let Some(p) = self.pipelines.get(&format) {
            return Ok(p.clone());
        }
        let attributes: Vec<wgpu::VertexAttribute> =
            (0..6).map(|i| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16 * i as u64, shader_location: i }).collect();
        let blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let (pipeline, error) = first_error(device, || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pc_stamp"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_stamp"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: INSTANCE_BYTES,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attributes,
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_stamp"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState { color: blend, alpha: blend }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        if let Some(e) = error {
            return Err(Unsupported(format!("couldn't build the dab stamping pipeline for {format:?}: {e}")));
        }
        self.pipelines.insert(format, pipeline.clone());
        Ok(pipeline)
    }
}

impl Compositor {
    /// Whether the device blends into R32F targets (`FLOAT32_BLENDABLE`), so an external mask can
    /// be [`MaskPrecision::Full`].
    pub fn full_precision_masks(device: &wgpu::Device) -> bool {
        device.features().contains(wgpu::Features::FLOAT32_BLENDABLE)
    }

    /// Give `layer`'s mask to the compositor: the planner binds a texture per page cell the
    /// stroke stamped into (see [`Compositor::stamp_dabs`]) instead of the document mask's
    /// surface, which stays as it is (the layer still needs an enabled mask, which reads as its
    /// default where no cell exists). Cells exist over `canvas` only. `precision` is the
    /// precision asked for (`None`: the best the device blends); the one used is returned, which
    /// is [`MaskPrecision::Half`] where the device cannot blend R32F. Replaces an earlier
    /// external mask of the layer, dropping its cells.
    pub fn begin_external_mask(&mut self, device: &wgpu::Device, doc: DocId, layer: LayerId, canvas: Rect, precision: Option<MaskPrecision>) -> MaskPrecision {
        let used = match precision {
            Some(MaskPrecision::Half) => MaskPrecision::Half,
            _ if Self::full_precision_masks(device) => MaskPrecision::Full,
            _ => MaskPrecision::Half,
        };
        self.drop_external_cells(layer);
        let t = super::batch::tile_grid(canvas);
        self.external.insert(layer, External { doc, canvas: t, kind: used.kind() });
        self.prefix.clear();
        used
    }

    /// Drop `layer`'s external mask: its cells are freed and the planner binds the document's
    /// mask again.
    pub fn clear_external_mask(&mut self, layer: LayerId) {
        self.external.remove(&layer);
        self.drop_external_cells(layer);
        self.prefix.clear();
    }

    fn drop_external_cells(&mut self, layer: LayerId) {
        self.residents.retain(|k, _| !(k.0 == layer && k.1 == Role::Mask));
    }

    /// Whether `layer` has an external mask.
    pub fn has_external_mask(&self, layer: LayerId) -> bool {
        self.external.contains_key(&layer)
    }

    /// The textures of `layer`'s external mask: each cell's document rect and its texture (R32F or
    /// R16F, readable with a copy), in no particular order.
    pub fn external_mask_cells(&self, layer: LayerId) -> Vec<(Rect, &wgpu::Texture)> {
        self.residents.iter().filter(|(k, _)| k.0 == layer && k.1 == Role::Mask).map(|(_, r)| (r.region, &r.texture)).collect()
    }

    /// The external masks' layers, for the pages that must not be evicted.
    pub(crate) fn is_external_mask(&self, key: &super::ResKey) -> bool {
        key.1 == Role::Mask && self.external.contains_key(&key.0)
    }

    /// Drop the lower-stack composites that include an external mask's layer: its pixels change
    /// without its document layer changing, so the prefix's snapshot comparison cannot see it. A
    /// composite of the layers *below* it stays.
    pub(crate) fn invalidate_external_prefix(&mut self, doc: &super::Document) {
        for layer in self.external.keys() {
            if let Some(root) = doc.path_of(*layer).and_then(|p| p.first().copied()) {
                self.prefix.invalidate_from(root);
            }
        }
    }

    /// Stamp `dabs`, in order, into `layer`'s external mask ([`Compositor::begin_external_mask`]),
    /// recording one pass per cell they reach into `encoder`. Cells are created as the dabs reach
    /// them, zeroed. Dabs outside the canvas are left out. The caller submits `encoder` before
    /// (or with) the render that shows the mask.
    pub fn stamp_dabs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        layer: LayerId,
        dabs: &[GpuDab],
    ) -> Result<StampStats, Unsupported> {
        let Some(ext) = self.external.get(&layer).copied() else {
            return Err(Unsupported("the layer has no external mask (begin_external_mask)".into()));
        };
        let page = self.page as i32;
        // Per cell, the dabs that reach it in stroke order.
        let mut by_cell: BTreeMap<Cell, Vec<usize>> = BTreeMap::new();
        for (i, d) in dabs.iter().enumerate() {
            let r = d.rect.intersect(&ext.canvas);
            if r.is_empty() {
                continue;
            }
            for cy in r.y0.div_euclid(page)..=(r.y1 - 1).div_euclid(page) {
                for cx in r.x0.div_euclid(page)..=(r.x1 - 1).div_euclid(page) {
                    by_cell.entry((cx, cy)).or_default().push(i);
                }
            }
        }
        let mut stats = StampStats::default();
        if by_cell.is_empty() {
            return Ok(stats);
        }
        let format = ext.kind.format();
        let stamper = self.stamper.get_or_insert_with(|| Stamper::new(device));
        let pipeline = stamper.pipeline(device, format)?;
        let bgl = stamper.bgl.clone();
        // Instances of every cell, one after another, and the cell uniforms.
        let mut instances: Vec<f32> = Vec::with_capacity(by_cell.values().map(Vec::len).sum::<usize>() * INSTANCE_FLOATS);
        let mut uniforms = vec![0u8; by_cell.len() * STRIDE as usize];
        let mut draws: Vec<(Cell, std::ops::Range<u64>, u32)> = Vec::new();
        for (n, (cell, list)) in by_cell.iter().enumerate() {
            let region = cell_rect(*cell, self.page).intersect(&ext.canvas);
            if !self.residents.contains_key(&(layer, Role::Mask, *cell)) {
                self.residents.insert((layer, Role::Mask, *cell), external_cell(device, &ext, region));
                stats.cells_created += 1;
            }
            let start = instances.len() as u64 * 4;
            for &i in list {
                instances.extend_from_slice(&dabs[i].instance());
            }
            draws.push((*cell, start..instances.len() as u64 * 4, list.len() as u32));
            let words = [region.x0 as f32, region.y0 as f32, region.width() as f32, region.height() as f32];
            for (k, w) in words.iter().enumerate() {
                if let Some(dst) = uniforms.get_mut(n * STRIDE as usize + k * 4..n * STRIDE as usize + k * 4 + 4) {
                    dst.copy_from_slice(&w.to_le_bytes());
                }
            }
        }
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_stamp_dabs"),
            size: instances.len() as u64 * 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vbuf, 0, &instances.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>());
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_stamp_cells"),
            size: uniforms.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, &uniforms);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_stamp_cells"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: std::num::NonZeroU64::new(16) }),
            }],
        });
        for (n, (cell, range, count)) in draws.iter().enumerate() {
            let Some(resident) = self.residents.get(&(layer, Role::Mask, *cell)) else { continue };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pc_stamp_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resident.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[(n as u64 * STRIDE) as u32]);
            pass.set_vertex_buffer(0, vbuf.slice(range.clone()));
            pass.draw(0..4, 0..*count);
            stats.passes += 1;
            stats.quads += *count as usize;
        }
        Ok(stats)
    }
}

/// A zeroed texture for the external mask's cell `region`.
fn external_cell(device: &wgpu::Device, ext: &External, region: Rect) -> Resident {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_external_mask"),
        size: wgpu::Extent3d { width: region.width().max(1), height: region.height().max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: ext.kind.format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    Resident {
        texture,
        view,
        region,
        kind: ext.kind,
        format: PixelFormat::GRAY8,
        tiles: HashMap::new(),
        default_nonzero: false,
        default_bits: vec![0],
        doc: ext.doc,
        last_used: 0,
        stamp: 0,
        cmyk: 0,
        ordered: false,
    }
}
