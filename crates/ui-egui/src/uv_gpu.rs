//! GPU-resident model preview. Only camera uniforms change during navigation.
//! Separate depth/MSAA attachments keep the editor's own render pass untouched.
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, util::DeviceExt},
};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Image},
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const STRIDE: u64 = 72;
type Key = (u64, u64, u64, u64);
fn gray_mask_map_key(generation: u64, size: [u32; 2]) -> Key {
    (generation, u64::from(size[0]), u64::from(size[1]), 0x4752_4159_4d41_50)
}
fn gray_mask_source_identity(key: Key, owner: u64, shell: u64, configuration_key: u64) -> (Key, u64, u64, u64) {
    (key, owner, shell, configuration_key)
}
struct GpuSurface {
    maps: Vec<Map>,
    gpu_spares: Vec<Option<Map>>,
    present: [bool; 5],
    use_color_for_pbr: bool,
    version: u64,
}
impl GpuSurface {
    fn flags(&self) -> u32 {
        self.present.iter().enumerate().fold(0, |bits, (i, yes)| bits | (u32::from(*yes) << i))
    }
}
struct GpuEnvironment {
    source: Arc<photocraft_uv::lighting::Environment>,
    layout: wgpu::BindGroupLayout,
    output_hash: u64,
    group: wgpu::BindGroup,
    _textures: Vec<wgpu::Texture>,
}
impl GpuEnvironment {
    fn new(
        rs: &RenderState,
        _mips: &crate::uv_mips::Mips,
        source: Arc<photocraft_uv::lighting::Environment>,
        output: Option<&(u64, Image)>,
    ) -> Result<Self, String> {
        let device = &rs.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv environment"),
            entries: &[
                env_texture_entry(0),
                env_texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                env_texture_entry(3),
                env_texture_entry(4),
            ],
        });
        let upload = |levels: &[Image]| -> Result<(wgpu::Texture, wgpu::TextureView), String> {
            let first = levels.first().ok_or("Empty environment")?;
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("uv linear HDR environment"),
                size: extent([first.width as u32, first.height as u32]),
                mip_level_count: levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for (mip, im) in levels.iter().enumerate() {
                let data: Vec<_> = im.rgba.iter().flat_map(|p| p.iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(*x).to_le_bytes())).collect();
                rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: mip as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(im.width as u32 * 8), rows_per_image: Some(im.height as u32) },
                    extent([im.width as u32, im.height as u32]),
                );
            }
            let view = tex.create_view(&Default::default());
            Ok((tex, view))
        };
        let (specular, sv) = upload(&source.specular)?;
        let (diffuse, dv) = upload(std::slice::from_ref(&source.diffuse))?;
        let (brdf, bv) = upload(std::slice::from_ref(&photocraft_uv::lighting::brdf_lut()))?;
        let (output_texture, ov) = upload(std::slice::from_ref(output.map_or(&crate::uv_lighting::identity_lut(), |(_, im)| im)))?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv sky sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uv sky"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&bv) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&ov) },
            ],
        });
        Ok(Self { source, layout, output_hash: output.map_or(0, |(hash, _)| *hash), group, _textures: vec![specular, diffuse, brdf, output_texture] })
    }
}
fn env_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
struct Map {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    key: Key,
    // CPU snapshots only exist for CPU-uploaded maps. A compositor-backed map has no
    // byte mirror: keeping one here would either require a readback or let a later
    // changed-region upload compare against stale pixels.
    data: Option<Vec<u8>>,
    source: MapSource,
    cutout: bool,
    levels: Vec<wgpu::TextureView>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MapSource {
    Cpu,
    GpuComposite,
    GpuMaskStack,
    CpuFallback,
}
struct Material {
    masked: bool,
    cutout: bool,
    color: MaterialColor,
    normal: Map,
    group: wgpu::BindGroup,
    _uniform: wgpu::Buffer,
    surface_version: u64,
}
enum MaterialColor {
    Owned(Box<Map>),
    Shared(Arc<Map>),
    Surface { size: [u32; 2], key: Key, cutout: bool },
}
impl MaterialColor {
    fn size(&self) -> [u32; 2] {
        match self {
            Self::Owned(map) => map.size,
            Self::Shared(map) => map.size,
            Self::Surface { size, .. } => *size,
        }
    }
    fn key(&self) -> Key {
        match self {
            Self::Owned(map) => map.key,
            Self::Shared(map) => map.key,
            Self::Surface { key, .. } => *key,
        }
    }
    fn cutout(&self) -> bool {
        match self {
            Self::Owned(map) => map.cutout,
            Self::Shared(map) => map.cutout,
            Self::Surface { cutout, .. } => *cutout,
        }
    }
}
fn material_map_key(key: Key, size: [u32; 2]) -> Key {
    (key.0, key.1, key.2, key.3 ^ ((u64::from(size[0])) << 32) ^ u64::from(size[1]))
}
struct Batch {
    material: usize,
    vertices: std::ops::Range<u32>,
}
struct Mesh {
    geometry: Arc<Geometry>,
    vertices: wgpu::Buffer,
    batches: Vec<Batch>,
    wire_vertices: wgpu::Buffer,
    wire_batches: Vec<Batch>,
    selection: wgpu::Buffer,
    group: wgpu::BindGroup,
    selected: BTreeSet<usize>,
}
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth: wgpu::TextureView,
    _depth_msaa: Option<wgpu::Texture>,
    depth_msaa: Option<wgpu::TextureView>,
    _msaa: Option<wgpu::Texture>,
    msaa: Option<wgpu::TextureView>,
    size: [u32; 2],
    samples: u32,
}

type FrameKey = (u64, Camera, [u32; 2], bool, u64, bool, [u8; 4], u32, Option<[u32; 12]>);

pub(crate) struct Renderer {
    rs: RenderState,
    health: photocraft_gpu::DeviceHealth,
    adapter_label: String,
    camera: wgpu::Buffer,
    lighting: photocraft_uv::lighting::Settings,
    environment: GpuEnvironment,
    surfaces: Vec<GpuSurface>,
    background_pipelines: Vec<wgpu::RenderPipeline>,
    wireframe_pipelines: Vec<wgpu::RenderPipeline>,
    symmetry_fill_pipelines: Vec<wgpu::RenderPipeline>,
    symmetry_outline_pipelines: Vec<wgpu::RenderPipeline>,
    camera_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    mask_sampler: wgpu::Sampler,
    mips: crate::uv_mips::Mips,
    compositor: Option<photocraft_gpu::Compositor>,
    mask_stack_renderer: photocraft_gpu::MaskStackRenderer,
    gray_mask_map: Option<Arc<Map>>,
    gray_mask_source_key: Option<(Key, u64, u64, u64)>,
    gray_mask_texture_generation: u64,
    gray_mask_materials: BTreeSet<usize>,
    gray_mask_rejected: Option<(u64, [u32; 2], u64, u64)>,
    gray_mask_brush_identity: Option<(u64, u64, u64, u64, [u32; 2])>,
    gray_mask_brush_signals: Option<(Arc<AtomicBool>, Arc<Mutex<Option<Result<Vec<photocraft_gpu::MaskBrushPatch>, String>>>>)>,
    gray_mask_brush_readback_done: Option<u64>,
    compositor_error: Option<String>,
    last_map_stats: Option<photocraft_gpu::Stats>,
    #[cfg(test)]
    pub(crate) last_map_counters: Option<photocraft_gpu::Stats>,
    map_blit_layout: wgpu::BindGroupLayout,
    map_blit: wgpu::RenderPipeline,
    pipelines: Vec<wgpu::RenderPipeline>,
    aa_samples: u32,
    mesh: Option<Mesh>,
    materials: Vec<Option<Material>>,
    target: Option<Target>,
    spare_target: Option<Target>,
    texture_id: Option<egui::TextureId>,
    last: Option<FrameKey>,
    wireframe_enabled: bool,
    wireframe_color: [u8; 4],
    wireframe_opacity: f32,
    map_generation: u64,
    last_pbr_active: bool,
    regions: std::collections::BTreeMap<(usize, bool), crate::uv_lighting::RegionHint>,
    #[cfg(test)]
    timer: Option<photocraft_gpu::timing::GpuTimer>,
}
pub(crate) struct Draw<'a> {
    pub scene: &'a Scene,
    pub geometry: Arc<Geometry>,
    pub camera: Camera,
    pub size: [u32; 2],
    pub antialias: bool,
    pub selected: &'a BTreeSet<usize>,
    pub hidden: &'a BTreeSet<usize>,
    pub generation: u64,
    pub colors: &'a [Option<Image>],
    pub normals: &'a [Option<Image>],
    pub keys: &'a [Key],
    pub normal_keys: &'a [(u64, u64, u64)],
    pub masked: &'a BTreeSet<usize>,
    /// Four corners in prepared mesh coordinates. The GPU renders them depth-tested.
    pub symmetry_plane: Option<[[f32; 3]; 4]>,
}

/// Source for a GPU-rendered native Gray mask map. A normal document Arc keeps the canonical
/// stack surfaces alive while the renderer updates its own GPU-only display cache.
pub(crate) struct GrayMaskGpuSource {
    pub document: Arc<photocraft_doc::Document>,
    pub layer: photocraft_doc::LayerId,
    pub shell_ids: Arc<Vec<u32>>,
    pub owner_key: u64,
    pub shell_key: u64,
    pub materials: BTreeSet<usize>,
    pub key: Key,
    pub brush_preview: Option<GrayMaskGpuBrushInput>,
}

