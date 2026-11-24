//! #13306: the Polygon tool's gear: Unconstrained / Symmetric / Fixed Size / Proportional /
//! Freeform and From Center, as the reference application measures them on an 800 × 600 document
//! with 5 sides. ⇧ during a drag turns an active Symmetric off; ⌥ during a Freeform drag centres
//! the polygon on the press point. The popover and the drag geometry are the framework's
//! (storytold/craft-libs); these tests drive them through the canvas and the options bar.

use egui::Modifiers;
use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::rectangle_gear_tests::{app, drag, layer_count};
use super::test_input::harness;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

const GEAR: &str = "Set additional shape and path options";
const D: [f64; 2] = [400.0, 300.0];
const NONE: Modifiers = Modifiers::NONE;
/// G8: Freeform 400,300 → 600,400, centred on 500,350 with its first vertex at the release point.
const G8: [(f64, f64); 5] = [(600.0, 400.0), (483.35, 460.56), (389.71, 368.33), (448.49, 250.77), (578.45, 270.35)];
/// H2: Freeform 400,300 → 600,380, centred on 500,340.
const H2: [(f64, f64); 5] = [(600.0, 380.0), (492.86, 447.47), (395.59, 366.42), (442.61, 248.86), (568.94, 257.26)];
/// H3: the same drag centred on the press point (circumradius 215.41).
const H3: [(f64, f64); 5] = [(600.0, 380.0), (385.72, 514.93), (191.17, 352.84), (285.22, 117.72), (537.89, 134.51)];

/// `shape.info` of the shape a Polygon-tool drag with options `set` draws, with `held` pressed
/// after the drag starts.
fn drawn(set: impl FnOnce(&mut ToolOptions), to: [f64; 2], held: Modifiers) -> Value {
    let mut app = app(Tool::Polygon, set);
    let layers = layer_count(&app);
    drag(&mut app, D, to, NONE, held);
    assert_eq!(layer_count(&app), layers + 1, "the drag draws one shape layer");
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.session.execute("shape.info", json!({"layer": id})).unwrap()
}

/// The anchors of a drag's polygon, in path order.
fn anchors(set: impl FnOnce(&mut ToolOptions), to: [f64; 2], held: Modifiers) -> Vec<(f64, f64)> {
    let info = drawn(set, to, held);
    let knots = info["path"]["subpaths"][0]["knots"].as_array().unwrap_or_else(|| panic!("a path: {info}"));
    knots.iter().map(|k| (k["anchor"][0].as_f64().unwrap(), k["anchor"][1].as_f64().unwrap())).collect()
}

/// The live polygon's box `[x, y, w, h]` of a drag (the box the polygon is drawn in).
fn live_box(set: impl FnOnce(&mut ToolOptions), to: [f64; 2], held: Modifiers) -> [f64; 4] {
    let info = drawn(set, to, held);
    assert_eq!(info["live"]["kind"], "polygon", "{info}");
    serde_json::from_value(info["live"]["rect"].clone()).unwrap_or_else(|_| panic!("a live box: {}", info["live"]))
}

/// Asserts `got` are `want`, in order, to 0.01 px.
fn same(got: &[(f64, f64)], want: &[(f64, f64)]) {
    let near = |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).abs() <= 0.01 && (a.1 - b.1).abs() <= 0.01;
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(g, w)| near(g, w)), "{got:?} vs {want:?}");
}

/// The Polygon tool with the gear's `constraint`.
fn constraint(c: &'static str) -> impl FnOnce(&mut ToolOptions) {
    move |o| o.polygon_constraint = c.into()
}

#[test]
fn symmetric_draws_the_regular_polygon_in_the_circle_of_the_shorter_side() {
    // G7: centre 450,350, circumradius 50.
    let want = [(450.0, 300.0), (497.55, 334.55), (479.39, 390.45), (420.61, 390.45), (402.45, 334.55)];
    same(&anchors(constraint("square"), [600.0, 400.0], NONE), &want);
}

