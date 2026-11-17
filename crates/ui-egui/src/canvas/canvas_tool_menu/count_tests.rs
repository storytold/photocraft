//! #11772: a right-click on the canvas with the Count tool, on a marker or off one, opens
//! Photoshop's generic layer menu. Its rows are enabled as the Layers panel menu enables them for
//! the active layer, and the right-click itself adds no marker and no history step.

use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Reply, handle};

/// A 64 × 48 document with a second pixel layer (active) and one count marker at 20,20.
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.run("count.add", json!({"x": 20, "y": 20})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Count;
    let ctx = app.ctx().clone();
    (app, ctx)
}

pub(crate) fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// A secondary click at document point (x, y) over the control channel, delivered to the canvas
/// (`testing::control`).
pub(crate) fn right_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    let r = crate::canvas::testing::control(
        app,
        "ui.pointer",
        json!({"button": "secondary", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}),
    );
    assert_eq!(r["ok"], true, "{r}");
}

/// The open menu's rows as `ui.inspect` lists them: ids, with "―" for separators.
pub(crate) fn listed(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<String> {
    let inspected = call(app, ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries.iter().map(|e| e["id"].as_str().map_or_else(|| "―".to_string(), str::to_string)).collect()
}

fn markers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().doc.measurement.count_groups.iter().flat_map(|g| g.points.clone()).collect()
}

/// The rows of the layer menu the Count tool opens.
const WANT: [&str; 17] = [
    "layer.layerStyle.blendingOptions",
    "―",
    "layer.duplicate",
    "layer.delete",
    "layer.new.groupFromLayers",
    "―",
    "layer.smartObjects.convertToSmartObject",
    "layer.smartObjects.convertToLayers",
    "―",
    "layer.rasterize.layer",
    "layer.rasterize.layerStyle",
    "―",
    "edit.freeTransform",
    "―",
    "select.colorRange",
    "select.loadSelection",
    "select.reselect",
];

#[test]
fn the_right_click_adds_no_marker_and_no_history_step() {
    for (x, y) in [(50.0, 40.0), (20.0, 20.0)] {
        let (mut app, _ctx) = app();
        let (before, steps) = (markers(&app), app.session.active().unwrap().history.past_len());
        right_click(&mut app, x, y);
        assert_eq!(markers(&app), before, "({x}, {y}): no marker added or moved");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "({x}, {y}): no history step");
        assert!(app.ui.canvas_tool_menu.is_some(), "({x}, {y}): the menu is open");
    }
}

#[test]
fn rows_are_enabled_as_the_layers_panel_menu_enables_them() {
    let (mut app, _ctx) = app();
    right_click(&mut app, 50.0, 40.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for id in WANT.iter().filter(|id| **id != "―") {
        assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id}");
    }
    // A pixel layer: Convert to Layers (a smart object's) and Reselect (no earlier selection) are
    // greyed; Delete Layer and Free Transform are not.
    assert!(!entry_enabled(&app, &menu, "layer.smartObjects.convertToLayers"));
    assert!(!entry_enabled(&app, &menu, "select.reselect"));
    assert!(entry_enabled(&app, &menu, "layer.delete") && entry_enabled(&app, &menu, "edit.freeTransform"));
    // After a selection is made and cleared, Reselect is enabled.
    app.ui.canvas_tool_menu = None;
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    right_click(&mut app, 50.0, 40.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(entry_enabled(&app, &menu, "select.reselect"), "Reselect after a cleared selection");
}
