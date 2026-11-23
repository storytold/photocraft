//! Tests that run paint commands, moved from the engine-core `actions_record` tests: 2 test(s).

use photocraft_doc::LayerId;

use serde_json::{Value, json};

use photocraft_engine::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    s
}

fn new_layer(s: &mut Session, name: &str) -> u64 {
    s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()
}

fn background(s: &Session) -> u64 {
    s.active().unwrap().doc.layers[0].id.0
}

#[test]
fn frame_action_keeps_the_layer_it_wraps() {
    let mut s = session();
    let picture = new_layer(&mut s, "Picture");
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("actions.record", json!({"name": "Frame picture"})).unwrap();
    s.execute("layer.new.frame", json!({"x": 1, "y": 1, "width": 20, "height": 20, "layer": picture})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Frame picture"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], "Picture", "the frame must wrap the recorded source layer");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Spare");
    let target = new_layer(&mut s, "Picture");
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_ne!(target, picture, "the replay target has a different document-local id");
    let played = s.execute("actions.play", json!({"action": "Frame picture"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(
        s.active().unwrap().doc.walk().iter().any(
            |(_, _, layer)| matches!(&layer.content, photocraft_doc::LayerContent::Group(group) if group.children.iter().any(|child| child.id.0 == target))
        ),
        "the replayed frame wraps Picture"
    );
}

#[test]
fn a_recording_replays_on_a_document_with_other_layer_ids() {
    let mut s = session();
    let bg = background(&s);
    s.execute("actions.record", json!({"name": "Shade"})).unwrap();
    let shade = new_layer(&mut s, "Shade");
    s.execute("layer.setProps", json!({"layer": shade, "opacity": 0.5})).unwrap();
    s.execute("layer.select", json!({"layer": bg})).unwrap();
    s.execute("edit.fill", json!({"layer": bg, "color": "#ff0000"})).unwrap();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    for (i, ch) in "Hi".chars().enumerate() {
        s.execute("type.edit", json!({"layer": id, "replace": {"start": i, "end": i, "text": ch.to_string()}, "coalesce": "t5"})).unwrap();
    }
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Shade"})).unwrap()["steps"].clone();
    let ids: Vec<&str> = steps.as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
    assert_eq!(ids, ["layer.new.layer", "layer.setProps", "layer.select", "edit.fill", "type.create"], "{steps}");
    assert!(steps.as_array().unwrap().iter().all(|st| st[1].get("layer").is_none_or(Value::is_string)), "no layer ids: {steps}");

    // A second document whose layers have other ids and a layer stacked over the background.
    s.execute("file.new", json!({"width": 40, "height": 40, "background": "black"})).unwrap();
    new_layer(&mut s, "Top");
    assert_ne!(background(&s), bg);
    let played = s.execute("actions.play", json!({"action": "Shade"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let d = &s.active().unwrap().doc;
    let shade = d.walk().into_iter().find(|(_, _, l)| l.name == "Shade").map(|(_, _, l)| l).unwrap();
    assert_eq!(shade.opacity, 0.5, "the new layer, not the one of the first document, got the opacity");
    let text = d.walk().into_iter().find_map(|(_, _, l)| match &l.content {
        photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("Hi"), "the typed text replayed as one step");
    assert!(s.documents()[0].doc.layer(LayerId(bg)).is_some());
}
