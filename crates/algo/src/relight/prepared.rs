//! Stream the six separable shading passes through row rings. Every source row is converted
//! once per column slab; the three vertical sums survive strip boundaries. No full-image float plane or cache.
use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

use super::{MAX_LUMA_PIXELS, Params, finite_all, read_radius, rec709_y, shade_pixels, sigma, step_for_sigma};
use crate::{Ctx, FilterParams, fxutil, image::Image, par};

const STRIP: usize = 32;
const SLAB: i32 = 1024;
const OUTPUT_TILE: i32 = 256;

fn zeros(len: usize) -> Option<Vec<f32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0.0);
    Some(v)
}

fn geometry(params: &FilterParams, area: Rect, bounds: Rect) -> Option<(Rect, [usize; 3], i32)> {
    let FilterParams::Relight { intensity, softness, .. } = *params else { return None };
    // One small source window is already cheap. Keep unusual parameter/coordinate domains on
    // the reference path, and check the ring capacity before allocating any input-sized data.
    if !intensity.is_finite() || intensity == 0.0 || !softness.is_finite() || area.is_empty() {
        return None;
    }
    let sig = sigma(softness, bounds);
    let reach = read_radius(sig);
    let field = area.inflate(reach);
    if u64::from(field.width()) != u64::from(area.width()) + 2 * reach as u64 || u64::from(field.height()) != u64::from(area.height()) + 2 * reach as u64 {
        return None;
    }
    if field.height() > i32::MAX as u32 {
        return None;
    }
    let radii = fxutil::gauss_box_radii(sig);
    let ring_rows: usize = radii.iter().map(|r| 2 * r + 1).sum();
    if (field.width() as usize).checked_mul(ring_rows.max(STRIP))? > MAX_LUMA_PIXELS {
        return None;
    }
    Some((field, radii, step_for_sigma(sig)))
}

/// Admission includes all row rings, f64 sums, strip buffers, parallel source reads and results.
/// Storage depends on width and sigma, never the document's height.
pub(crate) fn working_bytes(params: &FilterParams, area: Rect, bounds: Rect, channels: u64) -> u64 {
    let Some((field, radii, step)) = geometry(params, area, bounds) else { return 0 };
    if area.size().area() < 512 * 512 {
        return 0;
    }
    let slabs = u64::from(area.width()).div_ceil(SLAB as u64) + 1;
    // Nested Rayon jobs may suspend a stream while starting another one. Admit every
    // slab, even with fewer workers, rather than assuming one live ring per thread.
    let parallel = if cfg!(target_arch = "wasm32") { 1 } else { slabs };
    let halo = (u64::from(field.width()) - u64::from(area.width())) / 2;
    let w = u64::from(field.width()).min(SLAB as u64 + 2 * halo);
    let rings = radii.iter().map(|r| (2 * r + 1) as u64).sum::<u64>() * w * 2 * 4;
    let sums = w * 2 * 8 * 3;
    let rows = w * 2 * 4 * (STRIP as u64 + 4);
    let shading = w * (2 * step as u64 + STRIP as u64) * 4;
    let strips = w.saturating_mul(STRIP as u64).saturating_mul(channels).saturating_mul(4);
    let results = u64::from(area.width().min(SLAB as u32)).saturating_mul(STRIP as u64).saturating_mul(channels).saturating_mul(4);
    let workers = (par::threads() as u64).min(w.div_ceil(OUTPUT_TILE as u64));
    let sources = (OUTPUT_TILE as u64).saturating_mul(STRIP as u64).saturating_mul(channels).saturating_mul(4).saturating_mul(workers);
    rings
        .saturating_add(sums)
        .saturating_add(rows)
        .saturating_add(shading)
        .saturating_add(strips)
        .saturating_add(results)
        .saturating_add(sources)
        .saturating_mul(parallel)
}

