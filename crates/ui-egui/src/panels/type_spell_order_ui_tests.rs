//! #14174: the Check Spelling dialog opened from the menu with Check All Layers on starts at the
//! active layer, goes up and wraps, and keeps that start when a Change refreshes the list after
//! another layer became active.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::spell_enabled_ui_tests::{app, harness, open_edit_menu};

/// The dialog's words, in its order.
pub(super) fn words(app: &crate::PhotocraftApp) -> Vec<String> {
    let spell = app.ui.type_panels.spell.as_ref().expect("the Check Spelling dialog is open");
    spell.items[spell.index..].iter().map(|i| i["word"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn dialog_starts_at_the_opening_layer_and_keeps_it_after_a_change() {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh quik one"})).unwrap()["layer"].clone();
    app.run("type.create", json!({"x": 10, "y": 120, "text": "hello world"})).unwrap();
    let top = app.run("type.create", json!({"x": 10, "y": 180, "text": "brwn dog"})).unwrap()["layer"].clone();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    let mut h = harness(app);
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["Teh", "quik", "brwn"], "from the bottom layer up, not from the top");

    // The host makes the top layer active while the dialog is open; Change refreshes the list.
    h.state_mut().run("layer.select", json!({"layer": top})).unwrap();
    h.get_by_label("Change").click();
    h.run_steps(4);
    assert_eq!(words(h.state()), ["quik", "brwn"], "the start stays the layer the dialog was opened on");
    let layer = h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(bottom.as_u64().unwrap())).map(|l| l.content.clone());
    assert!(matches!(layer, Some(photocraft_doc::LayerContent::Text(t)) if t.text == "The quik one"), "Change replaced \"Teh\"");
}

/// Bottom "Teh one" and top "brwn dog" over the empty layer; returns their ids.
fn two_layers() -> (crate::PhotocraftApp, serde_json::Value, serde_json::Value) {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh one"})).unwrap()["layer"].clone();
    let top = app.run("type.create", json!({"x": 10, "y": 120, "text": "brwn dog"})).unwrap()["layer"].clone();
    (app, bottom, top)
}

/// Opened with no layer active, Check All Layers goes top to bottom, and selecting a layer later
/// does not move the start there.
#[test]
fn dialog_opened_without_an_active_layer_keeps_the_top_to_bottom_order() {
    let (mut app, bottom, top) = two_layers();
    app.run("select.deselectLayers", json!({})).unwrap();
    super::open_spelling(&mut app, true).unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    super::refresh_spelling(&mut app);
    let first = app.ui.type_panels.spell.as_ref().and_then(|d| d.items.first().map(|i| i["layer"].clone()));
    assert_eq!(first, Some(top), "the top layer comes first");
}

/// Opened with Check All Layers off on the bottom layer, then turned on after the top layer became
/// active: the check starts at the opening layer.
#[test]
fn turning_check_all_layers_on_starts_at_the_opening_layer() {
    let (mut app, bottom, top) = two_layers();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    super::open_spelling(&mut app, false).unwrap();
    app.run("layer.select", json!({"layer": top})).unwrap();
    if let Some(d) = app.ui.type_panels.spell.as_mut() {
        d.all_layers = true;
    }
    super::refresh_spelling(&mut app);
    let layers: Vec<serde_json::Value> = app.ui.type_panels.spell.as_ref().map(|d| d.items.iter().map(|i| i["layer"].clone()).collect()).unwrap_or_default();
    assert_eq!(layers, [bottom, top]);
}
