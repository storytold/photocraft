//! Generic dab-driven stroke machinery for the retouching tools (Clone Stamp, Healing Brush, Spot
//! Healing, Dodge/Burn/Sponge, Blur/Sharpen, Smudge, History Brush).
//!
//! Two stroke shapes cover every retouching tool:
//!
//! * **Accumulate, then composite** ([`stroke_coverage`] + [`apply_coverage`]): the stroke's dabs build
//!   up one coverage map (flow builds up, capped at 1), and a *paint image* computed once from the
//!   pre-stroke state (a cloned source, a healed patch, an earlier history state…) is composited
//!   through it at the stroke's opacity. Pixels painted earlier in the same stroke are never re-sampled.
//! * **Sequential dabs** ([`apply_dab_stroke`]): each dab edits a working copy in place (so dab *n*
//!   sees the result of dab *n − 1*, as Dodge, Blur or Smudge need), and the finished working copy is
//!   mixed back at the stroke's opacity.
//!
//! All pixel data here is the surface's *native* channels as normalised `f32` (any depth, any colour
//! model); colour-space-specific work happens in the callbacks.

use std::convert::Infallible;

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::{Dab, Stroke, dab_coverage, dabs};

/// A rectangle of native-channel pixels (`rect.width() × rect.height() × ch` floats, row-major).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub ch: usize,
    pub data: Vec<f32>,
}

impl Region {
    /// A zeroed region.
    pub fn new(rect: Rect, ch: usize) -> Self {
        Self { rect, ch, data: vec![0.0; rect.width() as usize * rect.height() as usize * ch] }
    }
    /// Read `rect` from a surface.
    pub fn read(s: &Surface, rect: Rect) -> Self {
        Self { rect, ch: s.channels(), data: s.read_region(rect) }
    }
    pub fn width(&self) -> usize {
        self.rect.width() as usize
    }
    pub fn height(&self) -> usize {
        self.rect.height() as usize
    }
    /// Pixel index of document coordinate `(x, y)` (must lie inside `rect`).
    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y - self.rect.y0) as usize * self.width() + (x - self.rect.x0) as usize
    }
    #[inline]
    pub fn px(&self, x: i32, y: i32) -> &[f32] {
        let i = self.index(x, y) * self.ch;
        &self.data[i..i + self.ch]
    }
    #[inline]
    pub fn px_mut(&mut self, x: i32, y: i32) -> &mut [f32] {
        let i = self.index(x, y) * self.ch;
        let n = self.ch;
        &mut self.data[i..i + n]
    }
    /// Copy of the sub-rectangle `r` (clipped to this region).
    pub fn crop(&self, r: Rect) -> Region {
        let r = r.intersect(&self.rect);
        let mut out = Region::new(r, self.ch);
        let (w, n) = (r.width() as usize, self.ch);
        for y in r.y0..r.y1 {
            let s = self.index(r.x0, y) * n;
            let d = (y - r.y0) as usize * w * n;
            out.data[d..d + w * n].copy_from_slice(&self.data[s..s + w * n]);
        }
        out
    }
    /// Write this region into a surface.
    pub fn write(&self, s: &mut Surface) {
        s.write_region(self.rect, &self.data);
    }
}

/// One dab's footprint: coverage (hardness falloff × dab alpha) over its bounding rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub dab: Dab,
    /// Position of the dab in the stroke (0 = first).
    pub index: usize,
    pub rect: Rect,
    /// `rect.width() × rect.height()` coverage values in 0..1 (already multiplied by flow/pressure).
    pub cov: Vec<f32>,
}

impl Footprint {
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> f32 {
        self.cov[(y - self.rect.y0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize]
    }
}

/// Bounding rectangle of a dab (1 px of anti-aliasing slack included).
pub fn dab_rect(d: &Dab) -> Rect {
    crate::render::rect_around(d, d.radius)
}

/// Rasterize one dab's coverage.
pub fn footprint(d: &Dab, index: usize, hardness: f32) -> Footprint {
    // A soft tip paints past its nominal radius.
    let rect = crate::render::rect_around(d, crate::tip_reach(d.radius, hardness).max(d.radius));
    let (cx, cy) = (d.center.x as f32, d.center.y as f32);
    let mut cov = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            cov.push(dab_coverage(dist, d.radius, hardness) * d.alpha);
        }
    }
    Footprint { dab: *d, index, rect, cov }
}

