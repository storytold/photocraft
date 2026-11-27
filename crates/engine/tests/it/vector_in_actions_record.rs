//! Tests that run vector commands, moved from the engine-core `actions_record` tests: 1 test(s).

use photocraft_doc::LayerId;

use serde_json::json;

use photocraft_engine::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    s
}

fn new_layer(s: &mut Session, name: &str) -> u64 {
    s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()
}

#[test]
fn selection_actions_keep_the_active_layer_as_their_source() {
    let mut s = session();
    let raster = new_layer(&mut s, "Pixels");
    s.execute("actions.record", json!({"name": "Load pixels"})).unwrap();
    s.execute("select.loadSelection", json!({"channel": "transparency", "layer": raster})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Load pixels"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], json!({"name": "Pixels"}), "missing layer loads merged pixels instead");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Pixels");
    let played = s.execute("actions.play", json!({"action": "Load pixels"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(
        s.active().unwrap().doc.selection.as_ref().map(|mask| mask.content_bounds()).unwrap_or(photocraft_geom::Rect::EMPTY),
        photocraft_geom::Rect::EMPTY,
        "the empty source layer must not load the opaque merged document"
    );

    let type_id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let type_name = s.active().unwrap().doc.layer(LayerId(type_id)).unwrap().name.clone();
    s.execute("actions.record", json!({"name": "Mask text"})).unwrap();
    s.execute("type.createSelection", json!({"layer": type_id, "removeLayer": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Mask text"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], type_name, "without the source, replay cannot remove that type layer");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Spare");
    let replay_type = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(replay_type, type_id);
    let played = s.execute("actions.play", json!({"action": "Mask text"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(s.active().unwrap().doc.selection.as_ref().is_some_and(|mask| !mask.content_bounds().is_empty()), "replay selects the glyphs");
    assert!(s.active().unwrap().doc.layer(LayerId(replay_type)).is_none(), "replay removed the named source type layer");
}
