//! Sampling around an area to fill it from its surroundings: Content-Aware fill (Edit › Fill, the
//! Crop tool, the Content-Aware Fill workspace, the retouching tools) and the composite the
//! retouching tools sample.

use photocraft_algo::content_aware::FillOptions;
use photocraft_color::PixelFormat;
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::retouch::Region;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::pixel_fill::Source;
use crate::{EngineError, Result};

/// The selection filled from its surroundings (`photocraft_algo::content_aware`), as straight
/// RGBA over the sampling window. Only the selected pixels are used.
pub fn content_aware(surf: &Surface, sel: &Surface, canvas: Rect, options: &FillOptions) -> Result<Source> {
    let hole_bounds = sel.content_bounds().intersect(&canvas);
    if hole_bounds.is_empty() {
        return Err(EngineError::Other("the selection is outside the canvas".into()));
    }
    let is_hole = |x: i32, y: i32| sel.sample_channel(x, y, 0) > 0.0;
    let (window, filled) = content_aware_native_with_options(surf, hole_bounds, canvas, options, is_hole, |_, _| true)?;
    let fmt = surf.format();
    let px = filled.chunks_exact(fmt.channels().max(1)).map(|c| to_rgba(&fmt, c)).collect();
    Ok(Source::Pixels(window, px))
}

/// The pixels `is_hole` picks inside `hole_bounds` filled from their surroundings
/// (`photocraft_algo::content_aware`): Photoshop's sampling window around `hole_bounds`, sampling
/// only the non-hole pixels `usable` allows. Returns the window and its pixels in the surface's
/// own channels (the hole filled, the rest as read). Shared by Edit › Fill › Content-Aware and the
/// Crop tool's Content-Aware fill (#3823).
pub fn content_aware_native(
    surf: &Surface,
    hole_bounds: Rect,
    canvas: Rect,
    color_adaptation: bool,
    seed: u64,
    is_hole: impl Fn(i32, i32) -> bool,
    usable: impl Fn(i32, i32) -> bool,
) -> Result<(Rect, Vec<f32>)> {
    content_aware_native_with_options(surf, hole_bounds, canvas, &content_aware_options(color_adaptation, seed), is_hole, usable)
}

/// The adaptation defaults shared by Fill and Content-Aware Crop.
pub fn content_aware_options(color_adaptation: bool, seed: u64) -> FillOptions {
    FillOptions { color_adaptation: photocraft_algo::content_aware::color_level(if color_adaptation { "default" } else { "none" }), seed, ..Default::default() }
}

/// Fill the bounded sampling window with native channel values and the requested adaptations.
fn content_aware_native_with_options(
    surf: &Surface,
    hole_bounds: Rect,
    canvas: Rect,
    options: &FillOptions,
    is_hole: impl Fn(i32, i32) -> bool,
    usable: impl Fn(i32, i32) -> bool,
) -> Result<(Rect, Vec<f32>)> {
    use photocraft_algo::content_aware::fill;
    let window = sampling_window(hole_bounds, canvas);
    let n = surf.format().channels();
    let (w, h) = window_size(window)?;
    let img = crate::allocation::read_region(surf, window, "Content-Aware fill samples")?;
    let len = w.checked_mul(h).ok_or_else(|| EngineError::Other("the area is too large for Content-Aware fill".into()))?;
    let mut hole = crate::allocation::filled(len, false, "Content-Aware fill")?;
    let mut source = crate::allocation::filled(len, false, "Content-Aware fill")?;
    for (i, (hv, sv)) in hole.iter_mut().zip(source.iter_mut()).enumerate() {
        let (x, y) = (window.x0 + (i % w) as i32, window.y0 + (i / w) as i32);
        *hv = is_hole(x, y);
        *sv = !*hv && usable(x, y);
    }
    if !source.iter().any(|s| *s) {
        return Err(EngineError::Other("Content-Aware fill needs pixels outside the area to fill to sample from".into()));
    }
    if img.len() != len * n {
        return Err(EngineError::Other("could not read the layer pixels".into()));
    }
    Ok((window, fill(w, h, n, &img, &hole, &source, options)))
}

