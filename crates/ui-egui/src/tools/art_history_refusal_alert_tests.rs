//! #12638: when the Art History Brush can't paint, a stroke opens the reference application's
//! alert with OK and paints nothing: at the press for a hidden target layer, Lock All, Lock Image
//! Pixels or a group's pixels; at the release for the source refusals (another canvas size, no
//! source, a target layer the source state lacks). The History Brush keeps today's status bar.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;

/// The fixture's history: 0 Open, 1 New Layer, 2 Fill; the source is the Open state, which holds
/// only the Background, so a stroke paints from row 1, which holds the active layer.
fn art_history() -> PhotocraftApp {
    let mut app = app(Tool::ArtHistoryBrush);
    app.run("history.setSource", json!({"state": 1})).unwrap();
    app
}

#[test]
fn a_hidden_target_layer() {
    let mut app = art_history();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the art history brush because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all_and_lock_image_pixels() {
    for lock in ["all", "pixels"] {
        let mut app = art_history();
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, "Could not use the art history brush because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    let mut app = art_history();
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the art history brush because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn the_canvas_size_changed() {
    let mut app = art_history();
    app.run("image.canvasSize", json!({"width": 80, "height": 64, "anchor": "topLeft"})).unwrap();
    refused(&mut app, "Could not use the art history brush because the current canvas size does not match that of the history state.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn no_source_is_set() {
    let mut app = app(Tool::ArtHistoryBrush);
    let snap = app.run("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    app.run("history.setSource", json!({"snapshot": snap})).unwrap();
    app.run("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
    refused(&mut app, "Could not use the art history brush because no history state has been designated as the source for painting.");
}

#[test]
fn the_source_state_lacks_the_target_layer() {
    let mut app = art_history();
    app.run("layer.new.layer", json!({})).unwrap();
    refused(&mut app, "Could not use the art history brush because the history state lacks a corresponding layer.");
}

#[test]
fn a_stroke_from_a_valid_source_paints() {
    let mut app = art_history();
    app.run("history.setSource", json!({"state": 2})).unwrap();
    app.run("edit.fill", json!({"color": "#1e1edc"})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1);
}

/// The History Brush keeps its status-bar texts.
#[test]
fn the_history_brush_keeps_the_status_bar() {
    let mut app = app(Tool::HistoryBrush);
    app.run("image.canvasSize", json!({"width": 80, "height": 64, "anchor": "topLeft"})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert!(!app.ui.status.contains("art history"), "{}", app.ui.status);
}
