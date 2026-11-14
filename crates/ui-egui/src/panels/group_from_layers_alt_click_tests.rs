//! #14717: ⌥-click on the Layers panel's "Create a new group" button opens New Group from Layers
//! and, on OK, puts the active layer into the new group also when it is the only selected layer,
//! as in Photoshop. With the Background or nothing active it keeps opening New Group (not
//! measured); plain, ⌘ and Shift clicks are unchanged.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// White Background, then Red, Green and Blue; only `active` is selected (`None`: no active layer).
fn document(active: Option<&str>) -> (Harness<'static, PhotocraftApp>, Vec<LayerId>) {
    let mut s = photocraft_engine::Session::new();
    s.execute("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let mut ids = vec![s.active().unwrap().doc.layers[0].id];
    for name in ["Red", "Green", "Blue"] {
        ids.push(LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()));
    }
    match active {
        Some(name) => {
            let i = ["Background", "Red", "Green", "Blue"].iter().position(|n| *n == name).unwrap();
            s.execute("layer.select", json!({"layer": ids[i].0})).unwrap();
        }
        None => {
            s.execute("select.deselectLayers", json!({})).unwrap();
        }
    }
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(4);
    (h, ids)
}

fn groups(h: &Harness<'static, PhotocraftApp>) -> Vec<Layer> {
    h.state().session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

/// ⌥-clicks the button and returns the opened dialog's id and command.
fn alt_click(h: &mut Harness<'static, PhotocraftApp>) -> (u64, String) {
    h.get_by_label("Create a new group").click_modifiers(Modifiers::ALT);
    h.run_steps(4);
    assert!(groups(h).is_empty(), "nothing happens before OK");
    let d = h.state().ui.dialogs.last().expect("a group dialog opened");
    assert!(crate::new_group_ui::owns(&d.fields));
    (d.id, d.fields["__command"].as_str().unwrap_or_default().to_string())
}

#[test]
fn alt_click_with_one_layer_opens_new_group_from_layers_and_groups_it() {
    let (mut h, ids) = document(Some("Blue"));
    let (id, command) = alt_click(&mut h);
    assert_eq!(command, crate::new_group_ui::FROM_LAYERS);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__label"], "New Group from Layers…");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![ids[3]], "Blue moves into the new group");
    let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top, vec![ids[0], ids[1], ids[2], groups[0].id], "the group takes Blue's place");
    assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![groups[0].id]);
}

#[test]
fn alt_click_with_one_lower_layer_groups_it_in_place() {
    let (mut h, ids) = document(Some("Red"));
    let (id, command) = alt_click(&mut h);
    assert_eq!(command, crate::new_group_ui::FROM_LAYERS);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(4);
    let groups = groups(&h);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![ids[1]]);
    let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top, vec![ids[0], groups[0].id, ids[2], ids[3]]);
}

/// Not measured: with the Background active, ⌥-click keeps opening New Group, and OK adds an
/// empty group.
#[test]
fn alt_click_with_the_background_active_keeps_opening_new_group() {
    let (mut h, _) = document(Some("Background"));
    let (id, command) = alt_click(&mut h);
    assert_eq!(command, crate::new_group_ui::NEW_GROUP);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(4);
    assert!(groups(&h)[0].children().unwrap().is_empty());
}

/// Not measured: with nothing active, ⌥-click keeps opening New Group.
#[test]
fn alt_click_with_nothing_active_keeps_opening_new_group() {
    let (mut h, _) = document(None);
    assert!(h.state().session.active().unwrap().active_layer.is_none());
    let (_, command) = alt_click(&mut h);
    assert_eq!(command, crate::new_group_ui::NEW_GROUP);
}

/// With one layer selected a plain click still adds an empty group above it, and Shift-click
/// still groups it.
#[test]
fn plain_and_shift_click_with_one_layer_are_unchanged() {
    let (mut h, _) = document(Some("Blue"));
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    assert!(h.state().ui.dialogs.is_empty());
    let g = groups(&h);
    assert!(g[0].children().unwrap().is_empty());

    let (mut h, ids) = document(Some("Blue"));
    h.get_by_label("Create a new group").click_modifiers(Modifiers::SHIFT);
    h.run_steps(4);
    assert!(h.state().ui.dialogs.is_empty());
    let g = groups(&h);
    assert_eq!(g[0].children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[3]]);
}