/// One dab's footprint over just the part of its rectangle inside `clip`: what
/// [`BrushContext::footprint`](crate::BrushContext::footprint) gives, without rasterising the rest.
/// Coverage is computed pixel by pixel, so the two agree wherever both have a value.
pub fn footprint_within(ctx: &crate::BrushContext, d: &Dab, index: usize, clip: Rect) -> Footprint {
    let rect = ctx.dab_rect(d, false).intersect(&clip);
    let mut cov = Vec::new();
    ctx.rasterize(d, false, rect, &mut cov);
    Footprint { dab: *d, index, rect, cov }
}

/// Union of the dabs' rectangles.
pub fn stroke_bounds(ds: &[Dab]) -> Rect {
    ds.iter().fold(Rect::EMPTY, |r, d| r.union(&dab_rect(d)))
}

/// Accumulated coverage of a whole stroke: dabs build up (`c ← c + d·(ceil − c)`, "flow"), capped at
/// each dab's opacity ceiling; stroke-level masks (dual brush, texture) applied. Returns the stroke
/// bounds and `width × height` coverage values. Uses the full brush engine (tips and dynamics).
pub fn stroke_coverage(stroke: &Stroke) -> (Rect, Vec<f32>) {
    let mut r = crate::StrokeRenderer::new(&stroke.brush, None, 1.0);
    r.push(&stroke.points);
    r.finish();
    r.dense_coverage()
}

/// Index of the alpha channel, if the format has one.
#[inline]
pub fn alpha_index(fmt: &PixelFormat) -> Option<usize> {
    fmt.alpha.then(|| fmt.mode.color_channels())
}

/// Normal-mode "source over destination" at coverage `k`, in native channels (colour channels are
/// channel-agnostic, so this works for RGB, Gray, CMYK, Lab… at any depth).
#[inline]
pub fn over_native(fmt: &PixelFormat, dst: &mut [f32], src: &[f32], k: f32, lock_transparency: bool) {
    match alpha_index(fmt) {
        Some(a) => {
            let (da, sa) = (dst[a], src[a] * k);
            let oa = sa + da * (1.0 - sa);
            if oa > 0.0 {
                for c in 0..a {
                    dst[c] = (src[c] * sa + dst[c] * da * (1.0 - sa)) / oa;
                }
            }
            if !lock_transparency {
                dst[a] = oa;
            }
        }
        None => {
            for c in 0..dst.len() {
                dst[c] += (src[c] - dst[c]) * k;
            }
        }
    }
}

/// Composite a paint image through a coverage map onto `target`.
///
/// `paint` must cover `rect`; `cov` has one value per pixel of `rect`. The per-pixel weight is
/// `cov × opacity × selection`. Non-normal blend modes composite in straight RGBA (W3C formula).
#[allow(clippy::too_many_arguments)]
pub fn apply_coverage(
    target: &mut Surface,
    rect: Rect,
    cov: &[f32],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    paint: &Region,
    blend: BlendMode,
) -> Rect {
    if rect.is_empty() {
        return Rect::EMPTY;
    }
    let fmt = target.format();
    let n = fmt.channels();
    let mut region = target.read_region(rect);
    let w = rect.width() as usize;
    let a = alpha_index(&fmt);
    for (i, c) in cov.iter().enumerate() {
        let x = rect.x0 + (i % w) as i32;
        let y = rect.y0 + (i / w) as i32;
        let sel = selection.map_or(1.0, |s| s.sample_channel(x, y, 0));
        let k = (c * opacity * sel).min(1.0);
        if k <= 0.0 {
            continue;
        }
        let px = &mut region[i * n..(i + 1) * n];
        if lock_transparency && a.is_some_and(|a| px[a] <= 0.0) {
            continue;
        }
        let src = paint.px(x, y);
        if blend == BlendMode::Normal {
            over_native(&fmt, px, src, k, lock_transparency);
        } else {
            let d = to_rgba(&fmt, px);
            let s = to_rgba(&fmt, src);
            let mut o = photocraft_color::blend::composite(blend, d, s, k);
            if lock_transparency {
                o[3] = d[3];
            }
            let mut enc = [0.0f32; 8];
            from_rgba_into(&fmt, o, &mut enc);
            px.copy_from_slice(&enc[..n]);
        }
    }
    target.write_region(rect, &region);
    rect
}

