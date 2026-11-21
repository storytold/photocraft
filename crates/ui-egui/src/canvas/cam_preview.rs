//! Content-Aware Move drags and its Transform On Drop box shown with the result: once the pointer
//! (or the box) rests for a moment, the move is computed as a background job
//! ([`photocraft_engine::retouch_cmds::content_aware_move_preview`]: a smaller sampling window than
//! the command) and the canvas shows the document with it. A selection too large to solve at once
//! shows a coarse result first (solved scaled down), then the exact one, computed as a second job.
//! Any change of the offset, scale or rotation cancels the jobs and starts the wait again, so the
//! UI never waits for the blend. Releasing (or Enter) runs the exact `paint.contentAwareMove`; the
//! document is untouched until then. Not available on wasm (no threads).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use craft::imaging::ui::live_preview::{LivePreview, PreviewKey, PreviewLane, PreviewSession, settings_hash};
use craft::ui::canvas::PixelRect;
use photocraft_doc::Document;
use photocraft_engine::jobs::JobCtx;
use photocraft_engine::retouch_cmds::ContentAwarePreview;
use serde_json::Value;

use crate::PhotocraftApp;
use crate::state::Tool;

/// How long (s) the move must stay unchanged before a preview is computed.
const SETTLE_S: f64 = 0.15;

struct Running {
    key: PreviewKey,
    ctx: JobCtx,
    rx: Receiver<photocraft_engine::Result<Document>>,
    /// Test-only notification sent after this worker publishes its real result.
    #[cfg(test)]
    finished: Receiver<()>,
    /// The full-resolution job that follows a coarse one.
    refine: Option<ContentAwarePreview>,
    /// This job refines a coarse preview already shown: its failure leaves that one shown.
    refining: bool,
}

pub(crate) struct CamPreview {
    /// The session (document, revision, active layer) and the settle time of the move's keys.
    live: LivePreview,
    /// The key of the params the move has now.
    key: PreviewKey,
    /// The key whose preview failed (a locked layer, say): not tried again.
    failed: Option<PreviewKey>,
    running: Option<Running>,
    /// The latest finished preview, with the key it shows under: `key.stage(1)` for a coarse result, the key itself
    /// for the exact one.
    shown: Option<(PreviewKey, Arc<Document>)>,
}

impl CamPreview {
    fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.ctx.cancel();
        }
    }

    /// Whether the shown result is of `key`'s move, coarse or exact: an older one would show a wrong place.
    fn shows(&self, key: PreviewKey) -> bool {
        self.shown.as_ref().is_some_and(|s| s.0 == key || s.0 == key.stage(1))
    }
}

impl Drop for CamPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `paint.contentAwareMove`'s params for what the pointer or the box has now; `None` without a
/// move to preview.
fn wanted(app: &mut PhotocraftApp) -> Option<Value> {
    if app.ui.tool != Tool::ContentAwareMove || cfg!(target_arch = "wasm32") {
        return None;
    }
    if let Some(t) = &app.ui.cam_transform {
        let place = crate::cam_transform::placement(t)?;
        if place.offset == [0, 0] && place.is_plain_move() {
            return None;
        }
        return Some(crate::cam_transform::params(app, &place));
    }
    let retouch = &app.canvas_tools.retouch;
    let press = retouch.press()?;
    let (start, mods) = (press.start, press.modifiers);
    let end = retouch.last_point();
    if !crate::retouch_ui::patch_drags_selection(app, start, mods) {
        return None;
    }
    let off = crate::retouch_ui::patch_offset(app, start, end);
    (off != [0, 0]).then(|| crate::retouch_ui::content_aware_move_params(app, off.map(i64::from)))
}

/// The document to show for the move on document `idx`: with the move applied, once its preview
/// has been computed for the current offset and transform. The move settles on the pass's time.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let now = app.ctx().input(|i| i.time) * 1000.0;
    step(app, now)
}

