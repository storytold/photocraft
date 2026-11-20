//! #14915: the options bar shows how to reposition a Gradient Overlay.
use crate::PhotocraftApp;
use crate::dialogs::canvas_input::{self, DialogPick};
use crate::state::Tool;
use egui::{accesskit::Role, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

pub(super) fn app_with_tool(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(1.25).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

pub(super) fn click_label(h: &mut Harness<'static, Box<PhotocraftApp>>, role: Role, label: &str) {
    let pos = h.get_by_role_and_label(role, label).rect().center();
    h.hover_at(pos);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(8);
}

pub(super) fn assert_move_hint(h: &Harness<'static, Box<PhotocraftApp>>, hint: &str) {
    assert!(h.query_by_label(hint).is_some(), "the active page describes its canvas drag");
    assert!(h.query_all_by_label("Move Tool").any(|node| node.rect().center().y < 80.0), "the Move icon appears in the options bar");
    assert!(h.query_by_label("Scroll All Windows").is_none(), "the Hand controls are replaced");
    assert!(h.query_all_by_label_contains("Auto-Select").next().is_none(), "the Move controls do not replace the hint");
}

#[test]
fn gradient_overlay_hint_replaces_the_hand_options() {
    const HINT: &str = "Click and drag to reposition the gradient.";
    let mut h = app_with_tool(Tool::Hand);
    assert!(h.query_by_label("Scroll All Windows").is_some());
    h.state_mut().run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let id = super::open(h.state_mut(), Some("gradientOverlay")).unwrap();
    h.run_steps(8);
    assert_move_hint(&h, HINT);
    assert!(matches!(canvas_input::armed(h.state()), Some(DialogPick::GradientDrag)));

    click_label(&mut h, Role::ListItem, "Blending Options");
    assert_eq!(super::tests::draft(h.state(), id).page, super::LayerStylePage::Blending);
    assert!(h.query_by_label(HINT).is_none(), "the hint is removed after a page change");
    assert!(h.query_by_label("Scroll All Windows").is_some(), "the Hand controls return");

    click_label(&mut h, Role::ListItem, "Gradient Overlay");
    assert_move_hint(&h, HINT);
    click_label(&mut h, Role::Button, "OK");
    assert!(h.query_by_label(HINT).is_none());
    assert!(h.query_by_label("Scroll All Windows").is_some());
    assert_eq!(h.state().ui.tool, Tool::Hand, "the prior tool is preserved after OK");
}

#[test]
fn gradient_overlay_on_an_empty_layer_keeps_the_hand_options() {
    let mut h = app_with_tool(Tool::Hand);
    super::open(h.state_mut(), Some("gradientOverlay")).unwrap();
    h.run_steps(8);
    assert!(h.query_by_label("Click and drag to reposition the gradient.").is_none());
    assert!(h.query_by_label("Scroll All Windows").is_some());
}

#[test]
fn gradient_overlay_keeps_the_move_options() {
    let mut h = app_with_tool(Tool::Move);
    super::open(h.state_mut(), Some("gradientOverlay")).unwrap();
    h.run_steps(8);
    assert!(h.query_by_label("Click and drag to reposition the gradient.").is_none());
    assert!(h.query_all_by_label_contains("Auto-Select").next().is_some());
}