/// `None` falls back to the reference implementation; `Some(None)` means cancellation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    extent: Option<Rect>,
    ctl: &Interrupt,
) -> Option<Option<Surface>> {
    if area.size().area() < 512 * 512 {
        return None;
    }
    geometry(params, area, bounds)?;
    let mut slabs = Vec::new();
    let mut x = area.x0;
    while x < area.x1 {
        // Only the first/last slabs may cut a physical tile, so results never replace a
        // neighbor's partial tile while merging. Widen before rounding negative coordinates.
        let edge = (i64::from(x) + i64::from(SLAB)).div_euclid(i64::from(OUTPUT_TILE)) * i64::from(OUTPUT_TILE);
        let end = edge.min(i64::from(area.x1)) as i32;
        slabs.push(Rect::new(x, area.y0, end, area.y1));
        x = end;
    }
    let done = AtomicU64::new(0);
    let progress: Vec<_> = slabs.iter().map(|_| AtomicU64::new(0)).collect();
    let total = area.size().area() as f64;
    let results = par::map(slabs.len(), true, |i| {
        let cancel = || ctl.cancelled();
        let report = |fraction: f32| {
            let pixels = (f64::from(fraction) * slabs[i].size().area() as f64) as u64;
            let previous = progress[i].swap(pixels, Ordering::Relaxed);
            let delta = pixels.saturating_sub(previous);
            let completed = done.fetch_add(delta, Ordering::Relaxed) + delta;
            ctl.progress(0.98 * (completed as f64 / total) as f32);
        };
        apply_stream(surface, params, slabs[i], bounds, selection, extent, &Interrupt::new(&cancel, &report), slabs.len() == 1)
    });
    let mut output = surface.clone();
    for (slab, result) in slabs.into_iter().zip(results) {
        let Some(mut result) = result? else { return Some(None) };
        output.put_tiles(result.take_tiles(slab));
    }
    if ctl.cancelled() {
        return Some(None);
    }
    output.prune();
    ctl.progress(1.0);
    Some(Some(output))
}

/// `None` falls back to the reference implementation; `Some(None)` means cancellation.
#[allow(clippy::too_many_arguments)]
fn apply_stream(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    extent: Option<Rect>,
    ctl: &Interrupt,
    parallel_rows: bool,
) -> Option<Option<Surface>> {
    if ctl.cancelled() {
        return Some(None);
    }
    let (field, radii, step) = geometry(params, area, bounds)?;
    // The public apply_tiled API can request output outside its read extent. Leave that
    // unusual case to its original per-window clamping rules.
    if extent.is_some_and(|e| !e.contains_rect(&area)) {
        return None;
    }
    let FilterParams::Relight { angle, elevation, intensity, ambient, warmth, softness } = *params else { return None };
    let p = Params { angle, elevation, intensity, ambient, warmth, softness };
    let fmt = surface.format();
    let ctx = Ctx { bounds, mode: fmt.mode, alpha: fmt.alpha };
    let row_len = (field.width() as usize).checked_mul(2)?;
    let mut stages = [BoxRows::new(row_len, radii[0])?, BoxRows::new(row_len, radii[1])?, BoxRows::new(row_len, radii[2])?];
    let mut rows = [zeros(row_len)?, zeros(row_len)?, zeros(row_len)?];
    let mut strip = zeros(row_len.checked_mul(STRIP)?)?;
    let shading_rows = 2 * step as usize + STRIP;
    let shading = zeros((field.width() as usize).checked_mul(shading_rows)?)?;
    // Keep only this slab's tile references. A full source-index clone per stream would
    // multiply metadata for large documents and retain unrelated tiles until every stream ends.
    let mut result = Surface::with_default(fmt, &surface.default_pixel());
    result.put_tiles(area.tiles().filter_map(|tc| surface.tile(tc).map(|tile| (tc, tile.clone()))));
    let mut output = Output { surface, selection, area, field, ctx, p, step, shading, shading_rows, received: 0, next_y: area.y0, result };
    let h = field.height() as usize;
    for y in (0..h).step_by(STRIP) {
        if ctl.cancelled() {
            return Some(None);
        }
        let count = STRIP.min(h - y);
        let values = strip.get_mut(..row_len * count)?;
        let rect = Rect::new(field.x0, field.y0 + y as i32, field.x1, field.y0 + (y + count) as i32);
        read_luma(surface, rect, extent, &ctx, values);
        // All horizontal passes precede all vertical passes, just as in gauss_blur_n.
        // Rows are independent, so parallelism never changes their addition order.
        par::chunks_mut(values, row_len, parallel_rows, |_, row| {
            let mut scratch = Vec::new();
            for r in radii {
                fxutil::box_rows_n(row, field.width() as usize, 2, r, &mut scratch);
            }
        });
        // Reuse the row storage while the vertical pipeline retains only its live windows.
        for row in values.chunks_exact(row_len) {
            if ctl.cancelled() {
                return Some(None);
            }
            if stages[0].push(row, &mut rows[0]) {
                for stage in 1..3 {
                    let (before, after) = rows.split_at_mut(stage);
                    if !stages[stage].push(&before[stage - 1], &mut after[0]) {
                        break;
                    }
                    if stage == 2 && !output.push(&after[0], ctl) {
                        return Some(None);
                    }
                }
            }
        }
    }
    // Repeat the last row at each pass boundary, not the unfiltered input's last row.
    // This preserves the reference's clamping between passes even on a one-row image.
    for stage in 0..3 {
        for _ in 0..radii[stage] {
            if ctl.cancelled() {
                return Some(None);
            }
            stages[stage].repeat(&mut rows[stage]);
            let mut emitted = true;
            for next in stage + 1..3 {
                let (before, after) = rows.split_at_mut(next);
                emitted = stages[next].push(&before[next - 1], &mut after[0]);
                if !emitted {
                    break;
                }
            }
            if emitted && !output.push(&rows[2], ctl) {
                return Some(None);
            }
        }
    }
    if ctl.cancelled() {
        return Some(None);
    }
    output.result.prune();
    ctl.progress(1.0);
    Some(Some(output.result))
}