/// Run a sequential-dab stroke: `effect(work, footprint)` edits the working copy for each dab in
/// order; the result is mixed back at `stroke.brush.opacity × selection`.
///
/// The working copy covers the stroke bounds grown by `halo` pixels so neighbourhood effects (blur,
/// sharpen) can read around each dab; only the stroke bounds are written back. With
/// `lock_transparency`, alpha is preserved. Returns the damaged rectangle. Nothing is clipped: see
/// [`apply_dab_stroke_clipped`] to keep the stroke to the canvas.
pub fn apply_dab_stroke(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    halo: i32,
    effect: impl FnMut(&mut Region, &Footprint),
) -> Rect {
    // A clip no dab reaches past keeps every rectangle exactly what it was before the clip existed.
    let everywhere = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let Ok(damage) = apply_dab_stroke_clipped(target, stroke, selection, lock_transparency, halo, everywhere, |_| Ok::<(), Infallible>(()), effect);
    damage
}

/// [`apply_dab_stroke`] that changes only the pixels inside `clip` (the canvas). The working copy
/// covers the dabs grown by `halo`, but never more than `clip` grown by `halo`, so a brush far
/// larger than the canvas costs what the canvas does (#5633): unclipped, one soft 5000 px dab
/// copies about 8,600 × 8,600 px several times over, whatever the image size. The halo ring past
/// `clip` is read and edited like the rest, so a neighbourhood effect still sees pixels beyond the
/// edge, but it is never written back.
///
/// `admit` gets the working rectangle before anything is allocated; its error ends the stroke with
/// `target` untouched (the caller's memory budget). Returns the damaged rectangle, inside `clip`.
#[allow(clippy::too_many_arguments)]
pub fn apply_dab_stroke_clipped<E>(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    halo: i32,
    clip: Rect,
    admit: impl FnOnce(Rect) -> Result<(), E>,
    mut effect: impl FnMut(&mut Region, &Footprint),
) -> Result<Rect, E> {
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let ctx = crate::BrushContext::new(&stroke.brush);
    let reach = ds.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
    let bounds = reach.intersect(&clip);
    if bounds.is_empty() {
        // Nothing to change, and no working copy for an effect to read: Smudge clamps into it.
        return Ok(Rect::EMPTY);
    }
    let outer = reach.inflate(halo).intersect(&clip.inflate(halo));
    admit(outer)?;
    let orig = Region::read(target, outer);
    let mut work = orig.clone();
    for (i, d) in ds.iter().enumerate() {
        // A dab off the canvas does nothing, as in the live preview: a Smudge stroke started past
        // the edge picks up paint where it first touches the canvas.
        if ctx.dab_rect(d, false).intersect(&clip).is_empty() {
            continue;
        }
        effect(&mut work, &footprint_within(&ctx, d, i, outer));
    }
    mix_back(target, &orig, &work, bounds, stroke.brush.opacity, selection, lock_transparency);
    Ok(bounds)
}

