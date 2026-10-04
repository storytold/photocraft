//! GPU compositor (architecture §7.2, milestone M5).
//!
//! Renders a [`Document`] with wgpu, producing the same straight-alpha composite as the CPU
//! reference (`photocraft-compose`) within ~1/255:
//!
//! - **Residency.** Layer and mask surfaces live on the GPU as dense textures covering their
//!   allocated tiles. Surfaces are copy-on-write `Arc` tiles, so a tile is re-uploaded only when
//!   its `Arc` changed (a brush stroke uploads just the touched 256² tiles; undo swaps pointers
//!   back and uploads only what differs). RGBA8 tiles upload with zero conversion.
//! - **Planner** ([`plan`]). The layer tree becomes a linear list of passes over abstract
//!   chunk-sized buffers (blend with every Photoshop mode, opacity × fill, masks, clipping groups,
//!   pass-through vs isolated groups, adjustments, solid / gradient fills, layer effects).
//! - **Layer effects** (`fx.rs`). Each effect layer's maps (shadow, glow, satin, bevel, stroke
//!   bands) are built by fragment passes over its effect region and cached on the GPU per layer
//!   state; a brush stroke rebuilds only the damaged tiles grown by the effect reach.
//! - **Execution.** The canvas is processed in chunks (1024² RGBA32F accumulators, reused), and
//!   each finished chunk is handed to a caller-supplied sink, e.g. to encode it straight into a
//!   display texture — no readback.
//!
//! Vector masks (rasterised once per mask state into a combined mask texture), layers clipped to
//! pass-through groups, stroked shapes with clipped layers (fill and stroke split once per shape
//! state), pattern fills and artboards are planned like everything else. What remains
//! (Multichannel documents, documents or effect regions larger than the device's texture limit)
//! returns [`Unsupported`]; callers fall back to the CPU compositor.
#![forbid(unsafe_code)]

pub mod bounds;
mod fx;
pub mod plan;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_compose::effects::FieldKind;
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId, Pattern};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

pub use plan::{Kernel, Plan, Role, Unsupported, plan};

/// Accumulator format for intermediate buffers. Full float: discontinuous operations
/// (Posterize, Threshold, Hard Mix, Dissolve) must land on the same side of their thresholds as
/// the CPU reference, and Color Burn/Dodge amplify input error.
pub const ACC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float; // default accumulation format; `preferred_acc_format` downgrades to Rgba16Float where 32-bit float isn't renderable
/// Effect intermediates (distances, blurs) and layer shapes.
const MAP32: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Final effect coverage maps (0..1; half precision is ~1/4000).
const MAP16: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// Side of the square chunks the canvas is processed in.
pub const CHUNK: u32 = 1024;
const STRIDE: u64 = 256;
const CHUNK_UNIFORM: u64 = 16;
const OP_UNIFORM: u64 = 176;
/// GPU memory the effect-map cache may hold before evicting layers not drawn this frame.
pub const FX_BUDGET: usize = 1536 << 20;

/// A finished chunk: straight-alpha RGBA32F pixels of `rect` (document coordinates) at the
/// texture's origin.
pub struct ChunkOut<'a> {
    pub rect: Rect,
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
}

/// What a render did (for profiling and tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub passes: usize,
    pub chunks: usize,
    pub slots: u32,
    pub tiles_uploaded: usize,
    pub bytes_uploaded: usize,
    /// Effect layers whose shape was (re)rendered, fully or partially.
    pub fx_shapes: usize,
    /// Effect map programs run (one per effect whose maps were rebuilt).
    pub fx_programs: usize,
    /// Effect-map pixels written (all stages).
    pub fx_pixels: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TexKind {
    /// RGBA8 tiles copied verbatim.
    Rgba8Direct,
    /// Other 8-bit formats converted to RGBA8.
    Rgba8,
    /// 16/32-bit formats converted to RGBA16F.
    Rgba16F,
    /// GRAY8 masks copied verbatim.
    R8Direct,
    /// Other masks as R32F.
    R32F,
}

impl TexKind {
    fn for_surface(role: Role, f: PixelFormat) -> Self {
        match role {
            Role::Content | Role::Stroke if f == PixelFormat::RGBA8 => TexKind::Rgba8Direct,
            Role::Content | Role::Stroke if f.sample == SampleType::U8 => TexKind::Rgba8,
            Role::Content | Role::Stroke => TexKind::Rgba16F,
            Role::Mask if f == PixelFormat::GRAY8 => TexKind::R8Direct,
            Role::Mask => TexKind::R32F,
        }
    }
    fn format(self) -> wgpu::TextureFormat {
        match self {
            TexKind::Rgba8Direct | TexKind::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            TexKind::Rgba16F => wgpu::TextureFormat::Rgba16Float,
            TexKind::R8Direct => wgpu::TextureFormat::R8Unorm,
            TexKind::R32F => wgpu::TextureFormat::R32Float,
        }
    }
    fn bytes_per_pixel(self) -> usize {
        match self {
            TexKind::Rgba8Direct | TexKind::Rgba8 => 4,
            TexKind::Rgba16F => 8,
            TexKind::R8Direct => 1,
            TexKind::R32F => 4,
        }
    }
}

/// Index into the render's resident key list, and the texture's region (x, y, w, h).
type ResidentRef = (usize, [i32; 4]);

struct Resident {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Document-pixel rect covered (tile aligned).
    region: Rect,
    kind: TexKind,
    format: PixelFormat,
    tiles: HashMap<TileCoord, Arc<Tile>>,
    default_nonzero: bool,
    doc: DocId,
    last_used: u64,
}

