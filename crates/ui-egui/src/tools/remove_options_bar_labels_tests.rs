//! #12546: the Remove tool's checkboxes read "Sample all layers", "Remove after each stroke",
//! "Create new layer", left to right, each with the reference tooltip.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const BOXES: [(&str, &str); 3] = [
    ("Sample all layers", "Sample content from all visible layers"),
    ("Remove after each stroke", "Remove area after a single stroke"),
    ("Create new layer", "Create new layer with results"),
];

/// A settled options bar with the Remove tool.
fn remove_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Remove;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

/// The control named `label` with `role`, found by its accessibility role so tooltip text never matches.
fn control<'a>(h: &'a Harness<'static, PhotocraftApp>, role: Role, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("missing {role:?} {label:?}"))
}

fn checked(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    control(h, Role::CheckBox, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn the_checkboxes_read_the_reference_labels_left_to_right() {
    let h = remove_bar();
    let mut boxes: Vec<(f32, String)> =
        h.query_all_by_role(Role::CheckBox).map(|n| (n.rect().left(), n.accesskit_node().label().unwrap_or_default())).collect();
    boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let labels: Vec<&str> = boxes.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, BOXES.map(|(label, _)| label));
}

#[test]
fn defaults_show_only_remove_after_each_stroke_on_and_reset_tool_restores_them() {
    let mut h = remove_bar();
    let defaults = [false, true, false];
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults);
    for (label, _) in BOXES {
        control(&h, Role::CheckBox, label).click();
        h.run_steps(2);
    }
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults.map(|on| !on), "each click toggles its own box");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults);
}

#[test]
fn create_new_layer_greys_when_a_mask_or_colour_subset_is_targeted() {
    let mut h = remove_bar();
    assert!(!control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled());
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    h.run_steps(2);
    assert!(control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled(), "a targeted mask greys it");
    let app = h.state_mut();
    app.ui.mask_target = false;
    app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
    h.run_steps(2);
    assert!(control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled(), "a colour subset greys it");
    for (label, _) in &BOXES[..2] {
        assert!(!control(&h, Role::CheckBox, label).accesskit_node().is_disabled(), "{label} stays enabled");
    }
}

#[test]
fn the_boxes_ticked_on_the_bar_reach_the_removal_options() {
    let mut h = remove_bar();
    for label in ["Sample all layers", "Create new layer"] {
        control(&h, Role::CheckBox, label).click();
        h.run_steps(2);
    }
    let app = h.state_mut();
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 30.0, y: 18.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 42.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let (id, p) = app.session.journal.last().cloned().unwrap();
    assert_eq!((id.as_str(), p["sampleAllLayers"].as_bool(), p["newLayer"].as_bool()), ("paint.remove", Some(true), Some(true)));
}