/// Mix a sequential-dab stroke's working pixels `work` back over the original `orig` at
/// `opacity × selection` inside `rect`, and write that area to `target`: the last step of
/// [`apply_dab_stroke`], shared with live previews that run the dabs as they come.
pub fn mix_back(target: &mut Surface, orig: &Region, work: &Region, rect: Rect, opacity: f32, selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = target.format();
    let a = alpha_index(&fmt);
    let mut out = orig.crop(rect);
    let n = out.ch;
    let opacity = opacity.clamp(0.0, 1.0);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let k = opacity * selection.map_or(1.0, |s| s.sample_channel(x, y, 0));
            if k <= 0.0 {
                continue;
            }
            let wv = work.px(x, y);
            let i = out.index(x, y) * n;
            let o = &mut out.data[i..i + n];
            match a {
                // Alpha changes: mix premultiplied, so colour under transparent pixels doesn't bleed in.
                Some(a) if !lock_transparency => mix_premultiplied(o, wv, a, k),
                _ => {
                    for c in 0..n {
                        if lock_transparency && Some(c) == a {
                            continue;
                        }
                        o[c] += (wv[c] - o[c]) * k;
                    }
                }
            }
        }
    }
    out.write(target);
}

/// `dst ← lerp(dst, src, k)` in premultiplied space (straight in, straight out; `a` = alpha index).
/// Where the result is fully transparent the colour is mixed straight, so it stays defined.
#[inline]
pub fn mix_premultiplied(dst: &mut [f32], src: &[f32], a: usize, k: f32) {
    let (Some(&da), Some(&sa)) = (dst.get(a), src.get(a)) else { return };
    let oa = da + (sa - da) * k;
    for (c, (d, s)) in dst.iter_mut().zip(src).enumerate() {
        if c == a {
            continue;
        }
        *d = if oa > 1e-6 { (*d * da + (*s * sa - *d * da) * k) / oa } else { *d + (*s - *d) * k };
    }
    if let Some(d) = dst.get_mut(a) {
        *d = oa;
    }
}

/// Smudge state: a colour buffer carried from dab to dab (centred on the dab), which is laid down at
/// `strength` and then re-picks-up the pixels under the dab. Use with [`apply_dab_stroke`].
#[derive(Clone, Debug)]
pub struct Smudge {
    /// 0..1: how much carried colour each dab lays down (and keeps).
    pub strength: f32,
    /// Finger painting: start the stroke with this native-channel colour instead of the pixels under
    /// the first dab.
    pub finger: Option<Vec<f32>>,
    radius: i32,
    /// Alpha channel index: when set, the carried colour is premultiplied, so smudging into or out
    /// of transparency never drags in the (meaningless) colour of transparent pixels.
    alpha: Option<usize>,
    /// Carried colour (premultiplied when `alpha` is set).
    carry: Option<Vec<f32>>,
}

impl Smudge {
    pub fn new(strength: f32, finger: Option<Vec<f32>>, max_brush_size: f32) -> Self {
        Self { strength: strength.clamp(0.0, 1.0), finger, radius: (max_brush_size / 2.0).ceil() as i32 + 2, alpha: None, carry: None }
    }

    /// Mix in premultiplied space using alpha channel `a` (the format's [`alpha_index`]).
    pub fn with_alpha(mut self, a: Option<usize>) -> Self {
        self.alpha = a;
        self
    }