/// A texture and its default view.
#[derive(Clone)]
struct Tex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Tex {
    fn new(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Tex { texture, view }
    }
    fn map(device: &wgpu::Device, label: &str, r: Rect, format: wgpu::TextureFormat) -> Self {
        Tex::new(device, label, r.width(), r.height(), format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST)
    }
    /// Upload R32F values (row-major over `r`) into this region-space texture at `r`.
    fn write_r32(&self, queue: &wgpu::Queue, region: Rect, r: Rect, v: &[f32]) {
        if r.is_empty() {
            return;
        }
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x: (r.x0 - region.x0) as u32, y: (r.y0 - region.y0) as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r.width() * 4), rows_per_image: Some(r.height()) },
            wgpu::Extent3d { width: r.width(), height: r.height(), depth_or_array_layers: 1 },
        );
    }
    fn bytes(&self) -> usize {
        let s = self.texture.size();
        let bpp = match self.texture.format() {
            wgpu::TextureFormat::R16Float => 2,
            wgpu::TextureFormat::Rgba32Float => 16,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => 4,
        };
        s.width as usize * s.height as usize * bpp
    }
}

/// Cached maps of one effect (one enabled item of a layer).
struct ProgState {
    key: u64,
    maps: Vec<Option<Tex>>,
}

/// Cached effect state of one layer.
struct FxEntry {
    doc: DocId,
    region: Rect,
    shape_key: u64,
    /// Tiles the shape was rendered from (content, mask): damage is their difference.
    tiles: [Option<fx::Tiles>; 2],
    /// Groups: a clone pinning the tile pointers `shape_key` hashes.
    _pin: Option<Layer>,
    /// The shape on the CPU (distance fields are computed from it) and on the GPU.
    shape_cpu: Vec<f32>,
    shape: Tex,
    /// Distance fields: the reach each is exact to, and its texture.
    fields: HashMap<FieldKind, (i32, Tex)>,
    progs: Vec<ProgState>,
    last_used: u64,
}

impl FxEntry {
    fn bytes(&self) -> usize {
        self.shape.bytes() + self.shape_cpu.len() * 4 + self.fields.values().map(|(_, t)| t.bytes()).sum::<usize>() + self.progs.iter().flat_map(|p| p.maps.iter().flatten()).map(Tex::bytes).sum::<usize>()
    }
}

/// Pipelines and layouts, shared by cheap clones (wgpu handles are reference counted).
#[derive(Clone)]
struct Kit {
    pipelines: HashMap<(Kernel, wgpu::TextureFormat), wgpu::RenderPipeline>,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    dummy: wgpu::TextureView,
}

/// One draw of an effect-map kernel (outside the chunk loop).
struct MapDraw {
    kernel: Kernel,
    format: wgpu::TextureFormat,
    target: wgpu::TextureView,
    /// Target-local scissor (x, y, w, h).
    scissor: [u32; 4],
    /// Chunk record: origin and size of the target's space.
    space: [i32; 4],
    op: Vec<u8>,
    views: [Option<wgpu::TextureView>; 8],
    lut: Option<Arc<Vec<f32>>>,
}

impl Kit {
    /// Record `draws` (each its own render pass) into `encoder`.
    fn run(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, draws: &[MapDraw]) {
        if draws.is_empty() {
            return;
        }
        let mut data = vec![0u8; draws.len() * 2 * STRIDE as usize];
        for (i, d) in draws.iter().enumerate() {
            let w = words(&[I(d.space[0]), I(d.space[1]), I(d.space[2]), I(d.space[3])]);
            data[2 * i * STRIDE as usize..][..w.len()].copy_from_slice(&w);
            data[(2 * i + 1) * STRIDE as usize..][..d.op.len()].copy_from_slice(&d.op);
        }
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("pc_fx_uniforms"), size: data.len() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&ubuf, 0, &data);
        let bg0 = uniform_group(device, &self.bgl0, &ubuf);
        for (i, d) in draws.iter().enumerate() {
            let lut = d.lut.as_ref().map(|l| {
                let mut row = [0.0f32; 4096];
                row[..l.len().min(4096)].copy_from_slice(&l[..l.len().min(4096)]);
                lut_texture(device, queue, &[row])
            });
            let mut views: Vec<&wgpu::TextureView> = d.views.iter().map(|v| v.as_ref().unwrap_or(&self.dummy)).collect();
            if let Some((_, lv)) = &lut {
                views[4] = lv;
            }
            let bg1 = texture_group(device, &self.bgl1, &views);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pc_fx_map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &d.target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines[&(d.kernel, d.format)]);
            pass.set_scissor_rect(d.scissor[0], d.scissor[1], d.scissor[2], d.scissor[3]);
            pass.set_bind_group(0, &bg0, &[(2 * i as u64 * STRIDE) as u32, ((2 * i + 1) as u64 * STRIDE) as u32]);
            pass.set_bind_group(1, &bg1, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn uniform_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, ubuf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_compose_uniforms"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: ubuf, offset: 0, size: NonZeroU64::new(CHUNK_UNIFORM) }) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: ubuf, offset: 0, size: NonZeroU64::new(OP_UNIFORM) }) },
        ],
    })
}

fn texture_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, views: &[&wgpu::TextureView]) -> wgpu::BindGroup {
    let e: Vec<wgpu::BindGroupEntry> = views.iter().enumerate().map(|(b, v)| wgpu::BindGroupEntry { binding: b as u32, resource: wgpu::BindingResource::TextureView(v) }).collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pc_compose_pass"), layout, entries: &e })
}

/// GPU compositor. Create once per device; call [`Compositor::render`] per refresh.
pub struct Compositor {
    kit: Kit,
    pool: Vec<(wgpu::Texture, wgpu::TextureView)>,
    residents: HashMap<(LayerId, Role), Resident>,
    fx: HashMap<LayerId, FxEntry>,
    /// Effect temporaries (R32F, region sized) and the frame they were last used.
    temps: Vec<(Tex, u64)>,
    patterns: HashMap<String, (u64, Tex)>,
    max_dim: u32,
    frame: u64,
    acc_format: wgpu::TextureFormat,
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { multisampled: false, sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2 },
        count: None,
    }
}

fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: NonZeroU64::new(size) },
        count: None,
    }
}

/// The compositor's WGSL source (exposed for validation in tests).
pub const SHADER: &str = include_str!("compose.wgsl");

