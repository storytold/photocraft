//! Running a filter plug-in over a surface, band by band.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::manifest::Area;
use crate::runtime::{Deadline, Limits, Plugin};
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

impl Plugin {
    /// The rectangle a run writes: the layer's content (grown by the plug-in's overlap) or the
    /// canvas, limited to the selection's bounds. This parameter-free query uses the manifest
    /// fallback; `apply` resolves an optional `pc_overlap` before calculating its actual bounds.
    pub fn output_area(&self, surface: &Surface, canvas: Rect, selection: Option<&Surface>) -> Rect {
        self.output_area_with_overlap(surface, canvas, selection, self.manifest().overlap as i32)
    }

    fn output_area_with_overlap(&self, surface: &Surface, canvas: Rect, selection: Option<&Surface>, overlap: i32) -> Rect {
        let content = surface.content_bounds();
        let extent = canvas.union(&content);
        let mut area = match self.manifest().area {
            Area::Content if content.is_empty() => Rect::EMPTY,
            Area::Content => content.inflate(overlap).intersect(&extent),
            Area::Canvas => canvas,
        };
        if let Some(sel) = selection {
            area = area.intersect(&sel.content_bounds());
        }
        area
    }

    /// Runs the filter on `surface` with user `params` (validated against the manifest) and
    /// returns the new surface. `canvas` is the document bounds; `selection` (coverage in
    /// channel 0) normally limits and feathers the result. Opt-in `selectionShape` choices
    /// instead use it as the input silhouette, composite onto the unselected source, and permit
    /// the effect to extend outside the selection by at most the resolved overlap.
    pub fn apply(&self, surface: &Surface, canvas: Rect, selection: Option<&Surface>, params: &Value) -> Result<Surface> {
        let resolved = self.manifest().resolve_params(params)?;
        let shape = selection.filter(|_| self.manifest().uses_selection_shape(&resolved)).map(Surface::content_bounds);
        let clamp_context = self.manifest().uses_clamped_context(&resolved);
        let params = Value::Object(resolved);
        let deadline = Deadline::new(self.limits());
        let ov = self.overlap_for_params(&params, &deadline)? as i32;
        let area = if let Some(bounds) = shape {
            if bounds.is_empty() { Rect::EMPTY } else { self.output_area_with_overlap(surface, canvas, None, ov).intersect(&bounds.inflate(ov)) }
        } else {
            self.output_area_with_overlap(surface, canvas, selection, ov)
        };
        let mut out = surface.clone();
        if area.is_empty() {
            return Ok(out);
        }
        let fmt = PixelFormat { alpha: surface.format().alpha || shape.is_some(), ..surface.format() };
        let n = fmt.channels();
        let extent = canvas.union(&surface.content_bounds());
        // Large-context runs group whole tiles in both dimensions to avoid repeatedly
        // processing a large halo around tiny output regions. Other runs retain short bands.
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads().max(1);
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        let clamp_context = clamp_context && ov > 256;
        let (bands, concurrency) = if ov > 256 {
            (context_bands(area, extent, ov, n, self.limits(), clamp_context)?, 1)
        } else {
            let layout = band_layout(self.limits(), n, ov as usize, workers)?;
            let mut bands = Vec::new();
            let (t0, t1) = (TileCoord::containing(area.x0, area.y0), TileCoord::containing(area.x1 - 1, area.y1 - 1));
            // Counted before any is made: a few distant tiles make `area` a grid of millions of
            // bands, each a call into the plug-in, planned before the deadline starts.
            let columns = (i64::from(t1.tx) - i64::from(t0.tx) + 1).max(0) as u64;
            let rows = (i64::from(t1.ty) - i64::from(t0.ty) + 1).max(0) as u64;
            if columns.div_ceil(layout.across.max(1) as u64).saturating_mul(rows) > MAX_BANDS {
                return Err(Error::Limit(format!("more than {MAX_BANDS} bands (the layer's content is spread too widely)")));
            }
            for ty in t0.ty..=t1.ty {
                let mut tx = t0.tx;
                while tx <= t1.tx {
                    bands.push(tile_block(tx, ty, layout.across, 1).intersect(&area));
                    tx = tx.saturating_add(layout.across);
                }
            }
            (bands, layout.workers)
        };
        let ctx = BandCtx { surface, extent, canvas, selection, shape, fmt, ov, clamp_context, params };
        let run = |b: &Rect| {
            let r = self.run_band(&ctx, *b, &deadline).map(|data| encode_band(surface, *b, &data));
            if let Err(error) = &r {
                deadline.abort(error.clone());
            }
            r
        };
        // Large halos also include per-worker host copies: don't multiply them by every CPU.
        for chunk in bands.chunks(concurrency) {
            #[cfg(not(target_arch = "wasm32"))]
            let results: Vec<Result<Surface>> = {
                use rayon::prelude::*;
                chunk.par_iter().map(run).collect()
            };
            #[cfg(target_arch = "wasm32")]
            let results: Vec<Result<Surface>> = chunk.iter().map(run).collect();
            for r in results {
                for (c, t) in r?.tiles() {
                    out.tile_mut(*c).bytes_mut().copy_from_slice(t.bytes());
                }
            }
        }
        out.prune();
        Ok(out)
    }

