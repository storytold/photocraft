//! A released Brush stroke finishing over the next frames (`strokes::continue_finishing_stroke`):
//! the release doesn't wait for the live stroke's thread, presses and commands made meanwhile
//! wait their turn, and the commit takes the stroke's pixels once it is done.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;

use super::strokes::{continue_finishing_stroke, feed_live_stroke_waiting, land_finished_stroke, live_stroke_busy, still_finishing, stroke_command};
use super::{PhotocraftApp, Tool, ToolEvent, tool_event};

const M: egui::Modifiers = egui::Modifiers::NONE;

/// A document with an empty layer and the Brush set up to paint a short stroke on it, with
/// background work on, as on the desktop.
fn app() -> PhotocraftApp {
    app_with(Default::default())
}

/// [`app`] with `services`.
fn app_with(services: crate::Services) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 160, "height": 60, "background": "#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "pressureSize": false}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.background_jobs = true;
    app
}

/// Press, then move along the stroke with the live stroke's thread holding its step, as a slow
/// brush would; the release comes while it is still painting. Returns what lets the step go.
fn release_while_the_thread_is_busy(app: &mut PhotocraftApp) -> impl FnOnce() + Send + 'static {
    tool_event(app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
    let release = app.live_stroke.as_ref().unwrap().paint.hold();
    app.defer_live_stroke = true;
    for x in (30..=130).step_by(10) {
        tool_event(app, ToolEvent::Move { x: f64::from(x), y: 30.0, pressure: 1.0 }, M);
    }
    app.defer_live_stroke = false;
    feed_live_stroke_waiting(app, Duration::ZERO);
    assert!(live_stroke_busy(app), "the thread is still painting");
    tool_event(app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);
    release
}

/// Run frames until the released stroke has committed (`commit`), without the next frame.
fn commit(app: &mut PhotocraftApp) {
    let start = Instant::now();
    while continue_finishing_stroke(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "the stroke finishes");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Run frames until the released stroke has committed, and the next frame, which runs the
/// commands that waited for it.
fn finish(app: &mut PhotocraftApp) {
    commit(app);
    land_finished_stroke(app);
}

/// The command the release ran, undone and run again from its params: the pixels must match.
fn assert_commit_replays(app: &mut PhotocraftApp) {
    let cmd = stroke_command(app, Tool::Brush);
    let doc = app.session.active().unwrap().doc.clone();
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(doc.layer(id).unwrap().surface().unwrap().rgba(128, 30)[3] > 0.0, "the stroke reaches its last point");
    let params = app.session.journal.iter().rev().find(|(name, _)| name == cmd).unwrap_or_else(|| panic!("{cmd} ran")).1.clone();
    app.run("edit.undo", json!({})).unwrap();
    app.run(cmd, params).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *doc, "the committed stroke");
}

/// The release returns at once while the thread is still painting: nothing is committed and the
/// canvas keeps showing the live stroke. Once the thread is done, the stroke commits as one edit
/// that takes the live stroke's pixels.
#[test]
fn a_release_does_not_wait_for_the_thread_and_commits_once_it_is_done() {
    let mut app = app();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_while_the_thread_is_busy(&mut app);
    assert!(app.finishing_stroke.is_some(), "the release didn't wait");
    assert!(app.live_stroke.is_some(), "the canvas still shows the stroke");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "nothing committed yet");
    release();
    finish(&mut app);
    assert!(app.finishing_stroke.is_none() && app.live_stroke.is_none());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one edit");
    assert!(app.committed_stroke.is_some(), "the commit took the live stroke's pixels");
    assert_commit_replays(&mut app);
}

/// Meanwhile a new press waits briefly and is then turned away with a note; automation gets an
/// error to retry; a command made by hand runs once the stroke has committed, as if made after
/// it.
#[test]
fn presses_and_commands_wait_for_a_finishing_stroke() {
    let mut app = app();
    let layers = app.session.active().unwrap().doc.layers.len();
    let release = release_while_the_thread_is_busy(&mut app);

    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, M);
    assert!(app.drag.is_none(), "the press is turned away");
    assert_eq!(app.ui.status, still_finishing());

    app.automation_input = true;
    assert_eq!(app.run("layer.new.layer", json!({})).unwrap_err(), still_finishing(), "automation retries");
    app.automation_input = false;

    let queued = app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(queued["queued"], json!(true));
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "not yet");

    release();
    finish(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the command ran after the stroke");
    let cmd = stroke_command(&app, Tool::Brush);
    let order: Vec<&str> = app.session.journal.iter().map(|(name, _)| name.as_str()).filter(|n| *n == cmd || *n == "layer.new.layer").collect();
    assert_eq!(order.last().copied(), Some("layer.new.layer"), "the stroke first, then the command: {order:?}");
}

