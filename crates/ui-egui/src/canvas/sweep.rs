//! Gradual redraws of a large GPU canvas. Where the canvas would recomposite a large area in one
//! frame (the whole document for a change nothing bounded, the first draw of a document, or a
//! change that bounded but large, like committing a transform of a big layer), it redraws it in
//! blocks instead, a frame's budget at a time, the blocks in view first. On a gigapixel canvas the one-frame redraw took over
//! a second on the UI thread, long enough for a Wayland compositor to drop the window; in blocks
//! the window keeps answering, and until a block is redrawn it shows what it showed before.
//!
//! A sweep is also the safety net behind [`super::doc_diff`]: after a change whose area was worked
//! out by comparing documents, a sweep redraws everything, so if that comparison ever missed a
//! pixel, it shows for a moment at most, never for good.
//!
//! Only on the GPU path, only with background jobs (the desktop app), and only for canvases of at
//! least [`SWEEP_MIN_PX`]; elsewhere the canvas redraws in one go as before. A block that doesn't
//! come back as a plain GPU refresh (the CPU compositor stepping in, the texture replaced) ends
//! the sweep, and the document redraws in one go until the next full refresh succeeds.

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::{DocId, Document};
use photocraft_engine::display_color::CanvasDisplay;
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::gpu_canvas::GpuCanvas;

/// Canvases this large (64 MP: 8192 × 8192) redraw in blocks; smaller ones are quick in one go.
pub(crate) const SWEEP_MIN_PX: u64 = 64 << 20;

/// On such a canvas, a redraw of at least this area goes in blocks too (16 MP, a few tenths of
/// a second in one frame on the gigapixel canvas this was measured on).
pub(crate) const SWEEP_REGION_PX: u64 = 16 << 20;

/// A block in document pixels: 2 MP, whole render tiles, a few milliseconds each, and under
/// [`crate::gpu_canvas::DEFER_PX`], so the compositor treats each as a small refresh.
const BLOCK_W: i32 = 2048;
const BLOCK_H: i32 = 1024;

/// UI-thread time a frame spends on blocks (at least one block is drawn per frame).
const FRAME_BUDGET_MS: f64 = 24.0;

/// Per document: the document its GPU canvas last showed (what [`super::doc_diff`] compares
/// against) and any sweep in progress.
#[derive(Default)]
pub(crate) struct CanvasState {
    pub(crate) shown: Option<(Arc<Document>, u64)>,
    pending: Vec<Rect>,
    /// A block came back other than as a GPU refresh: redraw in one go until a full one lands.
    failed: bool,
}

pub(crate) type CanvasStates = HashMap<DocId, CanvasState>;

/// The outcome of drawing a frame's blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Blocks remain.
    Continuing,
    /// The canvas is all redrawn.
    Done,
    /// A block didn't redraw on the GPU: the sweep is dropped; redraw in one go.
    Failed,
}

/// Whether `doc`'s canvas redraws everything in blocks rather than in one frame.
pub(crate) fn eligible(app: &PhotocraftApp, doc: &Document) -> bool {
    app.background_jobs && doc.bounds().size().area() >= SWEEP_MIN_PX && !app.canvas_states.get(&doc.id).is_some_and(|s| s.failed)
}

/// Whether a sweep of `doc` still has blocks to draw.
pub(crate) fn running(app: &PhotocraftApp, doc: DocId) -> bool {
    app.canvas_states.get(&doc).is_some_and(|s| !s.pending.is_empty())
}

/// (Re)start a sweep of all of `doc`'s canvas, the blocks nearest `visible` first.
pub(crate) fn start(app: &mut PhotocraftApp, doc: &Document, visible: Rect) {
    let state = app.canvas_states.entry(doc.id).or_default();
    state.pending = blocks(doc.bounds(), visible);
}

/// Add `region` of `doc`'s canvas to its sweep, drawn before what was already pending.
pub(crate) fn add(app: &mut PhotocraftApp, doc: &Document, region: Rect, visible: Rect) {
    let region = region.intersect(&doc.bounds());
    if region.is_empty() {
        return;
    }
    let state = app.canvas_states.entry(doc.id).or_default();
    // Popped from the end: the new blocks first. A block pending twice is drawn twice, which
    // is only wasted time, never a wrong pixel.
    state.pending.extend(blocks(region, visible));
}

