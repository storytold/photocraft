//! #11784: a Magic Wand press on a layer group with Sample All Layers off opens the reference's
//! error dialog; any other press, refused or not, does not.

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};
use serde_json::{Value, json};

const MESSAGE: &str = "Could not use the magic wand because the target is a group.";

/// A 64 x 64 document with one painted layer, grouped and targeted as a group.
fn grouped_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    app.ui.tool = Tool::MagicWand;
    app.sync_views();
    app
}

fn press(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect()
}

#[test]
fn a_group_target_without_sample_all_layers_opens_the_error_dialog() {
    let mut app = grouped_app();
    app.ui.tool_options.sample_all.magic_wand = false;
    let before = app.session.active().unwrap().history.past_len();
    press(&mut app);
    assert_eq!(alerts(&app), [MESSAGE]);
    assert_eq!(app.session.active().unwrap().history.past_len(), before, "no history step");
    assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing selected");
}

#[test]
fn a_group_target_with_sample_all_layers_selects_without_a_dialog() {
    let mut app = grouped_app();
    app.ui.tool_options.sample_all.magic_wand = true;
    press(&mut app);
    assert!(alerts(&app).is_empty());
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the composite was selected");
}

#[test]
fn a_press_on_a_pixel_layer_opens_no_dialog() {
    let mut app = grouped_app();
    let child = app.session.active().unwrap().doc.layers[1].children().unwrap()[0].id.0;
    app.run("layer.select", json!({"layer": child})).unwrap();
    app.ui.tool_options.sample_all.magic_wand = false;
    press(&mut app);
    assert!(alerts(&app).is_empty());
}
