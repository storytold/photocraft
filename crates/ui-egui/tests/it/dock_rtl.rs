//! The dock mirrors in a right-to-left UI: the docked panels move to the reading end (the
//! left), the canvas keeps its own direction.
//!
//! The twin switches the language through the real preference path, so only the direction
//! changes.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, control, dock::Group, layer_row_ui};
use serde_json::json;

/// The app with a document and the Layers panel alone in the dock.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, Default::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = control::ControlRequest::new("ui.set", json!({"theme": "promedium", "dockWidth": 290.0}));
    control::handle(h.state_mut(), &ctx, &req);
    // Every other dock group collapses to its tab strip, so the Layers rows have the height.
    for group in [Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History, Group::Brushes] {
        h.state_mut().ui.dock.set_collapsed(group, true);
    }
    h.run_steps(8);
    h
}

/// Where the Layers panel's rows are: the dock's side of the window.
fn dock_side(h: &Harness<'_, PhotocraftApp>) -> f32 {
    let rows = layer_row_ui::recorded(&h.ctx);
    let row = rows.first().unwrap_or_else(|| panic!("the Layers panel drew a row: {rows:?}"));
    row.row.center().x
}

/// The dock sits on the right in a left-to-right UI and mirrors to the left in an Arabic one.
#[test]
fn an_arabic_dock_mirrors_to_the_left() {
    let mut h = harness();
    let window = h.ctx.content_rect();
    assert!(dock_side(&h) > window.center().x, "the dock is on the right: {} in {window:?}", dock_side(&h));
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "ar"})).expect("Arabic is registered");
    h.run_steps(4);
    assert!(dock_side(&h) < window.center().x, "the dock mirrors to the left: {} in {window:?}", dock_side(&h));
}
