//! The layers above one root boundary, cached as three renders of them (#5973, P60).
//!
//! Above an edited layer, a stack whose passes are all affine in their backdrop (Normal,
//! Multiply and Screen blends of layers and their effects, see [`affine_in_backdrop`]) composites
//! any backdrop `x` to `F0 + (F1 − F0)·x.a + (F2 − F1)·x.rgb` in premultiplied colour, where F0,
//! F1 and F2 are that stack rendered over transparent, opaque black and opaque white. Once those
//! three are cached per chunk, a repeated edit below the boundary (an Opacity drag, hiding and
//! showing a layer) draws the layers up to the boundary and then one pass (`fs_suffix`) instead
//! of every layer above it. The result equals the full render in real arithmetic; in floats it
//! differs by rounding (about 1e-6), far within the GPU's tolerance against the CPU compositor.
//!
//! An entry stays valid while the document outside its layers and every root layer from the
//! boundary up compare equal to when it was built, like [`crate::prefix`]. Building costs three
//! renders of the stack above, so it happens only when the same layer is edited twice in a row.
use std::collections::HashMap;

use photocraft_doc::{BlendMode, Document, Layer};
use photocraft_geom::Rect;

use crate::plan::{self, Checkpoint, Kernel, Pass};

/// Fewer passes above the boundary than this aren't worth three renders and the memory.
const MIN_PASSES: usize = 8;

/// The backdrops the stack above is rendered over: transparent, opaque black, opaque white.
pub(crate) const BACKDROPS: [wgpu::Color; 3] =
    [wgpu::Color::TRANSPARENT, wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, wgpu::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }];

/// Flags whose passes aren't affine in the backdrop: knockout of a group (not the drop shadow's
/// own `F_KNOCKOUT` on `FxPaint`), a clipping base forced opaque, Lab mixing, quantising,
/// Blend If, text gamma, channel restrictions, HDR and the additive difference modes.
const NOT_AFFINE: u32 =
    plan::F_ATOP | plan::F_LAB | plan::F_QUANT | plan::F_BLEND_IF | plan::F_TEXT_GAMMA | plan::F_CHANNELS | plan::F_HDR | plan::F_ADD_DIFF | plan::F_FIRST;

/// Whether `p`'s output is an affine function of the backdrop slots it reads. Layer pixels,
/// masks, coverage and effect maps never depend on the backdrop; only the blend onto it does,
/// and Normal, Multiply and Screen are affine in premultiplied colour (`B(Cb, Cs)` is linear in
/// `Cb`). Anything else (adjustments, other modes, outside strokes, ...) is refused.
pub(crate) fn affine_in_backdrop(p: &Pass<'_>) -> bool {
    let knockout = if p.kernel == Kernel::FxPaint { 0 } else { plan::F_KNOCKOUT };
    if p.flags & (NOT_AFFINE | knockout) != 0 {
        return false;
    }
    let mode = matches!(p.mode, BlendMode::Normal | BlendMode::Multiply | BlendMode::Screen);
    match p.kernel {
        Kernel::Clear | Kernel::Content | Kernel::Mask | Kernel::CopyRect | Kernel::CopyFull | Kernel::FxInit => true,
        Kernel::LayerBlend | Kernel::Blend | Kernel::FxPaint | Kernel::FxMerge => mode,
        _ => false,
    }
}

/// F0, F1 and F2 of one chunk, in the accumulator format.
pub(crate) struct Entry {
    pub(crate) tex: [wgpu::Texture; 3],
    pub(crate) views: [wgpu::TextureView; 3],
    bytes: u64,
}

/// What a render does with the cache: chunks that hold an entry apply it at `at` (the backdrop
/// the stack above starts from); with `build`, chunks without one build it there first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Use {
    pub(crate) at: Checkpoint,
    pub(crate) build: bool,
    pub(crate) canvas: Rect,
}

