//! Layer Style › Blending Options › Advanced Blending in the CPU compositor (the oracle):
//!
//! - **Knockout.** Inside the layer's shape the backdrop is replaced by the knockout target
//!   before the layer composites: for Shallow the bottom of the enclosing group (the backdrop the
//!   group's stack started from: transparent in an isolated group, the layers beneath in a
//!   pass-through one) or of the clipping group (the base layer's content); for Deep, and for
//!   Shallow at the top level, the Background layer (transparency without one). Isolated groups
//!   and grouped clipping layers stop a deep knockout at their bottom, as Photoshop renders it
//!   (see [`Scope`]); pass-through groups let it through. Fill opacity sets how much of the layer
//!   shows over the knocked-out area, layer opacity how strong the whole knockout is.
//! - **Blend Clipped Layers as Group off.** The base composites alone; each clipped layer then
//!   composites onto the result in its own mode, kept inside the base's shape (its alpha with
//!   masks, times its opacity).
//! - **Transparency Shapes Layer off.** The layer's effects and knockout cover the whole layer
//!   (its masks still apply) instead of following its transparency. In the special eight
//!   ([`crate::psblend::fill_is_special`]) the layer's own transparency then blends as Fill does
//!   (`blend_layer_into` in `composite.rs`; measured in Photoshop, #5692).
//! - **Layer / Vector Mask Hides Effects.** The effects are built from the layer without that
//!   mask, and the mask is applied to the finished layer and effects.
//!
//! Blend Interior Effects as Group lives in [`crate::effects`], where interior effects are
//! painted. Every path here is skipped for layers at Photoshop's defaults, so documents that
//! don't use these options render exactly as before at no extra cost.
//!
//! Measured on the psd-tools knockout files (deep/shallow, nested, pass-through, no Background,
//! groups with knockout). Not yet measured against Photoshop: the clip of individually blended
//! clipped layers (base alpha × opacity, ignoring its fill), Transparency Shapes Layer off, and the
//! masks hiding effects.

use std::borrow::Cow;

use photocraft_doc::{Knockout, Layer, LayerContent};
use photocraft_geom::Rect;

use crate::content::render_content_with;
use crate::extents::recycle_mask;
use crate::{Buffer, BufferAlloc, Ctx, effects, mask_vals};

/// Where a layer stack sits, for knockouts: at the document's top level, and/or inside an
/// isolated (non-pass-through) group. Photoshop stops a deep knockout at the bottom of the
/// innermost isolated group around it (psd-tools knockout-deep-nested renders exactly like
/// knockout-shallow-nested), while pass-through groups let it reach the Background
/// (knockout-deep-nested-pt renders like knockout-deep-normal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    /// The document's top-level stack: a shallow knockout there reaches the Background.
    pub root: bool,
    /// Inside an isolated group: deep knockouts stop at its (transparent) bottom.
    pub isolated: bool,
}

impl Scope {
    pub const ROOT: Scope = Scope { root: true, isolated: false };
    /// An isolated group's own stack (it starts from a transparent buffer).
    pub const ISOLATED: Scope = Scope { root: false, isolated: true };

    /// The stack of a pass-through group inside this one.
    pub fn pass_through(self) -> Scope {
        Scope { root: false, isolated: self.isolated }
    }
}

/// Premultiplied linear interpolation between two straight-alpha pixels.
#[inline]
fn mix(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    let alpha = a[3] + (b[3] - a[3]) * k;
    if alpha <= 0.0 {
        return [0.0; 4];
    }
    let c = |c: usize| ((a[c] * a[3] + (b[c] * b[3] - a[c] * a[3]) * k) / alpha).clamp(0.0, 1.0);
    [c(0), c(1), c(2), alpha.min(1.0)]
}

