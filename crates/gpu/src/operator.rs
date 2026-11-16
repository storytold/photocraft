//! Layer operators (#5772): the content of a layer, as the compositor samples it, is the output of
//! a GPU program run on the layer's pixels. Everything above the layer composes unchanged (blend
//! modes, opacity, masks, clipping, adjustments); only where the layer's pixels come from changes.
//!
//! The first program is a separable blur (Gaussian Blur's live preview). The interface already has
//! what later programs need, so it can grow without being redesigned:
//! - inputs are a list ([`OperatorInput`]): the target's content and the selection today; other
//!   layers, masks and channels later (Displace, Lens Blur's depth, Apply Image);
//! - each program states its region rule ([`OperatorProgram::source_region`]): grown by a reach
//!   here; an inverse-mapped area for a warp, or the whole image for statistics, later;
//! - a reduction pre-pass hook ([`OperatorProgram::prepass`]) is reserved but unused.
//!
//! Every program has a CPU twin (the filter itself, in `photocraft-algo`), which stays the oracle
//! for the parity tests, the fallback, and what OK applies.
//!
//! The compositor runs a program per page cell over a window of the layer's pixels (the cell's
//! part of the output grown by the reach), uploaded once from the layer's tiles and kept while
//! they don't change, so a parameter change only re-runs the passes. The output is stored as a
//! resident page (`Role::Operator`), which the cell's passes bind instead of the layer's own.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use photocraft_color::SampleType;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// An input a program reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OperatorInput {
    /// The pixels of the layer the operator is on.
    TargetContent,
    /// The selection the filter is limited to.
    Selection,
    /// Displace's map (another document's composite).
    Map,
}

/// A reduction pass a program needs before its per-pixel work (histograms, min/max): reserved for
/// later programs, none uses one yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Prepass {}

/// A blur made of passes along each axis, horizontal first: the description the CPU blur is built
/// from (`photocraft_algo::separable::Separable`), handed over as data so the GPU never derives
/// weights of its own.
#[derive(Clone, Debug, PartialEq)]
pub enum SeparableBlur {
    /// One pass of this normalised kernel (odd length, centred) along each axis.
    Kernel(Vec<f32>),
    /// Running-mean box passes of these odd widths, in turn, along each axis.
    Boxes(Vec<usize>),
    /// Minimum or Maximum over a square: per channel of the straight values (alpha included),
    /// the extreme over `radius` texels each side along each axis in turn.
    Extreme { radius: u32, max: bool },
    /// Minimum or Maximum over a disc: on each row `dy` in `-r ..= r` (`r` = half the length),
    /// the extreme over `half_widths[dy + r]` texels each side.
    Disc { half_widths: Vec<u32>, max: bool },
    /// A convolution of the straight colour values: the sum of `weight * texel(dx, dy)` over the
    /// taps in order, divided by `scale` and offset by `offset`; alpha stays.
    Convolve { taps: Vec<(i32, i32, f32)>, scale: f32, offset: f32 },
    /// Displace (`photocraft_algo` `distort2::displace`): each pixel samples the source where the
    /// map shifts it. It can read anywhere inside `bounds` (and wraps or repeats past them), so
    /// it reads the layer's whole extent ([`SeparableBlur::whole`]) rather than a reach.
    Displace(Arc<DisplaceParams>),
}

/// What [`SeparableBlur::Displace`] needs besides the source.
#[derive(Debug, PartialEq)]
pub struct DisplaceParams {
    /// The map's straight RGBA texels, row by row (`photocraft_compose::flatten` of the map
    /// document): red shifts horizontally, green vertically. Shared by a dialog session's steps:
    /// its identity keys the GPU's copy.
    pub map: Arc<Vec<[f32; 4]>>,
    pub map_size: [u32; 2],
    /// The filter's bounds (the selection's, else the canvas): the map is stretched over or
    /// tiled from them, and the undefined areas are past them.
    pub bounds: Rect,
    /// The largest shift along each axis in pixels (Scale / 100 × 128).
    pub scale: [f32; 2],
    /// Stretch to Fit, else Tile.
    pub stretch: bool,
    pub undefined: Undefined,
}

/// Displace's Undefined Areas (`photocraft_algo` `image::Edge`): what a sample past the bounds
/// reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Undefined {
    Transparent,
    /// Repeat Edge Pixels.
    Repeat,
    /// Wrap Around.
    Wrap,
}

/// Most taps a [`SeparableBlur::Convolve`] may have (Custom's 5×5).
const MAX_TAPS: usize = 25;

