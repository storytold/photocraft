//! The canvas around a live stroke's thread (`paint_thread`): pen-up and cancel while the thread
//! is still painting, and a step that panics there.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::strokes::{feed_live_stroke_waiting, live_stroke_busy, stroke_command};
use super::{PhotocraftApp, Tool, ToolEvent, settle_pending_stroke, testing, tool_event};

const M: egui::Modifiers = egui::Modifiers::NONE;

/// A document with an empty layer and `tool` set up to paint a short stroke on it.
fn app_for(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width": 160, "height": 60, "background": "#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "pressureSize": false}})).unwrap();
    app.ui.tool = tool;
    app
}

/// The pen moves along the stroke to x 120, one pass per move. Each pass feeds the live stroke and,
/// with `LIVE_STROKE_WAIT` at `Duration::MAX` in tests, waits until it has painted, so the moves run
/// before the thread is held: a pass would wait for a held step for ever.
fn move_along(app: &mut PhotocraftApp) {
    for x in (30..=120).step_by(10) {
        tool_event(app, ToolEvent::Move { x: f64::from(x), y: 30.0, pressure: 1.0 }, M);
    }
}

/// The pen's last point (x 130) reaches the drag between passes and is handed to the thread without
/// waiting for its step, as `tools::run` does with the paint state taken out of the app: a held
/// thread is then still painting it.
fn feed_last_point_without_waiting(app: &mut PhotocraftApp) {
    let mut state = std::mem::take(&mut app.canvas_tools.paint);
    if let Some(stroke) = state.stroke.as_mut() {
        stroke.points.push([130.0, 30.0, 1.0]);
        feed_live_stroke_waiting(app, stroke, Duration::ZERO);
    }
    app.canvas_tools.paint = state;
}

/// The command the release ran, undone and run again from its params: the pixels must match.
fn assert_commit_replays(app: &mut PhotocraftApp, cmd: &str, what: &str) {
    let doc = app.session.active().unwrap().doc.clone();
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(doc.layer(id).unwrap().surface().unwrap().rgba(128, 30)[3] > 0.0, "{what}: the stroke reaches its last point");
    let params = app.session.journal.iter().rev().find(|(name, _)| name == cmd).unwrap_or_else(|| panic!("{what}: {cmd} ran")).1.clone();
    app.run("edit.undo", json!({})).unwrap();
    app.run(cmd, params).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *doc, "{what}: the committed stroke");
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pen-up while the thread is still painting: the release waits for the steps it was given, and
/// the commit has every point and replays to the same document.
#[test]
fn pen_up_while_the_thread_is_busy_commits_the_whole_stroke() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let what = format!("{tool:?}");
        let mut app = app_for(tool);
        let steps = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
        move_along(&mut app);
        let release = app.live_stroke.as_ref().unwrap().paint.hold();
        feed_last_point_without_waiting(&mut app);
        assert!(live_stroke_busy(&app), "{what}: the thread is still painting");
        // The slow step finishes while the release waits for it.
        let slow = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            release();
        });
        tool_event(&mut app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);
        slow.join().unwrap();
        assert!(app.live_stroke.is_none(), "{what}");
        assert!(!app.ui.status_error, "{what}: {}", app.ui.status);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "{what}: one edit");
        let cmd = if tool == Tool::Brush { stroke_command(&app, tool) } else { "paint.mixerBrush" };
        if tool == Tool::Brush {
            assert!(app.committed_stroke.is_some(), "the commit took the live stroke's pixels");
        }
        assert_commit_replays(&mut app, cmd, &what);
    }
}

/// Cancelling while the thread is mid-step (Esc with the Mixer Brush) doesn't wait for the step,
/// leaves the document and history alone, and the thread ends once the step is done.
#[test]
fn cancel_while_the_thread_is_busy_changes_nothing_and_the_thread_ends() {
    let mut app = app_for(Tool::MixerBrush);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
    move_along(&mut app);
    let paint = &app.live_stroke.as_ref().unwrap().paint;
    let (release, alive) = (paint.hold(), paint.thread_alive());
    feed_last_point_without_waiting(&mut app);
    assert!(live_stroke_busy(&app), "the thread is still painting");

    // The host cancels the gesture on Escape.
    testing::key(&mut app, egui::Key::Escape, M);
    tool_event(&mut app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);

    assert!(app.live_stroke.is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "the document is untouched");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(!app.session.journal.iter().any(|(id, _)| id == "paint.mixerBrush"));
    release();
    wait_until("the thread ends after the cancelled stroke", || !alive());
}

/// A step that panics on the thread ends the thread and loses the live stroke, but never the
/// document: the stroke painted only its own copy, so the document stays as it was, and the
/// release renders the whole stroke from the drag (one edit, the same pixels as running the
/// command again), without an error.
#[test]
fn a_step_that_panics_on_the_thread_leaves_the_document_and_the_release_paints_the_stroke() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let what = format!("{tool:?}");
        let mut app = app_for(tool);
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
        let paint = &app.live_stroke.as_ref().unwrap().paint;
        paint.panic_next_step();
        let alive = paint.thread_alive();
        for x in (30..=130).step_by(10) {
            tool_event(&mut app, ToolEvent::Move { x: f64::from(x), y: 30.0, pressure: 1.0 }, M);
        }
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{what}: no partial document");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{what}");
        assert!(!live_stroke_busy(&app), "{what}: a stopped thread asks for no more frames");

        tool_event(&mut app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);
        assert!(app.live_stroke.is_none(), "{what}");
        assert!(!app.ui.status_error, "{what}: {}", app.ui.status);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "{what}: one edit");
        if tool == Tool::Brush {
            assert!(app.committed_stroke.is_none(), "rendered again, not taken from the lost live stroke");
        }
        let cmd = if tool == Tool::Brush { stroke_command(&app, tool) } else { "paint.mixerBrush" };
        assert_commit_replays(&mut app, cmd, &what);
        wait_until("the thread ended", || !alive());
    }
}

