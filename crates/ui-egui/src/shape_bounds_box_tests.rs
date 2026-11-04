//! #11107: the box round several selected shape layers (Path Selection and the shape tools), its
//! handles scaling them, measured against the reference application: two rectangles
//! 100,98–403,301 and 420,318–723,451 scaled by dragging the bottom-right handle by (−100, −60).

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::shape_bounds_box::scale_for;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

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
fn the_box_needs_two_shapes_and_a_path_or_shape_tool() {
    // One selected shape keeps its own widgets: no box, no drag.
    let (mut app, a, _) = app_with_two_shapes(Tool::PathSelection);
    app.run("layer.select", json!({"layer": a})).unwrap();
    assert!(crate::shape_bounds_box::cursor(&app, [403.0, 301.0]).is_none());
    tool_event(&mut app, ToolEvent::Down { x: 403.0, y: 301.0, pressure: 1.0 }, NONE);
    assert!(app.ui.shape_box.is_none());
    tool_event(&mut app, ToolEvent::Up { x: 403.0, y: 301.0 }, NONE);
    near(bounds(&app, a), [100.0, 98.0, 403.0, 301.0]);

    // Another tool has none either.
    let (mut app, ..) = app_with_two_shapes(Tool::Move);
    assert!(crate::shape_bounds_box::cursor(&app, [723.0, 451.0]).is_none());
    app.ui.tool = Tool::Brush;
    assert!(crate::shape_bounds_box::cursor(&app, [723.0, 451.0]).is_none());
}

#[test]
fn the_handles_show_their_scale_cursors() {
    use egui::CursorIcon::{ResizeHorizontal, ResizeNeSw, ResizeNwSe, ResizeVertical};
    let (app, ..) = app_with_two_shapes(Tool::PathSelection);
    let at = |p: [f64; 2]| crate::shape_bounds_box::cursor(&app, p);
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