    fn run_band(&self, c: &BandCtx, band: Rect, deadline: &Deadline) -> Result<Vec<f32>> {
        let n = c.fmt.channels();
        let input = context_rect(band, c.extent, c.ov, c.clamp_context);
        let orig_full = read_clamped(c.surface, input, c.extent);
        let source_n = c.surface.channels();
        let mut data = if c.shape.is_some() {
            let coverage = c.selection.map(|s| s.read_region(input)).unwrap_or_default();
            let selection_n = c.selection.map_or(1, Surface::channels);
            let mut masked = Vec::with_capacity(orig_full.len() / source_n * n);
            for (index, pixel) in orig_full.chunks_exact(source_n).enumerate() {
                let alpha = if c.surface.format().alpha { pixel.last().copied().unwrap_or(0.0) } else { 1.0 };
                masked.extend_from_slice(pixel.get(..n - 1).unwrap_or_default());
                masked.push(unit(alpha) * unit(coverage.get(index * selection_n).copied().unwrap_or(0.0)));
            }
            masked
        } else {
            orig_full.clone()
        };
        let (iw, ih) = (input.width(), input.height());
        let mut params = c.params.clone();
        if let Value::Object(m) = &mut params {
            let mut context = json!({
                "x": input.x0, "y": input.y0, "width": iw, "height": ih, "overlap": c.ov,
                "canvasWidth": c.canvas.width(), "canvasHeight": c.canvas.height(),
                "mode": mode_name(c.fmt.mode), "depth": format_code(c.fmt) & 0xff, "alpha": c.fmt.alpha, "channels": n,
            });
            if c.clamp_context {
                context["outputX"] = json!(i64::from(band.x0) - i64::from(input.x0));
                context["outputY"] = json!(i64::from(band.y0) - i64::from(input.y0));
                context["outputWidth"] = json!(band.width());
                context["outputHeight"] = json!(band.height());
            }
            if let Some(shape) = c.shape {
                context["shapeX"] = json!(shape.x0);
                context["shapeY"] = json!(shape.y0);
                context["shapeWidth"] = json!(shape.width());
                context["shapeHeight"] = json!(shape.height());
            }
            m.insert("_image".into(), context);
        }
        let pbytes = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
        let run =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.filter_buffer(&mut data, iw, ih, n as u32, format_code(c.fmt), &pbytes, deadline)));
        run.map_err(|_| Error::Trap("the runtime panicked".into()))??;
        // Keep the band (drop the overlap margins), sanitise and blend by the selection.
        let (bw, bh) = (band.width() as usize, band.height() as usize);
        let left = (i64::from(band.x0) - i64::from(input.x0)) as usize;
        let top = (i64::from(band.y0) - i64::from(input.y0)) as usize;
        let input_width = iw as usize;
        let mut out = vec![0.0f32; bw * bh * source_n];
        let sel_row = c.selection.map(|s| s.format().channels());
        let mut sel = Vec::new();
        let int_depth = c.fmt.sample != SampleType::F32;
        for row in 0..bh {
            if let (Some(s), Some(sn)) = (c.selection, sel_row) {
                let r = Rect::new(band.x0, band.y0 + row as i32, band.x1, band.y0 + row as i32 + 1);
                sel = s.read_region(r).chunks_exact(sn.max(1)).map(|p| p.first().copied().unwrap_or(0.0).clamp(0.0, 1.0)).collect();
            }
            for col in 0..bw {
                let si = ((row + top) * input_width + col + left) * n;
                let di = (row * bw + col) * source_n;
                let original_index = ((row + top) * input_width + col + left) * source_n;
                let k = if c.selection.is_some() { sel.get(col).copied().unwrap_or(0.0) } else { 1.0 };
                if c.shape.is_some() {
                    if let (Some(original), Some(filtered), Some(dst)) =
                        (orig_full.get(original_index..original_index + source_n), data.get(si..si + n), out.get_mut(di..di + source_n))
                    {
                        composite_shape(original, filtered, dst, c.surface.format(), k);
                    }
                    continue;
                }
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
                        *d = o + (v - o) * k;
                    }
                }
            }
        }
        Ok(out)
    }
}

