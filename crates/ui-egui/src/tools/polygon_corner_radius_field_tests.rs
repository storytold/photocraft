//! #13302: the Polygon tool's options bar has its own corner radius after Sides, in px (0 to
//! start, and after Reset Tool), in Shape and Path mode. It rounds the next polygon drawn, never
//! the active one, and shares the Rectangle Radius's 0..150000 range and its "closest value"
//! alert. Text that is not a number keeps the previous value, as the framework's number fields do.

use craft::draw::ui::VectorMode;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::vector::Knot;
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// An 800 × 600 document with the Polygon tool current.
fn new_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Polygon;
    app
}

/// The options bar drawn for `app`.
fn bar_with(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// The options-bar number field named `label`.
fn field_rect(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> Option<egui::Rect> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).map(|n| n.rect())
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

/// How many alerts are open.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    crate::dialogs::alerts::alerts(h.state()).len()
}

/// The Polygon tool's radius, and the Rectangle tool's, which it must leave alone.
fn radii(h: &Harness<'static, Box<PhotocraftApp>>) -> (f32, f32) {
    let o = &h.state().ui.tool_options;
    (o.polygon_corner_radius, o.corner_radius)
}

/// The active layer's live shape and outline knots.
fn active_shape(app: &PhotocraftApp) -> (LiveShape, Vec<Knot>) {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    (sh.live.clone().unwrap(), sh.path.subpaths[0].knots.clone())
}

/// A Polygon tool drag from `from` to `to`, released there.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

/// The issue's measured drag, 400,300 to 600,400, with the Polygon tool's radius at `radius`:
/// the new layer's live shape and outline knots.
fn drawn_with(radius: f32) -> (LiveShape, Vec<Knot>) {
    let mut app = new_app();
    app.ui.tool_options.polygon_corner_radius = radius;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    active_shape(&app)
}

/// Where each vertex of the sharp polygon `v` is rounded by an arc of radius `r` tangent to both
/// its edges: the points `r / tan(half the vertex angle)` from it along the edge it comes from and
/// the edge it goes to, at most half the shorter edge (a radius the shape cannot take is capped).
fn tangent_points(v: &[[f64; 2]], r: f64) -> Vec<[f64; 2]> {
    let n = v.len();
    let along = |from: [f64; 2], to: [f64; 2], t: f64| {
        let l = (to[0] - from[0]).hypot(to[1] - from[1]);
        [from[0] + (to[0] - from[0]) / l * t, from[1] + (to[1] - from[1]) / l * t]
    };
    (0..n)
        .flat_map(|i| {
            let (p, c, q) = (v[(i + n - 1) % n], v[i], v[(i + 1) % n]);
            let (a, b) = ([p[0] - c[0], p[1] - c[1]], [q[0] - c[0], q[1] - c[1]]);
            let (la, lb) = (a[0].hypot(a[1]), b[0].hypot(b[1]));
            let half = ((a[0] * b[0] + a[1] * b[1]) / (la * lb)).acos() / 2.0;
            let t = (r / half.tan()).min(la.min(lb) / 2.0);
            [along(c, p, t), along(c, q, t)]
        })
        .collect()
}

/// `a` and `b` within a thousandth of a pixel.
fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

/// Checks `knots` round every vertex of the sharp polygon `sharp` with radius `r`: two anchors a
/// corner, on its edges, each with its handle along that edge towards the vertex (the arc is
/// tangent to the edges), all inside the 400,300 to 600,400 drag box.
fn assert_rounds(knots: &[Knot], sharp: &[Knot], r: f64) {
    let v: Vec<[f64; 2]> = sharp.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    let want = tangent_points(&v, r);
    assert_eq!(knots.len(), 2 * v.len(), "two anchors a corner");
    for (i, (k, w)) in knots.iter().zip(&want).enumerate() {
        let a = [k.anchor.x, k.anchor.y];
        assert!(near(a, *w), "anchor {i}: {a:?}, want {w:?}");
        let corner = v[i / 2];
        // The handle that bends towards the corner: the out handle at the arc's start, the in
        // handle at its end. It lies on the edge, between the anchor and the vertex.
        let h = if i % 2 == 0 { k.out_ctrl } else { k.in_ctrl };
        let cross = (corner[0] - a[0]) * (h.y - a[1]) - (corner[1] - a[1]) * (h.x - a[0]);
        let dot = (corner[0] - a[0]) * (h.x - a[0]) + (corner[1] - a[1]) * (h.y - a[1]);
        assert!(cross.abs() < 1e-6 && dot > 0.0, "knot {i}: the arc is tangent to the edge");
        for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
            assert!((400.0..=600.0).contains(&p.x) && (300.0..=400.0).contains(&p.y), "knot {i} is inside the drag box: {p:?}");
        }
    }
}

#[test]
fn the_radius_field_follows_sides_and_starts_at_zero_in_shape_and_path_mode() {
    for mode in [VectorMode::Shape, VectorMode::Path] {
        let mut app = new_app();
        app.ui.tool_options.shape_mode = mode;
        let h = bar_with(app);
        let sides = field_rect(&h, "Sides").expect("the Sides field");
        let radius = field_rect(&h, "Radius").unwrap_or_else(|| panic!("{mode:?}: the Radius field"));
        assert!(radius.left() > sides.right() && (radius.center().y - sides.center().y).abs() < 1.0, "{mode:?}: Radius right after Sides");
        assert_eq!(radii(&h), (0.0, 0.0), "{mode:?}: 0 px to start");
    }
    let h = bar_with({
        let mut app = new_app();
        app.ui.tool = Tool::Star;
        app
    });
    assert!(field_rect(&h, "Radius").is_none(), "the Star tool has no polygon radius");
}

