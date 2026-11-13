//! #13355: the Triangle tool's options-bar Radius is its own px value (default 0) for the next
//! triangle. It shares the Rectangle Radius's 0..150000 range and alert, never edits the active
//! shape, and the Create Triangle dialog starts from it.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use super::RADIUS_RANGE_ALERT;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app
}

/// The options bar with `tool` active.
fn bar_with(app: PhotocraftApp, tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = app;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn bar() -> Harness<'static, PhotocraftApp> {
    bar_with(new_app(), Tool::Triangle)
}

/// Types `text` into Radius, then Tabs out of it.
fn type_radius(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

fn radius(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.triangle_corner_radius
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The live shape of the active layer.
fn live(app: &PhotocraftApp) -> LiveShape {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    sh.live.clone().unwrap()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn the_triangle_tool_shows_a_radius_field_in_range_without_an_alert() {
    for (typed, want) in [("0", 0.0), ("10", 10.0), ("5000", 5000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
        assert_eq!(h.state().ui.tool_options.corner_radius, 0.0, "{typed}: the Rectangle's radius is separate");
    }
}

#[test]
fn a_radius_past_the_range_takes_the_closest_value_and_alerts_once() {
    for (typed, want) in [("-5", 0.0), ("200000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![RADIUS_RANGE_ALERT.to_string()], "{typed}: one alert");
        assert_eq!(h.state().ui.tool_options.corner_radius, 0.0, "{typed}: the Rectangle's radius is separate");
    }
}

#[test]
fn text_that_is_not_a_number_keeps_the_previous_radius() {
    let mut h = bar();
    type_radius(&mut h, "12");
    type_radius(&mut h, "abc");
    assert_eq!(radius(&h), 12.0);
}

#[test]
fn the_radius_field_leaves_the_active_triangle_alone() {
    let mut app = new_app();
    app.run("shape.create", json!({"kind": "triangle", "rect": [100, 100, 200, 100], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    let before_live = live(&app);
    let before_history = history_len(&app);
    let mut h = bar_with(app, Tool::Triangle);
    type_radius(&mut h, "30");
    assert_eq!(radius(&h), 30.0);
    assert_eq!(live(h.state()), before_live, "the active triangle keeps its shape");
    assert_eq!(history_len(h.state()), before_history, "no History state");
}

#[test]
fn the_next_triangle_dragged_takes_the_radius() {
    let mut app = new_app();
    app.ui.tool = Tool::Triangle;
    app.ui.tool_options.triangle_corner_radius = 20.0;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    match live(&app) {
        LiveShape::Polygon { sides, corner_radius, .. } => assert_eq!((sides, corner_radius), (3, 20.0)),
        other => panic!("not a polygon: {other:?}"),
    }
}

#[test]
fn a_zero_radius_draws_a_sharp_triangle() {
    let mut app = new_app();
    app.ui.tool = Tool::Triangle;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    match live(&app) {
        LiveShape::Polygon { sides, corner_radius, .. } => assert_eq!((sides, corner_radius), (3, 0.0)),
        other => panic!("not a polygon: {other:?}"),
    }
}

#[test]
fn reset_tool_restores_a_zero_radius() {
    let mut o = crate::state::ToolOptions { triangle_corner_radius: 33.0, ..Default::default() };
    crate::tools::tool_reset::reset_options(&mut o, Tool::Triangle);
    assert_eq!(o.triangle_corner_radius, 0.0);
}

#[test]
fn the_create_triangle_dialog_starts_from_the_radius() {
    let mut app = new_app();
    app.ui.tool_options.triangle_corner_radius = 15.0;
    let id = crate::dialogs::shape_dialog::open(&mut app, Tool::Triangle, [200.0, 150.0]).expect("a dialog");
    let d = app.ui.dialog_mut(id).unwrap();
    assert_eq!(d.fields.get("cornerRadius").and_then(|v| v.as_f64()), Some(15.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    match live(&app) {
        LiveShape::Polygon { sides, corner_radius, .. } => assert_eq!((sides, corner_radius), (3, 15.0)),
        other => panic!("not a polygon: {other:?}"),
    }
}
