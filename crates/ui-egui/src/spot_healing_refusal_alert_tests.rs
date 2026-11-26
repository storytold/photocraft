//! #12581: when the Spot Healing Brush can't heal (a hidden target layer, Lock All, a group's
//! pixels), a press opens the reference application's alert with OK and changes nothing. On a hidden
//! empty pixel layer with Sample All Layers off the press does nothing and says nothing. Lock
//! transparent pixels is no refusal.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::SpotHealing);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the spot healing brush because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all() {
    let mut app = app(Tool::SpotHealing);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the spot healing brush because the layer is locked.");
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::SpotHealing);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the spot healing brush because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn a_hidden_empty_layer_with_sample_all_layers_off_does_nothing_quietly() {
    let mut app = app(Tool::SpotHealing);
    app.ui.tool_options.sample_all.spot_healing = false;
    let empty = app.run("layer.new.layer", json!({"name": "Empty"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.setProps", json!({"layer": empty, "visible": false})).unwrap();
    app.sync_views();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no alert: {:?}", alerts(&app));
    assert_eq!(steps(&app), before, "no History step");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error: {:?}", app.ui.status);
}

#[test]
fn lock_transparent_pixels_still_heals() {
    let mut app = app(Tool::SpotHealing);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no refusal: {:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the stroke was applied");
}

#[test]
fn the_shared_hidden_text_stays() {
    assert_eq!(photocraft_engine::hidden_target::HIDDEN, "Could not complete your request because the target layer is hidden.");
}
