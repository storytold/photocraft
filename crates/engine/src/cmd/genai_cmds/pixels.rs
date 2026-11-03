//! Generative Fill's pixels: the area sent around the selection, the picture and mask sent, and
//! a result matched to the photo around it and placed back in document coordinates.
//!
//! Pixel interchange follows #730's approach (composite through the document's profile to sRGB,
//! and back into the document's mode and depth on the way in).

use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::Serialize;

use crate::{EngineError, Result};

/// Largest result accepted, per side.
const RESULT_SIDE: u32 = 8192;

/// The shapes image models most often return (square, 3:2 and 2:3). The area sent is grown to the
/// nearest of them, so a model that returns one of these shapes isn't stretched when its result
/// is scaled back. No particular model is assumed: a model that keeps the shape it was sent gets
/// it unchanged.
const SHAPES: [(u32, u32); 3] = [(1, 1), (3, 2), (2, 3)];

pub(super) fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

/// The entry of `ratios` (width, height) closest to `w`×`h`, compared in log space so 2:1 and
/// 1:2 are equally far from 1:1.
pub(super) fn nearest_ratio(w: u32, h: u32, ratios: &[(u32, u32)]) -> Option<(u32, u32)> {
    let target = (f64::from(w.max(1)) / f64::from(h.max(1))).ln();
    ratios.iter().copied().filter(|&(a, b)| a > 0 && b > 0).min_by(|&(a, b), &(c, d)| {
        let da = ((f64::from(a) / f64::from(b)).ln() - target).abs();
        let dc = ((f64::from(c) / f64::from(d)).ln() - target).abs();
        da.total_cmp(&dc)
    })
}

/// The shape the area around a `w`×`h` selection is grown to.
pub(super) fn request_shape(w: u32, h: u32) -> (u32, u32) {
    nearest_ratio(w, h, &SHAPES).unwrap_or((w, h))
}

