//! Text made in place from an empty pixel layer (`type.create` with `layer`): the paint that
//! makes a layer non-empty lives outside the core crate, so this runs on the whole engine.

use photocraft_engine::Session;
use serde_json::json;

fn session_with_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

/// `type.create` with `layer` turns that empty pixel layer into the text layer in one edit, in
/// place: the id, the place and its own settings stay; anything else is refused unchanged.
#[test]
fn type_create_can_turn_an_empty_pixel_layer_into_text_in_place() {
    let mut s = session_with_doc();
    let id = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "Caption"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
    let above = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let count = |s: &Session| s.active().unwrap().doc.layers.len();
    let (layers, steps) = (count(&s), s.execute("document.inspect", json!({})).unwrap()["history"].as_array().unwrap().len());
    let made = s.execute("type.create", json!({"x": 0, "y": 0, "text": "", "layer": id.0})).unwrap();
    assert_eq!(made["layer"], json!(id.0));
    assert_eq!(count(&s), layers, "no layer added or left behind");
    let steps_now = s.execute("document.inspect", json!({})).unwrap()["history"].as_array().unwrap().len();
    assert_eq!(steps_now, steps + 1, "one edit");
    let l = s.active().unwrap().doc.layer(id).unwrap();
    assert!(matches!(l.content, photocraft_doc::LayerContent::Text(_)));
    assert_eq!((l.name.as_str(), l.opacity), ("Caption", 0.5));
    // One undo brings the pixel layer back, as it was.
    s.execute("edit.undo", json!({})).unwrap();
    let l = s.active().unwrap().doc.layer(id).unwrap();
    assert!(matches!(l.content, photocraft_doc::LayerContent::Raster(_)));
    assert_eq!((l.name.as_str(), l.opacity, count(&s)), ("Caption", 0.5, layers));
    s.execute("edit.redo", json!({})).unwrap();
    // A layer with pixels, or the background, stays as it was.
    s.execute("select.all", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": above})).unwrap();
    s.execute("edit.fill", json!({"color": "#404040"})).unwrap();
    assert!(s.execute("type.create", json!({"x": 0, "y": 0, "text": "", "layer": above})).is_err());
    let background = s.active().unwrap().doc.layers[0].id;
    assert!(s.active().unwrap().doc.is_background(background));
    assert!(s.execute("type.create", json!({"x": 0, "y": 0, "text": "", "layer": background.0})).is_err());
    assert_eq!(count(&s), layers);
}
