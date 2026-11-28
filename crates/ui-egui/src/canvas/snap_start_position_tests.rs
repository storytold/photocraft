//! #16855: a Move-tool drag of one layer lines the layer up with its own start position and pulls to it:
//! the rectangle the drag started from counts as a target like any other layer's box (edges and centres).
//! Every expected value below is derived from the rectangles this file sets up, never a fixed literal:
//! an edge guide runs from the far end of the live box to the far end of the start box.

use super::*;
use crate::canvas::transform_controls::{ControlsPress, begin_controls};
use crate::canvas::{ToolEvent, tool_event, tool_event_response};
use craft::behavior::geometry::SnapLine;
use serde_json::json;

/// The moving layer's box at the press. Its centre is off the document's centre line on both axes.
const M: [f64; 4] = [600.0, 420.0, 750.0, 540.0];
/// A quiet other layer, far from the drag paths used here.
const S: [f64; 4] = [100.0, 100.0, 300.0, 260.0];

fn app_with_mover() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 800})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 1.0;
    // The 1000 x 800 document centred in the frames' window: every document point these tests use
    // lands on the canvas.
    app.ui.views[0].center = [500.0, 400.0];
    app.ui.views[0].clear_pending();
    app.run("layer.new.layer", json!({"name": "still"})).unwrap();
    app.run("select.rect", json!({"x": S[0] as i64, "y": S[1] as i64, "width": (S[2] - S[0]) as i64, "height": (S[3] - S[1]) as i64})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
    app.run("select.rect", json!({"x": M[0] as i64, "y": M[1] as i64, "width": (M[2] - M[0]) as i64, "height": (M[3] - M[1]) as i64})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"name": "mover"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.view.show.smart_guides = true;
    app
}

fn bounds_of(app: &PhotocraftApp, name: &str) -> photocraft_geom::Rect {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.surface().unwrap().content_bounds()
}

fn mover_id(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

/// The press at the mover's centre; the Move to `to`, with the drag left live, giving that move's snap lines.
fn drag_move(app: &mut PhotocraftApp, to: [f64; 2]) -> Vec<SnapLine> {
    let m = egui::Modifiers::NONE;
    let at = [(M[0] + M[2]) / 2.0, (M[1] + M[3]) / 2.0];
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, m);
    tool_event_response(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m).snapped
}

fn drag_up(app: &mut PhotocraftApp, to: [f64; 2]) {
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, egui::Modifiers::NONE);
}

/// The horizontal (across y) line at `at`.
fn h_line(lines: &[SnapLine], at: f64) -> Option<&SnapLine> {
    lines.iter().find(|l| l.along == Axis::X && (l.position - at).abs() < 1e-9)
}

/// The vertical (across x) line at `at`.
fn v_line(lines: &[SnapLine], at: f64) -> Option<&SnapLine> {
    lines.iter().find(|l| l.along == Axis::Y && (l.position - at).abs() < 1e-9)
}

/// A target of kind at `at`, with the mover excluded as during a layers drag.
fn near(app: &PhotocraftApp, exclude: &[LayerId], along: Axis, at: f64) -> Option<(SnapKind, f64)> {
    targets(app, 0, exclude).nearest(along, at + 0.5, 1.0).map(|t| (t.kind, t.position))
}

/// Done when 1: a horizontal drag with y unchanged lines the live box up with its start box - the
/// top, bottom and centre of the start box each draw their line; the edge lines run from the far
/// end of one box to the far end of the other.
#[test]
fn the_start_position_lines_up_under_a_drag() {
    let mut app = app_with_mover();
    // End the drag where the live box's right edge sits 13 px past the quiet layer's right edge:
    // no x anchor of the live box is within the snap distance of any other target, and y is unchanged.
    let dx = S[2] + 13.0 - M[2];
    let to = [(M[0] + M[2]) / 2.0 + dx, (M[1] + M[3]) / 2.0];
    let lines = drag_move(&mut app, to);
    let (lo, hi) = (M[0] + dx, M[2] + dx); // the live box along x
    let top = h_line(&lines, M[1]).expect("a line at the start box's top edge");
    assert_eq!(top.kind, SnapKind::LayerEdge);
    assert_eq!(top.span, Some((lo.min(M[0]), hi.max(M[2]))), "end of the live box to end of the start box");
    let bottom = h_line(&lines, M[3]).expect("a line at the start box's bottom edge");
    assert_eq!(bottom.kind, SnapKind::LayerEdge);
    assert_eq!(bottom.span, Some((lo.min(M[0]), hi.max(M[2]))));
    let centre = h_line(&lines, (M[1] + M[3]) / 2.0).expect("a line at the start box's centre");
    assert_eq!(centre.kind, SnapKind::LayerCenter);
    assert!(lines.iter().all(|l| l.along == Axis::X), "the x drag lands past every vertical target: {lines:?}");
    drag_up(&mut app, to);
    let b = bounds_of(&app, "mover");
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (lo as i32, M[1] as i32, hi as i32, M[3] as i32), "y stayed put, x kept its raw offset");
}