/// Convert an input strip, repeating the extent's edges even for a wholly exterior halo strip.
fn read_luma(surface: &Surface, rect: Rect, extent: Option<Rect>, ctx: &Ctx, values: &mut [f32]) {
    let mapped = match extent.filter(|e| !e.is_empty()) {
        Some(e) => Rect::new(
            rect.x0.clamp(e.x0, e.x1 - 1),
            rect.y0.clamp(e.y0, e.y1 - 1),
            rect.x1.saturating_sub(1).clamp(e.x0, e.x1 - 1).saturating_add(1),
            rect.y1.saturating_sub(1).clamp(e.y0, e.y1 - 1).saturating_add(1),
        ),
        None => rect,
    };
    let src = Image::read(surface, mapped);
    let w = rect.width() as usize;
    for (row, values) in values.chunks_exact_mut(w * 2).enumerate() {
        let y = (rect.y0 + row as i32).clamp(mapped.y0, mapped.y1 - 1);
        for (x, pair) in values.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let x = (rect.x0 + x as i32).clamp(mapped.x0, mapped.x1 - 1);
            let c = fxutil::rgba(ctx, src.px(x, y));
            let luma = rec709_y(c);
            *pair = if finite_all(&c) && (!ctx.alpha || c[3] > 0.0) && luma.is_finite() { [luma, 1.0] } else { [0.0, 0.0] };
        }
    }
}

/// One vertical box pass, interleaved luma/weight. Its sums follow the same f64 recurrence
/// as box_cols_n: add (enter - leave) once, then round to f32 at each pass's output.
struct BoxRows {
    r: usize,
    len: usize,
    inputs: usize,
    ring: Vec<f32>,
    sums: Vec<f64>,
}

impl BoxRows {
    fn new(len: usize, r: usize) -> Option<Self> {
        let mut sums = Vec::new();
        sums.try_reserve_exact(len).ok()?;
        sums.resize(len, 0.0);
        Some(Self { r, len, inputs: 0, ring: zeros(len.checked_mul(2 * r + 1)?)?, sums })
    }

    fn push(&mut self, input: &[f32], output: &mut [f32]) -> bool {
        let k = self.inputs;
        let count = 2 * self.r + 1;
        if k == 0 {
            for _ in 0..=self.r {
                for (sum, v) in self.sums.iter_mut().zip(input) {
                    *sum += f64::from(*v);
                }
            }
        } else if k <= self.r {
            for (sum, v) in self.sums.iter_mut().zip(input) {
                *sum += f64::from(*v);
            }
        } else {
            let leaving = k.saturating_sub(count) % count * self.len;
            for ((sum, enter), leave) in self.sums.iter_mut().zip(input).zip(&self.ring[leaving..leaving + self.len]) {
                *sum += f64::from(*enter) - f64::from(*leave);
            }
        }
        let slot = k % count * self.len;
        self.ring[slot..slot + self.len].copy_from_slice(input);
        self.inputs += 1;
        if k < self.r {
            return false;
        }
        let norm = 1.0 / count as f64;
        for (v, sum) in output.iter_mut().zip(&self.sums) {
            *v = (*sum * norm) as f32;
        }
        true
    }

