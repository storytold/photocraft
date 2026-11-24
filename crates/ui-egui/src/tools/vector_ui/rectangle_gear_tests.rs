//! #13343: the Rectangle tool's gear: Unconstrained / Square / Fixed Size / Proportional and From
//! Center, as the reference application measures them on an 800 × 600 document. The gear's popup is the
//! framework's. ⇧ during a drag
//! turns an active Square off and ⌥ an active From Center, instead of adding to them.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

/// An 800 × 600 document with `tool` active and its options set by `set`.
fn app(tool: Tool, set: impl FnOnce(&mut ToolOptions)) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    set(&mut app.ui.tool_options);
    app
}

/// A canvas drag from `from` to `to`, pressed with `press` held and moved and released with `held`
/// (a key pressed after the drag starts), as the canvas sends it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, held: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, held);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, held);
}

/// The live rectangle a drag with `tool` and options `set` draws, as `[x0, y0, x1, y1]`.
fn drawn(tool: Tool, set: impl FnOnce(&mut ToolOptions), from: [f64; 2], to: [f64; 2], held: Modifiers) -> [f64; 4] {
    let mut app = app(tool, set);
    let layers = layer_count(&app);
    drag(&mut app, from, to, Modifiers::NONE, held);
    assert_eq!(layer_count(&app), layers + 1, "the drag draws one shape layer");
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    let r: [f64; 4] = serde_json::from_value(info["live"]["rect"].clone()).unwrap_or_else(|_| panic!("a live box: {}", info["live"]));
    [r[0], r[1], r[0] + r[2], r[1] + r[3]]
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// The Rectangle tool with the gear's `constraint`.
fn constraint(c: &'static str) -> impl FnOnce(&mut ToolOptions) {
    move |o| o.rect_constraint = c.into()
}

#[test]
fn square_takes_the_shorter_side_of_the_drag() {
    // D1 and D1b.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 500.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [450.0, 500.0], Modifiers::NONE), [400.0, 300.0, 450.0, 350.0]);
    // Towards the top left, the square still grows from the press point.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [200.0, 250.0], Modifiers::NONE), [350.0, 250.0, 400.0, 300.0]);
}

#[test]
fn shift_during_the_drag_turns_square_off_and_still_squares_an_unconstrained_one() {
    // D6.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 600.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("unconstrained"), [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 500.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("unconstrained"), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

/// Fixed Size `w` × `h`.
fn fixed(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.rect_constraint, o.rect_fixed_width, o.rect_fixed_height) = ("fixedSize".into(), w, h)
}

#[test]
fn fixed_size_puts_its_top_left_corner_at_the_release_point() {
    // D2, and the same from a drag the other way: the corner follows the pointer, not the press.
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 60.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 720.0, 460.0]);
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 60.0), [600.0, 400.0], [400.0, 300.0], Modifiers::NONE), [400.0, 300.0, 520.0, 360.0]);
    // A field left empty (as both start) draws the drag as Unconstrained; not measured.
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 0.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn a_click_with_fixed_size_still_opens_create_rectangle() {
    // D2b: the dialog, and no fixed-size rectangle dropped.
    let mut app = app(Tool::Rectangle, fixed(120.0, 60.0));
    let layers = layer_count(&app);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, Modifiers::NONE);
    let dialog = app.framework.dialogs.get::<craft::draw::ui::shape_drag::CreateShapeDialog>().expect("one Create Rectangle dialog");
    assert_eq!(dialog.options().size, (100.0, 100.0));
    assert_eq!(layer_count(&app), layers);
}

/// Proportional `w` : `h`.
fn ratio(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.rect_constraint, o.rect_ratio_width, o.rect_ratio_height) = ("proportional".into(), w, h)
}

#[test]
fn proportional_draws_the_largest_box_of_its_ratio_inside_the_drag() {
    // D3 and D3b.
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [600.0, 500.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [450.0, 400.0], Modifiers::NONE), [400.0, 300.0, 450.0, 325.0]);
    // From the press point towards the top left, and a ratio that is not whole.
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [200.0, 100.0], Modifiers::NONE), [200.0, 200.0, 400.0, 300.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(1.5, 2.5), [100.0, 100.0], [400.0, 600.0], Modifiers::NONE), [100.0, 100.0, 400.0, 600.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(1.5, 2.5), [100.0, 100.0], [400.0, 400.0], Modifiers::NONE), [100.0, 100.0, 280.0, 400.0]);
}

#[test]
fn from_center_draws_from_the_press_point_and_alt_turns_it_off() {
    let from_center = |o: &mut ToolOptions| o.rect_from_center = true;
    // D4 and D5.
    assert_eq!(drawn(Tool::Rectangle, from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE), [300.0, 250.0, 500.0, 350.0]);
    assert_eq!(drawn(Tool::Rectangle, from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT), [400.0, 300.0, 500.0, 350.0]);
    // Off, ⌥ still draws from the centre.
    assert_eq!(drawn(Tool::Rectangle, |_| {}, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT), [300.0, 250.0, 500.0, 350.0]);
    // From Center with a constraint: the constrained box about the press point.
    let square_from_center = |o: &mut ToolOptions| (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    assert_eq!(drawn(Tool::Rectangle, square_from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE), [350.0, 250.0, 450.0, 350.0]);
}

#[test]
fn the_gear_settings_are_the_rectangles_alone() {
    let gear = |o: &mut ToolOptions| (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    assert_eq!(drawn(Tool::EllipseShape, gear, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn path_mode_draws_the_constrained_outline() {
    let mut app = app(Tool::Rectangle, |o| (o.rect_constraint, o.shape_mode) = ("square".into(), craft::draw::ui::VectorMode::Path));
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    let path = app.session.active().unwrap().doc.work_path.clone().expect("the work path");
    let pts: Vec<_> = path.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor)).collect();
    let xs = pts.iter().map(|p| p.x);
    let ys = pts.iter().map(|p| p.y);
    let bounds = [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)];
    assert_eq!(bounds, [400.0, 300.0, 500.0, 400.0]);
}
