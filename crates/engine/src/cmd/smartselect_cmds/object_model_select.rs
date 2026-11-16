//! Object Selection on a segmentation model (SAM 2.1 through the frontend's [`ObjectModel`]).
//!
//! The model sees a downscaled crop of the picture (at most [`MODEL_LONG_SIDE`] pixels on its long
//! side) and answers with a mask over that crop. The mask is upsampled to the crop and, when the
//! crop was downscaled, its edge is refined against the full-resolution pixels with a guided
//! filter, so the selection follows the real edge rather than the model's coarse one.

use std::sync::Arc;

use photocraft_algo::matting;
use photocraft_algo::segment::RgbImage;

use super::{EngineError, Rect, Region, Result, Session, Value, apply, b, bad, json, mode, object_edge, trimmed, with_doc_sampler};
use crate::jobs::JobCtx;
use crate::object_model::{ObjectModel, SegmentRequest};

/// Longest side, in pixels, of the picture sent to the model.
pub(super) const MODEL_LONG_SIDE: u32 = 2048;
/// Most pixels of a crop the model path accepts, before any downscaling.
const MODEL_MAX_PIXELS: u64 = 1 << 27;
/// Crops above this many pixels skip the full-resolution edge refinement (its working memory is
/// dozens of bytes a pixel).
const GUIDE_MAX_PIXELS: u64 = 1 << 26;
/// The crop around a rectangle extends by at least this many pixels on every side.
const CROP_MARGIN_MIN: u32 = 16;

/// What the user pointed at, in document pixels.
#[derive(Clone, Copy, Debug)]
pub(super) enum Prompt {
    Box(Rect),
    Click(f32, f32),
}

/// Which part of the document goes to the model, and what is asked of it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Prepared {
    pub crop: Rect,
    /// Integer downscale factor from the crop to the picture sent (1 = none).
    pub step: usize,
    pub area: Option<[f32; 4]>,
    pub clicks: Vec<(f32, f32, bool)>,
}

/// The crop and prompts for `prompt` on a document of bounds `canvas`. `Ok(None)`: the prompt lies
/// outside the canvas, so there is nothing to find.
pub(super) fn prepare(canvas: Rect, prompt: Prompt) -> Result<Option<Prepared>> {
    let (crop, area, clicks) = match prompt {
        Prompt::Box(rect) => {
            let r = rect.intersect(&canvas);
            if r.is_empty() {
                return Ok(None);
            }
            let grow = |len: u32| i32::try_from(u64::from(len) * 15 / 100).unwrap_or(i32::MAX).max(i32::try_from(CROP_MARGIN_MIN).unwrap_or(16));
            let (gx, gy) = (grow(r.width()), grow(r.height()));
            let crop = Rect::new(r.x0.saturating_sub(gx), r.y0.saturating_sub(gy), r.x1.saturating_add(gx), r.y1.saturating_add(gy)).intersect(&canvas);
            if crop.is_empty() {
                return Ok(None);
            }
            let (cw, ch) = (crop.width() as f32, crop.height() as f32);
            let frac = |v: i32, o: i32, n: f32| ((i64::from(v) - i64::from(o)) as f32 / n).clamp(0.0, 1.0);
            let area = [frac(r.x0, crop.x0, cw), frac(r.y0, crop.y0, ch), frac(r.x1, crop.x0, cw), frac(r.y1, crop.y0, ch)];
            (crop, Some(area), Vec::new())
        }
        Prompt::Click(x, y) => {
            if !x.is_finite() || !y.is_finite() || !canvas.contains(x.floor() as i32, y.floor() as i32) {
                return Ok(None);
            }
            let crop = canvas;
            let fx = ((x - crop.x0 as f32) / crop.width() as f32).clamp(0.0, 1.0);
            let fy = ((y - crop.y0 as f32) / crop.height() as f32).clamp(0.0, 1.0);
            (crop, None, vec![(fx, fy, true)])
        }
    };
    let pixels = u64::from(crop.width()) * u64::from(crop.height());
    if pixels > MODEL_MAX_PIXELS {
        return Err(bad("select.object", &format!("the area to select in is too large for the model ({pixels} pixels); select a smaller rectangle")));
    }
    let long = crop.width().max(crop.height());
    let step = usize::try_from(long.div_ceil(MODEL_LONG_SIDE)).unwrap_or(1).max(1);
    Ok(Some(Prepared { crop, step, area, clicks }))
}

/// `mask` (`sw` × `sh`, 0..=255) resampled bilinearly to `w` × `h`, as coverage in `0..=1`.
/// `mask` must hold `sw * sh` bytes; anything missing reads as 0.
pub(super) fn upsample(mask: &[u8], sw: usize, sh: usize, w: usize, h: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(w.saturating_mul(h));
    if sw == 0 || sh == 0 || w == 0 || h == 0 {
        out.resize(w.saturating_mul(h), 0.0);
        return out;
    }
    let at = |x: usize, y: usize| mask.get(y * sw + x).map_or(0.0, |v| f32::from(*v) / 255.0);
    let axis = |i: usize, small: usize, big: usize| -> (usize, usize, f32) {
        let f = ((i as f32 + 0.5) * small as f32 / big as f32 - 0.5).max(0.0);
        let i0 = (f.floor() as usize).min(small - 1);
        (i0, (i0 + 1).min(small - 1), (f - i0 as f32).clamp(0.0, 1.0))
    };
    let xs: Vec<(usize, usize, f32)> = (0..w).map(|x| axis(x, sw, w)).collect();
    for y in 0..h {
        let (y0, y1, ty) = axis(y, sh, h);
        for &(x0, x1, tx) in &xs {
            let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
            let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
            out.push(top * (1.0 - ty) + bottom * ty);
        }
    }
    out
}

