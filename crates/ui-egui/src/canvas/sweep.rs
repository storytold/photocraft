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
    state.pending = blocks(doc.bounds(), doc.bounds(), visible);
}

/// Add `region` of `doc`'s canvas to its sweep, drawn before what was already pending.
pub(crate) fn add(app: &mut PhotocraftApp, doc: &Document, region: Rect, visible: Rect) {
    let region = region.intersect(&doc.bounds());
    if region.is_empty() {
        return;
    }
    let state = app.canvas_states.entry(doc.id).or_default();
    // Blocks sit on one grid over the canvas, so a block already pending is the same rectangle:
    // it moves to the front instead of being queued twice (a Move drag adds the same blocks
    // every frame). Popped from the end: the new blocks first.
    let new = blocks(region, doc.bounds(), visible);
    let queued: std::collections::HashSet<(i32, i32)> = new.iter().map(|b| (b.x0, b.y0)).collect();
    state.pending.retain(|b| !queued.contains(&(b.x0, b.y0)));
    state.pending.extend(new);
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
        // Counted like any refresh, so `ui.inspect` and slow-frame reports include the blocks.
        if r.fallback.is_none() && matches!(r.kind, "gpu-rect" | "gpu-full" | "rect") {
            app.perf.record_refresh(&r);
            if r.kind.starts_with("gpu") {
                app.perf.gpu_uploads = r.uploads;
            }
        }
        if let Some(end) = after_block(app, doc.id, r.kind, r.fallback.is_some()) {
            return end;
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

/// What a block's refresh (`kind`, and whether the CPU stepped in) means for the sweep: `None`,
/// carry on; `Done`, that refresh drew everything (the texture had to be made again); `Failed`,
/// it didn't redraw on the GPU, so the sweep is dropped and the document redraws in one go until
/// a full refresh lands ([`full_refresh_landed`]).
fn after_block(app: &mut PhotocraftApp, doc: DocId, kind: &str, fallback: bool) -> Option<Step> {
    match (kind, fallback) {
        ("gpu-rect" | "rect", false) => None,
        ("gpu-full", false) => {
            if let Some(s) = app.canvas_states.get_mut(&doc) {
                s.pending.clear();
            }
            Some(Step::Done)
        }
        _ => {
            log::info!("canvas: redrawing in blocks stopped ({kind}); redrawing in one go");
            if let Some(s) = app.canvas_states.get_mut(&doc) {
                s.pending.clear();
                s.failed = true;
            }
            Some(Step::Failed)
        }
    }
}

/// The blocks of a grid over `canvas` that cover `region`, in drawing order from the end (they
/// are popped): the blocks the view shows, nearest its center first, then the rest by distance.
/// On one grid, the same block of two regions is the same rectangle.
fn blocks(region: Rect, canvas: Rect, visible: Rect) -> Vec<Rect> {
    let region = region.intersect(&canvas);
    let mut out = Vec::new();
    if region.is_empty() {
        return out;
    }
    let snap = |v: i32, origin: i32, step: i32| origin + (v - origin).div_euclid(step) * step;
    let mut y = snap(region.y0, canvas.y0, BLOCK_H);
    while y < region.y1 {
        let mut x = snap(region.x0, canvas.x0, BLOCK_W);
        while x < region.x1 {
            out.push(Rect::new(x, y, (x + BLOCK_W).min(canvas.x1), (y + BLOCK_H).min(canvas.y1)));
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
        let b = blocks(bounds, bounds, visible);
        let area: u64 = b.iter().map(|r| r.size().area()).sum();
        assert_eq!(area, bounds.size().area(), "every pixel once");
        let first = *b.last().unwrap();
        assert!(!first.intersect(&visible).is_empty(), "the view's blocks come first: {first:?}");
        let last = b[0];
        assert!(last.intersect(&visible).is_empty(), "far blocks come last: {last:?}");
    }

    #[test]
    fn blocks_of_overlapping_regions_are_the_same_rectangles() {
        let canvas = Rect::new(0, 0, 10_000, 5_000);
        let a = blocks(Rect::new(100, 100, 3_000, 1_500), canvas, canvas);
        let b = blocks(Rect::new(200, 50, 3_100, 1_400), canvas, canvas);
        let set = |v: &[Rect]| v.iter().map(|r| (r.x0, r.y0, r.x1, r.y1)).collect::<std::collections::HashSet<_>>();
        assert_eq!(set(&a), set(&b), "both regions fall on the same grid blocks");
        for r in &a {
            assert!(r.x0 % BLOCK_W == 0 && r.y0 % BLOCK_H == 0, "on the grid: {r:?}");
        }
        let covered = a.iter().fold(Rect::EMPTY, |u, r| if u.is_empty() { *r } else { u.union(r) });
        assert!(covered.contains(100, 100) && covered.contains(2_999, 1_499), "the region is covered: {covered:?}");
    }

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app
    }

    fn doc(w: u32, h: u32) -> Arc<Document> {
        Arc::new(Document::new("t", photocraft_geom::Size::new(w, h), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8))
    }

    #[test]
    fn only_large_canvases_with_background_jobs_and_no_failed_sweep_are_eligible() {
        let mut app = app();
        let big = doc(8192, 8192);
        assert!(eligible(&app, &big), "64 MP with background jobs");
        assert!(!eligible(&app, &doc(8192, 8191)), "under SWEEP_MIN_PX");
        app.background_jobs = false;
        assert!(!eligible(&app, &big), "no background jobs (web, tests)");
        app.background_jobs = true;
        app.canvas_states.entry(big.id).or_default().failed = true;
        assert!(!eligible(&app, &big), "a failed sweep");
    }

    #[test]
    fn a_block_off_the_gpu_fails_the_sweep_until_a_full_refresh_lands() {
        let mut app = app();
        let big = doc(8192, 8192);
        start(&mut app, &big, big.bounds());
        assert_eq!(after_block(&mut app, big.id, "gpu-rect", false), None, "a GPU block carries on");
        assert_eq!(after_block(&mut app, big.id, "rect", false), None, "nothing to draw carries on");
        assert_eq!(after_block(&mut app, big.id, "rect", true), Some(Step::Failed), "the CPU stepped in");
        assert!(!running(&app, big.id), "the rest of the sweep is dropped");
        assert!(!eligible(&app, &big), "and the document redraws in one go");
        start(&mut app, &big, big.bounds());
        full_refresh_landed(&mut app, big.id);
        assert!(eligible(&app, &big) && !running(&app, big.id), "a full refresh clears it");
        start(&mut app, &big, big.bounds());
        assert_eq!(after_block(&mut app, big.id, "gpu-full", false), Some(Step::Done), "a texture made again drew everything");
        assert!(!running(&app, big.id));
    }

    #[test]
    fn adding_a_region_twice_queues_its_blocks_once_and_first() {
        let mut app = app();
        let big = doc(10_000, 10_000);
        let region = Rect::new(100, 100, 5_000, 3_000);
        add(&mut app, &big, Rect::new(6_000, 6_000, 9_000, 9_000), big.bounds());
        add(&mut app, &big, region, big.bounds());
        let once = app.canvas_states[&big.id].pending.clone();
        add(&mut app, &big, region, big.bounds());
        let twice = &app.canvas_states[&big.id].pending;
        assert_eq!(twice.len(), once.len(), "no duplicates");
        let set = |v: &[Rect]| v.iter().map(|r| (r.x0, r.y0)).collect::<std::collections::HashSet<_>>();
        assert_eq!(set(twice), set(&once));
        let last = *twice.last().unwrap();
        assert!(!last.intersect(&region).is_empty(), "the region is drawn first: {last:?}");
    }

    #[test]
    fn a_comparison_needs_the_same_display() {
        let mut app = app();
        let big = doc(8192, 8192);
        shown(&mut app, &big, 1);
        assert_eq!(changed_since_shown(&app, &big, 2), None, "another display key: nothing to compare");
        assert_eq!(changed_since_shown(&app, &doc(8192, 8192), 1), None, "another document: nothing shown");
    }
}
