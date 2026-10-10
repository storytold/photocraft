//! Destructive pixel operations on layers (Image → Adjustments, flips, fills).

use photocraft_compose::{Buffer, adjust};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, to_rgba};

/// Composite `layers` inside their visible composite bounds (effects included) and write the result directly into a
/// tiled surface. Large merges used to hold both a full-canvas RGBA buffer and a second converted
/// float buffer at once; banded rendering keeps only one compositor band plus the destination.
pub fn composite_layers(solo: &Document, format: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> Surface {
    let canvas = solo.bounds();
    let area = solo
        .layers
        .iter()
        .filter(|layer| layer.visible)
        // Composite bounds include layer effects (a drop shadow reaches past the pixels); `None`
        // means the layer may draw anywhere.
        .map(|layer| photocraft_compose::composite_bounds(layer, canvas).unwrap_or(canvas))
        .filter(|bounds| !bounds.is_empty())
        .reduce(|a, b| a.union(&b))
        .unwrap_or(Rect::EMPTY)
        .intersect(&canvas);
    let mut surface = Surface::new(format);
    if area.is_empty() {
        return surface;
    }

    let channels = format.channels();
    let mut data = Vec::new();
    let _ = photocraft_compose::render_bands(solo, area, 0, |mut band| -> Result<(), ()> {
        if let Some(color) = background {
            for pixel in &mut band.px {
                let alpha = pixel[3];
                for channel in 0..3 {
                    pixel[channel] = pixel[channel] * alpha + color[channel] * (1.0 - alpha);
                }
                pixel[3] = 1.0;
            }
        }
        data.resize(band.px.len() * channels, 0.0);
        for (pixel, encoded) in band.px.iter().zip(data.chunks_exact_mut(channels)) {
            photocraft_raster::from_rgba_into(&format, *pixel, encoded);
        }
        surface.write_region(band.rect, &data);
        Ok(())
    });
    surface.prune();
    surface
}

/// Apply an adjustment destructively to a surface, weighted by an optional selection.
/// Applies `adj` to a surface (any colour model and depth, via straight RGBA) through the
/// selection; `mode` is the document's, for the tone transfer (e.g. Exposure in Grayscale).
/// Works tile by tile in parallel, so a large layer needs one tile's buffers per thread rather
/// than four full-layer copies, and the result is the same as one pass over the whole region.
pub fn adjust_surface(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) {
    use rayon::prelude::*;
    let r = s.content_bounds();
    if r.is_empty() {
        return;
    }
    let fmt = s.format();
    let done: Vec<_> = s
        .take_tiles(r)
        .into_par_iter()
        .flat_map_iter(|(tc, tile)| {
            // A one-tile surface, so `adjusted` reads and writes this tile only.
            let mut one = Surface::new(fmt);
            one.put_tiles([(tc, tile)]);
            let tr = tc.rect().intersect(&r);
            let out = adjusted(&one, tr, adj, selection, mode);
            one.write_region(tr, &out);
            one.take_tiles(tc.rect())
        })
        .collect();
    s.put_tiles(done);
}

#[cfg(test)]
#[path = "pixels_tests.rs"]
mod tests;

/// Applies `adj` to a layer mask through the selection. Untouched mask pixels count (they read as
/// the mask's default value), and without a selection the default changes too, since a mask
/// reaches past the canvas: Invert turns a reveal-all mask into a hide-all one (#780).
pub fn adjust_mask(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>) {
    let mode = photocraft_color::ColorMode::Grayscale;
    if let Some(sel) = selection {
        let r = sel.content_bounds();
        if !r.is_empty() {
            let out = adjusted(s, r, adj, Some(sel), mode);
            s.write_region(r, &out);
            s.prune();
        }
        return;
    }
    let fmt = s.format();
    let unit = Rect::from_xywh(0, 0, 1, 1);
    let mut out = Surface::with_default(fmt, &adjusted(&Surface::with_default(fmt, &s.default_pixel()), unit, adj, None, mode));
    let r = s.tile_bounds();
    if !r.is_empty() {
        out.write_region(r, &adjusted(s, r, adj, None, mode));
    }
    out.prune();
    *s = out;
}

/// `s`'s pixels over `r` (untouched ones included) with `adj` applied through the selection, encoded.
fn adjusted(s: &Surface, r: Rect, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) -> Vec<f32> {
    let fmt = s.format();
    let n = fmt.channels();
    let raw = s.read_region(r);
    let mut buf = Buffer { rect: r, px: raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect() };
    let orig = buf.clone();
    // 32-bit documents get the float behaviour of an adjustment layer there (Levels doesn't clip);
    // integer depths keep the unrounded, clipped curves.
    let depth = (fmt.sample == photocraft_color::SampleType::F32).then_some(fmt.sample);
    adjust::apply_depth(adj, &mut buf, adjust::Transfer::for_document(mode, fmt.sample), depth);
    let w = r.width() as usize;
    let mut out = Vec::with_capacity(raw.len());
    for (i, (a, o)) in buf.px.iter().zip(&orig.px).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(r.x0 + (i % w) as i32, r.y0 + (i / w) as i32, 0));
        let mixed: [f32; 4] = std::array::from_fn(|c| o[c] + (a[c] - o[c]) * k);
        let mut enc = [0.0f32; 8];
        let m = photocraft_raster::from_rgba_into(&fmt, mixed, &mut enc);
        out.extend_from_slice(&enc[..m]);
    }
    out
}