/// Per-frame bridge from the authoritative engine Dab stream to a transient GPU preview.
/// The queue is shared with the UI source builder so samples remain pending if the GPU renderer
/// cannot consume them in the current frame.
pub(crate) struct GrayMaskGpuBrushInput {
    pub stroke_key: u64,
    pub begin: Option<(photocraft_raster::Surface, photocraft_gpu::MaskBrushSettings)>,
    pub pending_dabs: Arc<Mutex<VecDeque<photocraft_gpu::MaskRoundDab>>>,
    pub release_requested: Arc<AtomicBool>,
    pub failed: Arc<AtomicBool>,
    pub patches: Arc<Mutex<Option<Result<Vec<photocraft_gpu::MaskBrushPatch>, String>>>>,
}
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn valid_symmetry_plane(corners: &[[f32; 3]; 4]) -> bool {
    corners.iter().flatten().all(|value| value.is_finite() && value.abs() <= 10_000.0)
}
fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 }
}
fn texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent(size),
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
impl Renderer {
    fn ensure_surface(&mut self, id: usize) -> Result<(), String> {
        while self.surfaces.len() <= id {
            let mut maps = Vec::new();
            for _ in 0..5 {
                maps.push(self.map_region(None, Key::default(), true, None, None)?);
            }
            self.surfaces.push(GpuSurface {
                maps,
                gpu_spares: std::iter::repeat_with(|| None).take(5).collect(),
                present: [false; 5],
                use_color_for_pbr: false,
                version: 0,
            });
        }
        Ok(())
    }
    /// Refresh the native Gray mask map only when its document/view key changes. The texture is
    /// GPU-resident and shared by every UV material bound to that mask document.
    pub(crate) fn set_gray_mask_source(&mut self, source: Option<&GrayMaskGpuSource>) {
        let Some(source) = source else {
            self.fail_gray_mask_brush_if_pending("Gray mask GPU source unavailable");
            self.gray_mask_rejected = None;
            self.gray_mask_materials.clear();
            let map_changed = self.gray_mask_map.take().is_some();
            let key_changed = self.gray_mask_source_key.take().is_some();
            let changed = map_changed || key_changed;
            self.mask_stack_renderer.clear_cache();
            self.gray_mask_brush_identity = None;
            self.gray_mask_brush_readback_done = None;
            if changed {
                self.last = None;
                self.map_generation = self.map_generation.wrapping_add(1);
            }
            return;
        };
        let size = [source.document.size.width, source.document.size.height];
        let scope_changed = self.gray_mask_materials != source.materials;
        self.gray_mask_materials = source.materials.clone();
        let Some(stack) = source.document.layer(source.layer).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()) else {
            self.set_gray_mask_source(None);
            return;
        };
        let configuration_key = photocraft_gpu::MaskStackRenderer::configuration_key(stack);
        let source_identity = gray_mask_source_identity(source.key, source.owner_key, source.shell_key, configuration_key);
        let brush_identity = source.brush_preview.as_ref().map(|input| (input.stroke_key, source.owner_key, source.shell_key, configuration_key, size));
        if self.gray_mask_brush_identity.is_some_and(|active| Some(active) == brush_identity) && self.gray_mask_map.as_ref().is_some_and(|map| map.size == size)
        {
            // `canvas::display_doc` advances its live key for every dab batch, while the
            // canonical Paint surface remains unchanged on this GPU-only path. Re-rendering the
            // canonical stack here would invalidate the brush owner's cache and make every move a
            // full-frame recomposition before the dirty dab regions are applied.
            self.gray_mask_source_key = Some(source_identity);
            if scope_changed {
                self.last = None;
                self.map_generation = self.map_generation.wrapping_add(1);
            }
            self.sync_gray_mask_brush_preview(source);
            return;
        }
        if self.gray_mask_source_key == Some(source_identity) && self.gray_mask_map.as_ref().is_some_and(|map| map.size == size) {
            if scope_changed {
                self.last = None;
                self.map_generation = self.map_generation.wrapping_add(1);
            }
            self.sync_gray_mask_brush_preview(source);
            return;
        }
        let rejection_key = (source.owner_key, size, source.shell_key, configuration_key);
        if self.gray_mask_rejected == Some(rejection_key) {
            if let Some(input) = source.brush_preview.as_ref() {
                Self::fail_gray_mask_brush_input(input, "Gray mask GPU configuration is unsupported");
            }
            self.mask_stack_renderer.cancel_mask_brush_preview();
            self.gray_mask_brush_identity = None;
            return;
        }
        if let Err(error) = photocraft_gpu::MaskStackRenderer::validate(stack) {
            log::warn!("GPU Gray mask preview fell back to its CPU cache: {error}");
            if let Some(input) = source.brush_preview.as_ref() {
                Self::fail_gray_mask_brush_input(input, error.clone());
            }
            self.mask_stack_renderer.clear_cache();
            self.set_gray_mask_source(None);
            self.gray_mask_rejected = Some(rejection_key);
            return;
        }
        if let Some(map) = self.gray_mask_map.as_ref().filter(|map| map.size == size) {
            if let Err(error) = self.mask_stack_renderer.render_cached(
                &self.rs.device,
                &self.rs.queue,
                stack,
                size,
                &source.shell_ids,
                source.owner_key,
                source.shell_key,
                &map.texture,
            ) {
                log::warn!("GPU Gray mask preview fell back to its CPU cache: {error}");
                if let Some(input) = source.brush_preview.as_ref() {
                    Self::fail_gray_mask_brush_input(input, error.clone());
                }
                self.mask_stack_renderer.clear_cache();
                self.set_gray_mask_source(None);
                self.gray_mask_rejected = Some(rejection_key);
                return;
            }
            self.gray_mask_source_key = Some(source_identity);
        } else {
            let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("uv gpu gray mask stack"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            if let Err(error) = self.mask_stack_renderer.render_cached(
                &self.rs.device,
                &self.rs.queue,
                stack,
                size,
                &source.shell_ids,
                source.owner_key,
                source.shell_key,
                &texture,
            ) {
                log::warn!("GPU Gray mask preview fell back to its CPU cache: {error}");
                if let Some(input) = source.brush_preview.as_ref() {
                    Self::fail_gray_mask_brush_input(input, error.clone());
                }
                self.mask_stack_renderer.clear_cache();
                self.gray_mask_rejected = Some(rejection_key);
                self.set_gray_mask_source(None);
                self.gray_mask_rejected = Some(rejection_key);
                return;
            }
            let view = texture.create_view(&Default::default());
            let level = texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: 0, mip_level_count: Some(1), ..Default::default() });
            self.gray_mask_texture_generation = self.gray_mask_texture_generation.wrapping_add(1);
            let map_key = gray_mask_map_key(self.gray_mask_texture_generation, size);
            self.gray_mask_map =
                Some(Arc::new(Map { texture, view, size, key: map_key, data: None, source: MapSource::GpuMaskStack, cutout: false, levels: vec![level] }));
            self.gray_mask_source_key = Some(source_identity);
        }
        self.gray_mask_rejected = None;
        self.last = None;
        self.map_generation = self.map_generation.wrapping_add(1);
        self.sync_gray_mask_brush_preview(source);
    }

    /// Consume one UI-frame batch of authoritative dabs, then render the transient Paint
    /// surface into the same stable Gray material map used by the regular stack preview.
    fn sync_gray_mask_brush_preview(&mut self, source: &GrayMaskGpuSource) {
        let Some(input) = source.brush_preview.as_ref() else {
            self.fail_gray_mask_brush_if_pending("Gray mask brush preview disappeared");
            self.mask_stack_renderer.cancel_mask_brush_preview();
            self.gray_mask_brush_identity = None;
            return;
        };
        if self.gray_mask_brush_readback_done == Some(input.stroke_key) {
            return;
        }
        self.gray_mask_brush_signals = Some((Arc::clone(&input.failed), Arc::clone(&input.patches)));

        let stack = source.document.layer(source.layer).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref());
        let configuration_key = stack.map_or(0, photocraft_gpu::MaskStackRenderer::configuration_key);
        let expected_identity =
            (input.stroke_key, source.owner_key, source.shell_key, configuration_key, [source.document.size.width, source.document.size.height]);

        // A completed readback is owned by the release mailbox. Do not re-render/re-read it while
        // the UI waits for the engine to apply patches and finish the live stroke.
        if self.gray_mask_brush_identity == Some(expected_identity)
            && input.release_requested.load(Ordering::Acquire)
            && input.patches.lock().is_ok_and(|mailbox| mailbox.is_some())
        {
            return;
        }

        let size = [source.document.size.width, source.document.size.height];
        if let Some((base, settings)) = input.begin.as_ref() {
            if let Err(error) =
                self.mask_stack_renderer.begin_round_mask_brush_preview(&self.rs.device, &self.rs.queue, input.stroke_key, size, base, *settings)
            {
                log::warn!("Could not start GPU Gray mask brush preview; using CPU stroke path: {error}");
                Self::fail_gray_mask_brush_input(input, error);
                self.mask_stack_renderer.cancel_mask_brush_preview();
                self.gray_mask_brush_identity = None;
                return;
            }
            self.gray_mask_brush_identity = Some(expected_identity);
            self.gray_mask_brush_readback_done = None;
        } else if self.gray_mask_brush_identity != Some(expected_identity) {
            log::warn!("GPU Gray mask brush preview lost its stroke state; using CPU stroke path");
            Self::fail_gray_mask_brush_input(input, "GPU Gray mask brush preview state was lost");
            self.mask_stack_renderer.cancel_mask_brush_preview();
            self.gray_mask_brush_identity = None;
            return;
        }

        let queued = match input.pending_dabs.lock() {
            Ok(queue) => queue.iter().copied().collect::<Vec<_>>(),
            Err(_) => {
                log::warn!("GPU Gray mask brush Dab queue was poisoned; using CPU stroke path");
                Self::fail_gray_mask_brush_input(input, "GPU Gray mask brush Dab queue was poisoned");
                self.mask_stack_renderer.cancel_mask_brush_preview();
                self.gray_mask_brush_identity = None;
                return;
            }
        };
        let dirty = match self.mask_stack_renderer.push_round_mask_brush_dabs(&self.rs.device, &self.rs.queue, &queued) {
            Ok(dirty) => dirty,
            Err(error) => {
                log::warn!("GPU Gray mask brush rejected a Dab batch; using CPU stroke path: {error}");
                Self::fail_gray_mask_brush_input(input, error);
                self.mask_stack_renderer.cancel_mask_brush_preview();
                self.gray_mask_brush_identity = None;
                return;
            }
        };
        if !queued.is_empty() {
            match input.pending_dabs.lock() {
                Ok(mut queue) => {
                    for _ in 0..queued.len().min(queue.len()) {
                        queue.pop_front();
                    }
                }
                Err(_) => {
                    log::warn!("GPU Gray mask Dab queue became unavailable after submission; falling back to CPU");
                    Self::fail_gray_mask_brush_input(input, "GPU Gray mask Dab queue was poisoned after submission");
                    self.mask_stack_renderer.cancel_mask_brush_preview();
                    self.gray_mask_brush_identity = None;
                    return;
                }
            }
        }

        if let Err(error) = self.render_gray_mask_brush_preview(
            source,
            source
                .document
                .layer(source.layer)
                .and_then(|layer| layer.mask.as_ref())
                .and_then(|mask| mask.paint.as_ref())
                .map_or(usize::MAX, |stack| stack.active),
            &dirty,
        ) {
            log::warn!("Could not render GPU Gray mask brush preview; using CPU stroke path: {error}");
            Self::fail_gray_mask_brush_input(input, error);
            self.mask_stack_renderer.cancel_mask_brush_preview();
            self.gray_mask_brush_identity = None;
            return;
        }

        if input.release_requested.load(Ordering::Acquire) {
            match self.read_gray_mask_brush_patches() {
                Ok(patches) => {
                    if let Ok(mut mailbox) = input.patches.lock() {
                        *mailbox = Some(Ok(patches));
                        self.gray_mask_brush_readback_done = Some(input.stroke_key);
                        self.gray_mask_brush_signals = None;
                    } else {
                        Self::fail_gray_mask_brush_input(input, "GPU Gray mask patch mailbox was poisoned");
                        self.mask_stack_renderer.cancel_mask_brush_preview();
                        self.gray_mask_brush_identity = None;
                    }
                }
                Err(error) => {
                    log::warn!("Could not read back GPU Gray mask brush result; replaying on CPU: {error}");
                    Self::fail_gray_mask_brush_input(input, error);
                    self.mask_stack_renderer.cancel_mask_brush_preview();
                    self.gray_mask_brush_identity = None;
                }
            }
        }
    }

    /// Recompose the current mask stack with the transient Paint map. `set_gray_mask_source` must
    /// have prepared the stable output map for this document before this method is called.
    pub(crate) fn render_gray_mask_brush_preview(
        &mut self,
        source: &GrayMaskGpuSource,
        paint_layer_index: usize,
        dirty_regions: &[photocraft_geom::Rect],
    ) -> Result<(), String> {
        let stack = source
            .document
            .layer(source.layer)
            .and_then(|layer| layer.mask.as_ref())
            .and_then(|mask| mask.paint.as_ref())
            .ok_or("Gray mask Paint stack disappeared during GPU brush preview")?;
        let size = [source.document.size.width, source.document.size.height];
        let map = self.gray_mask_map.as_ref().filter(|map| map.size == size).ok_or("Gray mask GPU target is not ready")?;
        self.mask_stack_renderer.render_cached_with_brush_preview(
            &self.rs.device,
            &self.rs.queue,
            stack,
            size,
            &source.shell_ids,
            source.shell_key,
            paint_layer_index,
            dirty_regions,
            &map.texture,
        )?;
        self.last = None;
        Ok(())
    }

    /// Read the final changed RGBA8 Paint patches once at stroke release for the engine's
    /// canonical Surface/undoable commit path.
    pub(crate) fn read_gray_mask_brush_patches(&self) -> Result<Vec<photocraft_gpu::MaskBrushPatch>, String> {
        self.mask_stack_renderer.read_mask_brush_patches(&self.rs.device, &self.rs.queue)
    }

    fn fail_gray_mask_brush_if_pending(&mut self, reason: &str) {
        if let Some((failed, patches)) = self.gray_mask_brush_signals.take() {
            if let Ok(mut mailbox) = patches.lock() {
                if mailbox.is_none() {
                    failed.store(true, Ordering::Release);
                    *mailbox = Some(Err(reason.to_owned()));
                }
            } else {
                failed.store(true, Ordering::Release);
            }
        }
    }

    fn fail_gray_mask_brush_input(input: &GrayMaskGpuBrushInput, reason: impl Into<String>) {
        input.failed.store(true, Ordering::Release);
        if let Ok(mut mailbox) = input.patches.lock()
            && mailbox.is_none()
        {
            *mailbox = Some(Err(reason.into()));
        }
    }

    pub(crate) fn set_lighting(
        &mut self,
        settings: photocraft_uv::lighting::Settings,
        maps: &crate::uv_lighting::SurfaceMaps,
        environment: Arc<photocraft_uv::lighting::Environment>,
    ) -> Result<(), String> {
        let settings = settings.sanitized();
        if settings != self.lighting {
            self.lighting = settings;
            self.last = None;
            self.last_pbr_active = false;
        }
        if !Arc::ptr_eq(&self.environment.source, &environment) || self.environment.output_hash != maps.output.as_ref().map_or(0, |(hash, _)| *hash) {
            self.environment = GpuEnvironment::new(&self.rs, &self.mips, environment, maps.output.as_ref())?;
            self.last = None;
        }
        for material in 0..maps.images.iter().map(Vec::len).chain(maps.gpu.iter().map(Vec::len)).max().unwrap_or(0) {
            self.ensure_surface(material)?;
            let use_color_for_pbr = maps.use_color_for_pbr.get(material).copied().unwrap_or(false);
            if let Some(surface) = self.surfaces.get_mut(material)
                && surface.use_color_for_pbr != use_color_for_pbr
            {
                surface.use_color_for_pbr = use_color_for_pbr;
                surface.version = surface.version.wrapping_add(1);
                self.map_generation = self.map_generation.wrapping_add(1);
            }
            for kind in 0..5 {
                let image = maps.images.get(kind).and_then(|m| m.get(material)).and_then(Option::as_ref);
                let gpu_request = maps.gpu.get(kind).and_then(|m| m.get(material)).and_then(Option::as_ref);
                let key = gpu_request.map_or_else(|| maps.keys.get(kind).and_then(|m| m.get(material)).copied().unwrap_or_default(), |request| request.key);
                let present = image.is_some() || gpu_request.is_some();
                let gpu_source = gpu_request.is_some();
                let changed = self.surfaces.get(material).and_then(|s| s.maps.get(kind)).is_none_or(|m| {
                    m.key != key || if gpu_source { !matches!(m.source, MapSource::GpuComposite | MapSource::CpuFallback) } else { m.source != MapSource::Cpu }
                }) || self.surfaces.get(material).and_then(|s| s.present.get(kind)).copied() != Some(present);
                if changed {
                    let hint = maps.regions.get(kind).and_then(|slots| slots.get(material)).cloned().flatten();
                    let map = if let Some(request) = gpu_request {
                        let active_matches = request.damage.is_some()
                            && request.previous_key.is_some_and(|expected| {
                                self.surfaces.get(material).and_then(|surface| surface.maps.get(kind)).is_some_and(|map| {
                                    map.key == expected
                                        && map.source == MapSource::GpuComposite
                                        && map.size == [request.doc.size.width, request.doc.size.height]
                                })
                            });
                        let previous = if active_matches {
                            let empty = self.map_single(None, Key::default(), true, None, None)?;
                            let previous =
                                self.surfaces.get_mut(material).and_then(|surface| surface.maps.get_mut(kind)).map(|map| std::mem::replace(map, empty));
                            if let Some(surface) = self.surfaces.get_mut(material)
                                && let Some(spare) = surface.gpu_spares.get_mut(kind)
                            {
                                *spare = None;
                            }
                            previous
                        } else {
                            self.surfaces.get_mut(material).and_then(|surface| surface.gpu_spares.get_mut(kind)).and_then(Option::take)
                        };
                        let damage = active_matches.then_some(request.damage).flatten();
                        match self.map_gpu_composite(&request.doc, request.key, request.linear_srgb, damage, previous) {
                            Ok(map) => map,
                            Err(_) => {
                                // Unsupported GPU features and transient device errors keep a
                                // complete CPU map available; never expose a partially written
                                // destination or leave the previous revision active.
                                let fallback_doc = crate::gpu_canvas::materialize_paint_masks(&request.doc);
                                let rendered = photocraft_compose::render(&fallback_doc, fallback_doc.bounds());
                                let rgba = if request.linear_srgb {
                                    rendered
                                        .px
                                        .into_iter()
                                        .map(|mut p| {
                                            for c in &mut p[..3] {
                                                *c = if *c <= 0.04045 { *c / 12.92 } else { ((*c + 0.055) / 1.055).powf(2.4) };
                                            }
                                            p
                                        })
                                        .collect()
                                } else {
                                    rendered.px
                                };
                                let image = Image { width: request.doc.size.width as usize, height: request.doc.size.height as usize, rgba };
                                let empty = self.map_single(None, Key::default(), true, None, None)?;
                                let previous = self.surfaces.get_mut(material).and_then(|s| s.maps.get_mut(kind)).map(|map| std::mem::replace(map, empty));
                                let mut map = self.map_region(Some(&image), request.key, true, previous, None)?;
                                map.source = MapSource::CpuFallback;
                                map
                            }
                        }
                    } else {
                        let empty = self.map_single(None, Key::default(), true, None, None)?;
                        let previous = self.surfaces.get_mut(material).and_then(|s| s.maps.get_mut(kind)).map(|map| std::mem::replace(map, empty));
                        self.map_region(image, key, true, previous, hint)?
                    };
                    let mut map = map;
                    map.cutout = kind == 0 && (gpu_source || image.is_some_and(|im| im.rgba.iter().any(|pixel| pixel[3] < 1.0)));
                    if let Some(surface) = self.surfaces.get_mut(material) {
                        if let Some(slot) = surface.maps.get_mut(kind) {
                            let old = std::mem::replace(slot, map);
                            if old.source == MapSource::GpuComposite
                                && let Some(spare) = surface.gpu_spares.get_mut(kind)
                            {
                                *spare = Some(old);
                            }
                        }
                        if let Some(v) = surface.present.get_mut(kind) {
                            *v = present;
                        }
                        surface.version = surface.version.wrapping_add(1);
                    }
                    self.map_generation = self.map_generation.wrapping_add(1);
                }
            }
        }
        Ok(())
    }
    pub fn adapter_label(&self) -> &str {
        &self.adapter_label
    }
    pub(crate) fn take_map_stats(&mut self) -> Option<photocraft_gpu::Stats> {
        let stats = self.last_map_stats.take();
        #[cfg(test)]
        {
            self.last_map_counters = stats;
        }
        stats
    }
    pub fn texture_sizes(&self) -> Vec<([u32; 2], [u32; 2])> {
        self.materials.iter().map(|m| m.as_ref().map_or(([0; 2], [0; 2]), |m| (m.color.size(), m.normal.size))).collect()
    }
    pub fn max_texture_side(&self) -> u32 {
        self.rs.device.limits().max_texture_dimension_2d
    }
    pub fn frame_info(&self) -> Option<([u32; 2], u32)> {
        self.target.as_ref().map(|t| (t.size, t.samples))
    }
    pub fn set_wireframe(&mut self, enabled: bool, color: [u8; 4], opacity: f32) {
        let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.75 };
        self.wireframe_enabled = enabled;
        self.wireframe_color = color;
        self.wireframe_opacity = opacity;
    }
    pub fn diagnostics(&self, channel: photocraft_uv::preview::Channel) -> serde_json::Value {
        let frame = self.frame_info();
        let maps: Vec<_> = self
            .surfaces
            .iter()
            .enumerate()
            .map(|(material, surface)| {
                let maps: Vec<_> = surface
                    .maps
                    .iter()
                    .enumerate()
                    .filter(|(kind, _)| surface.present.get(*kind).copied().unwrap_or(false))
                    .map(|(kind, map)| {
                        let channel = match kind {
                            0 => "Color",
                            1 => "Normal",
                            2 => "AO",
                            3 => "Metallic",
                            4 => "Roughness",
                            _ => "Unknown",
                        };
                        let source = match map.source {
                            MapSource::Cpu => "cpu",
                            MapSource::GpuComposite => "gpu-composite",
                            MapSource::GpuMaskStack => "gpu-mask-stack",
                            MapSource::CpuFallback => "cpu-fallback",
                        };
                        serde_json::json!({"channel": channel, "source": source, "size": map.size})
                    })
                    .collect();
                serde_json::json!({"material": material, "maps": maps})
            })
            .collect();
        serde_json::json!({
            "rendererActive": self.health.is_ok(),
            "backend": self.adapter_label,
            "renderTargetSize": frame.map(|(size, _)| size),
            "samples": frame.map(|(_, samples)| samples),
            "pbrEnabled": self.lighting.pbr,
            "pbrActive": self.last_pbr_active && self.last.as_ref().is_some_and(|(_, camera, _, _, _, _, _, _, _)| camera.channel == channel),
            "channel": channel.label(),
            "mapCompositorActive": self.compositor.is_some() && self.health.is_ok(),
            "mapCompositorError": self.compositor_error,
            "maps": maps,
            "maxTextureDimension2d": self.max_texture_side(),
        })
    }
    pub fn clear(&mut self) {
        self.regions.clear();
        self.mesh = None;
        self.materials.clear();
        self.surfaces.clear();
        self.gray_mask_map = None;
        self.gray_mask_source_key = None;
        self.gray_mask_materials.clear();
        self.gray_mask_rejected = None;
        self.gray_mask_brush_identity = None;
        self.gray_mask_brush_signals = None;
        self.gray_mask_brush_readback_done = None;
        self.target = None;
        self.spare_target = None;
        self.last = None;
        self.last_pbr_active = false;
        if let Some(id) = self.texture_id.take() {
            self.rs.renderer.write().free_texture(&id);
        }
    }
    pub fn new(rs: RenderState, health: photocraft_gpu::DeviceHealth) -> Result<Self, String> {
        Self::new_with_map_compositor(rs, health, true)
    }
    fn new_with_map_compositor(rs: RenderState, health: photocraft_gpu::DeviceHealth, enable_map_compositor: bool) -> Result<Self, String> {
        use std::future::Future;
        let validation = rs.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = rs.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::create(rs.clone(), health, enable_map_compositor)));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        for pop in [memory.pop(), validation.pop()] {
            if let std::task::Poll::Ready(Some(error)) = std::pin::pin!(pop).poll(&mut cx) {
                return Err(format!("3D GPU setup failed: {error}"));
            }
        }
        result.map_err(|_| "3D GPU setup panicked; using CPU preview".to_owned())?
    }
    fn create(rs: RenderState, health: photocraft_gpu::DeviceHealth, enable_map_compositor: bool) -> Result<Self, String> {
        if !crate::gpu_canvas::supports_f16_canvas(&rs.adapter) {
            return Err("GPU does not support filtered half-float normal-map mip targets".into());
        }
        let device = &rs.device;
        if device.limits().max_storage_buffers_per_shader_stage == 0 {
            return Err("GPU model preview needs fragment storage buffers".into());
        }
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv camera"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv material"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mips = crate::uv_mips::Mips::new(&rs);
        let mask_stack_renderer = photocraft_gpu::MaskStackRenderer::new(device);
        let environment = GpuEnvironment::new(&rs, &mips, Arc::new(photocraft_uv::lighting::Environment::studio()), None)?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uv"),
            bind_group_layouts: &[Some(&camera_layout), Some(&material_layout), Some(&environment.layout)],
            immediate_size: 0,
        });
        let shader = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("uv"), source: wgpu::ShaderSource::Wgsl(include_str!("uv_gpu.wgsl").into()) });
        let aa_samples = if [FORMAT, DEPTH].iter().all(|f| rs.adapter.get_texture_format_features(*f).flags.sample_count_supported(4)) { 4 } else { 1 };
        let make_pipeline = |count, cull, cutout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("uv model"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: STRIDE,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3,4=>Float32x3,5=>Float32x2,6=>Uint32],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if cutout { "fs_cutout" } else { "fs_opaque" }),
                    targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let background_pipelines = [1, aa_samples]
            .into_iter()
            .map(|count| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("uv sky"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_background"), buffers: &[], compilation_options: Default::default() },
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState { count, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_background"),
                        targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        let wireframe_pipelines = [1, aa_samples]
            .into_iter()
            .map(|count| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("uv depth-tested wireframe"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_wireframe"),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: 24,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3],
                        })],
                        compilation_options: Default::default(),
                    },
                    primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState { count, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_wireframe"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: FORMAT,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        let symmetry_pipeline = |count, label, topology, fragment| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_symmetry_plane"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    // The vertex shader applies a tiny clip-space depth offset; GPU depth
                    // bias is only valid for triangle topologies, not the outline lines.
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let symmetry_fill_pipelines = [1, aa_samples]
            .into_iter()
            .map(|count| symmetry_pipeline(count, "uv depth-tested symmetry plane fill", wgpu::PrimitiveTopology::TriangleList, "fs_symmetry_fill"))
            .collect();
        let symmetry_outline_pipelines = [1, aa_samples]
            .into_iter()
            .map(|count| symmetry_pipeline(count, "uv depth-tested symmetry plane outline", wgpu::PrimitiveTopology::LineList, "fs_symmetry_outline"))
            .collect();
        let mut pipelines = Vec::new();
        for samples in [1, aa_samples] {
            for cull in [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)] {
                for cutout in [false, true] {
                    pipelines.push(make_pipeline(samples, cull, cutout));
                }
            }
        }
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv camera"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv repeat bilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: if rs.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) { 4 } else { 1 },
            ..Default::default()
        });
        let mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv mask edge-safe sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: if rs.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) { 4 } else { 1 },
            ..Default::default()
        });
        let (mut compositor, compositor_error) = if enable_map_compositor {
            match photocraft_gpu::Compositor::try_new_with_format(device, photocraft_gpu::Compositor::preferred_acc_format(&rs.adapter)) {
                Ok(compositor) => (Some(compositor), None),
                Err(error) => {
                    let detail = error.to_string();
                    log::warn!("UV GPU map compositor unavailable; retaining the 3D renderer with CPU map fallback: {detail}");
                    (None, Some(detail))
                }
            }
        } else {
            (None, Some("disabled".into()))
        };
        if let Some(compositor) = &mut compositor {
            compositor.measure_gpu_time(device, &rs.queue);
        }
        let map_blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv compositor map blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(24),
                    },
                    count: None,
                },
            ],
        });
        let map_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("uv compositor map blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("uv_map.wgsl").into()),
        });
        let map_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uv compositor map blit"),
            bind_group_layouts: &[Some(&map_blit_layout)],
            immediate_size: 0,
        });
        let map_blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("uv compositor map blit"),
            layout: Some(&map_pipeline_layout),
            vertex: wgpu::VertexState { module: &map_shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &map_shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        #[cfg(test)]
        let timer = photocraft_gpu::timing::GpuTimer::new(&rs.device, &rs.queue);
        let adapter_label = format!("{} · {:?}", rs.adapter.get_info().name, rs.adapter.get_info().backend);
        Ok(Self {
            rs,
            health,
            adapter_label,
            camera,
            lighting: photocraft_uv::lighting::Settings { pbr: false, ..Default::default() },
            environment,
            surfaces: Vec::new(),
            background_pipelines,
            wireframe_pipelines,
            symmetry_fill_pipelines,
            symmetry_outline_pipelines,
            camera_layout,
            material_layout,
            sampler,
            mask_sampler,
            mips,
            compositor,
            mask_stack_renderer,
            gray_mask_map: None,
            gray_mask_source_key: None,
            gray_mask_texture_generation: 0,
            gray_mask_materials: BTreeSet::new(),
            gray_mask_rejected: None,
            gray_mask_brush_identity: None,
            gray_mask_brush_signals: None,
            gray_mask_brush_readback_done: None,
            compositor_error,
            last_map_stats: None,
            #[cfg(test)]
            last_map_counters: None,
            map_blit_layout,
            map_blit,
            pipelines,
            aa_samples,
            mesh: None,
            materials: Vec::new(),
            target: None,
            spare_target: None,
            texture_id: None,
            last: None,
            wireframe_enabled: false,
            wireframe_color: [240, 240, 240, 255],
            wireframe_opacity: 0.75,
            map_generation: 0,
            last_pbr_active: false,
            regions: Default::default(),
            #[cfg(test)]
            timer,
        })
    }
    fn mesh(&mut self, geometry: Arc<Geometry>) -> Result<bool, String> {
        if self.mesh.as_ref().is_some_and(|m| Arc::ptr_eq(&m.geometry, &geometry)) {
            return Ok(false);
        }
        let count = geometry.triangles.len().checked_mul(3).ok_or("GPU mesh too large")?;
        let len = count.checked_mul(STRIDE as usize).ok_or("GPU mesh too large")?;
        let wire_count = geometry.triangles.len().checked_mul(6).ok_or("GPU wireframe too large")?;
        let wire_len = wire_count.checked_mul(24).ok_or("GPU wireframe too large")?;
        if len as u64 > self.rs.device.limits().max_buffer_size || wire_len as u64 > self.rs.device.limits().max_buffer_size {
            return Err("Model exceeds the GPU vertex buffer limit".into());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| "Not enough memory to upload model")?;
        let mut wire_bytes = Vec::new();
        wire_bytes.try_reserve_exact(wire_len).map_err(|_| "Not enough memory to upload wireframe")?;
        let mut order: Vec<_> = (0..geometry.triangles.len()).collect();
        order.sort_unstable_by_key(|id| geometry.triangles.get(*id).map_or(usize::MAX, |t| t.material));
        let mut batches: Vec<Batch> = Vec::new();
        let mut wire_batches: Vec<Batch> = Vec::new();
        let mut at = 0u32;
        let mut wire_at = 0u32;
        for id in order {
            let Some(t) = geometry.triangles.get(id) else { continue };
            if let Some(batch) = batches.last_mut().filter(|b| b.material == t.material) {
                batch.vertices.end += 3;
            } else {
                batches.push(Batch { material: t.material, vertices: at..at + 3 });
            }
            if let Some(batch) = wire_batches.last_mut().filter(|b| b.material == t.material) {
                batch.vertices.end += 6;
            } else {
                wire_batches.push(Batch { material: t.material, vertices: wire_at..wire_at + 6 });
            }
            at += 3;
            wire_at += 6;
            for corner in 0..3 {
                for value in t.positions[corner].into_iter().chain(t.normals[corner]).chain(t.face).chain(t.tangent).chain(t.bitangent).chain(t.uv[corner]) {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                // The selection buffer is indexed by face. The CPU expands selected UV
                // shells into their constituent triangles before each draw, so individual
                // polygon selection can use this same depth-tested shader path.
                let triangle = u32::try_from(id).map_err(|_| "UV face count exceeds GPU selection range")?;
                bytes.extend_from_slice(&triangle.to_le_bytes());
            }
            for [a, b] in [[0, 1], [1, 2], [2, 0]] {
                for vertex in [t.positions[a], t.positions[b]] {
                    for value in vertex {
                        wire_bytes.extend_from_slice(&value.to_le_bytes());
                    }
                    for value in t.face {
                        wire_bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        let device = &self.rs.device;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uv static vertices"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let wire_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uv wireframe line vertices"),
            contents: &wire_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let faces = u64::try_from(geometry.triangles.len()).map_err(|_| "UV face count exceeds GPU selection range")?.max(1);
        if faces * 4 > device.limits().max_storage_buffer_binding_size {
            return Err("UV selection exceeds GPU storage limit".into());
        }
        let selection = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv face selection"),
            size: faces * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uv view"),
            layout: &self.camera_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.camera.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: selection.as_entire_binding() },
            ],
        });
        self.mesh = Some(Mesh { geometry, vertices, batches, wire_vertices, wire_batches, selection, group, selected: BTreeSet::new() });
        self.materials.clear();
        self.last = None;
        Ok(true)
    }
    pub(crate) fn hint_region(&mut self, material: usize, normal: bool, previous: Key, rect: Vec<[u32; 4]>) {
        self.regions.insert((material, normal), (previous, rect));
    }
    #[cfg(test)]
    fn map(&self, image: Option<&Image>, key: Key, normal: bool, previous: Option<Map>) -> Result<Map, String> {
        self.map_region(image, key, normal, previous, None)
    }
    fn map_region(
        &self,
        image: Option<&Image>,
        key: Key,
        normal: bool,
        previous: Option<Map>,
        hint: Option<crate::uv_lighting::RegionHint>,
    ) -> Result<Map, String> {
        if let Some((expected, regions)) = hint
            && previous.as_ref().is_some_and(|m| m.source == MapSource::Cpu && m.key == expected)
            && image.is_some_and(|im| previous.as_ref().is_some_and(|m| m.size == [im.width as u32, im.height as u32]))
        {
            let mut current = previous;
            if regions.is_empty()
                && let Some(m) = current.as_mut()
            {
                m.key = key;
            }
            let mut from = expected;
            for rect in regions {
                current = Some(self.map_single(image, key, normal, current, Some((from, rect)))?);
                from = key;
            }
            return current.ok_or_else(|| "Missing texture".into());
        }
        self.map_single(image, key, normal, previous, None)
    }
    fn map_single(&self, image: Option<&Image>, key: Key, normal: bool, mut previous: Option<Map>, hint: Option<(Key, [u32; 4])>) -> Result<Map, String> {
        if hint.is_none() && previous.as_ref().is_some_and(|m| m.source == MapSource::Cpu && m.key == key) {
            return previous.ok_or_else(|| "Missing cached texture".into());
        }
        let size = image.map_or([1, 1], |im| [im.width as u32, im.height as u32]);
        let max = self.rs.device.limits().max_texture_dimension_2d;
        if size.contains(&0)
            || image.is_some_and(|im| im.width > max as usize || im.height > max as usize || im.width.checked_mul(im.height) != Some(im.rgba.len()))
        {
            return Err("Invalid material image dimensions or length".into());
        }
        if let (Some(im), Some((expected, [x, y, w, h]))) = (image, hint)
            && previous.as_ref().is_some_and(|old| old.source == MapSource::Cpu && old.key == expected && old.size == size)
            && x.checked_add(w).is_some_and(|end| end <= size[0])
            && y.checked_add(h).is_some_and(|end| end <= size[1])
        {
            let mut old = previous.take().ok_or("Missing previous map")?;
            let stride = if normal { 8usize } else { 4 };
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(w as usize * h as usize * stride).map_err(|_| "Not enough memory for texture update")?;
            for row in im.rgba.chunks_exact(im.width).skip(y as usize).take(h as usize) {
                for pixel in row.get(x as usize..(x + w) as usize).ok_or("Invalid texture update")? {
                    for value in pixel {
                        if normal {
                            bytes.extend_from_slice(&crate::gpu_canvas::f32_to_f16(*value).to_le_bytes());
                        } else {
                            bytes.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                        }
                    }
                    old.cutout |= !normal && pixel[3] < 1.0;
                }
            }
            if w > 0 && h > 0 {
                self.rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &old.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    &bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * stride as u32), rows_per_image: Some(h) },
                    extent([w, h]),
                );
                for (row, source) in bytes.chunks_exact(w as usize * stride).enumerate() {
                    let start = ((y as usize + row) * im.width + x as usize) * stride;
                    old.data
                        .as_mut()
                        .and_then(|data| data.get_mut(start..start + source.len()))
                        .ok_or("Invalid cached texture update")?
                        .copy_from_slice(source);
                }
                self.mips.regenerate(&self.rs, &old.texture, size, &old.levels, [x, y, w, h]);
            }
            old.key = key;
            return Ok(old);
        }
        let cutout = !normal && image.is_some_and(|im| im.rgba.iter().any(|p| p[3] < 1.0));
        let max = self.rs.device.limits().max_texture_dimension_2d;
        if size.contains(&0) || size.iter().any(|x| *x > max) {
            return Err("Material map exceeds GPU texture limit".into());
        }
        if image.is_some_and(|im| im.width.checked_mul(im.height) != Some(im.rgba.len())) {
            return Err("Invalid material image length".into());
        }
        let data: Vec<u8> = if !normal {
            image.map_or_else(|| vec![255u8; 4], |im| im.rgba.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect())
        } else if let Some(im) = image {
            im.rgba.iter().flat_map(|p| p.iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(*x).to_le_bytes())).collect()
        } else {
            [0.5, 0.5, 1.0, 1.0].into_iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(x).to_le_bytes()).collect()
        };
        let bytes_per_texel = if normal { 8 } else { 4 };
        if let Some(mut old) = previous.filter(|m| m.size == size && m.source == MapSource::Cpu) {
            if let Some([x, y, w, h]) = old.data.as_deref().and_then(|old_data| changed_region(old_data, &data, size, bytes_per_texel)) {
                let offset = (y as usize * size[0] as usize + x as usize) * bytes_per_texel;
                let Some(bytes) = data.get(offset..) else {
                    return Err("Invalid texture update region".into());
                };
                self.rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &old.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * bytes_per_texel as u32), rows_per_image: Some(h) },
                    extent([w, h]),
                );
                self.mips.regenerate(&self.rs, &old.texture, size, &old.levels, [x, y, w, h]);
            }
            old.data = Some(data);
            old.key = key;
            old.cutout = cutout;
            return Ok(old);
        }
        let levels_n = 32 - size[0].max(size[1]).leading_zeros();
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("uv full-resolution map"),
            size: extent(size),
            mip_level_count: levels_n,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if normal { wgpu::TextureFormat::Rgba16Float } else { FORMAT },
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let levels: Vec<_> = (0..levels_n)
            .map(|level| texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() }))
            .collect();
        self.rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * bytes_per_texel as u32), rows_per_image: Some(size[1]) },
            extent(size),
        );
        let view = texture.create_view(&Default::default());
        self.mips.regenerate(&self.rs, &texture, size, &levels, [0, 0, size[0], size[1]]);
        Ok(Map { texture, view, size, key, data: Some(data), source: MapSource::Cpu, cutout, levels })
    }
    fn map_gpu_composite(
        &mut self,
        doc: &photocraft_doc::Document,
        key: Key,
        linear_srgb: bool,
        damage: Option<photocraft_geom::Rect>,
        previous: Option<Map>,
    ) -> Result<Map, String> {
        let size = [doc.size.width, doc.size.height];
        // Avoid allocating a full-resolution texture and every mip view when the optional
        // compositor could not be created. The caller will use its normal CPU map fallback.
        if self.compositor.is_none() {
            return Err("GPU map compositor unavailable".into());
        }
        let max = self.rs.device.limits().max_texture_dimension_2d;
        if size.contains(&0) || size.iter().any(|side| *side > max) {
            return Err("GPU-composited map exceeds the full-resolution texture limit".into());
        }
        let mut map = if let Some(map) = previous.filter(|map| map.source == MapSource::GpuComposite && map.size == size) {
            map
        } else {
            let levels_n = 32 - size[0].max(size[1]).leading_zeros();
            let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("uv compositor PBR map"),
                size: extent(size),
                mip_level_count: levels_n,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let levels: Vec<_> = (0..levels_n)
                .map(|level| texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() }))
                .collect();
            let view = texture.create_view(&Default::default());
            Map { texture, view, size, key, data: None, source: MapSource::GpuComposite, cutout: true, levels }
        };
        let device = &self.rs.device;
        let queue = &self.rs.queue;
        let layout = &self.map_blit_layout;
        let pipeline = &self.map_blit;
        let target = &map.levels[0];
        let linear_srgb = u32::from(linear_srgb);
        let compositor = self.compositor.as_mut().ok_or_else(|| "GPU map compositor unavailable".to_string())?;
        let render_region = damage.map_or_else(|| doc.bounds(), |region| region.intersect(&doc.bounds()));
        if render_region.is_empty() {
            self.last_map_stats = Some(photocraft_gpu::Stats::default());
            map.key = key;
            return Ok(map);
        }
        let stats = compositor
            .render(device, queue, doc, render_region, |encoder, chunk| {
                let [x, y, w, h] = [chunk.rect.x0.max(0) as u32, chunk.rect.y0.max(0) as u32, chunk.rect.width(), chunk.rect.height()];
                if w == 0 || h == 0 || x.saturating_add(w) > size[0] || y.saturating_add(h) > size[1] {
                    return;
                }
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("uv compositor map chunk"),
                    contents: &[
                        x.to_le_bytes(),
                        y.to_le_bytes(),
                        linear_srgb.to_le_bytes(),
                        0u32.to_le_bytes(),
                        chunk.origin[0].to_le_bytes(),
                        chunk.origin[1].to_le_bytes(),
                    ]
                    .concat(),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("uv compositor map chunk"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(chunk.view) },
                        wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("uv compositor map chunk"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            })
            .map_err(|e| e.to_string())?;
        self.last_map_stats = Some(stats);
        map.key = key;
        map.data = None;
        map.source = MapSource::GpuComposite;
        self.mips.regenerate(
            &self.rs,
            &map.texture,
            size,
            &map.levels,
            [render_region.x0 as u32, render_region.y0 as u32, render_region.width(), render_region.height()],
        );
        Ok(map)
    }
    fn materials(&mut self, draw: &Draw<'_>) -> Result<(), String> {
        if draw.scene.materials.len() > 4096 {
            return Err("Model exceeds GPU material budget (4096)".into());
        }
        let gray_mask_map = self.gray_mask_map.clone();
        let keys: Vec<_> = draw
            .scene
            .materials
            .iter()
            .enumerate()
            .map(|(id, _)| {
                let c = draw.keys.get(id).copied().unwrap_or_default();
                let n = draw.normal_keys.get(id).copied().unwrap_or_default();
                let gpu_color = (self.lighting.pbr && draw.camera.channel == photocraft_uv::preview::Channel::Material && !draw.masked.contains(&id))
                    .then(|| {
                        self.surfaces
                            .get(id)
                            .filter(|surface| surface.use_color_for_pbr && surface.present.first().copied().unwrap_or(false))
                            .and_then(|surface| surface.maps.first())
                    })
                    .flatten();
                let gray_mask = (draw.masked.contains(&id) && self.gray_mask_materials.contains(&id)).then(|| gray_mask_map.as_deref()).flatten();
                (
                    gray_mask.map_or_else(
                        || {
                            gpu_color.map_or_else(
                                || {
                                    draw.colors
                                        .get(id)
                                        .and_then(Option::as_ref)
                                        .map_or((0, 0, 0, 0), |im| (c.0, c.1, c.2, c.3 ^ ((im.width as u64) << 32) ^ im.height as u64))
                                },
                                |map| material_map_key(map.key, map.size),
                            )
                        },
                        |map| material_map_key(map.key, map.size),
                    ),
                    draw.normals.get(id).and_then(Option::as_ref).map_or((0, 0, 0, 0), |im| (n.0, n.1, n.2, ((im.width as u64) << 32) | im.height as u64)),
                )
            })
            .collect();
        for (id, material) in draw.scene.materials.iter().enumerate() {
            let Some(&(ck, nk)) = keys.get(id) else { continue };
            if self.materials.get(id).and_then(Option::as_ref).is_some_and(|m| {
                m.masked == draw.masked.contains(&id)
                    && m.color.key() == ck
                    && m.normal.key == nk
                    && m.surface_version == self.surfaces.get(id).map_or(0, |s| s.version)
            }) {
                continue;
            }
            // Cache by document revision: moving the camera never uploads these maps.
            let previous = self.materials.get_mut(id).and_then(Option::take);
            let (pc, pn) = previous.map_or((None, None), |m| {
                (
                    match m.color {
                        MaterialColor::Owned(map) => Some(*map),
                        MaterialColor::Shared(_) | MaterialColor::Surface { .. } => None,
                    },
                    Some(m.normal),
                )
            });
            self.ensure_surface(id)?;
            let color_hint = self.regions.remove(&(id, false));
            let normal_hint = self.regions.remove(&(id, true));
            let surface_color = (self.lighting.pbr && draw.camera.channel == photocraft_uv::preview::Channel::Material && !draw.masked.contains(&id))
                .then(|| {
                    self.surfaces
                        .get(id)
                        .filter(|surface| surface.use_color_for_pbr && surface.present.first().copied().unwrap_or(false))
                        .and_then(|surface| surface.maps.first())
                })
                .flatten();
            let color = if let Some(map) = surface_color {
                MaterialColor::Surface { size: map.size, key: material_map_key(map.key, map.size), cutout: map.cutout }
            } else if draw.masked.contains(&id)
                && self.gray_mask_materials.contains(&id)
                && let Some(map) = gray_mask_map.as_ref()
            {
                MaterialColor::Shared(map.clone())
            } else {
                MaterialColor::Owned(Box::new(self.map_region(draw.colors.get(id).and_then(Option::as_ref), ck, false, pc, color_hint)?))
            };
            let color_view = match &color {
                MaterialColor::Owned(map) => &map.view,
                MaterialColor::Shared(map) => &map.view,
                MaterialColor::Surface { .. } => &surface_color.ok_or("Missing GPU surface map")?.view,
            };
            let normal = self.map_region(draw.normals.get(id).and_then(Option::as_ref), nk, true, pn, normal_hint)?;
            let surface = self.surfaces.get(id).ok_or("Missing PBR maps")?;
            let uniform = self.rs.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uv material"),
                contents: &floats(&[
                    material.color[0],
                    material.color[1],
                    material.color[2],
                    material.color[3],
                    if nk.3 != 0 { 1.0 } else { 0.0 },
                    surface.flags() as f32,
                    f32::from(draw.masked.contains(&id)),
                    0.0,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = self.rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uv material"),
                layout: &self.material_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(color_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&normal.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&surface.maps[0].view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&surface.maps[1].view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&surface.maps[2].view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&surface.maps[3].view) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&surface.maps[4].view) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.mask_sampler) },
                ],
            });
            let value = Material {
                masked: draw.masked.contains(&id),
                surface_version: self.surfaces.get(id).map_or(0, |s| s.version),
                cutout: !draw.masked.contains(&id) && (color.cutout() || material.color[3] < 1.0),
                color,
                normal,
                group,
                _uniform: uniform,
            };
            if let Some(slot) = self.materials.get_mut(id) {
                *slot = Some(value);
            } else {
                self.materials.push(Some(value));
            }
            self.map_generation = self.map_generation.wrapping_add(1);
        }
        self.regions.clear();
        Ok(())
    }
    pub fn draw(&mut self, draw: Draw<'_>) -> Result<egui::TextureId, String> {
        if !self.health.is_ok() {
            return Err(self.health.fault().map_or_else(|| "GPU unavailable".into(), |f| f.to_string()));
        }
        self.mesh(draw.geometry.clone())?;
        self.materials(&draw)?;
        let pbr_active = self.lighting.pbr
            && draw.camera.shaded
            && draw.camera.channel == photocraft_uv::preview::Channel::Material
            && draw.scene.materials.iter().enumerate().any(|(id, _)| !draw.masked.contains(&id));
        let samples = if draw.antialias { self.aa_samples } else { 1 };
        let symmetry_plane = draw.symmetry_plane.filter(valid_symmetry_plane);
        let symmetry_key = symmetry_plane.map(|corners| std::array::from_fn(|index| corners[index / 3][index % 3].to_bits()));
        let key = (
            draw.generation,
            draw.camera,
            draw.size,
            draw.antialias,
            self.map_generation,
            self.wireframe_enabled,
            self.wireframe_color,
            self.wireframe_opacity.to_bits(),
            symmetry_key,
        );
        if self.last == Some(key) && self.mesh.as_ref().is_some_and(|mesh| &mesh.selected == draw.selected) {
            self.last_pbr_active = pbr_active;
            return self.texture_id.ok_or_else(|| "GPU viewport texture unavailable".into());
        }
        let device = &self.rs.device;
        if draw.size.contains(&0)
            || draw.size.iter().any(|s| *s > device.limits().max_texture_dimension_2d)
            || u64::from(draw.size[0]) * u64::from(draw.size[1]) > 16_777_216
        {
            return Err("GPU viewport exceeds texture budget".into());
        }
        if self.target.as_ref().is_none_or(|t| t.size != draw.size) && self.spare_target.as_ref().is_some_and(|t| t.size == draw.size) {
            std::mem::swap(&mut self.target, &mut self.spare_target);
            if let (Some(id), Some(target)) = (self.texture_id, &self.target) {
                self.rs.renderer.write().update_egui_texture_from_wgpu_texture(device, &target.view, wgpu::FilterMode::Linear, id);
            }
        }
        if self.target.as_ref().is_none_or(|t| t.size != draw.size) {
            self.spare_target = self.target.take();
            let texture = texture(
                device,
                draw.size,
                FORMAT,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                "uv viewport",
            );
            let view = texture.create_view(&Default::default());
            let depth_tex = super_texture(device, draw.size, DEPTH, 1, "uv depth");
            let depth = depth_tex.create_view(&Default::default());
            let mut renderer = self.rs.renderer.write();
            if let Some(id) = self.texture_id {
                renderer.update_egui_texture_from_wgpu_texture(device, &view, wgpu::FilterMode::Linear, id);
            } else {
                self.texture_id = Some(renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear));
            }
            self.target = Some(Target {
                _texture: texture,
                view,
                _depth: depth_tex,
                depth,
                _depth_msaa: None,
                depth_msaa: None,
                _msaa: None,
                msaa: None,
                size: draw.size,
                samples,
            });
        }
        if let Some(target) = &mut self.target {
            if samples > 1 && target.msaa.is_none() {
                let color = super_texture(device, draw.size, FORMAT, samples, "uv cached MSAA");
                let depth = super_texture(device, draw.size, DEPTH, samples, "uv cached MSAA depth");
                target.msaa = Some(color.create_view(&Default::default()));
                target._msaa = Some(color);
                target.depth_msaa = Some(depth.create_view(&Default::default()));
                target._depth_msaa = Some(depth);
            }
            target.samples = samples;
        }
        let target = self.target.as_ref().ok_or("Missing GPU viewport")?;
        let mesh = self.mesh.as_mut().ok_or("Missing GPU mesh")?;
        if &mesh.selected != draw.selected {
            let count = (mesh.selection.size() / 4) as usize;
            let mut data = vec![0u8; count * 4];
            for (face_id, face) in mesh.geometry.triangles.iter().enumerate() {
                let selected_shell = draw.selected.contains(&(face.shell as usize));
                let selected_face = draw.selected.contains(&selected_face_key(face_id));
                if (selected_shell || selected_face)
                    && let Some(bytes) = data.get_mut(face_id.saturating_mul(4)..face_id.saturating_mul(4).saturating_add(4))
                {
                    bytes.copy_from_slice(&1u32.to_le_bytes());
                }
            }
            self.rs.queue.write_buffer(&mesh.selection, 0, &data);
            mesh.selected = draw.selected.clone();
        }
        let mut values = draw.geometry.camera_values(draw.camera, draw.size).to_vec();
        let s = self.lighting;
        values.extend_from_slice(&[
            f32::from(s.pbr),
            s.brightness,
            s.rotation.to_radians(),
            f32::from(s.background),
            s.roughness,
            s.metallic,
            s.emissive,
            s.background_brightness,
        ]);
        let [r, g, b, a] = self.wireframe_color;
        values.extend_from_slice(&[f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, (f32::from(a) / 255.0) * self.wireframe_opacity]);
        values.extend_from_slice(&[f32::from(u8::from(draw.camera.cull_backfaces)), 0.0, 0.0, 0.0]);
        self.rs.queue.write_buffer(&self.camera, 0, &floats(&values));
        let symmetry_buffers = symmetry_plane.map(|corners| {
            let fill = [corners[0], corners[1], corners[2], corners[2], corners[1], corners[3]];
            let outline = [corners[0], corners[1], corners[1], corners[3], corners[3], corners[2], corners[2], corners[0]];
            let fill_values: Vec<f32> = fill.into_iter().flatten().collect();
            let outline_values: Vec<f32> = outline.into_iter().flatten().collect();
            let fill_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uv symmetry plane fill vertices"),
                contents: &floats(&fill_values),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let outline_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uv symmetry plane outline vertices"),
                contents: &floats(&outline_values),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (fill_buffer, outline_buffer)
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("uv model") });
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.collect();
            timer.start(&mut encoder, u64::from(draw.size[0]) * u64::from(draw.size[1]));
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("uv model"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if samples > 1 { target.msaa.as_ref().unwrap_or(&target.view) } else { &target.view },
                    resolve_target: (samples > 1).then_some(&target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 35.0 / 255.0, g: 35.0 / 255.0, b: 35.0 / 255.0, a: 1.0 }),
                        store: if samples > 1 { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: if samples > 1 { target.depth_msaa.as_ref().unwrap_or(&target.depth) } else { &target.depth },
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &mesh.group, &[]);
            pass.set_bind_group(2, &self.environment.group, &[]);
            if self.lighting.background
                && draw.camera.channel == photocraft_uv::preview::Channel::Material
                && let (Some(pipeline), Some(material)) = (self.background_pipelines.get(usize::from(samples > 1)), self.materials.iter().flatten().next())
            {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, &material.group, &[]);
                pass.draw(0..3, 0..1);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            for batch in &mesh.batches {
                if !draw.hidden.contains(&batch.material)
                    && let Some(m) = self.materials.get(batch.material).and_then(Option::as_ref)
                {
                    let cull = if !draw.camera.cull_backfaces {
                        0
                    } else if draw.camera.flip_faces {
                        2
                    } else {
                        1
                    };
                    let index = usize::from(samples > 1) * 6 + cull * 2 + usize::from(m.cutout);
                    let Some(pipeline) = self.pipelines.get(index) else { continue };
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(1, &m.group, &[]);
                    pass.draw(batch.vertices.clone(), 0..1);
                }
            }
            if let (Some((fill_buffer, outline_buffer)), Some(fill_pipeline), Some(outline_pipeline)) =
                (&symmetry_buffers, self.symmetry_fill_pipelines.get(usize::from(samples > 1)), self.symmetry_outline_pipelines.get(usize::from(samples > 1)))
            {
                pass.set_pipeline(fill_pipeline);
                pass.set_vertex_buffer(0, fill_buffer.slice(..));
                pass.draw(0..6, 0..1);
                pass.set_pipeline(outline_pipeline);
                pass.set_vertex_buffer(0, outline_buffer.slice(..));
                pass.draw(0..8, 0..1);
            }
            if self.wireframe_enabled
                && let Some(pipeline) = self.wireframe_pipelines.get(usize::from(samples > 1))
            {
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, mesh.wire_vertices.slice(..));
                for batch in &mesh.wire_batches {
                    if !draw.hidden.contains(&batch.material) {
                        pass.draw(batch.vertices.clone(), 0..1);
                    }
                }
            }
        }
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.finish(&mut encoder);
        }
        self.rs.queue.submit([encoder.finish()]);
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.submitted();
        }
        self.last = Some(key);
        self.last_pbr_active = pbr_active;
        self.texture_id.ok_or_else(|| "Missing GPU texture id".into())
    }
}

