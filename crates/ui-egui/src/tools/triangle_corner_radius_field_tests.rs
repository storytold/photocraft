//! #13355: the Triangle tool's options-bar Radius is its own px value (default 0) for the next
//! triangle. It shares the Rectangle Radius's 0..150000 range and alert, never edits the active
//! shape, and the Create Triangle dialog starts from it.

use craft::draw::ui::shape_drag::CreateShapeDialog;
use craft::ui::dialog::StackDialog as _;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn new_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app
}

/// The options bar with `tool` active.
fn bar_with(app: Box<PhotocraftApp>, tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = app;
    app.ui.tool = tool;
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(1800.0, 300.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    bar_with(new_app(), Tool::Triangle)
}

/// The Radius fields the bar shows.
fn radius_fields(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.query_all_by_role(Role::SpinButton).filter(|n| n.accesskit_node().label().as_deref() == Some("Radius")).count()
}

/// Types `text` into Radius, then Tabs out of it.
fn type_radius(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// How many alerts are open.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    crate::dialogs::alerts::alerts(h.state()).len()
}

fn radius(h: &Harness<'static, Box<PhotocraftApp>>) -> f32 {
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

/// The anchor points of the active shape layer's outline.
fn anchors(app: &PhotocraftApp) -> Vec<(f64, f64)> {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    sh.path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()
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
        assert_eq!(radius_fields(&h), 1, "one Radius field on the Triangle bar");
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), 0, "{typed} is in range");
        assert_eq!(h.state().ui.tool_options.corner_radius, 0.0, "{typed}: the Rectangle's radius is separate");
    }
}

#[test]
fn a_radius_past_the_range_takes_the_closest_value_and_alerts_once() {
    for (typed, want) in [("-5", 0.0), ("200000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), 1, "{typed}: one alert");
        assert_eq!(h.state().ui.tool_options.corner_radius, 0.0, "{typed}: the Rectangle's radius is separate");
    }
}

#[test]
fn the_bar_shows_the_triangles_own_radius() {
    let mut app = new_app();
    app.ui.tool_options.corner_radius = 7.0;
    app.ui.tool_options.triangle_corner_radius = 15.0;
    let h = bar_with(app, Tool::Triangle);
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius")).expect("the Radius field");
    let shown = field.accesskit_node().value().map(|v| v.to_string());
    assert!(shown.as_deref().is_some_and(|v| v.starts_with("15")), "the Triangle's 15, not the Rectangle's 7: {shown:?}");
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
    app.ui.tool_options.corner_radius = 7.0;
    app.ui.tool_options.triangle_corner_radius = 20.0;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    match live(&app) {
        LiveShape::Triangle { corner_radius, .. } => assert_eq!(corner_radius, 20.0, "the Triangle's radius, not the Rectangle's"),
        other => panic!("not a triangle: {other:?}"),
    }
    // The arcs' ends on the triangle that fills the 400,300 to 600,400 box (#13350): two per rounded corner.
    let want = [(485.858, 314.142), (514.142, 314.142), (565.858, 365.858), (551.716, 400.0), (448.284, 400.0), (434.142, 365.858)];
    let got = anchors(&app);
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() < 1e-3 && (g.1 - w.1).abs() < 1e-3, "{got:?}");
    }
}

#[test]
fn a_zero_radius_draws_a_sharp_triangle() {
    let mut app = new_app();
    app.ui.tool = Tool::Triangle;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    match live(&app) {
        LiveShape::Triangle { corner_radius, .. } => assert_eq!(corner_radius, 0.0),
        other => panic!("not a triangle: {other:?}"),
    }
    assert_eq!(anchors(&app).len(), 3, "one anchor per sharp corner");
}

#[test]
fn reset_tool_restores_a_zero_radius() {
    let mut o = crate::state::ToolOptions { triangle_corner_radius: 33.0, ..Default::default() };
    crate::tools::tool_reset::reset_options(&mut o, Tool::Triangle);
    assert_eq!(o.triangle_corner_radius, 0.0);
}

#[test]
fn resetting_another_shape_tool_keeps_the_triangle_radius() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Polygon, Tool::Star, Tool::CustomShape] {
        let mut o = crate::state::ToolOptions { triangle_corner_radius: 33.0, ..Default::default() };
        crate::tools::tool_reset::reset_options(&mut o, tool);
        assert_eq!(o.triangle_corner_radius, 33.0, "{tool:?}");
    }
}

#[test]
fn a_corrupt_saved_radius_is_kept_in_range() {
    let mut o = crate::state::ToolOptions { triangle_corner_radius: f32::NAN, ..Default::default() };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!(o.triangle_corner_radius, 0.0, "NaN falls back to the default");
    let mut o = crate::state::ToolOptions { triangle_corner_radius: 9.0e9, ..Default::default() };
    crate::tool_memory::sanitize(&mut o);
    assert!((0.0..=150_000.0).contains(&o.triangle_corner_radius), "{}", o.triangle_corner_radius);
}

#[test]
fn the_create_triangle_dialog_starts_from_the_radius() {
    let mut app = new_app();
    app.ui.tool_options.corner_radius = 7.0;
    app.ui.tool_options.triangle_corner_radius = 15.0;
    let at = craft::behavior::kurbo::Point::new(200.0, 150.0);
    let id = crate::dialogs::shape_dialog::open(&mut app, craft::draw::ui::shape_drag::ShapeKind::Triangle, at).expect("a dialog");
    let fields = app.framework.dialogs.get::<CreateShapeDialog>().and_then(|d| d.json_fields()).expect("the dialog's fields");
    assert_eq!(fields["cornerRadius"], json!(15.0), "the Triangle's radius, not the Rectangle's");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    match live(&app) {
        LiveShape::Triangle { corner_radius, .. } => assert_eq!(corner_radius, 15.0),
        other => panic!("not a triangle: {other:?}"),
    }
}

#[test]
fn escape_on_the_triangle_radius_compares_the_triangles_own_radius() {
    use craft::draw::ui::shape::{ShapeAppearance, ShapeChange, ShapeToolSettings};
    use craft::draw::ui::shape_drag::ShapeKind;
    let mut settings = ShapeToolSettings::default();
    settings.corner_radius = 7.0;
    settings.triangle_corner_radius = 15.0;
    let appearance = ShapeAppearance::new(crate::shape_paint_ui::paint_of(None), crate::shape_paint_ui::paint_of(None), 1.0);
    let restore = ShapeChange::CornerRadius(15.0);
    assert!(super::already_shown(&restore, ShapeKind::Triangle, &appearance, &settings), "the bar already shows the Triangle's 15");
    assert!(!super::already_shown(&restore, ShapeKind::Rectangle, &appearance, &settings), "the Rectangle's radius is 7");
    let restore = ShapeChange::CornerRadius(7.0);
    assert!(!super::already_shown(&restore, ShapeKind::Triangle, &appearance, &settings), "the Triangle's radius is not 7");
}