/// Largest disc ([`SeparableBlur::Disc`]) the GPU runs. Its passes read about four texels a
/// pixel per unit of radius (one step per half-width, [`record_disc`]): at this radius 63 ms a
/// frame on an RTX 4090 Laptop for a 1920×1080 view (the CPU filter takes 22 s over 36 MP).
/// Larger ones would block a slower GPU's frames for too long, and preview on the CPU, off the
/// UI thread.
pub const MAX_DISC_RADIUS: usize = 100;

/// Most texels a kernel or a square Minimum/Maximum pass reads per pixel along each axis: about
/// the disc's cost at [`MAX_DISC_RADIUS`]. Measured on an RTX 4090 Laptop at a 1920×1080 view, a
/// square of radius 10 (21 texels) takes 2.1 ms and Unsharp Mask's 241-tap kernel (radius 40)
/// 9.2 ms, so 601 texels comes to about 60 ms; on an integrated GPU, much more could run past
/// the driver's timeout and lose the device. Past it (Unsharp Mask and High Pass above radius
/// 100, squares above radius 300) the preview stays on the CPU.
pub const MAX_AXIS_TAPS: usize = 601;

impl SeparableBlur {
    /// How far outside an output pixel the blur reads, along each axis.
    pub fn reach(&self) -> i32 {
        let reach = match self {
            SeparableBlur::Kernel(k) => k.len() / 2,
            SeparableBlur::Boxes(boxes) => boxes.iter().fold(0usize, |sum, w| sum.saturating_add(w / 2)),
            SeparableBlur::Extreme { radius, .. } => *radius as usize,
            SeparableBlur::Disc { half_widths, .. } => (half_widths.iter().copied().max().unwrap_or(0) as usize).max(half_widths.len() / 2),
            SeparableBlur::Convolve { taps, .. } => taps.iter().map(|&(dx, dy, _)| dx.unsigned_abs().max(dy.unsigned_abs()) as usize).max().unwrap_or(0),
            // Reads its whole window ([`SeparableBlur::whole`]).
            SeparableBlur::Displace(_) => 0,
        };
        i32::try_from(reach).unwrap_or(i32::MAX)
    }

    /// Whether the passes read the layer's whole extent rather than the output grown by a reach:
    /// one window for every cell, uploaded once.
    pub fn whole(&self) -> bool {
        matches!(self, SeparableBlur::Displace(_))
    }

    /// Whether the passes premultiply what they read (the blurs, as the CPU blurs do); the window
    /// filters work on the straight values.
    fn premultiplies(&self) -> bool {
        matches!(self, SeparableBlur::Kernel(_) | SeparableBlur::Boxes(_))
    }

    /// Whether the description is one the passes can run: an odd, finite kernel, or odd boxes.
    fn valid(&self) -> bool {
        match self {
            SeparableBlur::Kernel(k) => k.len() % 2 == 1 && k.len() <= MAX_AXIS_TAPS && k.iter().all(|w| w.is_finite()),
            SeparableBlur::Boxes(b) => !b.is_empty() && b.iter().all(|w| w % 2 == 1),
            SeparableBlur::Extreme { radius, .. } => (*radius as usize).saturating_mul(2).saturating_add(1) <= MAX_AXIS_TAPS,
            SeparableBlur::Disc { half_widths, .. } => half_widths.is_empty() || (half_widths.len() % 2 == 1 && half_widths.len() / 2 <= MAX_DISC_RADIUS),
            SeparableBlur::Convolve { taps, scale, offset } => {
                taps.len() <= MAX_TAPS && taps.iter().all(|t| t.2.is_finite()) && scale.is_finite() && *scale != 0.0 && offset.is_finite()
            }
            SeparableBlur::Displace(d) => {
                let [w, h] = d.map_size;
                w > 0 && h > 0 && d.map.len() as u64 == u64::from(w) * u64::from(h) && d.scale.iter().all(|s| s.is_finite()) && !d.bounds.is_empty()
            }
        }
    }
}

/// What a program runs on the GPU. Internal to the compositor; programs describe themselves with
/// it so their GPU work stays inside this crate.
#[derive(Debug)]
#[non_exhaustive]
pub enum Program<'a> {
    /// A separable blur of the target's content ([`GaussianBlur`]).
    Separable(&'a GaussianBlur),
}