/// Encodes a selected face in the existing selection set without colliding with
/// normal UV shell IDs (which are bounded by the imported mesh size).
pub(crate) fn selected_face_key(face_id: usize) -> usize {
    usize::MAX - face_id
}
/// Smallest texel-aligned rectangle containing changed half-RGBA texels.
fn changed_region(old: &[u8], new: &[u8], size: [u32; 2], stride: usize) -> Option<[u32; 4]> {
    if old.len() != new.len() {
        return Some([0, 0, size[0], size[1]]);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (size[0], size[1], 0, 0);
    for (id, (a, b)) in old.chunks_exact(stride.max(1)).zip(new.chunks_exact(stride.max(1))).enumerate() {
        if a != b {
            let (x, y) = (id as u32 % size[0].max(1), id as u32 / size[0].max(1));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0 < x1 && y0 < y1).then(|| [x0, y0, x1 - x0, y1 - y0])
}
fn super_texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, label: &str) -> wgpu::Texture {
    texture(device, size, format, samples, wgpu::TextureUsages::RENDER_ATTACHMENT, label)
}
impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(id) = self.texture_id {
            self.rs.renderer.write().free_texture(&id);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)] // Assertions in test-only GPU setup/readback helpers.
mod tests {
    use super::*;
    use photocraft_uv::{
        import,
        preview::{self, Materials},
        topology::Topology,
    };
    fn renderer() -> Renderer {
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let renderer = Renderer::new(rs, health).unwrap();
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
        assert!(renderer.compositor.is_some(), "map compositor errors must not masquerade as a healthy renderer");
        renderer
    }
    #[test]
    fn gray_mask_live_key_changes_keep_output_texture_identity_stable() {
        let before = gray_mask_source_identity((56, 1, 0, 0), 12, 34, 78);
        let during = gray_mask_source_identity((56, 2, 0, 0), 12, 34, 78);
        assert_ne!(before, during);
        assert_eq!(gray_mask_map_key(9, [64, 64]), gray_mask_map_key(9, [64, 64]));
        assert_ne!(gray_mask_map_key(9, [64, 64]), gray_mask_map_key(10, [64, 64]));
    }