/// Photoshop's Content-Aware sampling window around the selection bounds `hole`
/// ([`photocraft_algo::content_aware::sampling_window`]): Edit › Fill, the workspace's Rectangular
/// and Auto areas and the retouching tools all copy from within it. Worked out in i64, so a loaded
/// document declaring an absurd selection can't overflow it (#963).
pub fn sampling_window(hole: Rect, canvas: Rect) -> Rect {
    photocraft_algo::content_aware::sampling_window(hole.intersect(&canvas), canvas)
}

/// The size of a Content-Aware sampling window, or an error for a window too large to read and
/// fill (#963). Same cap as other whole-image operations: refuse absurd windows instead of
/// allocating.
pub fn window_size(window: Rect) -> Result<(usize, usize)> {
    let (w, h) = (window.width(), window.height());
    if u64::from(w) * u64::from(h) > 400_000_000 {
        return Err(EngineError::Other("the selection is too large for Content-Aware fill".into()));
    }
    Ok((w as usize, h as usize))
}

/// [`Surface::read_region`] of `r`, read in parallel bands and asking `cancelled` before each
/// (`None` once it says so): a Content-Aware window can be tens of megapixels.
pub fn read_window(surf: &Surface, r: Rect, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    use rayon::prelude::*;
    const BAND: usize = 64;
    let n = surf.channels();
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut out = vec![0.0f32; w * h * n];
    if out.is_empty() {
        return Some(out);
    }
    // Each worker reads its bands through one buffer of its own.
    out.par_chunks_mut(BAND * w * n).enumerate().for_each_init(Vec::new, |buf, (i, chunk)| {
        if cancelled() {
            return;
        }
        let y0 = r.y0 + (i * BAND) as i32;
        let band = Rect::new(r.x0, y0, r.x1, y0 + (chunk.len() / (w * n)) as i32);
        surf.read_region_into(band, buf);
        if buf.len() == chunk.len() {
            chunk.copy_from_slice(buf);
        }
    });
    if cancelled() { None } else { Some(out) }
}

/// Channel 0 of [`read_window`] (a selection's or an alpha channel's coverage).
pub fn read_window_mask(surf: &Surface, r: Rect, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    use rayon::prelude::*;
    let n = surf.channels().max(1);
    let all = read_window(surf, r, cancelled)?;
    if n == 1 {
        return Some(all);
    }
    Some(all.par_chunks_exact(n).map(|px| px[0]).collect())
}

/// Which pixels the retouching tools (Clone Stamp, Healing Brush, Remove, Content-Aware Move) sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleLayers {
    Current,
    CurrentAndBelow,
    All,
}

/// Composite pixels of `rect` in `fmt` (native channels): the whole document, or the target layer
/// and everything below it.
pub fn composite_region(pre: &Document, id: Option<LayerId>, which: SampleLayers, rect: Rect, fmt: PixelFormat) -> Region {
    let mut doc = pre.clone();
    if which == SampleLayers::CurrentAndBelow
        && let Some(id) = id
    {
        // Hide everything composited after (above) the target in the bottom-to-top walk.
        let walk: Vec<(Vec<usize>, LayerId)> = pre.walk().into_iter().map(|(p, _, l)| (p, l.id)).collect();
        if let Some(pos) = walk.iter().position(|(_, l)| *l == id) {
            for (path, _) in &walk[pos + 1..] {
                if let Some(l) = doc.layer_at_mut(path) {
                    l.visible = false;
                }
            }
        }
    }
    let buf = photocraft_compose::render(&doc, rect);
    let n = fmt.channels();
    let mut out = Region::new(rect, n);
    let mut enc = [0.0f32; 8];
    for (i, px) in buf.px.iter().enumerate() {
        from_rgba_into(&fmt, *px, &mut enc);
        out.data[i * n..(i + 1) * n].copy_from_slice(&enc[..n]);
    }
    out
}
