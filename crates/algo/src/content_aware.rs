//! Edit › Content-Aware Fill: patch-based completion ([`crate::inpaint::complete_masked`]) with
//! Photoshop's sampling area, colour adaptation and geometric adaptation.
//!
//! Photoshop 25.4 was measured as a black box (synthetic and public-domain images, results read
//! back exactly; none of its code or assets were used):
//!
//! * **Sampling window** ([`sampling_window`]): the fill copies only from a square centred on the
//!   selection's bounding box, of side `4·√(max(w, 50)·max(h, 50))`: four times a square
//!   selection's side, 200 px at least, and for a long thin selection four times the geometric
//!   mean of its sides. At the canvas edge the square slides inwards rather than being cut. Pixels
//!   outside it never change the result (Edit › Fill and the workspace's Rectangular area alike).
//! * **Auto sampling area** ([`auto_sampling`]): the workspace's default keeps, within that
//!   square, only what looks like the selection's surroundings: a region of another brightness or
//!   another texture is left out along its edges, wherever it lies (a bright band through the
//!   window is excluded, the matching area beyond it is not).
//! * **The fill** copies pieces of the sampling area pixel for pixel (15–40 px across), joined by
//!   seams a few pixels wide: see [`crate::inpaint`].
//! * **Colour adaptation** None / Default / High / Very High: with None the copies are exact; the
//!   others let each copied patch take a gain and a bias towards its surroundings, more at each
//!   level ([`color_level`]). The patch transform is that of S. Darabi et al., *Image Melding*,
//!   SIGGRAPH 2012.
//! * **Rotation / scale / mirror adaptation**: the sampling window is extended with rotated,
//!   rescaled and mirrored copies of itself, laid out side by side (separated by gaps that are no
//!   image at all), so the patch search can pick transformed source patches, as in the
//!   generalised PatchMatch of C. Barnes, E. Shechtman, D. B. Goldman, A. Finkelstein, *The
//!   Generalized PatchMatch Correspondence Algorithm*, ECCV 2010 (which searches over rotations
//!   and scales; we enumerate a small set instead).
//!
//! Buffers are interleaved `w × h × ch` normalised floats (any model/depth); deterministic.

use photocraft_geom::Rect;

use std::borrow::Cow;

use photocraft_raster::{Cancelled, Interrupt};

use crate::inpaint::{Adapt, CompleteParams, Px, complete_masked, complete_masked_owned};
use crate::par;
use crate::poisson::membrane_fill_with;

/// Options for [`fill`].
#[derive(Clone, Debug, PartialEq)]
pub struct FillOptions {
    /// Colour adaptation of the copies ([`color_level`]).
    pub color_adaptation: Adapt,
    /// Extra source rotations in degrees (Photoshop's rotation adaptation levels).
    pub rotations: Vec<f32>,
    /// Add downscaled and upscaled source copies.
    pub scale: bool,
    /// Add a horizontally mirrored source copy.
    pub mirror: bool,
    pub seed: u64,
}

impl Default for FillOptions {
    fn default() -> Self {
        Self { color_adaptation: color_level("default"), rotations: Vec::new(), scale: false, mirror: false, seed: 1 }
    }
}

/// Rotation set for Photoshop's adaptation levels: none, low, medium, high, full.
pub fn rotation_level(level: &str) -> Vec<f32> {
    match level {
        "low" => vec![-10.0, 10.0],
        "medium" => vec![-20.0, 20.0],
        "high" => vec![-35.0, 35.0],
        "full" => vec![90.0, 180.0, 270.0],
        _ => Vec::new(),
    }
}

/// Colour adaptation for Photoshop's levels: none, default, high, veryHigh. Measured on a texture
/// under a brightness ramp: with None the copies are exact; Default shifts them by up to about
/// 10/255, High about 20, Very High about 35, scaling their contrast a little more at each level.
pub fn color_level(level: &str) -> Adapt {
    match level {
        "none" => Adapt::NONE,
        "high" => Adapt { gain: (0.8, 1.25), bias: 0.08 },
        "veryHigh" => Adapt { gain: (0.67, 1.5), bias: 0.14 },
        _ => Adapt { gain: (0.9, 1.1), bias: 0.04 },
    }
}

