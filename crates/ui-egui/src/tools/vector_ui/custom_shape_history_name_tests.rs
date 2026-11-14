//! #13461: a Custom Shape tool drag is one History step named "Custom Shape Tool" in Shape mode
//! (a new layer, or a combine / subtract drag into the active shape layer) and in Path mode;
//! `shape.presets.place` run without the tool keeps "New Shape Layer". A Line tool combine drag
//! is likewise named "Line Tool", as #13244 measured.

use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_step(app: &PhotocraftApp) -> Option<&str> {
    app.session.active().unwrap().history.undo_label()
}

/// Drag the Custom Shape tool and check it added exactly one "Custom Shape Tool" step.
fn drag_is_one_custom_shape_step(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let n = steps(app);
    finish_shape(app, Tool::CustomShape, start, end, egui::Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(steps(app), n + 1);
    assert_eq!(last_step(app), Some("Custom Shape Tool"));
}

#[test]
fn shape_mode_drags_are_one_step_named_custom_shape_tool() {
    let mut app = app();
    let layers = app.session.active().unwrap().doc.layers.len();
    drag_is_one_custom_shape_step(&mut app, [400.0, 300.0], [600.0, 400.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "a new shape layer");
    for op in ["combine", "subtract"] {
        app.ui.tool_options.shape_path_op = op.into();
        drag_is_one_custom_shape_step(&mut app, [450.0, 320.0], [520.0, 380.0]);
    }
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "joined the active shape layer");
}

#[test]
fn each_path_mode_drag_is_one_step_named_custom_shape_tool() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    drag_is_one_custom_shape_step(&mut app, [100.0, 100.0], [200.0, 200.0]);
    drag_is_one_custom_shape_step(&mut app, [400.0, 300.0], [600.0, 400.0]);
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 2);
}

#[test]
fn placing_a_shape_without_the_tool_keeps_new_shape_layer() {
    let mut app = app();
    let preset = app.ui.presets_ui.shape().to_string();
    app.session.execute("shape.presets.place", json!({"preset": preset, "rect": [10, 10, 100, 100]})).unwrap();
    assert_eq!(last_step(&app), Some("New Shape Layer"));
}

#[test]
fn a_line_combine_drag_into_its_layer_is_named_line_tool() {
    let mut app = app();
    finish_shape(&mut app, Tool::Line, [100.0, 100.0], [300.0, 100.0], egui::Modifiers::NONE);
    let layers = app.session.active().unwrap().doc.layers.len();
    app.ui.tool_options.shape_path_op = "combine".into();
    let n = steps(&app);
    finish_shape(&mut app, Tool::Line, [100.0, 200.0], [300.0, 200.0], egui::Modifiers::NONE);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(last_step(&app), Some("Line Tool"));
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "joined the line layer");
}

#[test]
fn a_combine_without_a_tool_name_keeps_add_shape() {
    let mut app = app();
    let preset = app.ui.presets_ui.shape().to_string();
    app.session.execute("shape.presets.place", json!({"preset": preset, "rect": [10, 10, 100, 100]})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let place = json!({"preset": preset, "rect": [50, 50, 100, 100], "addTo": id, "op": "combine"});
    app.session.execute("shape.presets.place", place).unwrap();
    assert_eq!(last_step(&app), Some("Add Shape"));
}