/// A GPU program whose output replaces a layer's content.
pub trait OperatorProgram: Send + Sync + std::fmt::Debug {
    /// Fingerprint of the parameters and the identity of the inputs other than the target's
    /// pixels (whose tiles the compositor tracks itself): outputs with another key are redrawn.
    fn key(&self) -> u64;
    /// What it reads.
    fn inputs(&self) -> &[OperatorInput];
    /// The source pixels the output over `out` reads (the region rule).
    fn source_region(&self, out: Rect) -> Rect;
    /// Where the output can differ from the source (elsewhere it is the source).
    fn changed_area(&self) -> Rect;
    /// A reduction pass before the per-pixel work. Reserved: always `None` so far.
    fn prepass(&self) -> Option<Prepass> {
        None
    }
    /// The GPU work, for the compositor.
    fn program(&self) -> Program<'_>;
}

/// What of its layer an operator's output replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OperatorTarget {
    /// The layer's pixels.
    #[default]
    Content,
    /// The layer's pixel mask (a filter run with the mask targeted). The program reads the mask
    /// as grey and the compositor samples the output's first channel in place of the mask; a
    /// layer whose mask is combined with vector masks is refused.
    Mask,
}

/// An operator on one layer of a document.
#[derive(Clone, Debug)]
pub struct LayerOperator {
    pub layer: LayerId,
    pub target: OperatorTarget,
    pub program: Arc<dyn OperatorProgram>,
}

/// How a [`GaussianBlur`]'s result is made from the blurred and the source pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Combine {
    /// The blur itself (Gaussian Blur).
    Blur,
    /// Unsharp Mask (`photocraft_algo` `sharpen::exact`): per colour channel, with
    /// `d = source - blurred`, `source + amount * d` where `|d| + 1e-6 >= threshold`, else the
    /// source; a fully transparent source pixel and its alpha stay. `amount` is a fraction
    /// (Amount / 100), `threshold` a fraction of 1 (Threshold / 255).
    Unsharp { amount: f32, threshold: f32 },
    /// High Pass (`photocraft_algo` `sharpen::high_pass`): per colour channel
    /// `0.5 + source - blurred`; the source's alpha, fully transparent pixels untouched.
    HighPass,
}

/// Gaussian Blur (or any separable blur) of a raster layer, exactly as the filter runs on the CPU
/// (`photocraft_algo::apply_in_with`): the edge pixels repeat at `extent`, only `area` changes,
/// the selection mixes the result with the source, Lock Transparency keeps the source's alpha,
/// and results are rounded to the document's depth.
#[derive(Debug)]
pub struct GaussianBlur {
    pub(crate) blur: SeparableBlur,
    pub(crate) extent: Rect,
    pub(crate) area: Rect,
    pub(crate) selection: Option<Arc<Surface>>,
    pub(crate) keep_alpha: bool,
    pub(crate) depth: SampleType,
    pub(crate) combine: Combine,
    key: u64,
}

/// The inputs of a [`GaussianBlur`].
const BLUR_INPUTS: &[OperatorInput] = &[OperatorInput::TargetContent, OperatorInput::Selection];

/// The inputs of a Displace.
const DISPLACE_INPUTS: &[OperatorInput] = &[OperatorInput::TargetContent, OperatorInput::Selection, OperatorInput::Map];

/// The longest reach a blur may have: its window is a page cell grown by the reach on each side.
pub const MAX_REACH: i32 = 1024;

