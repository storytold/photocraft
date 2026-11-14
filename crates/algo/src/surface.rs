//! Surface-aware painting: the photo's shading under a stroke, and the local projection of a
//! perspective plane.
//!
//! [`ShadingField`] measures how much brighter or darker each pixel of a sample is than its
//! surroundings: `S = Y / Y_ref`, where `Y` is the Rec. 709 luma and `Y_ref` is a Gaussian blur of
//! it (σ = the brush's Detail). Painting with the colour scaled by `S` leaves the stroke's average
//! colour as picked, with the folds and shadows of the photo showing through. The blurred field is
//! kept at reduced resolution and `S` is computed per requested pixel, so no full-resolution float
//! map exists and any tiling of a request gives the same values.
//!
//! [`PlaneSurface`] turns a [`vanishing::Scene`] into the local scale and foreshortening of the
//! plane under an image point, which the brush uses to size and squash its dabs.

use photocraft_color::ColorMode;
use photocraft_geom::Rect;

use crate::Ctx;
use crate::fxutil::{gauss_blur_n, rgba};
use crate::image::Image;
use crate::vanishing::Scene;

/// Most sample pixels a field is built over.
pub const MAX_FIELD_PIXELS: u64 = 1 << 28;
/// Most cells of the reduced-resolution luma grid; a larger area gets a coarser grid.
const MAX_GRID_CELLS: u64 = 1 << 22;
/// Largest blur σ in pixels.
pub const MAX_SIGMA: f32 = 4096.0;
/// Shading factors are clamped to this.
const MAX_SHADE: f32 = 4.0;
/// A blurred luma below this counts as this, so shading black never divides by ~0.
const MIN_REF: f32 = 1e-3;

/// Rec. 709 luma of straight RGB.
#[inline]
fn rec709(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Luma and weight (alpha) of a native pixel; `None` for non-finite or fully transparent ones.
#[inline]
fn luma_weight(ctx: &Ctx, px: &[f32]) -> Option<(f32, f32)> {
    if px.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // RGB needs no conversion (the same values `to_rgba` gives).
    let c = if ctx.mode == ColorMode::Rgb { [px[0], px[1], px[2], if ctx.alpha { px[3] } else { 1.0 }] } else { rgba(ctx, px) };
    let a = if ctx.alpha { c[3] } else { 1.0 };
    let y = rec709(c);
    (a > 0.0 && y.is_finite()).then_some((y, a.min(1.0)))
}

/// Luma is stored as a 16-bit square-root code over `0..=LUMA_MAX`: a fixed code, so a field is the
/// same however it was built in bands, with fine steps in the darks (about 0.4 % of a luma of
/// 0.001, 0.01 % of 1) and room above 1 for 32-bit float samples.
const LUMA_MAX: f32 = 16.0;
const CODE_MAX: f32 = 65534.0;
/// The code of a pixel that has no luma (transparent or non-finite).
const NO_LUMA: u16 = u16::MAX;

#[inline]
fn encode_luma(y: f32) -> u16 {
    ((y.max(0.0) / LUMA_MAX).min(1.0).sqrt() * CODE_MAX).round() as u16
}

#[inline]
fn decode_luma(code: u16) -> f32 {
    let q = f32::from(code) / CODE_MAX;
    q * q * LUMA_MAX
}

/// Pixels of the sample a band holds at most (a 16 byte float RGBA pixel each: 64 MiB).
pub const BAND_PIXELS: usize = 1 << 22;

/// Builds a [`ShadingField`] a band of rows at a time, so a large image is never held in memory
/// as floats: [`add_band`](Self::add_band) takes the sample's rows in bands of
/// [`band_rows`](Self::band_rows) (the last may be shorter), then [`finish`](Self::finish).
#[derive(Debug)]
pub struct ShadingBuilder {
    ctx: Ctx,
    area: Rect,
    sigma: f32,
    k: usize,
    gw: usize,
    gh: usize,
    /// Luma sum and weight per grid cell, interleaved.
    packed: Vec<f32>,
    /// Luma code per pixel of the area.
    luma: Vec<u16>,
    band_rows: usize,
}

fn reserve<T: Clone>(n: usize, value: T) -> Result<Vec<T>, String> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| "out of memory building the shading field".to_string())?;
    v.resize(n, value);
    Ok(v)
}

impl ShadingBuilder {
    /// A builder for `area` (colour model `mode`, last channel alpha when `alpha`), blurring luma
    /// with Gaussian `sigma` pixels. Errors on a non-positive or non-finite `sigma`, or an area
    /// over [`MAX_FIELD_PIXELS`].
    pub fn new(mode: ColorMode, alpha: bool, area: Rect, sigma: f32) -> Result<Self, String> {
        if !sigma.is_finite() || sigma <= 0.0 {
            return Err(format!("shading detail must be a positive number, got {sigma}"));
        }
        let sigma = sigma.min(MAX_SIGMA);
        let (w, h) = (area.width() as usize, area.height() as usize);
        if u64::from(area.width()) * u64::from(area.height()) > MAX_FIELD_PIXELS {
            return Err(format!("the shading area is too large (over {MAX_FIELD_PIXELS} pixels)"));
        }
        // Block size: about σ/4 pixels, coarser when the grid would be too big.
        let by_sigma = (sigma / 4.0).round() as usize;
        let by_cells = ((w as f64 * h as f64 / MAX_GRID_CELLS as f64).sqrt().ceil()) as usize;
        let k = by_sigma.max(by_cells).max(1);
        let (gw, gh) = (w.div_ceil(k), h.div_ceil(k));
        let mut b = Self {
            ctx: Ctx { bounds: area, mode, alpha },
            area,
            sigma,
            k,
            gw,
            gh,
            packed: reserve(gw * gh * 2, 0.0f32)?,
            luma: reserve(w * h, NO_LUMA)?,
            band_rows: k,
        };
        b.band_rows = (BAND_PIXELS / w.max(1)).max(1);
        b.set_band_rows(b.band_rows);
        Ok(b)
    }

    /// The area being built.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Rows per band: a multiple of the grid's block size (any band height gives the same field).
    pub fn band_rows(&self) -> usize {
        self.band_rows
    }

    /// Use bands of about `rows` rows (rounded up to a multiple of the block size). For tests.
    #[doc(hidden)]
    pub fn set_band_rows(&mut self, rows: usize) {
        self.band_rows = rows.max(1).div_ceil(self.k) * self.k;
    }

    /// Add the rows `band` of the area: full width, anywhere in the area (bands come in order, so
    /// each grid cell sums its pixels in the same order whatever the band heights). `img` must
    /// cover `band` and hold the colour model's channels.
    pub fn add_band(&mut self, img: &Image, band: Rect) -> Result<(), String> {
        let need = crate::fxutil::fmt(&self.ctx).channels();
        let (iw, ih) = (u64::from(img.rect.width()), u64::from(img.rect.height()));
        if !img.rect.contains_rect(&band) || img.ch < need || (iw * ih).checked_mul(img.ch as u64) != Some(img.data.len() as u64) {
            return Err("the shading sample does not match its colour model".to_string());
        }
        self.accumulate(band, &img.data, img.ch, img.rect)
    }