/// A step that panics on the thread while the stroke finishes loses the live stroke but never
/// the document: the release renders the whole stroke from the drag, as one edit. The panic is
/// armed on the held thread after the release, so it can only happen while the stroke finishes.
#[test]
fn a_finishing_stroke_whose_thread_fails_is_rendered_from_the_drag() {
    let mut app = app();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_while_the_thread_is_busy(&mut app);
    app.live_stroke.as_ref().unwrap().paint.panic_next_step();
    assert!(app.finishing_stroke.is_some(), "released, still finishing");
    release();
    finish(&mut app);
    assert!(app.live_stroke.is_none());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one edit");
    assert!(app.committed_stroke.is_none(), "rendered again, not taken from the lost live stroke");
    assert_commit_replays(&mut app);
}

/// Tool commands keep running while a stroke finishes. A commit rendered from its command (its
/// thread lost) still paints with the colour and brush of the release, as the canvas showed it;
/// the newer ones stay for the next stroke.
#[test]
fn a_commit_rendered_from_its_command_paints_with_the_release_colour_and_brush() {
    let mut app = app();
    let release = release_while_the_thread_is_busy(&mut app);
    app.live_stroke.as_ref().unwrap().paint.panic_next_step();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40}})).unwrap();
    release();
    finish(&mut app);
    assert!(app.committed_stroke.is_none(), "rendered from its command");
    let doc = app.session.active().unwrap().doc.clone();
    let surface = doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
    let px = surface.rgba(80, 30);
    assert!(px[3] > 0.0 && px[0] > px[2], "red, the release's colour: {px:?}");
    assert_eq!(surface.rgba(80, 48)[3], 0.0, "12 px wide, the release's brush, not 40");
    assert!(app.session.tools.foreground[2] > 0.9, "blue stays picked for the next stroke");
}

/// A slow value to free: its drop waits for `go`, then counts itself.
struct SlowDrop(Arc<AtomicBool>, Arc<AtomicUsize>);

impl Drop for SlowDrop {
    fn drop(&mut self) {
        while !self.0.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.1.fetch_add(1, Ordering::AcqRel);
    }
}

/// A few values while the bin is busy: queued, then each on a thread of its own, and handing them
/// over never waits. Every one is freed in the end.
#[test]
fn a_busy_drop_bin_frees_on_threads_of_its_own_without_waiting() {
    let bin = super::paint_thread::DropBin::new(2, 2);
    let (go, freed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let start = Instant::now();
    // At most 2 waiting + the one the bin frees + 2 extra threads: never one freed here.
    for _ in 0..4 {
        bin.drop_elsewhere(SlowDrop(go.clone(), freed.clone()));
    }
    assert!(start.elapsed() < Duration::from_secs(2), "handing them over never waits for a drop");
    go.store(true, Ordering::Release);
    while freed.load(Ordering::Acquire) < 4 || bin.outstanding() > 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "all four are freed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Under sustained load the outstanding values stay bounded (2 waiting, the one the bin frees, 2
/// extra threads): past that, a value is freed where it is handed over, so the caller waits for it
/// instead of documents and threads piling up. Freeing where no thread can start takes the same
/// path.
#[test]
fn a_drop_bin_under_sustained_load_stays_bounded() {
    let bin = super::paint_thread::DropBin::new(2, 2);
    let (go, freed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let releaser = {
        let go = go.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            go.store(true, Ordering::Release);
        })
    };
    let start = Instant::now();
    let mut peak = 0;
    for _ in 0..12 {
        bin.drop_elsewhere(SlowDrop(go.clone(), freed.clone()));
        peak = peak.max(bin.outstanding());
    }
    assert!(peak <= 5, "at most 5 outstanding, saw {peak}");
    assert!(start.elapsed() >= Duration::from_millis(250), "past the bound the caller waited for a drop");
    releaser.join().unwrap();
    while freed.load(Ordering::Acquire) < 12 || bin.outstanding() > 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "all twelve are freed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// While it finishes, the canvas keeps drawing the live stroke. Dropping it there redrew the whole
/// document without the stroke (2 s on a 1.1-gigapixel storyboard), and a press meanwhile made it
/// do so frame after frame until Wayland dropped the window.
#[test]
fn the_canvas_keeps_drawing_a_released_stroke_while_it_finishes() {
    let mut app = app();
    let release = release_while_the_thread_is_busy(&mut app);
    let idx = app.session.active_index().unwrap();
    assert!(super::strokes::live_stroke(&app, idx).is_some(), "the canvas draws the live stroke, not the document without it");
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, M);
    assert!(super::strokes::live_stroke(&app, idx).is_some(), "a press turned away changes nothing");
    release();
    finish(&mut app);
    assert!(super::strokes::live_stroke(&app, idx).is_none(), "committed: the document shows it");
}

/// Ctrl+Z right after the release is offered even on a document with nothing to undo before the
/// stroke, waits for the stroke and undoes it.
#[test]
fn undo_right_after_a_release_undoes_the_stroke() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 160, "height": 60, "background": "#00ff00"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "pressureSize": false}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.background_jobs = true;
    assert!(!crate::menus::is_enabled(&app, "edit.undo"), "nothing to undo before the stroke");
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_while_the_thread_is_busy(&mut app);
    assert!(crate::menus::is_enabled(&app, "edit.undo"), "Undo is offered while the stroke finishes");
    assert_eq!(app.run("edit.undo", json!({})).unwrap()["queued"], json!(true));
    release();
    finish(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "the stroke was undone");
    assert_eq!(*app.session.active().unwrap().doc, *before, "back to the document before the stroke");
}