/// Fill (respecting selection coverage) with a straight RGBA colour.
pub(crate) fn try_fill_surface(s: &mut Surface, area: Rect, color: [f32; 4], selection: Option<&Surface>, lock_transparency: bool) -> crate::Result<()> {
    let fmt = s.format();
    let n = fmt.channels();
    let mut region = crate::allocation::read_region(s, area, "filling pixels")?;
    fill_region(&mut region, fmt, n, area, color, selection, lock_transparency);
    s.try_write_region(area, &region)
        .map_err(|e| crate::EngineError::Other(format!("not enough memory while writing filled pixels ({e}); the document was not changed")))
}

fn fill_region(
    region: &mut [f32],
    fmt: photocraft_color::PixelFormat,
    n: usize,
    area: Rect,
    color: [f32; 4],
    selection: Option<&Surface>,
    lock_transparency: bool,
) {
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let x = area.x0 + (i % w) as i32;
        let y = area.y0 + (i / w) as i32;
        let k = selection.map_or(1.0, |sel| sel.sample_channel(x, y, 0));
        if k <= 0.0 {
            continue;
        }
        let d = to_rgba(&fmt, px);
        let sa = color[3] * k;
        let oa = sa + d[3] * (1.0 - sa);
        let mut o = [0.0f32; 4];
        if oa > 0.0 {
            for c in 0..3 {
                o[c] = (color[c] * sa + d[c] * d[3] * (1.0 - sa)) / oa;
            }
        }
        o[3] = if lock_transparency { d[3] } else { oa };
        let mut enc = [0.0f32; 8];
        photocraft_raster::from_rgba_into(&fmt, o, &mut enc);
        px.copy_from_slice(&enc[..n]);
    }
}

/// Clear pixels (make transparent) within the selection.
pub(crate) fn try_clear_surface(s: &mut Surface, area: Rect, selection: Option<&Surface>) -> crate::Result<()> {
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha {
        return Ok(());
    }
    let mut region = crate::allocation::read_region(s, area, "erasing pixels")?;
    clear_region(&mut region, n, area, selection);
    s.try_write_region(area, &region)
        .map_err(|e| crate::EngineError::Other(format!("not enough memory while erasing pixels ({e}); the document was not changed")))?;
    s.prune();
    Ok(())
}

fn clear_region(region: &mut [f32], n: usize, area: Rect, selection: Option<&Surface>) {
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0));
        px[n - 1] *= 1.0 - k;
        // Fully cleared pixels become the empty pixel, so bounds and tile pruning see them as gone.
        if px[n - 1] <= 0.0 {
            px.fill(0.0);
        }
    }
}

/// Remap a surface through a pixel-coordinate mapping (used for flips/rotations).
pub fn remap_surface(s: &Surface, map: impl Fn(i32, i32) -> (i32, i32)) -> Surface {
    let r = s.content_bounds();
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let n = s.channels();
    let raw = s.read_region(r);
    let w = r.width() as usize;
    for (i, px) in raw.chunks_exact(n).enumerate() {
        let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
        let (nx, ny) = map(x, y);
        out.write_pixel(nx, ny, px);
    }
    out
}

/// Merge `upper` onto `lower` producing a raster layer (Layer → Merge Down).
pub fn merge_down(doc_bounds: Rect, lower: &Layer, upper: &Layer, format: photocraft_color::PixelFormat) -> Layer {
    let stack = vec![lower.clone(), upper.clone()];
    let mut tmp = Document::new("merge", doc_bounds.size(), format.mode, format.sample);
    tmp.layers = stack;
    let s = composite_layers(&tmp, format, None);
    let mut merged = Layer::new(lower.name.clone(), LayerContent::Raster(s));
    merged.id = lower.id;
    merged.blend = lower.blend;
    merged.opacity = 1.0;
    // Merge Down replaces the lower layer, so keep its editing restrictions.
    // In particular, a merged Background must remain opaque and position-locked.
    merged.locks = lower.locks;
    merged
}
