//! #2561: the Layers panel New Group button. Like Photoshop's folder button a click moves the
//! selected layers into the new group; with no layer to group it adds an empty one.

use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// Background, then `A` and `B` on top of it with both selected (`B` active).
fn two_selected() -> (egui_kittest::Harness<'static, PhotocraftApp>, LayerId, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let h = harness(s);
    (h, a, b)
}

/// A document whose layers are all deselected, so the button has nothing to group.
fn no_selection() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("select.deselectLayers", json!({})).unwrap();
    harness(s)
}

fn harness(s: photocraft_engine::Session) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(4);
    h
}

fn groups(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> Vec<Layer> {
    h.state().session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

#[test]
fn click_groups_the_selected_layers_in_order() {
    let (mut h, a, b) = two_selected();
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![a, b]);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 2, "background and the new group stay at the top level");
}

#[test]
fn shift_click_still_groups_the_selected_layers() {
    let (mut h, a, b) = two_selected();
    h.get_by_label("Create a new group").click_modifiers(Modifiers::SHIFT);
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![a, b]);
}

#[test]
fn plain_click_without_a_selection_adds_an_empty_group() {
    let mut h = no_selection();
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    assert!(groups[0].children().unwrap().is_empty());
    let doc = &h.state().session.active().unwrap().doc;
    let top: Vec<LayerId> = doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top.len(), 2, "the background stays and the empty group is added");
}
