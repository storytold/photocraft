//! #13361: the Triangle tool's gear: Unconstrained / Equilateral / Fixed Size / Proportional and
//! From Center, as the reference application measures them on an 800 × 600 document (the bounding
//! box of the triangle's anchors). ⇧ during a drag turns an active Equilateral off and ⌥ an active
//! From Center, instead of adding to them. The popover and the drag geometry are the framework's
//! (storytold/craft-libs); these tests drive them through the canvas and the options bar.

use craft::ui::dialog::StackDialog as _;
use egui::Modifiers;
use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::rectangle_gear_tests::{app, drag, layer_count};
use super::test_input::harness;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

const GEAR: &str = "Set additional shape and path options";
const D: [f64; 2] = [400.0, 300.0];
const NONE: Modifiers = Modifiers::NONE;

/// The bounding box `[x0, y0, x1, y1]` of the anchors of the shape a drag with `tool` and options
/// `set` draws, with `held` pressed after the drag starts, to hundredths as the issue gives it.
fn drawn(tool: Tool, set: impl FnOnce(&mut ToolOptions), from: [f64; 2], to: [f64; 2], held: Modifiers) -> [f64; 4] {
    let mut app = app(tool, set);
    let layers = layer_count(&app);
    drag(&mut app, from, to, NONE, held);
    assert_eq!(layer_count(&app), layers + 1, "the drag draws one shape layer");
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    let knots = info["path"]["subpaths"][0]["knots"].as_array().unwrap_or_else(|| panic!("a path: {info}"));
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for k in knots {
        let (x, y) = (k["anchor"][0].as_f64().unwrap(), k["anchor"][1].as_f64().unwrap());
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b.map(|v| (v * 100.0).round() / 100.0)
}

/// The Triangle tool's box for a drag.
fn triangle(set: impl FnOnce(&mut ToolOptions), to: [f64; 2], held: Modifiers) -> [f64; 4] {
    drawn(Tool::Triangle, set, D, to, held)
}

/// The Triangle tool with the gear's `constraint`.
fn constraint(c: &'static str) -> impl FnOnce(&mut ToolOptions) {
    move |o| o.triangle_constraint = c.into()
}

#[test]
fn equilateral_draws_the_triangle_in_the_circle_of_the_shorter_side() {
    // D1 and D1b: ⇧'s triangle, kept on, with its exact corners.
    assert_eq!(triangle(constraint("square"), [600.0, 400.0], NONE), [406.7, 300.0, 493.3, 375.0]);
    assert_eq!(triangle(constraint("square"), [450.0, 500.0], NONE), [403.35, 300.0, 446.65, 337.5]);
}

#[test]
fn shift_during_the_drag_turns_equilateral_off_and_still_constrains_an_unconstrained_one() {
    // D6: the triangle fills the drag again.
    assert_eq!(triangle(constraint("square"), [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 600.0, 400.0]);
    // Unconstrained, ⇧ still draws the equilateral triangle (#13351), and a plain drag fills its box.
    assert_eq!(triangle(constraint("unconstrained"), [600.0, 400.0], Modifiers::SHIFT), [406.7, 300.0, 493.3, 375.0]);
    assert_eq!(triangle(constraint("unconstrained"), [600.0, 400.0], NONE), [400.0, 300.0, 600.0, 400.0]);
}

/// Fixed Size `w` × `h`.
fn fixed(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.triangle_constraint, o.triangle_fixed_width, o.triangle_fixed_height) = ("fixedSize".into(), w, h)
}

#[test]
fn fixed_size_puts_its_top_left_corner_at_the_release_point() {
    // D2, and a drag the other way.
    assert_eq!(triangle(fixed(120.0, 60.0), [600.0, 400.0], NONE), [600.0, 400.0, 720.0, 460.0]);
    assert_eq!(drawn(Tool::Triangle, fixed(120.0, 60.0), [600.0, 400.0], D, NONE), [400.0, 300.0, 520.0, 360.0]);
    // ⇧ does not make a fixed size equilateral; not measured, so the size is kept.
    assert_eq!(triangle(fixed(120.0, 60.0), [600.0, 400.0], Modifiers::SHIFT), [600.0, 400.0, 720.0, 460.0]);
}

#[test]
fn a_click_with_fixed_size_still_opens_create_triangle() {
    let mut app = app(Tool::Triangle, fixed(120.0, 60.0));
    let layers = layer_count(&app);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, NONE);
    let dialog = app.framework.dialogs.get::<craft::draw::ui::shape_drag::CreateShapeDialog>().expect("one Create Triangle dialog");
    assert_eq!(dialog.options().kind, craft::draw::ui::shape_drag::ShapeKind::Triangle);
    assert_eq!(dialog.options().size, (100.0, 100.0));
    assert_eq!(layer_count(&app), layers);
}

