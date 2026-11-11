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
//!
//! On the desktop the blocks are composited on the redraw thread ([`super::redraw_thread`]) while
//! the UI thread only hands them out and shows them as they land; a block the thread can't draw
//! comes back to the UI thread, which draws it as described above.

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
    /// UI-thread time of a render's planning, and of each block on top of it (ms, moving
    /// averages): how many blocks the rest of a frame's budget holds ([`batch_len`]).
    plan_ms: Option<f64>,
    block_ms: Option<f64>,
    /// Jobs out on the redraw thread: their tickets and blocks.
    in_flight: Vec<(u64, Vec<Rect>)>,
    /// A job failed on the redraw thread: this sweep goes on on the UI thread.
    thread_failed: bool,
    /// No jobs before this time ([`crate::gpu_canvas::now_ms`]): the compositor wasn't ready.
    thread_wait_until: f64,
}

/// How long the redraw thread waits for a compositor that isn't ready yet (ms).
const THREAD_RETRY_MS: f64 = 100.0;

/// Most blocks drawn in one render.
const MAX_BATCH: usize = 4;

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

/// Whether a sweep of `doc` still has blocks to draw (or out on the redraw thread).
pub(crate) fn running(app: &PhotocraftApp, doc: DocId) -> bool {
    app.canvas_states.get(&doc).is_some_and(|s| !s.pending.is_empty() || !s.in_flight.is_empty())
}

/// (Re)start a sweep of all of `doc`'s canvas, the blocks nearest `visible` first.
pub(crate) fn start(app: &mut PhotocraftApp, doc: &Document, visible: Rect) {
    let state = app.canvas_states.entry(doc.id).or_default();
    state.pending = blocks(doc.bounds(), doc.bounds(), visible);
    state.thread_failed = false;
}

/// The safety net behind a change bounded by comparing documents ([`super::doc_diff`]): redraw
/// everything in blocks. A sweep already running is that net for the blocks it hasn't drawn, and
/// restarting it would throw away the ones it has: each change (a transform's commit and the
/// selection update after it, say) restarted a whole-canvas redraw.
pub(crate) fn safety_net(app: &mut PhotocraftApp, doc: &Document, visible: Rect) {
    if !running(app, doc.id) {
        start(app, doc, visible);
    }
}

/// Add `region` of `doc`'s canvas to its sweep. Its blocks still waiting from before (what is
/// being redrawn was touched again) go first; its other blocks take turns with the rest of the
/// sweep, a job's worth of each at a time, so a change elsewhere (a storyboard hidden or shown)
/// doesn't hold up the rest of the canvas until it is drawn, nor the rest hold up the change.
pub(crate) fn add(app: &mut PhotocraftApp, doc: &Document, region: Rect, visible: Rect) {
    let region = region.intersect(&doc.bounds());
    if region.is_empty() {
        return;
    }
    let state = app.canvas_states.entry(doc.id).or_default();
    // A new redraw tries the redraw thread again, whatever happened to the last one.
    if state.pending.is_empty() && state.in_flight.is_empty() {
        state.thread_failed = false;
    }
    // Blocks sit on one grid over the canvas, so a block already pending is the same rectangle:
    // it is queued once (a Move drag adds the same blocks every frame).
    let new = blocks(region, doc.bounds(), visible);
    state.pending = queue_after_add(std::mem::take(&mut state.pending), new);
}

