//! #12020: when the Magic Eraser can't erase (a hidden target layer, Lock All or Lock Image Pixels,
//! a group), a click opens the reference application's alert with OK and erases nothing. Lock
//! position still erases; Lock transparent pixels still fills the matched area with the background
//! colour.

use egui::Modifiers;
use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, steps};
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::control::{ControlRequest, Reply, handle};

/// A Magic Eraser click on the painted layer.
fn click(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 30.0 }, Modifiers::NONE);
}

/// The click opens one alert with `want` and changes nothing; OK closes it.
fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = steps(app);
    click(app);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!(steps(app), before, "nothing erased, no History step");
    let id = app.framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    assert!(alerts(app).is_empty(), "OK closes the alert");
}

fn pixel(app: &PhotocraftApp) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 30)
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::MagicEraser);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the magic eraser because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
    assert_eq!(photocraft_engine::hidden_target::HIDDEN, "Could not complete your request because the target layer is hidden.", "the shared text is unchanged");
}

#[test]
fn lock_all_and_lock_image_pixels() {
    for lock in ["all", "pixels"] {
        let mut app = app(Tool::MagicEraser);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, "Could not use the magic eraser because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::MagicEraser);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the magic eraser because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::MagicEraser);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    click(&mut app);
    let mut call = |method: &str, params: serde_json::Value| {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(&mut app, &ctx, &req) {
            Reply::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let inspect = call("ui.inspect", json!({}));
    let dialog = &inspect["result"]["dialogs"][0];
    assert_eq!(dialog["fields"]["message"], "Could not use the magic eraser because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn lock_position_still_erases() {
    let mut app = app(Tool::MagicEraser);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1);
    assert_eq!(pixel(&app)[3], 0.0, "erased");
}

#[test]
fn lock_transparent_pixels_fills_with_the_background_colour() {
    let mut app = app(Tool::MagicEraser);
    app.run("tools.setColors", json!({"background": "#ffffff"})).unwrap();
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1);
    let [r, g, b, a] = pixel(&app);
    assert!(a > 0.99 && r > 0.99 && g > 0.99 && b > 0.99, "filled with white: {:?}", [r, g, b, a]);
}