/// Photoshop's sampling window for a selection with bounding box `hole`: the square of side
/// `4·√(max(w, 50)·max(h, 50))` centred on it, slid inside `canvas` (and clipped to it only where
/// the canvas is smaller than the square). A selection longer than that square (more than 16
/// times as long as it is wide, which was not measured) keeps its whole length.
pub fn sampling_window(hole: Rect, canvas: Rect) -> Rect {
    if hole.is_empty() || canvas.is_empty() {
        return Rect::EMPTY;
    }
    let (w, h) = (f64::from(hole.width().max(50)), f64::from(hole.height().max(50)));
    let side = (4.0 * (w * h).sqrt()).round().min(i32::MAX as f64) as i64;
    // Twice the centre, so that odd sizes stay exact.
    let span = |c2: i64, extent: u32, lo: i32, hi: i32| -> (i32, i32) {
        let (lo, hi) = (i64::from(lo), i64::from(hi));
        let side = side.max(i64::from(extent));
        if side >= hi - lo {
            return (lo as i32, hi as i32);
        }
        let a = ((c2 - side) / 2).clamp(lo, hi - side);
        (a as i32, (a + side) as i32)
    };
    let (x0, x1) = span(i64::from(hole.x0) + i64::from(hole.x1), hole.width(), canvas.x0, canvas.x1);
    let (y0, y1) = span(i64::from(hole.y0) + i64::from(hole.y1), hole.height(), canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// One source variant: pixels and which of them are usable.
struct Variant {
    w: usize,
    h: usize,
    img: Vec<f32>,
    ok: Vec<bool>,
}

/// Most pixels the mosaic of the image and its variants may span. It is one dense rectangle, so
/// thin images turned to different angles leave it mostly empty: the cost grows with the square
/// of their length, not with their pixels.
const MAX_MOSAIC_PIXELS: usize = 1 << 25;

/// Size of the mosaic of an `w`×`h` image and `extra` variant sizes separated by `gap`, or `None`
/// when it spans more than [`MAX_MOSAIC_PIXELS`] (or overflows).
fn mosaic_size(w: usize, h: usize, gap: usize, extra: &[(usize, usize)]) -> Option<(usize, usize)> {
    let col_w = extra.iter().map(|v| v.0).max().unwrap_or(0);
    let col_h = extra.iter().try_fold(0usize, |n, v| n.checked_add(v.1)?.checked_add(gap))?;
    let mw = w.checked_add(gap)?.checked_add(col_w)?;
    let mh = h.max(col_h);
    (mw.checked_mul(mh)? <= MAX_MOSAIC_PIXELS).then_some((mw, mh))
}

/// A source buffer and which of its pixels may be sampled.
#[derive(Clone, Copy)]
struct Src<'a> {
    w: usize,
    h: usize,
    ch: usize,
    img: &'a [f32],
    ok: &'a [bool],
}

/// Sample the source bilinearly at (x, y); false when any tap is unusable.
fn sample(src: Src, x: f32, y: f32, out: &mut [f32]) -> bool {
    let Src { w, h, ch, img, ok } = src;
    if x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 {
        return false;
    }
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    for (xx, yy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        if !ok[yy * w + xx] {
            return false;
        }
    }
    for (c, o) in out.iter_mut().enumerate().take(ch) {
        let p = |xx: usize, yy: usize| img[(yy * w + xx) * ch + c];
        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
        let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
        *o = top + (bot - top) * ty;
    }
    true
}

/// Transformed copy of the source: rotation (degrees) and scale about the centre, optional mirror.
/// `Err` when cancelled.
/// Dimensions of the [`variant`] copy: the same bounding-box maths without building its pixels.
fn variant_dims(w: usize, h: usize, deg: f32, scale: f32) -> (usize, usize) {
    let (s, c) = (-deg.to_radians()).sin_cos();
    let (fw, fh) = (w as f32 * scale, h as f32 * scale);
    let ow = ((fw * c.abs() + fh * s.abs()).round() as usize).max(1);
    let oh = ((fw * s.abs() + fh * c.abs()).round() as usize).max(1);
    (ow, oh)
}

fn variant(src: Src, deg: f32, scale: f32, mirror: bool, ctl: &Interrupt) -> Result<Variant, Cancelled> {
    let Src { w, h, ch, .. } = src;
    // Output size: the rotated, scaled bounding box.
    let (ow, oh) = variant_dims(w, h, deg, scale);
    let (s, c) = (-deg.to_radians()).sin_cos();
    let (icx, icy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let (ocx, ocy) = ((ow as f32 - 1.0) / 2.0, (oh as f32 - 1.0) / 2.0);
    let mut out = Variant { w: ow, h: oh, img: vec![0.0; ow * oh * ch], ok: vec![false; ow * oh] };
    // Every pixel is sampled on its own, so the rows can be done in any order.
    let row = |y: usize, irow: &mut [f32], orow: &mut [bool]| {
        if ctl.cancelled() {
            return;
        }
        let mut px = vec![0.0f32; ch];
        for x in 0..ow {
            let (dx, dy) = ((x as f32 - ocx) / scale, (y as f32 - ocy) / scale);
            let mut sx = icx + dx * c - dy * s;
            let sy = icy + dx * s + dy * c;
            if mirror {
                sx = (w as f32 - 1.0) - sx;
            }
            if sample(src, sx, sy, &mut px) {
                orow[x] = true;
                irow[x * ch..(x + 1) * ch].copy_from_slice(&px);
            }
        }
    };
    par::chunks2_mut(&mut out.img, ow * ch, &mut out.ok, ow, true, row);
    ctl.check()?;
    Ok(out)
}

/// Fill `hole` in `img`, copying only from pixels where `source` is true (and not in the hole).
/// Returns the full buffer (unchanged outside the hole). Falls back to the membrane fill when
/// nothing can be sampled.
pub fn fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], source: &[bool], opts: &FillOptions) -> Vec<f32> {
    // Never cancelled; the fallback (the input unchanged) is unreachable.
    fill_with(w, h, ch, img, hole, source, opts, &photocraft_raster::Interrupt::NONE).unwrap_or_else(|_| img.to_vec())
}