/// The sweep's queue (`pending`, popped from the end) once `new` (popped from the end too) joins
/// it, as [`add`] describes: `new`'s blocks already pending first, then its other blocks and the
/// rest of the queue in turns of [`JOB_BLOCKS`].
///
/// [`JOB_BLOCKS`]: super::redraw_thread::JOB_BLOCKS
fn queue_after_add(pending: Vec<Rect>, new: Vec<Rect>) -> Vec<Rect> {
    use super::redraw_thread::JOB_BLOCKS;
    let key = |b: &Rect| (b.x0, b.y0, b.x1, b.y1);
    let waiting: std::collections::HashSet<_> = pending.iter().map(key).collect();
    let queued: std::collections::HashSet<_> = new.iter().map(key).collect();
    // In the order they are drawn.
    let rest: Vec<Rect> = pending.into_iter().rev().filter(|b| !queued.contains(&key(b))).collect();
    let (again, fresh): (Vec<Rect>, Vec<Rect>) = new.into_iter().rev().partition(|b| waiting.contains(&key(b)));
    let mut order = again;
    let (mut a, mut b) = (fresh.chunks(JOB_BLOCKS), rest.chunks(JOB_BLOCKS));
    loop {
        match (a.next(), b.next()) {
            (None, None) => break,
            (x, y) => {
                order.extend(x.into_iter().flatten());
                order.extend(y.into_iter().flatten());
            }
        }
    }
    order.reverse();
    order
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
/// while blocks remain. With the redraw thread, hand it the next blocks instead ([`step_threaded`]).
pub(crate) fn step(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, doc: &Arc<Document>, display: Option<&CanvasDisplay>) -> Step {
    land_finished(app);
    if app.canvas_states.get(&doc.id).is_some_and(|s| !s.thread_failed) && super::redraw_thread::thread(app, gpu, ctx).is_some() {
        return step_threaded(app, ctx, doc, display);
    }
    let started = crate::gpu_canvas::now_ms();
    let mut batches = true;
    loop {
        // The blocks the rest of the budget holds go in one render: the document is planned
        // once for all of them (about 2 ms a plan on a storyboard of ~450 passes, per block before).
        let n = if batches { batch_len(app, doc.id, started) } else { 1 };
        // Checked before rendering: a batch of one (the sweep's last block) goes the one-block way
        // below, and rendering it here too drew it twice.
        let batch: Vec<Rect> =
            if n > 1 { app.canvas_states.get(&doc.id).map(|s| s.pending.iter().rev().take(n).copied().collect()).unwrap_or_default() } else { Vec::new() };
        if batch.len() > 1 {
            match gpu.refresh_blocks(doc.id.0, doc, &batch, display) {
                Some(r) => {
                    app.perf.record_refresh(&r);
                    app.perf.gpu_uploads = r.uploads;
                    if let Some(s) = app.canvas_states.get_mut(&doc.id) {
                        let keep = s.pending.len().saturating_sub(batch.len());
                        s.pending.truncate(keep);
                        s.learn(&r, batch.len());
                    }
                    if !running(app, doc.id) {
                        return Step::Done;
                    }
                    if crate::gpu_canvas::now_ms() - started >= FRAME_BUDGET_MS {
                        ctx.request_repaint();
                        return Step::Continuing;
                    }
                    continue;
                }
                // One at a time for the rest of the frame, each with its own fallback.
                None => batches = false,
            }
        }
        let Some(block) = app.canvas_states.get_mut(&doc.id).and_then(|s| s.pending.pop()) else { return Step::Done };
        let r = gpu.refresh_with(doc.id.0, doc, Some(block), display, false);
        // Counted like any refresh, so `ui.inspect` and slow-frame reports include the blocks.
        if r.fallback.is_none() && matches!(r.kind, "gpu-rect" | "gpu-full" | "rect") {
            app.perf.record_refresh(&r);
            if r.kind.starts_with("gpu") {
                app.perf.gpu_uploads = r.uploads;
            }
        }
        if r.fallback.is_none()
            && let Some(s) = app.canvas_states.get_mut(&doc.id)
        {
            s.learn(&r, 1);
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

/// Keep `doc`'s sweep going on the redraw thread on a frame that drew an edit of its own (a brush
/// step, a drag): [`step`] runs only on frames with nothing new, so the rest of the canvas waited
/// until the stroke or drag ended. Never draws on this thread: without the redraw thread the sweep
/// waits as before, rather than take frame time from the edit.
pub(crate) fn keep_fed(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, doc: &Arc<Document>, display: Option<&CanvasDisplay>) {
    if !running(app, doc.id) {
        return;
    }
    land_finished(app);
    if app.canvas_states.get(&doc.id).is_some_and(|s| !s.thread_failed) && super::redraw_thread::thread(app, gpu, ctx).is_some() {
        let _ = step_threaded(app, ctx, doc, display);
    }
}

/// [`step`] on the redraw thread: keep it fed with [`IN_FLIGHT`] jobs of the next blocks and say
/// whether the sweep is over. Never draws on this thread.
///
/// [`IN_FLIGHT`]: super::redraw_thread::IN_FLIGHT
fn step_threaded(app: &mut PhotocraftApp, ctx: &egui::Context, doc: &Arc<Document>, display: Option<&CanvasDisplay>) -> Step {
    use super::redraw_thread::{IN_FLIGHT, JOB_BLOCKS, JOB_BLOCKS_IDLE};
    super::redraw_thread::publish(app, doc, display.is_some_and(|d| d.encode_srgb));
    let now = crate::gpu_canvas::now_ms();
    let waiting = app.canvas_states.get(&doc.id).is_some_and(|s| s.thread_wait_until > now);
    // Larger jobs while nothing is drawn by hand; small ones keep an edit's wait for the
    // compositor short.
    let job = if app.drag.is_none() && app.live_stroke.is_none() { JOB_BLOCKS_IDLE } else { JOB_BLOCKS };
    while !waiting && let Some(s) = app.canvas_states.get_mut(&doc.id) {
        if s.in_flight.len() >= IN_FLIGHT || s.pending.is_empty() {
            break;
        }
        // The next blocks (popped from the end), in drawing order.
        let keep = s.pending.len().saturating_sub(job);
        let blocks: Vec<Rect> = s.pending.drain(keep..).rev().collect();
        let Some(ticket) = app.redraw_thread.as_mut().and_then(|t| t.send(doc.id, blocks.clone())) else {
            if let Some(s) = app.canvas_states.get_mut(&doc.id) {
                s.pending.extend(blocks.into_iter().rev());
            }
            thread_gone(app);
            break;
        };
        if let Some(s) = app.canvas_states.get_mut(&doc.id) {
            s.in_flight.push((ticket, blocks));
        }
    }
    if !running(app, doc.id) {
        return Step::Done;
    }
    // The thread asks for a repaint as each job lands; this is a backstop (and the retry of a
    // compositor that wasn't ready).
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    Step::Continuing
}

/// Take the jobs the redraw thread finished, of any document ([`CanvasState::landed`]).
fn land_finished(app: &mut PhotocraftApp) {
    let Some((finished, alive)) = app.redraw_thread.as_ref().map(super::redraw_thread::RedrawThread::finished) else { return };
    for done in finished {
        if let Some(s) = app.canvas_states.get_mut(&done.doc) {
            s.landed(done.ticket, done.blocks, done.result);
        }
    }
    if !alive {
        thread_gone(app);
    }
}

/// The redraw thread stopped: the blocks it had go back to their sweeps, which go on on the UI
/// thread, and no thread is started again.
fn thread_gone(app: &mut PhotocraftApp) {
    log::warn!("canvas: the redraw thread stopped; redrawing on the UI thread");
    app.redraw_thread = None;
    app.redraw_thread_failed = true;
    for s in app.canvas_states.values_mut() {
        for (_, blocks) in std::mem::take(&mut s.in_flight) {
            s.pending.extend(blocks.iter().rev());
        }
        s.thread_failed = true;
    }
}

impl CanvasState {
    /// Job `ticket` of `blocks` came back from the redraw thread with `result`. Blocks it couldn't
    /// draw go back to the front of the sweep: to try again shortly if the compositor wasn't
    /// built yet, else to draw on the UI thread.
    fn landed(&mut self, ticket: u64, blocks: Vec<Rect>, result: Result<photocraft_gpu::Stats, photocraft_gpu::Unsupported>) {
        self.in_flight.retain(|(t, _)| *t != ticket);
        let Err(e) = result else { return };
        // Popped from the end: the first of them is drawn first.
        self.pending.extend(blocks.iter().rev());
        if e.0 == crate::gpu_canvas::COMPOSITOR_BUILDING {
            self.thread_wait_until = crate::gpu_canvas::now_ms() + THREAD_RETRY_MS;
        } else {
            log::info!("canvas: the redraw thread couldn't draw {} block(s) ({}); going on on the UI thread", blocks.len(), e.0);
            self.thread_failed = true;
        }
    }
}

impl CanvasState {
    /// Learn from a GPU render of `blocks` blocks what planning and each block cost.
    fn learn(&mut self, r: &crate::gpu_canvas::Refresh, blocks: usize) {
        let Some(stats) = r.stats.as_ref() else { return };
        let avg = |old: Option<f64>, new: f64| Some(old.map_or(new, |o| o * 0.7 + new * 0.3));
        self.plan_ms = avg(self.plan_ms, stats.plan_ms);
        self.block_ms = avg(self.block_ms, ((r.composite_ms - stats.plan_ms) / blocks.max(1) as f64).max(0.1));
    }
}

/// How many of `doc`'s pending blocks fit what is left of this frame's budget, once a render's
/// cost is known: 1 until then, at most [`MAX_BATCH`].
fn batch_len(app: &PhotocraftApp, doc: DocId, started: f64) -> usize {
    let Some(s) = app.canvas_states.get(&doc) else { return 1 };
    let (Some(plan), Some(block)) = (s.plan_ms, s.block_ms) else { return 1 };
    let left = FRAME_BUDGET_MS - (crate::gpu_canvas::now_ms() - started) - plan;
    ((left / block).floor().max(1.0) as usize).min(MAX_BATCH)
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

    /// A region added to a running sweep: its blocks still waiting from before go first, then its
    /// other blocks and the rest of the sweep take turns, a job's worth at a time.
    #[test]
    fn a_new_region_takes_turns_with_the_sweep_and_blocks_waiting_again_go_first() {
        use super::super::redraw_thread::JOB_BLOCKS;
        assert_eq!(JOB_BLOCKS, 3, "the order below is for jobs of three");
        let r = |x: i32| Rect::new(x, 0, x + 1, 1);
        // Drawn in the order 0, 1, ... 7 (popped from the end).
        let pending: Vec<Rect> = (0..8).rev().map(r).collect();
        // A region of blocks 100..106 and block 5, which was still waiting.
        let new: Vec<Rect> = [100, 101, 102, 5, 103, 104, 105].into_iter().rev().map(r).collect();
        let mut q = queue_after_add(pending, new);
        let mut order = Vec::new();
        while let Some(b) = q.pop() {
            order.push(b.x0);
        }
        assert_eq!(order, [5, 100, 101, 102, 0, 1, 2, 103, 104, 105, 3, 4, 6, 7]);
        // Nothing running: the region alone, in its own order.
        let mut q = queue_after_add(Vec::new(), [1, 2].into_iter().rev().map(r).collect());
        assert_eq!((q.pop().map(|b| b.x0), q.pop().map(|b| b.x0)), (Some(1), Some(2)));
    }

    /// Blocks out on the redraw thread keep the sweep running; a block the thread couldn't draw
    /// comes back to be drawn on the UI thread, and a compositor not built yet makes the blocks
    /// wait a moment instead. A new sweep tries the thread again.
    #[test]
    fn blocks_on_the_redraw_thread_keep_the_sweep_running_and_failed_ones_come_back() {
        let mut app = app();
        let big = doc(10_000, 10_000);
        start(&mut app, &big, big.bounds());
        let s = app.canvas_states.get_mut(&big.id).unwrap();
        // The first three blocks go out, in drawing order (popped from the end).
        let keep = s.pending.len() - 3;
        let out: Vec<Rect> = s.pending.drain(keep..).rev().collect();
        s.in_flight.push((1, out.clone()));
        s.pending.clear();
        assert!(running(&app, big.id), "blocks in flight are still to draw");
        let s = app.canvas_states.get_mut(&big.id).unwrap();
        s.landed(1, out.clone(), Err(photocraft_gpu::Unsupported(crate::gpu_canvas::COMPOSITOR_BUILDING.into())));
        assert!(!s.thread_failed && s.thread_wait_until > 0.0, "not built yet: they wait and go again");
        assert_eq!(s.pending.iter().rev().copied().collect::<Vec<_>>(), out, "back in the order they were sent");
        s.pending.clear();
        s.in_flight.push((2, out.clone()));
        s.landed(2, out.clone(), Err(photocraft_gpu::Unsupported("no canvas texture to redraw into".into())));
        assert!(s.thread_failed, "failed: the sweep goes on on the UI thread");
        assert!(s.in_flight.is_empty() && s.pending.len() == 3);
        s.pending.clear();
        s.in_flight.push((3, out.clone()));
        s.landed(3, out, Ok(photocraft_gpu::Stats::default()));
        assert!(!running(&app, big.id), "drawn: the sweep is over");
        add(&mut app, &big, Rect::new(0, 0, 100, 100), big.bounds());
        assert!(!app.canvas_states[&big.id].thread_failed, "a new redraw tries the thread again");
    }

    /// The blocks cover every pixel once, and the ones in view are drawn first.
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

    /// Overlapping regions split into the same grid blocks, so they can be deduplicated.
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

    /// An app with background jobs on, as on the desktop.
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app
    }

    /// An empty RGB document of `w` x `h` pixels.
    fn doc(w: u32, h: u32) -> Arc<Document> {
        Arc::new(Document::new("t", photocraft_geom::Size::new(w, h), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8))
    }

    /// Sweeps need a canvas of at least `SWEEP_MIN_PX`, background jobs, and no failed sweep.
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

    /// A block the CPU compositor drew ends the sweep until a full refresh lands.
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

    /// A region added twice queues its blocks once, at the front.
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

    /// The safety net keeps a running sweep, and the changed area is drawn before the rest.
    #[test]
    fn the_safety_net_keeps_a_running_sweep_and_draws_the_change_first() {
        let mut app = app();
        let big = doc(10_000, 10_000);
        start(&mut app, &big, big.bounds());
        // Part of the sweep drawn.
        let drawn = 10;
        for _ in 0..drawn {
            app.canvas_states.get_mut(&big.id).unwrap().pending.pop();
        }
        let left = app.canvas_states[&big.id].pending.len();
        // A compared change: the net doesn't restart the sweep, and the changed area goes first.
        let changed = Rect::new(7_000, 7_000, 8_000, 8_000);
        safety_net(&mut app, &big, big.bounds());
        assert_eq!(app.canvas_states[&big.id].pending.len(), left, "the running sweep carries on");
        add(&mut app, &big, changed, big.bounds());
        let next = *app.canvas_states[&big.id].pending.last().unwrap();
        assert!(!next.intersect(&changed).is_empty(), "the changed area is drawn next: {next:?}");
        // With nothing running, the net is a whole sweep.
        app.canvas_states.get_mut(&big.id).unwrap().pending.clear();
        safety_net(&mut app, &big, big.bounds());
        assert!(running(&app, big.id));
    }

    /// One block a frame until a render has been measured; then as many as the frame budget
    /// holds (up to `MAX_BATCH`), and back to one when each block costs a frame.
    #[test]
    fn blocks_are_batched_once_a_render_has_been_measured() {
        let mut app = app();
        let big = doc(10_000, 10_000);
        start(&mut app, &big, big.bounds());
        let now = crate::gpu_canvas::now_ms();
        assert_eq!(batch_len(&app, big.id, now), 1, "one block until a render is measured");
        let stats = photocraft_gpu::Stats { plan_ms: 2.0, ..Default::default() };
        let r = crate::gpu_canvas::Refresh { kind: "gpu-rect", composite_ms: 6.0, stats: Some(stats), ..Default::default() };
        app.canvas_states.get_mut(&big.id).unwrap().learn(&r, 1);
        // 24 ms budget: a 2 ms plan, then 4 ms a block, at most MAX_BATCH.
        assert_eq!(batch_len(&app, big.id, now), MAX_BATCH);
        let slow = crate::gpu_canvas::Refresh { kind: "gpu-rect", composite_ms: 62.0, stats: Some(stats), ..Default::default() };
        for _ in 0..20 {
            app.canvas_states.get_mut(&big.id).unwrap().learn(&slow, 1);
        }
        assert_eq!(batch_len(&app, big.id, now), 1, "blocks that cost a frame each go one at a time");
    }

    /// A comparison needs the same document shown with the same display key.
    #[test]
    fn a_comparison_needs_the_same_display() {
        let mut app = app();
        let big = doc(8192, 8192);
        shown(&mut app, &big, 1);
        assert_eq!(changed_since_shown(&app, &big, 2), None, "another display key: nothing to compare");
        assert_eq!(changed_since_shown(&app, &doc(8192, 8192), 1), None, "another document: nothing shown");
    }
}
