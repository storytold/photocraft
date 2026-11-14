//! #13277: the Path Selection bar's W, link and H. A live shape's size, right after the stroke
//! type; typing resizes the shape layer in one History step as the Properties panel's W / H do.
//! Empty and greyed with nothing selected or with subpaths of a saved path selected.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerContent;
use serde_json::json;

use super::rectangle_radius_target_tests::{bar_with, history_len, radii, type_into};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The Path Selection bar with a live 200 x 150 px rectangle shape (solid stroke) selected.
fn bar() -> Harness<'static, PhotocraftApp> {
    let mut h = bar_with(Tool::PathSelection);
    h.state_mut().run("shape.edit", json!({"rect": [0, 20, 200, 150], "stroke": {"width": 3, "color": "#000000"}})).unwrap();
    h.run_steps(3);
    h
}

/// The active layer's live shape: whether it is still live, and its path's width and height.
fn shape(h: &Harness<'static, PhotocraftApp>) -> (bool, f64, f64) {
    let st = h.state().session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    (sh.live.is_some(), x1 - x0, y1 - y0)
}

fn field<'h>(h: &'h Harness<'static, PhotocraftApp>, label: &str) -> Option<egui_kittest::Node<'h>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label))
}

/// The number a field shows ("200"; its unit is drawn beside it), or None without the field.
fn shown(h: &Harness<'static, PhotocraftApp>, label: &str) -> Option<String> {
    field(h, label).map(|n| n.accesskit_node().value().unwrap_or_default().trim().to_string())
}

fn assert_close(got: (bool, f64, f64), want: (f64, f64)) {
    assert!(got.0, "still a live shape");
    // The path is f64, the typed value an f32: compare to well under the field's rounding.
    assert!((got.1 - want.0).abs() < 1e-3 && (got.2 - want.1).abs() < 1e-3, "size {:?}, want {want:?}", (got.1, got.2));
}

#[test]
fn a_live_rectangle_shows_its_size_right_after_the_stroke_type() {
    let h = bar();
    assert_eq!(shown(&h, "W:").as_deref(), Some("200"));
    assert_eq!(shown(&h, "H:").as_deref(), Some("150"));
    let x = |n: egui_kittest::Node<'_>| n.rect().center().x;
    let (stroke, w, h_) = (x(field(&h, "Stroke:").unwrap()), x(field(&h, "W:").unwrap()), x(field(&h, "H:").unwrap()));
    let link = x(h.get_by_label("Link width and height"));
    let stroke_type = x(h.get_by_role_and_label(Role::Button, "Stroke Options"));
    assert!(stroke < stroke_type && stroke_type < w && w < link && link < h_, "Stroke {stroke}, type {stroke_type}, W {w}, link {link}, H {h_}");
    assert!(x(field(&h, "Radius:").unwrap()) > h_, "Radius stays after them");
}

#[test]
fn typing_a_width_resizes_the_shape_in_one_undoable_step() {
    let mut h = bar();
    // Unlinked, H stays.
    h.get_by_label("Link width and height").click();
    h.run_steps(2);
    let before = history_len(&h);
    type_into(&mut h, "W:", "120.5");
    assert_close(shape(&h), (120.5, 150.0));
    assert_eq!(history_len(&h), before + 1, "one History step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_close(shape(&h), (200.0, 150.0));
}

#[test]
fn linked_width_and_height_keep_the_shapes_ratio() {
    let mut h = bar();
    type_into(&mut h, "W:", "100");
    assert_close(shape(&h), (100.0, 75.0));
    assert_eq!(shown(&h, "H:").as_deref(), Some("75"));
    type_into(&mut h, "H:", "150");
    assert_close(shape(&h), (200.0, 150.0));
}

/// Types `text` into the field named `label` one key at a time, as a user does, then `end`.
fn type_keys(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str, end: egui::Key) {
    field(h, label).unwrap_or_else(|| panic!("the {label} field")).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for c in text.chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run_steps(1);
    }
    h.key_press(end);
    h.run_steps(3);
}

fn last_step(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    h.state().session.active().unwrap().history.entries().last().map(ToString::to_string)
}

