//! The scroll wheel over a Blend Mode dropdown in the Layer Style dialog (Blending Options and
//! each effect, e.g. Drop Shadow) steps through the modes, as over the Layers panel's (#1747).

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A pixel layer with a drop shadow and the Layer Style dialog open on `page`.
fn harness(page: &'static str) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#cc3366"})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.layerStyle.replace", json!({"layer": layer.0, "effects": [{"kind": "dropShadow"}]})).unwrap();
        crate::layer_style::open_for_layer(&mut app, layer, Some(page)).unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn fields(h: &Harness<'_, PhotocraftApp>) -> serde_json::Map<String, Value> {
    h.state().ui.dialogs.last().unwrap().fields.clone()
}

/// The drop shadow's blend mode as the dialog holds it.
fn shadow_blend(h: &Harness<'_, PhotocraftApp>) -> String {
    let f = fields(h);
    let fx = f["effects"].as_array().unwrap().iter().find(|e| e["kind"] == "dropShadow").unwrap();
    fx["params"]["blend"].as_str().unwrap().to_string()
}

fn page_blend(h: &Harness<'_, PhotocraftApp>) -> String {
    fields(h)["p:blendingOptions"]["blend"].as_str().unwrap().to_string()
}

/// The dialog's dropdown showing `label`: the leftmost below the options bar, as the Layers panel (on the right)
/// shows the layer's own blend mode too.
fn dropdown(h: &Harness<'_, PhotocraftApp>, label: &str) -> Rect {
    h.query_all_by_role(Role::ComboBox)
        .filter(|n| n.query_by_label(label).is_some() || n.value().as_deref() == Some(label))
        .map(|n| n.rect())
        // Below the options bar, which has a Mode dropdown of its own.
        .filter(|r| r.top() > 100.0)
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("no dropdown showing {label}"))
}

/// `lines` wheel lines at `at` in one event: negative is down.
fn wheel(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, lines: f32) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(3);
}

#[test]
fn the_wheel_over_the_drop_shadow_blend_mode_steps_its_mode() {
    let mut h = harness("dropShadow");
    assert_eq!(shadow_blend(&h), "Multiply");
    let at = dropdown(&h, "Multiply").center();
    wheel(&mut h, at, -1.0);
    assert_eq!(shadow_blend(&h), "Color Burn", "a notch down picks the next mode");
    wheel(&mut h, at, 1.0);
    wheel(&mut h, at, 1.0);
    assert_eq!(shadow_blend(&h), "Darken", "two notches up go back two");
}

#[test]
fn the_wheel_over_the_blending_options_blend_mode_steps_its_mode() {
    let mut h = harness(crate::layer_style::BLENDING);
    assert_eq!(page_blend(&h), "Normal");
    let at = dropdown(&h, "Normal").center();
    wheel(&mut h, at, -1.0);
    assert_eq!(page_blend(&h), "Dissolve");
}
