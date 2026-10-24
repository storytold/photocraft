//! Average changes colour, never coverage. Keep the existing RGB conversion and selection
//! weighting; Photoshop RGB8 observations are documented in docs/average-filter.md.
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface, from_rgba_into, to_rgba};

use crate::{Ctx, Image};

#[derive(Default)]
struct Mean {
    sum: [f64; 3],
    weight: f64,
}

impl Mean {
    fn add(&mut self, fmt: &PixelFormat, px: &[f32], coverage: f32) {
        let rgba = to_rgba(fmt, px);
        let w = f64::from(coverage);
        for (sum, value) in self.sum.iter_mut().zip(rgba) {
            *sum += f64::from(value) * w;
        }
        self.weight += w;
    }

    fn colour(&self) -> Option<[f32; 3]> {
        (self.weight > 0.0).then(|| self.sum.map(|s| (s / self.weight) as f32))
    }
}

fn mix(fmt: &PixelFormat, px: &mut [f32], colour: [f32; 3], coverage: f32) {
    if coverage <= 0.0 {
        return;
    }
    let mut rgba = to_rgba(fmt, px);
    for (value, mean) in rgba.iter_mut().zip(colour) {
        *value += (mean - *value) * coverage;
    }
    from_rgba_into(fmt, rgba, px);
}

// Rect's public coordinates can span more than i32::MAX. Reject these before raster helpers
// do their signed coordinate arithmetic, and cap a single operation's sample count.
fn valid_area(area: Rect, channels: usize) -> bool {
    area.width() <= i32::MAX as u32
        && area.height() <= i32::MAX as u32
        && u64::from(area.width()).saturating_mul(u64::from(area.height())).saturating_mul(channels as u64) <= 512 * 1024 * 1024
}

// Row-major reduction preserves the dense implementation's f64 addition order. A fixed
// width avoids a full-width allocation even for very wide documents.
const REDUCE_PIXELS: i32 = 4096;
// At most eight native 256x256 tiles (10 MiB at five channels) are live as float results.
const OUTPUT_GROUP: usize = 8;

fn coverage_buffer(selection: Option<&Surface>, width: usize) -> Option<Vec<f32>> {
    let count = if selection.is_some() { width } else { 0 };
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(count).ok()?;
    buffer.resize(count, 0.0);
    Some(buffer)
}

pub(crate) fn apply(surface: &Surface, area: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Option<Surface> {
    let fmt = surface.format();
    let n = fmt.channels();
    // Scratch is bounded now: limit total work in pixels, not full-frame decoded samples.
    if !valid_area(area, 1) || ctl.cancelled() {
        return None;
    }
    if area.is_empty() {
        return Some(surface.clone());
    }
    let total = u64::from(area.width()) * u64::from(area.height());
    let mut done = 0u64;
    let mut mean = Mean::default();
    let mut cover = coverage_buffer(selection, area.width().min(REDUCE_PIXELS as u32) as usize)?;
    for y in area.y0..area.y1 {
        let mut x = area.x0;
        while x < area.x1 {
            if ctl.cancelled() {
                return None;
            }
            let end = x.saturating_add(REDUCE_PIXELS).min(area.x1);
            let row = Rect::new(x, y, end, y + 1);
            let raw = surface.try_read_region(row).ok()?;
            if let Some(selection) = selection {
                let cover = cover.get_mut(..row.width() as usize)?;
                selection.sample_row_strided(y, x, 1, 0, cover);
                for (px, &coverage) in raw.chunks_exact(n).zip(cover.iter()) {
                    mean.add(&fmt, px, coverage);
                }
            } else {
                for px in raw.chunks_exact(n) {
                    mean.add(&fmt, px, 1.0);
                }
            }
            done += u64::from(row.width());
            ctl.progress(0.5 * done as f32 / total as f32);
            x = end;
        }
    }
    drop(cover);
    if ctl.cancelled() {
        return None;
    }
    let Some(colour) = mean.colour() else {
        ctl.progress(1.0);
        return Some(surface.clone());
    };
    let run = |r: &Rect| -> Option<(Rect, Vec<f32>)> {
        if ctl.cancelled() {
            return None;
        }
        let mut raw = surface.try_read_region(*r).ok()?;
        let width = r.width() as usize;
        let row_samples = width.checked_mul(n)?;
        if row_samples == 0 {
            return None;
        }
        let mut cover = coverage_buffer(selection, width)?;
        for (y, row) in raw.chunks_exact_mut(row_samples).enumerate() {
            if ctl.cancelled() {
                return None;
            }
            if let Some(selection) = selection {
                selection.sample_row_strided(r.y0 + y as i32, r.x0, 1, 0, &mut cover);
                for (px, &coverage) in row.chunks_exact_mut(n).zip(&cover) {
                    mix(&fmt, px, colour, coverage);
                }
            } else {
                for px in row.chunks_exact_mut(n) {
                    mix(&fmt, px, colour, 1.0);
                }
            }
        }
        Some((*r, raw))
    };
    let mut out = surface.clone();
    let mut tiles = area.tiles();
    done = 0;
    loop {
        if ctl.cancelled() {
            return None;
        }
        let chunk: Vec<_> = tiles.by_ref().take(OUTPUT_GROUP).map(|t| t.rect().intersect(&area)).collect();
        if chunk.is_empty() {
            break;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let results: Option<Vec<_>> = {
            use rayon::prelude::*;
            chunk.par_iter().map(run).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let results: Option<Vec<_>> = chunk.iter().map(run).collect();
        for (r, data) in results? {
            if ctl.cancelled() {
                return None;
            }
            out.try_write_region(r, &data).ok()?;
            done += u64::from(r.width()) * u64::from(r.height());
        }
        ctl.progress(0.5 + 0.5 * done as f32 / total as f32);
    }
    out.prune();
    if ctl.cancelled() {
        return None;
    }
    ctl.progress(1.0);
    Some(out)
}

// The direct kernel API has no selection. Its mean is over the reference bounds, even when
// the requested output is only a tile; the surface dispatcher above also handles selections.
pub(crate) fn kernel(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    let fmt = PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha);
    let n = fmt.channels();
    if src.ch != n || !valid_area(src.rect, n) || !valid_area(out, n) {
        return Vec::new();
    }
    let read = |x: i32, y: i32| -> Option<&[f32]> {
        if !src.rect.contains(x, y) {
            return None;
        }
        let i = (i64::from(y) - i64::from(src.rect.y0)) as usize;
        let j = (i64::from(x) - i64::from(src.rect.x0)) as usize;
        let start = i.checked_mul(src.rect.width() as usize)?.checked_add(j)?.checked_mul(n)?;
        src.data.get(start..start.checked_add(n)?)
    };
    let bounds = ctx.bounds.intersect(&src.rect);
    let mut mean = Mean::default();
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            if let Some(px) = read(x, y) {
                mean.add(&fmt, px, 1.0);
            }
        }
    }
    let len = (out.width() as usize).checked_mul(out.height() as usize).and_then(|s| s.checked_mul(n));
    let Some(len) = len else { return Vec::new() };
    let mut result = Vec::new();
    if result.try_reserve_exact(len).is_err() {
        return result;
    }
    let colour = mean.colour();
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let mut px = [0.0; 5];
            let Some(pixel) = px.get_mut(..n) else { return Vec::new() };
            if let Some(source) = read(x, y) {
                pixel.copy_from_slice(source);
            }
            if let Some(colour) = colour {
                mix(&fmt, pixel, colour, 1.0);
            }
            result.extend_from_slice(pixel);
        }
    }
    result
}

#[cfg(test)]
mod tests;