impl GaussianBlur {
    /// A blur of `blur` over `area` (`photocraft_algo::output_area`), repeating the edge at `extent`
    /// (`photocraft_algo::separable::edge_extent`), mixed by `selection`, keeping the source's
    /// alpha when `keep_alpha`, rounded to `depth`. `None` for a description the passes can't run
    /// or a reach past [`MAX_REACH`] (the preview then stays on the CPU).
    pub fn new(blur: SeparableBlur, extent: Rect, area: Rect, selection: Option<Arc<Surface>>, keep_alpha: bool, depth: SampleType) -> Option<Self> {
        if !blur.valid() || blur.reach() > MAX_REACH || extent.is_empty() {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match &blur {
            SeparableBlur::Kernel(k) => (0u8, k.iter().map(|w| w.to_bits()).collect::<Vec<_>>()).hash(&mut h),
            SeparableBlur::Boxes(b) => (1u8, b).hash(&mut h),
            SeparableBlur::Extreme { radius, max } => (4u8, radius, max).hash(&mut h),
            SeparableBlur::Disc { half_widths, max } => (5u8, half_widths, max).hash(&mut h),
            SeparableBlur::Convolve { taps, scale, offset } => {
                (6u8, taps.iter().map(|&(dx, dy, w)| (dx, dy, w.to_bits())).collect::<Vec<_>>(), scale.to_bits(), offset.to_bits()).hash(&mut h)
            }
            // The settings, and the map by identity (a dialog session holds one).
            SeparableBlur::Displace(d) => {
                (7u8, Arc::as_ptr(&d.map) as usize, d.map_size, d.bounds, d.scale.map(f32::to_bits), d.stretch, d.undefined).hash(&mut h)
            }
        }
        (extent, area, keep_alpha, depth as u8).hash(&mut h);
        // The selection by identity: a dialog session holds one.
        selection.as_ref().map(|s| Arc::as_ptr(s) as usize).hash(&mut h);
        Some(GaussianBlur { blur, extent, area, selection, keep_alpha, depth, combine: Combine::Blur, key: h.finish() })
    }

    /// The same blur combined with the source as Unsharp Mask does: `amount` and `threshold` as
    /// the filter's own (Amount in percent, Threshold in levels). `None` for values that aren't
    /// finite.
    pub fn unsharp(self, amount: f32, threshold: f32) -> Option<Self> {
        if !amount.is_finite() || !threshold.is_finite() {
            return None;
        }
        let combine = Combine::Unsharp { amount: amount / 100.0, threshold: threshold / 255.0 };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.key, 2u8, amount.to_bits(), threshold.to_bits()).hash(&mut h);
        Some(GaussianBlur { combine, key: h.finish(), ..self })
    }

    /// The same blur as High Pass combines it with the source.
    pub fn high_pass(self) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.key, 3u8).hash(&mut h);
        GaussianBlur { combine: Combine::HighPass, key: h.finish(), ..self }
    }

    /// How far the blur reads outside an output pixel.
    pub fn reach(&self) -> i32 {
        self.blur.reach()
    }
}

impl OperatorProgram for GaussianBlur {
    fn key(&self) -> u64 {
        self.key
    }
    fn inputs(&self) -> &[OperatorInput] {
        if self.blur.whole() { DISPLACE_INPUTS } else { BLUR_INPUTS }
    }
    fn source_region(&self, out: Rect) -> Rect {
        match () {
            _ if out.is_empty() => Rect::EMPTY,
            _ if self.blur.whole() => self.extent,
            _ => out.inflate(self.reach()).intersect(&self.extent),
        }
    }
    fn changed_area(&self) -> Rect {
        self.area.intersect(&self.extent)
    }
    fn program(&self) -> Program<'_> {
        Program::Separable(self)
    }
}