/// The area sent: the selection's bounds with a margin of context around them, grown toward the
/// shape asked for ([`request_shape`]) so the result isn't stretched, inside the canvas.
pub(super) fn context_area(selected: Rect, canvas: Rect, aspect: impl Fn(u32, u32) -> (u32, u32)) -> Rect {
    let side = selected.width().max(selected.height());
    // The selection's own size on each side, so the model can read a pattern or trend that runs
    // through it (a row of panels stepping down) and not just what touches its edge. Half the
    // size was tried first: the model saw two panels of seven and carried the pattern under the
    // selection straight up instead of continuing the row's curve. Twice the size fixed that but
    // left the selection a fifth of an area models return at about 1.2K, so a small selection
    // came back as a few hundred blurry pixels.
    let margin = side.clamp(64, 2048) as i32;
    let r = Rect::new(selected.x0 - margin, selected.y0 - margin, selected.x1.saturating_add(margin), selected.y1.saturating_add(margin)).intersect(&canvas);
    let (a, b) = aspect(r.width(), r.height());
    if a == 0 || b == 0 || r.is_empty() {
        return r;
    }
    let (w, h) = (u64::from(r.width()), u64::from(r.height()));
    let (want_w, want_h) =
        if w * u64::from(b) < h * u64::from(a) { ((h * u64::from(a)).div_ceil(u64::from(b)), h) } else { (w, (w * u64::from(b)).div_ceil(u64::from(a))) };
    let grow = |lo: i32, hi: i32, want: u64, min: i32, max: i32| -> (i32, i32) {
        let want = want.min((max - min) as u64) as i32;
        let extra = want - (hi - lo);
        let lo = (lo - extra / 2).max(min);
        let hi = (lo + want).min(max);
        (hi - want, hi)
    };
    let (x0, x1) = grow(r.x0, r.x1, want_w, canvas.x0, canvas.x1);
    let (y0, y1) = grow(r.y0, r.y1, want_h, canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// The visible image over `area`, in 8-bit sRGB.
pub(super) fn srgb_composite(doc: &Document, area: Rect) -> Result<Surface> {
    let rgb = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut surface = Surface::new(rgb);
    let mut data = Vec::new();
    photocraft_compose::render_bands(doc, area, 0, |band| -> Result<()> {
        data.clear();
        data.resize(band.px.len() * 4, 0.0);
        for (p, out) in band.px.iter().zip(data.as_chunks_mut::<4>().0.iter_mut()) {
            photocraft_raster::from_rgba_into(&rgb, *p, out);
        }
        surface.write_region(band.rect, &data);
        Ok(())
    })?;
    let t = Transform::new(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(other)?;
    Ok(crate::color_cmds::convert_surface(&surface, ColorMode::Rgb, rgb, &t))
}

/// sRGB pixels into the document's mode, depth and profile.
fn to_document(doc: &Document, s: &Surface) -> Result<Surface> {
    let fmt = PixelFormat::new(doc.mode, doc.depth, true);
    let t = Transform::new(Builtin::Srgb.profile(), &crate::color_cmds::document_profile(doc), Intent::RelativeColorimetric, true).map_err(other)?;
    let converted = crate::color_cmds::convert_surface(s, ColorMode::Rgb, fmt, &t);
    Ok(if converted.format() == fmt { converted } else { converted.convert(fmt) })
}

/// `area` of `s` scaled to `w`×`h`, at the origin.
pub(super) fn scaled(s: &Surface, area: Rect, w: u32, h: u32) -> Surface {
    let mut local = Surface::new(s.format());
    local.write_region(Rect::from_xywh(0, 0, area.width(), area.height()), &s.read_region(area));
    if (w, h) == (area.width(), area.height()) {
        return local;
    }
    let sx = f64::from(w) / f64::from(area.width());
    let sy = f64::from(h) / f64::from(area.height());
    photocraft_algo::resample::resize_surface(&local, sx, sy, photocraft_algo::resample::Resample::Bicubic)
}

/// Largest mean difference (0..1) around the selection still treated as colour drift.
const MAX_DRIFT: f64 = 0.25;
/// Mean difference past which a pixel outside the selection counts as changed by the model, not
/// just relit.
const CHANGED: f32 = 0.25;
/// Difference in fine detail past which a pixel outside the selection counts as changed.
const DETAIL: f32 = 0.03;
/// Levels in the fitted tone curves.
const BINS: usize = 32;
/// Grey that transparent parts of the image are shown on when sent.
const HOLE: f32 = 128.0;
/// How bright the placeholder over the selection is, as a share of the colours around it.
/// At full brightness, bright content running into the selection (lit panels below an empty
/// sky) smears through it and reads as that content carrying on to the far edge; dimmed, it
/// reads as a gap to fill. Dimming didn't make the model draw dark shapes into bright scenes.
const PLACEHOLDER: f32 = 0.35;

/// `px` (RGBA, 8-bit, `w`×`h`) as PNG.
pub(super) fn encode_rgba(px: Vec<u8>, w: u32, h: u32) -> Result<Vec<u8>> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}

/// The picture and mask sent, both `w`×`h`: `image` (sRGB, already scaled) with the selection
/// replaced by a dimmed soft blur of the colours around it, and the selection as a PNG with alpha
/// (transparent and white where the fill goes, opaque black where the picture is kept). The model
/// never sees what is being replaced, as Photoshop's fill doesn't, so it can't paint it back. A
/// flat grey hole was tried first: a model read it as an object (a grey box, a lamp) and drew it
/// in.
pub(super) fn request_images(image: &Surface, sel: &Surface, area: Rect, w: u32, h: u32) -> Result<(Vec<u8>, Vec<u8>, Reference)> {
    let r = Rect::from_xywh(0, 0, w, h);
    let rgba = image.convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true)).to_interleaved(r);
    let coverage: Vec<f32> = scaled(sel, area, w, h).read_region(r).into_iter().map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }).collect();
    let mut original = Vec::with_capacity(coverage.len() * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = f32::from(px[3]) / 255.0;
        // Transparent areas read as mid grey, not black.
        for &c in &px[..3] {
            original.push((f32::from(c) * a + HOLE * (1.0 - a)) / 255.0);
        }
    }
    if original.len() != coverage.len() * 3 {
        return Err(other("Generative Fill could not read the area around the selection"));
    }
    // Any pixel the selection touches, even faintly along a feathered edge, is replaced whole and
    // gives the placeholder nothing of its own colour: none of what is being replaced is sent.
    // The mask keeps the feather, so the model still blends the edge.
    let selected: Vec<f32> = coverage.iter().map(|&m| if m > 0.0 { 1.0 } else { 0.0 }).collect();
    let fill = surroundings_fill(&original, &selected, w as usize, h as usize);
    let mut cleared = Vec::with_capacity(rgba.len());
    let mut mask = Vec::with_capacity(rgba.len());
    for (i, &m) in coverage.iter().enumerate() {
        for c in 0..3 {
            let v = if m > 0.0 { fill[i * 3 + c] * PLACEHOLDER } else { original[i * 3 + c] };
            cleared.push((v * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        cleared.push(255);
        let g = (m * 255.0).round().clamp(0.0, 255.0) as u8;
        mask.extend_from_slice(&[g, g, g, 255 - g]);
    }
    let reference = Reference { original, coverage, w, h };
    Ok((encode_rgba(cleared, w, h)?, encode_rgba(mask, w, h)?, reference))
}

/// Colours for the selected area taken from around it (RGB, 0..1, `w`×`h`), by push-pull: the
/// unselected pixels are averaged down a pyramid of halving sizes until every cell has some, then
/// the averages are brought back up with bilinear interpolation, each level keeping its own known
/// pixels. The result keeps the edge colours and turns smooth toward the middle, with no rings
/// or steps a model could read as shapes.
pub(super) fn surroundings_fill(original: &[f32], coverage: &[f32], w: usize, h: usize) -> Vec<f32> {
    // Each level: (width, height, weight 0..1, colour).
    let mut levels: Vec<(usize, usize, Vec<f32>, Vec<f32>)> = vec![(w, h, coverage.iter().map(|c| 1.0 - c).collect(), original.to_vec())];
    while let Some((lw, lh, lwt, lc)) = levels.last()
        && (*lw > 1 || *lh > 1)
    {
        let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
        let (mut wt, mut col) = (vec![0.0f32; nw * nh], vec![0.0f32; nw * nh * 3]);
        for y in 0..nh {
            for x in 0..nw {
                let (mut sum, mut acc) = (0.0f32, [0.0f32; 3]);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < *lw && sy < *lh {
                        let i = sy * lw + sx;
                        sum += lwt[i];
                        for c in 0..3 {
                            acc[c] += lwt[i] * lc[i * 3 + c];
                        }
                    }
                }
                let i = y * nw + x;
                if sum > 0.0 {
                    for c in 0..3 {
                        col[i * 3 + c] = acc[c] / sum;
                    }
                }
                wt[i] = sum.min(1.0);
            }
        }
        levels.push((nw, nh, wt, col));
    }
    // Bring the averages back up: unknown parts of each level take the level above, interpolated.
    for l in (0..levels.len() - 1).rev() {
        let (cw, ch, _, coarse) = levels[l + 1].clone();
        let (lw, lh, wt, col) = &mut levels[l];
        for y in 0..*lh {
            let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (ch - 1) as f32);
            let (y0, ty) = (fy.floor() as usize, fy.fract());
            let y1 = (y0 + 1).min(ch - 1);
            for x in 0..*lw {
                let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (cw - 1) as f32);
                let (x0, tx) = (fx.floor() as usize, fx.fract());
                let x1 = (x0 + 1).min(cw - 1);
                let i = y * *lw + x;
                for c in 0..3 {
                    let at = |xx: usize, yy: usize| coarse[(yy * cw + xx) * 3 + c];
                    let up = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
                    col[i * 3 + c] = wt[i] * col[i * 3 + c] + (1.0 - wt[i]) * up;
                }
            }
        }
    }
    levels.swap_remove(0).3
}