/// Pass-level resources resolved for one plan: residents (texture, mask), effect maps (view and
/// region) and pattern textures per pass.
struct Bound {
    views: Vec<(Option<ResidentRef>, Option<ResidentRef>)>,
    keys: Vec<(LayerId, Role)>,
    maps: Vec<Option<(wgpu::TextureView, Rect)>>,
    patterns: Vec<Option<wgpu::TextureView>>,
}

impl Compositor {
    /// Create a compositor with the default accumulation format ([`ACC_FORMAT`]).
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_format(device, ACC_FORMAT)
    }

    /// The accumulation format to use on `adapter`: `Rgba32Float` when it can be a render target,
    /// else `Rgba16Float`. Some drivers (e.g. Intel Vulkan) don't support rendering to `Rgba32Float`
    /// and would otherwise panic at render-pipeline creation.
    pub fn preferred_acc_format(adapter: &wgpu::Adapter) -> wgpu::TextureFormat {
        let feats = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba32Float);
        if feats.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            wgpu::TextureFormat::Rgba32Float
        } else {
            wgpu::TextureFormat::Rgba16Float
        }
    }

    /// Create a compositor whose accumulation/render-target format is `acc_format`. Pass
    /// `Rgba16Float` on adapters that can't render to `Rgba32Float` (e.g. some Intel Vulkan drivers),
    /// which otherwise panics at render-pipeline creation. See `Compositor::preferred_acc_format`.
    pub fn new_with_format(device: &wgpu::Device, acc_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_compose"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pc_compose_uniforms"), entries: &[uniform_entry(0, CHUNK_UNIFORM), uniform_entry(1, OP_UNIFORM)] });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pc_compose_textures"), entries: &(0..8).map(tex_entry).collect::<Vec<_>>() });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pc_compose"), bind_group_layouts: &[Some(&bgl0), Some(&bgl1)], immediate_size: 0 });
        let mut pipelines = HashMap::new();
        for k in Kernel::DRAWN {
            let entry = k.entry().expect("drawn kernels have an entry point");
            let formats: &[wgpu::TextureFormat] = if k.is_map() { &[MAP32, MAP16] } else { &[acc_format] };
            for &format in formats {
                let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
                    multiview_mask: None,
                    cache: None,
                });
                pipelines.insert((k, format), p);
            }
        }
        let dummy = Tex::new(device, "pc_compose_dummy", 1, 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING).view;
        Self {
            kit: Kit { pipelines, bgl0, bgl1, dummy },
            pool: Vec::new(),
            residents: HashMap::new(),
            fx: HashMap::new(),
            temps: Vec::new(),
            patterns: HashMap::new(),
            max_dim: device.limits().max_texture_dimension_2d,
            frame: 0,
            acc_format,
        }
    }

    /// Whether `doc` can be composited on the GPU.
    pub fn supports(&self, doc: &Document) -> Result<(), Unsupported> {
        let b = doc.bounds();
        if b.width() > self.max_dim || b.height() > self.max_dim {
            return Err(Unsupported(format!("document larger than the GPU texture limit ({})", self.max_dim)));
        }
        let p = plan(doc)?;
        self.check_fx(doc, &p)
    }

    /// Effect regions and patterns must fit in textures.
    fn check_fx(&self, doc: &Document, p: &Plan<'_>) -> Result<(), Unsupported> {
        for f in &p.fx {
            if f.region.width() > self.max_dim || f.region.height() > self.max_dim {
                return Err(Unsupported(format!("effect region of `{}` larger than the GPU texture limit ({})", f.layer.name, self.max_dim)));
            }
            for e in f.layer.effects.items.iter().filter(|e| e.enabled()) {
                for s in fx::program_with(e, &doc.global_light, false, &doc.patterns, (0.0, 0.0)).stages {
                    if s.lut.as_ref().is_some_and(|l| l.len() > 4096) {
                        return Err(Unsupported(format!("effect blur on `{}` too wide for the GPU path", f.layer.name)));
                    }
                }
            }
        }
        for pass in &p.passes {
            if let Some(pat) = pass.pattern
                && (pat.width > self.max_dim || pat.height > self.max_dim)
            {
                return Err(Unsupported(format!("pattern `{}` larger than the GPU texture limit", pat.name)));
            }
        }
        Ok(())
    }

    /// Drop GPU textures of a closed document.
    pub fn forget_doc(&mut self, doc: DocId) {
        self.residents.retain(|_, r| r.doc != doc);
        self.fx.retain(|_, e| e.doc != doc);
    }

    /// GPU memory held by cached effect maps (bytes).
    pub fn fx_cache_bytes(&self) -> usize {
        self.fx.values().map(FxEntry::bytes).sum()
    }

    /// Composite `region` of `doc`. Each finished chunk is passed to `sink` while its encoder is
    /// still open; the chunk texture is reused afterwards, so the sink must record any copies
    /// or passes that read it into the given encoder. Submits the work before returning.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, region: Rect, mut sink: impl FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>)) -> Result<Stats, Unsupported> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_compose") });
        let stats = self.encode(device, queue, &mut encoder, doc, region, &mut sink)?;
        queue.submit([encoder.finish()]);
        Ok(stats)
    }

    /// Like [`Self::render`] but records into `encoder` without submitting.
    pub fn encode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, doc: &Document, region: Rect, sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>)) -> Result<Stats, Unsupported> {
        let canvas = doc.bounds();
        if canvas.width() > self.max_dim || canvas.height() > self.max_dim {
            return Err(Unsupported(format!("document larger than the GPU texture limit ({})", self.max_dim)));
        }
        let region = region.intersect(&canvas);
        let plan = plan(doc)?;
        self.check_fx(doc, &plan)?;
        let mut stats = Stats { passes: plan.passes.len(), slots: plan.slots, ..Default::default() };
        if region.is_empty() {
            return Ok(stats);
        }
        self.frame += 1;

        // Effect maps first (they may render group shapes through sub-plans).
        for f in &plan.fx {
            self.sync_fx(device, queue, encoder, doc, f, &mut stats);
        }

        // Chunks.
        let mut chunks = Vec::new();
        let mut y = region.y0;
        while y < region.y1 {
            let mut x = region.x0;
            while x < region.x1 {
                chunks.push(Rect::new(x, y, (x + CHUNK as i32).min(region.x1), (y + CHUNK as i32).min(region.y1)));
                x += CHUNK as i32;
            }
            y += CHUNK as i32;
        }
        stats.chunks = chunks.len();
        self.run_plan(device, queue, encoder, doc.id, canvas, &plan, &chunks, &mut stats, sink);

        // Evict this document's textures whose layers are gone (hidden layers stay resident so
        // toggling visibility costs no upload).
        let frame = self.frame;
        let live: std::collections::HashSet<LayerId> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        self.residents.retain(|(id, _), r| r.doc != doc.id || r.last_used == frame || live.contains(id));
        self.fx.retain(|id, e| e.doc != doc.id || e.last_used == frame || live.contains(id));
        self.evict_fx();
        self.temps.retain(|(_, used)| frame.saturating_sub(*used) < 240);
        Ok(stats)
    }

    /// Keep the effect cache within [`FX_BUDGET`], dropping layers not drawn this frame
    /// (least recently used first).
    fn evict_fx(&mut self) {
        let mut total = self.fx_cache_bytes();
        while total > FX_BUDGET {
            let Some((&id, _)) = self.fx.iter().filter(|(_, e)| e.last_used != self.frame).min_by_key(|(_, e)| e.last_used) else { break };
            if let Some(e) = self.fx.remove(&id) {
                total = total.saturating_sub(e.bytes());
            }
        }
    }

    /// Bring every surface `plan` samples up to date and resolve its effect maps and patterns.
    fn bind_plan(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, doc: DocId, canvas: Rect, plan: &Plan<'_>, stats: &mut Stats) -> Bound {
        let mut views: Vec<(Option<ResidentRef>, Option<ResidentRef>)> = Vec::with_capacity(plan.passes.len());
        let mut keys: Vec<(LayerId, Role)> = Vec::new();
        let mut maps = Vec::with_capacity(plan.passes.len());
        let mut patterns = Vec::with_capacity(plan.passes.len());
        for p in &plan.passes {
            let tex = p.tex.as_ref().and_then(|t| self.sync(device, queue, doc, t.layer, t.role, t.surface.get(), tile_grid(canvas), stats)).map(|(k, r)| {
                keys.push(k);
                (keys.len() - 1, r)
            });
            let mask = p.mask.as_ref().and_then(|m| self.sync(device, queue, doc, m.layer, Role::Mask, m.surface.get(), tile_grid(canvas), stats)).map(|(k, r)| {
                keys.push(k);
                (keys.len() - 1, r)
            });
            views.push((tex, mask));
            maps.push(p.map.and_then(|m| {
                let e = self.fx.get(&plan.fx[m.fx].layer.id)?;
                let t = e.progs.get(m.item)?.maps.get(m.map)?.as_ref()?;
                Some((t.view.clone(), e.region))
            }));
            patterns.push(p.pattern.map(|pat| self.pattern_view(device, queue, pat)));
        }
        Bound { views, keys, maps, patterns }
    }

    /// Run `plan` over `chunks` (document rects, at most CHUNK square), handing each finished
    /// chunk to `sink`.
    #[allow(clippy::too_many_arguments)]
    fn run_plan(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, doc: DocId, canvas: Rect, plan: &Plan<'_>, chunks: &[Rect], stats: &mut Stats, sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>)) {
        let bound = self.bind_plan(device, queue, doc, canvas, plan, stats);

        // Chunk pool.
        while self.pool.len() < plan.slots as usize {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pc_compose_chunk"),
                size: wgpu::Extent3d { width: CHUNK, height: CHUNK, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.acc_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            self.pool.push((t, v));
        }

        // Uniforms: chunk records, then one record per pass.
        let op_base = chunks.len() as u64 * STRIDE;
        let mut data = vec![0u8; (op_base + plan.passes.len() as u64 * STRIDE) as usize];
        for (i, c) in chunks.iter().enumerate() {
            let w = words(&[I(c.x0), I(c.y0), I(c.width() as i32), I(c.height() as i32)]);
            data[i * STRIDE as usize..][..w.len()].copy_from_slice(&w);
        }
        for (i, p) in plan.passes.iter().enumerate() {
            let (tex, mask) = bound.views[i];
            let map = bound.maps[i].as_ref().map(|(_, r)| *r);
            let w = op_words(p, tex.map(|t| t.1), mask.map(|m| m.1), map);
            let off = (op_base + i as u64 * STRIDE) as usize;
            data[off..][..w.len()].copy_from_slice(&w);
        }
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("pc_compose_uniforms"), size: data.len() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&ubuf, 0, &data);
        let bg0 = uniform_group(device, &self.kit.bgl0, &ubuf);

        // Per-pass texture bind groups (chunk-independent: pool slots are reused per chunk).
        let resident_views: Vec<&wgpu::TextureView> = bound.keys.iter().map(|k| &self.residents[k].view).collect();
        let luts: Vec<_> = plan.passes.iter().map(|p| p.lut.as_ref().map(|rows| lut_texture(device, queue, rows))).collect();
        let dummy = &self.kit.dummy;
        let mut bg1 = Vec::with_capacity(plan.passes.len());
        for (i, p) in plan.passes.iter().enumerate() {
            if p.kernel.entry().is_none() {
                bg1.push(None);
                continue;
            }
            let slot = |s: Option<u32>| s.map_or(dummy, |s| &self.pool[s as usize].1);
            let (tex, mask) = &bound.views[i];
            let tv = tex.map_or(slot(p.d), |(k, _)| resident_views[k]);
            let mv = mask.map_or(dummy, |(k, _)| resident_views[k]);
            let lv = luts[i].as_ref().map_or(dummy, |(_, v)| v);
            let map = bound.maps[i].as_ref().map_or(dummy, |(v, _)| v);
            let pat = bound.patterns[i].as_ref().unwrap_or(dummy);
            bg1.push(Some(texture_group(device, &self.kit.bgl1, &[slot(p.a), slot(p.b), tv, mv, lv, slot(p.c), map, pat])));
        }

        for (ci, c) in chunks.iter().enumerate() {
            for (i, p) in plan.passes.iter().enumerate() {
                let scissor = match p.clip {
                    Some(clip) => {
                        let r = clip.intersect(c);
                        if r.is_empty() {
                            continue;
                        }
                        Rect::new(r.x0 - c.x0, r.y0 - c.y0, r.x1 - c.x0, r.y1 - c.y0)
                    }
                    None => Rect::new(0, 0, c.width() as i32, c.height() as i32),
                };
                match p.kernel {
                    Kernel::CopyRect | Kernel::CopyFull => {
                        let src = &self.pool[p.a.expect("copy source") as usize].0;
                        let dst = &self.pool[p.dst as usize].0;
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin: wgpu::Origin3d { x: scissor.x0 as u32, y: scissor.y0 as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                            wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin: wgpu::Origin3d { x: scissor.x0 as u32, y: scissor.y0 as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                            wgpu::Extent3d { width: scissor.width(), height: scissor.height(), depth_or_array_layers: 1 },
                        );
                        continue;
                    }
                    _ => {}
                }
                let target = &self.pool[p.dst as usize].1;
                let clear = p.kernel == Kernel::Clear;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pc_compose_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if clear {
                    continue;
                }
                pass.set_pipeline(&self.kit.pipelines[&(p.kernel, self.acc_format)]);
                pass.set_scissor_rect(scissor.x0 as u32, scissor.y0 as u32, scissor.width(), scissor.height());
                pass.set_bind_group(0, &bg0, &[(ci as u64 * STRIDE) as u32, (op_base + i as u64 * STRIDE) as u32]);
                pass.set_bind_group(1, bg1[i].as_ref(), &[]);
                pass.draw(0..3, 0..1);
            }
            let (t, v) = &self.pool[plan.root as usize];
            sink(encoder, ChunkOut { rect: *c, texture: t, view: v });
        }
    }

    /// Upload changed tiles of `surface`; returns the resident key and its region (x, y, w, h).
    #[allow(clippy::too_many_arguments)]
    fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, doc: DocId, layer: LayerId, role: Role, surface: &Surface, grid: Rect, stats: &mut Stats) -> Option<((LayerId, Role), [i32; 4])> {
        let region = surface.tile_bounds().intersect(&grid);
        if region.is_empty() {
            return None;
        }
        let format = surface.format();
        let kind = TexKind::for_surface(role, format);
        let key = (layer, role);
        let stale = self.residents.get(&key).is_none_or(|r| r.region != region || r.kind != kind || r.format != format);
        if stale {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pc_compose_layer"),
                size: wgpu::Extent3d { width: region.width(), height: region.height(), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: kind.format(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let default_nonzero = surface.default_pixel().iter().any(|v| *v != 0.0);
            let r = Resident { texture, view, region, kind, format, tiles: HashMap::new(), default_nonzero, doc, last_used: 0 };
            if default_nonzero {
                // Missing tiles read as the default pixel: initialise them.
                let bytes = convert_tile(surface, None, kind, TileCoord::new(0, 0));
                for c in region.tiles() {
                    if surface.tile(c).is_none() {
                        write_tile(queue, &r, c, &bytes);
                    }
                }
            }
            self.residents.insert(key, r);
        }
        let r = self.residents.get_mut(&key).expect("inserted");
        r.last_used = self.frame;
        r.doc = doc;
        for (c, t) in surface.tiles() {
            if region.intersect(&c.rect()) != c.rect() {
                continue;
            }
            if r.tiles.get(c).is_some_and(|old| Arc::ptr_eq(old, t)) {
                continue;
            }
            let bytes = convert_tile(surface, Some(t), kind, *c);
            write_tile(queue, r, *c, &bytes);
            stats.tiles_uploaded += 1;
            stats.bytes_uploaded += bytes.len();
            r.tiles.insert(*c, t.clone());
        }
        let gone: Vec<TileCoord> = r.tiles.keys().filter(|c| surface.tile(**c).is_none()).copied().collect();
        if !gone.is_empty() {
            let bytes = if r.default_nonzero { convert_tile(surface, None, kind, TileCoord::new(0, 0)) } else { vec![0u8; (TILE_SIZE * TILE_SIZE) as usize * kind.bytes_per_pixel()] };
            for c in gone {
                write_tile(queue, r, c, &bytes);
                r.tiles.remove(&c);
            }
        }
        Some((key, [region.x0, region.y0, region.width() as i32, region.height() as i32]))
    }

    /// Premultiplied RGBA32F texture of a pattern (cached by pixel identity).
    fn pattern_view(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pat: &Pattern) -> wgpu::TextureView {
        let fp = pat.surface.tiles().fold((pat.width as u64) << 32 | pat.height as u64, |acc, (c, t)| acc.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (Arc::as_ptr(t) as usize as u64) ^ ((c.tx as u64) << 20) ^ c.ty as u64);
        let key = format!("{}\u{0}{}", pat.id, pat.name);
        if let Some((f, t)) = self.patterns.get(&key)
            && *f == fp
        {
            return t.view.clone();
        }
        let (w, h) = (pat.width, pat.height);
        let mut px = vec![[0.0f32; 4]; w as usize * h as usize];
        pat.surface.read_rgba_into(pat.rect(), &mut px);
        let bytes: Vec<u8> = px.iter().flat_map(|q| [q[0] * q[3], q[1] * q[3], q[2] * q[3], q[3]]).flat_map(f32::to_le_bytes).collect();
        let t = Tex::new(device, "pc_pattern", w, h, wgpu::TextureFormat::Rgba32Float, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let v = t.view.clone();
        self.patterns.insert(key, (fp, t));
        v
    }

    /// A region-sized R32F temporary not in `taken`.
    fn temp(&mut self, device: &wgpu::Device, region: Rect, taken: &[wgpu::TextureView]) -> wgpu::TextureView {
        let (w, h) = (region.width(), region.height());
        let frame = self.frame;
        for (t, used) in &mut self.temps {
            let s = t.texture.size();
            if s.width == w && s.height == h && !taken.contains(&t.view) {
                *used = frame;
                return t.view.clone();
            }
        }
        let t = Tex::map(device, "pc_fx_temp", region, MAP32);
        let v = t.view.clone();
        self.temps.push((t, frame));
        v
    }

    /// Build or update the effect maps of one layer.
    fn sync_fx(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, doc: &Document, f: &plan::FxLayer<'_>, stats: &mut Stats) {
        let layer = f.layer;
        let region = f.region;
        let canvas = doc.bounds();
        let is_group = matches!(layer.content, LayerContent::Group(_));
        let shape_key = if is_group { fx::group_key(layer, &doc.global_light) } else { fx::shape_key(layer, canvas) };
        let (content_src, mask_src) = if is_group { (None, None) } else { fx::shape_sources(layer) };
        let frame = self.frame;
        let (rw, rh) = (region.width(), region.height());

        // What changed: everything (new settings / size), the position only, some tiles, or
        // nothing.
        let mut damage = Rect::EMPTY;
        let prev = self.fx.get(&layer.id).filter(|e| e.shape_key == shape_key).map(|e| e.region);
        let mut rebuild = true;
        if prev == Some(region) {
            rebuild = false;
            let e = &self.fx[&layer.id];
            for (old, cur) in e.tiles.iter().zip([content_src, mask_src]) {
                let d = match (old, cur) {
                    (Some(o), Some(s)) => fx::damage(o, s),
                    (None, None) => Rect::EMPTY,
                    // A source appeared or vanished (the key changes with it too).
                    _ => region,
                };
                damage = if damage.is_empty() { d } else { damage.union(&d) };
            }
            damage = damage.intersect(&region);
        } else if let Some(old) = prev
            && (old.width(), old.height()) == (rw, rh)
            && !region.is_empty()
        {
            // Moved by whole pixels: maps are computed relative to the region, so if the shape
            // moved unchanged with it, every map is still exact.
            let v = fx::shape(doc, layer, region);
            let e = self.fx.get_mut(&layer.id).expect("entry");
            if v == e.shape_cpu {
                e.region = region;
                rebuild = false;
            }
        }
        if rebuild {
            let shape = Tex::map(device, "pc_fx_shape", region, MAP32);
            let n = rw as usize * rh as usize;
            self.fx.insert(layer.id, FxEntry { doc: doc.id, region, shape_key, tiles: [None, None], _pin: is_group.then(|| layer.clone()), shape_cpu: vec![0.0; n], shape, fields: HashMap::new(), progs: Vec::new(), last_used: frame });
            damage = region;
        }
        let e = self.fx.get_mut(&layer.id).expect("inserted");
        e.last_used = frame;
        e.doc = doc.id;
        e.tiles = [content_src.map(fx::snapshot), mask_src.map(fx::snapshot)];
        if region.is_empty() {
            return;
        }

        let t_trace = web_time_now();
        // Shape over the damage (compose's own alpha).
        if !damage.is_empty() {
            stats.fx_shapes += 1;
            let v = fx::shape(doc, layer, damage);
            fx::paste(&mut e.shape_cpu, region, damage, &v);
            e.shape.write_r32(queue, region, damage, &v);
        }

        // Programs, and the distance fields they read (max reach per field).
        let vector_shape = matches!(layer.content, LayerContent::Shape(_));
        let anchor = layer.effects.reference.unwrap_or((f64::from(f.bounds.x0), f64::from(f.bounds.y0)));
        let progs: Vec<fx::MapProgram> = layer.effects.items.iter().filter(|e| e.enabled()).map(|e| fx::program_with(e, &doc.global_light, vector_shape, &doc.patterns, anchor)).collect();
        let mut want: HashMap<FieldKind, i32> = HashMap::new();
        for (k, r) in progs.iter().flat_map(|p| p.fields.iter()) {
            let w = want.entry(*k).or_insert(0);
            *w = (*w).max(*r);
        }
        e.fields.retain(|k, _| want.contains_key(k));
        for (&kind, &reach) in &want {
            let have = e.fields.get(&kind).map(|(r, _)| *r);
            let out = match have {
                Some(r) if r >= reach => {
                    if damage.is_empty() {
                        continue;
                    }
                    damage.inflate(fx::field_radius(kind, r)).intersect(&region)
                }
                _ => region,
            };
            let reach = have.filter(|r| *r >= reach).unwrap_or(reach);
            let v = fx::field(kind, reach, &e.shape_cpu, region, out);
            if have.is_none_or(|r| r < reach) {
                e.fields.insert(kind, (reach, Tex::map(device, "pc_fx_field", region, MAP32)));
            }
            e.fields[&kind].1.write_r32(queue, region, out, &v);
            stats.fx_pixels += out.width() as u64 * out.height() as u64;
        }

        if let Some(t) = t_trace.filter(|_| !damage.is_empty()) {
            eprintln!("effect maps of `{}`: shape + distance fields over {damage:?} in {:.2} ms (CPU)", layer.name, t.elapsed().as_secs_f64() * 1000.0);
        }
        e.progs.truncate(progs.len());
        while e.progs.len() < progs.len() {
            e.progs.push(ProgState { key: 0, maps: Vec::new() });
        }
        for (i, prog) in progs.iter().enumerate() {
            if prog.maps == 0 {
                continue;
            }
            let e = self.fx.get_mut(&layer.id).expect("entry");
            let same = e.progs[i].key == prog.key && e.progs[i].maps.len() == prog.maps;
            let d = if same { damage } else { region };
            if d.is_empty() {
                continue;
            }
            if !same {
                e.progs[i] = ProgState { key: prog.key, maps: (0..prog.maps).map(|k| prog.stages.iter().any(|s| s.out == Some(k)).then(|| Tex::map(device, "pc_fx_map", region, MAP16))).collect() };
            }
            stats.fx_programs += 1;
            self.run_program(device, queue, encoder, layer.id, i, prog, d, stats);
        }
    }

    /// Record the passes of `prog` (maps of item `item` of `layer`) recomputing damage `d`.
    #[allow(clippy::too_many_arguments)]
    fn run_program(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, layer: LayerId, item: usize, prog: &fx::MapProgram, d: Rect, stats: &mut Stats) {
        let pattern_views: Vec<Option<wgpu::TextureView>> = prog.stages.iter().map(|s| s.pattern.as_ref().map(|p| self.pattern_view(device, queue, p))).collect();
        let e = &self.fx[&layer];
        let region = e.region;
        let shape = e.shape.view.clone();
        let finals: Vec<Option<wgpu::TextureView>> = e.progs[item].maps.iter().map(|m| m.as_ref().map(|t| t.view.clone())).collect();
        let fields: HashMap<FieldKind, (i32, wgpu::TextureView)> = e.fields.iter().map(|(k, (r, t))| (*k, (*r, t.view.clone()))).collect();
        let windows = prog.windows(d, region, &|k| fields.get(&k).map_or(0, |f| f.0));
        let (assign, count) = prog.temps();
        let mut temps: Vec<wgpu::TextureView> = Vec::with_capacity(count);
        for _ in 0..count {
            let v = self.temp(device, region, &temps);
            temps.push(v);
        }
        let target_of = |i: usize| -> (wgpu::TextureView, wgpu::TextureFormat) {
            match prog.stages[i].out {
                Some(k) => (finals[k].clone().expect("final map allocated"), MAP16),
                None => (temps[assign[i].expect("temporary assigned")].clone(), MAP32),
            }
        };
        let view_of = |inp: Option<fx::In>| -> Option<wgpu::TextureView> {
            match inp? {
                fx::In::Shape => Some(shape.clone()),
                fx::In::Field(k) => fields.get(&k).map(|f| f.1.clone()),
                fx::In::Val(k) => Some(target_of(k).0),
            }
        };
        let mut draws = Vec::with_capacity(prog.stages.len());
        for (i, s) in prog.stages.iter().enumerate() {
            let w = windows[i];
            if w.is_empty() {
                continue;
            }
            stats.fx_pixels += w.width() as u64 * w.height() as u64;
            let (target, format) = target_of(i);
            let mut p = plan::Pass::new(s.kernel, 0);
            p.params[0] = s.p0;
            p.params[1] = s.p1;
            p.params[3] = s.p3;
            p.extra = s.p4;
            draws.push(MapDraw {
                kernel: s.kernel,
                format,
                target,
                scissor: [(w.x0 - region.x0) as u32, (w.y0 - region.y0) as u32, w.width(), w.height()],
                space: [region.x0, region.y0, region.width() as i32, region.height() as i32],
                op: op_words(&p, None, None, None),
                views: [view_of(s.a), view_of(s.b), view_of(s.s), None, None, None, None, pattern_views[i].clone()],
                lut: s.lut.clone(),
            });
        }
        self.kit.run(device, queue, encoder, &draws);
    }
}

/// A start time when `PHOTOCRAFT_FX_TRACE=1` (it prints the CPU time of effect shapes and
/// distance fields); never on wasm.
fn web_time_now() -> Option<std::time::Instant> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        T.get_or_init(|| std::env::var_os("PHOTOCRAFT_FX_TRACE").is_some()).then(std::time::Instant::now)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// The uniform record of a pass.
fn op_words(p: &plan::Pass<'_>, tex: Option<[i32; 4]>, mask: Option<[i32; 4]>, map: Option<Rect>) -> Vec<u8> {
    let mut flags = p.flags;
    let (mut to, mut ts, mut mo, mut ms) = ([0; 2], [0; 2], [0; 2], [0; 2]);
    if let Some(r) = tex {
        flags |= 4;
        to = [r[0], r[1]];
        ts = [r[2], r[3]];
    }
    let (mut density, mut mdefault) = (0.0, 1.0);
    if let Some(m) = &p.mask {
        flags |= 1;
        density = m.density;
        mdefault = m.default;
        if let Some(r) = mask {
            flags |= 2;
            mo = [r[0], r[1]];
            ms = [r[2], r[3]];
        }
    }
    if p.gradient {
        flags |= 8;
    }
    let (mpo, mps) = map.map_or(([0, 0], [0, 0]), |r| ([r.x0, r.y0], [r.width() as i32, r.height() as i32]));
    let mut v = vec![I(plan::mode_index(p.mode)), I(p.adjust_kind), U(flags), U(0), F(p.opacity), F(density), F(mdefault), F(0.0), I(to[0]), I(to[1]), I(ts[0]), I(ts[1]), I(mo[0]), I(mo[1]), I(ms[0]), I(ms[1])];
    v.extend(p.color.iter().map(|f| F(*f)));
    for row in &p.params {
        v.extend(row.iter().map(|f| F(*f)));
    }
    v.extend([I(mpo[0]), I(mpo[1]), I(mps[0]), I(mps[1])]);
    v.extend(p.extra.iter().map(|f| F(*f)));
    words(&v)
}

/// Canvas rounded out to whole tiles.
fn tile_grid(canvas: Rect) -> Rect {
    let t = TILE_SIZE;
    Rect::new(canvas.x0.div_euclid(t) * t, canvas.y0.div_euclid(t) * t, (canvas.x1 + t - 1).div_euclid(t) * t, (canvas.y1 + t - 1).div_euclid(t) * t)
}

fn write_tile(queue: &wgpu::Queue, r: &Resident, c: TileCoord, bytes: &[u8]) {
    let tr = c.rect();
    let bpp = r.kind.bytes_per_pixel() as u32;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &r.texture, mip_level: 0, origin: wgpu::Origin3d { x: (tr.x0 - r.region.x0) as u32, y: (tr.y0 - r.region.y0) as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(TILE_SIZE as u32 * bpp), rows_per_image: Some(TILE_SIZE as u32) },
        wgpu::Extent3d { width: TILE_SIZE as u32, height: TILE_SIZE as u32, depth_or_array_layers: 1 },
    );
}

/// Tile pixels in the texture's format. `tile = None` gives the default pixel everywhere.
fn convert_tile(surface: &Surface, tile: Option<&Arc<Tile>>, kind: TexKind, c: TileCoord) -> Vec<u8> {
    if let (Some(t), TexKind::Rgba8Direct | TexKind::R8Direct) = (tile, kind) {
        return t.bytes().to_vec();
    }
    let n = (TILE_SIZE * TILE_SIZE) as usize;
    let fmt = surface.format();
    let ch = fmt.channels();
    let raw: Vec<f32> = match tile {
        Some(_) => surface.read_region(c.rect()),
        None => surface.default_pixel().repeat(n),
    };
    let mut out = Vec::with_capacity(n * kind.bytes_per_pixel());
    for px in raw.chunks_exact(ch) {
        match kind {
            TexKind::Rgba8Direct | TexKind::Rgba8 => {
                let v = photocraft_raster::to_rgba(&fmt, px);
                out.extend(v.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
            }
            TexKind::Rgba16F => {
                let v = photocraft_raster::to_rgba(&fmt, px);
                for x in v {
                    out.extend(f32_to_f16(x).to_le_bytes());
                }
            }
            TexKind::R8Direct => out.push((px[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
            TexKind::R32F => out.extend(px[0].to_le_bytes()),
        }
    }
    out
}

fn lut_texture(device: &wgpu::Device, queue: &wgpu::Queue, rows: &[[f32; 4096]]) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_compose_lut"),
        size: wgpu::Extent3d { width: 4096, height: rows.len() as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = rows.iter().flat_map(|r| r.iter().flat_map(|v| v.to_le_bytes())).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4096 * 4), rows_per_image: Some(rows.len() as u32) },
        wgpu::Extent3d { width: 4096, height: rows.len() as u32, depth_or_array_layers: 1 },
    );
    let v = t.create_view(&Default::default());
    (t, v)
}

#[derive(Clone, Copy)]
enum W {
    I(i32),
    U(u32),
    F(f32),
}
use W::{F, I, U};

fn words(v: &[W]) -> Vec<u8> {
    v.iter()
        .flat_map(|w| match w {
            I(i) => i.to_le_bytes(),
            U(u) => u.to_le_bytes(),
            F(f) => f.to_le_bytes(),
        })
        .collect()
}

/// IEEE half from f32 (round to nearest even; overflow → inf).
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rounded = (m + half - 1 + ((m >> shift) & 1)) >> shift;
        return sign | rounded as u16;
    }
    let mut h = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h as u16
}

