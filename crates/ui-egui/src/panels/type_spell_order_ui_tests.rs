//! #14174: the Check Spelling dialog opened from the menu with Check All Layers on starts at the
//! active layer, goes up and wraps, and keeps that start when a Change refreshes the list after
//! another layer became active.

use craft::text::ui::spelling::CheckSpellingDialog;
use craft::ui::dialog::StackDialog as _;
use serde_json::json;

use super::super::spell_enabled_ui_tests::{app, choose};
use super::*;

fn dialog(app: &PhotocraftApp) -> &CheckSpellingDialog {
    app.framework.dialogs.get::<CheckSpellingDialog>().expect("the Check Spelling dialog is open")
}

/// The dialog's words, in its order.
fn words(app: &PhotocraftApp) -> Vec<String> {
    dialog(app).items().iter().map(|i| i.word.clone()).collect()
}

/// The layers of the dialog's words, in its order.
fn layers(app: &PhotocraftApp) -> Vec<u64> {
    dialog(app).items().iter().map(|i| i.object).collect()
}

#[test]
fn dialog_starts_at_the_opening_layer_and_keeps_it_after_a_change() {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh quik one"})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.create", json!({"x": 10, "y": 120, "text": "hello world"})).unwrap();
    let top = app.run("type.create", json!({"x": 10, "y": 180, "text": "brwn dog"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    choose(&mut app).unwrap();
    assert_eq!(words(&app), ["Teh", "quik", "brwn"], "from the bottom layer up, not from the top");

    // The host makes the top layer active while the dialog is open; Change refreshes the list.
    app.run("layer.select", json!({"layer": top})).unwrap();
    let item = dialog(&app).items()[0].clone();
    change(&mut app, &item, "The");
    let items = relist(&mut app);
    app.framework.dialogs.get_mut::<CheckSpellingDialog>().unwrap().set_items(items);
    assert_eq!(words(&app), ["quik", "brwn"], "the start stays the layer the dialog was opened on");
    assert_eq!(text_of(&app, bottom), "The quik one", "Change replaced \"Teh\"");
}

/// Bottom "Teh one" and top "brwn dog" over the empty layer; returns their ids.
fn two_layers() -> (Box<PhotocraftApp>, u64, u64) {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh one"})).unwrap()["layer"].as_u64().unwrap();
    let top = app.run("type.create", json!({"x": 10, "y": 120, "text": "brwn dog"})).unwrap()["layer"].as_u64().unwrap();
    (app, bottom, top)
}

/// Opened with no layer active, Check All Layers goes top to bottom, and selecting a layer later
/// does not move the start there.
#[test]
fn dialog_opened_without_an_active_layer_keeps_the_top_to_bottom_order() {
    let (mut app, bottom, top) = two_layers();
    app.run("select.deselectLayers", json!({})).unwrap();
    open_spelling(&mut app, true).unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    let first = relist(&mut app).first().map(|i| i.object);
    assert_eq!(first, Some(top), "the top layer comes first");
}

/// Opened with Check All Layers off on the bottom layer, then turned on after the top layer became
/// active: the check starts at the opening layer.
#[test]
fn turning_check_all_layers_on_starts_at_the_opening_layer() {
    let (mut app, bottom, top) = two_layers();
    let ctx = app.ctx().clone();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    open_spelling(&mut app, false).unwrap();
    app.run("layer.select", json!({"layer": top})).unwrap();
    app.framework.dialogs.get_mut::<CheckSpellingDialog>().unwrap().set_json_field(&ctx, "allLayers", json!(true)).unwrap();
    take(&mut app, &ctx);
    assert_eq!(layers(&app), [bottom, top]);
}
