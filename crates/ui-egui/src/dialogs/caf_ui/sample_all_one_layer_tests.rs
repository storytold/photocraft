//! #14217: Sample All Layers in Content-Aware Fill is greyed out and stays off on a one-layer document.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{CafWorkspace, PhotocraftApp, control, open, show};

fn app_with_layers(extra_layers: usize) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 80, "height": 80, "depth": 8})).unwrap();
    for _ in 0..extra_layers {
        app.session.execute("layer.new.layer", json!({})).unwrap();
    }
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    app
}

fn ws(app: &PhotocraftApp) -> &CafWorkspace {
    app.distort.caf.as_ref().unwrap()
}

fn sample_all_checkbox<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some("Sample All Layers")).expect("Sample All Layers checkbox")
}

#[test]
fn one_layer_disables_sample_all_layers_checkbox_and_control_channel() {
    let mut app = app_with_layers(0);
    open(&mut app).unwrap();
    assert!(!ws(&app).opts.sample_all);

    let res = control(&mut app, &json!({"sampleAllLayers": true})).unwrap();
    assert_eq!(res["settings"]["sampleAllLayers"], false);
    assert!(!ws(&app).opts.sample_all);

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, app: &mut PhotocraftApp| show(app, ui.ctx()), app);
    h.run_steps(2);
    assert!(sample_all_checkbox(&h).accesskit_node().is_disabled());
    sample_all_checkbox(&h).click();
    h.run_steps(2);
    assert!(!ws(h.state()).opts.sample_all);
}

#[test]
fn multi_layer_keeps_sample_all_layers_enabled() {
    let mut app = app_with_layers(1);
    open(&mut app).unwrap();

    let res = control(&mut app, &json!({"sampleAllLayers": true})).unwrap();
    assert_eq!(res["settings"]["sampleAllLayers"], true);
    assert!(ws(&app).opts.sample_all);

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, app: &mut PhotocraftApp| show(app, ui.ctx()), app);
    h.run_steps(2);
    assert!(!sample_all_checkbox(&h).accesskit_node().is_disabled());
}
