//! #16867: the Star tool's number-of-points field sets the next star's points, whole numbers 3 to 20 (a value typed
//! past either end becomes the closest one with the reference's "An integer between 3 and 20 is required. Closest
//! value inserted." alert), 5 to start and after Reset Tool, and kept apart from the Polygon tool's sides.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const ALERT: &str = "An integer between 3 and 20 is required. Closest value inserted.";

/// The options bar of `tool`, with the dialog host that draws the alerts.
fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Types `text` into the Sides field, then Tabs out of it.
fn type_sides(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides")).expect("the Sides field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// The messages of the open alerts.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// A Star tool drag from 400,300 to 501,401 on the canvas: the live shape it makes, and its anchor count.
fn drag_star(app: &mut PhotocraftApp) -> (LiveShape, usize) {
    use crate::canvas::testing;
    app.ui.tool = Tool::Star;
    app.sync_views();
    testing::press(app, [400.0, 300.0], egui::Modifiers::NONE);
    testing::move_to(app, [450.0, 350.0], egui::Modifiers::NONE);
    testing::move_to(app, [501.0, 401.0], egui::Modifiers::NONE);
    testing::release(app, [501.0, 401.0], egui::Modifiers::NONE);
    let st = app.session.active().expect("a document");
    let layer = st.doc.layer(st.active_layer.expect("an active layer")).expect("the layer");
    let LayerContent::Shape(shape) = &layer.content else { panic!("the drag makes a shape layer, got {:?}", layer.name) };
    let live = shape.live.clone().expect("a live star");
    let anchors = photocraft_vector::shapes::live_path(&live).subpaths.iter().map(|s| s.knots.len()).sum();
    (live, anchors)
}

#[test]
fn the_field_sets_the_next_stars_points() {
    let mut h = bar(Tool::Star);
    type_sides(&mut h, "8");
    assert_eq!(h.state().ui.tool_options.star_points, 8);
    assert!(alerts(&h).is_empty(), "8 is in range");
    let (live, anchors) = drag_star(h.state_mut());
    assert!(matches!(live, LiveShape::Polygon { sides: 8, .. }), "an 8-point star, got {live:?}");
    assert_eq!(anchors, 16, "8 points give 16 anchors");
}

#[test]
fn points_past_the_range_take_the_closest_value_with_the_3_to_20_alert() {
    for (typed, kept) in [("25", 20), ("2", 3), ("21", 20), ("100", 20), ("1000", 20), ("0", 3)] {
        let mut h = bar(Tool::Star);
        type_sides(&mut h, typed);
        assert_eq!(h.state().ui.tool_options.star_points, kept, "{typed} keeps {kept}");
        assert_eq!(alerts(&h), vec![ALERT.to_string()], "{typed}: one alert with the star's range");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(h.state().ui.tool_options.star_points, kept, "{typed}: the value stays after OK");
    }
}

#[test]
fn points_in_range_are_kept_without_an_alert() {
    for (typed, kept) in [("3", 3), ("6", 6), ("20", 20)] {
        let mut h = bar(Tool::Star);
        type_sides(&mut h, typed);
        assert_eq!(h.state().ui.tool_options.star_points, kept, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn star_points_and_polygon_sides_are_kept_apart() {
    let mut h = bar(Tool::Star);
    type_sides(&mut h, "8");
    assert_eq!((h.state().ui.tool_options.star_points, h.state().ui.tool_options.polygon_sides), (8, 5), "the star's points leave the sides");

    let mut h = bar(Tool::Polygon);
    type_sides(&mut h, "30");
    assert_eq!((h.state().ui.tool_options.polygon_sides, h.state().ui.tool_options.star_points), (30, 5), "the sides leave the star's points");
    assert!(alerts(&h).is_empty(), "30 sides are in the Polygon tool's range");
    let (live, _) = drag_star(h.state_mut());
    assert!(matches!(live, LiveShape::Polygon { sides: 5, .. }), "the next star keeps 5 points, got {live:?}");
}

#[test]
fn the_field_shows_the_stars_points() {
    let mut h = bar(Tool::Star);
    h.state_mut().ui.tool_options.polygon_sides = 9;
    h.state_mut().ui.tool_options.star_points = 7;
    h.run_steps(2);
    let shown =
        h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides")).and_then(|n| n.accesskit_node().numeric_value());
    assert_eq!(shown, Some(7.0), "the Star tool's field shows its points, not the Polygon tool's sides");
}

#[test]
fn reset_tool_gives_5_points() {
    let mut o = crate::state::ToolOptions::default();
    assert_eq!(o.star_points, 5, "5 to start");
    o.star_points = 12;
    crate::tools::tool_reset::reset_options(&mut o, Tool::Star);
    assert_eq!(o.star_points, 5, "Reset Tool gives 5");
}

#[test]
fn an_arrow_step_past_20_stops_there_without_an_alert() {
    assert_eq!(super::star_points(21, 20, true), (Some(20), false), "a step from 20 stops at 20");
    assert_eq!(super::star_points(21, 8, true), (Some(20), true), "21 typed over 8 alerts");
    assert_eq!(super::star_points(40, 20, false), (None, false), "a running gesture past 20 keeps the points");
    assert_eq!(super::star_points(12, 5, true), (Some(12), false), "in range");
    assert_eq!(super::star_points(12, 5, false), (Some(12), false), "in range while the gesture runs");
}
