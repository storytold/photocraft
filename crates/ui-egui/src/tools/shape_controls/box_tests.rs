//! #11107: the box round several selected shape layers (Path Selection and the shape tools), its handles scaling
//! them, measured against the reference application: two rectangles 100,98–403,301 and 420,318–723,451 scaled by
//! dragging the bottom-right handle by (−100, −60). Every test drives whole canvas passes.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::box_scale;
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// A 1000 × 700 document with the two measured rectangles, both selected, fitted on the screen.
fn app_with_two_shapes(tool: Tool) -> (Box<PhotocraftApp>, u64, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [100, 98, 303, 203], "fill": "#e02020"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [420, 318, 303, 133], "fill": "#2050e0"})).unwrap();
    let b = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": a.0})).unwrap();
    app.run("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    app.ui.tool = tool;
    app.run("view.fitOnScreen", json!({})).unwrap();
    testing::frame(&mut app);
    (app, a.0, b.0)
}

fn bounds(app: &PhotocraftApp, id: u64) -> (f64, f64, f64, f64) {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content else { panic!("a shape layer") };
    sh.path.control_bounds().unwrap()
}

fn near(got: (f64, f64, f64, f64), want: [f64; 4]) {
    let got = [got.0, got.1, got.2, got.3];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.0), "{got:?} vs {want:?}");
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

/// A press at `from`, two moves and a release at `to`, as a mouse does it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    press_and_drag(app, from, to);
    testing::release(app, to, NONE);
}

/// A press at `from` and two moves to `to`, the button still down.
fn press_and_drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    testing::press(app, from, NONE);
    testing::move_to(app, [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0], NONE);
    testing::move_to(app, to, NONE);
}

/// The bottom-right corner handle, pressed 3 px outside the corner: the corner radius widget of a sharp corner sits
/// inside it, so the press reaches the handle only.
const CORNER: [f64; 2] = [726.0, 454.0];
/// The press `CORNER` drags to: the measured (−100, −60) move.
const CORNER_TO: [f64; 2] = [626.0, 394.0];

#[test]
fn an_edge_handle_scales_one_way_and_undo_restores_both_shapes() {
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    // The right edge's middle, dragged 100 px left: x scales about the left edge (100), y not at all.
    drag(&mut app, [723.0, 274.5], [623.0, 274.5]);
    let s = 523.0 / 623.0;
    near(bounds(&app, a), [100.0, 98.0, 100.0 + 303.0 * s, 301.0]);
    near(bounds(&app, b), [100.0 + 320.0 * s, 318.0, 100.0 + 623.0 * s, 451.0]);
    app.run("edit.undo", json!({})).unwrap();
    near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);
    near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
}

#[test]
fn box_scale_gives_the_scale_and_the_fixed_point_and_nothing_for_a_click() {
    let r = [100.0, 98.0, 723.0, 451.0];
    // A click or a drag back to the start moves nothing.
    assert_eq!(box_scale(r, r), None);
    // The right edge 100 left: one axis, about the left edge; the other axis keeps the box's centre.
    let (s, about) = box_scale(r, [100.0, 98.0, 623.0, 451.0]).unwrap();
    assert!((s[0] - 523.0 / 623.0).abs() < 1e-9 && s[1] == 1.0, "{s:?}");
    assert!((about[0] - 100.0).abs() < 1e-6 && (about[1] - 274.5).abs() < 1e-9, "{about:?}");
    // The top-left corner dragged in: both axes about the bottom-right corner.
    let (s, about) = box_scale(r, [200.0, 98.0 + 353.0 * 0.5, 723.0, 451.0]).unwrap();
    assert!((s[0] - 523.0 / 623.0).abs() < 1e-9 && (s[1] - 0.5).abs() < 1e-9, "{s:?}");
    assert!((about[0] - 723.0).abs() < 1e-6 && (about[1] - 451.0).abs() < 1e-6, "{about:?}");
    // A collapsed box reaches the lowest scale `path.transform` takes.
    let (s, _) = box_scale(r, [100.0, 98.0, 100.0, 451.0]).unwrap();
    assert_eq!(s[0], 0.01);
}

#[test]
fn a_corner_radius_widget_wins_over_the_box() {
    // One rectangle 40,40–160,120 with radii 20: its top-left widget sits inside the corner at (60, 60), not on the
    // box's handle at (40, 40). A press on the widget rounds the corners; a press on the handle scales.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 260})).unwrap();
    app.run("shape.create", json!({"kind": "roundedRect", "rect": [40, 40, 120, 80], "radii": 20})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.ui.tool = Tool::Rectangle;
    app.run("view.actualPixels", json!({})).unwrap();
    testing::frame(&mut app);
    let steps = past(&app);
    drag(&mut app, [60.0, 60.0], [70.0, 70.0]);
    assert_eq!(past(&app), steps + 1);
    assert_ne!(last_step(&app).as_deref(), Some("Transform Path"), "the widget rounded, the box did not scale");
    near(bounds(&app, id), [40.0, 40.0, 160.0, 120.0]);
    drag(&mut app, [37.0, 37.0], [17.0, 17.0]);
    assert_eq!(past(&app), steps + 2);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "the handle scaled");
    let (x0, _, x1, _) = bounds(&app, id);
    assert!(x1 - x0 > 130.0, "{} wide", x1 - x0);
}

