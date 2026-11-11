//! Resolve and freeze a clone source without changing the sampled document.
use std::sync::Arc;

use crate::presets::clone_source::uses_slot;
use photocraft_cms::{Transform, TransformOptions};
use photocraft_color::ColorMode;

use super::sampling::{SampleLayers, composite_region, sample, sample_layers};
use super::{Document, EngineError, LayerId, PixelFormat, Rect, Region, Result, SampleType, Session, Surface, Value, bad, periodic, targets_pixels};

/// Frozen source-document and layer selection shared by preview and stroke sampling.
#[derive(Clone, Default)]
pub(super) struct CloneFrom {
    other: Option<(Arc<Document>, u64)>,
    layer: Option<LayerId>,
    transform: Option<Arc<Transform>>,
}

impl CloneFrom {
    /// Whether cached live pixels still refer to the same source snapshot and color transform.
    pub(super) fn unchanged(&self, other: &Self) -> bool {
        let document = match (&self.other, &other.other) {
            (Some((a, ar)), Some((b, br))) => ar == br && Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let colors = match (&self.transform, &other.transform) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        document && self.layer == other.layer && colors
    }

    /// Read a source region into the destination's native channels, optionally wrapping its canvas.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sample(
        &self,
        pre: &Document,
        target: Option<LayerId>,
        surf: &Surface,
        which: SampleLayers,
        rect: Rect,
        wrap: bool,
        cmd: &str,
    ) -> Result<Region> {
        let doc = self.other.as_ref().map_or(pre, |(doc, _)| doc.as_ref());
        let fmt = surf.format();
        if self.other.is_some() && fmt.mode != pre.mode {
            return Err(bad(cmd, "the destination layer color model does not match its document"));
        }
        let read = |rect| {
            let (region, from) = if self.other.is_some() && which != SampleLayers::Current {
                let from = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
                (composite_region(doc, self.layer, which, rect, from), from)
            } else {
                let sampled = match self.layer {
                    Some(id) => doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad(cmd, "the clone source layer no longer has pixels"))?,
                    None => surf,
                };
                if self.other.is_some() && sampled.format().mode != doc.mode {
                    return Err(bad(cmd, "the source layer color model does not match its document; choose All Layers"));
                }
                (sample(doc, target, sampled, which, rect), sampled.format())
            };
            convert_region(region, from, fmt, self.transform.as_deref(), cmd)
        };
        if wrap { periodic::sample_with(doc.bounds(), fmt.channels(), rect, cmd, read) } else { read(rect) }
    }
}

/// Convert only the sampled region; preserve alpha and skip identical-profile HDR conversion.
fn convert_region(region: Region, from: PixelFormat, to: PixelFormat, transform: Option<&Transform>, cmd: &str) -> Result<Region> {
    // A malformed document can contain a surface whose model disagrees with its ICC profile.
    // Check the native strides before the transform indexes them or its bounded scratch buffer.
    if transform.is_some_and(|t| t.inputs() != from.mode.color_channels() || t.outputs() != to.mode.color_channels() || t.outputs() > 16) {
        return Err(bad(cmd, "clone source color channels do not match the document profiles"));
    }
    if transform.is_none() && from.mode == to.mode && from.alpha == to.alpha {
        return Ok(region);
    }
    let mut out = Region::new(region.rect, to.channels());
    for (src, dst) in region.data.chunks_exact(from.channels()).zip(out.data.chunks_exact_mut(to.channels())) {
        if let Some(transform) = transform {
            let colors = to.mode.color_channels();
            let mut converted = [0.0; 16];
            transform.eval(src, &mut converted);
            dst[..colors].copy_from_slice(&converted[..colors]);
            if to.alpha {
                dst[colors] = if from.alpha { src.get(from.mode.color_channels()).copied().unwrap_or(1.0) } else { 1.0 };
            }
        } else if from.mode == to.mode {
            let colors = to.mode.color_channels();
            dst[..colors].copy_from_slice(&src[..colors]);
            if to.alpha {
                dst[colors] = if from.alpha { src.get(colors).copied().unwrap_or(1.0) } else { 1.0 };
            }
        } else {
            dst.copy_from_slice(&photocraft_raster::convert_pixel(&from, &to, src));
        }
    }
    Ok(out)
}