/// Forget a sweep that failed, after a full refresh landed.
pub(crate) fn full_refresh_landed(app: &mut PhotocraftApp, doc: DocId) {
    if let Some(s) = app.canvas_states.get_mut(&doc) {
        s.failed = false;
        s.pending.clear();
    }
}

/// Remember what the canvas of `doc` shows now (with display key `display_key`).
pub(crate) fn shown(app: &mut PhotocraftApp, doc: &Arc<Document>, display_key: u64) {
    app.canvas_states.entry(doc.id).or_default().shown = Some((doc.clone(), display_key));
}

/// The area where `doc` differs from what its canvas last showed, if that can be bounded.
pub(crate) fn changed_since_shown(app: &PhotocraftApp, doc: &Document, display_key: u64) -> Option<Rect> {
    let (prev, key) = app.canvas_states.get(&doc.id)?.shown.as_ref()?;
    if *key != display_key {
        return None;
    }
    super::doc_diff::changed_area(prev, doc)
}

/// Draw blocks of `doc`'s sweep until this frame's budget is spent, and ask for another frame
/// while blocks remain.
pub(crate) fn step(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, doc: &Document, display: Option<&CanvasDisplay>) -> Step {
    let started = crate::gpu_canvas::now_ms();
    loop {
        let Some(block) = app.canvas_states.get_mut(&doc.id).and_then(|s| s.pending.pop()) else { return Step::Done };
        let r = gpu.refresh_with(doc.id.0, doc, Some(block), display, false);
        match (r.kind, &r.fallback) {
            ("gpu-rect" | "rect", None) => {}
            // The texture had to be made again: that refresh drew everything.
            ("gpu-full", None) => {
                if let Some(s) = app.canvas_states.get_mut(&doc.id) {
                    s.pending.clear();
                }
                return Step::Done;
            }
            _ => {
                log::info!("canvas: redrawing in blocks stopped ({}); redrawing in one go", r.kind);
                if let Some(s) = app.canvas_states.get_mut(&doc.id) {
                    s.pending.clear();
                    s.failed = true;
                }
                return Step::Failed;
            }
        }
        if !running(app, doc.id) {
            return Step::Done;
        }
        if crate::gpu_canvas::now_ms() - started >= FRAME_BUDGET_MS {
            ctx.request_repaint();
            return Step::Continuing;
        }
    }
}

/// Blocks covering `bounds`, in drawing order from the end (they are popped): the blocks the view
/// shows, nearest its center first, then the rest by distance.
fn blocks(bounds: Rect, visible: Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let mut x = bounds.x0;
        while x < bounds.x1 {
            out.push(Rect::new(x, y, (x + BLOCK_W).min(bounds.x1), (y + BLOCK_H).min(bounds.y1)));
            x += BLOCK_W;
        }
        y += BLOCK_H;
    }
    let (cx, cy) = (i64::from(visible.x0 / 2 + visible.x1 / 2), i64::from(visible.y0 / 2 + visible.y1 / 2));
    let key = |b: &Rect| {
        let (bx, by) = (i64::from(b.x0 / 2 + b.x1 / 2), i64::from(b.y0 / 2 + b.y1 / 2));
        (!b.intersect(&visible).is_empty(), -((bx - cx).pow(2) + (by - cy).pow(2)))
    };
    // Ascending, so the last (drawn first) is in view and nearest the center.
    out.sort_by_key(key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_cover_the_canvas_once_and_draw_the_view_first() {
        let bounds = Rect::new(0, 0, 10_000, 5_000);
        let visible = Rect::new(6_000, 1_000, 8_000, 3_000);
        let b = blocks(bounds, visible);
        let area: u64 = b.iter().map(|r| r.size().area()).sum();
        assert_eq!(area, bounds.size().area(), "every pixel once");
        let first = *b.last().unwrap();
        assert!(!first.intersect(&visible).is_empty(), "the view's blocks come first: {first:?}");
        let last = b[0];
        assert!(last.intersect(&visible).is_empty(), "far blocks come last: {last:?}");
    }
}
