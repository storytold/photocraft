//! #14871: the Color Overlay's swatch opens a picker titled "Color Picker (Overlay Color)"; Cancel
//! keeps the colour, OK applies it to the page and the preview; other effects keep their title.
use serde_json::{Map, Value, json};

use super::color_picker::{open, preview_fields};
use crate::{PhotocraftApp, dialogs, layer_style};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
    app.run("file.new", json!({"width": 32, "height": 24, "background": "#406080"})).unwrap();
    app
}

fn fields(app: &PhotocraftApp, id: u64) -> Map<String, Value> {
    app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

fn overlay_color(f: &Map<String, Value>, effect: &str) -> Value {
    super::entry(f, effect).unwrap()["params"]["color"].clone()
}

#[test]
fn color_overlay_picker_is_titled_overlay_color_and_cancel_keeps_ok_applies() {
    let mut app = app();
    let parent = layer_style::open(&mut app, Some("colorOverlay")).unwrap();
    let effect = fields(&app, parent)["selected"].as_str().unwrap().to_string();
    let original = fields(&app, parent);

    let picker = open(&mut app, parent, &effect, "color", None).unwrap();
    assert_eq!(fields(&app, picker)["__label"], "Color Picker (Overlay Color)");
    app.ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#23a567"));
    assert_eq!(overlay_color(&preview_fields(&app, parent, &original), &effect), "#23a567", "the preview follows the picker");
    dialogs::cancel(&mut app, picker).unwrap();
    assert_eq!(fields(&app, parent), original, "Cancel keeps the overlay colour");

    let picker = open(&mut app, parent, &effect, "color", None).unwrap();
    app.ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#23a567"));
    dialogs::confirm(&mut app, picker).unwrap();
    assert_eq!(overlay_color(&fields(&app, parent), &effect), "#23a567", "OK applies it to the page");
}

#[test]
fn other_effects_keep_the_shared_picker_title() {
    let mut app = app();
    let parent = layer_style::open(&mut app, Some("stroke")).unwrap();
    let effect = fields(&app, parent)["selected"].as_str().unwrap().to_string();
    let picker = open(&mut app, parent, &effect, "color", None).unwrap();
    assert_eq!(fields(&app, picker)["__label"], "Color Picker (Layer Style Color)");
}