/// Resolve a picked slot or explicit local coordinates without anchoring or modifying a document.
pub(super) fn resolve(s: &Session, p: &Value, cmd: &str) -> Result<CloneFrom> {
    if !targets_pixels(p) {
        // A local mask/channel has no corresponding surface in another document. Refuse this
        // unsupported target instead of quietly cloning the destination's own mask/channel.
        let picked = uses_slot(p);
        if picked && s.presets.clone.active().document.is_some_and(|id| s.active().is_none_or(|st| st.doc.id != id)) {
            return Err(bad(cmd, "cross-document cloning requires a pixel-layer destination"));
        }
        return Ok(CloneFrom::default());
    }
    let which = sample_layers(p, cmd)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let slot = s.presets.clone.active();
    // Explicit coordinates are in the destination document. A picked slot carries its document.
    let picked = uses_slot(p);
    let doc_id = picked.then_some(slot.document).flatten().unwrap_or(active.doc.id);
    let st = s.documents().iter().find(|st| st.doc.id == doc_id).ok_or_else(|| bad(cmd, "the clone source document was closed; pick a new source"))?;
    let external = st.doc.id != active.doc.id;
    // Adobe documents matching document modes for Clone Stamp; Healing Brush additionally
    // permits Grayscale on either side. Profile conversion still applies to compatible pairs.
    let healing_gray = matches!(cmd, "paint.healingBrush" | "healing.preview") && [st.doc.mode, active.doc.mode].contains(&ColorMode::Grayscale);
    if external && st.doc.mode != active.doc.mode && !healing_gray {
        return Err(bad(cmd, "the source and destination color modes are incompatible with this tool; convert a document before sampling"));
    }
    let mut layer = None;
    if which == SampleLayers::Current || (external && which == SampleLayers::CurrentAndBelow) {
        let id = match p.get("sourceLayer") {
            Some(value) => Some(LayerId(value.as_u64().ok_or_else(|| bad(cmd, "sourceLayer must be a layer id"))?)),
            None if picked => slot.layer.map(LayerId),
            None => None,
        }
        .or(if external { st.active_layer } else { None });
        if let Some(id) = id {
            let source = st.doc.layer(id).ok_or_else(|| bad(cmd, "the clone source layer no longer exists; pick a new source"))?;
            if source.surface().is_some() || which != SampleLayers::Current {
                layer = Some(id);
            } else if external {
                return Err(bad(cmd, "the clone source layer has no pixels; choose All Layers or pick a pixel layer"));
            }
        } else if external && which == SampleLayers::Current {
            return Err(bad(cmd, "the clone source has no pixel layer"));
        }
    }
    let transform = if external { color_transform(s, &st.doc, which, cmd)? } else { None };
    Ok(CloneFrom { other: external.then(|| (st.doc.clone(), st.revision)), layer, transform })
}

/// Choose the sampled pixel/composite profile and cache its conversion to the destination profile.
fn color_transform(s: &Session, doc: &Document, which: SampleLayers, cmd: &str) -> Result<Option<Arc<Transform>>> {
    let source = if which == SampleLayers::Current { crate::color_cmds::document_profile(doc) } else { crate::color_cmds::composite_profile(doc) };
    let target = s.active().ok_or(EngineError::NoDocument)?;
    let destination = crate::color_cmds::document_profile(&target.doc);
    let inputs = if which == SampleLayers::Current { doc.mode.color_channels() } else { 3 };
    if source.channels() != inputs || destination.channels() != target.doc.mode.color_channels() {
        return Err(bad(cmd, "cross-document cloning does not support this color model"));
    }
    if source.same_bytes(&destination) {
        return Ok(None);
    }
    let opts = TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..Default::default() };
    photocraft_cms::transform::cached(&source, &destination, opts).map(Some).map_err(|e| bad(cmd, format!("cannot convert clone source colors: {e}")))
}