/// Nothing else lands on the document in the frame a released stroke commits, so the canvas shows
/// the commit first and redraws only the stroke's last steps. Undo queued behind the stroke ran in
/// that frame and made it redraw the whole canvas (2 s on a 1.1-gigapixel storyboard, long enough
/// for Wayland to drop the window). A command queued behind it, and a new press, wait a frame.
#[test]
fn nothing_else_lands_in_the_frame_a_stroke_commits() {
    let mut app = app();
    let (steps, layers) = (app.session.active().unwrap().history.past_len(), app.session.active().unwrap().doc.layers.len());
    let painted = app.session.active().unwrap().active_layer.unwrap();
    let release = release_while_the_thread_is_busy(&mut app);
    assert_eq!(app.run("edit.undo", json!({})).unwrap()["queued"], json!(true));
    release();
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "committed; the Undo waits for the next frame");
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, M);
    assert!(app.drag.is_none(), "a press in the commit's frame is turned away");
    assert_eq!(app.run("layer.new.layer", json!({})).unwrap()["queued"], json!(true), "a command queues behind the Undo");

    assert!(land_finished_stroke(&mut app), "the next frame runs them");
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(doc.layer(painted).unwrap().surface().unwrap().rgba(80, 30)[3], 0.0, "the Undo took the stroke back");
    assert_eq!(doc.layers.len(), layers + 1, "then the new layer");
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, M);
    assert!(app.drag.is_some(), "a press now paints");
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 40.0 }, M);
    finish(&mut app);
}

/// Commands queued behind a finishing stroke run in order once it lands; one that fails doesn't
/// stop the rest, and its error is what the status bar shows afterwards, not hidden by a later
/// success.
#[test]
fn a_queued_command_that_fails_is_reported_after_the_queue_runs() {
    let mut app = app();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_while_the_thread_is_busy(&mut app);
    for id in ["no.suchCommand", "layer.new.layer"] {
        assert_eq!(app.run(id, json!({})).unwrap()["queued"], json!(true), "{id}: queued");
    }
    release();
    finish(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 2, "the stroke, then the new layer");
    assert!(app.ui.status_error, "the failed command is reported: {}", app.ui.status);
    assert!(!app.ui.status.is_empty());
}