/// Whether any Advanced Blending option changes how `layer` (with its clipping group `clipped`)
/// composites. The GPU compositor sends such documents to this CPU path.
pub fn advanced_active(layer: &Layer, clipped: &[Layer]) -> bool {
    let a = &layer.advanced;
    if a.is_default() {
        return false;
    }
    let fx = effects::has_effects(layer);
    // Transparency Shapes Layer off changes the special eight's blend itself (#5692).
    let shapeless_blend = crate::psblend::fill_is_special(layer.blend) && !matches!(layer.content, LayerContent::Adjustment(_));
    a.knockout != Knockout::None
        || clips_individually(layer, clipped)
        || (a.blend_interior && fx)
        || (!a.transparency_shapes && (fx || shapeless_blend))
        || hides_effects(layer)
}

/// Whether the layer's effects (or knockout) cover the whole layer: Transparency Shapes Layer
/// off on a layer that has either.
pub fn shapeless(layer: &Layer) -> bool {
    !layer.advanced.transparency_shapes && (layer.advanced.knockout != Knockout::None || effects::has_effects(layer))
}

/// Whether `l`, the bottom layer of a stack, is the Background: identified by its locks and kind,
/// never by its (localized) name ([`photocraft_doc::Document::background`]).
pub fn is_background(l: &Layer) -> bool {
    l.has_background_traits()
}

fn deep_target_with<A: BufferAlloc>(cx: &Ctx, rect: Rect, alloc: &A) -> Result<Buffer, A::Error> {
    let Some(doc) = cx.doc else { return alloc.filled(rect, [0.0; 4]) };
    let mut b = crate::multichannel::backdrop_with(doc, rect, alloc)?;
    if let Some(bg) = doc.layers.first().filter(|l| l.visible && is_background(l)) {
        crate::composite::composite_layer_with(bg, &[], &mut b, cx, None, Scope::ROOT, alloc)?;
    }
    Ok(b)
}

pub(crate) fn knockout_target_with<'b, A: BufferAlloc>(
    layer: &Layer,
    floor: Option<&'b Buffer>,
    rect: Rect,
    cx: &Ctx,
    scope: Scope,
    alloc: &A,
) -> Result<Option<Cow<'b, Buffer>>, A::Error> {
    let deep = || if scope.isolated { alloc.filled(rect, [0.0; 4]) } else { deep_target_with(cx, rect, alloc) };
    match layer.advanced.knockout {
        Knockout::None => Ok(None),
        Knockout::Shallow => match floor {
            Some(floor) => Ok(Some(Cow::Borrowed(floor))),
            None => Ok(Some(Cow::Owned(deep()?))),
        },
        Knockout::Deep => Ok(Some(Cow::Owned(deep()?))),
    }
}

/// Whether `layers` holds a visible unclipped shallow knockout (its stack then keeps a copy of
/// its starting backdrop).
pub(crate) fn needs_floor(layers: &[Layer]) -> bool {
    layers.iter().any(|l| l.visible && !l.clipped && l.advanced.knockout == Knockout::Shallow)
}

fn knockout_shape_with<A: BufferAlloc>(layer: &Layer, rect: Rect, cx: &Ctx, alloc: &A) -> Result<Vec<f32>, A::Error> {
    let n = rect.width() as usize * rect.height() as usize;
    let masks = || mask_vals(layer, rect, cx).unwrap_or_else(|| vec![1.0; n]);
    if !layer.advanced.transparency_shapes {
        return Ok(masks());
    }
    match render_content_with(layer, rect, cx, alloc)? {
        Some(b) => Ok(b.px.iter().map(|p| p[3]).collect()),
        None => Ok(masks()),
    }
}