    /// [`add_band`](Self::add_band) for straight RGBA pixels covering exactly `band` (a composite
    /// band), without copying them into an [`Image`]. The builder's colour model must be RGB with
    /// alpha.
    pub fn add_band_rgba(&mut self, band: Rect, px: &[[f32; 4]]) -> Result<(), String> {
        if self.ctx.mode != ColorMode::Rgb || !self.ctx.alpha || (u64::from(band.width()) * u64::from(band.height())) != px.len() as u64 {
            return Err("the shading sample does not match its colour model".to_string());
        }
        self.accumulate(band, px.as_flattened(), 4, band)
    }

    /// Accumulates `band` from `data` (interleaved, `ch` channels, covering `data_rect`), one grid
    /// row of blocks per task: tasks own disjoint rows of the luma codes and of the grid.
    fn accumulate(&mut self, band: Rect, data: &[f32], ch: usize, data_rect: Rect) -> Result<(), String> {
        let a = self.area;
        if band.x0 != a.x0 || band.x1 != a.x1 || band.y0 < a.y0 || band.y1 > a.y1 {
            return Err("a shading band must span the area's width and lie inside it".to_string());
        }
        if band.is_empty() {
            return Ok(());
        }
        let (w, k, gw) = (a.width() as usize, self.k, self.gw);
        let first_gy = (band.y0 - a.y0) as usize / k;
        let last_gy = (band.y1 - 1 - a.y0) as usize / k;
        let too_short = || "the shading area does not match its buffers".to_string();
        let luma_band = self.luma.get_mut((band.y0 - a.y0) as usize * w..(band.y1 - a.y0) as usize * w).ok_or_else(too_short)?;
        let packed_rows = self.packed.get_mut(first_gy * gw * 2..(last_gy + 1) * gw * 2).ok_or_else(too_short)?;
        // One task per grid row: its pixel rows (a block row, or part of one at a band's edge).
        let mut tasks: Vec<(i32, i32, &mut [u16], &mut [f32])> = Vec::with_capacity(last_gy - first_gy + 1);
        let (mut rest, mut y) = (luma_band, band.y0);
        for (gy, grid_row) in (first_gy..=last_gy).zip(packed_rows.chunks_mut(gw * 2)) {
            let y_end = band.y1.min(a.y0.saturating_add(((gy + 1) * k) as i32));
            let (head, tail) = rest.split_at_mut(((y_end - y) as usize * w).min(rest.len()));
            rest = tail;
            tasks.push((y, y_end, head, grid_row));
            y = y_end;
        }
        let dw = data_rect.width() as usize;
        let ctx = self.ctx;
        let work = |(y0, y1, luma, grid): &mut (i32, i32, &mut [u16], &mut [f32])| {
            for y in *y0..*y1 {
                let luma_row = luma.get_mut((y - *y0) as usize * w..(y - *y0 + 1) as usize * w);
                let src_row = (y - data_rect.y0) as usize * dw;
                let Some(luma_row) = luma_row else { continue };
                // The grid column advances every `k` pixels: counted, not divided.
                let (mut gx, mut in_block) = (0usize, 0usize);
                for (xi, slot) in luma_row.iter_mut().enumerate() {
                    let x = a.x0 + xi as i32;
                    let o = (src_row + (x - data_rect.x0) as usize) * ch;
                    if let Some((yl, wt)) = data.get(o..o + ch).and_then(|px| luma_weight(&ctx, px)) {
                        let code = encode_luma(yl);
                        *slot = code;
                        if let Some(cell) = grid.get_mut(gx * 2..gx * 2 + 2) {
                            cell[0] += decode_luma(code) * wt;
                            cell[1] += wt;
                        }
                    }
                    in_block += 1;
                    if in_block == k {
                        in_block = 0;
                        gx += 1;
                    }
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            tasks.par_iter_mut().for_each(work);
        }
        #[cfg(target_arch = "wasm32")]
        tasks.iter_mut().for_each(work);
        Ok(())
    }

    /// Blur the luma grid and finish the field.
    pub fn finish(mut self) -> ShadingField {
        // Luma sums and weights blur alike, so their ratio is the weighted local mean.
        if self.gw > 0 && self.gh > 0 {
            gauss_blur_n(&mut self.packed, self.gw, self.gh, 2, self.sigma / self.k as f32);
        }
        let y_ref = self.packed.as_chunks::<2>().0.iter().map(|c| if c[1] > 1e-6 && c[0].is_finite() { c[0] / c[1] } else { f32::NAN }).collect();
        ShadingField { area: self.area, k: self.k, inv: 1.0 / self.k as f32, gw: self.gw, gh: self.gh, y_ref, luma: self.luma }
    }
}

/// The shading of a sample: its luma at full resolution (16-bit) and its blurred luma at reduced
/// resolution.
#[derive(Debug)]
pub struct ShadingField {
    /// The pixels the field covers (the sample's rect within the requested bounds).
    area: Rect,
    /// Block size of the reduced grid, and its reciprocal.
    k: usize,
    inv: f32,
    gw: usize,
    gh: usize,
    /// Blurred luma per grid cell, NaN where no pixel contributed.
    y_ref: Vec<f32>,
    /// Luma code per pixel of the area ([`NO_LUMA`] where there is none).
    luma: Vec<u16>,
}

impl ShadingField {
    /// Build the field of `sample` (colour model `mode`, last channel alpha when `alpha`) over
    /// `bounds` (clipped to the sample's rectangle), blurring luma with Gaussian `sigma` pixels.
    /// For a sample too big to hold as floats, use [`ShadingBuilder`] band by band.
    ///
    /// Errors on a malformed sample, a non-positive or non-finite `sigma`, or an area over
    /// [`MAX_FIELD_PIXELS`]. Pixels outside the area, and transparent or non-finite ones, are
    /// not shaded (S = 1) and do not enter the blur.
    pub fn build(sample: Image, mode: ColorMode, alpha: bool, bounds: Rect, sigma: f32) -> Result<Self, String> {
        let area = sample.rect.intersect(&bounds);
        let mut b = ShadingBuilder::new(mode, alpha, area, sigma)?;
        let rows = b.band_rows() as i32;
        let mut y = area.y0;
        while y < area.y1 {
            let end = y.saturating_add(rows).min(area.y1);
            b.add_band(&sample, Rect::new(area.x0, y, area.x1, end))?;
            y = end;
        }
        // An empty area still checks the sample against the colour model.
        if area.is_empty() {
            b.add_band(&sample, Rect::new(area.x0, area.y0, area.x1, area.y0))?;
        }
        Ok(b.finish())
    }

    /// The pixels the field covers; elsewhere S is 1.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The reduced grid's block size (pixels).
    pub fn block(&self) -> usize {
        self.k
    }

    /// The two grid rows of blurred luma around pixel row `y`, and the weight between them.
    fn ref_row(&self, y: i32) -> Option<RefRow<'_>> {
        if self.gw == 0 || self.gh == 0 {
            return None;
        }
        let fy = (((y - self.area.y0) as f32 + 0.5) * self.inv - 0.5).clamp(0.0, (self.gh - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(self.gh - 1);
        let (r0, r1) = (self.y_ref.get(y0 * self.gw..(y0 + 1) * self.gw)?, self.y_ref.get(y1 * self.gw..(y1 + 1) * self.gw)?);
        Some(RefRow { r0, r1, ty: fy - y0 as f32 })
    }

    /// The blurred luma at pixel column `x` of a row from [`ref_row`](Self::ref_row) (bilinear
    /// between block centres), `None` where no pixel contributed.
    #[inline]
    fn ref_at(&self, row: &RefRow<'_>, x: i32) -> Option<f32> {
        let fx = (((x - self.area.x0) as f32 + 0.5) * self.inv - 0.5).clamp(0.0, (self.gw - 1) as f32);
        let x0 = fx as usize;
        let x1 = (x0 + 1).min(self.gw - 1);
        let tx = fx - x0 as f32;
        let (a, b, c, d) = (*row.r0.get(x0)?, *row.r0.get(x1)?, *row.r1.get(x0)?, *row.r1.get(x1)?);
        let v = (a + (b - a) * tx) * (1.0 - row.ty) + (c + (d - c) * tx) * row.ty;
        v.is_finite().then_some(v)
    }

    #[inline]
    fn shade_code(&self, code: u16, reference: Option<f32>) -> f32 {
        if code == NO_LUMA {
            return 1.0;
        }
        let Some(r) = reference else { return 1.0 };
        let s = decode_luma(code) / r.max(MIN_REF);
        if s.is_finite() { s.clamp(0.0, MAX_SHADE) } else { 1.0 }
    }

    /// The shading factor at one pixel, `0..=4`; 1 outside the area or where nothing is known.
    pub fn shade_at(&self, x: i32, y: i32) -> f32 {
        if !self.area.contains(x, y) {
            return 1.0;
        }
        let idx = (y - self.area.y0) as usize * self.area.width() as usize + (x - self.area.x0) as usize;
        let Some(&code) = self.luma.get(idx) else { return 1.0 };
        let reference = self.ref_row(y).and_then(|row| self.ref_at(&row, x));
        self.shade_code(code, reference)
    }

    /// Fill `out` (row-major over `rect`) with the shading factor of each pixel. A request is
    /// independent of how it is split: tiles give the same values as one call (and the same values
    /// as [`shade_at`](Self::shade_at)). `out` shorter than the rect is filled as far as it goes.
    /// Row by row: the row's grid lines are found once, and the pixel loop has no `%` or `/`.
    pub fn shade_into(&self, rect: Rect, out: &mut [f32]) {
        let w = rect.width() as usize;
        if w == 0 {
            return;
        }
        let aw = self.area.width() as usize;
        for (r, out_row) in out.chunks_mut(w).enumerate() {
            let y = rect.y0.saturating_add(r as i32);
            let inside_rows = y >= self.area.y0 && y < self.area.y1;
            let ref_row = if inside_rows { self.ref_row(y) } else { None };
            let luma_row = if inside_rows { self.luma.get((y - self.area.y0) as usize * aw..((y - self.area.y0) as usize + 1) * aw) } else { None };
            for (c, o) in out_row.iter_mut().enumerate() {
                let x = rect.x0.saturating_add(c as i32);
                *o = match luma_row {
                    Some(luma) if x >= self.area.x0 && x < self.area.x1 => {
                        let code = luma.get((x - self.area.x0) as usize).copied().unwrap_or(NO_LUMA);
                        let reference = ref_row.as_ref().and_then(|row| self.ref_at(row, x));
                        self.shade_code(code, reference)
                    }
                    _ => 1.0,
                };
            }
        }
    }
}

/// Two grid rows of blurred luma and the weight between them (see `ShadingField::ref_row`).
struct RefRow<'a> {
    r0: &'a [f32],
    r1: &'a [f32],
    ty: f32,
}

/// The perspective planes of a Vanishing Point scene as a brush surface.
#[derive(Clone, Debug)]
pub struct PlaneSurface {
    pub scene: Scene,
}

/// Closed-form SVD of the 2x2 matrix `[[a, b], [c, d]]`: `(σ1, σ2, φ)` with σ1 ≥ σ2 ≥ 0 and φ the
/// direction (radians, in the matrix's own axes) of the image of the unit circle's major axis,
/// i.e. `U`'s first column. `None` for non-finite input or σ1 ≈ 0.
pub fn svd2(m: [[f64; 2]; 2]) -> Option<(f64, f64, f64)> {
    let [[a, b], [c, d]] = m;
    let q = ((a + d).powi(2) + (c - b).powi(2)).sqrt();
    let r = ((a - d).powi(2) + (c + b).powi(2)).sqrt();
    let (s1, s2) = ((q + r) / 2.0, (q - r).abs() / 2.0);
    if !(s1.is_finite() && s1 > 1e-9 && s2.is_finite()) {
        return None;
    }
    let phi = 0.5 * (2.0 * (a * c + b * d)).atan2((a * a + b * b) - (c * c + d * d));
    phi.is_finite().then_some((s1, s2, phi))
}

/// Squashes below this are an edge-on plane: no dab is drawn on it.
const MIN_SQUASH: f64 = 1e-4;
/// Finite-difference step in plane-unit coordinates.
const UNIT_STEP: f64 = 1e-3;

impl PlaneSurface {
    pub fn new(scene: Scene) -> Self {
        Self { scene }
    }

    /// The projection of the plane under image point `p`: `(scale, squash_angle, squash)`.
    ///
    /// The metric-to-image map of the plane (the unit-square homography composed with the plane's
    /// metric side lengths) is differentiated at `p`; its Jacobian `J = U Σ Vᵀ` maps a metric
    /// circle to the ellipse with semi-axes σ1 along `U`'s first column and σ2 along its second.
    /// `scale` is σ1, `squash` is σ2/σ1, and `squash_angle` the image direction of σ2's axis,
    /// negated into the y-up, counter-clockwise convention of the brush's dab projection angle
    /// (the image's y axis points down).
    ///
    /// `None` off every plane, for a degenerate or non-finite plane, and for an edge-on one.
    pub fn local(&self, p: [f64; 2]) -> Option<(f32, f32, f32)> {
        if !p[0].is_finite() || !p[1].is_finite() {
            return None;
        }
        let i = self.scene.plane_at(p)?;
        self.project(i, p)
    }

    /// The scale ([`local`](Self::local)'s first value) at the centre (corner mean) of the first
    /// plane: the size the brush means on a Vanishing Point scene, the same for every stroke.
    /// `None` without planes or for a degenerate first plane.
    pub fn reference_scale(&self) -> Option<f32> {
        let plane = self.scene.planes.first()?;
        let n = plane.corners.len() as f64;
        let c = plane.corners.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
        self.project(0, c).map(|l| l.0)
    }

    /// [`local`](Self::local) on plane `i` (its quad is not tested against `p`).
    fn project(&self, i: usize, p: [f64; 2]) -> Option<(f32, f32, f32)> {
        if !p[0].is_finite() || !p[1].is_finite() {
            return None;
        }
        let (h, lu, lv) = self.scene.frame(i)?;
        if !(lu.is_finite() && lv.is_finite() && lu > 0.0 && lv > 0.0) {
            return None;
        }
        let inv = h.inverse()?;
        let (u, v) = inv.apply(p[0], p[1]);
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        // Central differences of the unit-square map, then per metric unit.
        let e = UNIT_STEP;
        let (xa, xb) = (h.apply(u - e, v), h.apply(u + e, v));
        let (ya, yb) = (h.apply(u, v - e), h.apply(u, v + e));
        let (a, c) = ((xb.0 - xa.0) / (2.0 * e * lu), (xb.1 - xa.1) / (2.0 * e * lu));
        let (b, d) = ((yb.0 - ya.0) / (2.0 * e * lv), (yb.1 - ya.1) / (2.0 * e * lv));
        if ![a, b, c, d].iter().all(|x| x.is_finite()) {
            return None;
        }
        let (s1, s2, phi) = svd2([[a, b], [c, d]])?;
        let squash = s2 / s1;
        if squash.is_nan() || squash < MIN_SQUASH {
            return None;
        }
        // The minor axis is perpendicular; flip y for the y-up convention.
        let mut angle = -(phi + std::f64::consts::FRAC_PI_2);
        while angle > std::f64::consts::PI {
            angle -= 2.0 * std::f64::consts::PI;
        }
        while angle <= -std::f64::consts::PI {
            angle += 2.0 * std::f64::consts::PI;
        }
        let out = (s1 as f32, angle as f32, squash.min(1.0) as f32);
        (out.0.is_finite() && out.0 > 0.0 && out.1.is_finite() && out.2.is_finite()).then_some(out)
    }
}

/// Target block (pixels per grid cell side) of a [`DepthGeometry`] grid.
const DEPTH_BLOCK: usize = 4;
/// Most cells a [`DepthGeometry`] grid may have; larger areas get a coarser block.
const DEPTH_MAX_CELLS: usize = 1 << 22;
/// Tilt gain on the normalised slope `s = |∇D| / p90`: `g = relief * TILT_GAIN * s` and
/// `squash = 1 / sqrt(1 + g²)`. At relief 1 a p90 slope gives `g = 3.18` and a squash of 0.3; at
/// the default relief 0.5, 0.53.
const TILT_GAIN: f64 = 3.18;
/// Cells whose gradient magnitude (depth per pixel) is below this do not count towards the
/// slope scale (flat areas would otherwise drag the percentile to 0).
const SLOPE_EPS: f32 = 1e-6;
/// The gradient is measured over at least this fraction of the brush radius (and a grid cell), so
/// a narrow fold is not averaged away.
const STEP_RADIUS: f64 = 0.35;
/// Directions of a [`DepthGeometry::footprint`] (the brush crate's `FOOTPRINT_RAYS`).
pub const FOOTPRINT_RAYS: usize = 32;
/// Most march steps of one footprint ray.
const FOOTPRINT_STEPS: usize = 64;
/// Squashes below this are clamped (a dab on a steep slope stays visible).
const DEPTH_MIN_SQUASH: f64 = 0.15;
/// Smallest dab scale from `size_by_depth`, so the far plane does not shrink dabs to nothing.
const DEPTH_MIN_SCALE: f64 = 0.05;

/// A depth plane (0..1 per pixel, larger = nearer) as a brush surface: the local slope of depth
/// under a point tilts and shrinks the dab.
///
/// The plane is reduced to a grid of area means (block about 4 px, coarser if the grid would pass
/// 2^22 cells), lightly smoothed (a 3-tap box per axis, edge-clamped, which removes the stair
/// steps of an 8-bit channel), so a lookup costs a few bilinear samples at any image size.
///
/// Depth estimators normalise D over the whole photo, so how steep a fold looks in D varies per
/// photo. The slope is therefore judged against the photo's own 90th percentile of `|∇D|`
/// (over cells that are not flat): a p90 slope is a steep face, whatever the depth's contrast.
#[derive(Clone, Debug)]
pub struct DepthGeometry {
    area: Rect,
    block: usize,
    gw: usize,
    gh: usize,
    grid: Vec<f32>,
    /// 90th percentile of the grid's `|∇D|` (depth per pixel); `None` when the depth is flat.
    slope_scale: Option<f32>,
    /// Median of the grid: the photo's middle distance.
    median: f32,
}

/// 3-tap box blur of a `gw` x `gh` grid along x, then y, clamping at the edges.
fn smooth_grid(grid: &[f32], gw: usize, gh: usize) -> Vec<f32> {
    let at = |g: &[f32], x: usize, y: usize| g.get(y * gw + x).copied().unwrap_or(0.0);
    let tmp: Vec<f32> = (0..gh)
        .flat_map(|y| (0..gw).map(move |x| (x, y)))
        .map(|(x, y)| (at(grid, x.saturating_sub(1), y) + at(grid, x, y) + at(grid, (x + 1).min(gw - 1), y)) / 3.0)
        .collect();
    (0..gh)
        .flat_map(|y| (0..gw).map(move |x| (x, y)))
        .map(|(x, y)| (at(&tmp, x, y.saturating_sub(1)) + at(&tmp, x, y) + at(&tmp, x, (y + 1).min(gh - 1))) / 3.0)
        .collect()
}

/// The `q`-quantile (0..1, lower index) of `v`, reordering it; `None` when empty.
fn quantile(v: &mut [f32], q: f64) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let k = (((v.len() - 1) as f64) * q).floor() as usize;
    let (_, m, _) = v.select_nth_unstable_by(k.min(v.len() - 1), |a, b| a.total_cmp(b));
    Some(*m)
}

impl DepthGeometry {
    /// Build from `depth`, row-major over `area` (`width * height` values, clamped to 0..1).
    /// Errors for an empty area, a length mismatch or a non-finite value.
    pub fn new(area: Rect, depth: &[f32]) -> Result<Self, String> {
        let (w, h) = (i64::from(area.x1) - i64::from(area.x0), i64::from(area.y1) - i64::from(area.y0));
        if w <= 0 || h <= 0 {
            return Err("empty depth area".into());
        }
        let (w, h) = (w as usize, h as usize);
        if w.checked_mul(h) != Some(depth.len()) {
            return Err("depth plane does not match its area".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let finite = {
            use rayon::prelude::*;
            depth.par_iter().all(|v| v.is_finite())
        };
        #[cfg(target_arch = "wasm32")]
        let finite = depth.iter().all(|v| v.is_finite());
        if !finite {
            return Err("depth plane has non-finite values".into());
        }
        let mut block = DEPTH_BLOCK;
        let cells = |b: usize| w.div_ceil(b).saturating_mul(h.div_ceil(b));
        while cells(block) > DEPTH_MAX_CELLS {
            block += 1;
        }
        let (gw, gh) = (w.div_ceil(block), h.div_ceil(block));
        // Area means, one band of `block` rows per grid row (bands are independent).
        let mut mean = vec![0.0f32; gw * gh];
        let band = |(gy, out): (usize, &mut [f32])| {
            let mut sum = vec![0.0f32; gw];
            let mut count = vec![0u32; gw];
            let rows = depth.chunks_exact(w).skip(gy * block).take(block);
            for row in rows {
                for (x, v) in row.iter().enumerate() {
                    if let (Some(s), Some(c)) = (sum.get_mut(x / block), count.get_mut(x / block)) {
                        *s += v.clamp(0.0, 1.0);
                        *c += 1;
                    }
                }
            }
            for ((o, s), c) in out.iter_mut().zip(&sum).zip(&count) {
                *o = if *c > 0 { s / *c as f32 } else { 0.0 };
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            mean.par_chunks_mut(gw).enumerate().for_each(band);
        }
        #[cfg(target_arch = "wasm32")]
        mean.chunks_mut(gw).enumerate().for_each(band);
        let grid = smooth_grid(&mean, gw, gh);
        let median = quantile(&mut grid.clone(), 0.5).unwrap_or(0.5);
        // Central differences over one cell (clamped at the edges), in depth per pixel.
        let at = |x: usize, y: usize| grid.get(y * gw + x).copied().unwrap_or(0.0);
        let mut mags = Vec::new();
        for y in 0..gh {
            let (ya, yb) = (y.saturating_sub(1), (y + 1).min(gh - 1));
            for x in 0..gw {
                let (xa, xb) = (x.saturating_sub(1), (x + 1).min(gw - 1));
                let gx = if xb > xa { (at(xb, y) - at(xa, y)) / ((xb - xa) * block) as f32 } else { 0.0 };
                let gy = if yb > ya { (at(x, yb) - at(x, ya)) / ((yb - ya) * block) as f32 } else { 0.0 };
                let m = gx.hypot(gy);
                if m > SLOPE_EPS {
                    mags.push(m);
                }
            }
        }
        let slope_scale = quantile(&mut mags, 0.9).filter(|s| s.is_finite() && *s > 0.0);
        Ok(Self { area, block, gw, gh, grid, slope_scale, median })
    }

    /// The median depth of the grid: the photo's middle distance, where the brush size is meant
    /// when sizing by depth.
    pub fn median(&self) -> f32 {
        self.median
    }

    /// The scale a dab has at the median depth (`max(median, 0.05)`), the size the brush means
    /// with `size_by_depth`.
    pub fn reference_scale(&self) -> f32 {
        f64::from(self.median).max(DEPTH_MIN_SCALE) as f32
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Bilinear sample at pixel position `(x, y)` (document coordinates); cell centres sit at
    /// half a block, and positions past the outermost centres clamp to the edge.
    fn sample(&self, x: f64, y: f64) -> f64 {
        let b = self.block as f64;
        let fx = ((x - f64::from(self.area.x0)) / b - 0.5).clamp(0.0, (self.gw - 1) as f64);
        let fy = ((y - f64::from(self.area.y0)) / b - 0.5).clamp(0.0, (self.gh - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.gw - 1), (y0 + 1).min(self.gh - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |gx: usize, gy: usize| f64::from(self.grid.get(gy * self.gw + gx).copied().unwrap_or(0.0));
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bot = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        top * (1.0 - ty) + bot * ty
    }

    /// The dab projection at `(x, y)` for a brush of `radius` pixels: `(scale, squash_angle,
    /// squash)`, in the same convention as [`PlaneSurface::local`] (`squash_angle` is the y-up,
    /// counter-clockwise direction of the compressed axis).
    ///
    /// The depth gradient comes from central differences of the smoothed grid at a step of
    /// `max(0.35 * radius, one cell)`: narrow, so a thin fold is not averaged flat. With the
    /// normalised slope `s = |∇D| / p90` (see [`DepthGeometry`]), `g = relief * 3.18 * s` and
    /// `squash = 1 / sqrt(1 + g²)` (not below 0.15), along the gradient. The tilt of a surface
    /// does not depend on the brush size, so `g` has no radius in it; a flat depth never squashes.
    /// `scale` is `max(D, 0.05)` when `size_by_depth` is set, else 1.
    ///
    /// `None` for non-finite arguments, a non-positive radius and points outside the area.
    pub fn local(&self, x: f64, y: f64, radius: f32, relief: f32, size_by_depth: bool) -> Option<(f32, f32, f32)> {
        if !(x.is_finite() && y.is_finite() && radius.is_finite() && radius > 0.0 && relief.is_finite()) {
            return None;
        }
        let a = self.area;
        if x < f64::from(a.x0) || y < f64::from(a.y0) || x >= f64::from(a.x1) || y >= f64::from(a.y1) {
            return None;
        }
        let relief = f64::from(relief.clamp(0.0, 1.0));
        let step = (f64::from(radius) * STEP_RADIUS).max(self.block as f64);
        let gx = (self.sample(x + step, y) - self.sample(x - step, y)) / (2.0 * step);
        let gy = (self.sample(x, y + step) - self.sample(x, y - step)) / (2.0 * step);
        let mag = gx.hypot(gy);
        let g = self.slope_scale.map_or(0.0, |p90| relief * TILT_GAIN * mag / f64::from(p90));
        let squash = (1.0 / (1.0 + g * g).sqrt()).max(DEPTH_MIN_SQUASH);
        // Image y points down; the dab convention is y-up.
        let angle = if mag > 1e-9 { (-gy).atan2(gx) } else { 0.0 };
        let scale = if size_by_depth { self.sample(x, y).max(DEPTH_MIN_SCALE) } else { 1.0 };
        let out = (scale as f32, angle as f32, squash as f32);
        (out.0.is_finite() && out.1.is_finite() && out.2.is_finite()).then_some(out)
    }
}

impl DepthGeometry {
    /// The outline of a dab of `radius_px` pixels at `(x, y)` bent over the surface: the distance
    /// to its edge along [`FOOTPRINT_RAYS`] evenly spaced directions (ray `k` at angle
    /// `k · 2π / 32`, counter-clockwise, y up).
    ///
    /// Each ray is marched over the smoothed grid in image pixels with a step of
    /// `max(radius / 16, 0.5)` (at most 64 steps). A step `ds` along a slope `t` of the surface
    /// covers `sqrt(1 + t²) · ds` of surface, with `t = relief * 3.18 * (dD/ds) / p90`: the
    /// normalised tilt of [`local`](Self::local), so a ray on a uniform slope ends at
    /// `radius / sqrt(1 + g²)`, the squash. The ray stops when the surface length reaches the
    /// radius, and is clamped to `[0.15 * radius, radius]`. A flat depth gives a circle.
    ///
    /// `None` for non-finite arguments, a non-positive radius and points outside the area.
    pub fn footprint(&self, x: f64, y: f64, radius_px: f32, relief: f32) -> Option<[f32; FOOTPRINT_RAYS]> {
        if !(x.is_finite() && y.is_finite() && radius_px.is_finite() && radius_px > 0.0 && relief.is_finite()) {
            return None;
        }
        let a = self.area;
        if x < f64::from(a.x0) || y < f64::from(a.y0) || x >= f64::from(a.x1) || y >= f64::from(a.y1) {
            return None;
        }
        let r = f64::from(radius_px);
        let relief = f64::from(relief.clamp(0.0, 1.0));
        let mut rays = [radius_px; FOOTPRINT_RAYS];
        let Some(p90) = self.slope_scale.map(f64::from).filter(|_| relief > 0.0) else { return Some(rays) };
        let ds = (r / 16.0).max(0.5);
        let gain = relief * TILT_GAIN / p90;
        for (k, ray) in rays.iter_mut().enumerate() {
            let (sn, cs) = (k as f64 * std::f64::consts::TAU / FOOTPRINT_RAYS as f64).sin_cos();
            // Image y points down.
            let (dx, dy) = (cs, -sn);
            let (mut t, mut surface) = (0.0f64, 0.0f64);
            let mut here = self.sample(x, y);
            for _ in 0..FOOTPRINT_STEPS {
                let next = self.sample(x + dx * (t + ds), y + dy * (t + ds));
                let tilt = gain * (next - here) / ds;
                let seg = (1.0 + tilt * tilt).sqrt() * ds;
                if surface + seg >= r {
                    t += ds * (r - surface) / seg;
                    break;
                }
                surface += seg;
                t += ds;
                here = next;
            }
            *ray = t.clamp(0.15 * r, r) as f32;
        }
        rays.iter().all(|v| v.is_finite()).then_some(rays)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::Homography;
    use crate::vanishing::VpPlane;
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;

    fn sample(w: i32, h: i32, f: impl Fn(i32, i32) -> [f32; 4]) -> Image {
        let rect = Rect::new(0, 0, w, h);
        let mut s = Surface::new(PixelFormat::RGBA32F);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        s.write_region(rect, &data);
        Image::read(&s, rect)
    }

    fn field(img: Image, sigma: f32) -> ShadingField {
        let b = img.rect;
        ShadingField::build(img, ColorMode::Rgb, true, b, sigma).unwrap()
    }

    #[test]
    fn a_flat_image_has_no_shading() {
        let f = field(sample(70, 50, |_, _| [0.4, 0.5, 0.6, 1.0]), 12.0);
        let mut out = vec![0.0; 70 * 50];
        f.shade_into(Rect::new(0, 0, 70, 50), &mut out);
        assert!(out.iter().all(|s| (s - 1.0).abs() < 1e-3), "{:?}", out.iter().cloned().fold(0.0f32, |m, s| m.max((s - 1.0).abs())));
    }

    #[test]
    fn stripes_finer_than_the_detail_show_as_shading() {
        // Alternating dark and light columns on a mid-grey: S follows the stripes.
        let f = field(sample(96, 32, |x, _| if x % 2 == 0 { [0.3, 0.3, 0.3, 1.0] } else { [0.7, 0.7, 0.7, 1.0] }), 16.0);
        let (dark, light) = (f.shade_at(48, 16), f.shade_at(49, 16));
        assert!(dark < 0.8 && light > 1.2, "{dark} {light}");
    }

    #[test]
    fn tiled_requests_equal_one_full_request() {
        let img = sample(150, 90, |x, y| {
            let v = 0.2 + 0.6 * (((x * 7 + y * 13) % 17) as f32 / 16.0);
            [v, v * 0.8, v * 0.5, if x < 10 { 0.0 } else { 1.0 }]
        });
        let f = field(img, 20.0);
        let full = Rect::new(-5, -5, 155, 95);
        let mut whole = vec![0.0; 160 * 100];
        f.shade_into(full, &mut whole);
        let mut tiled = vec![0.0; 160 * 100];
        for ty in 0..2 {
            for tx in 0..3 {
                let r = Rect::new(-5 + tx * 54, -5 + ty * 50, (-5 + tx * 54 + 54).min(155), (-5 + ty * 50 + 50).min(95));
                let mut part = vec![0.0; (r.width() * r.height()) as usize];
                f.shade_into(r, &mut part);
                for (i, s) in part.iter().enumerate() {
                    let (x, y) = (r.x0 + (i as i32 % r.width() as i32), r.y0 + (i as i32 / r.width() as i32));
                    tiled[((y - full.y0) * 160 + (x - full.x0)) as usize] = *s;
                }
            }
        }
        assert!(whole.iter().zip(&tiled).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert!(whole.iter().all(|s| (0.0..=4.0).contains(s)));
        // Transparent pixels and pixels outside the area are 1.
        assert_eq!(f.shade_at(3, 40), 1.0);
        assert_eq!(f.shade_at(-5, 0), 1.0);
    }

    #[test]
    fn band_built_equals_single_pass_built() {
        let rect = Rect::new(0, 0, 50, 41);
        let mut surf = Surface::new(PixelFormat::RGBA32F);
        let mut data = Vec::new();
        for y in 0..41 {
            for x in 0..50 {
                let v = 0.1 + 0.8 * (((x * 5 + y * 11) % 13) as f32 / 12.0);
                data.extend_from_slice(&[v, v * 0.7, v * 0.4, if (x + y) % 17 == 0 { 0.0 } else { 1.0 }]);
            }
        }
        surf.write_region(rect, &data);
        for sigma in [4.0f32, 20.0] {
            let whole = ShadingField::build(Image::read(&surf, rect), ColorMode::Rgb, true, rect, sigma).unwrap();
            let mut b = ShadingBuilder::new(ColorMode::Rgb, true, rect, sigma).unwrap();
            b.set_band_rows(3);
            // Bands of any height (not only whole blocks) give the same field.
            let rows = if sigma > 10.0 { 7 } else { b.band_rows() as i32 };
            assert!(rows >= 3);
            let mut y = 0;
            while y < 41 {
                let band = Rect::new(0, y, 50, (y + rows).min(41));
                b.add_band(&Image::read(&surf, band), band).unwrap();
                y += rows;
            }
            let banded = b.finish();
            let (mut a, mut c) = (vec![0.0; 50 * 41], vec![0.0; 50 * 41]);
            whole.shade_into(rect, &mut a);
            banded.shade_into(rect, &mut c);
            assert!(a.iter().zip(&c).all(|(x, y)| x.to_bits() == y.to_bits()), "sigma {sigma}");
        }
        // A band that misses the width, or lies outside the area, is refused.
        let mut b = ShadingBuilder::new(ColorMode::Rgb, true, rect, 20.0).unwrap();
        let outside = Rect::new(0, 30, 50, 60);
        assert!(b.add_band(&Image::read(&surf, outside), outside).is_err());
        let narrow = Rect::new(0, 0, 10, 5);
        assert!(b.add_band(&Image::read(&surf, narrow), narrow).is_err());
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let img = || sample(8, 8, |_, _| [0.5; 4]);
        for sigma in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            assert!(ShadingField::build(img(), ColorMode::Rgb, true, Rect::new(0, 0, 8, 8), sigma).is_err());
        }
        // A model with more channels than the sample has.
        assert!(ShadingField::build(img(), ColorMode::Cmyk, true, Rect::new(0, 0, 8, 8), 4.0).is_err());
        // Bounds that miss the sample: nothing to shade.
        let f = ShadingField::build(img(), ColorMode::Rgb, true, Rect::new(100, 100, 120, 120), 4.0).unwrap();
        assert_eq!(f.shade_at(2, 2), 1.0);
        let mut out = [0.0f32; 4];
        f.shade_into(Rect::new(0, 0, 0, 0), &mut out);
    }

    fn square_on() -> PlaneSurface {
        let plane = VpPlane { corners: [[100.0, 100.0], [300.0, 100.0], [300.0, 300.0], [100.0, 300.0]] };
        PlaneSurface::new(Scene::new(plane, [200.0, 200.0], None, 1000.0))
    }

    /// A 2 x 1 rectangle tilted 0.6 rad about x and turned 0.4 rad about y, seen by a camera with
    /// f = 500 px centred at (400, 300).
    fn slanted() -> PlaneSurface {
        let (f, c) = (500.0f64, [400.0f64, 300.0f64]);
        let corners = [[-1.0, -0.5], [1.0, -0.5], [1.0, 0.5], [-1.0, 0.5]].map(|[x, y]: [f64; 2]| {
            let (s, co) = 0.6f64.sin_cos();
            let q = [x, co * y, s * y];
            let (s2, c2) = 0.4f64.sin_cos();
            let z = -s2 * q[0] + c2 * q[2] + 4.0;
            [c[0] + f * (c2 * q[0] + s2 * q[2]) / z, c[1] + f * q[1] / z]
        });
        PlaneSurface::new(Scene::new(VpPlane { corners }, c, None, 1000.0))
    }

    /// Image points inside the plane at unit coordinates `(u, v)`.
    fn at(s: &PlaneSurface, u: f64, v: f64) -> [f64; 2] {
        let (h, _, _) = s.scene.frame(0).unwrap();
        let p = h.apply(u, v);
        [p.0, p.1]
    }

    #[test]
    fn a_square_on_plane_has_uniform_scale_and_no_squash() {
        let s = square_on();
        let (_, lu, _) = s.scene.frame(0).unwrap();
        for p in [[120.0, 120.0], [200.0, 200.0], [290.0, 150.0]] {
            let (scale, _, squash) = s.local(p).unwrap();
            assert!((squash - 1.0).abs() < 1e-3, "{squash}");
            // 200 px across the plane's `lu` metric units.
            assert!((f64::from(scale) * lu - 200.0).abs() < 0.5, "{scale} {lu}");
        }
        assert!(s.local([50.0, 50.0]).is_none());
    }

    #[test]
    fn local_area_matches_the_homography_local_scale() {
        let s = slanted();
        let (h, lu, lv) = s.scene.frame(0).unwrap();
        let inv = h.inverse().unwrap();
        for (u, v) in [(0.5, 0.5), (0.2, 0.8), (0.8, 0.2), (0.1, 0.1)] {
            let p = at(&s, u, v);
            let (scale, _, squash) = s.local(p).unwrap();
            // σ1·σ2 is the metric-to-image area scale: 1 / (lu · lv · local_scale² of the image-to-unit map).
            let ls = Homography::local_scale(&inv, p[0], p[1]);
            let expect = 1.0 / (ls * ls * lu * lv);
            let got = f64::from(scale) * f64::from(scale) * f64::from(squash);
            assert!((got / expect - 1.0).abs() < 0.02, "{got} vs {expect} at {p:?}");
        }
    }

    #[test]
    fn a_slanted_plane_squashes_and_its_scale_varies() {
        let s = slanted();
        let (_, _, squash) = s.local(at(&s, 0.5, 0.5)).unwrap();
        assert!(squash < 0.95 && squash > 0.2, "{squash}");
        let scales: Vec<f32> = [(0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9)].iter().map(|&(u, v)| s.local(at(&s, u, v)).unwrap().0).collect();
        let (lo, hi) = scales.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!(hi / lo > 1.05, "{scales:?}");
        let (_, angle, _) = s.local(at(&s, 0.5, 0.5)).unwrap();
        assert!(angle > -std::f32::consts::PI && angle <= std::f32::consts::PI);
    }

    #[test]
    fn degenerate_or_non_finite_planes_have_no_projection() {
        let flat = VpPlane { corners: [[0.0, 0.0], [100.0, 0.0], [200.0, 0.0], [300.0, 0.0]] };
        assert!(PlaneSurface::new(Scene::new(flat, [0.0, 0.0], None, 1000.0)).local([50.0, 0.0]).is_none());
        let nan = VpPlane { corners: [[f64::NAN, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]] };
        assert!(PlaneSurface::new(Scene::new(nan, [0.0, 0.0], None, 1000.0)).local([50.0, 50.0]).is_none());
        assert!(square_on().local([f64::NAN, 150.0]).is_none());
        let none = PlaneSurface::new(Scene { planes: Vec::new(), focal: 1000.0, center: [0.0, 0.0] });
        assert!(none.local([1.0, 1.0]).is_none());
    }

    fn ramp(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> DepthGeometry {
        let mut d = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                d.push(f(x, y));
            }
        }
        DepthGeometry::new(Rect { x0: 0, y0: 0, x1: w as i32, y1: h as i32 }, &d).unwrap()
    }

    #[test]
    fn depth_ramp_squashes_along_the_ramp_whatever_the_brush_size() {
        // D rises 0..1 over 200 px along +x: a p90 slope, so squash 1/sqrt(1 + 3.18^2) = 0.3.
        let g = ramp(200, 100, |x, _| x as f32 / 200.0);
        for radius in [3.0, 10.0, 40.0] {
            let (_, angle, squash) = g.local(100.0, 50.0, radius, 1.0, false).unwrap();
            assert!((squash - 0.3).abs() < 0.03, "radius {radius}: {squash}");
            assert!(angle.abs() < 1e-3, "{angle}");
        }
        // The default relief is gentler.
        assert!((g.local(100.0, 50.0, 10.0, 0.5, false).unwrap().2 - 0.53).abs() < 0.04);
        // Relief 0 never squashes; a ramp along image +y is y-down, so the y-up angle is -90 deg.
        assert!((g.local(100.0, 50.0, 10.0, 0.0, false).unwrap().2 - 1.0).abs() < 1e-6);
        let v = ramp(100, 200, |_, y| y as f32 / 200.0);
        let (_, angle, squash) = v.local(50.0, 100.0, 10.0, 1.0, false).unwrap();
        assert!(squash < 0.4);
        assert!((angle + std::f32::consts::FRAC_PI_2).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn a_narrow_fold_squashes_on_its_flanks_even_with_a_big_brush() {
        // Period 120 px = twice a brush diameter of 60 (radius 30).
        let fold = |amp: f32| ramp(480, 40, move |x, _| 0.5 + amp * (std::f32::consts::TAU * x as f32 / 120.0).sin());
        let g = fold(0.4);
        // Flank (x = 120, the sine's rising zero crossing) and crest (x = 150).
        let flank = g.local(240.0, 20.0, 30.0, 1.0, false).unwrap().2;
        let crest = g.local(270.0, 20.0, 30.0, 1.0, false).unwrap().2;
        assert!(flank < 0.45, "flank {flank}");
        assert!(crest > 0.9, "crest {crest}");
    }

    #[test]
    fn the_squash_does_not_depend_on_the_depths_contrast() {
        let fold = |amp: f32| ramp(480, 40, move |x, _| 0.5 + amp * (std::f32::consts::TAU * x as f32 / 120.0).sin());
        let (strong, weak) = (fold(0.4), fold(0.04));
        for x in [200.0, 240.0, 265.0, 330.0] {
            let (a, b) = (strong.local(x, 20.0, 12.0, 1.0, false).unwrap().2, weak.local(x, 20.0, 12.0, 1.0, false).unwrap().2);
            assert!((a - b).abs() < 2e-3, "x {x}: {a} vs {b}");
        }
    }

    #[test]
    fn a_quantised_ramp_squashes_smoothly_along_x() {
        // An 8-bit channel: D steps by 1/255 about every 4 px.
        let g = ramp(1000, 40, |x, _| (x as f32 / 1000.0 * 255.0).round() / 255.0);
        let sq: Vec<f32> = (100..900).step_by(7).map(|x| g.local(x as f64, 20.0, 10.0, 1.0, false).unwrap().2).collect();
        let (lo, hi) = sq.iter().fold((1.0f32, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi - lo < 0.1 && lo > 0.2 && hi < 0.45, "{lo}..{hi}");
    }

    #[test]
    fn a_flat_depth_has_no_slope_scale_and_never_squashes() {
        let g = ramp(64, 64, |_, _| 0.6);
        assert!(g.slope_scale.is_none());
        assert_eq!(g.local(32.0, 32.0, 8.0, 1.0, false).unwrap().2, 1.0);
        // A slope below the flatness floor is flat too.
        let tiny = ramp(64, 64, |x, _| 0.5 + x as f32 * 1e-9);
        assert_eq!(tiny.local(32.0, 32.0, 8.0, 1.0, false).unwrap().2, 1.0);
    }

    #[test]
    fn the_median_and_the_reference_scale_follow_the_depth() {
        let g = ramp(200, 20, |x, _| x as f32 / 200.0);
        assert!((g.median() - 0.5).abs() < 0.03, "{}", g.median());
        assert!((g.reference_scale() - g.median()).abs() < 1e-6);
        assert!((ramp(16, 16, |_, _| 0.0).reference_scale() - 0.05).abs() < 1e-6);
    }

    #[test]
    fn plane_reference_scale_is_the_scale_at_the_first_planes_centre() {
        let s = square_on();
        let r = s.reference_scale().unwrap();
        let c = s.local([200.0, 200.0]).map(|l| l.0);
        assert!(c.is_some_and(|c| (c - r).abs() < r * 0.05), "{r} {c:?}");
        let none = PlaneSurface::new(Scene { planes: Vec::new(), focal: 1000.0, center: [0.0, 0.0] });
        assert!(none.reference_scale().is_none());
    }

    #[test]
    fn depth_flat_does_not_squash_and_scale_follows_depth() {
        let g = ramp(64, 64, |_, _| 0.6);
        let (scale, _, squash) = g.local(32.0, 32.0, 8.0, 1.0, true).unwrap();
        assert_eq!(squash, 1.0);
        assert!((scale - 0.6).abs() < 1e-5);
        assert_eq!(g.local(32.0, 32.0, 8.0, 1.0, false).unwrap().0, 1.0);
        // Far plane keeps a minimum scale.
        let far = ramp(16, 16, |_, _| 0.0);
        assert!((far.local(8.0, 8.0, 4.0, 1.0, true).unwrap().0 - 0.05).abs() < 1e-6);
        // Scale tracks D across a ramp.
        let r = ramp(200, 20, |x, _| x as f32 / 200.0);
        let (a, b) = (r.local(40.0, 10.0, 2.0, 0.0, true).unwrap().0, r.local(160.0, 10.0, 2.0, 0.0, true).unwrap().0);
        assert!(b > a * 3.0);
    }

    #[test]
    fn depth_squash_is_clamped_and_bounds_checked() {
        let cliff = ramp(100, 20, |x, _| if x < 50 { 0.0 } else { 1.0 });
        let squash = cliff.local(50.0, 10.0, 10.0, 1.0, false).unwrap().2;
        assert!((0.15..0.4).contains(&squash), "{squash}");
        let g = ramp(32, 32, |_, _| 0.5);
        assert!(g.local(-1.0, 5.0, 4.0, 1.0, true).is_none());
        assert!(g.local(5.0, 32.0, 4.0, 1.0, true).is_none());
        assert!(g.local(f64::NAN, 5.0, 4.0, 1.0, true).is_none());
        assert!(g.local(5.0, 5.0, f32::NAN, 1.0, true).is_none());
        assert!(g.local(5.0, 5.0, 0.0, 1.0, true).is_none());
        assert!(g.local(5.0, 5.0, 4.0, f32::INFINITY, true).is_none());
    }

    #[test]
    fn depth_geometry_rejects_bad_input() {
        let r = Rect { x0: 0, y0: 0, x1: 4, y1: 4 };
        assert!(DepthGeometry::new(r, &[0.0; 15]).is_err());
        assert!(DepthGeometry::new(Rect::EMPTY, &[]).is_err());
        let mut d = vec![0.5f32; 16];
        d[3] = f32::NAN;
        assert!(DepthGeometry::new(r, &d).is_err());
        // An offset area is addressed in document coordinates.
        let o = Rect { x0: 100, y0: 50, x1: 104, y1: 54 };
        let g = DepthGeometry::new(o, &[0.5; 16]).unwrap();
        assert!(g.local(101.0, 51.0, 2.0, 1.0, true).is_some());
        assert!(g.local(1.0, 1.0, 2.0, 1.0, true).is_none());
    }

    #[test]
    fn a_flat_depth_footprint_is_a_circle() {
        let g = ramp(64, 64, |_, _| 0.6);
        let rays = g.footprint(32.0, 32.0, 10.0, 1.0).unwrap();
        assert!(rays.iter().all(|r| (r - 10.0).abs() < 1e-5), "{rays:?}");
        assert!(g.footprint(32.0, 32.0, 0.0, 1.0).is_none());
        assert!(g.footprint(-1.0, 32.0, 10.0, 1.0).is_none());
        assert!(g.footprint(32.0, 32.0, f32::NAN, 1.0).is_none());
    }

    #[test]
    fn a_ramp_footprint_is_shorter_along_the_ramp_than_across() {
        let g = ramp(400, 200, |x, _| x as f32 / 400.0);
        let rays = g.footprint(200.0, 100.0, 20.0, 1.0).unwrap();
        // Ray 0 points along +x (the ramp), ray 8 along +y (across it): the squash's direction.
        let (along, across) = (rays[0].min(rays[16]), rays[8].max(rays[24]));
        assert!(along < across * 0.5, "{along} {across}");
        assert!((across - 20.0).abs() < 0.5);
        // About the squash 0.3 of a p90 slope.
        assert!((along / 20.0 - 0.3).abs() < 0.06, "{}", along / 20.0);
        // Relief 0 is a circle.
        assert!(g.footprint(200.0, 100.0, 20.0, 0.0).unwrap().iter().all(|r| (r - 20.0).abs() < 1e-5));
    }

    #[test]
    fn a_ridge_shortens_the_rays_that_cross_it_and_is_asymmetric_off_centre() {
        // A gentle climb to a crest at x = 100, then a steep drop to the floor at x = 120.
        let ridge = |x: usize| if x < 100 { 0.5 * x as f32 / 100.0 } else { (0.5 - 0.5 * (x as f32 - 100.0) / 20.0).max(0.0) };
        let g = ramp(240, 100, |x, _| ridge(x));
        let rays = g.footprint(92.0, 50.0, 24.0, 1.0).unwrap();
        // Rays along +x cross the crest and drop; along y they stay on the contour (a full circle).
        let (east, west, along_y) = (rays[0], rays[16], rays[8].min(rays[24]));
        assert!((along_y - 24.0).abs() < 0.5, "{along_y}");
        assert!(east < along_y * 0.8 && west < along_y, "{east} {west} {along_y}");
        // Off the middle of the ridge the outline is not symmetric.
        assert!(east < west - 0.5, "{east} {west}");
        assert!(rays.iter().all(|r| (3.6 - 1e-4..=24.0 + 1e-4).contains(r)));
    }
}
