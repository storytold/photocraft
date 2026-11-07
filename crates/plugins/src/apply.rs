//! Running a filter plug-in over a surface, band by band.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::manifest::Area;
use crate::runtime::{Deadline, Plugin, RunBudget};
use crate::{Error, Result};

/// The `format` argument of `pc_filter`: bits 0–7 the source bit depth (8, 16 or 32), bit 8 set
/// when the last channel is (straight) alpha, bits 16–23 the colour mode as numbered in the PSD
/// file format (1 Grayscale, 2 Indexed, 3 RGB, 4 CMYK, 7 Multichannel, 8 Duotone, 9 Lab;
/// 0 Bitmap).
pub fn format_code(f: PixelFormat) -> u32 {
    let depth = match f.sample {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        SampleType::F32 => 32,
    };
    let mode = match f.mode {
        ColorMode::Bitmap => 0,
        ColorMode::Grayscale => 1,
        ColorMode::Indexed => 2,
        ColorMode::Rgb => 3,
        ColorMode::Cmyk => 4,
        ColorMode::Multichannel => 7,
        ColorMode::Duotone => 8,
        ColorMode::Lab => 9,
    };
    depth | (u32::from(f.alpha) << 8) | (mode << 16)
}

fn mode_name(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "grayscale",
        ColorMode::Indexed => "indexed",
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        ColorMode::Multichannel => "multichannel",
        ColorMode::Duotone => "duotone",
    }
}

/// Reads `rect`, repeating the edge pixels of `extent` outside it (like built-in neighbourhood
/// filters at the canvas edge).
fn read_clamped(s: &Surface, rect: Rect, extent: Rect) -> Vec<f32> {
    let inner = rect.intersect(&extent);
    if inner.is_empty() || inner == rect {
        return s.read_region(rect);
    }
    let src = s.read_region(inner);
    let n = s.channels();
    let (w, iw) = (rect.width() as usize, inner.width() as usize);
    let mut out = vec![0.0f32; w * rect.height() as usize * n];
    for (row, y) in (rect.y0..rect.y1).enumerate() {
        let sy = (y.clamp(inner.y0, inner.y1 - 1) - inner.y0) as usize;
        for (col, x) in (rect.x0..rect.x1).enumerate() {
            let sx = (x.clamp(inner.x0, inner.x1 - 1) - inner.x0) as usize;
            let s = (sy * iw + sx) * n;
            let d = (row * w + col) * n;
            if let (Some(dst), Some(srcpx)) = (out.get_mut(d..d + n), src.get(s..s + n)) {
                dst.copy_from_slice(srcpx);
            }
        }
    }
    out
}

/// Most bands one run is split into.
const MAX_BANDS: u64 = 1 << 16;

/// Errors from a band that hit the shared memory cap may have the same variant as unrelated
/// plug-in errors, so use the band's denial flag rather than the error's shape.
fn hard_error_in_discarded_attempt(results: &[(Result<Surface>, bool)]) -> Option<Error> {
    results.iter().find_map(|(result, denied)| if *denied { None } else { result.as_ref().err().cloned() })
}

impl Plugin {
    /// The rectangle a run writes: the layer's content (grown by the plug-in's overlap) or the
    /// canvas, limited to the selection's bounds.
    pub fn output_area(&self, surface: &Surface, canvas: Rect, selection: Option<&Surface>) -> Rect {
        let content = surface.content_bounds();
        let extent = canvas.union(&content);
        let mut area = match self.manifest().area {
            Area::Content if content.is_empty() => Rect::EMPTY,
            Area::Content => content.inflate(self.manifest().overlap as i32).intersect(&extent),
            Area::Canvas => canvas,
        };
        if let Some(sel) = selection {
            area = area.intersect(&sel.content_bounds());
        }
        area
    }

