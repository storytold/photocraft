//! #11107: the box round several selected shape layers (Path Selection and the shape tools), its
//! handles scaling them, measured against the reference application: two rectangles
//! 100,98–403,301 and 420,318–723,451 scaled by dragging the bottom-right handle by (−100, −60).

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::shape_bounds_box::scale_for;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// A 1:1 view of the 1000 × 700 document, turned by `rotation` degrees.
fn view(rotation: f32) -> ViewXform {
    ViewXform {
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0)),
        zoom: 1.0,
        center: [500.0, 350.0],
        flip: false,
        rotation,
        aspect: 1.0,
    }
}

fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<egui::CursorIcon> {
    crate::shape_bounds_box::cursor(app, &view(0.0), p)
}

/// A 1000 × 700 document with the two measured rectangles, both selected.
fn app_with_two_shapes(tool: Tool) -> (PhotocraftApp, u64, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [100, 98, 303, 203], "fill": "#e02020"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [420, 318, 303, 133], "fill": "#2050e0"})).unwrap();
    let b = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": a.0})).unwrap();
    app.run("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    app.ui.tool = tool;
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

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, NONE);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, NONE);
}

#[test]
fn the_measured_drag_gives_the_reference_bounds_with_every_tool() {
    for tool in [Tool::Rectangle, Tool::PathSelection, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line] {
        let (mut app, a, b) = app_with_two_shapes(tool);
        let steps = app.session.active().unwrap().history.past_len();
        drag(&mut app, [723.0, 451.0], [623.0, 391.0]);
        near(bounds(&app, a), [100.0, 98.0, 354.0, 268.0]);
        near(bounds(&app, b), [368.0, 282.0, 621.0, 394.0]);
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1, "{tool:?}: one history step");
        assert_eq!(st.history.entries().last().map(String::as_str), Some("Transform Path"), "{tool:?}");
        assert!(app.ui.transform.is_none() && app.ui.shape_box.is_none(), "{tool:?}: no Free Transform session opens");
        // The shapes stay live.
        for id in [a, b] {
            let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content else { panic!() };
            assert!(sh.live.is_some(), "{tool:?}: still a live shape");
        }
    }
}

#[test]
fn an_edge_handle_scales_one_way_and_undo_restores_both_shapes() {
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    // The right edge's middle, dragged 100 px left: x scales about the left edge (100).
    drag(&mut app, [723.0, 274.5], [623.0, 274.5]);
    let s = 523.0 / 623.0;
    near(bounds(&app, a), [100.0, 98.0, 100.0 + 303.0 * s, 301.0]);
    near(bounds(&app, b), [100.0 + 320.0 * s, 318.0, 100.0 + 623.0 * s, 451.0]);
    app.run("edit.undo", json!({})).unwrap();
    near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);
    near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
}

#[test]
fn a_click_without_movement_changes_nothing() {
    let (mut app, ..) = app_with_two_shapes(Tool::Rectangle);
    let steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, [723.0, 451.0], [723.0, 451.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(app.ui.shape_box.is_none());
}

#[test]
fn one_selected_shape_gets_the_box_and_other_tools_none() {
    // Photoshop shows the box for a single shape too: its corner handle scales that shape.
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    app.run("layer.select", json!({"layer": a})).unwrap();
    assert_eq!(cursor(&app, [403.0, 301.0]), Some(egui::CursorIcon::ResizeNwSe));
    assert!(cursor(&app, [723.0, 451.0]).is_none(), "the other shape's corner is not a handle");
    drag(&mut app, [403.0, 301.0], [303.0, 241.0]);
    let s = 1.0 + (-100.0 * 303.0 + -60.0 * 203.0) / (303.0f64 * 303.0 + 203.0 * 203.0);
    near(bounds(&app, a), [100.0, 98.0, 100.0 + 303.0 * s, 98.0 + 203.0 * s]);
    near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("Transform Path"));

    // Another tool has none.
    let (mut app, ..) = app_with_two_shapes(Tool::Move);
    assert!(cursor(&app, [723.0, 451.0]).is_none());
    app.ui.tool = Tool::Brush;
    assert!(cursor(&app, [723.0, 451.0]).is_none());
}

#[test]
fn the_handles_show_their_scale_cursors() {
    use egui::CursorIcon::{ResizeHorizontal, ResizeNeSw, ResizeNwSe, ResizeVertical};
    let (app, ..) = app_with_two_shapes(Tool::PathSelection);
    let at = |p: [f64; 2]| cursor(&app, p);
    assert_eq!(at([100.0, 98.0]), Some(ResizeNwSe));
    assert_eq!(at([723.0, 451.0]), Some(ResizeNwSe));
    assert_eq!(at([723.0, 98.0]), Some(ResizeNeSw));
    assert_eq!(at([100.0, 451.0]), Some(ResizeNeSw));
    assert_eq!(at([411.5, 98.0]), Some(ResizeVertical));
    assert_eq!(at([723.0, 274.5]), Some(ResizeHorizontal));
    assert_eq!(at([411.5, 274.5]), None, "inside the box the tool's own arrow stays, and outside it nothing rotates");
    assert_eq!(at([760.0, 480.0]), None);
}

