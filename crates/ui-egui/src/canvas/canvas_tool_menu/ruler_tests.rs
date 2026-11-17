//! #12036: a right-click on the canvas with the Ruler, on the measured line or off it, opens the
//! generic layer menu, with its rows enabled as the Layers panel menu enables them. The
//! right-click leaves the ruler and the history as they were.

use serde_json::json;

use super::count_tests::{call, listed, right_click};
use super::*;

/// A 64 × 48 document with a second pixel layer (active), a ruler from 10,10 to 50,30 and the
/// Ruler tool.
fn app(protractor: Option<[f64; 2]>) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [10, 10], "end": [50, 30], "protractor": protractor})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Ruler;
    (app, egui::Context::default())
}

fn ruler(app: &PhotocraftApp) -> Option<photocraft_doc::Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

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
fn a_right_click_on_or_off_the_line_opens_the_layer_menu_in_order() {
    for (x, y, what) in [(30.0, 20.0, "the line"), (10.0, 40.0, "away from it"), (50.0, 30.0, "an end")] {
        let (mut app, ctx) = app(None);
        right_click(&mut app, &ctx, x, y);
        assert_eq!(listed(&mut app, &ctx), WANT, "on {what}");
        let menu = app.ui.canvas_tool_menu.as_ref().expect("the menu");
        let labels: Vec<&str> = rows(menu).iter().flatten().map(|(l, _)| *l).collect();
        assert_eq!(labels.first(), Some(&"Blending Options…"));
        assert_eq!(labels.last(), Some(&"Reselect"));
    }
}

#[test]
fn the_right_click_leaves_the_ruler_and_the_history_as_they_were() {
    for protractor in [None, Some([10.0, 40.0])] {
        for (x, y) in [(30.0, 20.0), (10.0, 10.0), (60.0, 5.0)] {
            let (mut app, ctx) = app(protractor);
            let (before, steps) = (ruler(&app), app.session.active().unwrap().history.past_len());
            right_click(&mut app, &ctx, x, y);
            assert_eq!(ruler(&app), before, "({x}, {y}), {protractor:?}: the ruler is unchanged");
            assert_eq!(app.session.active().unwrap().history.past_len(), steps, "({x}, {y}): no history step");
            assert!(app.ui.canvas_tool_menu.is_some(), "({x}, {y}): the menu is open");
        }
    }
}

#[test]
fn rows_are_enabled_as_the_layers_panel_menu_enables_them() {
    let (mut app, ctx) = app(None);
    right_click(&mut app, &ctx, 30.0, 20.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for (_, id) in rows(&menu).iter().flatten() {
        assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id}");
    }
}

#[test]
fn picking_a_row_runs_it_on_the_active_layer() {
    let (mut app, ctx) = app(None);
    let layers = app.session.active().unwrap().doc.layers.len();
    right_click(&mut app, &ctx, 30.0, 20.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "layer.delete"}))["ok"], true);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1, "Delete Layer ran on the active layer");
    assert!(app.ui.canvas_tool_menu.is_none(), "the pick closes the menu");
}
