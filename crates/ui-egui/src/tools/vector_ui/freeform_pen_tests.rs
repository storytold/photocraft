//! The Freeform Pen through the canvas pass: a freehand run and ⌥-clicked straight segments append
//! to the path the stroke began on, once, and a stale target, source, revision or document cancels
//! the stroke instead of finishing it elsewhere.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, testing, tool_event};
use crate::state::Tool;
use craft::draw::ui::VectorMode;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.extras.snap = false;
    app
}

/// An app drawing into the saved path "Saved drawing", with a Work Path beside it.
fn freeform_app() -> Box<PhotocraftApp> {
    let mut app = app();
    for name in ["work", "Saved drawing"] {
        app.run("path.set", json!({"name": name, "path":{"subpaths":[{"knots":[[20,20],[220,20]]}]}})).unwrap();
    }
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app.ui.selected_path = Some("Saved drawing".into());
    app
}

/// A ⌥-click at the document point `(x, y)`, with a drag of `drag_by` px when given.
fn alt_click_raw(app: &mut PhotocraftApp, x: f64, y: f64, drag_by: Option<f64>) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT);
    if let Some(d) = drag_by {
        tool_event(app, ToolEvent::Move { x: x + d, y: y + d, pressure: 1.0 }, Modifiers::ALT);
    }
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::ALT);
}

/// Whether the Freeform Pen has a trace open: after a release with ⌥ held the trace stays open between presses.
fn tracing(app: &PhotocraftApp) -> bool {
    app.canvas_tools.vector.freeform.draft.is_open()
}

/// A freehand run released with ⌥ held, then a ⌥-click: the fitted path stays in progress.
fn freeform_run_and_straight_click(app: &mut PhotocraftApp) {
    testing::press(app, [50.0, 50.0], Modifiers::NONE);
    testing::move_to(app, [90.0, 40.0], Modifiers::NONE);
    testing::move_to(app, [120.0, 50.0], Modifiers::NONE);
    testing::release(app, [120.0, 50.0], Modifiers::ALT);
    assert!(tracing(app), "Alt-held release keeps the fitted path in progress");
    testing::press(app, [180.0, 100.0], Modifiers::ALT);
    testing::release(app, [180.0, 100.0], Modifiers::ALT);
    assert!(tracing(app));
}

#[test]
fn real_freeform_freehand_and_alt_straight_segments_append_to_the_owned_saved_path_once() {
    let mut app = freeform_app();
    let original = app.session.active().unwrap().doc.clone();
    let n = app.session.active().unwrap().history.past_len();
    freeform_run_and_straight_click(&mut app);
    testing::modifiers(&mut app, Modifiers::NONE);
    assert!(!tracing(&app) && app.ui.pen.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.work_path, original.work_path);
    let path = &st.doc.paths[0].path;
    assert_eq!(path.subpaths.len(), 2);
    assert_eq!(path.subpaths[0], original.paths[0].path.subpaths[0]);
    let knots = &path.subpaths[1].knots;
    assert_eq!(knots.last().unwrap().anchor, photocraft_geom::Point::new(180.0, 100.0));
    assert_eq!(knots.last().unwrap().in_ctrl, knots.last().unwrap().anchor);
    assert_eq!(knots[knots.len() - 2].out_ctrl, knots[knots.len() - 2].anchor);
    assert_eq!(st.history.past_len(), n + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.paths, original.paths);
}

#[test]
fn real_freeform_stale_target_source_revision_and_document_cancel_without_finishing_elsewhere() {
    for change in ["target", "source", "revision", "document"] {
        let mut app = freeform_app();
        freeform_run_and_straight_click(&mut app);
        match change {
            "target" => app.ui.selected_path = Some("work".into()),
            "source" => {
                let st = app.session.active_mut().unwrap();
                st.doc = std::sync::Arc::new((*st.doc).clone());
            }
            "revision" => app.session.active_mut().unwrap().revision += 1,
            "document" => {
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            }
            _ => panic!("unexpected stale-gesture test case: {change}"),
        }
        let source = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        testing::move_to(&mut app, [200.0, 120.0], Modifiers::ALT);
        testing::release(&mut app, [200.0, 120.0], Modifiers::NONE);
        testing::modifiers(&mut app, Modifiers::NONE);
        assert!(!tracing(&app) && app.ui.pen.is_none(), "{change}: no stale trace remains");
        assert!(std::sync::Arc::ptr_eq(&source, &app.session.active().unwrap().doc), "{change}: late input changed document pixels or paths");
        assert_eq!(app.session.active().unwrap().history.past_len(), n, "{change}");
    }
}

#[test]
fn another_freeform_trace_cannot_replace_a_failed_owned_draft_or_retry_it_automatically() {
    let mut app = app();
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("refused path fixture".into())) } else { Ok(()) });
    for [x, y] in [[40.0, 60.0], [100.0, 60.0], [140.0, 80.0]] {
        alt_click_raw(&mut app, x, y, None);
    }
    testing::modifiers(&mut app, Modifiers::NONE);
    let failed = app.ui.pen.clone().expect("invalid fitted trace must remain available");
    assert!(failed.commit_failed && failed.owner.is_some());
    assert!(!tracing(&app));
    let source = app.session.active().unwrap().doc.clone();
    let n = app.session.active().unwrap().history.past_len();
    let journal = app.session.journal.len();
    for middle in [[180.0, 90.0], [220.0, 90.0]] {
        for [x, y] in [[160.0, 120.0], middle, [260.0, 140.0]] {
            alt_click_raw(&mut app, x, y, Some(10.0));
            assert!(!tracing(&app), "pending fitted draft owns the next drawing attempt");
        }
        testing::modifiers(&mut app, Modifiers::NONE);
        assert_eq!(app.ui.pen.as_ref(), Some(&failed));
        assert!(std::sync::Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        assert_eq!(app.session.journal.len(), journal, "drawing must not retry the failed engine command");
    }
}

#[test]
fn shift_drag_constrains_freeform_pen_to_a_straight_line() {
    let mut app = app();
    app.ui.tool_options.vector_mode = VectorMode::Path;
    let shift = Modifiers::SHIFT;
    testing::press(&mut app, [10.0, 60.0], shift);
    for (x, y) in [(30.0, 63.0), (50.0, 55.0), (80.0, 68.0), (100.0, 52.0)] {
        testing::move_to(&mut app, [x, y], shift);
    }
    testing::release(&mut app, [100.0, 52.0], shift);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("the stroke committed a work path");
    let knots = &work.subpaths[0].knots;
    assert!(knots.len() >= 2, "{knots:?}");
    let y0 = knots[0].anchor.y;
    assert!(knots.iter().all(|k| (k.anchor.y - y0).abs() < 1e-6), "⇧ keeps the freehand segment horizontal: {knots:?}");
}