/// Straight RGB bytes of `img`.
fn rgb_bytes(img: &RgbImage) -> Vec<u8> {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    img.px.iter().flat_map(|p| p.map(q)).collect()
}

/// The model's answer as a region over `crop`: `mask` (`sw` × `sh`) upsampled, its edge refined
/// against `guide` (the crop at full resolution) when there is one, binarized when `hard`, and
/// cut down to the selected pixels. `None` when nothing is selected.
pub(super) fn region_from_mask(crop: Rect, mask: &[u8], sw: usize, sh: usize, guide: Option<&RgbImage>, hard: bool) -> Option<Region> {
    let (w, h) = (crop.width() as usize, crop.height() as usize);
    let mut cover = upsample(mask, sw, sh, w, h);
    if let Some(g) = guide.filter(|g| g.w == w && g.h == h && g.px.len() == cover.len() && (sw != w || sh != h)) {
        let scale = (w.max(h) as f32 / sw.max(sh).max(1) as f32).max(1.0);
        let radius = (2.0 * scale).round().max(2.0) as usize;
        cover = matting::guided_filter_color(g, &cover, radius, 1e-3);
    }
    let bytes: Vec<u8> = cover
        .iter()
        .map(|v| {
            let level = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if hard { if level >= 128 { 255 } else { 0 } } else { level }
        })
        .collect();
    if !bytes.iter().any(|v| *v > 0) {
        return None;
    }
    let region = trimmed(Region { bbox: crop, mask: bytes }, w);
    (!region.mask.is_empty()).then_some(region)
}

/// Object Selection on the model: runs it as a job so a long model run stays cancellable.
pub(super) fn select(s: &mut Session, p: &Value, prompt: Prompt, all_layers: bool, model: Arc<dyn ObjectModel>, method: &'static str) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let Some(prep) = prepare(canvas, prompt)? else {
        let mut out = apply(s, "Object Selection", None, mode(p, "replace"))?;
        tag(&mut out, method, None);
        return Ok(out);
    };
    let doc = Arc::clone(&d.doc);
    let (doc_id, layer) = (doc.id, d.active_layer);
    let hard = b(p, "hardEdge", true);
    let params = p.clone();
    let work = move |ctx: &JobCtx| -> Result<Option<Region>> {
        ctx.check()?;
        let crop = prep.crop;
        let pixels = u64::from(crop.width()) * u64::from(crop.height());
        let (small, guide) = with_doc_sampler(&doc, layer, all_layers, |smp, _| {
            let small = smp.rgb_scaled(crop, prep.step);
            let guide = (prep.step > 1 && pixels <= GUIDE_MAX_PIXELS).then(|| smp.rgb(crop));
            (small, guide)
        });
        ctx.check()?;
        let (sw, sh) = (small.w, small.h);
        if sw == 0 || sh == 0 || small.px.len() != sw.saturating_mul(sh) {
            return Err(EngineError::Other("could not read the picture for the model".into()));
        }
        let (Ok(width), Ok(height)) = (u32::try_from(sw), u32::try_from(sh)) else {
            return Err(EngineError::Other("the picture for the model is too large".into()));
        };
        let req = SegmentRequest { rgb: rgb_bytes(&small), width, height, area: prep.area, clicks: prep.clicks.clone() };
        drop(small);
        let answer = model.segment(&req, &|| ctx.cancelled());
        let answer = match answer {
            Ok(a) => a,
            Err(_) if ctx.cancelled() => return Err(EngineError::Cancelled),
            Err(msg) => return Err(EngineError::Other(msg)),
        };
        ctx.check()?;
        if answer.mask.len() != sw.saturating_mul(sh) {
            return Err(EngineError::Other("the model's mask has the wrong size".into()));
        }
        if !answer.matched {
            return Ok(None);
        }
        Ok(region_from_mask(crop, &answer.mask, sw, sh, guide.as_ref(), hard))
    };
    let apply_found = move |s: &mut Session, region: Option<Region>| -> Result<Value> {
        if s.active().map(|d| d.doc.id) != Some(doc_id) {
            return Err(EngineError::Other("the document changed while the model ran; nothing was selected".into()));
        }
        let region = object_edge(canvas, &params, region.as_ref());
        let mut out = apply(s, "Object Selection", region.as_deref(), mode(&params, "replace"))?;
        tag(&mut out, method, None);
        Ok(out)
    };
    crate::jobs::run(s, "Object Selection", false, work, apply_found)
}

/// Adds which method selected (and why the model was passed over, if it was) to a result.
pub(super) fn tag(out: &mut Value, method: &str, unavailable: Option<&str>) {
    if let Some(o) = out.as_object_mut() {
        o.insert("method".into(), json!(method));
        if let Some(why) = unavailable {
            o.insert("modelUnavailable".into(), json!(why));
        }
    }
}