/// [`fill`] that can be cancelled (see [`crate::inpaint::complete_with`]) and reports progress.
#[allow(clippy::too_many_arguments)]
pub fn fill_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    source: &[bool],
    opts: &FillOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Vec<f32>, photocraft_raster::Cancelled> {
    fill_cow(w, h, ch, Cow::Borrowed(img), hole, source, opts, ctl)
}

/// [`fill_with`] that fills `img` itself rather than a copy of it (a big window need not be held
/// twice). The result differs from `img` only in the hole.
#[allow(clippy::too_many_arguments)]
pub fn fill_owned(
    w: usize,
    h: usize,
    ch: usize,
    img: Vec<f32>,
    hole: &[bool],
    source: &[bool],
    opts: &FillOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Vec<f32>, photocraft_raster::Cancelled> {
    fill_cow(w, h, ch, Cow::Owned(img), hole, source, opts, ctl)
}

/// The patch radius of the fill for the option set #14191 measured (Default colour adaptation,
/// no rotation, no scale, no mirror). The straight features of its document are 10 px bands and
/// 20 px stripes: the default 7x7 patch fits inside one stripe or inside one band gap, so it
/// carries no phase information and the completion decides from colour alone and runs one region
/// into the other. An 11x11 patch covers a feature and the one next to it, which is what keeps
/// their phase through the hole. Every other option set, and a sampling area other than the
/// Rectangular one, is marked "not measured" in #14191 and keeps [`CompleteParams`]' default radius.
const MEASURED_STRUCTURE_PATCH_RADIUS: usize = 5;

/// Whether `opts` holds the option set #14191 measured on its test document: Default colour
/// adaptation, no rotation, no scale and no mirror.
fn structure_measured(opts: &FillOptions) -> bool {
    opts.color_adaptation == color_level("default") && opts.rotations.is_empty() && !opts.scale && !opts.mirror
}

/// Whether `source` is the Rectangular sampling area of the buffer: every pixel outside the hole.
/// #14191 measured Rectangular, and Auto where it keeps that same area; Auto where its
/// content-based choice differs (or any other area) is not measured.
fn rectangular_area(hole: &[bool], source: &[bool]) -> bool {
    hole.iter().zip(source).all(|(h, s)| *h || *s)
}

/// The variant specs of `opts`: mirror, the rotation set, the scaled pair.
fn variant_specs(opts: &FillOptions) -> Vec<(f32, f32, bool)> {
    let mut specs: Vec<(f32, f32, bool)> = Vec::new();
    if opts.mirror {
        specs.push((0.0, 1.0, true));
    }
    for &deg in &opts.rotations {
        specs.push((deg, 1.0, false));
    }
    if opts.scale {
        specs.push((0.0, 0.8, false));
        specs.push((0.0, 1.25, false));
    }
    specs
}

/// The variants of `src` for `specs`, or none when none are asked for or their mosaic would not fit
/// in the budget. The mosaic gap follows `params`, so they are built again for every attempt.
fn mosaic_variants(src: Src, specs: &[(f32, f32, bool)], params: &CompleteParams, ctl: &Interrupt) -> Result<Vec<Variant>, Cancelled> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let gap = 2 * params.patch_radius + 2;
    let dims: Vec<(usize, usize)> = specs.iter().map(|&(deg, scale, _)| variant_dims(src.w, src.h, deg, scale)).collect();
    if mosaic_size(src.w, src.h, gap, &dims).is_none() {
        return Ok(Vec::new());
    }
    let mut extra = Vec::new();
    for &(deg, scale, mirror) in specs {
        extra.push(variant(src, deg, scale, mirror, ctl)?);
    }
    Ok(extra)
}

/// The roles of the window's pixels: the hole, what may be copied from, and the rest of the image.
fn window_roles(hole: &[bool], ok: &[bool]) -> Vec<Px> {
    par::map(hole.len(), true, |s| {
        if hole[s] {
            Px::Hole
        } else if ok[s] {
            Px::Source
        } else {
            Px::Known
        }
    })
}

