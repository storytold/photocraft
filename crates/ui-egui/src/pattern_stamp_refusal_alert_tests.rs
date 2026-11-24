//! #12886: when the Pattern Stamp can't paint (hidden target layer, Lock All, a group active), a
//! press opens the reference application's modal alert with OK instead of a status-bar message, and
//! paints nothing. Lock transparent pixels is not a refusal.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 64 × 64 document with an empty layer active, black foreground, the Pattern Stamp current.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = Tool::PatternStamp;
    app.sync_views();
    app
}

fn active(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The messages of the open framework alerts.
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// A press and release at the same point, then the alert it should have opened with `want`, and
/// no History step.
fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = steps(app);
    tool_event(app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!(steps(app), before, "nothing painted, no History step");
    // Over the control channel it is an ordinary dialog: confirming closes it.
    let id = app.framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    assert!(alerts(app).is_empty(), "OK closes the alert");
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the pattern stamp because the target layer is hidden.");
}

#[test]
fn lock_all() {
    let mut app = app();
    let id = active(&app);
    app.run("layer.lockLayers", json!({"layer": id, "all": true})).unwrap();
    refused(&mut app, "Could not use the pattern stamp because the layer is locked.");
}

#[test]
fn an_active_group() {
    let mut app = app();
    app.run("layer.new.group", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(active(&app))).unwrap().is_group(), "the new group is the active layer");
    refused(&mut app, "Could not use the pattern stamp because the pixels in a group cannot be edited without merging the group.");
}

/// Lock transparent pixels is no refusal: the press adds a Pattern Stamp state and paints nothing
/// on an empty layer, as it already did.
#[test]
fn lock_transparent_pixels_still_paints() {
    let mut app = app();
    let id = active(&app);
    app.run("layer.lockLayers", json!({"layer": id, "transparency": true})).unwrap();
    let before = steps(&app);
    let pixels = {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
    };
    assert!(pixels.iter().all(|pixel| *pixel == 0.0), "the target layer starts empty");
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    assert_eq!(alerts(&app), Vec::<String>::new(), "no refusal alert");
    assert_eq!(steps(&app), before + 1, "a Pattern Stamp state is still added");
    let st = app.session.active().unwrap();
    let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_eq!(surface.read_region(st.doc.bounds()), pixels, "the press changes no pixels");
}

/// The Brush keeps the alerts it had: the Pattern Stamp's entry did not take its place.
#[test]
fn the_brush_alerts_are_unchanged() {
    let mut app = app();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the brush tool because the target layer is hidden.");
}
