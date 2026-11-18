//! #1719: layer effects are dragged onto another layer in the Layers panel, as in Photoshop: an
//! effect row moves its effect there, ⌥-drag copies it.

use egui::{Modifiers, PointerButton, Pos2, accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layers_panel_nodes::{drag, row};

/// `a` with a Drop Shadow and a Color Overlay, and `b` above it without effects.
fn harness() -> (Harness<'static, PhotocraftApp>, LayerId, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let a = s.execute("layer.new.layer", json!({"name": "a"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff8000"})).unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "b"})).unwrap()["layer"].as_u64().unwrap();
    // Tall enough that the Layers panel shows every row of the sample, the last effect row included.
    let mut h = Harness::builder().with_size(vec2(1440.0, 1200.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    (h, LayerId(a), LayerId(b))
}

fn effects(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> Vec<&'static str> {
    h.state().session.active().unwrap().doc.layer(id).unwrap().effects.items.iter().map(photocraft_doc::Effect::label).collect()
}

#[test]
fn alt_dragging_an_effect_row_onto_a_layer_copies_it() {
    let (mut h, a, b) = harness();
    assert_eq!(effects(&h, a), ["Drop Shadow", "Color Overlay"]);
    let steps = h.state().session.active().unwrap().history.past_len();

    // ⌥-drag one effect row onto `b`: just that effect is copied.
    let from = row(&h, "Color Overlay").center();
    let to = row(&h, "b").center();
    drag(&mut h, from, to, Modifiers::ALT);
    assert_eq!(effects(&h, a), ["Drop Shadow", "Color Overlay"]);
    assert_eq!(effects(&h, b), ["Color Overlay"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one history step");
}

/// The Layers panel footer's Delete button.
fn trash_button(h: &Harness<'_, PhotocraftApp>) -> Pos2 {
    h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().as_deref() == Some("Delete layer")).unwrap().rect().center()
}

/// #3102: an effect row dropped on the footer's Delete button deletes that effect, the "Effects"
/// row all of them; the layer stays.
#[test]
fn dropping_effects_on_the_trash_deletes_them() {
    let (mut h, a, _) = harness();
    let layers = h.state().session.active().unwrap().doc.layers.len();
    let steps = h.state().session.active().unwrap().history.past_len();
    let from = row(&h, "Drop Shadow").center();
    let trash = trash_button(&h);
    drag(&mut h, from, trash, Modifiers::NONE);
    assert_eq!(effects(&h, a), ["Color Overlay"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one history step");
    let from = row(&h, "Effects").center();
    let trash = trash_button(&h);
    drag(&mut h, from, trash, Modifiers::NONE);
    assert!(effects(&h, a).is_empty());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "no layer was deleted");
}

/// #2846: right-clicking an effect row opens Photoshop's layer style menu for that layer; Create
/// Layer turns its effects into layers of their own.
#[test]
fn right_clicking_an_effect_row_offers_create_layer_for_that_layer() {
    let (mut h, a, b) = harness();
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(b), "`b` is active, not the clicked layer");
    let at = row(&h, "Color Overlay").center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    for label in ["Blending Options…", "Copy Layer Style", "Paste Layer Style", "Clear Layer Style", "Global Light…", "Hide All Effects", "Scale Effects…"]
    {
        h.get_by_label(label);
    }
    h.get_by_label("Create Layer").click();
    h.run_steps(4);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.active_layer, Some(a), "the menu acts on the right-clicked layer");
    assert!(effects(&h, a).is_empty(), "the effects became layers");
    let names: Vec<&str> = st.doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains(&"a's Drop Shadow") && names.contains(&"a's Color Overlay"), "{names:?}");
    assert!(h.query_by_label("Create Layer").is_none(), "the menu closed");
}
