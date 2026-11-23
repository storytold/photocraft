pub(super) use crate::content_sampling::{SampleLayers, composite_region};

use super::{Document, Footprint, LayerId, PixelFormat, Rect, Region, Result, Surface, Value, bad, flag, string, targets_pixels};

// ---------------------------------------------------------------------------------------------
// Sampling sources
// ---------------------------------------------------------------------------------------------

pub(super) fn sample_layers(p: &Value, cmd: &str) -> Result<SampleLayers> {
    match string(p, "sampleLayer", "current") {
        "current" => Ok(SampleLayers::Current),
        "currentAndBelow" => Ok(SampleLayers::CurrentAndBelow),
        "all" => Ok(SampleLayers::All),
        o => Err(bad(cmd, format!("unknown sampleLayer `{o}` (current|currentAndBelow|all)"))),
    }
}

/// Pixels to sample at `rect` (document coordinates, already offset to the source position).
pub(super) fn sample(pre: &Document, id: Option<LayerId>, surf: &Surface, which: SampleLayers, rect: Rect) -> Region {
    match which {
        SampleLayers::Current => Region::read(surf, rect),
        _ => composite_region(pre, id, which, rect, surf.format()),
    }
}

/// "Sample All Layers" for Spot Healing, Blur, Sharpen and Smudge (layer pixels only: a mask or
/// channel has nothing to sample from other layers).
pub(super) fn sample_all_layers(p: &Value) -> bool {
    flag(p, "sampleAllLayers", false) && targets_pixels(p)
}

/// Sample All Layers for the sequential-dab tools: each dab's effect runs on a copy of the visible
/// composite (kept across dabs, so a dab sees the earlier ones), and the result is laid onto the
/// layer under the dab. An empty layer above the image so picks up the blurred or smudged picture,
/// and pixels the effect didn't change look as before (they are what was already visible).
pub(super) struct AllLayers<'a> {
    pre: &'a Document,
    fmt: PixelFormat,
    visible: Option<Region>,
}

impl<'a> AllLayers<'a> {
    pub(super) fn new(pre: &'a Document, fmt: PixelFormat) -> Self {
        Self { pre, fmt, visible: None }
    }

    pub(super) fn dab(&mut self, work: &mut Region, fp: &Footprint, effect: impl FnOnce(&mut Region, &Footprint)) {
        let (pre, fmt) = (self.pre, self.fmt);
        let visible = self.visible.get_or_insert_with(|| composite_region(pre, None, SampleLayers::All, work.rect, fmt));
        if visible.rect != work.rect || visible.ch != work.ch {
            return effect(work, fp);
        }
        effect(visible, fp);
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                if fp.at(x, y) > 0.0 {
                    work.px_mut(x, y).copy_from_slice(visible.px(x, y));
                }
            }
        }
    }
}