    /// Apply one dab to the working copy.
    pub fn dab(&mut self, work: &mut Region, fp: &Footprint) {
        let (r, n) = (self.radius, work.ch);
        let side = (2 * r + 1) as usize;
        let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
        let wr = work.rect;
        let alpha = self.alpha.filter(|a| *a < n);
        let clamp_px = |work: &Region, x: i32, y: i32| -> Vec<f32> {
            let mut v = work.px(x.clamp(wr.x0, wr.x1 - 1), y.clamp(wr.y0, wr.y1 - 1)).to_vec();
            premultiply(&mut v, alpha);
            v
        };
        let carry = self.carry.get_or_insert_with(|| {
            let mut c = vec![0.0f32; side * side * n];
            for qy in 0..side {
                for qx in 0..side {
                    let v = match &self.finger {
                        Some(f) => {
                            let mut f = f.clone();
                            f.resize(n, 1.0);
                            premultiply(&mut f, alpha);
                            f
                        }
                        None => clamp_px(work, cx + qx as i32 - r, cy + qy as i32 - r),
                    };
                    c[(qy * side + qx) * n..(qy * side + qx + 1) * n].copy_from_slice(&v[..n]);
                }
            }
            c
        });
        // Lay down the carried colour.
        for y in fp.rect.y0.max(wr.y0)..fp.rect.y1.min(wr.y1) {
            for x in fp.rect.x0.max(wr.x0)..fp.rect.x1.min(wr.x1) {
                let k = fp.at(x, y) * self.strength;
                let (qx, qy) = (x - cx + r, y - cy + r);
                if k <= 0.0 || qx < 0 || qy < 0 || qx >= side as i32 || qy >= side as i32 {
                    continue;
                }
                let q = (qy as usize * side + qx as usize) * n;
                let px = work.px_mut(x, y);
                match alpha {
                    Some(a) => {
                        // Back to straight for the mix (premultiplied lerp towards the carried colour).
                        let ca = carry[q + a];
                        let mut buf = [0.0f32; 16];
                        let m = n.min(buf.len());
                        buf[..m].copy_from_slice(&carry[q..q + m]);
                        let src = &mut buf[..m];
                        if ca > 1e-6 {
                            for (c, v) in src.iter_mut().enumerate() {
                                if c != a {
                                    *v /= ca;
                                }
                            }
                        }
                        mix_premultiplied(px, src, a, k);
                    }
                    None => {
                        for c in 0..n {
                            px[c] += (carry[q + c] - px[c]) * k;
                        }
                    }
                }
            }
        }
        // Pick up: the carried colour keeps `strength` of itself.
        let keep = self.strength;
        for qy in 0..side {
            for qx in 0..side {
                let (x, y) = (cx + qx as i32 - r, cy + qy as i32 - r);
                if !wr.contains(x, y) {
                    continue;
                }
                let q = (qy * side + qx) * n;
                let px = work.px(x, y);
                let pa = alpha.map_or(1.0, |a| px[a]);
                for c in 0..n {
                    let v = if alpha.is_some_and(|a| a != c) { px[c] * pa } else { px[c] };
                    carry[q + c] = carry[q + c] * keep + v * (1.0 - keep);
                }
            }
        }
    }
}