/// f32 from IEEE half.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f32;
    match exp {
        0 => sign * man * 2f32.powi(-24),
        0x1f => {
            if man == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Blocking readback of a composite (native tests and tools).
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<Vec<[f32; 4]>, Unsupported> {
    render_to_vec_stats(comp, device, queue, doc, rect).map(|(v, _)| v)
}

/// [`render_to_vec`] plus the render's [`Stats`].
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec_stats(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<(Vec<[f32; 4]>, Stats), Unsupported> {
    let rect = rect.intersect(&doc.bounds());
    let mut staging: Vec<(Rect, wgpu::Buffer, u32)> = Vec::new();
    let mut bpp = 16u32;
    let stats = comp.render(device, queue, doc, rect, |enc, out| {
        // Bytes per pixel of the accumulation format (Rgba32Float = 16, Rgba16Float = 8).
        bpp = if out.texture.format() == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        let row = (out.rect.width() * bpp).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("pc_readback"), size: (row * out.rect.height()) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(out.rect.height()) } },
            wgpu::Extent3d { width: out.rect.width(), height: out.rect.height(), depth_or_array_layers: 1 },
        );
        staging.push((out.rect, buf, row));
    })?;
    for (_, b, _) in &staging {
        b.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    }
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let w = rect.width() as usize;
    let mut out = vec![[0.0f32; 4]; w * rect.height() as usize];
    for (r, b, row) in &staging {
        let data = b.slice(..).get_mapped_range().expect("mapped readback buffer");
        for y in 0..r.height() as usize {
            for x in 0..r.width() as usize {
                let o = y * *row as usize + x * bpp as usize;
                let px: [f32; 4] = if bpp == 16 {
                    std::array::from_fn(|i| f32::from_le_bytes(data[o + i * 4..o + i * 4 + 4].try_into().expect("4 bytes")))
                } else {
                    std::array::from_fn(|i| half::f16::from_le_bytes([data[o + i * 2], data[o + i * 2 + 1]]).to_f32())
                };
                let (dx, dy) = ((r.x0 - rect.x0) as usize + x, (r.y0 - rect.y0) as usize + y);
                out[dy * w + dx] = px;
            }
        }
    }
    Ok((out, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_roundtrip() {
        for v in [0.0f32, 1.0, 0.5, 0.25, 1.0 / 255.0, 0.333, 65504.0, -2.5, 1e-6] {
            let r = f16_to_f32(f32_to_f16(v));
            assert!((r - v).abs() <= v.abs() / 1024.0 + 1e-7, "{v} -> {r}");
        }
    }

    #[test]
    fn shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
        for k in Kernel::DRAWN {
            let e = k.entry().unwrap();
            assert!(module.entry_points.iter().any(|ep| ep.name == e), "missing entry point {e}");
        }
    }

    #[test]
    fn op_record_fits_the_uniform() {
        let p = plan::Pass::new(Kernel::FxPaint, 0);
        assert_eq!(op_words(&p, None, None, None).len() as u64, OP_UNIFORM);
    }

    #[test]
    fn grid_rounds_out() {
        assert_eq!(tile_grid(Rect::new(0, 0, 300, 256)), Rect::new(0, 0, 512, 256));
    }
}
