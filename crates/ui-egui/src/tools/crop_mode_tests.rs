//! The app's rules around the Crop tool's image mode: the view commands that read a pending crop's turn,
//! the view of a dropped crop, and the P and H keys.

use egui::Modifiers;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;

use crate::canvas::testing::{ToolEvent, frame, tool_event};
use crate::state::Tool;
use crate::{PhotocraftApp, crop_ui};

const NONE: Modifiers = Modifiers::NONE;

/// A 200 × 100 document with the Crop tool picked and its default frame shown.
fn app() -> Box<PhotocraftApp> {
    let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.sync_views();
    app.ui.tool = Tool::Crop;
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    frame(&mut app);
    app
}

/// A drag through document points.
fn drag(app: &mut PhotocraftApp, points: &[[f64; 2]]) {
    let at = |p: [f64; 2]| (p[0], p[1]);
    let (x, y) = at(points[0]);
    // The frame's handles take presses from the pass that drew them: a frame set since the last pass is shown first.
    frame(app);
    // The pointer is over a handle before it presses it.
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, NONE);
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, NONE);
    for p in &points[1..] {
        let (x, y) = at(*p);
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, NONE);
    }
    let (x, y) = at(points[points.len() - 1]);
    tool_event(app, ToolEvent::Up { x, y }, NONE);
}

/// Pulls the default frame's bottom right handle in: a pending frame of 100 × 70 at the top left.
fn pend_a_frame(app: &mut PhotocraftApp) {
    drag(app, &[[200.0, 100.0], [150.0, 80.0], [100.0, 70.0]]);
    assert!(crop_ui::pending(app), "the handle drag leaves a pending frame");
}

/// Turns the image by dragging outside the frame, from its right side to below it.
fn turn_the_view(app: &mut PhotocraftApp) {
    pend_a_frame(app);
    drag(app, &[[150.0, 35.0], [140.0, 70.0], [50.0, 135.0]]);
    assert!(crop_ui::turns_view(app), "dragging outside the frame turns the view in image mode");
}

/// A second 300 × 150 document, the first active, its view upright and the second turned 15°.
fn with_a_turned_second_document(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "view.rotateView", json!({"angle": 15.0})).unwrap();
    assert!(app.session.set_active(0));
    app.sync_views();
}

/// Window › Arrange › Match Rotation copies the view's own turn, not a pending crop's; Match Zoom and Match
/// Location report the camera as it is. None of them touches another document's view beyond what it copies.
#[test]
fn match_rotation_zoom_and_location_leave_a_pending_crops_turn_out() {
    for id in ["window.arrange.matchRotation", "window.arrange.matchAll", "window.arrange.matchZoom", "window.arrange.matchLocation"] {
        let mut app = app();
        with_a_turned_second_document(&mut app);
        turn_the_view(&mut app);
        let ctx = app.ctx().clone();
        let r = crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
        if id.ends_with("matchRotation") || id.ends_with("matchAll") {
            assert_eq!(r["rotation"], 0.0, "{id}: {r}");
            assert_eq!(app.ui.views[1].rotation, 0.0, "{id}: the other document gets no crop turn");
        } else {
            assert_eq!(app.ui.views[1].rotation, 15.0, "{id}: the other view keeps its rotation");
        }
        assert!(crop_ui::turns_view(&app), "{id}: the crop still turns its own view");
    }
}

/// Switching documents drops a turned pending crop and turns its own document's view upright, as Esc would, not
/// only the view of the document switched to.
#[test]
fn a_dropped_crop_turns_its_own_documents_view_upright() {
    let mut app = app();
    with_a_turned_second_document(&mut app);
    turn_the_view(&mut app);
    assert_ne!(app.ui.views[0].rotation, 0.0, "the view turned with the crop");
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "window.nextDocument", json!({})).unwrap();
    app.sync_views();
    frame(&mut app);
    assert!(!crop_ui::pending(&app), "the turned crop is dropped with its document");
    assert_eq!(app.ui.views[0].rotation, 0.0, "its own document's view is upright again");
}

/// Picking another tool commits a turned crop, and the view the crop turned goes upright with it: the turn is in the
/// cropped pixels now, not in the view.
#[test]
fn another_tool_commits_a_turned_crop_and_turns_the_view_upright() {
    let mut app = app();
    turn_the_view(&mut app);
    assert_ne!(app.ui.views[0].rotation, 0.0, "the view turned with the crop");
    app.ui.tool = Tool::Brush;
    frame(&mut app);
    frame(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo_label(), Some("Crop"), "leaving the tool commits the turned crop");
    assert_ne!(st.doc.size, Size::new(200, 100));
    assert_eq!(app.ui.views[0].rotation, 0.0, "the view is upright again");
}
