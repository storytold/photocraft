//! The Gradient Overlay Angle field takes -360..=360 and the dial stays in -180..=180 (#14913).
use egui_kittest::kittest::Queryable;

use super::*;

const KIND: &str = "gradientOverlay";

/// Type `text` into the Angle field of the open dialog and tab out of it.
fn type_angle(kind: &str, text: &str) -> Value {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run(&format!("layer.layerStyle.{kind}"), json!({})).unwrap();
    let state = app.session.active().unwrap();
    let fields = initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some(kind), 120.0);
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1000.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            // The harness draws once before setup_context installs the app fonts.
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, fields),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle").click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(1);
    harness.key_press(egui::Key::Tab);
    harness.run_steps(3);
    let (app, fields) = harness.state_mut();
    confirm(app, fields).unwrap();
    let state = app.session.active().unwrap();
    let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
    serde_json::to_value(&layer.effects.items[0]).unwrap()
}

fn angle(kind: &str, text: &str) -> f64 {
    let fx = type_angle(kind, text);
    find_angle(&fx).unwrap_or(f64::NAN)
}

/// The stored angle, wherever the effect's snapshot keeps it.
fn find_angle(v: &Value) -> Option<f64> {
    match v {
        Value::Object(o) => o.get("angle").and_then(Value::as_f64).or_else(|| o.values().find_map(find_angle)),
        Value::Array(a) => a.iter().find_map(find_angle),
        _ => None,
    }
}

#[test]
fn typed_angles_up_to_360_are_kept_with_their_sign_and_360_is_stored_as_0() {
    assert_eq!(angle(KIND, "270"), 270.0);
    assert_eq!(angle(KIND, "-270"), -270.0);
    assert_eq!(angle(KIND, "-90"), -90.0);
    assert_eq!(angle(KIND, "360"), 0.0);
}

#[test]
fn values_past_the_range_are_clamped() {
    assert_eq!(angle(KIND, "400"), 0.0, "400 becomes 360, which is stored as 0");
    assert_eq!(angle(KIND, "-400"), -360.0);
}

#[test]
fn other_effects_keep_the_180_range() {
    assert_eq!(angle("dropShadow", "270"), 180.0);
    assert_eq!(angle("innerShadow", "-270"), -180.0);
}
