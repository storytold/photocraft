//! #13229: Create Ellipse (a click with the Ellipse tool) opens with the active ellipse layer's
//! size, or 100 × 100 with a pixel layer active or in a new document, whatever the last OK was. From
//! Center starts unchecked (the tool has no From Center option yet). Another shape layer active
//! keeps today's remembered values, and the other tools' dialogs are unchanged.

use craft::ui::dialog::{DialogId, StackDialog as _};
use serde_json::{Value, json};

use super::*;
use crate::canvas::testing;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    app
}

/// A click with `tool` at `at`; the Create dialog it opens.
fn click(app: &mut PhotocraftApp, tool: Tool, at: [f64; 2]) -> DialogId {
    app.ui.tool = tool;
    testing::click(app, at, egui::Modifiers::NONE);
    crate::dialogs::testing::show(app);
    app.framework.dialogs.get::<CreateShapeDialog>().expect("a Create Shape dialog").id()
}

/// The dialog's width, height and From Center.
fn shown(app: &PhotocraftApp) -> (f64, f64, bool) {
    let f: Value = app.framework.dialogs.get::<CreateShapeDialog>().and_then(|d| d.json_fields()).expect("fields");
    (f["width"].as_f64().unwrap(), f["height"].as_f64().unwrap(), f["fromCenter"].as_bool().unwrap())
}

/// OK with `width` × `height` and From Center on, so the dialog remembers them.
fn ok_with(app: &mut PhotocraftApp, id: DialogId, width: f64, height: f64) {
    let ctx = app.ctx().clone();
    for (k, v) in [("width", json!(width)), ("height", json!(height)), ("fromCenter", json!(true))] {
        app.framework.dialogs.set_json_field(&ctx, id, k, v).unwrap();
    }
    crate::dialogs::testing::confirm(app, id).unwrap();
}

#[test]
fn the_active_ellipse_layers_size() {
    let mut app = app();
    app.run("shape.create", json!({"kind": "ellipse", "rect": [40, 40, 200, 100], "fill": "#3366aa"})).unwrap();
    app.sync_views();
    click(&mut app, Tool::EllipseShape, [330.0, 250.0]);
    assert_eq!(shown(&app), (200.0, 100.0, false));
}

#[test]
fn a_pixel_layer_or_a_new_document_opens_100_by_100_whatever_the_last_ok() {
    let mut app = app();
    let id = click(&mut app, Tool::EllipseShape, [20.0, 20.0]);
    ok_with(&mut app, id, 150.0, 80.0);
    // The ellipse it drew is the active layer now: back to a pixel layer.
    let paint = app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "Paint").unwrap().id.0;
    app.run("layer.select", json!({"layer": paint})).unwrap();
    app.sync_views();
    click(&mut app, Tool::EllipseShape, [300.0, 200.0]);
    assert_eq!(shown(&app), (100.0, 100.0, false), "a pixel layer");
    app.run("file.new", json!({"width": 300, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    click(&mut app, Tool::EllipseShape, [50.0, 50.0]);
    assert_eq!(shown(&app), (100.0, 100.0, false), "a new document");
}

#[test]
fn another_shape_layer_keeps_the_remembered_values() {
    let mut app = app();
    let id = click(&mut app, Tool::EllipseShape, [20.0, 20.0]);
    ok_with(&mut app, id, 150.0, 80.0);
    app.run("shape.create", json!({"kind": "rect", "rect": [200, 150, 60, 40], "fill": "#3366aa"})).unwrap();
    app.sync_views();
    click(&mut app, Tool::EllipseShape, [350.0, 280.0]);
    assert_eq!(shown(&app), (150.0, 80.0, true), "not measured: today's remembered values");
}

#[test]
fn the_rectangle_dialog_is_unchanged() {
    let mut app = app();
    let id = click(&mut app, Tool::Rectangle, [20.0, 20.0]);
    ok_with(&mut app, id, 150.0, 80.0);
    let paint = app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "Paint").unwrap().id.0;
    app.run("layer.select", json!({"layer": paint})).unwrap();
    app.sync_views();
    click(&mut app, Tool::Rectangle, [300.0, 200.0]);
    assert_eq!(shown(&app), (150.0, 80.0, true), "the Rectangle keeps its remembered values");
}