/// The mosaic of the window and its variants: identity at the left, the variants in a column to its
/// right, gaps that are no image. `None` when the mosaic spans more than the budget (the variants
/// then go unused, as they did before they were built).
#[allow(clippy::too_many_arguments)]
fn mosaic_pixels(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    ok: &[bool],
    extra: &[Variant],
    params: &CompleteParams,
) -> Option<(usize, usize, Vec<f32>, Vec<Px>)> {
    let gap = 2 * params.patch_radius + 2;
    if extra.is_empty() {
        return None;
    }
    let (mw, mh) = mosaic_size(w, h, gap, &extra.iter().map(|v| (v.w, v.h)).collect::<Vec<_>>())?;
    let mut mimg = vec![0.0f32; mw * mh * ch];
    let mut roles = vec![Px::Void; mw * mh];
    for y in 0..h {
        for x in 0..w {
            let (s, d) = (y * w + x, y * mw + x);
            mimg[d * ch..(d + 1) * ch].copy_from_slice(&img[s * ch..(s + 1) * ch]);
            roles[d] = if hole[s] {
                Px::Hole
            } else if ok[s] {
                Px::Source
            } else {
                Px::Known
            };
        }
    }
    let mut oy = 0;
    for v in extra {
        for y in 0..v.h {
            for x in 0..v.w {
                let (s, d) = (y * v.w + x, (oy + y) * mw + w + gap + x);
                mimg[d * ch..(d + 1) * ch].copy_from_slice(&v.img[s * ch..(s + 1) * ch]);
                if v.ok[s] {
                    roles[d] = Px::Source;
                }
            }
        }
        oy += v.h + gap;
    }
    Some((mw, mh, mimg, roles))
}

/// One completion attempt: the variant mosaic to run it on, or the plain window when no variant was
/// asked for or the mosaic would not fit the budget.
struct Attempt {
    /// The mosaic's dimensions (the window's own when there is no mosaic).
    mw: usize,
    mh: usize,
    /// The mosaic pixels; empty when the attempt runs on the window itself.
    mosaic: Vec<f32>,
    /// The roles of the attempt's pixels (the window's own when there is no mosaic).
    roles: Vec<Px>,
}

/// Build one attempt: variants first (their pixels are expensive, so the mosaic budget is settled
/// on their dimensions before any variant is built), then the mosaic or the plain window's roles.
/// The mosaic gap follows `params`, so an attempt is built for every patch radius.
#[allow(clippy::too_many_arguments)]
fn fill_attempt(
    w: usize,
    h: usize,
    ch: usize,
    pixels: &[f32],
    hole: &[bool],
    ok: &[bool],
    specs: &[(f32, f32, bool)],
    params: &CompleteParams,
    ctl: &Interrupt,
) -> Result<Attempt, Cancelled> {
    let src = Src { w, h, ch, img: pixels, ok };
    let extra = mosaic_variants(src, specs, params, ctl)?;
    ctl.check()?;
    match mosaic_pixels(w, h, ch, pixels, hole, ok, &extra, params) {
        Some((mw, mh, mosaic, roles)) => Ok(Attempt { mw, mh, mosaic, roles }),
        // No variant was asked for, or its mosaic would not fit the budget: complete the window.
        None => Ok(Attempt { mw: w, mh: h, mosaic: Vec::new(), roles: window_roles(hole, ok) }),
    }
}

/// The window's own pixels out of a finished mosaic (its left region, the variants and the gaps
/// beside it are no answer).
fn window_from_mosaic(w: usize, h: usize, ch: usize, mw: usize, mosaic: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h * ch];
    for y in 0..h {
        for x in 0..w {
            let (s, d) = ((y * mw + x) * ch, (y * w + x) * ch);
            out[d..d + ch].copy_from_slice(&mosaic[s..s + ch]);
        }
    }
    out
}

