//! #13366: the modifiers held when the Triangle tool's button goes down add the drag to the active
//! shape layer — Shift Combine Shapes, Alt Subtract Front Shape, both Intersect Shape Areas — each as
//! one History step. A plain press, and a modifier that only goes down after the press, still make a
//! new layer. The cases the issue leaves unmeasured (Path and Pixels mode, a press that also holds
//! Ctrl, no shape layer or a pixel layer active) keep the options bar's rule.
//!
//! Every expected position here is computed from the drag's own corners, not read off a measurement:
//! see [`inscribed_equilateral`] and the comments in the three press tests.

use craft::draw::ui::{ShapeKind, VectorMode};
use craft::keymap::Mods;
use egui::Modifiers;
use photocraft_doc::LayerContent;
use photocraft_doc::vector::{Path, PathOp};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 800})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Triangle;
    app
}

/// A Triangle drag from `from` to `to`: `press_mods` held when the button goes down, `drag_mods` held
/// for the moves and the release.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press_mods: Modifiers, drag_mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press_mods);
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: middle[0], y: middle[1], pressure: 1.0 }, drag_mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, drag_mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, drag_mods);
}

/// The reference drag: the plain triangle that becomes `Triangle 1`, in the box (400, 300) to
/// (600, 400) with its apex on the box's top edge at (500, 300).
fn base_triangle(app: &mut PhotocraftApp) {
    drag(app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
}

fn shape_layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.iter().filter(|l| matches!(l.content, LayerContent::Shape(_))).count()
}

/// The path of every shape layer, in the order the document lists them.
fn shape_paths(app: &PhotocraftApp) -> Vec<Path> {
    app.session
        .active()
        .unwrap()
        .doc
        .layers
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Shape(sh) => Some(sh.path.clone()),
            _ => None,
        })
        .collect()
}