/// Premultiply a straight pixel in place (no-op without alpha).
fn premultiply(px: &mut [f32], alpha: Option<usize>) {
    let Some(pa) = alpha.and_then(|a| px.get(a).copied()) else { return };
    for (c, v) in px.iter_mut().enumerate() {
        if Some(c) != alpha {
            *v *= pa;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_geom::Point;

    fn stroke(points: &[(f64, f64)], size: f32, hardness: f32) -> Stroke {
        Stroke {
            brush: BrushSettings { size, hardness, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
        }
    }

    #[test]
    fn coverage_matches_apply_stroke_bounds() {
        let st = stroke(&[(10.0, 10.0), (40.0, 12.0)], 8.0, 1.0);
        let (r, cov) = stroke_coverage(&st);
        assert!(r.contains(10, 10) && r.contains(40, 12));
        let w = r.width() as usize;
        assert!((cov[(10 - r.y0) as usize * w + (25 - r.x0) as usize] - 1.0).abs() < 1e-6);
        assert!(cov.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn coverage_composite_copies_exactly_at_full_hardness() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8, PixelFormat::GRAYA8] {
            let mut s = Surface::new(fmt);
            s.fill_rect(
                Rect::new(0, 0, 64, 64),
                &vec![0.2; fmt.channels()].iter().enumerate().map(|(i, v)| if i + 1 == fmt.channels() { 1.0 } else { *v }).collect::<Vec<_>>(),
            );
            let st = stroke(&[(20.0, 20.0)], 10.0, 1.0);
            let (r, cov) = stroke_coverage(&st);
            let mut paint = Region::new(r, fmt.channels());
            let n = fmt.channels();
            paint.data.iter_mut().enumerate().for_each(|(i, v)| *v = if fmt.alpha && i % n == n - 1 { 1.0 } else { 0.8 });
            apply_coverage(&mut s, r, &cov, 1.0, None, false, &paint, BlendMode::Normal);
            let p = s.pixel(20, 20);
            assert!(p[..n - 1].iter().all(|v| (v - 0.8).abs() < 1e-2) && p[n - 1] == 1.0, "{fmt:?} {p:?}");
            assert!((s.pixel(40, 40)[0] - 0.2).abs() < 1e-2);
        }
    }

    #[test]
    fn dab_stroke_runs_sequentially_and_respects_selection() {
        let mut s = Surface::new(PixelFormat::GRAY8);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 30, 100), &[1.0]);
        let st = stroke(&[(10.0, 10.0), (50.0, 10.0)], 6.0, 1.0);
        let mut seen = Vec::new();
        apply_dab_stroke(&mut s, &st, Some(&sel), false, 2, |work, fp| {
            seen.push(fp.index);
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    if fp.at(x, y) > 0.5 {
                        work.px_mut(x, y)[0] += 0.1;
                    }
                }
            }
        });
        assert!(seen.windows(2).all(|w| w[1] == w[0] + 1));
        assert!(s.pixel(20, 10)[0] > 0.1, "dabs accumulate");
        assert_eq!(s.pixel(40, 10)[0], 0.0, "outside the selection untouched");
    }

    /// A neighbourhood effect for the clip tests: a 3 × 3 mean, reading clamped to the working
    /// copy, laid down at the dab's coverage.
    fn box_blur(work: &mut Region, fp: &Footprint) {
        let (r, wr, n) = (fp.rect.intersect(&work.rect), work.rect, work.ch);
        let src = work.clone();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y);
                let mut mean = vec![0.0; n];
                for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                    let p = src.px((x + dx).clamp(wr.x0, wr.x1 - 1), (y + dy).clamp(wr.y0, wr.y1 - 1));
                    mean.iter_mut().zip(p).for_each(|(m, v)| *m += v / 9.0);
                }
                work.px_mut(x, y).iter_mut().zip(&mean).for_each(|(v, m)| *v += (m - *v) * k);
            }
        }
    }

    /// A 64 × 48 canvas of `fmt` with a pattern in every channel.
    fn patterned(fmt: PixelFormat) -> Surface {
        let mut s = Surface::new(fmt);
        let n = fmt.channels();
        let data: Vec<f32> = (0..64 * 48 * n).map(|i| ((i * 37) % 101) as f32 / 100.0).collect();
        s.write_region(Rect::new(0, 0, 64, 48), &data);
        s
    }

    #[test]
    fn clipped_dab_stroke_works_inside_the_clip_plus_halo() {
        // #5633: a 300 px soft tip on a 32 × 24 canvas reaches about 260 px past every edge.
        let clip = Rect::new(0, 0, 32, 24);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(clip, &[0.2, 0.4, 0.6, 1.0]);
        s.fill_rect(Rect::new(0, 0, 16, 24), &[0.9, 0.1, 0.1, 1.0]);
        let tiles = s.tile_bounds();
        let st = stroke(&[(4.0, 6.0), (28.0, 18.0)], 300.0, 0.0);
        let (mut admitted, mut seen) = (None, Vec::new());
        let dmg = apply_dab_stroke_clipped(
            &mut s,
            &st,
            None,
            false,
            13,
            clip,
            |area| {
                admitted = Some(area);
                Ok::<(), ()>(())
            },
            |work, fp| {
                seen.push((work.rect, fp.rect));
                box_blur(work, fp);
            },
        )
        .unwrap();
        assert!(admitted.is_some_and(|a| clip.inflate(13).contains_rect(&a)), "{admitted:?}");
        assert!(!seen.is_empty() && seen.iter().all(|(w, f)| clip.inflate(13).contains_rect(w) && w.contains_rect(f)), "{seen:?}");
        assert_eq!(dmg, clip);
        assert_eq!(s.tile_bounds(), tiles, "nothing written off the clip");
        assert!(s.pixel(16, 12)[0] < 0.85, "the stroke blurred the canvas");
    }

    #[test]
    fn clipped_dab_stroke_matches_the_unclipped_one_inside_the_clip() {
        // Dabs and halo inside the clip: clipping changes nothing, in any format.
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
            let st = stroke(&[(24.0, 20.0), (40.0, 28.0)], 10.0, 0.5);
            let mut clipped = patterned(fmt);
            let mut unclipped = clipped.clone();
            let ok = apply_dab_stroke_clipped(&mut clipped, &st, None, false, 2, Rect::new(0, 0, 64, 48), |_| Ok::<(), ()>(()), box_blur);
            let want = apply_dab_stroke(&mut unclipped, &st, None, false, 2, box_blur);
            assert_eq!(ok, Ok(want), "{fmt:?}");
            let around = Rect::new(-64, -64, 128, 112);
            assert!(clipped.read_region(around) == unclipped.read_region(around), "{fmt:?}");
        }
    }

    #[test]
    fn footprint_within_is_the_full_footprint_cut_to_the_clip() {
        // Hard and soft tips (the vectorised round path), noise (the per-pixel path) and a tiny
        // tip (the supersampled path).
        let brushes = [
            BrushSettings { size: 30.0, hardness: 1.0, ..Default::default() },
            BrushSettings { size: 30.0, hardness: 0.0, ..Default::default() },
            BrushSettings { size: 30.0, hardness: 0.3, noise: true, ..Default::default() },
            BrushSettings { size: 3.0, hardness: 0.8, ..Default::default() },
        ];
        for b in brushes {
            let ctx = crate::BrushContext::new(&b);
            let d = Dab::round(Point::new(20.3, 17.6), b.size / 2.0, 0.9);
            let full = ctx.footprint(&d, 4);
            for clip in [Rect::new(17, 3, 70, 19), Rect::new(-40, -40, 21, 18), Rect::new(200, 200, 210, 210)] {
                let part = footprint_within(&ctx, &d, 4, clip);
                assert_eq!((part.rect, part.index), (full.rect.intersect(&clip), 4), "size {} clip {clip:?}", b.size);
                assert_eq!(part.cov.len(), part.rect.width() as usize * part.rect.height() as usize);
                for y in part.rect.y0..part.rect.y1 {
                    for x in part.rect.x0..part.rect.x1 {
                        assert_eq!(part.at(x, y).to_bits(), full.at(x, y).to_bits(), "size {} ({x},{y})", b.size);
                    }
                }
            }
        }
    }

    #[test]
    fn clipped_dab_stroke_off_the_clip_paints_nothing() {
        // No dab touches the clip: no working copy, no effect call, no admit, so Smudge (which
        // clamps into the working copy) never sees an empty one.
        let mut s = patterned(PixelFormat::RGBA8);
        let before = s.clone();
        let st = stroke(&[(200.0, 200.0), (260.0, 210.0)], 10.0, 1.0);
        let mut sm = Smudge::new(0.8, None, 10.0);
        let mut admitted = false;
        let dmg = apply_dab_stroke_clipped(
            &mut s,
            &st,
            None,
            false,
            0,
            Rect::new(0, 0, 64, 48),
            |_| {
                admitted = true;
                Ok::<(), ()>(())
            },
            |w, fp| sm.dab(w, fp),
        );
        assert_eq!(dmg, Ok(Rect::EMPTY));
        assert!(!admitted && s.tile_bounds() == before.tile_bounds() && s.read_region(Rect::new(0, 0, 64, 48)) == before.read_region(Rect::new(0, 0, 64, 48)));
    }

    #[test]
    fn refused_clipped_dab_stroke_leaves_the_target_untouched() {
        let mut s = patterned(PixelFormat::RGBA16);
        let before = s.read_region(Rect::new(0, 0, 64, 48));
        let st = stroke(&[(10.0, 10.0), (50.0, 30.0)], 12.0, 0.5);
        let mut called = false;
        let r = apply_dab_stroke_clipped(&mut s, &st, None, false, 2, Rect::new(0, 0, 64, 48), |_| Err("over budget"), |_, _| called = true);
        assert_eq!(r, Err("over budget"));
        assert!(!called && s.read_region(Rect::new(0, 0, 64, 48)) == before);
    }

    #[test]
    fn smudge_drags_colour_along_the_stroke() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.0, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 1.0);
            let mut sm = Smudge::new(0.8, None, 12.0);
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            // Red dragged to the right of the edge, fading with distance.
            let g = |x| s.pixel(x, 20)[1];
            assert!(g(35) < 0.5, "{fmt:?} {}", g(35));
            assert!(g(35) < g(60), "{fmt:?}");
            assert_eq!(s.pixel(35, 35)[1], 1.0, "off-stroke untouched");
            // Finger painting starts with the given colour.
            let mut s2 = Surface::new(fmt);
            s2.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            let mut fg = Smudge::new(0.8, Some(vec![0.0, 0.0, 1.0, 1.0]), 12.0);
            apply_dab_stroke(&mut s2, &st, None, false, 0, |w, fp| fg.dab(w, fp));
            assert!(s2.pixel(22, 20)[0] < 0.5);
        }
    }

    #[test]
    fn smudge_into_transparency_leaves_no_dark_fringe() {
        // #207: straight mixing dragged the black of transparent pixels into the colour.
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 0.5);
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            let mut smudged = 0;
            for x in 30..80 {
                for y in 14..27 {
                    let p = s.rgba(x, y);
                    if p[3] > 0.02 {
                        smudged += 1;
                        assert!(p[0] > 0.97 && (p[1] - 0.8).abs() < 0.03, "{fmt:?} ({x},{y}): {p:?} darkened");
                    }
                }
            }
            assert!(smudged > 50, "{fmt:?}: colour was dragged into the transparent area");
            // And at partial opacity the mix back is premultiplied too.
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let mut st = st.clone();
            st.brush.opacity = 0.5;
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            let p = s.rgba(34, 20);
            assert!(p[3] > 0.02 && p[0] > 0.97, "{fmt:?}: {p:?}");
        }
    }

    #[test]
    fn premultiplied_mix_ignores_colour_of_transparent_pixels() {
        let mut d = [0.0, 0.0, 0.0, 0.0];
        mix_premultiplied(&mut d, &[1.0, 0.5, 0.0, 1.0], 3, 0.25);
        assert_eq!(d, [1.0, 0.5, 0.0, 0.25]);
        let mut d = [0.2, 0.2, 0.2, 1.0];
        mix_premultiplied(&mut d, &[0.0, 0.0, 0.0, 0.0], 3, 1.0);
        assert_eq!(d[3], 0.0);
        // Out-of-range alpha index: untouched, no panic.
        let mut d = [0.5, 0.5];
        mix_premultiplied(&mut d, &[1.0, 1.0], 7, 0.5);
        assert_eq!(d, [0.5, 0.5]);
    }

    #[test]
    fn footprint_stays_inside_the_tips_reach() {
        let radius = 10.5f32;
        for hardness in [1.0, 0.5, 0.0] {
            let dab = Dab::round(Point::new(32.0, 32.0), radius, 1.0);
            let fp = footprint(&dab, 0, hardness);
            let mut painted = 0usize;
            let mut farthest = 0.0f64;
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    let dist = ((x as f64 + 0.5 - dab.center.x).powi(2) + (y as f64 + 0.5 - dab.center.y).powi(2)).sqrt();
                    if fp.at(x, y) <= 0.0 {
                        continue;
                    }
                    painted += 1;
                    farthest = farthest.max(dist);
                    let reach = f64::from(crate::tip_reach(radius, hardness));
                    assert!(
                        dist < reach.max(radius as f64) + 0.5,
                        "hardness {hardness}: pixel ({x}, {y}) at {dist} px from \
                         the centre is painted past the Full Size ring of \
                         {radius} px"
                    );
                }
            }
            assert!(painted > 0, "hardness {hardness}: nothing painted");
            if hardness == 1.0 {
                assert!(
                    farthest > radius as f64,
                    "a hard tip painted only {farthest} px out, so the Full \
                     Size ring at {radius} px is never reached"
                );
            }
        }
    }
}
