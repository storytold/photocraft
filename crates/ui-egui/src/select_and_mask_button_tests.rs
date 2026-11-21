//! #7520: the options bar's Select and Mask… button follows the command `select.selectAndMask`
//! (it maps to `select.refineEdge`): greyed out while nothing is selected, live after Select All.
use crate::PhotocraftApp;
use crate::state::Tool;
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

fn harness(tool: Tool, pixels_per_point: f32) -> Harness<'static, Box<PhotocraftApp>> {
    // The whole app: the button's label and enabled state come from the command state its frame publishes.
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(pixels_per_point).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

fn button<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Select and Mask…")
}

#[test]
fn select_and_mask_follows_the_selection() {
    let mut h = harness(Tool::RectMarquee, 1.0);
    assert!(button(&h).accesskit_node().is_disabled(), "the button is greyed out with no selection");
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(4);
    assert!(!button(&h).accesskit_node().is_disabled(), "the button is live with a selection");
}

/// #15576: Object Selection offers the same action as the Select menu, from the options beside Select Subject.
#[test]
fn object_selection_opens_select_and_mask_from_its_options() {
    let mut h = harness(Tool::ObjectSelection, 1.25);
    assert!(button(&h).accesskit_node().is_disabled(), "no selection disables the action");
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(4);

    let button = button(&h);
    assert!(!button.accesskit_node().is_disabled(), "an active selection enables the action");
    let subject = h.get_by_role_and_label(egui::accesskit::Role::Button, "Select Subject");
    assert!(button.rect().left() >= subject.rect().right(), "the action follows Select Subject");
    let pos = button.rect().center();
    h.hover_at(pos);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(8);
    assert_eq!(crate::distort_ui::modal_workspace(h.state()), Some("select.selectAndMask"), "a pointer click opens the hosted workspace");
}