#[test]
fn a_corner_radius_widget_wins_over_the_box() {
    // The widget of the second rectangle's bottom-right corner sits inside the corner, not on the
    // box's handle: a press on it rounds the corners, no scale starts.
    let (mut app, ..) = app_with_two_shapes(Tool::Rectangle);
    let widget = [713.0, 441.0];
    tool_event(&mut app, ToolEvent::Down { x: widget[0], y: widget[1], pressure: 1.0 }, NONE);
    assert!(app.ui.corner_radius.is_some() && app.ui.shape_box.is_none());
    tool_event(&mut app, ToolEvent::Up { x: widget[0], y: widget[1] }, NONE);
    // A press right on the handle scales, not rounds.
    tool_event(&mut app, ToolEvent::Down { x: 723.0, y: 451.0, pressure: 1.0 }, NONE);
    assert!(app.ui.shape_box.is_some() && app.ui.corner_radius.is_none());
}

#[test]
fn scale_for_projects_a_corner_drag_onto_the_diagonal() {
    let r = [100.0, 98.0, 723.0, 451.0];
    // The measured drag: both ways by the same factor, (−100, −60) on this 623 × 353 box.
    let [sx, sy] = scale_for(r, [1, 1], [-100.0, -60.0]);
    assert_eq!(sx, sy);
    assert!((sx - 0.8372).abs() < 1e-3, "{sx}");
    // The top-left corner pulls the other way.
    let [s, _] = scale_for(r, [-1, -1], [100.0, 60.0]);
    assert!((s - 0.8372).abs() < 1e-3, "{s}");
    // A drag along the other diagonal changes nothing.
    assert_eq!(scale_for(r, [1, 1], [353.0, -623.0]), [1.0, 1.0]);
    // Edges scale one way; the box can't be flipped or blown up without bound.
    assert_eq!(scale_for(r, [1, 0], [-100.0, 500.0]), [1.0 - 100.0 / 623.0, 1.0]);
    assert_eq!(scale_for(r, [0, 1], [50.0, -60.0]), [1.0, 1.0 - 60.0 / 353.0]);
    assert_eq!(scale_for(r, [1, 0], [-9000.0, 0.0]), [0.01, 1.0]);
    assert_eq!(scale_for(r, [1, 1], [f64::NAN, 0.0]), [1.0, 1.0]);
    assert_eq!(scale_for(r, [0, 0], [10.0, 10.0]), [1.0, 1.0]);
}

#[test]
fn a_rotated_view_turns_the_handle_cursors() {
    use egui::CursorIcon::{ResizeHorizontal, ResizeNeSw, ResizeNwSe, ResizeVertical};
    let (app, ..) = app_with_two_shapes(Tool::PathSelection);
    let at = |rotation: f32, p: [f64; 2]| crate::shape_bounds_box::cursor(&app, &view(rotation), p);
    assert_eq!(at(0.0, [723.0, 451.0]), Some(ResizeNwSe));
    // A quarter turn swaps the diagonals and the edge arrows.
    assert_eq!(at(90.0, [723.0, 451.0]), Some(ResizeNeSw));
    assert_eq!(at(90.0, [723.0, 98.0]), Some(ResizeNwSe));
    assert_eq!(at(90.0, [723.0, 274.5]), Some(ResizeVertical));
    assert_eq!(at(90.0, [411.5, 98.0]), Some(ResizeHorizontal));
}

#[test]
fn hidden_paths_hide_the_box_and_its_handles_take_no_press() {
    // View › Extras off, or View › Show › Target Path off.
    for (extras, target_path) in [(false, true), (true, false)] {
        let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
        app.ui.view.extras = extras;
        app.ui.view.show.target_path = target_path;
        assert!(cursor(&app, [723.0, 451.0]).is_none(), "extras {extras}");
        tool_event(&mut app, ToolEvent::Down { x: 723.0, y: 451.0, pressure: 1.0 }, NONE);
        assert!(app.ui.shape_box.is_none(), "extras {extras}: no scale drag starts");
        tool_event(&mut app, ToolEvent::Up { x: 723.0, y: 451.0 }, NONE);
        near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);
        near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
    }
}

/// Press the bottom-right handle and drag it, without releasing.
fn press_and_drag(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 723.0, y: 451.0, pressure: 1.0 }, NONE);
    tool_event(app, ToolEvent::Move { x: 623.0, y: 391.0, pressure: 1.0 }, NONE);
    assert!(app.ui.shape_box.is_some());
}

