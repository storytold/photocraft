use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing::in_gesture;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use craft::draw::ui::VectorMode;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.extras.snap = false;
    app
}

fn down(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 50.0, y: 35.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 90.0, y: 80.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(in_gesture(app));
}

#[test]
fn freeform_and_path_selection_releases_cannot_edit_a_different_document() {
    for tool in [Tool::FreeformPen, Tool::PathSelection] {
        let mut app = app(tool);
        app.run("path.set", json!({"path": {"subpaths": [{"knots": [[20, 20], [50, 35], [90, 80]]}]}})).unwrap();
        let original = app.session.active().unwrap().doc.work_path.clone();
        down(&mut app);
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        let revision = app.session.active().unwrap().revision;
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, Modifiers::NONE);
        assert!(!in_gesture(&app));
        assert!(app.session.active().unwrap().doc.work_path.is_none());
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.documents()[0].doc.work_path, original);
    }
}

#[test]
fn freeform_release_cancels_after_revision_or_target_change() {
    for change in ["revision", "selected", "mode"] {
        let mut app = app(Tool::FreeformPen);
        down(&mut app);
        match change {
            "revision" => {
                app.run("path.set", json!({"path": {"subpaths": [{"knots": [[10, 10], [15, 15]]}]}})).unwrap();
            }
            "selected" => app.ui.selected_path = Some("Another path".into()),
            // The app default is Path, so this assignment changes the mode.
            _ => app.ui.tool_options.vector_mode = VectorMode::Shape,
        }
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, Modifiers::NONE);
        assert!(!in_gesture(&app));
        assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        assert_eq!(app.session.active().unwrap().revision, revision);
    }
}

#[test]
fn freeform_valid_release_is_one_undoable_path_edit() {
    let mut app = app(Tool::FreeformPen);
    let history = app.session.active().unwrap().history.entries().len();
    down(&mut app);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert_eq!(app.session.active().unwrap().history.entries().len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
}