/// The measured fill of #14191, asked again with today's radius when the 11x11 patch leaves no
/// donor: an 11x11 window needs room inside the buffer, and a hole can also use up every window of
/// a bigger one. Without that second attempt the fill would answer with the smooth membrane where
/// today's 7x7 patch still copies pieces; #14191 measures a 600x400 document and keeps every other
/// case on today's behaviour.
#[allow(clippy::too_many_arguments)]
fn fill_retry(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    ok: &[bool],
    specs: &[(f32, f32, bool)],
    params: &mut CompleteParams,
    inner: &Interrupt,
    ctl: &Interrupt,
) -> Result<Option<Vec<f32>>, Cancelled> {
    let measured = params.patch_radius;
    for radius in [measured, CompleteParams::default().patch_radius] {
        params.patch_radius = radius;
        let attempt = fill_attempt(w, h, ch, img, hole, ok, specs, params, ctl)?;
        if attempt.mosaic.is_empty() {
            if let Some(f) = complete_masked(w, h, ch, img, &attempt.roles, params, inner)? {
                return Ok(Some(f));
            }
        } else if let Some(f) = complete_masked(attempt.mw, attempt.mh, ch, &attempt.mosaic, &attempt.roles, params, inner)? {
            return Ok(Some(window_from_mosaic(w, h, ch, attempt.mw, &f)));
        }
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn fill_cow(
    w: usize,
    h: usize,
    ch: usize,
    img: Cow<[f32]>,
    hole: &[bool],
    source: &[bool],
    opts: &FillOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Vec<f32>, photocraft_raster::Cancelled> {
    assert_eq!(img.len(), w * h * ch);
    assert_eq!(hole.len(), w * h);
    assert_eq!(source.len(), w * h);
    if !hole.iter().any(|h| *h) {
        return Ok(img.into_owned());
    }
    let mut params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    if structure_measured(opts) && rectangular_area(hole, source) {
        params.patch_radius = MEASURED_STRUCTURE_PATCH_RADIUS;
    }
    let ok: Vec<bool> = par::map(w * h, true, |i| !hole[i] && source[i]);
    let specs = variant_specs(opts);
    // The completion is nearly all the work.
    let cancel = || ctl.cancelled();
    let progress = |f: f32| ctl.progress(f * 0.99);
    let inner = photocraft_raster::Interrupt::new(&cancel, &progress);
    // One completion for every option set but the measured one: it fills the caller's buffer in
    // place when it owns one. The measured set may be asked a second time (see `fill_retry`).
    if params.patch_radius != MEASURED_STRUCTURE_PATCH_RADIUS {
        let mut pixels = img.into_owned();
        let attempt = fill_attempt(w, h, ch, &pixels, hole, &ok, &specs, &params, ctl)?;
        ctl.check()?;
        let out = if attempt.mosaic.is_empty() {
            match complete_masked_owned(w, h, ch, pixels, &attempt.roles, &params, &inner)? {
                Ok(f) => f,
                // Nothing to copy from: the membrane, as today.
                Err(v) => membrane_fill_with(w, h, ch, &v, hole, ctl)?,
            }
        } else {
            match complete_masked(attempt.mw, attempt.mh, ch, &attempt.mosaic, &attempt.roles, &params, &inner)? {
                Some(f) => {
                    for y in 0..h {
                        for x in 0..w {
                            if hole[y * w + x] {
                                let (s, d) = ((y * attempt.mw + x) * ch, (y * w + x) * ch);
                                pixels[d..d + ch].copy_from_slice(&f[s..s + ch]);
                            }
                        }
                    }
                    pixels
                }
                None => membrane_fill_with(w, h, ch, &pixels, hole, ctl)?,
            }
        };
        ctl.check()?;
        ctl.progress(1.0);
        return Ok(out);
    }
    let out = fill_cow_measured(w, h, ch, img.as_ref(), hole, &ok, &specs, &mut params, &inner, ctl)?;
    ctl.check()?;
    ctl.progress(1.0);
    Ok(out)
}

/// The measured fill (#14191): the window is read where it stands, so both attempts see the same
/// pixels; the answer is a new buffer, as in [`fill_with`] (measured fills come from the command's
/// own copy of the sampling window).
#[allow(clippy::too_many_arguments)]
fn fill_cow_measured(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    ok: &[bool],
    specs: &[(f32, f32, bool)],
    params: &mut CompleteParams,
    inner: &Interrupt,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Vec<f32>, Cancelled> {
    match fill_retry(w, h, ch, img, hole, ok, specs, params, inner, ctl)? {
        Some(f) => Ok(f),
        // Nothing to copy from at either radius: the membrane, as today.
        None => membrane_fill_with(w, h, ch, img, hole, ctl),
    }
}
/// Box mean of radius `r` over a `w × h` plane (edge-clamped), by summed areas.
fn box_mean(w: usize, h: usize, v: &[f32], r: usize) -> Vec<f32> {
    let w1 = w + 1;
    let mut s = vec![0.0f64; w1 * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += f64::from(v[y * w + x]);
            s[(y + 1) * w1 + x + 1] = s[y * w1 + x + 1] + row;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = s[y1 * w1 + x1] + s[y0 * w1 + x0] - s[y0 * w1 + x1] - s[y1 * w1 + x0];
            out[y * w + x] = (sum / ((x1 - x0) * (y1 - y0)) as f64) as f32;
        }
    }
    out
}

/// Largest side [`auto_sampling`] works at: bigger windows are segmented at a reduced scale.
const AUTO_SIDE: usize = 384;

/// Content-Aware Fill's Auto sampling area within a window (the whole `w × h` buffer): the pixels
/// that look like the hole's surroundings. Each pixel is described by its local mean colour and
/// local contrast (7 × 7); the ring around the hole is summarised by a few clusters of those; a
/// pixel near a cluster prefers to be sampled, one far from all of them not, and one min cut
/// (contrast-sensitive, so the boundary follows edges) settles the area. Returns the mask of
/// pixels that may be copied from (never the hole). Falls back to everything outside the hole
/// when the area would be too small to copy from.
pub fn auto_sampling(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool]) -> Vec<bool> {
    // Never cancelled; the fallback (everything outside the hole) is unreachable.
    auto_sampling_with(w, h, ch, img, hole, &Interrupt::NONE).unwrap_or_else(|_| hole.iter().map(|h| !h).collect())
}

/// [`auto_sampling`] that can be cancelled. `Err(Cancelled)` when `ctl` was cancelled.
pub fn auto_sampling_with(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], ctl: &Interrupt) -> Result<Vec<bool>, Cancelled> {
    use crate::segment::{FREE, HARD_FG, RgbImage, grid_cut_cancellable};
    assert_eq!(img.len(), w * h * ch);
    assert_eq!(hole.len(), w * h);
    let everything = || -> Vec<bool> { par::map(hole.len(), true, |i| !hole[i]) };
    if w == 0 || h == 0 || ch == 0 || !hole.iter().any(|h| *h) {
        return Ok(everything());
    }
    // Work at a reduced scale for big windows (the area is smooth at this scale).
    let k = w.max(h).div_ceil(AUTO_SIDE).max(1);
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let nc = ch.min(3);
    // Each cell sums its k × k pixels, a row of cells at a time (in the same order as one pass
    // over the image would).
    // Per cell: the sums of its colour planes, its pixel count and whether it holds the hole.
    let mut sums = vec![0.0f32; sw * sh * (nc + 2)];
    par::chunks_mut(&mut sums, sw * (nc + 2), true, |sy, row| {
        if ctl.cancelled() {
            return;
        }
        for y in sy * k..((sy + 1) * k).min(h) {
            for x in 0..w {
                let cell = &mut row[(x / k) * (nc + 2)..(x / k + 1) * (nc + 2)];
                let p = y * w + x;
                if hole[p] {
                    cell[nc + 1] = 1.0;
                }
                for c in 0..nc {
                    cell[c] += img[p * ch + c];
                }
                cell[nc] += 1.0;
            }
        }
    });
    ctl.check()?;
    let cell = |i: usize| &sums[i * (nc + 2)..(i + 1) * (nc + 2)];
    let planes: Vec<Vec<f32>> = (0..nc).map(|c| (0..sw * sh).map(|i| cell(i)[c] / cell(i)[nc].max(1.0)).collect()).collect();
    let shole: Vec<bool> = (0..sw * sh).map(|i| cell(i)[nc + 1] > 0.0).collect();
    drop(sums);
    // Features: local means and local contrast (of the luma-ish average of the colour planes).
    let r = 3;
    let means: Vec<Vec<f32>> = planes.iter().map(|p| box_mean(sw, sh, p, r)).collect();
    let luma: Vec<f32> = (0..sw * sh).map(|i| planes.iter().map(|p| p[i]).sum::<f32>() / nc as f32).collect();
    let luma2: Vec<f32> = luma.iter().map(|v| v * v).collect();
    let (lm, lm2) = (box_mean(sw, sh, &luma, r), box_mean(sw, sh, &luma2, r));
    let contrast: Vec<f32> = lm.iter().zip(&lm2).map(|(m, m2)| (m2 - m * m).max(0.0).sqrt()).collect();
    let nf = nc + 1;
    let feat = |i: usize, out: &mut [f32]| {
        for (c, m) in means.iter().enumerate() {
            out[c] = m[i];
        }
        out[nc] = 2.0 * contrast[i];
    };
    // The ring: outside the hole, within a few pixels of it.
    let dist = crate::selection::edt(&shole, sw, sh);
    let ring_w = 6.0f32 / k as f32 + 1.0;
    let ring: Vec<usize> = (0..sw * sh).filter(|&i| !shole[i] && dist[i] <= ring_w).collect();
    if ring.is_empty() {
        return Ok(everything());
    }
    ctl.check()?;
    // A few cluster centres of the ring's features (k-means, deterministic start).
    let kc = 6.min(ring.len());
    let mut f = vec![0.0f32; nf];
    let mut centres: Vec<Vec<f32>> = (0..kc)
        .map(|j| {
            feat(ring[j * ring.len() / kc], &mut f);
            f.clone()
        })
        .collect();
    let d2 = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    for _ in 0..8 {
        let mut acc = vec![vec![0.0f32; nf]; kc];
        let mut n = vec![0usize; kc];
        for &i in &ring {
            feat(i, &mut f);
            let j = (0..kc).min_by(|&a, &b| d2(&f, &centres[a]).total_cmp(&d2(&f, &centres[b]))).unwrap_or(0);
            acc[j].iter_mut().zip(&f).for_each(|(a, v)| *a += v);
            n[j] += 1;
        }
        for j in 0..kc {
            if n[j] > 0 {
                centres[j] = acc[j].iter().map(|v| v / n[j] as f32).collect();
            }
        }
    }
    let nearest = |i: usize, f: &mut [f32]| -> f32 {
        feat(i, f);
        centres.iter().map(|c| d2(f, c)).fold(f32::INFINITY, f32::min).sqrt()
    };
    // How much the ring itself varies around its clusters sets the scale of "similar".
    let mut spread: Vec<f32> = ring.iter().map(|&i| nearest(i, &mut f)).collect();
    let q = (spread.len() * 9 / 10).min(spread.len() - 1);
    let (_, s90, _) = spread.select_nth_unstable_by(q, |a, b| a.total_cmp(b));
    let tau = (*s90 * 1.6).max(0.03);
    let mut cost_fg = vec![0.0f32; sw * sh];
    let mut cost_bg = vec![0.0f32; sw * sh];
    let mut fixed = vec![FREE; sw * sh];
    // Always the hole's own edge, and around a thin selection each part's surroundings.
    let keep = thin_band(sw, sh, &shole);
    for i in 0..sw * sh {
        if shole[i] || dist[i] <= keep {
            fixed[i] = HARD_FG;
            continue;
        }
        let d = nearest(i, &mut f) / tau;
        cost_fg[i] = d * d;
        cost_bg[i] = 1.0;
    }
    let rgb = RgbImage::from_fn(sw, sh, |x, y| {
        let i = y * sw + x;
        let c = |j: usize| means[j.min(nc - 1)][i];
        [c(0), c(1), c(2)]
    });
    // Contrast-sensitive smoothness (the weight of a squared colour difference). The scale of
    // "similar" and this weight were fitted to Photoshop's Auto overlays (mean IoU 0.92 on ten
    // synthetic and public-domain cases).
    ctl.check()?;
    let area =
        grid_cut_cancellable(&rgb, &cost_fg, &cost_bg, &fixed, |d2| 8.0 * (-d2 / (2.0 * 0.05 * 0.05)).exp() + 0.05, &|| ctl.cancelled()).ok_or(Cancelled)?;
    let mut out = vec![false; w * h];
    par::chunks_mut(&mut out, w, true, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = !hole[y * w + x] && area[(y / k) * sw + x / k];
        }
    });
    // Too little to copy from (fewer source pixels than twice the hole): sample everything.
    let n_hole = par::count(hole.len(), |i| hole[i]);
    if par::count(out.len(), |i| out[i]) < 2 * n_hole.max(64) {
        return Ok(everything());
    }
    Ok(out)
}

