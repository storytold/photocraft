//! #11924: when the Color Replacement tool can't paint (a hidden target layer, Lock All, a group),
//! a press opens the reference application's alert with OK and changes nothing. Lock transparent
//! pixels still recolours. Other tools keep their alerts.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::control::{ControlRequest, Reply, handle};

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::ColorReplacement);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the Color Replacement tool because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
    assert_eq!(photocraft_engine::hidden_target::HIDDEN, "Could not complete your request because the target layer is hidden.", "the shared text is unchanged");
}

#[test]
fn lock_all() {
    let mut app = app(Tool::ColorReplacement);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the Color Replacement tool because the layer is locked.");
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::ColorReplacement);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the Color Replacement tool because the target is a group.");
}

#[test]
fn lock_transparent_pixels_still_recolours() {
    let mut app = app(Tool::ColorReplacement);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no refusal: {:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the stroke was applied");
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::ColorReplacement);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    stroke(&mut app);
    let mut call = |method: &str, params: serde_json::Value| {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(&mut app, &ctx, &req) {
            Reply::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let inspect = call("ui.inspect", json!({}));
    let dialog = &inspect["result"]["dialogs"][0];
    assert_eq!(dialog["fields"]["message"], "Could not use the Color Replacement tool because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn the_brush_keeps_its_group_text() {
    let mut app = app(Tool::Brush);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the brush tool because the pixels in a group cannot be edited without merging the group.");
}
