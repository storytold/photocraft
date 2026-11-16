//! #13302: the Polygon tool's options bar has its own corner radius after Sides, in px (0 to
//! start, and after Reset Tool), in Shape and Path mode. It rounds the next polygon drawn, never
//! the active one, and shares the Rectangle Radius's 0..150000 range and alert; text that is not
//! a number keeps the previous value with the reference's invalid-entry alert.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::vector::Knot;
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use super::{POLYGON_RADIUS_INVALID_ALERT, RADIUS_RANGE_ALERT};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

/// An 800 × 600 document with the Polygon tool current.
fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Polygon;
    app
}

/// The options bar drawn for `app`.
fn bar_with(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// The options-bar number field named `label`.
fn field_rect(h: &Harness<'static, PhotocraftApp>, label: &str) -> Option<egui::Rect> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).map(|n| n.rect())
}

/// Clicks into Radius, selects its text and types `text`, `per_key` one character per pass, then
/// Tabs out of it.
fn enter_radius(h: &mut Harness<'static, PhotocraftApp>, text: &str, per_key: bool) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    let keys: Vec<String> = if per_key { text.chars().map(String::from).collect() } else { vec![text.to_owned()] };
    for k in keys {
        h.event(egui::Event::Text(k));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// The messages of the open alerts.
fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on every open alert, as `ui.dialog.confirm` presses it (this harness draws only the bar).
fn ok_all(h: &mut Harness<'static, PhotocraftApp>) {
    let ids: Vec<u64> = h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).map(|d| d.id).collect();
    for id in ids {
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
    }
    h.run_steps(3);
}

/// The Polygon tool's radius, and the Rectangle tool's, which it must leave alone.
fn radii(h: &Harness<'static, PhotocraftApp>) -> (f32, f32) {
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
    for mode in ["shape", "path"] {
        let mut app = new_app();
        app.ui.tool_options.shape_mode = mode.into();
        let h = bar_with(app);
        let sides = field_rect(&h, "Sides:").expect("the Sides field");
        let radius = field_rect(&h, "Radius:").unwrap_or_else(|| panic!("{mode}: the Radius field"));
        assert!(radius.left() > sides.right() && (radius.center().y - sides.center().y).abs() < 1.0, "{mode}: Radius right after Sides");
        assert_eq!(radii(&h), (0.0, 0.0), "{mode}: 0 px to start");
    }
}

#[test]
fn a_radius_in_range_is_taken_without_an_alert_and_leaves_the_rectangles_alone() {
    for (typed, want) in [("20", 20.0), ("0.5", 0.5), ("150000", 150_000.0), ("0", 0.0)] {
        let mut h = bar_with(new_app());
        enter_radius(&mut h, typed, false);
        assert_eq!(radii(&h), (want, 0.0), "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn a_radius_past_the_range_takes_the_closest_value_with_one_alert() {
    for (typed, want) in [("-5", 0.0), ("99999999", 150_000.0)] {
        let mut h = bar_with(new_app());
        enter_radius(&mut h, "12", false);
        enter_radius(&mut h, typed, false);
        assert_eq!(radii(&h), (want, 0.0), "{typed}");
        assert_eq!(alerts(&h), vec![RADIUS_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok_all(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(radii(&h).0, want, "{typed}: the closest value stays");
    }
}

#[test]
fn text_that_is_no_number_keeps_the_previous_radius_with_the_invalid_entry_alert() {
    let mut h = bar_with(new_app());
    enter_radius(&mut h, "12", false);
    enter_radius(&mut h, "abc", false);
    assert_eq!(radii(&h), (12.0, 0.0), "the last valid value stays");
    assert_eq!(alerts(&h), vec![POLYGON_RADIUS_INVALID_ALERT.to_string()], "one invalid-entry alert");
    ok_all(&mut h);
    // Visiting the field again without typing, then a number, raises nothing.
    enter_radius(&mut h, "", false);
    assert!(alerts(&h).is_empty(), "nothing typed: {:?}", alerts(&h));
    enter_radius(&mut h, "7", false);
    assert_eq!(radii(&h), (7.0, 0.0));
    assert!(alerts(&h).is_empty(), "7 is a number in range: {:?}", alerts(&h));
}

#[test]
fn arithmetic_is_a_number_until_text_spoils_it() {
    let mut h = bar_with(new_app());
    // `50/2` is held, unread, until the field is left: only that pass reads `50/2x`.
    enter_radius(&mut h, "50/2x", true);
    assert_eq!(alerts(&h), vec![POLYGON_RADIUS_INVALID_ALERT.to_string()], "50/2x is no number");
    ok_all(&mut h);
    enter_radius(&mut h, "50/2", true);
    assert_eq!(radii(&h), (25.0, 0.0));
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}

#[test]
fn the_radius_field_leaves_the_active_polygon_and_the_history_alone() {
    let mut app = new_app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    app.sync_views();
    let before = (active_shape(&app), app.session.active().unwrap().history.past_len());
    let mut h = bar_with(app);
    enter_radius(&mut h, "30", false);
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
fn path_mode_rounds_the_next_outline_too() {
    let mut app = new_app();
    app.ui.tool_options.shape_mode = "path".into();
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