/// What a result is matched against: the area as it was before the hole was cleared (RGB, 0..1)
/// and the selection's coverage, both at the request size.
#[derive(Clone)]
pub(super) struct Reference {
    pub(super) original: Vec<f32>,
    pub(super) coverage: Vec<f32>,
    pub(super) w: u32,
    pub(super) h: u32,
}

/// Box blur of a `w`×`h` plane with radius `r`, edges clamped.
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    let pass = |line: &mut [f32], tmp: &mut Vec<f32>| {
        let n = line.len();
        tmp.clear();
        tmp.extend_from_slice(line);
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| tmp[i]).sum();
        let mut count = r.min(n - 1) + 1;
        for i in 0..n {
            line[i] = sum / count as f32;
            if i + r + 1 < n {
                sum += tmp[i + r + 1];
                count += 1;
            }
            if i >= r {
                sum -= tmp[i - r];
                count -= 1;
            }
        }
    };
    let mut tmp = Vec::new();
    let mut col = vec![0.0; h];
    for row in v.chunks_mut(w) {
        pass(row, &mut tmp);
    }
    for x in 0..w {
        for y in 0..h {
            col[y] = v[y * w + x];
        }
        pass(&mut col, &mut tmp);
        for y in 0..h {
            v[y * w + x] = col[y];
        }
    }
}