    /// Runs the filter on `surface` with user `params` (validated against the manifest) and
    /// returns the new surface. `canvas` is the document bounds; `selection` (coverage in
    /// channel 0) limits and feathers the result, as for built-in filters.
    pub fn apply(&self, surface: &Surface, canvas: Rect, selection: Option<&Surface>, params: &Value) -> Result<Surface> {
        let resolved = self.manifest().resolve_params(params)?;
        let area = self.output_area(surface, canvas, selection);
        let mut out = surface.clone();
        if area.is_empty() {
            return Ok(out);
        }
        let fmt = surface.format();
        let n = fmt.channels();
        let ov = self.manifest().overlap as i32;
        let extent = canvas.union(&surface.content_bounds());
        // Bands are tile-aligned blocks, one tile row tall and as many tiles wide as fit in
        // `band_bytes`, so each worker can encode its own tiles and the merge is a tile copy.
        let tile_bytes = (TILE_SIZE as usize).pow(2) * n * 4;
        // Half the plug-in's memory cap is for the band (the module needs room of its own).
        let room = self.limits().max_memory_bytes / 2;
        let across = (self.limits().band_bytes.min(room) / tile_bytes).clamp(1, 1024) as i32;
        let in_bytes = ((across * TILE_SIZE + 2 * ov) as usize) * ((TILE_SIZE + 2 * ov) as usize) * n * 4;
        if in_bytes > room {
            return Err(Error::Limit("memory budget (a band does not fit in the plug-in's memory)".into()));
        }
        let mut bands = Vec::new();
        let (t0, t1) = (TileCoord::containing(area.x0, area.y0), TileCoord::containing(area.x1 - 1, area.y1 - 1));
        // Counted before any is made: a few distant tiles make `area` a grid of millions of
        // bands, each a call into the plug-in, planned before the deadline starts.
        let columns = (i64::from(t1.tx) - i64::from(t0.tx) + 1).max(0) as u64;
        let rows = (i64::from(t1.ty) - i64::from(t0.ty) + 1).max(0) as u64;
        if columns.div_ceil(across as u64).saturating_mul(rows) > MAX_BANDS {
            return Err(Error::Limit(format!("more than {MAX_BANDS} bands (the layer's content is spread too widely)")));
        }
        for ty in t0.ty..=t1.ty {
            let mut tx = t0.tx;
            while tx <= t1.tx {
                let r = Rect::new(tx * TILE_SIZE, ty * TILE_SIZE, (tx + across).min(t1.tx + 1) * TILE_SIZE, (ty + 1) * TILE_SIZE);
                bands.push(r.intersect(&area));
                tx += across;
            }
        }
        let deadline = Deadline::new(self.limits());
        let ctx = BandCtx { surface, extent, canvas, selection, fmt, ov, params: Value::Object(resolved) };
        let budget = RunBudget::new(self.limits());
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads();
        // Start with the conservative bound. Its first group supplies a real memory footprint
        // for later groups while preserving the existing parallel error cancellation.
        #[cfg(not(target_arch = "wasm32"))]
        let mut group = self.limits().parallel_bands(workers);
        #[cfg(target_arch = "wasm32")]
        let mut group = 1;
        let mut remaining = bands.as_slice();
        let mut measured = false;
        while !remaining.is_empty() {
            let chunk_size = group.min(remaining.len());
            let mut chunk = &remaining[..chunk_size];
            let mut width = group;
            while !chunk.is_empty() {
                let size = width.min(chunk.len());
                let slice = &chunk[..size];
                let denials = budget.snapshot().1;
                let attempt = deadline.attempt();
                let run = |b: &Rect| {
                    let denied = Arc::new(AtomicBool::new(false));
                    let r = self.run_band(&ctx, *b, &attempt, Arc::clone(&budget), Arc::clone(&denied)).map(|data| encode_band(surface, *b, &data));
                    let was_denied = denied.load(Ordering::Relaxed);
                    if let Err(error) = &r
                        && !was_denied
                    {
                        attempt.abort(error.clone());
                    }
                    (r, was_denied)
                };
                #[cfg(not(target_arch = "wasm32"))]
                let results: Vec<(Result<Surface>, bool)> = {
                    use rayon::prelude::*;
                    slice.par_iter().map(run).collect()
                };
                #[cfg(target_arch = "wasm32")]
                let results: Vec<(Result<Surface>, bool)> = slice.iter().map(run).collect();
                if budget.snapshot().1 != denials {
                    // A later band used more memory than the probe. Discard the entire attempt
                    // (including successful bands) and retry with fewer concurrent instances.
                    if let Some(error) = hard_error_in_discarded_attempt(&results) {
                        return Err(error);
                    }
                    if size == 1 {
                        return Err(Error::Limit("memory budget".into()));
                    }
                    width = (size / 2).max(1);
                    continue;
                }
                for (r, _) in results {
                    for (c, t) in r?.tiles() {
                        out.tile_mut(*c).bytes_mut().copy_from_slice(&t.bytes());
                    }
                }
                chunk = &chunk[size..];
            }
            remaining = &remaining[chunk_size..];
            if !measured {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    group = self.limits().parallel_bands_for(workers, budget.snapshot().0);
                }
                measured = true;
            } else {
                group = group.min(width);
            }
        }
        out.prune();
        Ok(out)
    }

    fn run_band(&self, c: &BandCtx, band: Rect, deadline: &Deadline, budget: Arc<RunBudget>, denied: Arc<AtomicBool>) -> Result<Vec<f32>> {
        let n = c.fmt.channels();
        let input = band.inflate(c.ov);
        let mut data = read_clamped(c.surface, input, c.extent);
        let (iw, ih) = (input.width(), input.height());
        let mut params = c.params.clone();
        if let Value::Object(m) = &mut params {
            m.insert(
                "_image".into(),
                json!({
                    "x": input.x0, "y": input.y0, "width": iw, "height": ih, "overlap": c.ov,
                    "canvasWidth": c.canvas.width(), "canvasHeight": c.canvas.height(),
                    "mode": mode_name(c.fmt.mode), "depth": format_code(c.fmt) & 0xff, "alpha": c.fmt.alpha, "channels": n,
                }),
            );
        }
        let pbytes = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
        let orig_full = data.clone();
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.filter_buffer(&mut data, iw, ih, n as u32, format_code(c.fmt), &pbytes, deadline, budget, denied)
        }));
        run.map_err(|_| Error::Trap("the runtime panicked".into()))??;
        // Keep the band (drop the overlap margins), sanitise and blend by the selection.
        let (bw, bh) = (band.width() as usize, band.height() as usize);
        let ov = c.ov as usize;
        let mut out = vec![0.0f32; bw * bh * n];
        let sel_row = c.selection.map(|s| s.format().channels());
        let mut sel = Vec::new();
        let int_depth = c.fmt.sample != SampleType::F32;
        for row in 0..bh {
            if let (Some(s), Some(sn)) = (c.selection, sel_row) {
                let r = Rect::new(band.x0, band.y0 + row as i32, band.x1, band.y0 + row as i32 + 1);
                sel = s.read_region(r).chunks_exact(sn.max(1)).map(|p| p.first().copied().unwrap_or(0.0).clamp(0.0, 1.0)).collect();
            }
            for col in 0..bw {
                let si = ((row + ov) * (bw + 2 * ov) + col + ov) * n;
                let di = (row * bw + col) * n;
                let k = if c.selection.is_some() { sel.get(col).copied().unwrap_or(0.0) } else { 1.0 };
                for ch in 0..n {
                    let o = orig_full.get(si + ch).copied().unwrap_or(0.0);
                    let mut v = data.get(si + ch).copied().unwrap_or(o);
                    if !v.is_finite() {
                        v = o;
                    }
                    if int_depth || (c.fmt.alpha && ch == n - 1) {
                        v = v.clamp(0.0, 1.0);
                    }
                    if let Some(d) = out.get_mut(di + ch) {
                        // As a built-in edit mixes through the selection (#9483).
                        *d = o * (1.0 - k) + v * k;
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The tiles under `band` with the band's new pixels written in (the rest of each tile is the
/// original), encoded in the surface's format: built on the worker thread.
fn encode_band(surface: &Surface, band: Rect, data: &[f32]) -> Surface {
    let mut s = Surface::with_default(surface.format(), &surface.default_pixel());
    for c in band.tiles() {
        if c.rect().intersect(&band) != c.rect()
            && let Some(t) = surface.tile(c)
        {
            // Only a partial tile needs its pixels outside the band preserved.
            s.tile_mut(c).bytes_mut().copy_from_slice(&t.bytes());
        }
    }
    s.write_region(band, data);
    s
}

struct BandCtx<'a> {
    surface: &'a Surface,
    extent: Rect,
    canvas: Rect,
    selection: Option<&'a Surface>,
    fmt: PixelFormat,
    ov: i32,
    params: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discarded_attempt_keeps_a_hard_error_even_when_variants_match() {
        let results = [(Err(Error::Failed("allocation failed".into())), true), (Err(Error::Failed("plug-in failed independently".into())), false)];
        assert_eq!(hard_error_in_discarded_attempt(&results), Some(Error::Failed("plug-in failed independently".into())));
    }

    #[test]
    fn run_band_forwards_the_shared_budget() {
        let limits = crate::Limits { max_memory_bytes: 4 << 20, max_total_memory_bytes: 5 << 20, ..crate::Limits::default() };
        let plugin = Plugin::load(include_bytes!("../tests/fixtures/invert.wasm"), limits.clone()).unwrap();
        let mut surface = Surface::new(PixelFormat::RGBA8);
        let canvas = Rect::new(0, 0, 256, 256);
        surface.fill_rect(canvas, &[0.25, 0.5, 0.75, 1.0]);
        let budget = RunBudget::new(&limits);
        budget.occupy_for_test(limits.max_total_memory_bytes);
        let denied = Arc::new(AtomicBool::new(false));
        let ctx = BandCtx { surface: &surface, extent: canvas, canvas, selection: None, fmt: surface.format(), ov: 0, params: json!({}) };

        assert!(plugin.run_band(&ctx, canvas, &Deadline::new(&limits), Arc::clone(&budget), Arc::clone(&denied)).is_err());
        assert!(denied.load(Ordering::Relaxed));
        assert!(budget.snapshot().1 > 0, "the band's instance must charge the shared budget");
    }

    #[test]
    fn encoding_preserves_original_pixels_outside_partial_tiles() {
        let mut source = Surface::new(PixelFormat::RGBA8);
        source.fill_rect(Rect::new(0, 0, 512, 256), &[0.0, 1.0, 0.0, 1.0]);
        let band = Rect::new(10, 0, 512, 256);
        let data = [1.0, 0.0, 0.0, 1.0].repeat(band.width() as usize * band.height() as usize);

        let encoded = encode_band(&source, band, &data);
        assert_eq!(encoded.pixel(0, 0), source.pixel(0, 0));
        assert_eq!(encoded.pixel(9, 255), source.pixel(9, 255));
        assert_eq!(encoded.pixel(10, 0), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(encoded.pixel(400, 255), vec![1.0, 0.0, 0.0, 1.0]);
    }
}
