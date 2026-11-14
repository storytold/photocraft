//! #14184: the Check Spelling dialog with Check All Layers off checks every layer selected when it
//! opened, from the active one up and wrapping, and keeps them when a Change refreshes the list.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::spell_enabled_ui_tests::{app, harness, open_edit_menu};
use super::spell_order_ui_tests::words;

#[test]
fn check_all_layers_off_checks_the_selected_layers_and_keeps_them_after_a_change() {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh one"})).unwrap()["layer"].clone();
    let middle = app.run("type.create", json!({"x": 10, "y": 120, "text": "quik mid"})).unwrap()["layer"].clone();
    let top = app.run("type.create", json!({"x": 10, "y": 180, "text": "brwn dog"})).unwrap()["layer"].clone();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    // ⌘-click in the Layers panel.
    app.run("layer.select", json!({"layer": top, "mode": "toggle"})).unwrap();
    let mut h = harness(app);
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    h.get_by_label("Check All Layers").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["brwn", "Teh"], "the active top layer, then the selected bottom one, not the middle");

    // The host selects the middle layer alone while the dialog is open; Change refreshes the list.
    h.state_mut().run("layer.select", json!({"layer": middle})).unwrap();
    h.get_by_label("Change").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["Teh"], "the layers selected when the dialog opened");
}

/// A selected layer deleted while the dialog is open drops out of the list; the other stays.
#[test]
fn deleting_a_selected_layer_keeps_the_other_selected_layer_listed() {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "quik one"})).unwrap()["layer"].clone();
    let top = app.run("type.create", json!({"x": 10, "y": 120, "text": "brwn dog Teh"})).unwrap()["layer"].clone();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    app.run("layer.select", json!({"layer": top, "mode": "toggle"})).unwrap();
    let mut h = harness(app);
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    h.get_by_label("Check All Layers").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["brwn", "Teh", "quik"]);

    h.state_mut().run("layer.delete", json!({"layer": bottom})).unwrap();
    h.get_by_label("Change").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["Teh"], "the top layer's word after the bottom layer is gone");
}