struct BandLayout {
    across: i32,
    workers: usize,
}

/// Preserve short auxiliary fields and sampling stencils at image edges. Plug-ins whose
/// dependency reaches farther outside the extent must opt out of clipped context for that choice.
/// Guard pixels retain ordinary edge repetition and zero selection coverage.
const CONTEXT_GUARD: i32 = 36;

fn context_rect(band: Rect, extent: Rect, overlap: i32, clipped: bool) -> Rect {
    let padded = band.inflate(overlap);
    if clipped { padded.intersect(&extent.inflate(CONTEXT_GUARD)) } else { padded }
}

fn tile_block(tx: i32, ty: i32, across: i32, down: i32) -> Rect {
    let edge = |v: i64| (v * i64::from(TILE_SIZE)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    Rect::new(edge(i64::from(tx)), edge(i64::from(ty)), edge(i64::from(tx) + i64::from(across)), edge(i64::from(ty) + i64::from(down)))
}

fn context_bytes(input: Rect, channels: usize) -> Option<usize> {
    (input.width() as usize).checked_mul(input.height() as usize)?.checked_mul(channels)?.checked_mul(4)
}

fn large_input_budget(limits: &Limits) -> usize {
    // Reserve module/allocator overhead, then room for source and intermediate fields. Bound
    // one active native instance plus host pixel copies without relaxing caller-supplied limits.
    const MODULE_RESERVE: usize = 16 << 20;
    const ACTIVE_MEMORY_BUDGET: usize = 2 << 30;
    let wasm_room = limits.max_memory_bytes.saturating_sub(MODULE_RESERVE) / 3;
    let host_room = ACTIVE_MEMORY_BUDGET.saturating_sub(limits.max_memory_bytes) / 3;
    wasm_room.min(host_room)
}

fn context_bands(area: Rect, extent: Rect, overlap: i32, channels: usize, limits: &Limits, clipped: bool) -> Result<Vec<Rect>> {
    let fail = || Error::Limit("memory budget (a band and its context do not fit in the plug-in's memory)".into());
    if channels == 0 || overlap < 0 {
        return Err(fail());
    }
    if area.is_empty() {
        return Ok(Vec::new());
    }
    let budget = large_input_budget(limits);
    let (t0, t1) = (TileCoord::containing(area.x0, area.y0), TileCoord::containing(area.x1 - 1, area.y1 - 1));
    let fits = |tx: i32, ty: i32, across: i32, down: i32| {
        let band = tile_block(tx, ty, across, down).intersect(&area);
        context_bytes(context_rect(band, extent, overlap, clipped), channels).is_some_and(|n| n <= budget)
    };
    let width_for = |tx: i32, ty: i32, down: i32| {
        let (mut low, mut high) = (0, (t1.tx - tx + 1).min(1024));
        while low < high {
            let middle = low + (high - low + 1) / 2;
            if fits(tx, ty, middle, down) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        low
    };
    let mut bands = Vec::new();
    let mut ty = t0.ty;
    while ty <= t1.ty {
        let mut best = (0_u64, 0_i32);
        for down in 1..=(t1.ty - ty + 1).min(1024) {
            // A tall block that fits at the left edge must still admit a minimum-width block
            // in the middle of this row, where its horizontal context can be larger.
            let strip = context_rect(tile_block(t0.tx, ty, 1, down).intersect(&area), extent, overlap, clipped);
            let full_width = TILE_SIZE as u64 + 2 * overlap as u64;
            let width = if clipped { u64::from(extent.inflate(CONTEXT_GUARD).width()).min(full_width) } else { full_width };
            let minimum = width.checked_mul(u64::from(strip.height())).and_then(|n| n.checked_mul(channels as u64)).and_then(|n| n.checked_mul(4));
            if minimum.is_none_or(|n| n > budget as u64) {
                break;
            }
            let across = width_for(t0.tx, ty, down);
            if across == 0 {
                break;
            }
            let rect = tile_block(t0.tx, ty, across, down).intersect(&area);
            let pixels = u64::from(rect.width()) * u64::from(rect.height());
            if pixels > best.0 {
                best = (pixels, down);
            }
        }
        let down = best.1;
        if down == 0 {
            return Err(fail());
        }
        let mut tx = t0.tx;
        while tx <= t1.tx {
            let across = width_for(tx, ty, down);
            if across == 0 {
                return Err(fail());
            }
            if bands.len() as u64 >= MAX_BANDS {
                return Err(Error::Limit(format!("more than {MAX_BANDS} bands (the layer's content is spread too widely)")));
            }
            bands.push(tile_block(tx, ty, across, down).intersect(&area));
            tx = tx.saturating_add(across);
        }
        ty = ty.saturating_add(down);
    }
    Ok(bands)
}

fn band_layout(limits: &Limits, channels: usize, overlap: usize, workers: usize) -> Result<BandLayout> {
    let memory_error = || Error::Limit("memory budget (a band and its context do not fit in the plug-in's memory)".into());
    let tile = TILE_SIZE as usize;
    let sample_bytes = channels.checked_mul(4).filter(|n| *n != 0).ok_or_else(memory_error)?;
    let tile_bytes = tile.checked_mul(tile).and_then(|n| n.checked_mul(sample_bytes)).ok_or_else(memory_error)?;
    // Preserve ordinary-filter sizing. Large neighborhoods need room for a source copy and
    // intermediate fields as well as the supplied input; the runtime still enforces the cap.
    let large = overlap > 256;
    let room = if large { large_input_budget(limits) } else { limits.max_memory_bytes / 2 };
    let halo = overlap.checked_mul(2).ok_or_else(memory_error)?;
    let height = tile.checked_add(halo).ok_or_else(memory_error)?;
    let row_bytes = height.checked_mul(sample_bytes).ok_or_else(memory_error)?;
    let padded_width = room / row_bytes;
    let fitting_tiles = padded_width.saturating_sub(halo) / tile;
    if fitting_tiles == 0 {
        return Err(memory_error());
    }
    let across = (limits.band_bytes.min(room) / tile_bytes).clamp(1, 1024).min(fitting_tiles);
    let input_bytes = across.checked_mul(tile).and_then(|n| n.checked_add(halo)).and_then(|n| n.checked_mul(row_bytes)).ok_or_else(memory_error)?;
    if input_bytes > room {
        return Err(memory_error());
    }
    let workers = if large { 1 } else { workers.max(1) };
    Ok(BandLayout { across: across as i32, workers })
}

/// Sanitise untrusted coverage, treating non-finite coverage as empty.
fn unit(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

/// Replace the selected cut-out with its filtered result, keeping the unselected source below.
/// The remainder is conditional coverage: `a = selected + remainder * (1 - selected)`.
/// Simply source-overing onto the original doubles partially transparent input alpha; applying
/// the selection again instead would square feather coverage and clip outer shadows/glows.
fn composite_shape(original: &[f32], filtered: &[f32], dst: &mut [f32], fmt: PixelFormat, coverage: f32) {
    let colors = fmt.mode.color_channels();
    let alpha = if fmt.alpha { unit(original.get(colors).copied().unwrap_or(0.0)) } else { 1.0 };
    let selected = alpha * unit(coverage);
    let value = |ch: usize| {
        let initial = if ch == colors { selected } else { original.get(ch).copied().unwrap_or(0.0) };
        let filtered = filtered.get(ch).copied().filter(|v| v.is_finite()).unwrap_or(initial);
        if ch == colors || fmt.sample != SampleType::F32 { filtered.clamp(0.0, 1.0) } else { filtered }
    };
    // Preserve hidden colour and exact source encodings for identity effects, including empty
    // source pixels and a no-op in a feathered selection.
    if (0..colors).all(|ch| value(ch) == original.get(ch).copied().unwrap_or(0.0)) && value(colors) == selected {
        dst.copy_from_slice(original);
        return;
    }
    let foreground = value(colors);
    let remainder = if selected < 1.0 { (alpha - selected) / (1.0 - selected) } else { 0.0 };
    let behind = remainder * (1.0 - foreground);
    let out_alpha = foreground + behind;
    for (ch, target) in dst.iter_mut().take(colors).enumerate() {
        *target = if out_alpha > 0.0 {
            (value(ch) * foreground + original.get(ch).copied().unwrap_or(0.0) * behind) / out_alpha
        } else {
            original.get(ch).copied().unwrap_or(0.0)
        };
    }
    if fmt.alpha
        && let Some(target) = dst.get_mut(colors)
    {
        *target = out_alpha;
    }
}

/// The tiles under `band` with the band's new pixels written in (the rest of each tile is the
/// original), encoded in the surface's format: built on the worker thread.
fn encode_band(surface: &Surface, band: Rect, data: &[f32]) -> Surface {
    let mut s = Surface::with_default(surface.format(), &surface.default_pixel());
    for c in band.tiles() {
        if let Some(t) = surface.tile(c) {
            s.tile_mut(c).bytes_mut().copy_from_slice(t.bytes());
        } else {
            s.tile_mut(c);
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
    shape: Option<Rect>,
    fmt: PixelFormat,
    ov: i32,
    clamp_context: bool,
    params: Value,
}

#[cfg(test)]
mod band_layout_tests {
    use super::{band_layout, context_bands, context_bytes, context_rect, large_input_budget};
    use crate::Limits;
    use photocraft_geom::Rect;

    #[test]
    fn ordinary_bands_keep_existing_parallelism() {
        let layout = band_layout(&Limits::default(), 4, 64, 32).unwrap();
        assert_eq!((layout.across, layout.workers), (4, 32));
    }

    #[test]
    fn large_halos_shrink_bands_and_bound_concurrent_memory() {
        let limits = Limits { max_memory_bytes: 512 << 20, ..Limits::default() };
        // Explicit lower caller limits remain lower; an impossible symmetric tile fails.
        assert!(band_layout(&limits, 4, 1536, 64).is_err());
        let layout = band_layout(&Limits::default(), 4, 1500, 64).unwrap();
        assert!(layout.across >= 1);
        assert_eq!(layout.workers, 1);
        let layout = band_layout(&Limits::default(), 1, 1536, 64).unwrap();
        assert!(layout.across >= 1);
        assert_eq!(layout.workers, 1);
    }

    #[test]
    fn impossible_bands_fail_before_allocating() {
        let limits = Limits { max_memory_bytes: 64 << 20, ..Limits::default() };
        assert!(band_layout(&limits, 4, 1536, 64).is_err());
        assert!(band_layout(&Limits::default(), usize::MAX, 1536, 4).is_err());
        assert!(band_layout(&Limits::default(), 4, usize::MAX, 4).is_err());
        assert!(band_layout(&Limits::default(), 0, 1536, 4).is_err());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn large_clipped_blocks_cover_24mp_without_repeated_small_tile_halos() {
        let area = Rect::new(0, 0, 6000, 4000);
        let limits = Limits::default();
        let bands = context_bands(area, area, 1536, 4, &limits, true).unwrap();
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert_eq!(bands.iter().map(|b| u64::from(b.width()) * u64::from(b.height())).sum::<u64>(), 24_000_000);
        for (i, band) in bands.iter().enumerate() {
            assert!(area.contains_rect(band));
            assert!(band.height() > 256);
            assert!(context_bytes(context_rect(*band, area, 1536, true), 4).unwrap() <= large_input_budget(&limits));
            assert!(bands.iter().skip(i + 1).all(|other| band.intersect(other).is_empty()));
        }
    }

    #[test]
    fn clipped_planning_respects_custom_caps_and_extreme_bounds() {
        let area = Rect::new(0, 0, 6000, 4000);
        let limits = Limits { max_memory_bytes: 64 << 20, ..Limits::default() };
        assert!(context_bands(area, area, 1536, 4, &limits, true).is_err());
        assert!(context_bands(area, area, 1536, usize::MAX, &Limits::default(), true).is_err());
        assert!(context_bands(area, area, -1, 4, &Limits::default(), true).is_err());
        let tiny = Rect::new(i32::MAX - 1, i32::MIN, i32::MAX, i32::MIN + 1);
        assert_eq!(context_bands(tiny, tiny, 1536, 4, &Limits::default(), true).unwrap(), [tiny]);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn large_unclipped_blocks_group_tiles_but_keep_complete_halos() {
        let area = Rect::new(0, 0, 6000, 4000);
        let limits = Limits::default();
        let bands = context_bands(area, area, 1536, 4, &limits, false).unwrap();
        assert!(bands.len() < 32, "{bands:?}");
        assert_eq!(bands.iter().map(|b| u64::from(b.width()) * u64::from(b.height())).sum::<u64>(), 24_000_000);
        for band in bands {
            let input = context_rect(band, area, 1536, false);
            assert_eq!(input, band.inflate(1536));
            assert!(band.height() > 256);
            assert!(context_bytes(input, 4).unwrap() <= large_input_budget(&limits));
        }
    }
}
