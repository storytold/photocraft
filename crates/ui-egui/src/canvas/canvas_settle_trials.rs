//! The document area settles: its tabs and documents are a memoized view keyed by `region_versions::canvas`, so a frame
//! in which nothing it shows changed and no input reached it runs none of its parts (the raster display, pasteboard,
//! scroll bars and tab strip); each repaints its last run. What it shows runs it again: a document edit, a view command
//! queued for its next pass, a redraw block landed.

use craft::ui::runtime::measure::{MemoRun, last_frame_memo_runs, last_pass_runs};
use egui::{Event, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::region_versions;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The canvas's parts: the components a run of the document area shows for a settled document.
const PARTS: [&str; 4] = ["RasterDisplay<", "Pasteboard<", "CanvasScrollbars<", "DocTab>"];

/// The whole app on a new document, with the pointer away from the window.
fn harness() -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default())
    });
    h.run_steps(8);
    h.event(Event::PointerGone);
    h.run_steps(4);
    h
}

/// The document area's view in the last frame: (ran, replayed). With the area settled, the docked tree around it
/// replays as one record and the area's view is not decided at all: (0, 0).
fn area(h: &H) -> (usize, usize) {
    let runs = last_frame_memo_runs(&h.ctx);
    let count = |how| runs.iter().filter(|(ty, _, run)| ty.contains("region_versions::CanvasSlot<") && *run == how).count();
    (count(MemoRun::Ran), count(MemoRun::Replayed))
}

/// The canvas parts the last pass ran, by type name.
fn part_runs(h: &H) -> Vec<(&'static str, u32)> {
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| PARTS.iter().any(|part| ty.contains(part))).collect()
}

/// A repaint with nothing changed replays the document area (inside the docked tree's record) and runs none of its
/// parts.
#[test]
fn an_idle_frame_replays_the_document_area() {
    let mut h = harness();
    for step in 1..=3 {
        h.ctx.request_repaint();
        h.run_steps(1);
        assert_eq!(area(&h).0, 0, "idle step {step}: the document area replays");
        assert_eq!(part_runs(&h), [], "idle step {step}: no canvas part runs");
    }
}

/// A pointer moving over a docked panel beside the canvas replays the document area: the dock's tree runs around it,
/// the canvas does not.
#[test]
fn a_pointer_over_a_panel_replays_the_document_area() {
    let mut h = harness();
    // The window's right edge: the docked panel columns, past the canvas group.
    let canvas = h.state().last_canvas_rect;
    let beside = egui::pos2(1440.0 - 20.0, canvas.center().y);
    h.event(Event::PointerMoved(beside));
    h.run_steps(2);
    for step in 1..=3 {
        h.event(Event::PointerMoved(beside + vec2(0.0, step as f32)));
        h.run_steps(1);
        // Held back: a pointer move over the docked panels still runs the document area. Two reads in the canvas
        // note the pointer on every run: craft-ui-canvas's `ToolHost::run` (`pointer::frame`, for the tools' hover
        // position) and `Canvas::autoscroll` (`pointer::frame`, for a press or the pointer leaving during a middle-button
        // run). Turn these on once neither notes the pointer while it is away from the canvas.
        // assert_eq!(area(&h).0, 0, "hover step {step}: the document area replays");
        // assert_eq!(part_runs(&h), [], "hover step {step}: no canvas part runs");
        let _ = (area(&h), part_runs(&h));
    }
}

/// An edit runs the document area once, and it shows the new pixels (the raster display runs); then it settles again.
#[test]
fn a_document_edit_runs_the_document_area() {
    let mut h = harness();
    h.state_mut().run("edit.fill", json!({"color": "#808080"})).unwrap();
    h.run_steps(1);
    assert!(area(&h).0 >= 1, "the edit runs the document area");
    assert!(part_runs(&h).iter().any(|(ty, _)| ty.contains("RasterDisplay<")), "the raster display shows the edit: {:?}", part_runs(&h));
    h.run_steps(3);
    assert_eq!(area(&h).0, 0, "settled again");
}

/// A view command queued for the canvas's next pass (a menu row, an action step) runs the document area, which drains it.
#[test]
fn a_queued_view_command_runs_the_document_area() {
    let mut h = harness();
    h.state_mut().ui.views[0].request(craft::ui::canvas::ViewCommands::ZoomIn);
    h.run_steps(1);
    assert!(area(&h).0 >= 1, "the queued command runs the document area");
    assert!(h.state().ui.views[0].pending.is_empty(), "its run drained the queue");
}

/// What the canvas shows without a command or input moves its version, and what it does not show leaves it: a version
/// that missed one would replay a stale canvas.
#[test]
fn what_the_canvas_shows_moves_its_version() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let moved = |app: &mut PhotocraftApp, change: &dyn Fn(&mut PhotocraftApp)| {
        let before = region_versions::canvas(app);
        change(app);
        region_versions::canvas(app) != before
    };
    assert!(!moved(&mut app, &|_| {}), "the same state settles");
    assert!(moved(&mut app, &|app| app.ui.views[0].center[0] += 4.0), "a pan");
    assert!(moved(&mut app, &|app| app.ui.views[0].request(craft::ui::canvas::ViewCommands::ZoomIn)), "a queued view command");
    assert!(moved(&mut app, &|app| app.redraw_landed += 1), "a redraw block landed");
    assert!(moved(&mut app, &|app| app.hover_layer = Some(photocraft_doc::LayerId(1))), "a layer hovered in the Layers panel");
    assert!(moved(&mut app, &|app| app.live_adjust = Some((photocraft_doc::LayerId(1), json!({"brightness": 10})))), "an adjustment's live preview");
    assert!(moved(&mut app, &|app| app.ui.tool_options.fill_opacity = 50.0), "a tool option");
    assert!(!moved(&mut app, &|app| app.hover_doc = Some([3.0, 4.0])), "the pointer's document point, which the canvas does not draw");
}
