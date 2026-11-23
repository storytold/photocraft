//! #13320: with the Polygon tool and a shape layer active, ⇧, ⌥ or ⇧⌥ held at the press add the
//! drawn polygon to that layer as a Combine, Subtract or Intersect component, in one History step.
//! The same keys still constrain (⇧) and draw from the centre (⌥). Pressed only after the drag
//! started, they only do that, and the drag makes a new layer as before.

use craft::draw::ui::VectorMode;
use egui::Modifiers;
use photocraft_doc::vector::{PathOp, Subpath};
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The issue's test document: 800 × 600, the Polygon tool in Shape mode with 5 sides, and the
/// shape layer Polygon 1 (a polygon drawn from 400,300 to 600,400) active.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    // View › Snap off: each box is the drag's own, not pulled onto Polygon 1's edges or centre.
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Polygon;
    app.ui.tool_options.polygon_sides = 5;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(active(&app).0, "Polygon 1");
    app
}

/// Press at `from` with `press` held, move to `to` (halfway first) and release there with `rest` held.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, rest: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, rest);
}

/// The active layer's name and shape; it must be a shape layer.
fn active(app: &PhotocraftApp) -> (String, ShapeLayer) {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("{} is not a shape layer", layer.name) };
    (layer.name.clone(), shape.clone())
}

/// The number of layers and of History states.
fn counts(app: &PhotocraftApp) -> (usize, usize) {
    let st = app.session.active().unwrap();
    (st.doc.layer_count(), st.history.past_len())
}

/// A component's anchors' box `[x0, y0, x1, y1]`.
fn bounds(sp: &Subpath) -> [f64; 4] {
    sp.knots.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, k| {
        [b[0].min(k.anchor.x), b[1].min(k.anchor.y), b[2].max(k.anchor.x), b[3].max(k.anchor.y)]
    })
}

/// `got` is `want` within 0.01 px on every side.
fn assert_box(got: [f64; 4], want: [f64; 4], what: &str) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01), "{what}: {got:?}, expected {want:?}");
}

/// The box of a regular pentagon with its first vertex at the top, centred on `c` with
/// circumradius `r` (sin 72° wide, 1 + cos 36° high).
fn pentagon(c: [f64; 2], r: f64) -> [f64; 4] {
    let half_width = r * 72f64.to_radians().sin();
    [c[0] - half_width, c[1] - r, c[0] + half_width, c[1] + r * 36f64.to_radians().cos()]
}

/// The coverage (0 to 1) of pixel (x, y) in the active shape layer's rendering.
fn alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let cache = active(app).1.cache.expect("the shape layer is rendered");
    cache.pixel(x, y).last().copied().unwrap()
}

/// Inside Polygon 1 and inside the ⌥ (Subtract) component; inside Polygon 1 only; and inside
/// Polygon 1 and the ⇧⌥ (Intersect) component.
const IN_BOTH: (i32, i32) = (480, 325);
const ONLY_POLYGON_1: (i32, i32) = (500, 370);
const IN_INTERSECT: (i32, i32) = (490, 308);

