use std::sync::Arc;

use serde_json::json;

use super::Tool;
use super::brush_refusal_alert_tests::{active, alerts, app, steps, stroke};
use crate::canvas::{ToolEvent, tool_event};
use crate::control::{ControlRequest, Reply, handle};

#[test]
fn blur_refuses_unpaintable_layers_on_press() {
    for (command, params, message) in [
        ("layer.setProps", json!({"visible": false}), "Could not use the blur tool because the target layer is hidden."),
        ("layer.lockLayers", json!({"all": true}), "Could not use the blur tool because the layer is locked."),
        ("layer.lockLayers", json!({"pixels": true}), "Could not use the blur tool because the layer is locked."),
        ("layer.new.group", json!({}), "Could not use the blur tool because the pixels in a group cannot be edited without merging the group."),
    ] {
        let mut app = app(Tool::Blur);
        let mut params = params;
        if command == "layer.setProps" {
            params["layer"] = json!(active(&app));
        }
        app.run(command, params).unwrap();
        let doc = Arc::clone(&app.session.active().unwrap().doc);
        let history = steps(&app);

        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(alerts(&app), [message], "{command} should open an alert at press time");
        tool_event(&mut app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 44.0, y: 32.0 }, egui::Modifiers::NONE);
        assert_eq!(alerts(&app), [message], "the refused stroke should keep exactly one alert open");
        assert_eq!(steps(&app), history);
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "a refused stroke should leave the document intact");

        let ctx = app.ctx().clone();
        let mut call = |method: &str, params: serde_json::Value| {
            let (req, _rx) = ControlRequest::new(method, params);
            match handle(&mut app, &ctx, &req) {
                Reply::Done(value) => value,
                _ => panic!("{method}: expected an immediate reply"),
            }
        };
        let inspect = call("ui.inspect", json!({}));
        let dialog = &inspect["result"]["dialogs"][0];
        assert_eq!(dialog["fields"]["message"], message, "{inspect}");
        assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
        assert!(alerts(&app).is_empty(), "OK should close the alert");
    }
}

#[test]
fn blur_still_paints_with_transparency_locked() {
    let mut app = app(Tool::Blur);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
    app.run("edit.fill", json!({"color": "#2030d0"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();

    let id = active(&app);
    let pixels = |app: &crate::PhotocraftApp| {
        let st = app.session.active().unwrap();
        st.doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().read_region(st.doc.bounds())
    };
    let before = pixels(&app);
    let history = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_ne!(pixels(&app), before, "the stroke should blur the contrasting edge");
    assert_eq!(steps(&app), history + 1);
}

#[test]
fn blur_on_a_hidden_group_mask_keeps_the_status_bar_refusal() {
    let mut app = app(Tool::Blur);
    app.run("layer.new.group", json!({})).unwrap();
    let id = active(&app);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.target", json!({"channel": "mask"})).unwrap();
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    let history = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "targeting a mask should keep the existing status-bar behaviour");
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
    assert_eq!(steps(&app), history);
}