/// The path of the shape layer named `name`.
fn shape_named(app: &PhotocraftApp, name: &str) -> Path {
    let st = app.session.active().unwrap();
    st.doc
        .layers
        .iter()
        .find(|l| l.name == name)
        .and_then(|l| match &l.content {
            LayerContent::Shape(sh) => Some(sh.path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no shape layer named {name}"))
}

/// The anchors of a subpath.
fn anchors(s: &photocraft_doc::vector::Subpath) -> Vec<(f64, f64)> {
    s.knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()
}

/// The box around `a`, as (x0, y0, x1, y1).
fn box_of(a: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let x = |p: &(f64, f64)| p.0;
    let y = |p: &(f64, f64)| p.1;
    (
        a.iter().map(x).fold(f64::INFINITY, f64::min),
        a.iter().map(y).fold(f64::INFINITY, f64::min),
        a.iter().map(x).fold(f64::NEG_INFINITY, f64::max),
        a.iter().map(y).fold(f64::NEG_INFINITY, f64::max),
    )
}

/// `got` within `within` of `want` on every edge. Align Edges rounds the committed shape to document
/// pixels, so a drag lands up to a pixel off the box its corners ask for.
fn near(got: (f64, f64, f64, f64), want: (f64, f64, f64, f64), within: f64) -> bool {
    let off = |a: f64, b: f64| (a - b).abs() <= within;
    off(got.0, want.0) && off(got.1, want.1) && off(got.2, want.2) && off(got.3, want.3)
}

/// The three corners of the equilateral triangle the Triangle tool draws in a ⇧ box `b` (#13351's
/// rule, worked out from the box here): the square of the box's shorter side, and inside it the
/// triangle inscribed in that square's inscribed circle, apex up. So with `r` the shorter side's
/// half, the apex is the top edge's midpoint, and the base lies `r / 2` below the centre, `√3 · r / 2`
/// out to either side of it.
fn inscribed_equilateral(b: (f64, f64, f64, f64)) -> Vec<(f64, f64)> {
    let (x0, y0, x1, y1) = b;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let r = (x1 - x0).min(y1 - y0) / 2.0;
    vec![(cx, cy - r), (cx - r * 3f64.sqrt() / 2.0, cy + r / 2.0), (cx + r * 3f64.sqrt() / 2.0, cy + r / 2.0)]
}

/// The three side lengths of a triangle, shortest first.
fn sides(a: &[(f64, f64)]) -> Vec<f64> {
    let len = |p: (f64, f64), q: (f64, f64)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
    let mut v = vec![len(a[0], a[1]), len(a[1], a[2]), len(a[2], a[0])];
    v.sort_by(|x, y| x.total_cmp(y));
    v
}

/// How much of the pixel (x, y) the path's fill covers: 1.0 draws, 0.0 leaves it alone.
fn covered(path: &Path, x: i32, y: i32) -> f32 {
    photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

/// The History length before the next command.
fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The drag left exactly one History step, under `label`. A new shape layer takes the tool's own
/// name; an addition to an existing layer takes the engine's "Add Shape", which `shape.create`
/// decides for every tool but the Line (#12890 pins it), so this issue's "one History step" is what
/// the tests pin here.
fn one_step(app: &PhotocraftApp, before: usize, label: &str) {
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one History step");
    assert_eq!(st.history.undo_label(), Some(label), "named for the step");
}

#[test]
fn shift_at_the_press_adds_a_combine_component_to_the_active_triangle() {
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers, "no new layer: Shift at the press adds to the active one");
    let path = shape_named(&app, "Triangle 1");
    assert_eq!(path.subpaths.len(), 2, "Triangle 1 holds a second component");
    assert_eq!(path.subpaths[1].op, PathOp::Combine, "Combine Shapes");
    let a = anchors(&path.subpaths[1]);
    assert_eq!(a.len(), 3, "the component is a triangle: {a:?}");
    // The drag box is already square, so the ⇧ square is the drag box and the component is the
    // equilateral triangle inscribed in it, as `inscribed_equilateral` works out from its corners.
    assert!(near(box_of(&a), box_of(&inscribed_equilateral((450.0, 450.0, 550.0, 550.0))), 2.0), "the inscribed equilateral, got {:?}", box_of(&a));
    let s = sides(&a);
    assert!((s[2] - s[0]).abs() < 1.0, "and equilateral: {s:?}");
    one_step(&app, before, "Add Shape");
    assert_eq!(covered(&path, 500, 500), 1.0, "the layer renders the added component");
    assert_eq!(covered(&path, 500, 350), 1.0, "and keeps the first component");
}

#[test]
fn alt_at_the_press_adds_a_subtract_component_drawn_from_the_centre() {
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(shape_layers(&app), layers, "no new layer");
    let path = shape_named(&app, "Triangle 1");
    assert_eq!(path.subpaths.len(), 2, "Triangle 1 holds a second component");
    assert_eq!(path.subpaths[1].op, PathOp::Subtract, "Subtract Front Shape");
    let a = anchors(&path.subpaths[1]);
    // ⌥ makes the press the box's centre, so the box reaches as far below and left of the press as
    // the pointer lies above and right of it: (450, 250) ± (100, 100).
    assert!(near(box_of(&a), (350.0, 150.0, 550.0, 350.0), 2.0), "drawn from the press as the centre, got {:?}", box_of(&a));
    one_step(&app, before, "Add Shape");
    assert_eq!(covered(&path, 500, 340), 0.0, "the subtracted area leaves no pixels");
    assert_eq!(covered(&path, 560, 390), 1.0, "the first component outside it still draws");
}

#[test]
fn shift_and_alt_at_the_press_add_an_intersect_component_equilateral_and_centred() {
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::SHIFT | Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(shape_layers(&app), layers, "no new layer");
    let path = shape_named(&app, "Triangle 1");
    assert_eq!(path.subpaths.len(), 2, "Triangle 1 holds a second component");
    assert_eq!(path.subpaths[1].op, PathOp::Intersect, "Intersect Shape Areas");
    let a = anchors(&path.subpaths[1]);
    // ⌥ centres the square on the press and ⇧ inscribes the triangle in it, both from the drag's
    // corners: the square (450, 250) ± 100, and `inscribed_equilateral` of that box.
    assert!(near(box_of(&a), box_of(&inscribed_equilateral((350.0, 150.0, 550.0, 350.0))), 2.0), "the centred inscribed equilateral, got {:?}", box_of(&a));
    let s = sides(&a);
    assert!((s[2] - s[0]).abs() < 1.0, "and equilateral: {s:?}");
    one_step(&app, before, "Add Shape");
}

#[test]
fn a_modifier_held_only_after_the_press_still_makes_a_new_layer() {
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    // Shift goes down with the first move, not at the press: it still constrains the drag.
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::NONE, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers + 1, "a new shape layer, as today");
    let path = shape_named(&app, "Triangle 2");
    assert_eq!(path.subpaths.len(), 1, "with one component of its own");
    let a = anchors(&path.subpaths[0]);
    let s = sides(&a);
    assert!((s[2] - s[0]).abs() < 1.0, "and still equilateral, so Shift reached the shape: {s:?}");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn a_plain_press_makes_a_new_layer() {
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(shape_layers(&app), layers + 1, "the no-modifier path is unchanged");
    assert_eq!(shape_named(&app, "Triangle 2").subpaths.len(), 1, "its own layer, one component");
    assert_eq!(shape_named(&app, "Triangle 1").subpaths.len(), 1, "and the first layer untouched");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn the_options_bar_still_decides_the_operation_of_a_plain_press() {
    let mut app = app();
    base_triangle(&mut app);
    app.ui.tool_options.shape_path_op = "subtract".into();
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 250.0], [550.0, 350.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(shape_layers(&app), layers, "the options bar's Subtract still joins the active layer");
    let path = shape_named(&app, "Triangle 1");
    assert_eq!(path.subpaths.len(), 2);
    assert_eq!(path.subpaths[1].op, PathOp::Subtract);
    one_step(&app, before, "Add Shape");
}

#[test]
fn path_mode_keeps_the_options_bar_rule_for_a_press_holding_shift() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    app.ui.tool_options.shape_path_op = "subtract".into();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers, "Path mode draws no shape layer of its own");
    let path = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
    assert_eq!(path.subpaths.len(), 2, "the outline joined the work path");
    assert_eq!(path.subpaths[1].op, PathOp::Subtract, "not measured, so the options bar's operation decides");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn pixels_mode_changes_nothing_for_a_press_holding_shift() {
    // Pixels mode is not measured, so it keeps today's behaviour: the same layers and the same
    // components as the same drag without a press modifier.
    let pixels = |mods: Modifiers| {
        let mut app = app();
        base_triangle(&mut app);
        app.ui.tool_options.shape_mode = VectorMode::Pixels;
        drag(&mut app, [450.0, 450.0], [550.0, 550.0], mods, mods);
        (shape_layers(&app), shape_named(&app, "Triangle 1").subpaths.len())
    };
    let plain = pixels(Modifiers::NONE);
    let at_press = pixels(Modifiers::SHIFT);
    assert_eq!(at_press, plain, "Pixels mode answers a Shift press as it answers a plain one");
}

#[test]
fn a_press_also_holding_ctrl_or_cmd_keeps_the_options_bar_rule() {
    // The issue measures Shift, Alt and Shift+Alt only, so a press that also holds Ctrl or Cmd keeps
    // today's rule. Pinned where the app decides it - `finish_drag_with_press`, which is what
    // `host::drain` calls with the press's modifiers; the same case as a gesture is pinned by
    // `a_press_holding_the_command_modifier_keeps_the_options_bar_rule`.
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    let geometry = craft::draw::geometry::shape::ShapeGeometry::Triangle { rect: craft::behavior::kurbo::Rect::new(450.0, 450.0, 550.0, 550.0) };
    for press_mods in [Mods { shift: true, ctrl: true, ..Mods::NONE }, Mods { shift: true, cmd: true, ..Mods::NONE }] {
        super::shapes::finish_drag_with_press(&mut app, ShapeKind::Triangle, &geometry, Mods::default(), press_mods);
    }
    assert_eq!(shape_layers(&app), layers + 2, "each press made its own layer, as today");
    assert_eq!(shape_named(&app, "Triangle 1").subpaths.len(), 1, "and added nothing to the active layer");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), before + 2, "one History step each");
    assert_eq!(st.history.undo_label(), Some("Triangle Tool"), "under the tool's own name");
}

#[test]
fn the_press_modifiers_the_host_hands_over_decide_the_operation() {
    // The same call with the measured presses: Shift adds Combine, Alt adds Subtract, both add Intersect.
    for (press_mods, want) in [
        (Mods { shift: true, ..Mods::NONE }, PathOp::Combine),
        (Mods { alt: true, ..Mods::NONE }, PathOp::Subtract),
        (Mods { shift: true, alt: true, ..Mods::NONE }, PathOp::Intersect),
    ] {
        let mut app = app();
        base_triangle(&mut app);
        let (before, layers) = (history_len(&app), shape_layers(&app));
        let geometry = craft::draw::geometry::shape::ShapeGeometry::Triangle { rect: craft::behavior::kurbo::Rect::new(450.0, 450.0, 550.0, 550.0) };
        super::shapes::finish_drag_with_press(&mut app, ShapeKind::Triangle, &geometry, Mods::default(), press_mods);
        assert_eq!(shape_layers(&app), layers, "{want:?}: no new layer");
        let path = shape_named(&app, "Triangle 1");
        assert_eq!(path.subpaths.len(), 2, "{want:?}: a second component");
        assert_eq!(path.subpaths[1].op, want, "{want:?}");
        one_step(&app, before, "Add Shape");
    }
}

#[test]
fn a_press_holding_shift_over_no_shape_layer_makes_a_new_layer() {
    let mut app = app();
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers + 1, "with no shape layer active a new layer is made");
    assert_eq!(shape_named(&app, "Triangle 1").subpaths.len(), 1, "with its own single component");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn a_press_holding_shift_over_a_pixel_layer_makes_a_new_layer() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap();
    app.sync_views();
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers + 1, "a pixel layer is not a shape layer: a new layer is made");
    assert_eq!(shape_named(&app, "Triangle 1").subpaths.len(), 1, "with its own single component");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn a_press_holding_alt_over_no_shape_layer_is_the_colour_sample_and_draws_no_layer() {
    // Not measured by this issue and unchanged by it: with no shape layer active an Alt press is the
    // colour sample (#13364), which the tool host takes before the Shape tool sees the press, so no
    // shape layer is made at all - while the same drag with Shift, or with nothing held, makes one.
    let mut app = app();
    let layers = shape_layers(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(shape_layers(&app), layers, "no shape layer, as today");
}

#[test]
fn a_press_holding_alt_over_a_pixel_layer_is_the_colour_sample_and_draws_no_layer() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap();
    app.sync_views();
    let layers = shape_layers(&app);
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(shape_layers(&app), layers, "a pixel layer is not a shape layer, so Alt samples instead");
}