/// One pass at `now` (ms): the keys take seconds, the clock of [`LivePreview`].
fn step(app: &mut PhotocraftApp, now: f64) -> Option<(Arc<Document>, u64)> {
    let now_s = now / 1000.0;
    let Some(params) = wanted(app) else {
        app.cam_preview = None;
        return None;
    };
    let st = app.session.active()?;
    let session = PreviewSession::new(st.doc.id.0, st.revision, st.active_layer.map_or(0, |l| l.0 + 1));
    let b = st.doc.bounds();
    if app.cam_preview.as_ref().is_some_and(|p| p.live.session() != Some(session)) {
        app.cam_preview = None;
    }
    let p = app.cam_preview.get_or_insert_with(|| CamPreview {
        live: LivePreview::new(PreviewLane::Tool),
        key: PreviewKey::NONE,
        failed: None,
        running: None,
        shown: None,
    });
    p.live.begin(session, PixelRect::new(b.x0, b.y0, b.x1, b.y1));
    let key = p.live.key(settings_hash(&[params.to_string().as_bytes()]), now_s);
    if p.key != key {
        p.key = key;
        // What is computing is already out of date.
        if p.running.as_ref().is_some_and(|r| r.key != key) {
            p.stop();
        }
    }
    if let Some(r) = &p.running {
        match r.rx.try_recv() {
            Ok(result) => {
                let r = p.running.take();
                match (result, r) {
                    (Ok(d), Some(r)) => {
                        // Duotone documents display through their inks.
                        let d = photocraft_engine::mode_cmds::display_document(&d).unwrap_or(d);
                        // A coarse result shows under its stage key while the exact one follows.
                        let shown_key = if r.refine.is_some() { r.key.stage(1) } else { r.key };
                        p.shown = Some((shown_key, Arc::new(d)));
                        if let Some(job) = r.refine {
                            p.running = spawn(job, r.key, true);
                        }
                    }
                    // A failed refinement leaves the coarse result shown.
                    (_, Some(r)) if r.refining => {}
                    (_, Some(r)) => p.failed = Some(r.key),
                    _ => {}
                }
            }
            Err(TryRecvError::Disconnected) => {
                if let Some(r) = p.running.take().filter(|r| !r.refining) {
                    p.failed = Some(r.key);
                }
            }
            Err(TryRecvError::Empty) => {}
        }
    }
    let settled = !p.live.settling(now_s, SETTLE_S);
    // A coarse result whose refinement failed counts as done: it stays shown.
    let done = p.shows(key);
    if settled && !done && p.running.is_none() && p.failed != Some(key) {
        match start(app, &params, key) {
            Some(r) => app.cam_preview.as_mut()?.running = Some(r),
            None => app.cam_preview.as_mut()?.failed = Some(key),
        }
    }
    let p = app.cam_preview.as_ref()?;
    // Only a preview of the move as it is now.
    let (shown_key, d) = p.shown.as_ref().filter(|_| p.shows(key))?;
    Some((d.clone(), shown_key.raw()))
}

/// Compute the preview of `params` on a worker thread.
fn start(app: &PhotocraftApp, params: &Value, key: PreviewKey) -> Option<Running> {
    let job = photocraft_engine::retouch_cmds::content_aware_move_preview(&app.session, params).ok()?;
    spawn(job, key, false)
}

/// Run `job` on a worker thread (`refining` a coarse preview already shown).
fn spawn(job: ContentAwarePreview, key: PreviewKey, refining: bool) -> Option<Running> {
    let refine = job.refine();
    let ctx = JobCtx::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = ctx.clone();
    #[cfg(test)]
    let (finished_tx, finished) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("cam-preview".into())
        .spawn(move || {
            let _ = tx.send(job.run(&worker).map(|(d, _)| d));
            #[cfg(test)]
            let _ = finished_tx.send(());
        })
        .ok()?;
    Some(Running {
        key,
        ctx,
        rx,
        #[cfg(test)]
        finished,
        refine,
        refining,
    })
}

