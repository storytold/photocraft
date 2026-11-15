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

    #[test]
    fn plane_reference_scale_is_the_scale_at_the_first_planes_centre() {
        let s = square_on();
        let r = s.reference_scale().unwrap();
        let c = s.local([200.0, 200.0]).map(|l| l.0);
        assert!(c.is_some_and(|c| (c - r).abs() < r * 0.05), "{r} {c:?}");
        let none = PlaneSurface::new(Scene { planes: Vec::new(), focal: 1000.0, center: [0.0, 0.0] });
        assert!(none.reference_scale().is_none());
    }
}
