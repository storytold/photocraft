//! #1318: the Layers panel's Blend Mode dropdown takes the arrow keys once it is open, ahead of
//! the tool shortcuts (the Move tool's arrow-key nudge).

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

#[test]
fn arrow_keys_step_the_open_blend_mode_dropdown_without_nudging_the_layer() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(6);
    let layer = |h: &Harness<'_, PhotocraftApp>| {
        let st = h.state().session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        (l.blend, l.surface().unwrap().content_bounds())
    };
    let (blend, bounds) = layer(&h);
    assert_eq!(blend, BlendMode::Normal);
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some("Normal") || a.label().as_deref() == Some("Normal")
    };
    let combo = h.query_all_by_role(Role::ComboBox).find(|n| named(n));
    combo.expect("the Layers panel's Blend Mode dropdown").click();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    let (blend, moved) = layer(&h);
    assert_ne!(blend, BlendMode::Normal, "↓ picks the next mode");
    assert_eq!(moved, bounds, "the Move tool's nudge doesn't also take the key");
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(layer(&h).0, BlendMode::Normal);
}