    #[test]
    fn gray_mask_brush_override_matches_full_stack_render_after_live_key_changes() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let size = [32, 32];
        let mut document = photocraft_doc::Document::with_background(
            "Gray mask brush preview",
            photocraft_geom::Size::new(size[0], size[1]),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::BLACK,
        );
        let base = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.0]);
        let paint_surface = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::RGBA8, &[0.0; 4]);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.paint = Some(photocraft_doc::mask_paint::Stack {
            base: base.clone(),
            layers: vec![photocraft_doc::mask_paint::Paint {
                name: "Paint".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: paint_surface,
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: photocraft_doc::mask_paint::EffectKind::Paint,
            }],
            active: 0,
        });
        photocraft_compose::mask_paint::refresh_in_canvas(&mut mask, document.bounds());
        let mut masked_layer = photocraft_doc::Layer::raster("Masked", photocraft_color::PixelFormat::RGBA8);
        let layer_id = masked_layer.id;
        masked_layer.mask = Some(mask);
        document.layers.push(masked_layer);
        let document = Arc::new(document);

        let pending_dabs = Arc::new(Mutex::new(VecDeque::from([photocraft_gpu::MaskRoundDab { center: [16.0, 16.0], radius: 5.0, alpha: 1.0, opacity: 1.0 }])));
        let failed = Arc::new(AtomicBool::new(false));
        let patches = Arc::new(Mutex::new(None));
        let release_requested = Arc::new(AtomicBool::new(false));
        let source = |key, begin, document: Arc<photocraft_doc::Document>| GrayMaskGpuSource {
            document,
            layer: layer_id,
            shell_ids: Arc::new(vec![1; (size[0] * size[1]) as usize]),
            owner_key: 100,
            shell_key: 200,
            materials: BTreeSet::new(),
            key: (1, key, 0, 0),
            brush_preview: Some(GrayMaskGpuBrushInput {
                stroke_key: 300,
                begin,
                pending_dabs: Arc::clone(&pending_dabs),
                release_requested: Arc::clone(&release_requested),
                failed: Arc::clone(&failed),
                patches: Arc::clone(&patches),
            }),
        };
        let settings = photocraft_gpu::MaskBrushSettings { hardness: 0.8, stroke_opacity: 1.0, color: [1.0; 4] };
        renderer.set_gray_mask_source(Some(&source(
            1,
            Some((photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::RGBA8, &[0.0; 4]), settings)),
            Arc::clone(&document),
        )));
        assert!(!failed.load(Ordering::Acquire));
        let stable_map = Arc::clone(renderer.gray_mask_map.as_ref().unwrap());

        pending_dabs.lock().unwrap().push_back(photocraft_gpu::MaskRoundDab { center: [19.0, 16.0], radius: 4.0, alpha: 0.7, opacity: 0.8 });
        renderer.set_gray_mask_source(Some(&source(2, None, Arc::clone(&document))));
        assert!(!failed.load(Ordering::Acquire));
        assert!(Arc::ptr_eq(&stable_map, renderer.gray_mask_map.as_ref().unwrap()), "live dabs should update the stable material texture");
        assert!(pending_dabs.lock().unwrap().is_empty(), "only successfully submitted Dabs are drained");
        let preview_pixels = read_rgba16f(&renderer, &stable_map.texture, 0);

        let final_patches = renderer.read_gray_mask_brush_patches().unwrap();
        assert!(!final_patches.is_empty());
        let mut expected_stack = document.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().clone();
        for patch in final_patches {
            let pixels: Vec<f32> = patch.pixels_rgba8.iter().map(|byte| f32::from(*byte) / 255.0).collect();
            expected_stack.layers[0].surface.write_region(patch.rect, &pixels);
        }
        let expected_texture = texture(
            &renderer.rs.device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            "gray mask fresh stack parity",
        );
        let mut fresh = photocraft_gpu::MaskStackRenderer::new(&renderer.rs.device);
        fresh.render(&renderer.rs.device, &renderer.rs.queue, &expected_stack, size, &vec![1; (size[0] * size[1]) as usize], &expected_texture).unwrap();
        let full_pixels = read_rgba16f(&renderer, &expected_texture, 0);
        assert_eq!(preview_pixels.len(), full_pixels.len());
        for (index, (preview, full)) in preview_pixels.iter().zip(&full_pixels).enumerate() {
            for channel in 0..4 {
                assert!(
                    (preview[channel] - full[channel]).abs() < 0.002,
                    "pixel {index}, channel {channel}: preview={} full={}",
                    preview[channel],
                    full[channel]
                );
            }
        }
    }
    #[test]
    fn symmetry_plane_rejects_nonfinite_and_extreme_prepared_coordinates() {
        let valid = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        assert!(valid_symmetry_plane(&valid));

        let mut nonfinite = valid;
        nonfinite[2][1] = f32::INFINITY;
        assert!(!valid_symmetry_plane(&nonfinite));

        let mut excessive = valid;
        excessive[3][2] = 10_001.0;
        assert!(!valid_symmetry_plane(&excessive));
    }
    #[test]
    fn symmetry_plane_gpu_pass_renders_and_invalidates_frame_cache() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let axis_bounds = geometry.bounds[0];
        let settings = crate::uv_symmetry::Settings {
            enabled: true,
            axis: crate::uv_symmetry::Axis::X,
            offset: (axis_bounds[0] + axis_bounds[1]) * 0.5,
            ..Default::default()
        };
        let plane = crate::uv_symmetry::plane_corners(&geometry, settings).expect("midpoint plane intersects model bounds");
        let empty = BTreeSet::new();
        let colors = vec![None; scene.materials.len()];
        let draw = |symmetry_plane| Draw {
            scene: &scene,
            geometry: geometry.clone(),
            camera: Camera::default(),
            size: [32, 32],
            antialias: false,
            selected: &empty,
            hidden: &empty,
            generation: 1,
            colors: &colors,
            normals: &[],
            keys: &[],
            normal_keys: &[],
            masked: &empty,
            symmetry_plane,
        };

        renderer.draw(draw(Some(plane))).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert!(renderer.last.is_some_and(|key| key.8.is_some()));

        renderer.draw(draw(None)).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert!(renderer.last.is_some_and(|key| key.8.is_none()));
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }
    #[cfg(target_os = "windows")]
    fn dx12_fxc_render_state() -> Result<RenderState, String> {
        let mut setup = crate::gpu_canvas::wgpu_setup();
        let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup else { return Err("native GPU setup must create a device".into()) };
        create.instance_descriptor.backends = wgpu::Backends::DX12;
        create.instance_descriptor.flags = wgpu::InstanceFlags::empty();
        create.instance_descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
        Ok(egui_kittest::wgpu::create_render_state(setup, Default::default()))
    }
    #[test]
    #[cfg(target_os = "windows")]
    fn dx12_fxc_renderer_initializes_for_production_backend() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = dx12_fxc_render_state().unwrap();
        assert_eq!(rs.adapter.get_info().backend, wgpu::Backend::Dx12);
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let renderer = Renderer::new(rs, health).unwrap();
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
        assert!(renderer.compositor.is_some(), "DX12/FXC map compositor unavailable: {:?}", renderer.compositor_error);
    }
    #[test]
    #[cfg(target_os = "windows")]
    fn dx12_fxc_map_compositor_renders_for_production_backend() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = dx12_fxc_render_state().unwrap();
        assert_eq!(rs.adapter.get_info().backend, wgpu::Backend::Dx12);
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let mut renderer = Renderer::new(rs, health).unwrap();
        assert!(renderer.compositor.is_some(), "DX12/FXC failed to create the map compositor: {:?}", renderer.compositor_error);
        let doc = photocraft_doc::Document::with_background(
            "DX12 FXC map compositor",
            photocraft_geom::Size::new(4, 4),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgb(0.2, 0.4, 0.6),
        );
        let map = renderer.map_gpu_composite(&doc, (doc.id.0, 1, 0, 0), false, None, None).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert_eq!(map.size, [4, 4]);
        assert_eq!(map.source, MapSource::GpuComposite);
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }
    #[test]
    #[cfg(target_os = "windows")]
    fn missing_map_compositor_keeps_dx12_orbit_and_pbr_cpu_map_fallback() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = dx12_fxc_render_state().unwrap();
        assert_eq!(rs.adapter.get_info().backend, wgpu::Backend::Dx12);
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let mut renderer = Renderer::new_with_map_compositor(rs, health, false).unwrap();
        assert!(renderer.compositor.is_none());

        let doc = Arc::new(photocraft_doc::Document::with_background(
            "PBR CPU fallback map",
            photocraft_geom::Size::new(16, 8),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgb(0.7, 0.2, 0.1),
        ));
        let (scene, geometry) = scene();
        let mut maps = crate::uv_lighting::SurfaceMaps::default();
        maps.gpu[0] = vec![
            Some(crate::uv_lighting::GpuMapRequest { doc, key: (1, 1, 0, 1), linear_srgb: true, previous_key: None, damage: None });
            scene.materials.len()
        ];
        maps.use_color_for_pbr = vec![true; scene.materials.len()];
        let settings = photocraft_uv::lighting::Settings { pbr: true, ..Default::default() };
        renderer.set_lighting(settings, &maps, Arc::new(photocraft_uv::lighting::Environment::studio())).unwrap();
        assert_eq!(renderer.surfaces[0].maps[0].source, MapSource::CpuFallback);
        assert_eq!(renderer.surfaces[0].maps[0].size, [16, 8]);

        let empty = BTreeSet::new();
        let colors = vec![None; scene.materials.len()];
        renderer
            .draw(Draw {
                scene: &scene,
                geometry,
                camera: Camera { channel: photocraft_uv::preview::Channel::Material, shaded: true, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &empty,
                symmetry_plane: None,
            })
            .unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert!(renderer.lighting.pbr, "PBR lighting must stay active during map-compositor fallback");
        assert!(renderer.last_pbr_active, "PBR must be active in the shaded Material view");
        assert!(matches!(renderer.materials[0].as_ref().unwrap().color, MaterialColor::Surface { .. }));
        let inspected = renderer.diagnostics(photocraft_uv::preview::Channel::Material);
        assert_eq!(inspected["rendererActive"], true);
        assert_eq!(inspected["pbrEnabled"], true);
        assert_eq!(inspected["pbrActive"], true);
        assert_eq!(inspected["renderTargetSize"], serde_json::json!([128, 128]));
        assert_eq!(inspected["maps"][0]["maps"][0]["source"], "cpu-fallback");
        assert_eq!(inspected["maps"][0]["maps"][0]["size"], serde_json::json!([16, 8]));
        assert!(inspected.get("pixels").is_none() && inspected.get("paths").is_none());
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }
    #[test]
    #[cfg(target_os = "windows")]
    fn dx12_fxc_keeps_full_resolution_maps_for_multiple_materials() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = dx12_fxc_render_state().unwrap();
        assert_eq!(rs.adapter.get_info().backend, wgpu::Backend::Dx12);
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let mut renderer = Renderer::new(rs, health).unwrap();
        assert!(renderer.compositor.is_some(), "DX12/FXC map compositor unavailable: {:?}", renderer.compositor_error);

        let doc = Arc::new(photocraft_doc::Document::with_background(
            "Full-resolution PBR source",
            photocraft_geom::Size::new(4097, 37),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_doc::Color::rgb(0.65, 0.35, 0.12),
        ));
        let (mut scene, geometry) = scene();
        scene.materials.push(scene.materials[0].clone());
        let mut maps = crate::uv_lighting::SurfaceMaps::default();
        maps.gpu[0] = vec![
            Some(crate::uv_lighting::GpuMapRequest { doc: doc.clone(), key: (doc.id.0, 1, 0, 1), linear_srgb: true, previous_key: None, damage: None }),
            Some(crate::uv_lighting::GpuMapRequest { doc: doc.clone(), key: (doc.id.0, 1, 0, 1), linear_srgb: true, previous_key: None, damage: None }),
        ];
        maps.use_color_for_pbr = vec![true; 2];
        let settings = photocraft_uv::lighting::Settings { pbr: true, ..Default::default() };
        renderer.set_lighting(settings, &maps, Arc::new(photocraft_uv::lighting::Environment::studio())).unwrap();
        for surface in renderer.surfaces.iter().take(2) {
            assert_eq!(surface.maps[0].size, [4097, 37]);
            assert_eq!(surface.maps[0].source, MapSource::GpuComposite);
        }
        let empty = BTreeSet::new();
        let colors = vec![None; scene.materials.len()];
        renderer
            .draw(Draw {
                scene: &scene,
                geometry,
                camera: Camera { channel: photocraft_uv::preview::Channel::Material, shaded: true, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &empty,
                symmetry_plane: None,
            })
            .unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert!(renderer.last_pbr_active);
        for material in renderer.materials.iter().take(2).flatten() {
            assert_eq!(material.color.size(), [4097, 37]);
            assert!(matches!(material.color, MaterialColor::Surface { .. }));
        }
    }
    fn opacity_bench_app(layer_count: usize) -> (crate::PhotocraftApp, photocraft_doc::LayerId) {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width":4096,"height":4096,"depth":16})).unwrap();
        session.uv.scene = Some(Arc::new(Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut state.doc);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        let gray = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U16, false);
        let rgba = photocraft_color::PixelFormat::RGBA16;
        let tile_size = photocraft_geom::TILE_SIZE;
        let mut base = photocraft_raster::Surface::with_default(gray, &[0.18]);
        for y in (0..4096).step_by(tile_size as usize) {
            for x in (0..4096).step_by(tile_size as usize) {
                let mut pixels = Vec::with_capacity(tile_size as usize * tile_size as usize);
                for py in 0..tile_size {
                    for px in 0..tile_size {
                        pixels.push(0.12 + (((x + px) * 3 + (y + py) * 5).rem_euclid(701) as f32) / 1400.0);
                    }
                }
                base.write_region(photocraft_geom::Rect::new(x, y, x + tile_size, y + tile_size), &pixels);
            }
        }
        let layers = (0..layer_count)
            .map(|index| {
                let mut surface = photocraft_raster::Surface::with_default(rgba, &[0.75, 0.2, 0.05, 0.28 + index as f32 * 0.01]);
                for y in (0..4096).step_by(tile_size as usize) {
                    for x in (0..4096).step_by(tile_size as usize) {
                        let mut pixels = Vec::with_capacity(tile_size as usize * tile_size as usize * 4);
                        for py in 0..tile_size {
                            for px in 0..tile_size {
                                let pattern = ((x + px) * 7 + (y + py) * 11 + index as i32 * 29).rem_euclid(257) as f32 / 257.0;
                                pixels.extend_from_slice(&[0.18 + pattern * 0.55, 0.08 + pattern * 0.22, 0.04 + pattern * 0.12, 0.24 + pattern * 0.52]);
                            }
                        }
                        surface.write_region(photocraft_geom::Rect::new(x, y, x + tile_size, y + tile_size), &pixels);
                    }
                }
                photocraft_doc::mask_paint::Paint {
                    name: format!("Paint {index}"),
                    visible: true,
                    opacity: 0.75,
                    blend: photocraft_color::BlendMode::Normal,
                    surface,
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Paint,
                }
            })
            .collect();
        mask.paint = Some(photocraft_doc::mask_paint::Stack { base, layers, active: layer_count.saturating_sub(1) });
        photocraft_compose::mask_paint::refresh(&mut mask, None);
        doc.layer_mut(layer_id).unwrap().mask = Some(mask);
        (crate::PhotocraftApp::new(session, Default::default()), layer_id)
    }
    fn median_p95(times: &mut [f64]) -> (f64, f64) {
        times.sort_by(f64::total_cmp);
        (times[times.len() / 2], times[times.len() * 95 / 100])
    }
    #[test]
    #[ignore = "explicit 4K mask Paint opacity CPU/GPU preview benchmark; waits for completed submissions"]
    fn benchmark_native_mask_paint_opacity_preview_4k() {
        let _gpu = photocraft_testkit::gpu_lock();
        const FRAMES: usize = 24;
        for layer_count in [1, 10] {
            let changed_indices: &[usize] = if layer_count == 1 { &[0] } else { &[0, layer_count - 1] };
            for &changed_index in changed_indices {
                let (mut canonical, layer) = opacity_bench_app(layer_count);
                let initial = &canonical.session.active().unwrap().doc;
                std::hint::black_box(photocraft_compose::render(initial, initial.bounds()));
                let mut cpu_times = Vec::with_capacity(FRAMES);
                for frame in 0..FRAMES {
                    let opacity = (frame + 1) as f32 / (FRAMES + 1) as f32;
                    let start = std::time::Instant::now();
                    canonical
                        .run(
                            "uv.maskPaint.modify",
                            serde_json::json!({"layer":layer.0,"index":changed_index,"opacity":opacity,"coalesce":"mask-opacity-bench"}),
                        )
                        .unwrap();
                    let doc = &canonical.session.active().unwrap().doc;
                    let pixels = photocraft_compose::render(doc, doc.bounds());
                    std::hint::black_box(pixels);
                    cpu_times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                let (cpu_median, cpu_p95) = median_p95(&mut cpu_times);

                let (mut preview, layer) = opacity_bench_app(layer_count);
                let mut renderer = renderer();
                let initial = preview.session.active().unwrap().doc.clone();
                let initial_key = (initial.id.0, preview.session.active().unwrap().revision, 0, 0);
                let cold_start = std::time::Instant::now();
                let mut active_map = Some(renderer.map_gpu_composite(&initial, initial_key, true, None, None).unwrap());
                let cold_submit_ms = cold_start.elapsed().as_secs_f64() * 1000.0;
                let cold_wait = std::time::Instant::now();
                renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                let cold_complete_ms = cold_wait.elapsed().as_secs_f64() * 1000.0;

                // Warm the transient-opacity plan and allocate the second texture used by the
                // production ping-pong map cache. Cold residency remains visible above, outside
                // the hot drag distributions below.
                crate::uv_mask_paint_preview::set(&mut preview, layer, changed_index, 0.5);
                let (warm_doc, warm_key) = crate::uv_mask_paint_preview::display_doc(&mut preview, 0).unwrap();
                let mut spare_map = Some(renderer.map_gpu_composite(&warm_doc, (warm_doc.id.0, 1, warm_key, 0), true, None, None).unwrap());
                renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                let mut prep_times = Vec::with_capacity(FRAMES);
                let mut submit_times = Vec::with_capacity(FRAMES);
                let mut complete_times = Vec::with_capacity(FRAMES);
                let mut gpu_ms = Vec::new();
                for frame in 0..FRAMES {
                    let opacity = (frame + 1) as f32 / (FRAMES + 1) as f32;
                    let start = std::time::Instant::now();
                    crate::uv_mask_paint_preview::set(&mut preview, layer, changed_index, opacity);
                    let (doc, key) = crate::uv_mask_paint_preview::display_doc(&mut preview, 0).unwrap();
                    prep_times.push(start.elapsed().as_secs_f64() * 1000.0);

                    let start = std::time::Instant::now();
                    let map = renderer.map_gpu_composite(&doc, (doc.id.0, 1, key, 0), true, None, spare_map.take()).unwrap();
                    submit_times.push(start.elapsed().as_secs_f64() * 1000.0);
                    spare_map = active_map.replace(map);
                    let stats = renderer.take_map_stats().unwrap();
                    if frame > 0
                        && let Some(timing) = stats.gpu_time
                    {
                        gpu_ms.push(timing.ms);
                    }
                    let start = std::time::Instant::now();
                    renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                    complete_times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                // Drain the last asynchronous timestamp without including this duplicate render
                // in the per-value submit and completion distributions.
                let (final_doc, final_key) = crate::uv_mask_paint_preview::display_doc(&mut preview, 0).unwrap();
                let final_map = renderer.map_gpu_composite(&final_doc, (final_doc.id.0, 1, final_key, 0), true, None, spare_map.take()).unwrap();
                let _ = active_map.replace(final_map);
                renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                if let Some(timing) = renderer.take_map_stats().and_then(|stats| stats.gpu_time) {
                    gpu_ms.push(timing.ms);
                }
                let commit = crate::uv_mask_paint_preview::pending(&preview).unwrap();
                let start = std::time::Instant::now();
                preview.run("uv.maskPaint.modify", commit.params()).unwrap();
                let commit_ms = start.elapsed().as_secs_f64() * 1000.0;
                let (prep_median, prep_p95) = median_p95(&mut prep_times);
                let (submit_median, submit_p95) = median_p95(&mut submit_times);
                let (complete_median, complete_p95) = median_p95(&mut complete_times);
                let (gpu_median, gpu_p95) = if gpu_ms.is_empty() { (f64::NAN, f64::NAN) } else { median_p95(&mut gpu_ms) };
                println!(
                    "4K U16 dense opacity layers={layer_count} index={changed_index}: cold GPU map encode+submit={cold_submit_ms:.2}ms, completion={cold_complete_ms:.2}ms; hot canonical command+CPU render median={cpu_median:.2}ms p95={cpu_p95:.2}ms; preview prep={prep_median:.2}/{prep_p95:.2}ms, GPU map encode+submit={submit_median:.2}/{submit_p95:.2}ms, completion wait={complete_median:.2}/{complete_p95:.2}ms, async GPU composite={gpu_median:.2}/{gpu_p95:.2}ms, final commit={commit_ms:.2}ms ({} profile)\n",
                    if cfg!(debug_assertions) { "test" } else { "release" }
                );
                assert_eq!(
                    preview.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[changed_index].opacity,
                    commit.opacity
                );
            }
        }
    }
    #[test]
    fn gpu_compositor_writes_pbr_map_without_a_cpu_mirror() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let mut doc = photocraft_doc::Document::with_background(
            "GPU map sink",
            photocraft_geom::Size::new(1031, 1037),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgb(0.5, 0.25, 0.75),
        );
        let bounds = photocraft_geom::Rect::new(1023, 1021, 1031, 1037);
        let format = doc.pixel_format();
        let pixel = photocraft_raster::from_rgba(&format, [0.9, 0.1, 0.3, 0.4]);
        doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &pixel);
        let key = (doc.id.0, 1, 0, 7);
        let map = renderer.map_gpu_composite(&doc, key, true, None, None).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert_eq!(map.source, MapSource::GpuComposite);
        assert_eq!(map.key, key);
        assert_eq!(map.size, [1031, 1037]);
        assert!(map.data.is_none());
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
        let pixels = read_rgba16f(&renderer, &map.texture, 0);
        let sample = |x: usize, y: usize| pixels[y * 1031 + x];
        let inside = sample(1025, 1032);
        let before_edge = sample(1022, 1032);
        let after_edge = sample(1030, 1020);
        assert!((inside[0] - 0.79).abs() < 0.015 && (inside[1] - 0.01).abs() < 0.004 && (inside[2] - 0.074).abs() < 0.008);
        assert!((inside[3] - 0.4).abs() < 0.01, "alpha is preserved: {inside:?}");
        assert!((before_edge[0] - 0.214).abs() < 0.015 && (before_edge[3] - 1.0).abs() < 0.01);
        assert!((after_edge[0] - 0.214).abs() < 0.015, "right chunk edge is initialized: {after_edge:?}");
        assert!(map.levels.len() > 1);
        let mip = read_rgba16f(&renderer, &map.texture, 1);
        let mip_pixel = mip[(1032 / 2) * 515 + 1024 / 2];
        assert!((mip_pixel[0] - inside[0]).abs() < 0.02 && (mip_pixel[3] - inside[3]).abs() < 0.02, "PBR mip keeps linear color and alpha: {mip_pixel:?}");
    }
    #[test]
    fn gpu_compositor_partial_damage_matches_full_refresh_and_mips() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let initial = photocraft_doc::Document::with_background(
            "GPU partial map",
            photocraft_geom::Size::new(1025, 1027),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgb(0.08, 0.12, 0.2),
        );
        let initial_key = (initial.id.0, 1, 0, 7);
        let previous = renderer.map_gpu_composite(&initial, initial_key, true, None, None).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();

        // Simulate several missed refresh revisions: both sparse edits must be covered by the
        // conservative union rectangle passed to the map compositor.
        let mut updated = initial.clone();
        let bounds_a = photocraft_geom::Rect::new(7, 9, 19, 23);
        let bounds_b = photocraft_geom::Rect::new(850, 861, 901, 908);
        let format = updated.pixel_format();
        let a = photocraft_raster::from_rgba(&format, [0.9, 0.08, 0.15, 1.0]);
        let b = photocraft_raster::from_rgba(&format, [0.05, 0.85, 0.2, 1.0]);
        updated.layers[0].surface_mut().unwrap().fill_rect(bounds_a, &a);
        updated.layers[0].surface_mut().unwrap().fill_rect(bounds_b, &b);
        let union = photocraft_geom::Rect::new(bounds_a.x0, bounds_a.y0, bounds_b.x1, bounds_b.y1);
        let next_key = (updated.id.0, 2, 3, 7);
        let partial = renderer.map_gpu_composite(&updated, next_key, true, Some(union), Some(previous)).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        let partial_stats = renderer.last_map_stats.expect("partial refresh stats");
        let full = renderer.map_gpu_composite(&updated, next_key, true, None, None).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        let full_stats = renderer.last_map_stats.expect("full refresh stats");

        assert_eq!(partial.key, next_key);
        assert_eq!(partial.source, MapSource::GpuComposite);
        assert_eq!(partial.levels.len(), full.levels.len());
        for mip in 0..partial.levels.len() as u32 {
            let partial_pixels = read_rgba16f(&renderer, &partial.texture, mip);
            let full_pixels = read_rgba16f(&renderer, &full.texture, mip);
            assert_eq!(partial_pixels.len(), full_pixels.len());
            for (index, (actual, expected)) in partial_pixels.iter().zip(&full_pixels).enumerate() {
                for channel in 0..4 {
                    assert!(
                        (actual[channel] - expected[channel]).abs() < 0.002,
                        "partial/full mismatch at mip {mip} pixel {index} channel {channel}: {} vs {}",
                        actual[channel],
                        expected[channel]
                    );
                }
            }
        }
        assert!(partial_stats.chunks < full_stats.chunks, "partial update should visit fewer chunks: {partial_stats:?} vs {full_stats:?}");
    }
    #[test]
    fn pbr_material_binds_the_gpu_composited_surface_map() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let doc = Arc::new(photocraft_doc::Document::with_background(
            "GPU PBR source",
            photocraft_geom::Size::new(16, 8),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgb(0.7, 0.2, 0.1),
        ));
        let (scene, geometry) = scene();
        let mut maps = crate::uv_lighting::SurfaceMaps::default();
        maps.gpu[0] =
            vec![
                Some(crate::uv_lighting::GpuMapRequest { doc: doc.clone(), key: (doc.id.0, 1, 0, 7), linear_srgb: true, previous_key: None, damage: None });
                scene.materials.len()
            ];
        maps.use_color_for_pbr = vec![true; scene.materials.len()];
        let settings = photocraft_uv::lighting::Settings { pbr: true, ..Default::default() };
        renderer.set_lighting(settings, &maps, Arc::new(photocraft_uv::lighting::Environment::studio())).unwrap();
        assert_eq!(renderer.surfaces[0].maps[0].source, MapSource::GpuComposite);
        let empty = BTreeSet::new();
        let colors = vec![None; scene.materials.len()];
        renderer
            .draw(Draw {
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera { channel: photocraft_uv::preview::Channel::Material, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &empty,
                symmetry_plane: None,
            })
            .unwrap();
        assert!(matches!(renderer.materials[0].as_ref().unwrap().color, MaterialColor::Surface { .. }));
        let generation = renderer.map_generation;
        renderer
            .draw(Draw {
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera { yaw: 0.3, channel: photocraft_uv::preview::Channel::Material, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 2,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &empty,
                symmetry_plane: None,
            })
            .unwrap();
        assert_eq!(renderer.map_generation, generation, "camera motion must reuse the GPU-backed material binding");
        maps.use_color_for_pbr[0] = false; // A bound opacity map is combined in the display image instead.
        renderer.set_lighting(settings, &maps, Arc::new(photocraft_uv::lighting::Environment::studio())).unwrap();
        renderer
            .draw(Draw {
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera { channel: photocraft_uv::preview::Channel::Material, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 3,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &empty,
                symmetry_plane: None,
            })
            .unwrap();
        assert!(matches!(renderer.materials[0].as_ref().unwrap().color, MaterialColor::Owned(_)), "opacity must not be dropped from the PBR map");
        maps.use_color_for_pbr[0] = true;
        let masked = BTreeSet::from([0usize]);
        renderer
            .draw(Draw {
                scene: &scene,
                geometry,
                camera: Camera { channel: photocraft_uv::preview::Channel::Material, ..Default::default() },
                size: [128, 128],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 3,
                colors: &colors,
                normals: &[],
                keys: &[],
                normal_keys: &[],
                masked: &masked,
                symmetry_plane: None,
            })
            .unwrap();
        assert!(matches!(renderer.materials[0].as_ref().unwrap().color, MaterialColor::Owned(_)), "mask display must use its own display map");
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }
    #[test]
    fn uv_gpu_mask_is_unlit_in_material_and_normal_modes_and_restores_cached_material() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut scene, g) = scene();
        let mut r = renderer();
        let empty = BTreeSet::new();
        let masked = BTreeSet::from([0usize]);
        scene.materials[0].color = [0.2, 0.8, 0.1, 0.0]; // A transparent material must still show its mask.
        let colors = vec![Some(Image { width: 4, height: 4, rgba: vec![[0.25, 0.25, 0.25, 1.0]; 16] }); scene.materials.len()];
        for channel in [photocraft_uv::preview::Channel::Material, photocraft_uv::preview::Channel::Normal] {
            r.draw(Draw {
                scene: &scene,
                geometry: g.clone(),
                camera: Camera { channel, shaded: true, ..Default::default() },
                size: [256, 256],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &[(1, 1, 0, 0)],
                normal_keys: &[],
                masked: &masked,
                symmetry_plane: None,
            })
            .unwrap();
            let px = read(&r);
            assert!(
                px.iter().filter(|p| p[0].abs_diff(64) < 2 && p[1].abs_diff(64) < 2 && p[2].abs_diff(64) < 2).count() > 10,
                "mask is visible, grayscale and unlit"
            );
            assert!(r.materials[0].as_ref().unwrap().masked);
        }
        r.draw(Draw {
            scene: &scene,
            geometry: g,
            camera: Camera::default(),
            size: [256, 256],
            antialias: false,
            selected: &empty,
            hidden: &empty,
            generation: 1,
            colors: &colors,
            normals: &[],
            keys: &[(1, 1, 0, 0)],
            normal_keys: &[],
            masked: &empty,
            symmetry_plane: None,
        })
        .unwrap();
        assert!(!r.materials[0].as_ref().unwrap().masked);
        assert!(r.health.is_ok());
    }

    #[test]
    fn mask_display_clamps_non_tiled_uv_edge_instead_of_sampling_opposite_border() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (scene, mut geometry) = scene();
        for triangle in &mut Arc::get_mut(&mut geometry).unwrap().triangles {
            // A constant UV on the left edge isolates address-mode behavior from mip LOD.
            triangle.uv = [[0.0, 0.5]; 3];
        }
        let mut renderer = renderer();
        let mut edge_map = Image { width: 4, height: 4, rgba: vec![[1.0, 1.0, 1.0, 1.0]; 16] };
        for y in 0..4 {
            edge_map.rgba[y * 4] = [0.0, 0.0, 0.0, 1.0];
        }
        let colors = vec![Some(edge_map); scene.materials.len()];
        let keys = vec![(1, 1, 0, 0); scene.materials.len()];
        let masked: BTreeSet<_> = (0..scene.materials.len()).collect();
        let empty = BTreeSet::new();
        let repeat_sampler = renderer.rs.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv test repeat mask sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let edge_sampler = std::mem::replace(&mut renderer.mask_sampler, repeat_sampler);
        renderer
            .draw(Draw {
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera::default(),
                size: [256, 256],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &keys,
                normal_keys: &[],
                masked: &masked,
                symmetry_plane: None,
            })
            .unwrap();
        let wrapped = read(&renderer);
        let gray = wrapped.iter().filter(|pixel| (126..=129).contains(&pixel[0]) && (126..=129).contains(&pixel[1]) && (126..=129).contains(&pixel[2])).count();
        assert!(gray > 10, "Repeat+Linear blends the black opposite border into an edge sample");

        renderer.mask_sampler = edge_sampler;
        renderer.materials.clear();
        renderer
            .draw(Draw {
                scene: &scene,
                geometry,
                camera: Camera::default(),
                size: [256, 256],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 2,
                colors: &colors,
                normals: &[],
                keys: &keys,
                normal_keys: &[],
                masked: &masked,
                symmetry_plane: None,
            })
            .unwrap();
        let clamped = read(&renderer);
        let black = clamped.iter().filter(|pixel| pixel[0] < 8 && pixel[1] < 8 && pixel[2] < 8).count();
        assert!(black > 10, "ClampToEdge keeps the mask's dark edge texel dark without changing its value");
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }

    #[test]
    fn mask_display_has_no_background_bright_seams_at_internal_triangle_edges() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (scene, geometry) = scene();
        let mut renderer = renderer();
        let size = [256, 256];
        let camera = Camera::default();
        let colors = vec![Some(Image { width: 1, height: 1, rgba: vec![[0.0, 0.0, 0.0, 1.0]] }); scene.materials.len()];
        let masked: BTreeSet<_> = (0..scene.materials.len()).collect();
        let empty = BTreeSet::new();
        renderer
            .draw(Draw {
                scene: &scene,
                geometry: geometry.clone(),
                camera,
                size,
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &vec![(1, 1, 0, 0); scene.materials.len()],
                normal_keys: &[],
                masked: &masked,
                symmetry_plane: None,
            })
            .unwrap();
        let actual = read(&renderer);
        let expected = preview::render_materials(
            &scene,
            camera,
            &Materials { colors: &colors, hidden: empty.clone(), normals: &[] },
            &vec![false; geometry.triangles.len()],
            size[0] as usize,
            size[1] as usize,
        )
        .unwrap();

        let mut checked = 0usize;
        let mut bright_seams = Vec::new();
        for y in 1..size[1] as usize - 1 {
            for x in 1..size[0] as usize - 1 {
                let index = y * size[0] as usize + x;
                let Some(triangle) = expected.triangles[index] else { continue };
                let neighbors = [index - 1, index + 1, index - size[0] as usize, index + size[0] as usize];
                let covered_neighbors = neighbors.iter().filter(|neighbor| expected.triangles[**neighbor].is_some()).count();
                let crosses_triangle_edge = neighbors.iter().any(|neighbor| expected.triangles[*neighbor] != Some(triangle));
                if covered_neighbors >= 2 && crosses_triangle_edge {
                    checked += 1;
                    if actual[index][0] > 2 || actual[index][1] > 2 || actual[index][2] > 2 {
                        bright_seams.push((x, y, actual[index]));
                    }
                }
            }
        }
        assert!(checked > 20, "fixture must expose internal triangle boundaries");
        assert!(bright_seams.is_empty(), "uniform black mask must not reveal the cleared viewport along internal triangle seams: {bright_seams:?}");
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }

    #[test]
    fn wireframe_depth_overlay_renders_consistently_over_coplanar_mask_faces() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (scene, geometry) = scene();
        let mut renderer = renderer();
        let size = [256, 256];
        let camera = Camera { cull_backfaces: false, ..Default::default() };
        let colors = vec![Some(Image { width: 1, height: 1, rgba: vec![[0.0, 0.0, 0.0, 1.0]] }); scene.materials.len()];
        let keys = vec![(1, 1, 0, 0); scene.materials.len()];
        let masked: BTreeSet<_> = (0..scene.materials.len()).collect();
        let empty = BTreeSet::new();
        let draw = |generation| Draw {
            scene: &scene,
            geometry: geometry.clone(),
            camera,
            size,
            antialias: false,
            selected: &empty,
            hidden: &empty,
            generation,
            colors: &colors,
            normals: &[],
            keys: &keys,
            normal_keys: &[],
            masked: &masked,
            symmetry_plane: None,
        };

        renderer.set_wireframe(false, [255; 4], 1.0);
        renderer.draw(draw(1)).unwrap();
        let base = read(&renderer);
        let expected = preview::render_materials(
            &scene,
            camera,
            &Materials { colors: &colors, hidden: empty.clone(), normals: &[] },
            &vec![false; geometry.triangles.len()],
            size[0] as usize,
            size[1] as usize,
        )
        .unwrap();

        renderer.set_wireframe(true, [255; 4], 1.0);
        renderer.draw(draw(2)).unwrap();
        let with_wireframe = read(&renderer);
        let mut best_edge = Vec::new();
        for (triangle_id, triangle) in geometry.triangles.iter().enumerate() {
            for [a, b] in [[0, 1], [1, 2], [2, 0]] {
                let Some(from) = geometry.project_normalized(camera, triangle.positions[a].map(f64::from), size) else { continue };
                let Some(to) = geometry.project_normalized(camera, triangle.positions[b].map(f64::from), size) else { continue };
                if from.iter().chain(to.iter()).any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                    continue;
                }
                let length = ((to[0] - from[0]).powi(2) * f64::from(size[0]).powi(2) + (to[1] - from[1]).powi(2) * f64::from(size[1]).powi(2)).sqrt();
                let steps = (length.ceil() as usize).max(2);
                let mut candidates = Vec::new();
                for step in 1..steps {
                    let t = step as f64 / steps as f64;
                    let x = ((from[0] * (1.0 - t) + to[0] * t) * f64::from(size[0])).round() as usize;
                    let y = ((from[1] * (1.0 - t) + to[1] * t) * f64::from(size[1])).round() as usize;
                    if x < 1 || y < 1 || x + 1 >= size[0] as usize || y + 1 >= size[1] as usize {
                        continue;
                    }
                    let index = y * size[0] as usize + x;
                    if expected.triangles[index] == Some(triangle_id) {
                        candidates.push((x, y));
                    }
                }
                if candidates.len() > best_edge.len() {
                    best_edge = candidates;
                }
            }
        }
        assert!(best_edge.len() >= 12, "fixture must expose a visible projected triangle edge");
        let covered = best_edge
            .iter()
            .filter(|(x, y)| {
                let edge_pixel = (*y - 1..=*y + 1).any(|py| {
                    (*x - 1..=*x + 1).any(|px| {
                        let pixel = with_wireframe[py * size[0] as usize + px];
                        pixel[0] > 180 && pixel[1] > 180 && pixel[2] > 180
                    })
                });
                let base_is_dark = base[y * size[0] as usize + x][0] < 8;
                edge_pixel && base_is_dark
            })
            .count();
        assert!(
            covered * 100 >= best_edge.len() * 75,
            "depth-tested wireframe should remain continuous on a coplanar visible edge: {covered}/{} samples",
            best_edge.len()
        );
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }

    #[test]
    fn gpu_composited_pbr_map_crops_blurred_mask_chunks_to_their_canvas_origin() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let size = photocraft_geom::Size::new(1031, 1037);
        let mut doc = photocraft_doc::Document::with_background(
            "GPU map padded Blur",
            size,
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::BLACK,
        );

        let gray = photocraft_color::PixelFormat::GRAY8;
        let rgba = photocraft_color::PixelFormat::RGBA8;
        let base = photocraft_raster::Surface::with_default(gray, &[0.2]);
        let mut paint = photocraft_raster::Surface::with_default(rgba, &[0.0; 4]);
        paint.write_region(photocraft_geom::Rect::new(1023, 1023, 1024, 1024), &[1.0, 1.0, 1.0, 1.0]);
        let empty = || photocraft_raster::Surface::with_default(rgba, &[0.0; 4]);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.paint = Some(photocraft_doc::mask_paint::Stack {
            base,
            layers: vec![
                photocraft_doc::mask_paint::Paint {
                    name: "Boundary impulse".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: paint,
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Paint,
                },
                photocraft_doc::mask_paint::Paint {
                    name: "Blur 3".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: empty(),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Blur { radius: 3 },
                },
                photocraft_doc::mask_paint::Paint {
                    name: "Blur 2".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: empty(),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Blur { radius: 2 },
                },
            ],
            active: 2,
        });
        photocraft_compose::mask_paint::refresh_in_canvas(&mut mask, doc.bounds());

        let mut layer = photocraft_doc::Layer::raster("Masked PBR source", rgba);
        *layer.surface_mut().unwrap() = photocraft_raster::Surface::with_default(rgba, &[1.0, 1.0, 1.0, 1.0]);
        let layer_id = layer.id;
        layer.mask = Some(mask);
        doc.layers.push(layer);

        let cpu = photocraft_compose::render(&doc, doc.bounds());
        let key = (doc.id.0, 1, 0, 9);
        let mut map = renderer.map_gpu_composite(&doc, key, true, None, None).unwrap();
        renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        assert_eq!(map.source, MapSource::GpuComposite);

        let gpu_pixels = read_rgba16f(&renderer, &map.texture, 0);
        let mut worst = (0.0f32, 0usize, 0usize);
        for (index, (actual, expected)) in gpu_pixels.iter().zip(&cpu.px).enumerate() {
            for channel in 0..3 {
                let expected = photocraft_color::convert::srgb_to_linear(expected[channel]);
                let diff = (actual[channel] - expected).abs();
                if diff > worst.0 {
                    worst = (diff, index, channel);
                }
            }
        }
        assert!(worst.0 < 0.004, "GPU map/CPU composite mismatch across padded Blur chunks: {worst:?}");

        let coverage = |x: usize, y: usize| gpu_pixels[y * size.width as usize + x][0];
        // PBR maps store linear RGB, so compare the spill against the linearized 0.2
        // background rather than treating the sRGB mask value as a linear threshold.
        let baseline = photocraft_color::convert::srgb_to_linear(0.2);
        assert!(coverage(1024, 1023) > baseline + 0.001, "Blur crosses the vertical chunk boundary");
        assert!(coverage(1023, 1024) > baseline + 0.001, "Blur crosses the horizontal chunk boundary");
        assert!(coverage(1024, 1024) > baseline + 0.001, "Blur reaches the diagonal chunk corner");

        // Effect-property edits are global to the mask stack. Exercise the same retained GPU
        // map after radius, opacity, and visibility changes, checking exact seam neighborhoods
        // against a fresh CPU materialization after every update.
        for (step, (radius, opacity, visible)) in [(7, 1.0, true), (7, 0.35, true), (7, 0.35, false), (5, 0.8, true)].into_iter().enumerate() {
            let canvas = doc.bounds();
            let mask = doc.layer_mut(layer_id).unwrap().mask.as_mut().unwrap();
            let old = mask.paint.clone();
            let stack = mask.paint.as_mut().unwrap();
            if let photocraft_doc::mask_paint::EffectKind::Blur { radius: current } = &mut stack.layers[1].kind {
                *current = radius;
            }
            stack.layers[1].opacity = opacity;
            stack.layers[1].visible = visible;
            photocraft_compose::mask_paint::refresh_changes_in_canvas(mask, old.as_ref(), canvas);

            let cpu = photocraft_compose::render(&doc, canvas);
            let next_key = (doc.id.0, 2 + step as u64, 0, 9);
            map = renderer.map_gpu_composite(&doc, next_key, true, Some(canvas), Some(map)).unwrap();
            renderer.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let gpu_pixels = read_rgba16f(&renderer, &map.texture, 0);
            let worst = gpu_pixels
                .iter()
                .zip(&cpu.px)
                .enumerate()
                .flat_map(|(index, (actual, expected))| {
                    (0..3).map(move |channel| {
                        let expected = photocraft_color::convert::srgb_to_linear(expected[channel]);
                        ((actual[channel] - expected).abs(), index, channel)
                    })
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap_or_default();
            assert!(worst.0 < 0.004, "GPU/CPU mismatch after Blur radius={radius}, opacity={opacity}, visible={visible}: {worst:?}");
            for (x, y) in [(1023, 1023), (1024, 1023), (1023, 1024), (1024, 1024)] {
                let i = y * size.width as usize + x;
                let expected = photocraft_color::convert::srgb_to_linear(cpu.get(x as i32, y as i32)[0]);
                assert!((gpu_pixels[i][0] - expected).abs() < 0.004, "chunk seam at ({x}, {y}) after Blur property edit");
            }
            assert_eq!(map.source, MapSource::GpuComposite);
        }
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
    }

    fn scene() -> (Scene, Arc<Geometry>) {
        let bytes = include_bytes!("../../uv/tests/fixtures/normals-reflected.fbx");
        let scene = import::load("test.fbx", bytes, 0, |_| Err(photocraft_uv::Error("absent".into()))).unwrap();
        let geometry = Arc::new(Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap());
        (scene, geometry)
    }
    fn read(renderer: &Renderer) -> Vec<[u8; 4]> {
        let target = renderer.target.as_ref().unwrap();
        read_texture(renderer, &target._texture, 0)
    }
    fn read_texture(renderer: &Renderer, texture: &wgpu::Texture, mip: u32) -> Vec<[u8; 4]> {
        let (w, h) = ((texture.width() >> mip).max(1), (texture.height() >> mip).max(1));
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = renderer.rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv test readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.rs.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            extent([w, h]),
        );
        renderer.rs.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        assert!(renderer.health.wait(&renderer.rs.device, None), "{:?}", renderer.health.fault());
        rx.recv().unwrap().unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        let pixels = mapped.chunks_exact(row as usize).flat_map(|r| r[..w as usize * 4].as_chunks::<4>().0.iter().copied()).collect();
        drop(mapped);
        buffer.unmap();
        pixels
    }
    fn read_rgba16f(renderer: &Renderer, texture: &wgpu::Texture, mip: u32) -> Vec<[f32; 4]> {
        let (w, h) = ((texture.width() >> mip).max(1), (texture.height() >> mip).max(1));
        let row = (w * 8).div_ceil(256) * 256;
        let buffer = renderer.rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv float map readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.rs.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            extent([w, h]),
        );
        renderer.rs.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            let _ = tx.send(result);
        });
        assert!(renderer.health.wait(&renderer.rs.device, None), "{:?}", renderer.health.fault());
        rx.recv().unwrap().unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        let pixels = mapped
            .chunks_exact(row as usize)
            .flat_map(|line| {
                line[..w as usize * 8].as_chunks::<8>().0.iter().map(|p| {
                    std::array::from_fn(|i| {
                        let bits = u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]);
                        let sign = u32::from(bits & 0x8000) << 16;
                        let exponent = u32::from((bits >> 10) & 0x1f);
                        let fraction = u32::from(bits & 0x03ff);
                        if exponent == 0 { 0.0 } else { f32::from_bits(sign | ((exponent + 112) << 23) | (fraction << 13)) }
                    })
                })
            })
            .collect();
        drop(mapped);
        buffer.unmap();
        pixels
    }
    #[test]
    fn pbr_environment_emissive_and_resident_navigation() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let env = Arc::new(photocraft_uv::lighting::Environment::studio());
        let mut surfaces = crate::uv_lighting::SurfaceMaps::default();
        for (kind, value) in [(0, [0.4, 0.1, 0.03, 1.0]), (1, [0.8; 4]), (2, [0.15; 4]), (3, [1.0; 4]), (4, [0.0, 0.0, 0.0, 1.0])] {
            surfaces.images[kind] = scene.materials.iter().map(|_| Some(Image { width: 2, height: 2, rgba: vec![value; 4] })).collect();
            surfaces.keys[kind] = scene.materials.iter().map(|_| (kind as u64 + 1, 1, 0, 0)).collect();
        }
        let hidden = BTreeSet::new();
        let selected = BTreeSet::new();
        let render = |r: &mut Renderer, camera: Camera, hidden: &BTreeSet<usize>| {
            r.draw(Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry: geometry.clone(),
                camera,
                size: [256, 256],
                antialias: false,
                selected: &selected,
                hidden,
                generation: 1,
                colors: &[],
                normals: &[],
                keys: &[],
                normal_keys: &[],
            })
            .unwrap();
            assert!(r.health.is_ok(), "{:?}", r.health.fault());
            read_texture(r, &r.target.as_ref().unwrap()._texture, 0)
        };
        let camera = Camera { shaded: true, ..Default::default() };
        let settings = photocraft_uv::lighting::Settings::default();
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        let first = render(&mut renderer, camera, &hidden);
        let generation = renderer.map_generation;
        let map_pointer = renderer.surfaces[0].maps[0].data.as_ref().unwrap().as_ptr();
        for i in 0..12 {
            renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
            render(&mut renderer, Camera { yaw: camera.yaw + i as f64 * 0.01, ..camera }, &hidden);
        }
        assert_eq!(renderer.map_generation, generation, "orbit must not upload maps");
        assert_eq!(renderer.surfaces[0].maps[0].data.as_ref().unwrap().as_ptr(), map_pointer);
        for im in surfaces.images[2].iter_mut().flatten() {
            im.rgba.fill([1.0; 4]);
        }
        for key in &mut surfaces.keys[2] {
            key.1 += 1;
        }
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        let rough = render(&mut renderer, camera, &hidden);
        assert!(first != rough, "roughness must change reflected lighting");
        let old = surfaces.keys[0][0];
        let pointer = renderer.surfaces[0].maps[0].data.as_ref().unwrap().as_ptr();
        surfaces.images[0][0].as_mut().unwrap().rgba[0] = [0.1, 0.5, 0.2, 1.0];
        surfaces.keys[0][0].1 += 1;
        surfaces.regions[0] = vec![None; scene.materials.len()];
        surfaces.regions[0][0] = Some((old, vec![[0, 0, 1, 1]]));
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        assert_eq!(pointer, renderer.surfaces[0].maps[0].data.as_ref().unwrap().as_ptr(), "PBR brush damage updates the existing encoded map");
        renderer.set_lighting(photocraft_uv::lighting::Settings { brightness: 0.0, ..settings }, &surfaces, env.clone()).unwrap();
        let dark = render(&mut renderer, camera, &hidden);
        for im in surfaces.images[4].iter_mut().flatten() {
            im.rgba.fill([4.0, 0.0, 0.0, 1.0]);
        }
        for key in &mut surfaces.keys[4] {
            key.1 += 1;
        }
        renderer.set_lighting(photocraft_uv::lighting::Settings { brightness: 0.0, ..settings }, &surfaces, env.clone()).unwrap();
        let glow = render(&mut renderer, camera, &hidden);
        assert!(glow.iter().zip(&dark).any(|(a, b)| a[0] > 200 && a[1] < 5 && b[0] < 5), "emissive must remain visible with no environment light");
        renderer.set_lighting(photocraft_uv::lighting::Settings { pbr: false, ..settings }, &surfaces, env.clone()).unwrap();
        assert!(glow != render(&mut renderer, camera, &hidden));
        let hidden = (0..scene.materials.len()).collect();
        let source = Image { width: 8, height: 4, rgba: (0..32).map(|i| [(i % 8) as f32, 0.2, 0.1, 1.0]).collect() };
        let env = Arc::new(photocraft_uv::lighting::Environment::prepare(&source).unwrap());
        renderer.set_lighting(photocraft_uv::lighting::Settings { background: true, ..settings }, &surfaces, env.clone()).unwrap();
        let sky = render(&mut renderer, Camera { perspective: true, ..camera }, &hidden);
        renderer.set_lighting(photocraft_uv::lighting::Settings { background: true, rotation: 90.0, ..settings }, &surfaces, env).unwrap();
        assert!(sky != render(&mut renderer, Camera { perspective: true, ..camera }, &hidden), "environment rotation changes the background");
    }
    #[test]
    fn mip_levels_preserve_native_texels_and_refresh_after_small_edits() {
        let _gpu = photocraft_testkit::gpu_lock();
        let renderer = renderer();
        let mut image = Image {
            width: 8,
            height: 8,
            rgba: (0..64)
                .map(|i| {
                    let c = if (i / 8 + i % 8) % 2 == 0 { 0.0 } else { 1.0 };
                    [c, c, c, 1.0]
                })
                .collect(),
        };
        let map = renderer.map(Some(&image), (1, 1, 0, 0), false, None).unwrap();
        assert_eq!(map.texture.size(), extent([8, 8]));
        assert_eq!(map.levels.len(), 4);
        let before = read_texture(&renderer, &map.texture, 3);
        assert!(before[0][0].abs_diff(128) <= 1);
        assert_eq!(read_texture(&renderer, &map.texture, 0)[0], [0, 0, 0, 255]);
        image.rgba[0] = [1.0; 4];
        let map = renderer.map(Some(&image), (1, 2, 0, 0), false, Some(map)).unwrap();
        assert_eq!(read_texture(&renderer, &map.texture, 0)[0], [255; 4]);
        assert!(read_texture(&renderer, &map.texture, 3)[0][0] > before[0][0]);
    }
    #[test]
    fn dirty_region_tracks_changed_pixels_across_rows() {
        let _gpu = photocraft_testkit::gpu_lock();
        let old = vec![0; 6 * 5 * 8];
        let mut new = old.clone();
        assert_eq!(changed_region(&old, &new, [6, 5], 8), None);
        new[(2 * 6 + 4) * 8] = 1;
        new[(4 * 6 + 1) * 8 + 7] = 1;
        assert_eq!(changed_region(&old, &new, [6, 5], 8), Some([1, 2, 4, 3]));
    }
    #[test]
    fn hinted_map_updates_only_the_region_and_stale_hints_fall_back_safely() {
        let _gpu = photocraft_testkit::gpu_lock();
        let renderer = renderer();
        for normal in [false, true] {
            let mut image = Image { width: 8, height: 8, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 64] };
            let map = renderer.map(Some(&image), (1, 1, 0, 0), normal, None).unwrap();
            let pointer = map.data.as_ref().unwrap().as_ptr();
            image.rgba[3 * 8 + 2] = [1.0, 0.0, 0.0, 0.0];
            let map = renderer.map_region(Some(&image), (1, 2, 0, 0), normal, Some(map), Some(((1, 1, 0, 0), vec![[2, 3, 1, 1]]))).unwrap();
            assert_eq!(pointer, map.data.as_ref().unwrap().as_ptr(), "small edits must retain the full encoded snapshot allocation");
            let stride = if normal { 8 } else { 4 };
            if normal {
                assert_eq!(&map.data.as_ref().unwrap()[(3 * 8 + 2) * stride..(3 * 8 + 2) * stride + 2], &crate::gpu_canvas::f32_to_f16(1.0).to_le_bytes());
            } else {
                assert_eq!(read_texture(&renderer, &map.texture, 0)[3 * 8 + 2], [255, 0, 0, 0]);
                assert!(map.cutout);
            }
            // The cache skipped an update: an old key cannot authorize a partial upload.
            image.rgba[7 * 8 + 7] = [0.0, 1.0, 0.0, 1.0];
            let map = renderer.map_region(Some(&image), (1, 3, 0, 0), normal, Some(map), Some(((1, 1, 0, 0), vec![[2, 3, 1, 1]]))).unwrap();
            if !normal {
                assert_eq!(read_texture(&renderer, &map.texture, 0)[7 * 8 + 7], [0, 255, 0, 255]);
            }
        }
    }
    #[test]
    fn gpu_matches_reference_normals_camera_selection_hidden_materials_and_live_maps() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let mut colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 8, height: 8, rgba: vec![[0.8, 0.4, 0.2, 1.0]; 64] })).collect();
        let normals: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 8, height: 8, rgba: vec![[0.7, 0.4, 0.95, 1.0]; 64] })).collect();
        for step in 0..16u64 {
            let selected = if step == 1 { BTreeSet::from([0]) } else { BTreeSet::new() };
            let hidden = if step == 4 { BTreeSet::from([0]) } else { BTreeSet::new() };
            let camera = Camera {
                channel: match step {
                    12 | 14 => preview::Channel::Color,
                    13 | 15 => preview::Channel::Normal,
                    _ => preview::Channel::Material,
                },
                perspective: step >= 9 && step != 12 && step != 13,
                zoom: if step == 11 { 4.0 } else { 1.0 },
                yaw: 0.65 + step as f64 * 0.15,
                shaded: true,
                authored_normals: step != 2,
                flip_faces: step == 3,
                flip_normal_y: step == 5,
                cull_backfaces: step != 8,
                ..Default::default()
            };
            if step == 6 {
                colors[0].as_mut().unwrap().rgba[3 * 8 + 3] = [0.0, 1.0, 0.0, 1.0];
            }
            if step >= 7 {
                for image in colors.iter_mut().flatten() {
                    for p in &mut image.rgba {
                        p[3] = if step == 7 { 0.0 } else { 1.0 };
                    }
                }
            }
            let keys = vec![(1, if step >= 6 { step - 4 } else { 1 }, 0, 0); colors.len()];
            let nkeys = vec![(2, 1, 0); normals.len()];
            let draw = Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry: geometry.clone(),
                camera,
                size: [256, 192],
                antialias: false,
                selected: &selected,
                hidden: &hidden,
                generation: step,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            };
            renderer.draw(draw).unwrap();
            if step > 0 && step < 6 {
                assert_eq!(renderer.map_generation, scene.materials.len() as u64, "camera and shell selection must not reload materials");
            }
            let actual = read(&renderer);
            let maps = Materials { colors: &colors, normals: &normals, hidden: hidden.clone() };
            let selection: Vec<_> = geometry.triangles.iter().map(|t| selected.contains(&(t.shell as usize))).collect();
            let expected = preview::render_materials(&scene, camera, &maps, &selection, 256, 192).unwrap();
            let mut bad = 0;
            let mut covered = 0;
            for y in 1..191 {
                for x in 1..255 {
                    let i = y * 256 + x;
                    // Raster edge ownership differs: compare pixels whose four neighbours share the same primitive.
                    let t = expected.triangles[i];
                    if [i - 1, i + 1, i - 256, i + 256].iter().any(|j| expected.triangles[*j] != t) {
                        continue;
                    }
                    covered += usize::from(t.is_some());
                    if actual[i].iter().zip(expected.rgba[i]).any(|(a, b)| a.abs_diff(b) > 3) {
                        bad += 1;
                    }
                }
            }
            assert!(bad < 15, "step {step}: {bad} incorrect interior pixels; {covered} covered");
        }
        // MSAA resolve, resize and cached frame reuse must leave a valid GPU image.
        let empty = BTreeSet::new();
        let keys = vec![(1, 7, 0, 0); colors.len()];
        let nkeys = vec![(2, 1, 0); normals.len()];
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry,
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &empty,
                hidden: &empty,
                generation: 8,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_eq!(read(&renderer).len(), 300 * 220);
        assert_eq!(renderer.target.as_ref().unwrap()._depth.sample_count(), 1);
        let retained = renderer.target.as_ref().unwrap()._texture.clone();
        // The shared preview budget can resize a map without changing its document revision.
        let geometry = renderer.mesh.as_ref().unwrap().geometry.clone();
        for image in colors.iter_mut().flatten() {
            *image = Image { width: 1, height: 1, rgba: vec![[0.0, 0.8, 0.0, 1.0]] };
        }
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &empty,
                hidden: &empty,
                generation: 9,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert!(renderer.materials.iter().flatten().all(|m| m.color.size() == [1, 1]));
        assert!(read(&renderer).iter().any(|p| p[1] > p[0] + 10));
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry,
                camera: Camera::default(),
                size: [300, 220],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 10,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_eq!(retained, renderer.target.as_ref().unwrap()._texture, "navigation must reuse the existing render target");
        // A small motion frame and the full idle frame retain their attachments across gestures.
        let geometry = renderer.mesh.as_ref().unwrap().geometry.clone();
        for (size, antialias) in [([150, 110], false), ([300, 220], true), ([150, 110], false), ([300, 220], true)] {
            renderer
                .draw(Draw {
                    masked: &BTreeSet::new(),
                    symmetry_plane: None,
                    scene: &scene,
                    geometry: geometry.clone(),
                    camera: Camera::default(),
                    size,
                    antialias,
                    selected: &empty,
                    hidden: &empty,
                    generation: 10,
                    colors: &colors,
                    normals: &normals,
                    keys: &keys,
                    normal_keys: &nkeys,
                })
                .unwrap();
            assert_eq!(read(&renderer).len(), size[0] as usize * size[1] as usize);
            if size == [300, 220] {
                assert_eq!(retained, renderer.target.as_ref().unwrap()._texture, "idle target must survive repeated motion gestures");
            }
        }
        let plain = read(&renderer);
        let selected = geometry.triangles.iter().map(|t| t.shell as usize).collect();
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                symmetry_plane: None,
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &selected,
                hidden: &empty,
                generation: 10,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_ne!(plain, read(&renderer), "a UI highlight toggle must invalidate the GPU frame even without an engine generation change");
    }
    #[test]
    #[ignore = "explicit GPU/CPU benchmark; waits for completed submissions, excludes first upload"]
    fn benchmark_gpu_and_cpu_same_scene() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let empty = BTreeSet::new();
        let colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.8, 0.8, 0.8, 1.0]; 2048 * 2048] })).collect();
        let normals: Vec<_> =
            scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 2048 * 2048] })).collect();
        let keys = vec![(1, 1, 0, 0); colors.len()];
        let nkeys = vec![(2, 1, 0); normals.len()];
        eprintln!("GPU adapter: {:?}", renderer.rs.adapter.get_info());
        for size in [[1024, 576], [1920, 1080], [3840, 2160]] {
            for aa in [false, true] {
                let mut gpu = Vec::new();
                let mut cpu = Vec::new();
                let mut timestamps = Vec::new();
                for step in 0..32 {
                    let camera = Camera { yaw: 0.65 + f64::from(step) * 0.01, shaded: true, ..Default::default() };
                    let start = std::time::Instant::now();
                    renderer
                        .draw(Draw {
                            masked: &BTreeSet::new(),
                            symmetry_plane: None,
                            scene: &scene,
                            geometry: geometry.clone(),
                            camera,
                            size,
                            antialias: aa,
                            selected: &empty,
                            hidden: &empty,
                            generation: 1,
                            colors: &colors,
                            normals: &normals,
                            keys: &keys,
                            normal_keys: &nkeys,
                        })
                        .unwrap();
                    assert!(renderer.health.wait(&renderer.rs.device, None), "{:?}", renderer.health.fault());
                    if step > 7 {
                        gpu.push(start.elapsed().as_secs_f64() * 1000.0);
                        if let Some(timing) = renderer.timer.as_mut().and_then(|t| t.take()) {
                            timestamps.push(timing.ms);
                        }
                    }
                }
                for step in 0..3 {
                    if !aa {
                        let camera = Camera { yaw: 0.65 + f64::from(step) * 0.01, shaded: true, ..Default::default() };
                        let start = std::time::Instant::now();
                        std::hint::black_box(
                            preview::render_materials(
                                &scene,
                                camera,
                                &Materials { colors: &colors, normals: &normals, ..Default::default() },
                                &[],
                                size[0] as usize,
                                size[1] as usize,
                            )
                            .unwrap(),
                        );
                        cpu.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                gpu.sort_by(f64::total_cmp);
                cpu.sort_by(f64::total_cmp);
                timestamps.sort_by(f64::total_cmp);
                eprintln!(
                    "{}x{} MSAA={}: GPU completed median {:.2} ms; CPU median {:?} ms",
                    size[0],
                    size[1],
                    if aa { renderer.aa_samples } else { 1 },
                    gpu[gpu.len() / 2],
                    cpu.get(cpu.len() / 2)
                );
                eprintln!("Hardware timestamp median {:?} ms", timestamps.get(timestamps.len() / 2));
            }
        }
    }
    #[test]
    #[ignore = "local-model CPU/GPU zoom diagnosis; supply PHOTOCRAFT_UV_PROFILE_MODEL"]
    fn profile_local_model_zoom() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::{PhotocraftApp, Services};
        use egui_kittest::kittest::Queryable as _;
        let path = std::env::var("PHOTOCRAFT_UV_PROFILE_MODEL").expect("Supply a local model path");
        let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![path]))]);
        let capture = Arc::new(std::sync::Mutex::new(None));
        let slot = capture.clone();
        let mut harness =
            egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 1440.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services { file_dialog: Some(dialog), ..Default::default() });
                let rs = cc.wgpu_render_state.clone().unwrap();
                *slot.lock().unwrap() = Some(rs.clone());
                app.set_uv_wgpu(rs).unwrap();
                app.ui.uv_workbench.view_3d = true;
                app.ui.uv_workbench.fast_navigation = false;
                crate::uv_ui::reveal(&mut app).unwrap();
                app
            });
        let rs = capture.lock().unwrap().clone().unwrap();
        assert_ne!(rs.adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        harness.input_mut().max_texture_side = Some(rs.device.limits().max_texture_dimension_2d as usize);
        harness.get_by_label(tl!("Import model…")).click();
        for _ in 0..4 {
            harness.step();
        }
        assert!(harness.state().session.uv.scene.is_some());
        assert!(harness.state().uv_gpu_active(), "native picker discarded the GPU renderer");
        eprintln!("Profile adapter {:?}", rs.adapter.get_info());
        eprintln!("Resident maps {:?}", harness.state().uv_gpu_texture_sizes());
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
        };
        let panel = true;
        {
            for aa in [false, true] {
                for zoom in [1.0, 3.0, 8.0] {
                    let mut cpu = Vec::new();
                    let mut wait = Vec::new();
                    let mut gpu = Vec::new();
                    for frame in 0..48 {
                        let state = &mut harness.state_mut().ui.uv_workbench;
                        state.zoom = zoom;
                        state.antialias = aa;
                        state.yaw = 0.65 + f64::from(frame) * 0.003;
                        let start = std::time::Instant::now();
                        harness.step();
                        let submitted = start.elapsed().as_secs_f64() * 1000.0;
                        let wait_start = std::time::Instant::now();
                        rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                        let waited = wait_start.elapsed().as_secs_f64() * 1000.0;
                        let renderer = harness.state_mut().uv_cache.gpu.as_mut().unwrap();
                        if frame >= 12 {
                            cpu.push(submitted);
                            wait.push(waited);
                            if let Some(timing) = renderer.timer.as_mut().and_then(|t| t.take()) {
                                gpu.push(timing.ms);
                            }
                        }
                    }
                    eprintln!(
                        "panel={panel} AA={aa} zoom={zoom}: CPU UI/submit {:.3}ms; completion wait {:.3}ms; GPU timestamp {:.3}ms ({} samples); target {:?}",
                        median(&mut cpu),
                        median(&mut wait),
                        median(&mut gpu),
                        gpu.len(),
                        harness.state().uv_cache.gpu.as_ref().unwrap().frame_info()
                    );
                }
            }
        }
        if let Ok(output) = std::env::var("PHOTOCRAFT_UV_PROFILE_SCREENSHOT") {
            harness.state_mut().ui.uv_workbench.zoom = 1.0;
            harness.step();
            harness.render().unwrap().save(output).unwrap();
        }
    }
    #[test]
    #[ignore = "explicit 100k-triangle navigation benchmark"]
    fn benchmark_large_grid() {
        let _gpu = photocraft_testkit::gpu_lock();
        use photocraft_uv::{Corner, Material, Triangle};
        let side = 224usize;
        let mut scene = Scene { materials: vec![Material::default()], ..Default::default() };
        for y in 0..=side {
            for x in 0..=side {
                scene.positions.push([x as f64 / side as f64, y as f64 / side as f64, 0.0]);
            }
        }
        for y in 0..side {
            for x in 0..side {
                let a = y * (side + 1) + x;
                let b = a + 1;
                let c = a + side + 1;
                let d = c + 1;
                for ids in [[a, b, d], [a, d, c]] {
                    scene.triangles.push(Triangle {
                        material: 0,
                        corners: ids.map(|vertex| Corner { vertex, uv: [scene.positions[vertex][0], scene.positions[vertex][1]], normal: None }),
                    });
                }
            }
        }
        let geometry = Arc::new(Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap());
        let mut renderer = renderer();
        let empty = BTreeSet::new();
        let colors = vec![Some(Image { width: 2048, height: 2048, rgba: vec![[0.8, 0.6, 0.4, 1.0]; 2048 * 2048] })];
        let normals = vec![Some(Image { width: 2048, height: 2048, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 2048 * 2048] })];
        for size in [[1920, 1080], [3840, 2160]] {
            let mut times = Vec::new();
            for step in 0..32 {
                let camera = Camera { yaw: 0.3 + f64::from(step) * 0.002, pitch: 0.3, shaded: true, ..Default::default() };
                let start = std::time::Instant::now();
                renderer
                    .draw(Draw {
                        masked: &BTreeSet::new(),
                        symmetry_plane: None,
                        scene: &scene,
                        geometry: geometry.clone(),
                        camera,
                        size,
                        antialias: false,
                        selected: &empty,
                        hidden: &empty,
                        generation: 1,
                        colors: &colors,
                        normals: &normals,
                        keys: &[(1, 1, 0, 0)],
                        normal_keys: &[(2, 1, 0)],
                    })
                    .unwrap();
                assert!(renderer.health.wait(&renderer.rs.device, None));
                if step > 7 {
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            let start = std::time::Instant::now();
            std::hint::black_box(
                preview::render_materials(
                    &scene,
                    Camera { yaw: 0.3, pitch: 0.3, shaded: true, ..Default::default() },
                    &Materials { colors: &colors, normals: &normals, ..Default::default() },
                    &[],
                    size[0] as usize,
                    size[1] as usize,
                )
                .unwrap(),
            );
            eprintln!(
                "{} triangles {}x{}: GPU completed median {:.2} ms; CPU single frame {:.2} ms",
                scene.triangles.len(),
                size[0],
                size[1],
                times[times.len() / 2],
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