#[test]
fn a_radius_in_range_is_taken_without_an_alert_and_leaves_the_rectangles_alone() {
    for (typed, want) in [("20", 20.0), ("0.5", 0.5), ("150000", 150_000.0), ("0", 0.0)] {
        let mut h = bar_with(new_app());
        enter_radius(&mut h, typed);
        assert_eq!(radii(&h), (want, 0.0), "{typed}");
        assert_eq!(alerts(&h), 0, "{typed} is in range");
    }
}

#[test]
fn a_radius_past_the_range_takes_the_closest_value_with_one_alert() {
    for (typed, want) in [("-5", 0.0), ("99999999", 150_000.0)] {
        let mut h = bar_with(new_app());
        enter_radius(&mut h, "12");
        enter_radius(&mut h, typed);
        assert_eq!(radii(&h), (want, 0.0), "{typed}");
        assert_eq!(alerts(&h), 1, "{typed}: one alert");
    }
}

#[test]
fn text_that_is_no_number_keeps_the_previous_radius() {
    let mut h = bar_with(new_app());
    enter_radius(&mut h, "12");
    enter_radius(&mut h, "abc");
    assert_eq!(radii(&h), (12.0, 0.0), "the last valid value stays");
}

#[test]
fn the_radius_field_leaves_the_active_polygon_and_the_history_alone() {
    let mut app = new_app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    let before = (active_shape(&app), app.session.active().unwrap().history.past_len());
    let mut h = bar_with(app);
    enter_radius(&mut h, "30");
    assert_eq!(radii(&h), (30.0, 0.0));
    let after = (active_shape(h.state()), h.state().session.active().unwrap().history.past_len());
    assert_eq!(after, before, "the active polygon keeps its corners and no History state is added");
}

#[test]
fn radius_20_rounds_every_corner_of_the_next_polygon_with_two_tangent_anchors() {
    let (_, sharp) = drawn_with(0.0);
    let (live, knots) = drawn_with(20.0);
    assert_eq!(live, LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio: 1.0, corner_radius: 20.0 });
    assert_rounds(&knots, &sharp, 20.0);
    // 20 px fits every corner, so no arc is capped: a larger radius would move every anchor.
    let v: Vec<[f64; 2]> = sharp.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert!(tangent_points(&v, 20.0).iter().zip(tangent_points(&v, 21.0)).all(|(a, b)| !near(*a, b)));
}

#[test]
fn radius_1000_is_capped_by_the_shape_without_an_error() {
    let (_, sharp) = drawn_with(0.0);
    let (live, knots) = drawn_with(1000.0);
    assert_eq!(live, LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio: 1.0, corner_radius: 1000.0 });
    assert_rounds(&knots, &sharp, 1000.0);
}

#[test]
fn radius_0_draws_the_sharp_polygon() {
    let (live, knots) = drawn_with(0.0);
    assert_eq!(live, LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio: 1.0, corner_radius: 0.0 });
    assert_eq!(knots.len(), 5, "one anchor a corner");
    assert!(knots.iter().all(|k| k.in_ctrl == k.anchor && k.out_ctrl == k.anchor), "no curves");
}

#[test]
fn the_rectangle_radius_does_not_round_the_polygon() {
    let mut app = new_app();
    app.ui.tool_options.corner_radius = 20.0;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    let (live, _) = active_shape(&app);
    assert_eq!(live, LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio: 1.0, corner_radius: 0.0 });
}

#[test]
fn path_mode_rounds_the_next_outline_too() {
    let mut app = new_app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    app.ui.tool_options.polygon_corner_radius = 20.0;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
    assert_eq!(work.subpaths.len(), 1);
    let (_, sharp) = drawn_with(0.0);
    assert_rounds(&work.subpaths[0].knots, &sharp, 20.0);
}

#[test]
fn the_radius_is_saved_with_the_tool_options_and_reset_tool_sets_it_back_to_zero() {
    let o = crate::state::ToolOptions { polygon_corner_radius: 33.0, ..Default::default() };
    let back: crate::state::ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert_eq!(back.polygon_corner_radius, 33.0, "saved and read back");
    assert!(crate::tools::tool_reset::option_keys(Tool::Polygon).contains(&"polygon_corner_radius"), "a Polygon tool preset stores it");
    let mut reset = o.clone();
    crate::tools::tool_reset::reset_options(&mut reset, Tool::Polygon);
    assert_eq!(reset.polygon_corner_radius, 0.0, "Reset Tool");
    // Remembered values the field could never hold come back in its range.
    for (saved, want) in [(-3.0, 0.0), (1.0e9, 150_000.0), (f32::NAN, 0.0), (40.0, 40.0)] {
        let mut o = crate::state::ToolOptions { polygon_corner_radius: saved, ..Default::default() };
        crate::tool_memory::sanitize(&mut o);
        assert_eq!(o.polygon_corner_radius, want, "{saved}");
    }
}
