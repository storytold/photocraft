//! A full GPU refresh of a very large canvas, a slice per frame. Compositing a 1.1 GP storyboard
//! in one frame took 1.0–1.9 s (on opening it, on switching back to its tab, after moving layers
//! or undoing), and the desktop's compositor closes a window that stops answering for about
//! 1.5–2 s: the app was closed right after opening the file. From [`SLICE_FROM_PX`] up, the canvas
//! is composited in bands of about [`BAND_PX`], the bands the view shows first, spending about
//! [`FRAME_MS`] a frame; the rest follow on the next frames while the window keeps answering.

use std::collections::VecDeque;

use photocraft_doc::DocId;
use photocraft_geom::Rect;

use super::{Document, PhotocraftApp};
use crate::gpu_canvas::GpuCanvas;

/// Canvases of this many pixels or more refresh in slices (about 134 MP).
pub(crate) const SLICE_FROM_PX: u64 = 1 << 27;
/// About this many pixels a band (about 16.8 MP).
const BAND_PX: u64 = 1 << 24;
/// Bands are composited until a frame has spent this long on them (at least one band a frame).
const FRAME_MS: f64 = 120.0;

/// The bands of `bounds`, full rows of about [`BAND_PX`] each: those meeting `visible` first, the
/// nearest to its middle first, then the rest by their distance from it.
pub(crate) fn bands(bounds: Rect, visible: Rect) -> VecDeque<Rect> {
    if bounds.is_empty() {
        return VecDeque::new();
    }
    let rows = (BAND_PX / u64::from(bounds.width())).clamp(1, u64::from(bounds.height())) as i32;
    let mut out = Vec::new();
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let y1 = y.saturating_add(rows).min(bounds.y1);
        out.push(Rect::new(bounds.x0, y, bounds.x1, y1));
        y = y1;
    }
    let middle = i64::from(visible.y0) + i64::from(visible.y1);
    out.sort_by_key(|b| (b.intersect(&visible).is_empty(), (i64::from(b.y0) + i64::from(b.y1) - middle).abs()));
    out.into()
}

/// Starts refreshing all of `doc` in slices, in place of one full refresh, and draws the first
/// frame's bands ([`step`]'s result). `None` (nothing drawn) when the canvas is small enough to
/// refresh at once, or the wgpu compositor doesn't draw it (still being built, say: the CPU
/// compositor then redraws in the background), or its texture can't be made: the ordinary refresh
/// decides. A refresh in slices still running for document `id` starts over.
pub(crate) fn start(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    (id, doc): (DocId, &Document),
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    visible: Rect,
) -> Option<bool> {
    app.gpu_slices.remove(&id);
    let px = u64::from(doc.size.width) * u64::from(doc.size.height);
    if px < SLICE_FROM_PX || doc.id != id || gpu.compositor_building() || !gpu.supports(doc) || gpu.prepare_texture(id.0, doc).is_none() {
        return None;
    }
    app.gpu_slices.insert(id, bands(doc.bounds(), visible.intersect(&doc.bounds())));
    Some(step(app, ctx, gpu, (id, doc), display))
}

/// Whether document `id` still has bands to draw.
pub(crate) fn pending(app: &PhotocraftApp, id: DocId) -> bool {
    app.gpu_slices.get(&id).is_some_and(|q| !q.is_empty())
}

/// Draws the next bands of document `id`'s refresh in slices, as `doc` is now, for about
/// [`FRAME_MS`] and asks for another frame while some are left. False when the GPU canvas can't
/// draw them (lost, or over its budget): the refresh in slices ends and this frame goes to the
/// CPU canvas.
pub(crate) fn step(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    (id, doc): (DocId, &Document),
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
) -> bool {
    let start = crate::gpu_canvas::now_ms();
    while let Some(band) = app.gpu_slices.get_mut(&id).and_then(VecDeque::pop_front) {
        let r = gpu.refresh(id.0, doc, Some(band), display);
        if r.kind == "lost" || r.kind == crate::gpu_canvas::REFRESH_OVER_BUDGET {
            app.gpu_slices.remove(&id);
            if let Some(reason) = r.fallback {
                crate::render_path::note(app, id.0, crate::render_path::Path::Cpu(reason));
            }
            return false;
        }
        app.perf.record_refresh(&r);
        if crate::gpu_canvas::now_ms() - start >= FRAME_MS {
            break;
        }
    }
    if pending(app, id) {
        ctx.request_repaint();
    } else {
        app.gpu_slices.remove(&id);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_the_canvas_once_the_visible_ones_first() {
        let bounds = Rect::new(0, 0, 40_000, 30_000);
        let visible = Rect::new(10_000, 20_000, 12_000, 21_000);
        let bands = bands(bounds, visible);
        let rows: u64 = bands.iter().map(|b| u64::from(b.height())).sum();
        assert_eq!(rows, 30_000, "every row once");
        assert!(bands.iter().all(|b| b.x0 == 0 && b.x1 == 40_000 && u64::from(b.width()) * u64::from(b.height()) <= BAND_PX));
        assert!(!bands[0].intersect(&visible).is_empty(), "the view's band first: {:?}", bands[0]);
        let first_hidden = bands.iter().position(|b| b.intersect(&visible).is_empty()).unwrap();
        assert!(bands.iter().skip(first_hidden).all(|b| b.intersect(&visible).is_empty()), "then the rest");
    }

    #[test]
    fn a_narrow_tall_or_empty_canvas_still_bands() {
        assert!(bands(Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0)).is_empty());
        let one = bands(Rect::new(0, 0, 100, 100), Rect::new(0, 0, 100, 100));
        assert_eq!(Vec::from(one), [Rect::new(0, 0, 100, 100)]);
        // Wider than a band: one row at a time.
        let wide = bands(Rect::new(0, 0, 20_000_000, 3), Rect::new(0, 0, 10, 1));
        assert_eq!(wide.len(), 3);
    }
}
