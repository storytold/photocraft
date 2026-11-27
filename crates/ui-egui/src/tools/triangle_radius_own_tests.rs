//! #13355: the Triangle tool's options-bar Radius (the framework's, since craft-libs#2165) is the Triangle's own:
//! it rounds the next triangle drawn and leaves the Rectangle's radius alone.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The options bar of an 800 × 600 document with the Triangle tool current.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Triangle;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// Clicks into Radius, selects its text and types `text`, then Tabs out of it.
fn enter_radius(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// A Triangle tool drag from `from` to `to`, released there.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn the_triangle_radius_rounds_the_next_triangle_and_leaves_the_rectangle_radius() {
    let mut h = bar();
    let rectangle_radius = h.state().ui.tool_options.corner_radius;
    enter_radius(&mut h, "20");
    let o = &h.state().ui.tool_options;
    assert!(o.triangle_corner_radius > 0.0, "the Triangle keeps the typed radius");
    assert_eq!(o.corner_radius, rectangle_radius, "the Rectangle's radius is not the Triangle's");

    drag(h.state_mut(), [400.0, 300.0], [600.0, 400.0]);
    let app = h.state();
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("the drag made no shape layer") };
    let Some(LiveShape::Triangle { corner_radius, .. }) = sh.live else { panic!("the drag made no live triangle: {:?}", sh.live) };
    assert!(corner_radius > 0.0, "the next triangle is rounded");
}