    fn repeat(&mut self, output: &mut [f32]) {
        let slot = self.inputs.saturating_sub(1) % (2 * self.r + 1) * self.len;
        let last = self.ring[slot..slot + self.len].to_vec();
        self.push(&last, output);
    }
}

struct Output<'a> {
    surface: &'a Surface,
    selection: Option<&'a Surface>,
    area: Rect,
    field: Rect,
    ctx: Ctx,
    p: Params,
    step: i32,
    shading: Vec<f32>,
    shading_rows: usize,
    received: usize,
    next_y: i32,
    result: Surface,
}

impl Output<'_> {
    fn push(&mut self, row: &[f32], ctl: &Interrupt) -> bool {
        let w = self.field.width() as usize;
        let slot = self.received % self.shading_rows * w;
        for (dst, pair) in self.shading[slot..slot + w].iter_mut().zip(row.as_chunks::<2>().0.iter()) {
            *dst = if pair[1] > 1e-6 && pair[0].is_finite() && pair[1].is_finite() { pair[0] / pair[1] } else { 0.5 };
        }
        let latest = self.field.y0 + self.received as i32;
        self.received += 1;
        let end = self.next_y.saturating_add(STRIP as i32).min(self.area.y1);
        if self.next_y >= end || latest < end - 1 + self.step {
            return !ctl.cancelled();
        }
        let band = Rect::new(self.area.x0, self.next_y, self.area.x1, end);
        let tiles = (band.width() as usize).div_ceil(OUTPUT_TILE as usize);
        let results = par::map(tiles, true, |i| {
            if ctl.cancelled() {
                return Vec::new();
            }
            let x0 = band.x0 + (i * OUTPUT_TILE as usize) as i32;
            let t = Rect::new(x0, band.y0, x0.saturating_add(OUTPUT_TILE).min(band.x1), band.y1);
            let mut src = Image::read(self.surface, t);
            let sample = |x: i32, y: i32| {
                let x = (x.clamp(self.field.x0, self.field.x1 - 1) - self.field.x0) as usize;
                let y = (y.clamp(self.field.y0, self.field.y1 - 1) - self.field.y0) as usize;
                self.shading[y % self.shading_rows * w + x]
            };
            if let Some(sel) = self.selection {
                // Selection mixing needs the original pixel too. Both buffers come from the
                // existing bounded pool, so repeated strips can reuse them at the same size.
                let mut filtered = src.clone();
                shade_pixels(&mut filtered.data, t, &self.ctx, self.p, self.step, sample);
                crate::mix_selection(&mut filtered.data, t, sel, &src, self.ctx.alpha);
                filtered.into_data()
            } else {
                shade_pixels(&mut src.data, t, &self.ctx, self.p, self.step, sample);
                src.into_data()
            }
        });
        // A cancellation callback need not stay true. Never write an empty worker result
        // even if the final cancellation check has already returned to false.
        let n = fxutil::fmt(&self.ctx).channels();
        let incomplete = results.iter().enumerate().any(|(i, data)| {
            let x0 = band.x0 + (i * OUTPUT_TILE as usize) as i32;
            let width = (band.x1 - x0).min(OUTPUT_TILE) as usize;
            data.len() != width * band.height() as usize * n
        });
        if ctl.cancelled() || incomplete {
            return false;
        }
        for (i, data) in results.into_iter().enumerate() {
            let x0 = band.x0 + (i * OUTPUT_TILE as usize) as i32;
            self.result.write_region(Rect::new(x0, band.y0, x0.saturating_add(OUTPUT_TILE).min(band.x1), band.y1), &data);
            photocraft_raster::scratch::give(data);
        }
        self.next_y = end;
        ctl.progress(0.98 * (end - self.area.y0) as f32 / self.area.height() as f32);
        true
    }
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