/// The window of straight RGBA texels a blur runs over for output `out`: `out` grown by the reach,
/// with the source's edge pixels repeated beyond `extent` (as `Image::read_clamped` reads them
/// on the CPU), converted as the page texels are (`photocraft_raster::to_rgba`).
pub(crate) fn read_window(surface: &Surface, rect: Rect, extent: Rect) -> Vec<[f32; 4]> {
    let fmt = surface.format();
    let ch = fmt.channels().max(1);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut texels = vec![[0.0f32; 4]; w.saturating_mul(h)];
    if texels.is_empty() {
        return texels;
    }
    // As `Image::read_clamped`: repeat the edge of the part inside the extent, or read the surface
    // as it is when the window lies entirely outside it.
    let inner = rect.intersect(&extent);
    let inner = if inner.is_empty() { rect } else { inner };
    let left = (inner.x0 - rect.x0) as usize;
    let fill = |row: usize, dst: &mut [[f32; 4]]| {
        let y = (rect.y0 + row as i32).clamp(inner.y0, inner.y1 - 1);
        let mut samples = Vec::new();
        surface.read_region_into(Rect::new(inner.x0, y, inner.x1, y + 1), &mut samples);
        let px: Vec<[f32; 4]> = samples.chunks_exact(ch).map(|s| photocraft_raster::to_rgba(&fmt, s)).collect();
        let (Some(first), Some(last)) = (px.first(), px.last()) else { return };
        for (i, d) in dst.iter_mut().enumerate() {
            *d = match i.checked_sub(left) {
                None => *first,
                Some(k) => px.get(k).copied().unwrap_or(*last),
            };
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        texels.par_chunks_mut(w).enumerate().for_each(|(row, dst)| fill(row, dst));
    }
    #[cfg(target_arch = "wasm32")]
    for (row, dst) in texels.chunks_mut(w).enumerate() {
        fill(row, dst);
    }
    texels
}

/// The selection over `rect` as one float per pixel (0 outside it).
pub(crate) fn read_selection(selection: &Surface, rect: Rect) -> Vec<f32> {
    let ch = selection.channels().max(1);
    selection.read_region(rect).chunks_exact(ch).map(|s| s.first().copied().unwrap_or(0.0)).collect()
}

/// The margin a blur's window keeps around its output: the reach rounded up to 128 pixels, so the
/// small reach changes of a slider drag keep the window (and its upload). A wider window gives
/// the same results: outputs never read within the reach of its edge, and beyond the extent it
/// repeats the edge anyway.
pub(crate) fn apron(reach: i32) -> i32 {
    reach.clamp(0, MAX_REACH).saturating_add(127) / 128 * 128
}

/// Whether a source window of a `depth` document is stored as 8-bit texels: exactly the values
/// the CPU reads (`u8 / 255`), a quarter of the floats' bytes.
pub(crate) fn source_is_8bit(depth: SampleType) -> bool {
    depth == SampleType::U8
}

/// A source window's texels as `source_is_8bit` stores them (RGBA8, or RGBA32F).
pub(crate) fn source_bytes(depth: SampleType, texels: &[[f32; 4]]) -> Vec<u8> {
    if source_is_8bit(depth) {
        texels.iter().flat_map(|t| t.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect()
    } else {
        texels.iter().flat_map(|t| t.iter().flat_map(|v| v.to_le_bytes())).collect()
    }
}

/// Steps per unit a result is rounded to at `depth` (0: floats).
pub(crate) fn levels(depth: SampleType) -> f32 {
    match depth {
        SampleType::U8 => 255.0,
        SampleType::U16 => 65535.0,
        SampleType::F32 => 0.0,
    }
}

/// Output pixels per box-pass invocation: long enough that the window sum taken at the start of
/// each segment is a small share of the work, short enough to keep many invocations in flight.
const BOX_SEGMENT: u32 = 64;

/// The compute pipelines of the operators, created on first use.
pub(crate) struct OpKit {
    blur_layout: wgpu::BindGroupLayout,
    disc_layout: wgpu::BindGroupLayout,
    displace_layout: wgpu::BindGroupLayout,
    finish_layout: wgpu::BindGroupLayout,
    kernel: wgpu::ComputePipeline,
    boxes: wgpu::ComputePipeline,
    extreme: wgpu::ComputePipeline,
    disc: wgpu::ComputePipeline,
    convolve: wgpu::ComputePipeline,
    displace: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
}

impl OpKit {
    /// The pipelines, or `None` when the device can't run them (no compute, or no float storage
    /// textures): operators are then unsupported and previews stay on the CPU.
    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        let limits = device.limits();
        if limits.max_compute_workgroups_per_dimension == 0 || limits.max_storage_textures_per_shader_stage == 0 {
            return None;
        }
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let buffer = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let weights = buffer(3);
        let (kit, error) = crate::first_error(device, || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pc_operator"),
                source: wgpu::ShaderSource::Wgsl(include_str!("operator.wgsl").into()),
            });
            let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pc_operator_blur"),
                entries: &[tex(0), storage(1), uniform(2), weights],
            });
            // A disc step: the horizontal extreme and the result so far in, both out.
            let disc_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pc_operator_disc"),
                entries: &[tex(0), tex(1), storage(2), storage(3), uniform(4), buffer(5)],
            });
            // Displace: the source window and the map in, the displaced output out.
            let displace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pc_operator_displace"),
                entries: &[tex(0), tex(1), storage(2), uniform(3)],
            });
            let finish_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pc_operator_finish"),
                entries: &[tex(0), tex(1), tex(2), storage(3), uniform(4)],
            });
            let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
                let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(entry),
                    bind_group_layouts: &[Some(layout)],
                    immediate_size: 0,
                });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pl),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            let kernel = pipeline(&blur_layout, "kernel_pass");
            let boxes = pipeline(&blur_layout, "box_pass");
            let extreme = pipeline(&blur_layout, "extreme_pass");
            let disc = pipeline(&disc_layout, "disc_step");
            let convolve = pipeline(&blur_layout, "convolve_pass");
            let displace = pipeline(&displace_layout, "displace_pass");
            let finish = pipeline(&finish_layout, "finish");
            OpKit { blur_layout, disc_layout, displace_layout, finish_layout, kernel, boxes, extreme, disc, convolve, displace, finish }
        });
        match error {
            None => Some(kit),
            Some(e) => {
                log::warn!("GPU layer operators unavailable: {e}");
                None
            }
        }
    }
}

