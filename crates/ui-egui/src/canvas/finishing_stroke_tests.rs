//! A released Brush stroke finishing over the next frames (`strokes::continue_finishing_stroke`):
//! the release doesn't wait for the live stroke's thread, presses and commands made meanwhile
//! wait their turn, and the commit takes the stroke's pixels once it is done.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::strokes::{continue_finishing_stroke, feed_live_stroke_waiting, land_finished_stroke, live_stroke_busy, still_finishing, stroke_command};
use super::{PhotocraftApp, Tool, ToolEvent, tool_event};

const M: egui::Modifiers = egui::Modifiers::NONE;

/// A document with an empty layer and the Brush set up to paint a short stroke on it, with
/// background work on, as on the desktop.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
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

/// Dropped values never wait in a queue longer than the bin's depth, and the call never waits:
/// past that, each is freed on a thread of its own. Every one is freed in the end.
#[test]
fn a_full_drop_bin_frees_on_threads_of_its_own_without_waiting() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Slow(Arc<AtomicBool>, Arc<AtomicUsize>);
    impl Drop for Slow {
        fn drop(&mut self) {
            while !self.0.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.1.fetch_add(1, Ordering::AcqRel);
        }
    }
    let (go, freed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let start = Instant::now();
    for _ in 0..6 {
        super::paint_thread::drop_elsewhere(Slow(go.clone(), freed.clone()));
    }
    assert!(start.elapsed() < Duration::from_secs(2), "handing them over never waits for a drop");
    go.store(true, Ordering::Release);
    while freed.load(Ordering::Acquire) < 6 {
        assert!(start.elapsed() < Duration::from_secs(10), "all six are freed");
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