/// Brightness of an RGBA or RGB pixel (0..1), Rec. 709 weights.
fn luma(p: &[f32]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// A tone curve taking brightness `from` to brightness `to`, fitted over the pixels with weight
/// `wt`: the mean change at each of [`BINS`] levels, so a model that lifts the shadows and leaves
/// the highlights is undone level by level. Levels without enough samples take their neighbours'
/// change; the curve never turns downward.
fn fit_curve(from: &[f32], to: &[f32], wt: &[f32]) -> [f32; BINS] {
    let (mut n, mut d) = ([0.0f64; BINS], [0.0f64; BINS]);
    for (i, &o) in wt.iter().enumerate() {
        let x = from[i].clamp(0.0, 1.0);
        let b = ((x * BINS as f32) as usize).min(BINS - 1);
        n[b] += f64::from(o);
        d[b] += f64::from(o) * f64::from(to[i] - x);
    }
    let known: Vec<usize> = (0..BINS).filter(|&b| n[b] >= 32.0).collect();
    let at = |k: usize| (d[k] / n[k]) as f32;
    let mut curve = [0.0f32; BINS];
    for (b, v) in curve.iter_mut().enumerate() {
        *v = match (known.iter().rev().find(|&&k| k <= b), known.iter().find(|&&k| k >= b)) {
            (Some(&lo), Some(&hi)) if lo != hi => at(lo) + (at(hi) - at(lo)) * (b - lo) as f32 / (hi - lo) as f32,
            (Some(&k), _) | (None, Some(&k)) => at(k),
            (None, None) => 0.0,
        };
    }
    // Rising: each level's output at least the one below it.
    for b in 1..BINS {
        curve[b] = curve[b].max(curve[b - 1] - 1.0 / BINS as f32);
    }
    curve
}

/// `v` through a curve from [`fit_curve`] (the change at each level's centre, interpolated).
fn apply_curve(curve: &[f32; BINS], v: f32) -> f32 {
    let t = (v.clamp(0.0, 1.0) * BINS as f32 - 0.5).clamp(0.0, (BINS - 1) as f32);
    let (b, f) = (t as usize, t.fract());
    v + curve[b] + (curve[(b + 1).min(BINS - 1)] - curve[b]) * f
}

/// An RGBA pixel's brightness moved along `curve`, its hue and saturation kept: scaled while
/// bright enough to scale, shifted near black.
fn tone(curve: &[f32; BINS], p: &mut [f32]) {
    let y = luma(p);
    let target = apply_curve(curve, y);
    for v in &mut p[..3] {
        *v = if y > 0.05 { *v * (target / y) } else { *v + (target - y) };
    }
}

/// Whether a result's colours were matched to the photo around the selection
/// ([`match_surroundings`]), and if not, why.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ColourMatch {
    /// Matched. `drift` is the mean difference outside the selection before matching (0..1),
    /// `changed` the share of that area the model redrew rather than relit (percent), and
    /// `shadows`, `midtones` and `highlights` the brightness change taken out at those levels.
    Matched { drift: f64, changed: f64, shadows: f32, midtones: f32, highlights: f32 },
    /// Skipped: too little of the area sent lies outside the selection to measure the drift.
    TooLittleOutside,
    /// Skipped: the result differs from the original around the selection by more than colour
    /// drift explains (reframed, restyled), so it was left as it came back.
    TooDifferent { drift: f64, limit: f64 },
}