/// A storage-capable float texture for an operator's window or output.
pub(crate) fn float_texture(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&float_desc(label, w, h))
}

/// The descriptor of a [`float_texture`].
pub(crate) fn float_desc(label: &str, w: u32, h: u32) -> wgpu::TextureDescriptor<'_> {
    wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    }
}

/// Upload straight RGBA `texels` (`w` wide) to `texture` at (0, 0).
pub(crate) fn write_texels(queue: &wgpu::Queue, texture: &wgpu::Texture, w: u32, h: u32, texels: &[[f32; 4]]) {
    let bytes: Vec<u8> = texels.iter().flat_map(|t| t.iter().flat_map(|v| v.to_le_bytes())).collect();
    write_bytes(queue, texture, w, h, 16, &bytes);
}

/// Upload rows of `bpp`-byte texels (`w` wide) to `texture` at (0, 0).
pub(crate) fn write_bytes(queue: &wgpu::Queue, texture: &wgpu::Texture, w: u32, h: u32, bpp: u32, bytes: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

/// Bytes of `uniform` as the shader reads it (little-endian u32/f32 words).
fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Record the blur of `blur` over the `size` part at `origin` of the window in `source` into
/// `encoder`, ping-ponging between the two `scratch` textures (at least `size`), ending in the
/// returned one (straight RGBA over `size`). `None` when there's nothing to run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_blur<'s>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    kit: &OpKit,
    blur: &SeparableBlur,
    source: &wgpu::Texture,
    origin: [u32; 2],
    size: [u32; 2],
    scratch: &'s [wgpu::Texture],
) -> Option<&'s wgpu::Texture> {
    use wgpu::util::DeviceExt;
    if let SeparableBlur::Disc { half_widths, max } = blur
        && !half_widths.is_empty()
    {
        return record_disc(device, encoder, kit, half_widths, *max, source, origin, size, scratch);
    }
    let axes: Vec<(usize, u32)> = match blur {
        SeparableBlur::Kernel(_) | SeparableBlur::Extreme { .. } => vec![(0, 0), (0, 1)],
        SeparableBlur::Boxes(b) => [0u32, 1].iter().flat_map(|&axis| (0..b.len()).map(move |i| (i, axis))).collect(),
        // One 2D pass (a disc has its own, `record_disc`; Displace `record_displace`).
        SeparableBlur::Disc { .. } | SeparableBlur::Convolve { .. } => vec![(0, 0)],
        SeparableBlur::Displace(_) => return None,
    };
    let weights: Vec<u8> = match blur {
        SeparableBlur::Kernel(k) => k.iter().flat_map(|w| w.to_le_bytes()).collect(),
        SeparableBlur::Boxes(_) | SeparableBlur::Extreme { .. } | SeparableBlur::Displace(_) => 0f32.to_le_bytes().to_vec(),
        SeparableBlur::Disc { half_widths, .. } if half_widths.is_empty() => 0f32.to_le_bytes().to_vec(),
        SeparableBlur::Disc { half_widths, .. } => half_widths.iter().flat_map(|w| (*w as f32).to_le_bytes()).collect(),
        // [scale, offset, (dx, dy, weight)...]
        SeparableBlur::Convolve { taps, scale, offset } => {
            [*scale, *offset].into_iter().chain(taps.iter().flat_map(|&(dx, dy, w)| [dx as f32, dy as f32, w])).flat_map(|v| v.to_le_bytes()).collect()
        }
    };
    let weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pc_operator_weights"),
        contents: &weights,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let ping = scratch.get(..2)?;
    let last = axes.len().checked_sub(1)?;
    for (n, &(i, axis)) in axes.iter().enumerate() {
        let src = if n == 0 { source } else { &ping[(n + 1) % 2] };
        let dst = &ping[n % 2];
        // `radius`: the kernel's half-length, the box's or square's radius, the disc's rows each
        // side, or the number of taps; `norm`: the box's 1 / width, or 1 for Maximum.
        let (radius, norm, pipeline) = match blur {
            SeparableBlur::Kernel(k) => ((k.len() / 2) as u32, 0.0f32, &kit.kernel),
            SeparableBlur::Boxes(b) => {
                let w = b.get(i).copied().unwrap_or(1);
                ((w / 2) as u32, 1.0 / w as f32, &kit.boxes)
            }
            SeparableBlur::Extreme { radius, max } => (*radius, f32::from(u8::from(*max)), &kit.extreme),
            // Only the empty disc (radius under 1) comes here: a copy.
            SeparableBlur::Disc { max, .. } => (0, f32::from(u8::from(*max)), &kit.extreme),
            SeparableBlur::Convolve { taps, .. } => (taps.len() as u32, 0.0, &kit.convolve),
            SeparableBlur::Displace(_) => return None,
        };
        let (ox, oy) = if n == 0 { (origin[0], origin[1]) } else { (0, 0) };
        let pre = blur.premultiplies();
        let params = words(&[size[0], size[1], radius, axis, BOX_SEGMENT, u32::from(pre && n == 0), u32::from(pre && n == last), norm.to_bits(), ox, oy]);
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pc_operator_pass"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let src_view = src.create_view(&Default::default());
        let dst_view = dst.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_operator_pass"),
            layout: &kit.blur_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dst_view) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: weights.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_operator_pass"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        match blur {
            SeparableBlur::Boxes(_) => {
                let (lines, len) = if axis == 0 { (size[1], size[0]) } else { (size[0], size[1]) };
                pass.dispatch_workgroups(lines.div_ceil(64), len.div_ceil(BOX_SEGMENT), 1);
            }
            _ => pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1),
        }
    }
    ping.get(last % 2)
}

