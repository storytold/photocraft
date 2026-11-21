//! #12685: when the Dodge tool can't work on the layer (a hidden target layer, Lock All, Lock Image
//! Pixels, a group's pixels), a press opens the reference application's alert with OK and changes
//! nothing. Lock position and Lock transparent pixels still dodge. The Brush keeps its alerts.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::control::{ControlRequest, Reply, handle};

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::Dodge);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the dodge tool because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

/// A targeted layer mask on a hidden layer keeps the status-bar refusal: only a pixel target gets the alert (CodeRabbit
/// on #16284).
#[test]
fn a_hidden_layer_with_a_mask_target_keeps_the_status_bar_message() {
    let mut app = app(Tool::Dodge);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    app.ui.mask_target = true;
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no alert: {:?}", alerts(&app));
    assert!(app.ui.status_error, "the status bar says it: {:?}", app.ui.status);
}

/// The same for a hidden group whose mask is the target (the case #12685 marks "not measured"): no alert either (CodeRabbit
/// on #16284).
#[test]
fn a_hidden_group_with_a_mask_target_keeps_the_status_bar_message() {
    let mut app = app(Tool::Dodge);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    app.ui.mask_target = true;
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no alert: {:?}", alerts(&app));
    assert!(app.ui.status_error, "the status bar says it: {:?}", app.ui.status);
}

#[test]
fn lock_all_and_lock_image_pixels() {
    for lock in ["all", "pixels"] {
        let mut app = app(Tool::Dodge);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, "Could not use the dodge tool because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::Dodge);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the dodge tool because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn lock_position_and_lock_transparent_pixels_still_dodge() {
    for lock in ["position", "transparency"] {
        let mut app = app(Tool::Dodge);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        let before = steps(&app);
        stroke(&mut app);
        assert!(alerts(&app).is_empty(), "{lock}: no refusal: {:?}", alerts(&app));
        assert_eq!(steps(&app), before + 1, "{lock}: the stroke was applied");
    }
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::Dodge);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
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
    assert_eq!(dialog["fields"]["message"], "Could not use the dodge tool because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn the_brush_keeps_todays_lock_image_pixels() {
    let mut app = app(Tool::Brush);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "the Brush has no Lock Image Pixels alert: {:?}", alerts(&app));
}