/// Done when 2: the start position pulls like another layer's box, within the same distance:
/// a drag that lands the live box's left edge 3 px past the start box's right edge snaps to that edge.
#[test]
fn the_start_position_pulls_the_copy_to_its_edge() {
    let mut app = app_with_mover();
    let dx = (M[2] - M[0]) + 3.0;
    let to = [(M[0] + M[2]) / 2.0 + dx, (M[1] + M[3]) / 2.0];
    let lines = drag_move(&mut app, to);
    let pull = v_line(&lines, M[2]).expect("the start box's right edge is aligned and pulled to");
    assert_eq!(pull.kind, SnapKind::LayerEdge);
    drag_up(&mut app, to);
    let b = bounds_of(&app, "mover");
    assert_eq!((b.x0, b.y0), (M[2] as i32, M[1] as i32), "the copy lands at the start rect's edge");
}

/// While the drag runs the start box is in the target set with the mover excluded from the walk;
/// after the release it is gone.
#[test]
fn the_start_box_is_a_target_only_while_the_drag_runs() {
    let mut app = app_with_mover();
    let mover = mover_id(&app);
    let dx = -(M[0] - S[0] - 69.0);
    let to = [(M[0] + M[2]) / 2.0 + dx, (M[1] + M[3]) / 2.0];
    let _lines = drag_move(&mut app, to);
    for (along, at, kind) in [(Axis::Y, M[0], SnapKind::LayerEdge), (Axis::Y, M[2], SnapKind::LayerEdge), (Axis::Y, (M[0] + M[2]) / 2.0, SnapKind::LayerCenter)]
    {
        assert_eq!(near(&app, &[mover], along, at), Some((kind, at)), "the start box's edge is a target during the drag");
    }
    drag_up(&mut app, to);
    for (along, at) in [(Axis::Y, M[0]), (Axis::Y, M[2]), (Axis::Y, (M[0] + M[2]) / 2.0)] {
        assert_eq!(near(&app, &[], along, at), None, "{at}: no target stands at the start box after the release");
    }
}

/// A drag whose box lands clear of every target - start box, quiet layer, document - keeps today's
/// behaviour: no line and the raw offset.
#[test]
fn an_unaligned_drag_draws_no_line_and_keeps_the_raw_offset() {
    let mut app = app_with_mover();
    let (dx, dy) = (S[0] + 69.0 - M[0], M[3] + 10.0 - M[1]);
    let to = [(M[0] + M[2]) / 2.0 + dx, (M[1] + M[3]) / 2.0 + dy];
    let lines = drag_move(&mut app, to);
    assert!(lines.iter().all(|l| l.kind != SnapKind::LayerEdge && l.kind != SnapKind::LayerCenter), "no layer guide for an unaligned drag: {lines:?}");
    drag_up(&mut app, to);
    let b = bounds_of(&app, "mover");
    assert_eq!((b.x0, b.y0, b.x1, b.y1), ((M[0] + dx) as i32, (M[1] + dy) as i32, (M[2] + dx) as i32, (M[3] + dy) as i32));
}

/// Other layers keep pulling and lining exactly as before, with the start box in the set: the
/// quiet layer's edge still takes the live box's edge.
#[test]
fn other_layers_still_pull_and_line_as_before() {
    let mut app = app_with_mover();
    let dx = S[2] - 3.0 - M[0];
    let to = [(M[0] + M[2]) / 2.0 + dx, (M[1] + M[3]) / 2.0];
    let lines = drag_move(&mut app, to);
    let hit = v_line(&lines, S[2]).expect("the quiet layer's right edge aligns and pulls");
    assert_eq!(hit.kind, SnapKind::LayerEdge);
    assert_eq!(hit.span, Some((S[1].min(M[1]), S[3].max(M[3]))), "end of one box to end of the other");
    drag_up(&mut app, to);
    assert_eq!(bounds_of(&app, "mover").x0, S[2] as i32, "pulled to the quiet layer's edge");
}

/// Measured: an open Free Transform draws no start-position lines. With Free Transform open the
/// layer is the transform's, no layers drag runs, and the start box must not join the targets.
#[test]
fn free_transform_open_adds_no_start_position_target() {
    let mut app = app_with_mover();
    app.ui.tool_options.move_show_transform = true;
    app.sync_views();
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started, "the frame's handle opens Free Transform");
    let mover = mover_id(&app);
    for (along, at) in [(Axis::Y, M[0]), (Axis::Y, M[2]), (Axis::Y, (M[0] + M[2]) / 2.0), (Axis::X, M[1]), (Axis::X, M[3]), (Axis::X, (M[1] + M[3]) / 2.0)] {
        assert_eq!(near(&app, &[mover], along, at), None, "{along:?} {at}: the start box is no target with Free Transform open");
    }
}