/// How far around a thin selection (strings, wires, and any small parts selected with them, all
/// at most a quarter as thick as the selection is long) the Auto area always reaches, in cells of
/// a `w × h` grid: as far as the selection is thick, so that each part, however far from the
/// rest, keeps its own surroundings to copy from. 1.5 cells for any other selection.
fn thin_band(w: usize, h: usize, hole: &[bool]) -> f32 {
    let inside = crate::selection::edt(&hole.iter().map(|h| !h).collect::<Vec<_>>(), w, h);
    let thickness = 2.0 * inside.iter().zip(hole).filter(|(_, h)| **h).map(|(d, _)| *d).fold(0.0f32, f32::max);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (i, _) in hole.iter().enumerate().filter(|(_, h)| **h) {
        let (x, y) = (i % w, i / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
    }
    let extent = x1.saturating_sub(x0).max(y1.saturating_sub(y0)) as f32;
    if 4.0 * thickness <= extent { thickness.max(1.5) } else { 1.5 }
}

/// The Patch Tool's and the Content-Aware Move Tool's Color option (0–10) as the colour adaptation
/// of the copied content. Measured on Photoshop 25.4 with content brighter than its new place: 0
/// keeps it as it is; 3 moves it about 0.12 (32 of 42 levels, and 31 of 102: a limit, not a
/// fraction), 6 about 80 % of 42 levels, 10 most of the way (35 of 42, 95 of 102). So the shift is
/// limited to about `0.04 · color`, the contrast change likewise widening.
pub fn color_amount(color: u8) -> Adapt {
    if color == 0 {
        return Adapt::NONE;
    }
    let c = f32::from(color.min(10));
    Adapt { gain: (1.0 - 0.033 * c, 1.0 + 0.05 * c), bias: 0.04 * c }
}

/// Gain and bias per channel taking the samples `from` to the level and contrast of the samples
/// `to` (both interleaved, `ch` per sample), within `a`: how copied content is fitted to its new
/// surroundings.
pub fn level_transform(ch: usize, from: &[f32], to: &[f32], a: Adapt) -> Vec<(f32, f32)> {
    let moments = |v: &[f32], c: usize| {
        let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for x in v.chunks_exact(ch.max(1)) {
            s += f64::from(x[c]);
            s2 += f64::from(x[c]) * f64::from(x[c]);
            n += 1.0;
        }
        (s, s2, n)
    };
    (0..ch)
        .map(|c| {
            let (fs, fs2, fnn) = moments(from, c);
            let (ts, ts2, tn) = moments(to, c);
            if a.is_none() || fnn == 0.0 || tn == 0.0 {
                return (1.0, 0.0);
            }
            // Both sets scaled to the same count for the fit.
            let k = fnn / tn;
            crate::inpaint::fit(a, fnn as f32, (ts * k) as f32, (ts2 * k) as f32, fs as f32, fs2 as f32)
        })
        .collect()
}

/// One stroke of the Content-Aware Fill workspace's Sampling Brush: a hard round brush of
/// diameter `size` along `points` (document px), adding to the sampling area or taking from it.
#[derive(Clone, Debug, PartialEq)]
pub struct SamplingStroke {
    pub add: bool,
    pub size: f64,
    pub points: Vec<[f64; 2]>,
}

impl SamplingStroke {
    /// Bounds of the pixels the stroke covers.
    pub fn bounds(&self) -> Rect {
        let r = (self.size / 2.0).max(0.5);
        self.points.iter().fold(Rect::EMPTY, |acc, p| {
            let b = Rect::new(
                (p[0] - r).floor() as i32,
                (p[1] - r).floor() as i32,
                ((p[0] + r).ceil() as i32).saturating_add(1),
                ((p[1] + r).ceil() as i32).saturating_add(1),
            );
            if acc.is_empty() { b } else { acc.union(&b) }
        })
    }
}

/// Squared distance from `p` to the segment `a`–`b`.
fn seg_dist2(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    qx * qx + qy * qy
}

/// Paint `strokes` in order into `mask` (row-major over `area`): a pixel whose centre lies within
/// half the brush size of a stroke's path is set (adding) or cleared (subtracting). A mask that
/// does not cover `area` exactly is left as it is.
pub fn paint_strokes(mask: &mut [bool], area: Rect, strokes: &[SamplingStroke]) {
    let w = area.width() as usize;
    if mask.len() != w * area.height() as usize {
        return;
    }
    for s in strokes {
        let r = (s.size / 2.0).max(0.5);
        let r2 = r.powi(2);
        let segs: Vec<([f64; 2], [f64; 2])> = match s.points.as_slice() {
            [] => continue,
            [p] => vec![(*p, *p)],
            pts => pts.windows(2).map(|p| (p[0], p[1])).collect(),
        };
        // A stroke covers the union of its segments' pixels: each segment only looks at its own
        // bounds.
        for (a, b) in segs {
            let bounds = Rect::new(
                (a[0].min(b[0]) - r).floor() as i32,
                (a[1].min(b[1]) - r).floor() as i32,
                ((a[0].max(b[0]) + r).ceil() as i32).saturating_add(1),
                ((a[1].max(b[1]) + r).ceil() as i32).saturating_add(1),
            )
            .intersect(&area);
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    let c = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                    if seg_dist2(c, a, b) <= r2
                        && let Some(m) = mask.get_mut((y - area.y0) as usize * w + (x - area.x0) as usize)
                    {
                        *m = s.add;
                    }
                }
            }
        }
    }
}