/// Record Displace's pass: the output over `out` (document pixels) from the source `window`
/// (document pixels, the layer's whole extent) and the `map` texture, into `scratch[0]`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_displace<'s>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    kit: &OpKit,
    d: &DisplaceParams,
    source: &wgpu::Texture,
    window: Rect,
    map: &wgpu::Texture,
    out: Rect,
    has_alpha: bool,
    scratch: &'s [wgpu::Texture],
) -> Option<&'s wgpu::Texture> {
    use wgpu::util::DeviceExt;
    let dst = scratch.first()?;
    let i = |v: i32| v as u32;
    let b = d.bounds;
    let edge = match d.undefined {
        Undefined::Transparent => 0u32,
        Undefined::Repeat => 1,
        Undefined::Wrap => 2,
    };
    let params = words(&[
        out.width(),
        out.height(),
        i(out.x0),
        i(out.y0),
        i(window.x0),
        i(window.y0),
        window.width(),
        window.height(),
        i(b.x0),
        i(b.y0),
        i(b.x1),
        i(b.y1),
        d.map_size[0],
        d.map_size[1],
        d.scale[0].to_bits(),
        d.scale[1].to_bits(),
        u32::from(d.stretch),
        edge,
        u32::from(has_alpha),
        0,
    ]);
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pc_operator_displace"),
        contents: &params,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let views = [source, map, dst].map(|t| t.create_view(&Default::default()));
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_operator_displace"),
        layout: &kit.displace_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
            wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_operator_displace"), timestamp_writes: None });
    pass.set_pipeline(&kit.displace);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(out.width().div_ceil(8), out.height().div_ceil(8), 1);
    drop(pass);
    Some(dst)
}

/// Scratch textures a description's passes need: a disc four (the horizontal extreme and the
/// result, each ping-ponged), the rest two.
pub(crate) fn scratch_count(blur: &SeparableBlur) -> usize {
    if matches!(blur, SeparableBlur::Disc { half_widths, .. } if !half_widths.is_empty()) { 4 } else { 2 }
}