#[test]
fn shift_during_the_drag_turns_symmetric_off() {
    // H6: the polygon is drawn in the whole drag box again.
    assert_eq!(live_box(constraint("square"), [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 200.0, 100.0]);
    // Unconstrained, ⇧ still squares the box, as before.
    assert_eq!(live_box(constraint("unconstrained"), [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 100.0, 100.0]);
    assert_eq!(live_box(constraint("unconstrained"), [600.0, 400.0], NONE), [400.0, 300.0, 200.0, 100.0]);
}

#[test]
fn freeform_centres_the_polygon_on_the_drag_with_a_vertex_at_the_release_point() {
    // G8: centre 500,350, the first vertex at the release point, the others clockwise.
    same(&anchors(constraint("freeform"), [600.0, 400.0], NONE), &G8);
    // H2: centre 500,340, circumradius 107.70.
    same(&anchors(constraint("freeform"), [600.0, 380.0], NONE), &H2);
}

#[test]
fn alt_during_a_freeform_drag_centres_it_on_the_press_point() {
    // H3.
    same(&anchors(constraint("freeform"), [600.0, 380.0], Modifiers::ALT), &H3);
    // From Center does the same, and ⌥ then turns it off (H2's polygon again).
    let from_center = || |o: &mut ToolOptions| (o.polygon_constraint, o.polygon_from_center) = ("freeform".into(), true);
    same(&anchors(from_center(), [600.0, 380.0], NONE), &H3);
    same(&anchors(from_center(), [600.0, 380.0], Modifiers::ALT), &H2);
}

#[test]
fn freeform_with_shift_turns_the_first_vertex_to_a_multiple_of_45_degrees() {
    // Measured for 400,300 → 600,380: the first vertex straight right of the centre, at the same
    // radius. The 45° step and the centre staying in the middle of the drag are this PR's choice.
    let want = [(607.70, 340.0), (533.28, 442.43), (412.87, 403.31), (412.87, 276.69), (533.28, 237.57)];
    same(&anchors(constraint("freeform"), [600.0, 380.0], Modifiers::SHIFT), &want);
    // A 63.4° drag turns to 45°.
    let got = anchors(constraint("freeform"), [500.0, 500.0], Modifiers::SHIFT);
    let (r, s) = (50f64.hypot(100.0), std::f64::consts::FRAC_1_SQRT_2);
    same(&got[..1], &[(450.0 + r * s, 400.0 + r * s)]);
}

#[test]
fn a_freeform_radius_sets_the_circumradius() {
    // Not measured: the drag places and turns the polygon, the Radius sizes it.
    let radius = |o: &mut ToolOptions| (o.polygon_constraint, o.polygon_freeform_radius) = ("freeform".into(), 50.0);
    let want = [(544.72, 372.36), (492.55, 399.44), (450.68, 358.20), (476.96, 305.62), (535.09, 314.38)];
    same(&anchors(radius, [600.0, 400.0], NONE), &want);
}

#[test]
fn a_freeform_polygon_is_a_polygon_layer_and_works_in_path_mode() {
    let mut shape = app(Tool::Polygon, |o| (o.polygon_constraint, o.polygon_sides) = ("freeform".into(), 6));
    drag(&mut shape, D, [600.0, 400.0], NONE, NONE);
    let st = shape.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(layer.name.starts_with("Polygon "), "{}", layer.name);
    let photocraft_doc::LayerContent::Shape(sh) = &layer.content else { panic!("a shape layer") };
    assert_eq!(sh.path.subpaths[0].knots.len(), 6, "the options bar's sides");

    let mut app = app(Tool::Polygon, |o| (o.polygon_constraint, o.shape_mode) = ("freeform".into(), craft::draw::ui::VectorMode::Path));
    let layers = layer_count(&app);
    drag(&mut app, D, [600.0, 400.0], NONE, NONE);
    assert_eq!(layer_count(&app), layers, "Path mode draws no layer");
    let path = app.session.active().unwrap().doc.work_path.clone().expect("the work path");
    let pts: Vec<_> = path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect();
    same(&pts, &G8);
}

#[test]
fn from_center_proportional_and_fixed_size_place_the_polygons_box() {
    // G9: the box about the press point, half its size the drag.
    assert_eq!(live_box(|o| o.polygon_from_center = true, [600.0, 400.0], NONE), [200.0, 200.0, 400.0, 200.0]);
    // G10 and H5: Proportional 2 : 1.
    let ratio = || |o: &mut ToolOptions| (o.polygon_constraint, o.polygon_ratio_width, o.polygon_ratio_height) = ("proportional".into(), 2.0, 1.0);
    assert_eq!(live_box(ratio(), [600.0, 400.0], NONE), [400.0, 300.0, 200.0, 100.0]);
    assert_eq!(live_box(ratio(), [600.0, 500.0], NONE), [400.0, 300.0, 200.0, 100.0]);
    // G11: Fixed Size 120 × 60 with its top-left corner at the release point.
    let fixed = |o: &mut ToolOptions| (o.polygon_constraint, o.polygon_fixed_width, o.polygon_fixed_height) = ("fixedSize".into(), 120.0, 60.0);
    assert_eq!(live_box(fixed, [600.0, 400.0], NONE), [600.0, 400.0, 120.0, 60.0]);
}

#[test]
fn a_click_with_fixed_size_still_opens_create_polygon() {
    // H4.
    let mut app = app(Tool::Polygon, |o| (o.polygon_constraint, o.polygon_fixed_width, o.polygon_fixed_height) = ("fixedSize".into(), 120.0, 60.0));
    let layers = layer_count(&app);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, NONE);
    let dialog = app.framework.dialogs.get::<craft::draw::ui::shape_drag::CreateShapeDialog>().expect("one Create Polygon dialog");
    assert_eq!(dialog.options().kind, craft::draw::ui::shape_drag::ShapeKind::Polygon);
    assert_eq!(dialog.options().size, (100.0, 100.0));
    assert_eq!(layer_count(&app), layers);
}

#[test]
fn each_tool_keeps_its_own_gear() {
    let polygon_gear = |o: &mut ToolOptions| (o.polygon_constraint, o.polygon_from_center) = ("square".into(), true);
    let mut app = app(Tool::Rectangle, polygon_gear);
    drag(&mut app, D, [600.0, 400.0], NONE, NONE);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    assert_eq!(app.session.execute("shape.info", json!({"layer": id})).unwrap()["live"]["rect"], json!([400.0, 300.0, 200.0, 100.0]));
    let others = |o: &mut ToolOptions| {
        (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
        (o.triangle_constraint, o.triangle_from_center) = ("square".into(), true);
    };
    assert_eq!(live_box(others, [600.0, 400.0], NONE), [400.0, 300.0, 200.0, 100.0]);
}

// ---------------------------------------------------------------------------------------------
// The gear in the options bar

type H = egui_kittest::Harness<'static, Box<crate::PhotocraftApp>>;

fn checked(h: &H, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn the_polygon_has_the_gear_after_its_path_arrangement() {
    let h = harness(Tool::Polygon);
    let (gear, arrange) = (h.get_by_label(GEAR).rect(), h.get_by_label("Path arrangement").rect());
    assert!(arrange.right() <= gear.left() && (gear.center().y - arrange.center().y).abs() < 4.0, "{gear:?} after {arrange:?}");
}

#[test]
fn the_gear_says_symmetric_adds_freeform_and_its_picks_are_the_polygons() {
    let mut h = harness(Tool::Polygon);
    h.get_by_label(GEAR).click();
    h.run_steps(4);
    let radios = ["Unconstrained", "Symmetric", "Fixed Size", "Proportional", "Freeform"];
    let tops: Vec<_> = radios.iter().map(|l| h.get_by_role_and_label(Role::RadioButton, l).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "top to bottom: {tops:?}");
    assert!(h.query_by_label("Square").is_none(), "the Polygon's says Symmetric");
    assert_eq!(radios.map(|l| checked(&h, l)), [true, false, false, false, false], "Unconstrained by default");
    assert!(h.get_by_label("Freeform Radius").accesskit_node().is_disabled(), "the Radius is greyed unless Freeform is chosen");

    h.get_by_role_and_label(Role::RadioButton, "Freeform").click();
    h.run_steps(3);
    assert!(!h.get_by_label("Freeform Radius").accesskit_node().is_disabled(), "Freeform enables its Radius");
    h.get_by_label("Freeform Radius").click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Freeform Radius").type_text("80");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.get_by_label("From Center").click();
    h.run_steps(3);
    let o = &h.state().ui.tool_options;
    assert_eq!((o.polygon_constraint.as_str(), o.polygon_freeform_radius, o.polygon_from_center), ("freeform", 80.0, true));
    assert_eq!((o.triangle_constraint.as_str(), o.rect_constraint.as_str()), ("unconstrained", "unconstrained"), "the other gears are untouched");

    h.get_by_role_and_label(Role::RadioButton, "Symmetric").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.query_by_label("From Center").is_none(), "Escape closes the popover");
    assert_eq!(h.state().ui.tool_options.polygon_constraint, "square", "and keeps Symmetric");
}

#[test]
fn the_settings_are_saved_and_reset_tool_restores_them() {
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    let saved = json!({"polygon_constraint": "unconstrained", "polygon_fixed_width": 0.0, "polygon_fixed_height": 0.0,
        "polygon_ratio_width": 1.0, "polygon_ratio_height": 1.0, "polygon_freeform_radius": 0.0, "polygon_from_center": false});
    for (k, want) in saved.as_object().unwrap() {
        assert_eq!(&v[k], want, "{k}");
    }
    v["polygon_constraint"] = json!("freeform");
    v["polygon_freeform_radius"] = json!(80.0);
    let on: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert_eq!((on.polygon_constraint.as_str(), on.polygon_freeform_radius), ("freeform", 80.0));
    for k in saved.as_object().unwrap().keys() {
        v.as_object_mut().unwrap().remove(k);
    }
    let old: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!((old.polygon_constraint.as_str(), old.polygon_ratio_width, old.polygon_freeform_radius), ("unconstrained", 1.0, 0.0));

    // Reset Tool: the Triangle's leaves the Polygon's gear alone; the Polygon's restores it and
    // leaves the Triangle's alone.
    let ctx = egui::Context::default();
    let mut app = app(Tool::Triangle, |o| {
        (o.polygon_constraint, o.polygon_freeform_radius, o.polygon_from_center) = ("freeform".into(), 80.0, true);
        (o.triangle_constraint, o.triangle_from_center) = ("square".into(), true);
    });
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    assert_eq!((app.ui.tool_options.polygon_constraint.as_str(), app.ui.tool_options.polygon_freeform_radius), ("freeform", 80.0));
    (app.ui.tool_options.triangle_constraint, app.ui.tool_options.triangle_from_center) = ("square".into(), true);
    app.ui.tool = Tool::Polygon;
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!((o.polygon_constraint.as_str(), o.polygon_freeform_radius, o.polygon_from_center), ("unconstrained", 0.0, false));
    assert_eq!((o.triangle_constraint.as_str(), o.triangle_from_center), ("square", true));
}
