//! #14843: Reset to Default on the Bevel & Emboss page replaced the page with the factory
//! defaults as they are, and those carry `angle: 120`. With Use Global Light on and Angle 45, the
//! field jumped to 120 while the dialog still applied the shared light 45 on OK, so field and
//! result disagreed. The reset now seeds the shared light angle into a globally lit effect, the
//! way a new instance starts from it (`fresh_params`).
use egui::vec2;
use egui_kittest::kittest::Queryable;

use super::*;

fn app_with_doc() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

/// The dialog body as it draws the selected page, so the real Reset to Default button is clicked.
fn draw_page(ui: &mut egui::Ui, state: &mut (PhotocraftApp, Map<String, Value>)) {
    // The harness draws once before setup_context installs the app fonts.
    if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
        body(&mut state.0, ui, &mut state.1);
    }
}

/// Use Global Light on, Angle 45 typed: Reset to Default keeps 45 and returns every other field
/// to its default; the other globally lit effect keeps the shared angle, and OK leaves the
/// document's Global Light at 45.
#[test]
fn reset_to_default_keeps_the_typed_shared_angle() {
    let mut app = app_with_doc();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({})).unwrap();
    let fields = {
        let state = app.session.active().unwrap();
        let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
        let mut f = initial_fields(layer, Some("bevelEmboss"), state.doc.global_light.angle);
        // What the Angle row does when 45 is typed, plus two edits the reset has to undo.
        set_param(&mut f, "fx1", "angle", json!(45.0));
        set_param(&mut f, "fx1", "depth", json!(40.0));
        set_param(&mut f, "fx1", "size", json!(9.0));
        assert_eq!(f["pendingGlobalLight"], json!(45.0), "fx1 and fx2 share 45");
        f
    };
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1200.0)).build_ui_state(draw_page, (app, fields));
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Reset to Default").click();
    harness.run_steps(3);
    let bevel = &harness.state().1["effects"][0]["params"];
    assert_eq!(bevel["angle"], json!(45.0), "the Angle field stays at the shared light angle");
    assert_eq!(bevel["useGlobalLight"], json!(true), "Use Global Light stays on");
    assert_eq!(harness.state().1["effects"][1]["params"]["angle"], json!(45.0), "the drop shadow keeps the shared angle");
    for (key, want) in [("depth", json!(100)), ("size", json!(5)), ("soften", json!(0)), ("altitude", json!(30)), ("style", json!("inner"))] {
        assert_eq!(bevel[key], want, "{key} went back to its default");
    }
    let (app, pending) = harness.state_mut();
    confirm(app, pending).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0, "OK sets the document's Global Light to 45");
}

/// The document's light already at 45 when the dialog opens, nothing typed: Reset to Default still
/// shows 45, and OK leaves the document at 45 instead of moving its Global Light to the factory 120.
#[test]
fn reset_to_default_does_not_move_the_documents_light() {
    let mut app = app_with_doc();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.globalLight", json!({"angle": 45})).unwrap();
    let fields = {
        let state = app.session.active().unwrap();
        let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
        let f = initial_fields(layer, Some("bevelEmboss"), state.doc.global_light.angle);
        assert_eq!(f["effects"][0]["params"]["angle"], json!(45.0), "the page shows the shared light 45");
        f
    };
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1200.0)).build_ui_state(draw_page, (app, fields));
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Reset to Default").click();
    harness.run_steps(3);
    let bevel = &harness.state().1["effects"][0]["params"];
    assert_eq!(bevel["angle"], json!(45.0), "the Angle field stays at the document's light angle");
    assert_eq!(bevel["size"], json!(5), "the other fields still go back to their defaults");
    let (app, pending) = harness.state_mut();
    confirm(app, pending).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0, "OK keeps the document's Global Light at 45");
}