/// Proportional `w` : `h`.
fn ratio(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.triangle_constraint, o.triangle_ratio_width, o.triangle_ratio_height) = ("proportional".into(), w, h)
}

#[test]
fn proportional_draws_in_the_largest_box_of_its_ratio_inside_the_drag() {
    // D3 and D3b.
    assert_eq!(triangle(ratio(2.0, 1.0), [600.0, 500.0], NONE), [400.0, 300.0, 600.0, 400.0]);
    assert_eq!(triangle(ratio(2.0, 1.0), [450.0, 400.0], NONE), [400.0, 300.0, 450.0, 325.0]);
    // Towards the top left, still from the press point.
    assert_eq!(triangle(ratio(2.0, 1.0), [200.0, 100.0], NONE), [200.0, 200.0, 400.0, 300.0]);
    // ⇧ squares the drag first, as for the Rectangle; the triangle then fills the 2 : 1 box
    // instead of turning equilateral (not measured).
    assert_eq!(triangle(ratio(2.0, 1.0), [600.0, 550.0], Modifiers::SHIFT), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn from_center_draws_from_the_press_point_and_alt_turns_it_off() {
    let from_center = |o: &mut ToolOptions| o.triangle_from_center = true;
    // D4 and D5.
    assert_eq!(triangle(from_center, [500.0, 350.0], NONE), [300.0, 250.0, 500.0, 350.0]);
    assert_eq!(triangle(from_center, [500.0, 350.0], Modifiers::ALT), [400.0, 300.0, 500.0, 350.0]);
    // Off, ⌥ still draws from the centre.
    assert_eq!(triangle(|_| {}, [500.0, 350.0], Modifiers::ALT), [300.0, 250.0, 500.0, 350.0]);
    // Equilateral with From Center: the triangle in the circle about the press point.
    let both = |o: &mut ToolOptions| (o.triangle_constraint, o.triangle_from_center) = ("square".into(), true);
    assert_eq!(triangle(both, [600.0, 400.0], NONE), [313.4, 200.0, 486.6, 350.0]);
}

#[test]
fn the_triangle_and_the_rectangle_each_keep_their_own_gear() {
    let triangle_gear = |o: &mut ToolOptions| (o.triangle_constraint, o.triangle_from_center) = ("square".into(), true);
    assert_eq!(drawn(Tool::Rectangle, triangle_gear, D, [600.0, 400.0], NONE), [400.0, 300.0, 600.0, 400.0]);
    let rectangle_gear = |o: &mut ToolOptions| (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    assert_eq!(triangle(rectangle_gear, [600.0, 400.0], NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn path_mode_draws_the_equilateral_outline() {
    let mut app = app(Tool::Triangle, |o| (o.triangle_constraint, o.shape_mode) = ("square".into(), craft::draw::ui::VectorMode::Path));
    drag(&mut app, D, [600.0, 400.0], NONE, NONE);
    let path = app.session.active().unwrap().doc.work_path.clone().expect("the work path");
    let pts: Vec<_> = path.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| (k.anchor.x, k.anchor.y))).collect();
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
    let want = [(450.0, 300.0), (493.30, 375.0), (406.70, 375.0)];
    assert!(pts.len() == 3 && want.iter().all(|w| pts.iter().any(|p| near(*p, *w))), "{pts:?}");
}

// ---------------------------------------------------------------------------------------------
// The gear in the options bar

type H = egui_kittest::Harness<'static, Box<crate::PhotocraftApp>>;

/// Whether the radio or check box named `label` is on.
fn checked(h: &H, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True)
}

/// The Triangle tool's bar with its gear open.
fn open_gear() -> H {
    let mut h = harness(Tool::Triangle);
    h.get_by_label(GEAR).click();
    h.run_steps(4);
    h
}

#[test]
fn the_triangle_has_the_gear_after_its_path_arrangement() {
    let h = harness(Tool::Triangle);
    let (gear, arrange) = (h.get_by_label(GEAR).rect(), h.get_by_label("Path arrangement").rect());
    assert!(arrange.right() <= gear.left() && (gear.center().y - arrange.center().y).abs() < 4.0, "{gear:?} after {arrange:?}");
}

#[test]
fn the_gear_says_equilateral_and_its_picks_are_the_triangles() {
    let mut h = open_gear();
    let radios = ["Unconstrained", "Equilateral", "Fixed Size", "Proportional"];
    let tops: Vec<_> = radios.iter().map(|l| h.get_by_role_and_label(Role::RadioButton, l).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "top to bottom: {tops:?}");
    assert!(h.query_by_label("Square").is_none(), "the Triangle's says Equilateral");
    assert_eq!(radios.map(|l| checked(&h, l)), [true, false, false, false], "Unconstrained by default");
    assert!(!checked(&h, "From Center"));

    h.get_by_role_and_label(Role::RadioButton, "Equilateral").click();
    h.run_steps(3);
    h.get_by_label("From Center").click();
    h.run_steps(3);
    let o = &h.state().ui.tool_options;
    assert_eq!((o.triangle_constraint.as_str(), o.triangle_from_center), ("square", true));
    assert_eq!((o.rect_constraint.as_str(), o.rect_from_center), ("unconstrained", false), "the Rectangle's gear is untouched");

    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.query_by_label("From Center").is_none(), "Escape closes the popover");
    assert_eq!(h.state().ui.tool_options.triangle_constraint, "square", "and keeps the pick");
}

#[test]
fn the_settings_are_saved_and_reset_tool_restores_them() {
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    let saved = json!({"triangle_constraint": "unconstrained", "triangle_fixed_width": 0.0, "triangle_fixed_height": 0.0,
        "triangle_ratio_width": 1.0, "triangle_ratio_height": 1.0, "triangle_from_center": false});
    for (k, want) in saved.as_object().unwrap() {
        assert_eq!(&v[k], want, "{k}");
    }
    v["triangle_constraint"] = json!("square");
    v["triangle_from_center"] = json!(true);
    let on: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert_eq!((on.triangle_constraint.as_str(), on.triangle_from_center), ("square", true));
    for k in saved.as_object().unwrap().keys() {
        v.as_object_mut().unwrap().remove(k);
    }
    let old: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!((old.triangle_constraint.as_str(), old.triangle_ratio_width, old.triangle_from_center), ("unconstrained", 1.0, false));

    // Reset Tool: the Rectangle's leaves the Triangle's gear alone; the Triangle's restores it and
    // leaves the Rectangle's alone.
    let ctx = egui::Context::default();
    let mut app = app(Tool::Rectangle, |o| {
        (o.triangle_constraint, o.triangle_fixed_width, o.triangle_from_center) = ("fixedSize".into(), 120.0, true);
        (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    });
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    assert_eq!((app.ui.tool_options.triangle_constraint.as_str(), app.ui.tool_options.triangle_from_center), ("fixedSize", true));
    (app.ui.tool_options.rect_constraint, app.ui.tool_options.rect_from_center) = ("square".into(), true);
    app.ui.tool = Tool::Triangle;
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!((o.triangle_constraint.as_str(), o.triangle_from_center), ("unconstrained", false));
    assert_eq!((o.triangle_fixed_width, o.triangle_fixed_height, o.triangle_ratio_width, o.triangle_ratio_height), (0.0, 0.0, 1.0, 1.0));
    assert_eq!((o.rect_constraint.as_str(), o.rect_from_center), ("square", true));
}

// ---------------------------------------------------------------------------------------------
// Create Triangle

/// A click with the Triangle tool, then OK in Create Triangle with From Center set to `from_center`; the tool
/// options after.
fn create_triangle_ok(set: impl FnOnce(&mut ToolOptions), from_center: bool) -> ToolOptions {
    let mut app = app(Tool::Triangle, set);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, NONE);
    let id = app.framework.dialogs.get::<craft::draw::ui::shape_drag::CreateShapeDialog>().expect("Create Triangle").id();
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "fromCenter", json!(from_center)).unwrap();
    let layers = layer_count(&app);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert_eq!(layer_count(&app), layers + 1, "OK draws the triangle");
    app.ui.tool_options.clone()
}

#[test]
fn create_triangle_ok_with_from_center_turns_it_on_in_the_gear() {
    let o = create_triangle_ok(|_| {}, true);
    assert_eq!((o.triangle_constraint.as_str(), o.triangle_from_center), ("unconstrained", true));
    // Unchecked, it leaves the gear as it is (only the checked box was measured), and the
    // Rectangle's gear is never touched.
    let o = create_triangle_ok(constraint("fixedSize"), false);
    assert_eq!((o.triangle_constraint.as_str(), o.triangle_from_center), ("fixedSize", false));
    let o = create_triangle_ok(|o| o.triangle_from_center = true, false);
    assert!(o.triangle_from_center);
    assert_eq!((o.rect_constraint.as_str(), o.rect_from_center), ("unconstrained", false));
}