impl ColourMatch {
    /// Did matching not run?
    pub fn skipped(&self) -> bool {
        !matches!(self, ColourMatch::Matched { .. })
    }
}

/// The mean difference (0..1) between `px` (RGBA at the reference size) and the original outside
/// the selection, weighted by how far outside each pixel is; `None` when too little is outside.
fn outside_drift(px: &[f32], r: &Reference) -> Option<f64> {
    let n = (r.w as usize).saturating_mul(r.h as usize);
    if px.len() != n * 4 || r.original.len() != n * 3 || r.coverage.len() != n {
        return None;
    }
    let outside = |i: usize| f64::from(1.0 - r.coverage[i].clamp(0.0, 1.0));
    let weight: f64 = (0..n).map(outside).sum();
    if weight < 16.0 {
        return None;
    }
    let diff: f64 = (0..n).map(|i| outside(i) * (0..3).map(|c| f64::from((px[i * 4 + c] - r.original[i * 3 + c]).abs())).sum::<f64>()).sum();
    Some(diff / (weight * 3.0))
}

/// Take out the colour drift a model adds when it redraws the whole image, so the fill meets the
/// photo around it without a seam. Everything outside the selection should have come back
/// unchanged; comparing it with the original measures the drift. First a tone curve on brightness
/// (models relight: shadows lifted, highlights kept), hue untouched, fitted again without the pixels the model
/// actually changed (an object it moved), then
/// what is left at the edge carried smoothly inward by push-pull, as Photoshop's fills blend into
/// their surroundings. `px` is RGBA (0..1) at the reference size. Says whether it ran.
pub(super) fn match_surroundings(px: &mut [f32], r: &Reference) -> ColourMatch {
    let (w, h) = (r.w as usize, r.h as usize);
    let mut outside: Vec<f32> = r.coverage.iter().map(|c| 1.0 - c.clamp(0.0, 1.0)).collect();
    let weight: f64 = outside.iter().map(|&o| f64::from(o)).sum();
    let Some(drift) = outside_drift(px, r) else { return ColourMatch::TooLittleOutside };
    // A result that doesn't resemble the original around the selection (reframed, restyled) is
    // left alone rather than forced onto it.
    if drift > MAX_DRIFT {
        return ColourMatch::TooDifferent { drift, limit: MAX_DRIFT };
    }
    let target: Vec<f32> = r.original.as_chunks::<3>().0.iter().map(|p| luma(p)).collect();
    let fit = |px: &[f32], wt: &[f32]| fit_curve(&px.as_chunks::<4>().0.iter().map(|p| luma(p)).collect::<Vec<_>>(), &target, wt);
    // What the model changed rather than relit: different detail (an edge moved, an object
    // redrawn), or a different colour by far more than relighting explains.
    let first = fit(px, &outside);
    let detail = |mut v: Vec<f32>| {
        let mut smooth = v.clone();
        box_blur(&mut smooth, w, h, 2);
        box_blur(&mut smooth, w, h, 2);
        v.iter_mut().zip(&smooth).for_each(|(a, b)| *a -= b);
        v
    };
    let (got, want) = (detail(px.as_chunks::<4>().0.iter().map(|p| luma(p)).collect()), detail(target.clone()));
    let mut differs: Vec<f32> = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).collect();
    box_blur(&mut differs, w, h, 4);
    let mut changed = 0.0f64;
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        let mut q = *p;
        tone(&first, &mut q);
        let far = (0..3).map(|c| (q[c] - r.original[i * 3 + c]).abs()).sum::<f32>() / 3.0 > CHANGED;
        if outside[i] > 0.0 && (far || differs[i] > DETAIL) {
            changed += f64::from(outside[i]);
            outside[i] = 0.0;
        }
    }
    let curve = if outside.iter().map(|&o| f64::from(o)).sum::<f64>() >= 16.0 { fit(px, &outside) } else { first };
    let stats =
        ColourMatch::Matched { drift, changed: 100.0 * changed / weight, shadows: curve[BINS / 8], midtones: curve[BINS / 2], highlights: curve[BINS * 7 / 8] };
    for p in px.as_chunks_mut::<4>().0 {
        tone(&curve, p);
    }
    // What the curve leaves, known only where the result kept the original, spread over the rest.
    let residual: Vec<f32> = (0..w * h).flat_map(|i| [0, 1, 2].map(|c| if outside[i] > 0.0 { r.original[i * 3 + c] - px[i * 4 + c] } else { 0.0 })).collect();
    let unknown: Vec<f32> = outside.iter().map(|o| 1.0 - o).collect();
    let spread = surroundings_fill(&residual, &unknown, w, h);
    for i in 0..w * h {
        for c in 0..3 {
            let v = &mut px[i * 4 + c];
            if r.coverage[i] > 0.0 {
                *v += spread[i * 3 + c];
            }
            *v = v.clamp(0.0, 1.0);
        }
    }
    stats
}