/// Minimum or Maximum over a disc in `half_widths.len() / 2 + 1` steps: step `h` holds the
/// horizontal extreme over `2h + 1` texels (`H_0` is the source, `H_{h+1}[x]` the extreme of
/// `H_h[x - 1]` and `H_h[x + 1]`, and of `H_0[x]` too for `h = 0`) and folds in the disc's rows
/// whose half-width is `h`. Every row is read once, so a pixel costs about four reads per unit
/// of radius rather than a read per texel of the disc.
#[allow(clippy::too_many_arguments)]
fn record_disc<'s>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    kit: &OpKit,
    half_widths: &[u32],
    max: bool,
    source: &wgpu::Texture,
    origin: [u32; 2],
    size: [u32; 2],
    scratch: &'s [wgpu::Texture],
) -> Option<&'s wgpu::Texture> {
    use wgpu::util::DeviceExt;
    let r = (half_widths.len() / 2) as i32;
    let steps = half_widths.iter().copied().max()? as usize + 1;
    // The disc's rows by half-width: `rows[start[h]..start[h + 1]]` are the offsets dy of step h.
    let mut rows = Vec::new();
    let mut start = Vec::with_capacity(steps + 1);
    for h in 0..steps {
        start.push(rows.len() as u32);
        rows.extend((-r..=r).zip(half_widths).filter(|(_, w)| **w as usize == h).map(|(dy, _)| dy));
    }
    start.push(rows.len() as u32);
    let rows_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pc_operator_disc_rows"),
        contents: &rows.iter().flat_map(|v| v.to_le_bytes()).chain(0i32.to_le_bytes()).collect::<Vec<u8>>(),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let (hs, acc) = (scratch.get(..2)?, scratch.get(2..4)?);
    for h in 0..steps {
        let h_in = if h == 0 { source } else { &hs[(h + 1) % 2] };
        let h_out = &hs[h % 2];
        let (acc_in, acc_out) = (&acc[(h + 1) % 2], &acc[h % 2]);
        let (o0, o1) = if h == 0 { (origin[0], origin[1]) } else { (0, 0) };
        // flags: 1 the first step (no result yet, and H_1 takes the centre too), 2 grow H.
        let flags = u32::from(h == 0) | (u32::from(h + 1 < steps) << 1);
        let params = words(&[size[0], size[1], o0, o1, start[h], start[h + 1] - start[h], flags, u32::from(max)]);
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pc_operator_disc_step"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let views = [h_in, acc_in, h_out, acc_out].map(|t| t.create_view(&Default::default()));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pc_operator_disc_step"),
            layout: &kit.disc_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[3]) },
                wgpu::BindGroupEntry { binding: 4, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: rows_buf.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_operator_disc_step"), timestamp_writes: None });
        pass.set_pipeline(&kit.disc);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
    }
    acc.get((steps - 1) % 2)
}

/// What the finishing pass needs besides its textures.
pub(crate) struct FinishParams {
    /// Output size.
    pub size: [u32; 2],
    /// Where the output starts inside the source window.
    pub offset: [u32; 2],
    /// Where the output starts inside the blurred window.
    pub blur_offset: [u32; 2],
    /// The filtered area in output pixels.
    pub area: Rect,
    /// The selection texture's origin in output pixels and size; `None` without a selection.
    pub selection: Option<(Rect, wgpu::TextureView)>,
    pub keep_alpha: bool,
    pub levels: f32,
    pub has_alpha: bool,
    pub combine: Combine,
}

/// Record the finishing pass: the blurred window mixed into `out` as the layer's new pixels.
pub(crate) fn record_finish(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    kit: &OpKit,
    source: &wgpu::TextureView,
    blurred: &wgpu::TextureView,
    out: &wgpu::TextureView,
    f: FinishParams,
) {
    use wgpu::util::DeviceExt;
    let (sel_rect, sel_view) = match f.selection {
        Some((r, v)) => (r, v),
        None => (Rect::EMPTY, float_texture(device, "pc_operator_nosel", 1, 1).create_view(&Default::default())),
    };
    let i = |v: i32| v as u32;
    let params = words(&[
        f.size[0],
        f.size[1],
        f.offset[0],
        f.offset[1],
        i(f.area.x0),
        i(f.area.y0),
        i(f.area.x1),
        i(f.area.y1),
        i(sel_rect.x0),
        i(sel_rect.y0),
        sel_rect.width(),
        sel_rect.height(),
        u32::from(f.keep_alpha),
        f.levels.to_bits(),
        u32::from(f.has_alpha),
        match f.combine {
            Combine::Blur => 0,
            Combine::Unsharp { .. } => 1,
            Combine::HighPass => 2,
        },
        f.blur_offset[0],
        f.blur_offset[1],
        match f.combine {
            Combine::Unsharp { amount, .. } => amount.to_bits(),
            Combine::Blur | Combine::HighPass => 0,
        },
        match f.combine {
            Combine::Unsharp { threshold, .. } => threshold.to_bits(),
            Combine::Blur | Combine::HighPass => 0,
        },
    ]);
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pc_operator_finish"),
        contents: &params,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_operator_finish"),
        layout: &kit.finish_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(blurred) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&sel_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(out) },
            wgpu::BindGroupEntry { binding: 4, resource: params.as_entire_binding() },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_operator_finish"), timestamp_writes: None });
    pass.set_pipeline(&kit.finish);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(f.size[0].div_ceil(8), f.size[1].div_ceil(8), 1);
}