/// The pixels of `area` (row-major) whose centres lie inside the polygon `pts` (even-odd rule):
/// what a lasso outlines.
pub fn polygon_mask(area: Rect, pts: &[[f64; 2]]) -> Vec<bool> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out = vec![false; w * h];
    if pts.len() < 3 {
        return out;
    }
    let mut xs: Vec<f64> = Vec::new();
    for row in 0..h {
        let cy = f64::from(area.y0) + row as f64 + 0.5;
        xs.clear();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            if (a[1] <= cy) != (b[1] <= cy) {
                xs.push(a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            // Pixel centres x + 0.5 within [pair0, pair1).
            let x0 = ((pair[0] - 0.5).ceil() as i64 - i64::from(area.x0)).clamp(0, w as i64) as usize;
            let x1 = ((pair[1] - 0.5).ceil() as i64 - i64::from(area.x0)).clamp(0, w as i64) as usize;
            out[row * w + x0..row * w + x1.max(x0)].iter_mut().for_each(|v| *v = true);
        }
    }
    out
}

/// Grow (`by` > 0) or shrink (`by` < 0) a mask by `|by|` pixels (Euclidean): the lasso's Expand
/// and Contract.
pub fn grow_mask(mask: &[bool], w: usize, h: usize, by: i32) -> Vec<bool> {
    if by == 0 || w == 0 || h == 0 {
        return mask.to_vec();
    }
    let r = f32::from(u16::try_from(by.unsigned_abs()).unwrap_or(u16::MAX));
    if by > 0 {
        let d = crate::selection::edt(mask, w, h);
        d.iter().map(|v| *v <= r).collect()
    } else {
        let outside: Vec<bool> = mask.iter().map(|v| !v).collect();
        let d = crate::selection::edt(&outside, w, h);
        d.iter().map(|v| *v > r).collect()
    }
}

/// The Sampling Brush's starting size for a selection with bounds `hole`: a tenth of the sampling
/// window's side, as Photoshop opens the workspace (20 px for a 50 px selection, 40 for 100).
pub fn default_brush_size(hole: Rect) -> f64 {
    let (w, h) = (f64::from(hole.width().max(50)), f64::from(hole.height().max(50)));
    (0.4 * (w * h).sqrt()).round()
}

#[cfg(test)]
mod structure_continue_tests;
#[cfg(test)]
mod tests;
