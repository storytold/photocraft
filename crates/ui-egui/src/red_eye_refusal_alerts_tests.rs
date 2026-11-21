//! #12574: a Red Eye click on a hidden target layer, a locked layer (Lock All or Lock Image
//! Pixels), a group, an adjustment layer or targeted layer mask, or with two layers selected opens
//! the reference application's alert with OK and changes nothing; Lock transparent pixels and Lock
//! position still work. The other tools keep today's behaviour.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;

const PREFIX: &str = "Could not use the red eye removal tool because";

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::RedEye);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, &format!("{PREFIX} the target layer is hidden."));
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all_and_lock_image_pixels() {
    for lock in ["all", "pixels"] {
        let mut app = app(Tool::RedEye);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, &format!("{PREFIX} the layer is locked."));
    }
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::RedEye);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, &format!("{PREFIX} the pixels in a group cannot be edited without merging the group."));
}

#[test]
fn an_adjustment_layer_or_a_targeted_layer_mask() {
    let mut app = app(Tool::RedEye);
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    refused(&mut app, &format!("{PREFIX} the target is not a pixel-based layer."));
    let mut app = self::app(Tool::RedEye);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"), "the new mask is the target");
    refused(&mut app, &format!("{PREFIX} the target is not a pixel-based layer."));
}

#[test]
fn two_layers_selected() {
    let mut app = app(Tool::RedEye);
    let second = active(&app);
    app.run("layer.new.layer", json!({})).unwrap();
    let first = active(&app);
    app.run("layer.select", json!({"layers": [second, first]})).unwrap();
    refused(&mut app, &format!("{PREFIX} more than one layer is selected."));
}

#[test]
fn lock_transparent_pixels_and_lock_position_still_work() {
    for lock in ["transparency", "position"] {
        let mut app = app(Tool::RedEye);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        let before = steps(&app);
        stroke(&mut app);
        assert!(alerts(&app).is_empty(), "{lock}: {:?}", alerts(&app));
        assert_eq!(steps(&app), before + 1, "{lock}: the click ran");
    }
}

#[test]
fn the_other_retouch_tools_keep_the_status_bar() {
    let mut app = app(Tool::SpotHealing);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
}