#[test]
fn shift_at_the_press_with_the_modifier_released_before_the_release_still_combines() {
    // The press asks for the operation; the shape follows the modifiers held during the drag. Shift
    // comes down at the press and goes up before the first move, so the component combines into
    // Triangle 1 and the triangle fills the dragged box instead of the inscribed equilateral.
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::NONE);
    assert_eq!(shape_layers(&app), layers, "no new layer");
    let path = shape_named(&app, "Triangle 1");
    assert_eq!(path.subpaths.len(), 2, "the drag joined Triangle 1");
    assert_eq!(path.subpaths[1].op, PathOp::Combine, "Shift at the press asked for Combine Shapes");
    let a = anchors(&path.subpaths[1]);
    assert!(near(box_of(&a), (450.0, 450.0, 550.0, 550.0), 2.0), "the shape takes the dragged box, got {:?}", box_of(&a));
    let s = sides(&a);
    assert!((s[2] - s[0]).abs() > 5.0, "and is not equilateral: {s:?}");
    one_step(&app, before, "Add Shape");
}

#[test]
fn the_press_of_the_last_gesture_does_not_carry_into_the_next_one() {
    // A Shift press that found no shape layer made its own layer; the next, plain press must not read
    // that old press: it starts a layer of its own too.
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    assert_eq!(
        (layers, shape_paths(&app).iter().map(|p| p.subpaths.len()).collect::<Vec<_>>()),
        (1, vec![1]),
        "the first press made one layer with one component"
    );
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(shape_layers(&app), layers + 1, "the plain press made its own layer");
    assert_eq!(shape_paths(&app).iter().map(|p| p.subpaths.len()).collect::<Vec<_>>(), vec![1, 1], "and neither holds a second component");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn a_press_holding_the_command_modifier_keeps_the_options_bar_rule() {
    // Off macOS a held Ctrl reaches the framework as the command modifier (the injected `ctrl` flag is
    // the macOS Control key and arrives as nothing), so this is how a real Ctrl press on Windows or
    // Linux asks for the drag: the press reports Cmd, the rule fails closed, and the drag starts a
    // layer of its own as it does today.
    let mut app = app();
    base_triangle(&mut app);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    let mods = Modifiers { shift: true, command: true, ..Modifiers::NONE };
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], mods, mods);
    assert_eq!(shape_layers(&app), layers + 1, "a new layer, as today");
    assert_eq!(shape_named(&app, "Triangle 1").subpaths.len(), 1, "and nothing added to the active layer");
    one_step(&app, before, "Triangle Tool");
}