/// The reply to `engine.execute` of `actions.play` "Save a copy" over the control channel.
fn play_over_control(app: &mut PhotocraftApp, ctx: &egui::Context) -> serde_json::Value {
    let (req, _rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": "actions.play", "params": {"action": "Save a copy"}}));
    match crate::control::handle(app, ctx, &req) {
        crate::control::Outcome::Done(v) => v,
        _ => panic!("engine.execute: expected an immediate reply"),
    }
}

/// An `actions.play` sent over the control channel while a stroke finishes is refused with an
/// error to retry, never queued (queued, it would run after the request, outside its gate). Its
/// retry once the stroke has landed runs under the request's automation gate: a `file.saveACopy`
/// step is refused, and nothing is written at any point. The gate here refuses `file.*` the way
/// the desktop's does (`authorize_desktop_engine_step`, in `photocraft-automation`, which this
/// crate doesn't depend on).
#[test]
fn an_actions_play_over_control_never_writes_during_or_after_a_stroke() {
    fn deny_files(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
        if id.starts_with("file.") && id != "file.new" {
            Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
        } else {
            Ok(())
        }
    }
    let written: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::default();
    let w = written.clone();
    let services = crate::Services {
        automation_authorize: Some(deny_files),
        automation_command: Some(Box::new(|id, params| deny_files(id, params).map_err(|e| e.to_string()))),
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = app_with(services);
    let outside = std::env::temp_dir().join("outside").join("x.png").to_string_lossy().into_owned();
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
        name: "Save a copy".into(),
        steps: vec![("file.saveACopy".into(), json!({"path": outside}))],
        ..Default::default()
    });
    let release = release_while_the_thread_is_busy(&mut app);
    let ctx = egui::Context::default();
    let refused = play_over_control(&mut app, &ctx);
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], still_finishing(), "an error to retry: {refused}");
    assert!(app.queued_commands.is_empty(), "not queued");
    assert!(app.session.authorize.is_none(), "the gate is only installed for the request");

    release();
    finish(&mut app);
    let retried = play_over_control(&mut app, &ctx);
    assert_eq!(retried["result"]["ran"], 0, "{retried}");
    assert_eq!(retried["result"]["failed"]["id"], "file.saveACopy", "{retried}");
    assert!(retried["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{retried}");
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(app.session.authorize.is_none());
}

/// Sets its flag when freed.
struct Marked(Arc<AtomicBool>);

impl Drop for Marked {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A [`super::paint_thread::Spawn`] that never starts a thread.
fn no_threads(_: &str, _: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    Err(std::io::Error::other("no threads here"))
}

/// A [`super::paint_thread::Spawn`] that starts the bin's own thread but no extra ones.
fn bin_thread_only(name: &str, f: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    if name == "photocraft-drop" { std::thread::Builder::new().name(name.into()).spawn(f).map(|_| ()) } else { no_threads(name, f) }
}

/// The bin's own thread can't start (the web, or out of threads): every value is freed where it is
/// handed over, at once, and none is left outstanding.
#[test]
fn a_drop_bin_without_its_thread_frees_in_place() {
    let bin = super::paint_thread::DropBin::with_spawn(2, 2, no_threads);
    let freed = Arc::new(AtomicBool::new(false));
    bin.drop_elsewhere(Marked(freed.clone()));
    assert!(freed.load(Ordering::Acquire), "freed before the call returned");
    assert_eq!(bin.outstanding(), 0);
}

/// The bin runs but an extra thread can't start: a value past the queue is freed where it is
/// handed over, and the counts stay right; the queued ones are freed by the bin as usual.
#[test]
fn a_drop_bin_whose_extra_thread_cant_start_frees_that_value_in_place() {
    let bin = super::paint_thread::DropBin::with_spawn(1, 2, bin_thread_only);
    let (go, freed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    // The bin takes the first and is busy freeing it; the second waits in its queue of one.
    let started = Arc::new(AtomicBool::new(false));
    /// Marks that the bin has started freeing it, then frees the slow value it holds.
    struct Busy {
        started: Arc<AtomicBool>,
        _slow: SlowDrop,
    }
    impl Drop for Busy {
        fn drop(&mut self) {
            self.started.store(true, Ordering::Release);
        }
    }
    bin.drop_elsewhere(Busy { started: started.clone(), _slow: SlowDrop(go.clone(), freed.clone()) });
    let start = Instant::now();
    while !started.load(Ordering::Acquire) {
        assert!(start.elapsed() < Duration::from_secs(10), "the bin takes the first");
        std::thread::sleep(Duration::from_millis(1));
    }
    bin.drop_elsewhere(SlowDrop(go.clone(), freed.clone()));
    let past = Arc::new(AtomicBool::new(false));
    bin.drop_elsewhere(Marked(past.clone()));
    assert!(past.load(Ordering::Acquire), "no extra thread: freed before the call returned");
    assert_eq!(bin.outstanding(), 2, "the two the bin still holds");
    go.store(true, Ordering::Release);
    while freed.load(Ordering::Acquire) < 2 || bin.outstanding() > 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "the bin frees the other two");
        std::thread::sleep(Duration::from_millis(2));
    }
}