/// Composite `layer` with `draw` as a knockout onto `target`: inside its shape the backdrop
/// becomes the target, then the layer draws (at its fill opacity), and layer opacity mixes the
/// result with the untouched backdrop. Compositing at opacity `o` is linear in `o`
/// (premultiplied), so with `B` the backdrop, `B'` the knocked-out one and `R'` what `draw`
/// produced at the layer's opacity, the result is `R' + (1 − o)(B − B')`.
///
/// `clip_alpha` keeps alpha from growing (clipped layers stay inside their base's alpha; a
/// knockout to transparency may still lower it).
pub(crate) fn knockout_with<A: BufferAlloc>(
    layer: &Layer,
    backdrop: &mut Buffer,
    target: &Buffer,
    clip_alpha: bool,
    cx: &Ctx,
    alloc: &A,
    draw: impl FnOnce(&mut Buffer) -> Result<(), A::Error>,
) -> Result<(), A::Error> {
    // Coverage knocks out proportionally. (Photoshop knocks out a little more at anti-aliased
    // shape edges: psd-tools knockout-isolated-groups differs on 0.45 % of its pixels, along
    // ellipse edges; a binary shape matched worse.)
    let shape = knockout_shape_with(layer, backdrop.rect, cx, alloc)?;
    if target.rect != backdrop.rect || shape.len() != backdrop.px.len() {
        // Never reached (targets are built over the backdrop's rect); compositing without the
        // knockout beats a wrong pixel or a panic.
        recycle_mask(Some(shape));
        draw(backdrop)?;
        return Ok(());
    }
    let before = alloc.clone(backdrop)?;
    for ((p, t), k) in backdrop.px.iter_mut().zip(&target.px).zip(&shape) {
        if *k > 0.0 {
            let a = p[3];
            *p = mix(*p, *t, k.min(1.0));
            if clip_alpha {
                p[3] = p[3].min(a);
            }
        }
    }
    let knocked = if layer.opacity < 1.0 { Some(alloc.clone(backdrop)?) } else { None };
    draw(backdrop)?;
    let Some(knocked) = knocked else {
        recycle_mask(Some(shape));
        return Ok(());
    };
    let rest = 1.0 - layer.opacity.clamp(0.0, 1.0);
    for (((p, b), k), s) in backdrop.px.iter_mut().zip(&before.px).zip(&knocked.px).zip(&shape) {
        if *s <= 0.0 {
            continue;
        }
        let mut pm = [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
        for c in 0..3 {
            pm[c] += rest * (b[c] * b[3] - k[c] * k[3]);
        }
        pm[3] += rest * (b[3] - k[3]);
        let a = pm[3].clamp(0.0, 1.0);
        *p = if a > 0.0 { [(pm[0] / a).clamp(0.0, 1.0), (pm[1] / a).clamp(0.0, 1.0), (pm[2] / a).clamp(0.0, 1.0), a] } else { [0.0; 4] };
    }
    recycle_mask(Some(shape));
    Ok(())
}

pub(crate) fn composite_clipped_with<A: BufferAlloc>(clipped: &[Layer], base: &mut Buffer, cx: &Ctx, alloc: &A) -> Result<(), A::Error> {
    let floor = if clipped.iter().any(|c| c.visible && c.advanced.knockout != Knockout::None) { Some(alloc.clone(base)?) } else { None };
    for c in clipped.iter().filter(|c| c.visible) {
        match &floor {
            Some(f) if c.advanced.knockout != Knockout::None => {
                knockout_with(c, base, f, true, cx, alloc, |b| crate::composite::composite_atop_with(c, b, cx, alloc))?
            }
            _ => crate::composite::composite_atop_with(c, base, cx, alloc)?,
        }
    }
    Ok(())
}

/// Whether `layer`'s clipped layers blend one by one (Blend Clipped Layers as Group off).
/// Adjustment layers keep the grouped path (they have no pixels of their own to clip to).
pub(crate) fn clips_individually(layer: &Layer, clipped: &[Layer]) -> bool {
    !layer.advanced.blend_clipped && !matches!(layer.content, LayerContent::Adjustment(_)) && clipped.iter().any(|c| c.visible)
}

/// Blend Clipped Layers as Group off: the base alone (with its knockout `ko`), then each visible
/// clipped layer composited in its own mode onto the result, inside the base's shape (its alpha
/// with masks, times its opacity). A clipped layer's shallow knockout stops at the backdrop
/// beneath the base (the bottom of the clipping group).
pub(crate) fn composite_clipped_individually_with<A: BufferAlloc>(
    layer: &Layer,
    clipped: &[Layer],
    backdrop: &mut Buffer,
    cx: &Ctx,
    ko: Option<&Buffer>,
    scope: Scope,
    alloc: &A,
) -> Result<(), A::Error> {
    let rect = backdrop.rect;
    let n = backdrop.px.len();
    let floor = if clipped.iter().any(|c| c.visible && c.advanced.knockout == Knockout::Shallow) { Some(alloc.clone(backdrop)?) } else { None };
    crate::composite::composite_layer_with(layer, &[], backdrop, cx, ko, scope, alloc)?;
    let op = layer.opacity.clamp(0.0, 1.0);
    let clip = match render_content_with(layer, rect, cx, alloc)? {
        Some(b) => b.px.iter().map(|p| p[3] * op).collect::<Vec<_>>(),
        None => {
            let mv = mask_vals(layer, rect, cx);
            let clip = mv.as_ref().map_or_else(|| vec![op; n], |m| m.iter().map(|m| m * op).collect());
            recycle_mask(mv);
            clip
        }
    };
    if clip.len() != n {
        return Ok(());
    }
    for c in clipped.iter().filter(|c| c.visible) {
        let before = alloc.clone(backdrop)?;
        let t = knockout_target_with(c, floor.as_ref(), rect, cx, Scope { root: false, ..scope }, alloc)?;
        crate::composite::composite_layer_with(c, &[], backdrop, cx, t.as_deref(), scope, alloc)?;
        for ((p, b), k) in backdrop.px.iter_mut().zip(&before.px).zip(&clip) {
            if *k < 1.0 {
                *p = mix(*b, *p, k.max(0.0));
            }
        }
    }
    Ok(())
}

/// Whether a mask of `layer` hides its effects instead of shaping them (Layer / Vector Mask
/// Hides Effects on an enabled mask of a layer with effects).
pub(crate) fn hides_effects(layer: &Layer) -> bool {
    let a = &layer.advanced;
    effects::has_effects(layer)
        && ((a.layer_mask_hides_effects && layer.mask.as_ref().is_some_and(|m| m.enabled))
            || (a.vector_mask_hides_effects && layer.vector_mask.as_ref().is_some_and(|m| m.enabled)))
}

/// The layer whose content and effect maps are composited: `layer` itself, or (when a mask hides
/// its effects) a copy without that mask. Pixels are shared copy-on-write, so the copy costs a
/// tile table, and its effect maps cache under their own key.
pub(crate) fn effects_source(layer: &Layer) -> Cow<'_, Layer> {
    if !hides_effects(layer) {
        return Cow::Borrowed(layer);
    }
    let mut l = layer.clone();
    if l.advanced.layer_mask_hides_effects {
        l.mask = None;
    }
    if l.advanced.vector_mask_hides_effects {
        l.vector_mask = None;
    }
    Cow::Owned(l)
}

/// The masks that hide `layer`'s effects over `rect` (row-major; `None` when none do).
pub(crate) fn hiding_mask(layer: &Layer, rect: Rect, cx: &Ctx) -> Option<Vec<f32>> {
    if !hides_effects(layer) {
        return None;
    }
    let a = &layer.advanced;
    let mut only = Layer::new(layer.name.clone(), LayerContent::Raster(photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA32F)));
    only.id = layer.id;
    only.mask = layer.mask.clone().filter(|_| a.layer_mask_hides_effects);
    only.vector_mask = layer.vector_mask.clone().filter(|_| a.vector_mask_hides_effects);
    mask_vals(&only, rect, cx)
}

/// Mix `out` back towards `before` where `mask` hides the layer: `out ← mix(before, out, mask)`.
pub(crate) fn apply_hiding_mask(out: &mut Buffer, before: &Buffer, mask: &[f32]) {
    for ((p, b), k) in out.px.iter_mut().zip(&before.px).zip(mask) {
        if *k < 1.0 {
            *p = mix(*b, *p, k.max(0.0));
        }
    }
}