/// Decode a returned PNG, JPEG or WebP into 8-bit sRGB, with its size.
pub(super) fn decode(bytes: &[u8]) -> Result<(Surface, Rect)> {
    let opts = codecs::DecodeOptions {
        limits: codecs::Limits { max_width: RESULT_SIDE, max_height: RESULT_SIDE, max_pixels: 40_000_000, max_alloc: 512 * 1024 * 1024 },
        keep_orientation: false,
    };
    let format = codecs::detect(bytes)
        .filter(|f| matches!(f, Format::Png | Format::Jpeg | Format::WebP))
        .ok_or_else(|| other("the AI service returned a picture PhotoCraft can't read"))?;
    let img = codecs::decode_as_with(format, bytes, &opts).map_err(|e| other(format!("the AI service returned a picture PhotoCraft can't read ({e})")))?;
    let img = img.convert(ChannelLayout::Rgba, CodecSample::U8);
    let r = Rect::from_xywh(0, 0, img.width(), img.height());
    if r.is_empty() {
        return Err(other("the AI service returned an empty picture"));
    }
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.write_interleaved(r, img.data());
    Ok((s, r))
}

/// How far (in log space) a result's shape may be from the area's and still count as the same
/// shape: 2 %, more than a model's rounding to a multiple of 8 or 64 pixels.
const SAME_SHAPE: f64 = 0.02;

/// How a returned picture was lined up with the area sent.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Framing {
    /// It came back the area's shape (within 2 %) and was scaled to it.
    Same,
    /// Another shape. Its centre, cut to the area's shape, lined up with the picture around the
    /// selection (the model had added to the sides), so that is what was used.
    Centre { returned: [u32; 2], sent: [u32; 2] },
    /// Another shape, but the whole of it lined up with the picture around the selection once
    /// scaled to the area (the model had resized what it was sent), so it was scaled.
    Scaled { returned: [u32; 2], sent: [u32; 2] },
}