fn release_changes_nothing(app: &mut PhotocraftApp, a: u64, b: u64, steps: usize, why: &str) {
    tool_event(app, ToolEvent::Up { x: 623.0, y: 391.0 }, NONE);
    assert!(app.ui.shape_box.is_none(), "{why}");
    near(bounds(app, a), [100.0, 98.0, 403.0, 301.0]);
    near(bounds(app, b), [420.0, 318.0, 723.0, 451.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{why}: no history step");
}

#[test]
fn escape_or_a_tool_change_cancels_a_handle_drag() {
    // Esc.
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    let steps = app.session.active().unwrap().history.past_len();
    press_and_drag(&mut app);
    // Through the shortcut handler, as the key arrives in the app.
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: NONE }],
        ..Default::default()
    });
    crate::shortcuts::handle(&mut app, &ctx);
    ctx.end_pass().textures_delta.clear();
    assert!(app.ui.shape_box.is_none());
    release_changes_nothing(&mut app, a, b, steps, "Esc");
    // The next press works again.
    drag(&mut app, [723.0, 451.0], [623.0, 391.0]);
    near(bounds(&app, a), [100.0, 98.0, 354.0, 268.0]);

    // Another tool, picked mid-drag.
    let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
    let steps = app.session.active().unwrap().history.past_len();
    press_and_drag(&mut app);
    app.ui.tool = Tool::Brush;
    app.sync_views();
    assert!(app.ui.shape_box.is_none());
    release_changes_nothing(&mut app, a, b, steps, "tool change");

    // Even without a frame between the change and the release.
    let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
    let steps = app.session.active().unwrap().history.past_len();
    press_and_drag(&mut app);
    app.ui.tool = Tool::Move;
    release_changes_nothing(&mut app, a, b, steps, "tool change, same frame");
}

#[test]
fn a_document_switch_or_an_undo_cancels_a_handle_drag() {
    let (mut app, a, b) = app_with_two_shapes(Tool::PathSelection);
    let steps = app.session.active().unwrap().history.past_len();
    press_and_drag(&mut app);
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    app.sync_views();
    assert!(app.ui.shape_box.is_none());
    let other = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Up { x: 623.0, y: 391.0 }, NONE);
    assert_eq!(app.session.active().unwrap().history.past_len(), other, "nothing lands in the other document");
    assert!(app.session.set_active(0));
    near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);
    near(bounds(&app, b), [420.0, 318.0, 723.0, 451.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);

    // An edit under the drag (Undo here): the release applies nothing to the changed shapes.
    let (mut app, ..) = app_with_two_shapes(Tool::PathSelection);
    press_and_drag(&mut app);
    app.run("edit.undo", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Up { x: 623.0, y: 391.0 }, NONE);
    assert!(app.ui.shape_box.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// With background jobs on, as in the desktop app, `edit.transform` lands after `run` returns.
/// The scale is still one step named "Transform Path", and the step before it (the second
/// shape's "New Shape Layer") keeps its name. Renaming the top step after `run` named the
/// shape's step "Transform Path" and left the scale as "Free Transform".
#[test]
fn with_background_jobs_the_scale_step_is_named_and_the_one_before_is_kept() {
    let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
    app.background_jobs = true;
    let before: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
    drag(&mut app, [723.0, 451.0], [623.0, 391.0]);
    let ctx = egui::Context::default();
    for _ in 0..2000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.session.has_jobs(), "the transform job never landed");
    near(bounds(&app, a), [100.0, 98.0, 354.0, 268.0]);
    near(bounds(&app, b), [368.0, 282.0, 621.0, 394.0]);
    let after: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
    assert_eq!(after.len(), before.len() + 1, "one history step: {after:?}");
    assert_eq!(after[..before.len()], before[..], "the earlier steps keep their names");
    assert_eq!(after.last().map(String::as_str), Some("Transform Path"));
}

/// A switch to another tool during a handle drag ends the drag, also when the new tool shows the
/// box too; the release scales nothing and is still the box's (it doesn't reach the new tool).
#[test]
fn a_tool_switch_during_a_drag_ends_it_and_takes_its_release() {
    let (mut app, a, b) = app_with_two_shapes(Tool::Rectangle);
    let steps = app.session.active().unwrap().history.past_len();
    let (ba, bb) = (bounds(&app, a), bounds(&app, b));
    tool_event(&mut app, ToolEvent::Down { x: 723.0, y: 451.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 623.0, y: 391.0, pressure: 1.0 }, NONE);
    assert!(app.ui.shape_box.is_some());
    app.ui.tool = Tool::PathSelection;
    assert!(crate::shape_bounds_box::pointer(&mut app, ToolEvent::Up { x: 623.0, y: 391.0 }, NONE), "the release is the box's");
    assert!(app.ui.shape_box.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!((bounds(&app, a), bounds(&app, b)), (ba, bb));
}