#[derive(Default)]
pub(crate) struct Suffix {
    context: Option<Document>,
    /// The root layers of the previous render, to find what it changed.
    layers: Vec<Layer>,
    /// The lowest root layer the previous edit changed.
    last_edit: Option<usize>,
    /// The cached stack: root layers from `boundary` up as they were, and their pass count.
    boundary: usize,
    above: Vec<Layer>,
    passes: usize,
    chunks: HashMap<Rect, Entry>,
    bytes: u64,
    /// The backdrop of the chunk being applied (the slot can be the one the result goes to).
    backdrop: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl Suffix {
    pub(crate) fn clear(&mut self) {
        self.context = None;
        self.layers.clear();
        self.last_edit = None;
        self.drop_entries();
    }

    fn drop_entries(&mut self) {
        self.above.clear();
        self.passes = 0;
        self.chunks.clear();
        self.bytes = 0;
    }

    pub(crate) fn forget(&mut self, doc: photocraft_doc::DocId) {
        if self.doc() == Some(doc) {
            self.clear();
        }
    }

    pub(crate) fn doc(&self) -> Option<photocraft_doc::DocId> {
        self.context.as_ref().map(|d| d.id)
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Drops what `doc` invalidated and decides what this render does (see [`Use`]).
    /// `resumable` are the plan's checkpoints a batch run starts at; `enabled` is false when the
    /// composite caches are switched off.
    pub(crate) fn prepare(&mut self, doc: &Document, plan: &plan::Plan<'_>, resumable: &[Checkpoint], enabled: bool) -> Option<Use> {
        if !enabled {
            self.clear();
            return None;
        }
        let mut context = doc.clone();
        let layers = std::mem::take(&mut context.layers);
        let same_context = self.context.as_ref() == Some(&context);
        let changed = self
            .layers
            .iter()
            .zip(&layers)
            .position(|(a, b)| a != b)
            .or_else(|| (layers.len() != self.layers.len()).then_some(layers.len().min(self.layers.len())));
        let repeated = same_context && changed.is_some() && changed == self.last_edit;
        if changed.is_some() || !same_context {
            self.last_edit = changed.filter(|_| same_context);
        }
        let valid = same_context && self.passes > 0 && layers.get(self.boundary..) == Some(&self.above[..]);
        if !valid {
            self.drop_entries();
        }
        let canvas = doc.bounds();
        let n = plan.passes.len();
        let mut decision = valid
            .then(|| resumable.iter().find(|c| c.layer_end <= self.boundary && c.pass_end == n.saturating_sub(self.passes)).copied())
            .flatten()
            .map(|at| Use { at, build: repeated, canvas });
        // A repeated edit with nothing cached for it: the boundary just above the edited unit.
        if decision.is_none()
            && repeated
            && let Some(edit) = changed
            && let Some(at) = resumable.iter().find(|c| c.layer_end > edit).copied()
            && n.saturating_sub(at.pass_end) >= MIN_PASSES
            && plan.passes.get(at.pass_end..).is_some_and(|above| above.iter().all(affine_in_backdrop))
        {
            self.drop_entries();
            self.boundary = at.layer_end;
            self.above = layers.get(at.layer_end..).map(<[Layer]>::to_vec).unwrap_or_default();
            self.passes = n - at.pass_end;
            decision = Some(Use { at, build: true, canvas });
        }
        self.context = Some(context);
        self.layers = layers;
        decision
    }

    /// The cached entry of chunk `c`.
    pub(crate) fn entry(&self, c: Rect) -> Option<&Entry> {
        self.chunks.get(&c)
    }

    /// A new entry for chunk `c` when `room` bytes allow it (none is ever replaced to admit one).
    pub(crate) fn admit(&mut self, device: &wgpu::Device, c: Rect, format: wgpu::TextureFormat, room: u64) -> Option<&Entry> {
        let bpp = if format == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        let bytes = u64::from(c.width()).saturating_mul(u64::from(c.height())).saturating_mul(bpp).saturating_mul(3);
        if c.is_empty() || bytes > room {
            return None;
        }
        let tex = std::array::from_fn(|k| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(["pc_compose_suffix_clear", "pc_compose_suffix_black", "pc_compose_suffix_white"][k]),
                size: wgpu::Extent3d { width: c.width(), height: c.height(), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let views = std::array::from_fn(|k: usize| tex[k].create_view(&Default::default()));
        self.bytes = self.bytes.saturating_add(bytes);
        Some(self.chunks.entry(c).or_insert(Entry { tex, views, bytes }))
    }

    /// Forgets chunk `c`'s entry (its build didn't complete).
    pub(crate) fn remove(&mut self, c: Rect) {
        if let Some(e) = self.chunks.remove(&c) {
            self.bytes = self.bytes.saturating_sub(e.bytes);
        }
    }

    /// The backdrop texture, once [`Self::backdrop`] made it.
    pub(crate) fn backdrop_view(&self) -> Option<&(wgpu::Texture, wgpu::TextureView)> {
        self.backdrop.as_ref()
    }

    /// The texture a chunk's backdrop is copied to before the stack above is applied to it.
    pub(crate) fn backdrop(&mut self, device: &wgpu::Device, side: u32, format: wgpu::TextureFormat) -> &(wgpu::Texture, wgpu::TextureView) {
        if self.backdrop.as_ref().is_some_and(|(t, _)| t.width() != side || t.format() != format) {
            self.backdrop = None;
        }
        self.backdrop.get_or_insert_with(|| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pc_compose_suffix_backdrop"),
                size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            (t, v)
        })
    }
}