/// A value is applied once it is entered, not at each key: "1" on the way to "100" would
/// otherwise squash the linked height to the 1 px minimum.
#[test]
fn a_width_typed_key_by_key_is_one_step_in_the_shapes_ratio() {
    let mut h = bar();
    let before = history_len(&h);
    type_keys(&mut h, "W:", "100", egui::Key::Enter);
    assert_close(shape(&h), (100.0, 75.0));
    assert_eq!(history_len(&h), before + 1, "one History step");
    assert_eq!(last_step(&h).as_deref(), Some("Free Transform"), "the step Properties' W / H make");
}

/// Unlinked, a key-by-key "300" scales only the width: the corner radius is not shrunk along
/// the way by the "3".
#[test]
fn an_unlinked_width_typed_key_by_key_keeps_the_radius() {
    let mut h = bar();
    h.get_by_label("Link width and height").click();
    h.run_steps(2);
    let before = history_len(&h);
    type_keys(&mut h, "W:", "300", egui::Key::Enter);
    assert_close(shape(&h), (300.0, 150.0));
    assert_eq!(radii(&h), vec![20.0]);
    assert_eq!(history_len(&h), before + 1, "one History step");
}

#[test]
fn width_then_tab_then_height_are_two_steps() {
    let mut h = bar();
    h.get_by_label("Link width and height").click();
    h.run_steps(2);
    let before = history_len(&h);
    type_keys(&mut h, "W:", "120", egui::Key::Tab);
    type_keys(&mut h, "H:", "100", egui::Key::Enter);
    assert_close(shape(&h), (120.0, 100.0));
    assert_eq!(history_len(&h), before + 2, "one History step each");
}

/// The fields with nothing selected: there, empty, greyed, and typing into them does nothing.
fn assert_greyed(mut h: Harness<'static, PhotocraftApp>) {
    for label in ["W:", "H:"] {
        let node = field(&h, label).unwrap_or_else(|| panic!("the {label} field"));
        assert!(node.accesskit_node().is_disabled(), "{label} greyed");
        assert_eq!(shown(&h, label).as_deref(), Some(""), "{label} empty");
    }
    let before = history_len(&h);
    field(&h, "W:").unwrap().click();
    h.run_steps(1);
    h.event(egui::Event::Text("50".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(history_len(&h), before, "no History step");
    assert_eq!(shown(&h, "W:").as_deref(), Some(""));
}

/// Direct Selection shares the bar without a shape layer (#13219), and was measured the same.
#[test]
fn with_nothing_selected_width_and_height_are_greyed_and_empty() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        let mut h = bar();
        h.state_mut().ui.tool = tool;
        h.state_mut().run("layer.new.layer", json!({})).unwrap();
        h.run_steps(3);
        assert_greyed(h);
    }
}

#[test]
fn with_subpaths_of_a_saved_path_selected_width_and_height_are_greyed_and_empty() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let square = |x: f64| json!({"closed": true, "op": "combine", "knots": [[x, 50.0], [x + 80.0, 50.0], [x + 80.0, 130.0], [x, 130.0]]});
    app.run("path.set", json!({"name": "Path 1", "path": {"subpaths": [square(40.0), square(200.0)]}})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::PathSelection;
    for (x, mods) in [(80.0, egui::Modifiers::NONE), (240.0, egui::Modifiers::SHIFT)] {
        tool_event(&mut app, ToolEvent::Down { x, y: 90.0, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Up { x, y: 90.0 }, mods);
    }
    assert_eq!(crate::path_components::selected(&app).map(|s| s.2.len()), Some(2), "both subpaths selected");
    app.sync_views();
    let mut h = Harness::builder().with_size(egui::vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    assert_greyed(h);
}

/// A saved path selected over an active live shape is the target: W and H stay greyed.
#[test]
fn a_saved_path_selected_over_a_live_shape_greys_width_and_height() {
    let mut h = bar();
    let square = json!({"closed": true, "op": "combine", "knots": [[240.0, 50.0], [320.0, 50.0], [320.0, 130.0], [240.0, 130.0]]});
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": {"subpaths": [square]}})).unwrap();
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(3);
    assert!(shape(&h).0, "the live shape stays the active layer");
    assert_greyed(h);
}

/// Direct Selection's W / H measure another box (#13219): not drawn here.
#[test]
fn direct_selection_draws_no_size_with_a_shape_layer() {
    let h = bar_with(Tool::DirectSelection);
    assert!(field(&h, "W:").is_none() && field(&h, "H:").is_none());
}