#[test]
fn shift_at_the_press_combines_a_regular_pentagon_into_polygon_1() {
    let mut app = app();
    let first = active(&app).1.path.subpaths[0].clone();
    assert_eq!(alpha(&app, 500, 500), 0.0, "nothing there yet");
    let (layers, states) = counts(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(counts(&app), (layers, states + 1), "no new layer, one History step");
    let (name, shape) = active(&app);
    assert_eq!(name, "Polygon 1");
    assert_eq!(shape.path.subpaths.len(), 2, "a second component");
    assert_eq!(shape.path.subpaths[0], first, "the first component stays");
    let added = &shape.path.subpaths[1];
    assert_eq!(added.op, PathOp::Combine);
    assert_eq!(added.knots.len(), 5);
    // The reference's 452.45,450 to 547.55,540.45.
    assert_box(bounds(added), pentagon([500.0, 500.0], 50.0), "a regular pentagon from the press corner");
    assert_box(bounds(added), [452.45, 450.0, 547.55, 540.45], "the measured box");
    assert_eq!(alpha(&app, 500, 500), 1.0, "the layer fills the combined pentagon");
    assert_eq!(alpha(&app, ONLY_POLYGON_1.0, ONLY_POLYGON_1.1), 1.0);
}

#[test]
fn alt_at_the_press_subtracts_a_pentagon_drawn_from_the_centre() {
    let mut app = app();
    assert_eq!(alpha(&app, IN_BOTH.0, IN_BOTH.1), 1.0, "Polygon 1 covers it before");
    let (layers, states) = counts(&app);
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(counts(&app), (layers, states + 1), "no new layer, one History step");
    let (name, shape) = active(&app);
    assert_eq!(name, "Polygon 1");
    assert_eq!(shape.path.subpaths.len(), 2);
    let added = &shape.path.subpaths[1];
    assert_eq!(added.op, PathOp::Subtract);
    // The press is the centre: the box 350,150 to 550,350.
    assert_box(bounds(added), pentagon([450.0, 250.0], 100.0), "a pentagon centred on the press");
    assert_eq!(alpha(&app, IN_BOTH.0, IN_BOTH.1), 0.0, "cut out of Polygon 1");
    assert_eq!(alpha(&app, ONLY_POLYGON_1.0, ONLY_POLYGON_1.1), 1.0, "the rest of Polygon 1 stays");
    assert_eq!(app.session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "no eyedropper: the foreground stays black");
}

#[test]
fn shift_alt_at_the_press_intersects_a_regular_pentagon_from_the_centre() {
    let mut app = app();
    let (layers, states) = counts(&app);
    let both = Modifiers::SHIFT | Modifiers::ALT;
    // An 100 × 80 drag: ⇧ squares it from the shorter side, so the circumradius is 80.
    drag(&mut app, [450.0, 250.0], [550.0, 330.0], both, both);
    assert_eq!(counts(&app), (layers, states + 1), "no new layer, one History step");
    let (name, shape) = active(&app);
    assert_eq!(name, "Polygon 1");
    assert_eq!(shape.path.subpaths.len(), 2);
    let added = &shape.path.subpaths[1];
    assert_eq!(added.op, PathOp::Intersect);
    assert_box(bounds(added), pentagon([450.0, 250.0], 80.0), "regular, centred on the press");
    assert_eq!(alpha(&app, IN_INTERSECT.0, IN_INTERSECT.1), 1.0, "the overlap stays");
    assert_eq!(alpha(&app, ONLY_POLYGON_1.0, ONLY_POLYGON_1.1), 0.0, "Polygon 1 outside the pentagon goes");
}

#[test]
fn modifiers_pressed_after_the_start_only_constrain_and_make_polygon_2() {
    for (rest, centre, r) in [(Modifiers::SHIFT, [500.0, 500.0], 50.0), (Modifiers::ALT, [450.0, 450.0], 100.0)] {
        let mut app = app();
        let (layers, states) = counts(&app);
        // 100 × 80: ⇧ makes it regular from the shorter side; ⌥ draws a 200 × 160 box from the centre.
        let to = if rest.shift { [550.0, 550.0] } else { [550.0, 530.0] };
        drag(&mut app, [450.0, 450.0], to, Modifiers::NONE, rest);
        assert_eq!(counts(&app), (layers + 1, states + 1), "{rest:?}: a new layer");
        let (name, shape) = active(&app);
        assert_eq!(name, "Polygon 2", "{rest:?}");
        assert_eq!(shape.path.subpaths.len(), 1, "{rest:?}: one component");
        let want = if rest.shift {
            pentagon(centre, r)
        } else {
            // Inscribed in the 200 × 160 box's ellipse.
            let p = pentagon([0.0, 0.0], 1.0);
            [centre[0] + 100.0 * p[0], centre[1] + 80.0 * p[1], centre[0] + 100.0 * p[2], centre[1] + 80.0 * p[3]]
        };
        assert_box(bounds(&shape.path.subpaths[0]), want, &format!("{rest:?}"));
    }
}

/// The press picks the operation only for the Polygon tool, in Shape mode, with one shape layer
/// selected: not measured otherwise, so a new layer as before.
#[test]
fn unmeasured_cases_keep_making_a_new_layer() {
    // A pixel layer active.
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    let (layers, _) = counts(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(counts(&app).0, layers + 1, "a pixel layer: a new shape layer");
    assert_eq!(active(&app).1.path.subpaths.len(), 1);

    // Another shape tool.
    let mut app = self::app();
    app.ui.tool = Tool::EllipseShape;
    let (layers, _) = counts(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(counts(&app).0, layers + 1, "the Ellipse tool: a new shape layer");

    // Two shape layers selected.
    let mut app = self::app();
    drag(&mut app, [100.0, 100.0], [200.0, 200.0], Modifiers::NONE, Modifiers::NONE);
    let st = app.session.active().unwrap();
    let shapes: Vec<u64> = st
        .doc
        .walk()
        .into_iter()
        .map(|(_, _, l)| l.id)
        .filter(|&id| matches!(st.doc.layer(id).map(|l| &l.content), Some(LayerContent::Shape(_))))
        .map(|id| id.0)
        .collect();
    for (i, layer) in shapes.iter().enumerate() {
        app.run("layer.select", json!({"layer": layer, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let (layers, _) = counts(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(counts(&app).0, layers + 1, "two shape layers selected: a new shape layer");

    // Path mode: the outline joins the Work Path as a new one would, with the options bar's
    // Exclude, not the press's Combine.
    let mut app = self::app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    drag(&mut app, [100.0, 100.0], [200.0, 200.0], Modifiers::NONE, Modifiers::NONE);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
    assert_eq!(work.subpaths.len(), 2);
    assert_eq!(work.subpaths[1].op, PathOp::Exclude, "Path mode keeps the options bar's operation");
}

/// The press's operation replaces the options bar's for that one drag; the next plain drag uses
/// the options bar's again.
#[test]
fn the_press_wins_over_the_options_bar_for_one_drag() {
    let mut app = app();
    app.ui.tool_options.shape_path_op = "exclude".into();
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::NONE, Modifiers::NONE);
    let ops: Vec<PathOp> = active(&app).1.path.subpaths.iter().map(|s| s.op).collect();
    assert_eq!(ops, [PathOp::Combine, PathOp::Subtract, PathOp::Exclude]);
}

/// Undo takes the component back off Polygon 1 in one step; Redo puts it back.
#[test]
fn undo_and_redo_the_added_component() {
    let mut app = app();
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(active(&app).1.path.subpaths.len(), 2);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(active(&app).1.path.subpaths.len(), 1);
    assert_eq!(alpha(&app, IN_BOTH.0, IN_BOTH.1), 1.0, "the cut is gone");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(active(&app).1.path.subpaths.len(), 2);
    assert_eq!(alpha(&app, IN_BOTH.0, IN_BOTH.1), 0.0);
}

/// The press's addition is one History step named Polygon Tool (#13320); a drag joined with the
/// options bar's operation keeps Add Shape.
#[test]
fn the_press_addition_is_named_polygon_tool() {
    let mut app = app();
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Polygon Tool"));
    app.ui.tool_options.shape_path_op = "exclude".into();
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Shape"));
}