#[test]
fn a_press_holding_shift_with_the_rectangle_tool_still_makes_a_new_layer() {
    // The Rectangle row of the epic (#13335) is open, so the Rectangle tool keeps today's rule:
    // Shift at the press constrains the box and starts a layer of its own.
    let mut app = app();
    app.ui.tool = Tool::Rectangle;
    drag(&mut app, [300.0, 200.0], [500.0, 300.0], Modifiers::NONE, Modifiers::NONE);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers + 1, "a new shape layer, not a component of the active one");
    assert_eq!(shape_paths(&app).iter().map(|p| p.subpaths.len()).collect::<Vec<_>>(), vec![1, 1], "each layer keeps one component");
    one_step(&app, before, "New Shape Layer");
}

#[test]
fn a_press_holding_shift_with_the_ellipse_tool_still_makes_a_new_layer() {
    // Same for the open Ellipse row (#13227): Shift at the press still starts a new layer, and the
    // shape is the constrained circle it always was.
    let mut app = app();
    app.ui.tool = Tool::EllipseShape;
    drag(&mut app, [300.0, 200.0], [500.0, 300.0], Modifiers::NONE, Modifiers::NONE);
    let (before, layers) = (history_len(&app), shape_layers(&app));
    drag(&mut app, [450.0, 450.0], [550.0, 550.0], Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(shape_layers(&app), layers + 1, "a new shape layer, not a component of the active one");
    let paths = shape_paths(&app);
    assert_eq!(paths.iter().map(|p| p.subpaths.len()).collect::<Vec<_>>(), vec![1, 1], "each layer keeps one component");
    // One of the two layers is the second drag: an ellipse filling the 100 x 100 box ⇧ asks for.
    let squared = paths.iter().any(|p| near(box_of(&anchors(&p.subpaths[0])), (450.0, 450.0, 550.0, 550.0), 2.0));
    assert!(squared, "Shift still squares the drag: {:?}", paths.iter().map(|p| box_of(&anchors(&p.subpaths[0]))).collect::<Vec<_>>());
    one_step(&app, before, "Ellipse Tool");
}