/// Alpha of the active layer's pixel.
fn layer_alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

/// A Brush stroke released while its thread is still painting the pen's last point and holds the
/// commit, with the canvas not waiting for it: the stroke is pending. Returns what lets the thread go.
fn release_with_commit_pending(app: &mut PhotocraftApp) -> impl FnOnce() + Send + 'static {
    tool_event(app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
    move_along(app);
    let release = app.live_stroke.as_ref().unwrap().paint.hold();
    feed_last_point_without_waiting(app);
    app.stroke_commit_wait = Duration::ZERO;
    tool_event(app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);
    assert!(app.pending_stroke.is_some(), "the commit is still being made");
    release
}

/// Let go of the held commit after a moment, from another thread, as a slow composite finishing.
fn release_soon(release: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        release();
    })
}

/// While the commit is pending the document is untouched and the canvas shows the stroke; the next
/// frame after the thread is done lands the commit with the live stroke's pixels.
#[test]
fn a_pending_commit_leaves_the_document_alone_and_lands_in_a_later_frame() {
    let mut app = app_for(Tool::Brush);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_with_commit_pending(&mut app);
    assert!(app.live_stroke.is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "the document is unchanged");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    let (shown, key) = super::display_doc(&mut app, 0);
    assert!(!Arc::ptr_eq(&shown, &before) && key != 0, "the canvas shows the stroke, not the document");
    let flat = photocraft_compose::flatten(&shown);
    assert!(flat.get(60, 30)[0] > 0.5 && flat.get(60, 30)[1] < 0.5, "the stroke's red over the green");

    // Not done yet: a frame's poll lands nothing.
    settle_pending_stroke(&mut app, false);
    assert!(app.pending_stroke.is_some());
    release();
    wait_until("the commit lands", || {
        settle_pending_stroke(&mut app, false);
        app.pending_stroke.is_none()
    });
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one edit");
    assert!(app.committed_stroke.is_some(), "the commit took the live stroke's pixels");
    assert!(layer_alpha(&app, 60, 30) > 0.5);
    assert_eq!(super::display_doc(&mut app, 0).1, 0, "the canvas shows the document again");
    assert_commit_replays(&mut app, "paint.stroke", "pending commit");
}

/// A command that runs while the commit is pending waits for it and runs after it: undo undoes the
/// stroke, not what came before it.
#[test]
fn a_command_waits_for_the_pending_commit_and_acts_on_the_stroke() {
    let mut app = app_for(Tool::Brush);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_with_commit_pending(&mut app);
    let slow = release_soon(release);
    app.run("edit.undo", json!({})).unwrap();
    slow.join().unwrap();
    assert!(app.pending_stroke.is_none());
    assert_eq!(*app.session.active().unwrap().doc, *before, "the stroke was undone");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    app.run("edit.redo", json!({})).unwrap();
    assert!(layer_alpha(&app, 60, 30) > 0.5, "what was undone was the stroke");
}

/// A stroke that starts while the first one's commit is pending lands after it: the Eraser's column
/// through the first stroke takes it out where they cross, which only holds if it came second.
#[test]
fn a_stroke_started_while_another_is_pending_lands_after_it() {
    let mut app = app_for(Tool::Brush);
    let steps = app.session.active().unwrap().history.past_len();
    let release = release_with_commit_pending(&mut app);
    let slow = release_soon(release);
    app.ui.tool = Tool::Eraser;
    for (i, y) in [10.0, 30.0, 50.0].into_iter().enumerate() {
        let ev = if i == 0 { ToolEvent::Down { x: 60.0, y, pressure: 1.0 } } else { ToolEvent::Move { x: 60.0, y, pressure: 1.0 } };
        tool_event(&mut app, ev, M);
    }
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 50.0 }, M);
    slow.join().unwrap();
    settle_pending_stroke(&mut app, true);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 2, "both strokes are in the history");
    assert!(layer_alpha(&app, 100, 30) > 0.5, "the first stroke is there");
    assert!(layer_alpha(&app, 60, 30) < 0.1, "the second stroke's erasing is on top of it");
}

/// The thread panics while it makes the commit: the canvas notices, renders the stroke itself, and
/// nothing hangs.
#[test]
fn a_panic_while_the_commit_is_made_falls_back_to_rendering_the_stroke() {
    let mut app = app_for(Tool::Brush);
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
    // To the pen's last point, all painted: the panic comes in the commit, not in a step.
    for x in (30..=130).step_by(10) {
        tool_event(&mut app, ToolEvent::Move { x: f64::from(x), y: 30.0, pressure: 1.0 }, M);
    }
    let paint = &app.live_stroke.as_ref().unwrap().paint;
    let (release, alive) = (paint.hold(), paint.thread_alive());
    paint.panic_next_step();
    app.stroke_commit_wait = Duration::ZERO;
    tool_event(&mut app, ToolEvent::Up { x: 130.0, y: 30.0 }, M);
    assert!(app.pending_stroke.is_some());
    release();
    wait_until("the thread ended", || !alive());
    wait_until("the stroke lands", || {
        settle_pending_stroke(&mut app, false);
        app.pending_stroke.is_none()
    });
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one edit");
    assert!(app.committed_stroke.is_none(), "rendered again, not taken from the lost thread");
    assert_commit_replays(&mut app, "paint.stroke", "panicked commit");
}
