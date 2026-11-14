//! `layer.setProps` on several layers at once (the Layers panel with several selected, as in
//! Photoshop), and Photoshop's History names for each kind of change.

use photocraft_engine::Session;
use serde_json::json;

fn doc() -> (Session, Vec<u64>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let ids = (0..3).map(|_| s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap()).collect();
    (s, ids)
}

fn layer(s: &Session, id: u64) -> photocraft_doc::Layer {
    s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clone()
}

fn last_step(s: &Session) -> String {
    s.active().unwrap().history.entries().last().cloned().unwrap_or_default()
}

#[test]
fn one_change_reaches_every_listed_layer_in_one_step() {
    let (mut s, ids) = doc();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("layer.setProps", json!({"layers": [ids[0], ids[2]], "opacity": 0.4})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one step");
    assert_eq!(last_step(&s), "Opacity Change");
    assert_eq!((layer(&s, ids[0]).opacity, layer(&s, ids[1]).opacity, layer(&s, ids[2]).opacity), (0.4, 1.0, 0.4));
    s.execute("layer.setProps", json!({"layers": ids, "blend": "Multiply"})).unwrap();
    assert_eq!(last_step(&s), "Blending Change");
    assert!(ids.iter().all(|id| layer(&s, *id).blend == photocraft_color::BlendMode::Multiply));
    s.execute("layer.setProps", json!({"layers": [ids[1], ids[1], ids[2]], "locks": {"all": true}})).unwrap();
    assert!(layer(&s, ids[1]).locks.all && layer(&s, ids[2]).locks.all && !layer(&s, ids[0]).locks.all, "duplicates are harmless");
    s.execute("layer.setProps", json!({"layers": [ids[0]], "fill": 0.5})).unwrap();
    assert_eq!(last_step(&s), "Fill Opacity Change");
    s.execute("layer.setProps", json!({"layer": ids[0], "name": "Sky"})).unwrap();
    assert_eq!(last_step(&s), "Rename Layer");
    s.execute("layer.setProps", json!({"layer": ids[0], "opacity": 0.3, "fill": 0.3})).unwrap();
    assert_eq!(last_step(&s), "Layer Properties", "a mix keeps the general name");
}

#[test]
fn bad_layer_lists_change_nothing() {
    let (mut s, ids) = doc();
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({"layers": [], "opacity": 0.5}),
        json!({"layers": "all", "opacity": 0.5}),
        json!({"layers": ["x"], "opacity": 0.5}),
        json!({"layers": [ids[0], 999_999], "opacity": 0.5}),
        json!({"layers": [ids[0], ids[1]], "name": "Both"}),
        json!({"layers": [ids[0], ids[1]], "opacity": 2.0}),
    ] {
        assert!(s.execute("layer.setProps", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc.layers, before.layers, "{p}");
    }
}

#[test]
fn pass_through_is_for_groups_only() {
    let (mut s, ids) = doc();
    let g = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    // A pixel layer, alone or with a group, can't take Pass Through; nothing changes.
    assert!(s.execute("layer.setProps", json!({"layer": ids[0], "blend": "Pass Through"})).is_err());
    let before = (layer(&s, ids[0]).blend, layer(&s, g).blend);
    assert!(s.execute("layer.setProps", json!({"layers": [g, ids[0]], "blend": "Pass Through"})).is_err());
    assert_eq!((layer(&s, ids[0]).blend, layer(&s, g).blend), before);
    s.execute("layer.setProps", json!({"layers": [g], "blend": "Normal"})).unwrap();
    s.execute("layer.setProps", json!({"layer": g, "blend": "Pass Through"})).unwrap();
    assert_eq!(layer(&s, g).blend, photocraft_color::BlendMode::PassThrough);
}

/// Two more layers in a new document, their ids shifted by an extra layer under them, selected.
fn another_document_with_two_selected(s: &mut Session) -> (u64, u64, u64) {
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let mut new_layer = || s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let (spare, a, b) = (new_layer(), new_layer(), new_layer());
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    (spare, a, b)
}

#[test]
fn a_recorded_change_to_the_selected_layers_replays_on_the_selection_in_another_document() {
    let (mut s, ids) = doc();
    s.execute("layer.select", json!({"layer": ids[1]})).unwrap();
    s.execute("layer.select", json!({"layer": ids[2], "mode": "add"})).unwrap();
    s.execute("actions.record", json!({"name": "Multiply"})).unwrap();
    s.execute("layer.setProps", json!({"layers": [ids[1], ids[2]], "blend": "Multiply"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let step = s.actions.list[0].steps[0].1.clone();
    assert!(step.get("layers").is_none(), "the step plays on the selection, not on these ids: {step}");

    let (spare, a, b) = another_document_with_two_selected(&mut s);
    let before = layer(&s, spare).blend;
    let r = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(layer(&s, a).blend, layer(&s, b).blend, "both selected layers changed");
    assert_ne!(layer(&s, a).blend, before, "the selected layers changed");
    assert_eq!(layer(&s, spare).blend, before, "the unselected layer stayed");
}

#[test]
fn a_recorded_change_to_other_layers_than_the_selection_keeps_their_ids() {
    let (mut s, ids) = doc();
    s.execute("layer.select", json!({"layer": ids[2]})).unwrap();
    s.execute("actions.record", json!({"name": "Others"})).unwrap();
    s.execute("layer.setProps", json!({"layers": [ids[0], ids[1]], "opacity": 0.5})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1["layers"], json!([ids[0], ids[1]]));
}

#[test]
fn a_change_without_a_target_reaches_every_selected_layer() {
    let (mut s, ids) = doc();
    s.execute("layer.select", json!({"layer": ids[0]})).unwrap();
    s.execute("layer.select", json!({"layer": ids[2], "mode": "add"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.25})).unwrap();
    assert_eq!((layer(&s, ids[0]).opacity, layer(&s, ids[1]).opacity, layer(&s, ids[2]).opacity), (0.25, 1.0, 0.25));
}

#[test]
fn a_rename_without_a_target_still_renames_only_the_active_layer() {
    let (mut s, ids) = doc();
    s.execute("layer.select", json!({"layer": ids[0]})).unwrap();
    s.execute("layer.select", json!({"layer": ids[2], "mode": "add"})).unwrap();
    s.execute("layer.setProps", json!({"name": "Renamed"})).unwrap();
    let renamed = ids.iter().filter(|id| layer(&s, **id).name == "Renamed").count();
    assert_eq!(renamed, 1);
}

#[test]
fn a_recorded_visibility_change_on_the_selected_layers_still_reaches_them_all() {
    let (mut s, ids) = doc();
    s.execute("layer.select", json!({"layer": ids[1]})).unwrap();
    s.execute("layer.select", json!({"layer": ids[2], "mode": "add"})).unwrap();
    s.execute("actions.record", json!({"name": "Hide"})).unwrap();
    s.execute("layer.setProps", json!({"layers": [ids[1], ids[2]], "visible": false})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("layer.setProps", json!({"layers": [ids[1], ids[2]], "visible": true})).unwrap();
    let r = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert!(!layer(&s, ids[1]).visible && !layer(&s, ids[2]).visible, "both are hidden again");
    assert!(layer(&s, ids[0]).visible, "the unselected layer stayed");
}
