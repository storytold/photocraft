//! #13463: a right-click on the canvas with the Custom Shape tool opens the Shape picker at the pointer, never a
//! context menu, in every measured state (a pixel layer, a custom shape layer under or away from the pointer, Shift
//! held, Path mode). Escape closes it; the Line tool keeps its own menu.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

use super::tests::{harness, right_click, screen_of};
use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

fn custom_shape() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::CustomShape;
    harness(app)
}

fn picker_open(h: &H) -> bool {
    crate::panels::preset_browser::shape_popover_at(h.state()).is_some()
}

fn menu_open(h: &H) -> bool {
    h.state().ui.canvas_tool_menu.is_some()
}

#[test]
fn a_right_click_opens_the_shape_picker_at_the_pointer_not_a_menu() {
    let mut h = custom_shape();
    let at = screen_of(h.state(), [150.0, 100.0]);
    right_click(&mut h, at, Modifiers::NONE);
    assert!(picker_open(&h), "the Shape picker opens");
    let opened = crate::panels::preset_browser::shape_popover_at(h.state()).unwrap();
    assert!((opened[0] - at.x).abs() < 1.0 && (opened[1] - at.y).abs() < 1.0, "at the pointer: {opened:?} vs {at:?}");
    assert!(!menu_open(&h), "no context menu");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!picker_open(&h), "Escape closes it");
}

#[test]
fn every_measured_state_opens_the_picker() {
    // Shift held.
    let mut h = custom_shape();
    let p = screen_of(h.state(), [150.0, 100.0]);
    right_click(&mut h, p, Modifiers::SHIFT);
    assert!(picker_open(&h) && !menu_open(&h), "Shift");
    // A custom shape layer, inside it and away from it.
    let mut h = custom_shape();
    let name = h.state().ui.presets_ui.shape().to_string();
    h.state_mut().run("shape.presets.place", json!({"preset": name, "rect": [100, 60, 100, 80]})).unwrap();
    h.run_steps(2);
    for at in [[150.0, 100.0], [20.0, 20.0]] {
        let p = screen_of(h.state(), at);
        right_click(&mut h, p, Modifiers::NONE);
        assert!(picker_open(&h) && !menu_open(&h), "shape layer, at {at:?}");
        h.key_press(Key::Escape);
        h.run_steps(2);
    }
    // Path mode.
    let mut h = custom_shape();
    h.state_mut().ui.tool_options.shape_mode = craft::draw::ui::VectorMode::Path;
    h.run_steps(2);
    let p = screen_of(h.state(), [150.0, 100.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert!(picker_open(&h) && !menu_open(&h), "Path mode");
}

#[test]
fn the_line_tool_keeps_its_menu() {
    let mut h = custom_shape();
    h.state_mut().ui.tool = Tool::Line;
    h.run_steps(2);
    let p = screen_of(h.state(), [150.0, 100.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert!(!picker_open(&h), "no Shape picker");
    assert!(menu_open(&h), "the Line tool's menu");
}

#[test]
fn a_pick_in_the_popover_sets_the_tools_shape() {
    use egui_kittest::kittest::Queryable;
    let mut h = custom_shape();
    let current = h.state().ui.presets_ui.shape().to_string();
    let other = photocraft_engine::presets::shapes::all_groups(&h.state().session)
        .iter()
        .flat_map(|g| g.items.iter().map(|s| s.name.clone()))
        .find(|n| *n != current)
        .expect("another shape");
    let p = screen_of(h.state(), [150.0, 100.0]);
    right_click(&mut h, p, Modifiers::NONE);
    h.query_all_by_label(&other).next().unwrap_or_else(|| panic!("{other} is listed")).click();
    h.run_steps(3);
    assert_eq!(h.state().ui.presets_ui.shape(), other, "the picked shape is the tool's");
}
