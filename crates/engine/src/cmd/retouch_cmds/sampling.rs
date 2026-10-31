use super::{
    Document, Footprint, LayerId, PixelFormat, Rect, Region, Result, SampleType, Stroke, Surface, Value, WorkTiles, bad, flag, from_rgba_into, string,
    targets_pixels,
};

// ---------------------------------------------------------------------------------------------
// Sampling sources
// ---------------------------------------------------------------------------------------------

/// Which pixels Clone Stamp / Healing Brush sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleLayers {
    Current,
    CurrentAndBelow,
    All,
}

pub(super) fn sample_layers(p: &Value, cmd: &str) -> Result<SampleLayers> {
    match string(p, "sampleLayer", "current") {
        "current" => Ok(SampleLayers::Current),
        "currentAndBelow" => Ok(SampleLayers::CurrentAndBelow),
        "all" => Ok(SampleLayers::All),
        o => Err(bad(cmd, format!("unknown sampleLayer `{o}` (current|currentAndBelow|all)"))),
    }
}

/// Composite pixels of `rect` in `fmt` (native channels): the whole document, or the target layer
/// and everything below it.
pub(crate) fn composite_region(pre: &Document, id: Option<LayerId>, which: SampleLayers, rect: Rect, fmt: PixelFormat) -> Region {
    let mut doc = pre.clone();
    if which == SampleLayers::CurrentAndBelow
        && let Some(id) = id
    {
        // Hide everything composited after (above) the target in the bottom-to-top walk.
        let walk: Vec<(Vec<usize>, LayerId)> = pre.walk().into_iter().map(|(p, _, l)| (p, l.id)).collect();
        if let Some(pos) = walk.iter().position(|(_, l)| *l == id) {
            for (path, _) in &walk[pos + 1..] {
                if let Some(l) = doc.layer_at_mut(path) {
                    l.visible = false;
                }
            }
        }
    }
    let buf = photocraft_compose::render(&doc, rect);
    let n = fmt.channels();
    let mut out = Region::new(rect, n);
    let mut enc = [0.0f32; 8];
    for (i, px) in buf.px.iter().enumerate() {
        from_rgba_into(&fmt, *px, &mut enc);
        out.data[i * n..(i + 1) * n].copy_from_slice(&enc[..n]);
    }
    out
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
    /// A stroke wrapped around the canvas (View › Pattern Preview): the composite at full
    /// precision under everything the stroke reads, and the dabs' edits of it, kept once per
    /// pixel of the canvas as the layer's working pixels are.
    wrapped: Option<(Surface, WorkTiles)>,
}

impl<'a> AllLayers<'a> {
    pub(super) fn new(pre: &'a Document, fmt: PixelFormat) -> Self {
        Self { pre, fmt, visible: None, wrapped: None }
    }

    /// Sample for `stroke` wrapped around `wrap`, whose dabs read `halo` pixels around them.
    pub(super) fn wrapped(mut self, wrap: Option<Rect>, stroke: &Stroke, halo: i32) -> Self {
        let Some(tile) = wrap else { return self };
        let ctx = photocraft_paint::BrushContext::new(&stroke.brush);
        let reach = photocraft_paint::dabs(stroke).iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false))).inflate(halo);
        let area = photocraft_paint::wrap::fold_area(reach, tile);
        let mut visible = Surface::new(PixelFormat { sample: SampleType::F32, ..self.fmt });
        if !area.is_empty() {
            visible.write_region(area, &composite_region(self.pre, None, SampleLayers::All, area, self.fmt).data);
        }
        self.wrapped = Some((visible, WorkTiles::new(self.fmt.channels()).wrapped(Some(tile))));
        self
    }

    pub(super) fn dab(&mut self, work: &mut Region, fp: &Footprint, effect: impl FnOnce(&mut Region, &Footprint)) {
        if let Some((visible, edits)) = self.wrapped.as_mut() {
            let mut shown = edits.read(visible, work.rect);
            effect(&mut shown, fp);
            lay(work, &shown, fp);
            edits.write_dab(visible, &shown, fp);
            return;
        }
        let (pre, fmt) = (self.pre, self.fmt);
        let visible = self.visible.get_or_insert_with(|| composite_region(pre, None, SampleLayers::All, work.rect, fmt));
        if visible.rect != work.rect || visible.ch != work.ch {
            return effect(work, fp);
        }
        effect(visible, fp);
        lay(work, visible, fp);
    }
}

/// Lay what a dab did to the visible pixels onto the layer's working pixels under it.
fn lay(work: &mut Region, visible: &Region, fp: &Footprint) {
    let r = fp.rect.intersect(&work.rect);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            if fp.at(x, y) > 0.0 {
                work.px_mut(x, y).copy_from_slice(visible.px(x, y));
            }
        }
    }
}