/// Whether the canvas shows a preview.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    app.cam_preview.as_ref().is_some_and(|p| p.shows(p.key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;

    /// The settle time in ms, the clock of `step`.
    const SETTLE_MS: f64 = SETTLE_S * 1000.0;

    /// The time of the app's last pass, in ms: the canvas pass of a tool event shows the move it made, so that pass
    /// keys the move at its own time.
    fn pass_ms(app: &PhotocraftApp) -> f64 {
        app.ctx().input(|i| i.time) * 1000.0
    }

    fn blue(d: &Document, x: i32, y: i32) -> bool {
        d.layers[0].surface().unwrap().rgba(x, y)[0] < 0.5
    }

    #[test]
    fn the_drag_previews_the_move_after_it_rests_and_never_touches_the_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
        app.run("paint.pencil", json!({"points": [[16, 30], [28, 30]], "size": 4, "color": "#0000ff"})).unwrap();
        app.run("select.lasso", json!({"points": [[6, 14], [38, 14], [38, 46], [6, 46]]})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        app.ui.tool_options.cam_transform_on_drop = false;
        let m = egui::Modifiers::NONE;
        let steps = app.session.active().unwrap().history.past_len();

        // Nothing to preview before the patch moves.
        tool_event(&mut app, ToolEvent::Down { x: 22.0, y: 30.0, pressure: 1.0 }, m);
        assert!(step(&mut app, 0.0).is_none() && app.cam_preview.is_none());
        tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 30.0, pressure: 1.0 }, m);
        let moved = pass_ms(&app);
        // Still moving: the wait starts and nothing is computed or shown yet.
        assert!(step(&mut app, moved).is_none());
        assert!(step(&mut app, moved + SETTLE_MS / 2.0).is_none() && app.cam_preview.as_ref().unwrap().running.is_none());
        // At rest: the job runs, off this thread, and its result shows once it arrives.
        let mut now = moved + SETTLE_MS + 1.0;
        assert!(step(&mut app, now).is_none() && app.cam_preview.as_ref().is_some_and(|p| p.running.is_some()), "the preview job starts off this thread");
        let (preview, key) = wait_for(&mut app, &mut now, 0).expect("a preview once the pointer rested");
        assert!(key != 0 && app.cam_preview.as_ref().unwrap().live.owns(PreviewKey::from_raw(key)) && showing(&app));
        assert!(blue(&preview, 72, 30) && !blue(&preview, 22, 30), "the preview shows the move");
        let doc = app.session.active().unwrap().doc.clone();
        assert!(blue(&doc, 22, 30) && !blue(&doc, 72, 30), "the document is untouched");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);

        // Moving on drops the stale preview at once and cancels what is not shown yet.
        tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 30.0, pressure: 1.0 }, m);
        assert!(step(&mut app, now + 5.0).is_none() && !showing(&app));
        // Release commits the exact move; the preview is gone.
        tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 30.0 }, m);
        assert!(step(&mut app, now + 10.0).is_none() && app.cam_preview.is_none());
    }

    #[test]
    fn the_transform_on_drop_box_previews_the_dropped_patch() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
        app.run("paint.pencil", json!({"points": [[16, 30], [28, 30]], "size": 4, "color": "#0000ff"})).unwrap();
        app.run("select.lasso", json!({"points": [[6, 14], [38, 14], [38, 46], [6, 46]]})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        app.ui.tool_options.cam_transform_on_drop = true;
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 22.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 30.0 }, m);
        assert!(crate::cam_transform::active(&app));
        let mut now = 0.0;
        let (preview, _) = wait_for(&mut app, &mut now, 0).expect("the box shows the result");
        assert!(blue(&preview, 72, 30) && !blue(&preview, 22, 30));
        // Cancelling the box drops the preview.
        crate::cam_transform::cancel(&mut app);
        assert!(step(&mut app, now + 1.0).is_none() && app.cam_preview.is_none());
    }

    /// Wait for the specific worker to publish its result, then let the normal non-blocking
    /// `step` path consume it. Coverage instrumentation can make a solve much slower than a
    /// fixed polling loop; the watchdog bounds a stuck worker without setting its expected speed. Returns the first
    /// shown result whose key is not `skip` (a raw key; 0 takes any).
    fn wait_for(app: &mut PhotocraftApp, now: &mut f64, skip: u64) -> Option<(Arc<Document>, u64)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            let shown = step(app, *now);
            if shown.as_ref().is_some_and(|s| s.1 != skip) {
                return shown;
            }
            let p = app.cam_preview.as_ref()?;
            if let Some(running) = &p.running {
                let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
                running.finished.recv_timeout(remaining).ok()?;
            } else if shown.is_some() || p.failed == Some(p.key) {
                return None;
            } else if p.live.settling(*now / 1000.0, SETTLE_S) {
                // Advance the simulated settle clock without sleeping in real time.
                *now += SETTLE_MS + 1.0;
            } else {
                return None;
            }
        }
    }

    #[test]
    fn a_large_selection_shows_a_coarse_result_then_the_exact_one() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The selection is just past the engine's full-resolution preview budget (256 x 256 pixels), the one condition
        // for the coarse stage; the solve's cost grows with the selection and the canvas, so both stay as small as the
        // move to x 250..507 allows (#15652).
        app.run("file.new", json!({"width": 520, "height": 280})).unwrap();
        app.run("paint.pencil", json!({"points": [[150, 160], [170, 160]], "size": 12, "color": "#0000ff"})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 257, "height": 256})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        app.ui.tool_options.cam_transform_on_drop = false;
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 340.0, y: 100.0, pressure: 1.0 }, m);
        assert!(step(&mut app, 0.0).is_none());
        let mut now = SETTLE_MS + 1.0;
        // First the coarse result, while the exact one computes.
        let (coarse, first) = wait_for(&mut app, &mut now, 0).expect("a coarse preview");
        assert!(blue(&coarse, 400, 160) && !blue(&coarse, 160, 160), "the coarse preview shows the move");
        let key = app.cam_preview.as_ref().unwrap().key;
        assert_eq!(first, key.stage(1).raw(), "the coarse result shows under the stage key");
        let refining = app.cam_preview.as_ref().unwrap().running.as_ref().is_some_and(|r| r.refining);
        assert!(refining, "the exact result follows");
        // Then the exact one: the command's own result, under the key itself.
        let (exact, second) = wait_for(&mut app, &mut now, first).expect("the exact preview");
        assert_eq!(second, key.raw());
        assert!(app.cam_preview.as_ref().unwrap().running.is_none() && showing(&app));
        assert!(!blue(&app.session.active().unwrap().doc, 400, 160), "the document is untouched");
        tool_event(&mut app, ToolEvent::Up { x: 340.0, y: 100.0 }, m);
        let doc = app.session.active().unwrap().doc.clone();
        let b = doc.bounds();
        assert!(
            exact.layers[0].surface().unwrap().read_region(b) == doc.layers[0].surface().unwrap().read_region(b),
            "the exact preview is what release makes"
        );
    }

    #[test]
    fn a_failed_preview_is_not_retried() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
        app.run("select.rect", json!({"x": 6, "y": 14, "width": 32, "height": 32})).unwrap();
        app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 22.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 30.0, pressure: 1.0 }, m);
        assert!(step(&mut app, 0.0).is_none());
        let mut now = SETTLE_MS + 1.0;
        for _ in 0..3000 {
            assert!(step(&mut app, now).is_none(), "nothing to show");
            if app.cam_preview.as_ref().unwrap().failed.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            now += 10.0;
        }
        let p = app.cam_preview.as_ref().unwrap();
        assert!(p.running.is_none() && p.shown.is_none() && p.failed == Some(p.key));
        // Resting longer starts nothing new.
        assert!(step(&mut app, now + 1000.0).is_none());
        assert!(app.cam_preview.as_ref().unwrap().running.is_none());
    }
}