/// The largest rect of `w`:`h`'s shape centred in `full`.
fn centred(full: Rect, w: u32, h: u32) -> Rect {
    let (fw, fh) = (u64::from(full.width()), u64::from(full.height()));
    let (w, h) = (u64::from(w.max(1)), u64::from(h.max(1)));
    if fw * h > fh * w {
        let cw = ((fh * w + h / 2) / h).clamp(1, fw) as i32;
        let x0 = full.x0 + (full.width() as i32 - cw) / 2;
        Rect::new(x0, full.y0, x0 + cw, full.y1)
    } else {
        let ch = ((fw * h + w / 2) / w).clamp(1, fh) as i32;
        let y0 = full.y0 + (full.height() as i32 - ch) / 2;
        Rect::new(full.x0, y0, full.x1, y0 + ch)
    }
}

/// `got` (of size `full`) at the request size, and how it was lined up. A result the area's shape
/// is scaled. Another shape is never just stretched: its centre cut to the area's shape and the
/// whole of it scaled are each compared with the picture around the selection, and the one that
/// lines up is used; when neither does (or too little is outside the selection to tell), the
/// variation is refused with the reason.
fn frame(got: &Surface, full: Rect, r: &Reference) -> std::result::Result<(Vec<f32>, Framing), String> {
    let at = Rect::from_xywh(0, 0, r.w, r.h);
    let shape = |w: u32, h: u32| (f64::from(w.max(1)) / f64::from(h.max(1))).ln();
    if (shape(full.width(), full.height()) - shape(r.w, r.h)).abs() <= SAME_SHAPE {
        return Ok((scaled(got, full, r.w, r.h).read_region(at), Framing::Same));
    }
    let (returned, sent) = ([full.width(), full.height()], [r.w, r.h]);
    let centre = scaled(got, centred(full, r.w, r.h), r.w, r.h).read_region(at);
    let whole = scaled(got, full, r.w, r.h).read_region(at);
    let fits = |px: &[f32]| outside_drift(px, r).filter(|d| *d <= MAX_DRIFT);
    let best = match (fits(&centre), fits(&whole)) {
        (Some(c), Some(w)) if w < c => Some((whole, Framing::Scaled { returned, sent })),
        (Some(_), _) => Some((centre, Framing::Centre { returned, sent })),
        (None, Some(_)) => Some((whole, Framing::Scaled { returned, sent })),
        (None, None) => None,
    };
    best.ok_or_else(|| {
        format!(
            "the AI service returned a {}×{} picture for a {}×{} area, framed so that it doesn't line up with the picture around the selection; that variation was not placed",
            returned[0], returned[1], sent[0], sent[1]
        )
    })
}

/// A returned picture placed: its pixels in document coordinates, and how it was lined up and
/// colour-matched.
pub(super) struct Placed {
    pub(super) surface: Surface,
    pub(super) framing: Framing,
    pub(super) colour: ColourMatch,
}

/// A returned picture, lined up with the area sent ([`frame`]), colour-matched to the original
/// around the selection, scaled into `area` in document coordinates. `Err` says why it can't be
/// placed (unreadable, framed differently).
pub(super) fn place(bytes: &[u8], area: Rect, reference: &Reference) -> std::result::Result<Placed, String> {
    let (got, full) = decode(bytes).map_err(|e| e.to_string())?;
    let r = Rect::from_xywh(0, 0, reference.w, reference.h);
    let (mut px, framing) = frame(&got, full, reference)?;
    let colour = match_surroundings(&mut px, reference);
    let mut at_request = Surface::new(got.format());
    at_request.write_region(r, &px);
    let fitted = scaled(&at_request, r, area.width(), area.height());
    let mut placed = Surface::new(fitted.format());
    placed.write_region(area, &fitted.read_region(Rect::from_xywh(0, 0, area.width(), area.height())));
    Ok(Placed { surface: placed, framing, colour })
}

pub(super) fn variation_layer(doc: &Document, pixels: &Surface, n: usize, visible: bool) -> Result<Layer> {
    let converted = to_document(doc, pixels)?;
    let mut layer = Layer::raster(format!("Variation {n}"), converted.format());
    *layer.surface_mut().ok_or_else(|| other("could not create the layer"))? = converted;
    layer.visible = visible;
    Ok(layer)
}