#[test]
fn hidden_paths_hide_the_box_and_its_handles_take_no_press() {
    // View › Extras off, or View › Show › Target Path off. The Rectangle tool draws where the box would scale.
    for (extras, target_path) in [(false, true), (true, false)] {
        let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
        app.ui.view.extras = extras;
        app.ui.view.show.target_path = target_path;
        testing::frame(&mut app);
        drag(&mut app, CORNER, CORNER_TO);
        assert!(!app.session.active().unwrap().history.entries().iter().any(|e| e == "Transform Path"), "extras {extras}: no scale ran");
        near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);
        near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
    }
}

/// Nothing scaled: both shapes as measured and no history step past `steps`.
fn assert_unscaled(app: &PhotocraftApp, a: u64, b: u64, steps: usize, why: &str) {
    near(bounds(app, a), [100.0, 98.0, 403.0, 301.0]);
    near(bounds(app, b), [420.0, 318.0, 723.0, 451.0]);
    assert_eq!(past(app), steps, "{why}: no history step");
}

#[test]
fn a_tool_change_cancels_a_handle_drag() {
    // Another tool, picked mid-drag, also one that shows the box too: the box's id salt carries the tool.
    for next in [Tool::Brush, Tool::PathSelection] {
        let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
        let steps = past(&app);
        press_and_drag(&mut app, CORNER, CORNER_TO);
        app.ui.tool = next;
        testing::frame(&mut app);
        testing::release(&mut app, CORNER_TO, NONE);
        assert_unscaled(&app, a, b, steps, &format!("{next:?}"));
    }
}

#[test]
fn a_document_switch_or_an_undo_cancels_a_handle_drag() {
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    let steps = past(&app);
    press_and_drag(&mut app, CORNER, CORNER_TO);
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    testing::frame(&mut app);
    let other = past(&app);
    testing::release(&mut app, CORNER_TO, NONE);
    assert_eq!(past(&app), other, "nothing lands in the other document");
    assert!(app.session.set_active(0));
    assert_unscaled(&app, a, b, steps, "document switch");

    // An edit under the drag (Undo here): the release applies nothing to the changed shapes.
    let (mut app, ..) = app_with_two_shapes(Tool::PathSelection);
    press_and_drag(&mut app, CORNER, CORNER_TO);
    app.run("edit.undo", json!({})).unwrap();
    testing::frame(&mut app);
    let steps = past(&app);
    testing::release(&mut app, CORNER_TO, NONE);
    assert_eq!(past(&app), steps, "undo");
}

/// With background jobs on, as in the desktop app, `edit.transform` lands after `run` returns. The scale is still one
/// step named "Transform Path", and the step before it (the second shape's "New Shape Layer") keeps its name.
#[test]
fn with_background_jobs_the_scale_step_is_named_and_the_one_before_is_kept() {
    let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
    app.background_jobs = true;
    let before: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
    drag(&mut app, CORNER, CORNER_TO);
    let ctx = app.ctx().clone();
    for _ in 0..2000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.session.has_jobs(), "the transform job never landed");
    near(bounds(&app, a), [100.0, 98.0, 354.0, 268.0]);
    // The scale reached the second shape too; its exact bounds are the measured-drag test's concern.
    assert!(bounds(&app, b).2 < 700.0, "the second shape scaled with the first: {:?}", bounds(&app, b));
    let after: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
    assert_eq!(after.len(), before.len() + 1, "one history step: {after:?}");
    assert_eq!(after[..before.len()], before[..], "the earlier steps keep their names");
    assert_eq!(after.last().map(String::as_str), Some("Transform Path"));
}

/// #11473 review: the pick under the side band skips a shape's outline test where the point is out of reach of it, and
/// still finds every shape it found before: an outline within the pick tolerance just past the shape's box, and a
/// fill's pixels past the path (an outside stroke).
#[test]
fn the_pick_still_finds_shapes_just_past_their_path_bounds() {
    use crate::vector_ui::pick_shape_at;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    // A stroke-only ellipse 200,200–400,300: picked by its outline alone.
    app.run("shape.create", json!({"kind": "ellipse", "rect": [200, 200, 200, 100], "fill": null, "stroke": {"width": 2}})).unwrap();
    let e = app.session.active().unwrap().active_layer.unwrap();
    // A filled rectangle 500,400–600,500 with a 20 px stroke outside its path.
    app.run("shape.create", json!({"kind": "rect", "rect": [500, 400, 100, 100], "fill": "#20a040", "stroke": {"width": 20, "align": "outside"}})).unwrap();
    let r = app.session.active().unwrap().active_layer.unwrap();
    let doc = &app.session.active().unwrap().doc;
    let pick = |p: [f64; 2], tol: f64| pick_shape_at(doc, p, tol);
    // Within the tolerance of the ellipse's extreme anchors, outside its box.
    assert_eq!(pick([405.5, 250.0], 6.0), Some(e));
    assert_eq!(pick([300.0, 194.5], 6.0), Some(e));
    assert_eq!(pick([406.5, 250.0], 6.0), None);
    assert_eq!(pick([300.0, 193.5], 6.0), None);
    // Within it of a curve, off the anchors: the ellipse's outline at 45°.
    let (cx, cy) = (300.0 + 100.0 * std::f64::consts::FRAC_1_SQRT_2, 250.0 + 50.0 * std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(pick([cx + 4.0, cy], 6.0), Some(e));
    // The stroke's pixels up to 20 px past the rectangle's path, far past the tolerance.
    assert_eq!(pick([612.0, 450.0], 1.0), Some(r));
    assert_eq!(pick([550.0, 381.5], 1.0), Some(r));
    assert_eq!(pick([625.0, 450.0], 1.0), None);
    assert_eq!(pick([550.0, 378.5], 1.0), None);
}
