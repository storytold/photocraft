//! The measured Freeform Pen Work Path gestures in #12836.

use super::*;
use crate::canvas::testing;
use crate::direct_select::PathRef;
use crate::state::Tool;
use craft::behavior::{Cursor, CursorBadge, PaintedCursor};
use craft::draw::ui::VectorMode;
use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

fn work_path(app: &PhotocraftApp) -> &Path {
    app.session.active().unwrap().doc.work_path.as_ref().unwrap()
}

fn drag(app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]) {
    testing::press(app, a, Modifiers::NONE);
    for i in 1..50 {
        let t = f64::from(i) / 50.0;
        testing::move_to(app, [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])], Modifiers::NONE);
    }
    testing::release(app, b, Modifiers::NONE);
}

fn shows_continue(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    testing::move_to(app, point, Modifiers::NONE);
    matches!(
        app.canvas_tools.last_response.as_ref().and_then(|response| response.cursor.as_ref()),
        Some(Cursor::Painted(PaintedCursor::Tool { badge: Some(CursorBadge::Continue), .. }))
    )
}

fn with_open_work_path() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":800,"height":600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app.ui.extras.snap = false;
    drag(&mut app, [100.0, 400.0], [500.0, 400.0]);
    assert_eq!(work_path(&app).subpaths.len(), 1);
    assert_eq!(work_path(&app).subpaths[0].knots.len(), 2);
    app
}

#[test]
fn selected_work_path_endpoint_continues_as_one_named_undo_step() {
    let mut app = with_open_work_path();
    assert!(!shows_continue(&mut app, [500.0, 400.0]));
    testing::click(&mut app, [300.0, 400.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Work), "temporary Direct Selection picks the Work Path");
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]), "the open line segment is selected");
    assert!(!shows_continue(&mut app, [530.0, 390.0]));
    let continue_at_end = shows_continue(&mut app, [500.0, 400.0]);
    let before = work_path(&app).clone();
    let history = app.session.active().unwrap().history.past_len();
    drag(&mut app, [500.0, 400.0], [700.0, 300.0]);

    assert_eq!(work_path(&app).subpaths.len(), 1, "the second trace continues the existing subpath");
    assert!(continue_at_end, "the Pen's continue badge appears at the selected end");
    let subpath = &work_path(&app).subpaths[0];
    assert!(!subpath.closed);
    assert_eq!(subpath.knots.len(), 3);
    assert_eq!(subpath.knots[0], before.subpaths[0].knots[0]);
    let join = &subpath.knots[1];
    assert_eq!(join.anchor, before.subpaths[0].knots[1].anchor);
    assert_eq!(join.out_ctrl, join.anchor);
    let end = subpath.knots[2].anchor;
    assert!((end.x - 700.0).abs() < 1.0 && (end.y - 300.0).abs() < 1.0, "{end:?}");
    let h = &app.session.active().unwrap().history;
    assert_eq!(h.past_len(), history + 1);
    assert_eq!(h.undo_label(), Some("Freeform Pen"));
    assert!(app.session.undo());
    assert_eq!(work_path(&app), &before);
}

#[test]
fn unselected_work_path_endpoint_consumes_no_history_and_empty_space_draws() {
    let mut app = with_open_work_path();
    let before = work_path(&app).clone();
    let history = app.session.active().unwrap().history.past_len();
    drag(&mut app, [500.0, 400.0], [700.0, 300.0]);
    assert_eq!(work_path(&app), &before, "an unselected endpoint does not begin a trace");
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    drag(&mut app, [530.0, 390.0], [700.0, 300.0]);
    assert_eq!(work_path(&app).subpaths.len(), 2);
    assert_eq!(work_path(&app).subpaths[0], before.subpaths[0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
}

#[test]
fn a_single_anchor_is_not_a_measured_continuation_endpoint() {
    let mut app = with_open_work_path();
    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"knots":[[100,400]]}]}})).unwrap();
    let before = work_path(&app).clone();
    assert_eq!(before.subpaths[0].knots.len(), 1);
    assert!(!shows_continue(&mut app, [100.0, 400.0]));
    drag(&mut app, [100.0, 400.0], [300.0, 300.0]);
    assert_eq!(work_path(&app).subpaths.len(), 2);
    assert_eq!(work_path(&app).subpaths[0], before.subpaths[0]);
}

#[test]
fn selecting_an_earlier_anchor_or_segment_leaves_the_end_unselected() {
    let mut app = with_open_work_path();
    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":false,"knots":[[100,400],[300,400],[500,400]]}]}})).unwrap();
    assert!(!work_path(&app).subpaths[0].closed);
    let before = work_path(&app).clone();
    let history = app.session.active().unwrap().history.past_len();
    for point in [[100.0, 400.0], [200.0, 400.0]] {
        testing::click(&mut app, point, Modifiers::COMMAND);
        assert_eq!(app.ui.direct_selection.target, Some(PathRef::Work));
        assert!(!shows_continue(&mut app, [500.0, 400.0]));
        drag(&mut app, [500.0, 400.0], [700.0, 300.0]);
        assert_eq!(work_path(&app), &before);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
    testing::click(&mut app, [400.0, 400.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.segment, Some([0, 1]));
    assert!(shows_continue(&mut app, [500.0, 400.0]));
    drag(&mut app, [500.0, 400.0], [700.0, 300.0]);
    assert_eq!(work_path(&app).subpaths.len(), 1);
    assert_eq!(work_path(&app).subpaths[0].knots.len(), 4);
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
}
