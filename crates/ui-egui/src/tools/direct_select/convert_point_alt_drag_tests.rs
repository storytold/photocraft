//! #12740: with the Convert Point tool, an ⌥-drag from an anchor pulls out ONE handle: the
//! out-handle goes to the pointer and the in-handle stays retracted on the anchor, so the point
//! stays a corner. A drag without ⌥ keeps the mirrored smooth pair, and the cases the reference
//! was not measured on (a smooth anchor, an open subpath's end anchor, ⇧ with ⌥, the Pen's ⌥ and
//! an ⌥-click without a drag) keep today's behaviour.

use egui::Modifiers;
use photocraft_doc::Knot;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use super::convert_point_history_tests::{added, drag};
use super::{Drag, pointer};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured corner of the reference path's closed square: its knot [0, 1].
const CORNER: [f64; 2] = [300.0, 300.0];
/// Where its out-handle ends after the measured (+50, +30) drag.
const DRAG_TO: [f64; 2] = [350.0, 330.0];

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// The reference document: 800x600 px with a saved path, targeted in the Paths panel: a closed
/// corner square, an open straight line and an open S-curve of two smooth points.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("path.set", json!({"name": "Path 1", "path": reference_path()})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.sync_views();
    app.ui.tool = tool;
    app
}

fn reference_path() -> serde_json::Value {
    json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [
            {"anchor": [400, 450], "in": [300, 550], "out": [500, 350], "smooth": true},
            {"anchor": [700, 450], "in": [600, 550], "out": [800, 350], "smooth": true},
        ]},
    ]})
}

fn path(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths[0].path.clone()
}

fn knot(app: &PhotocraftApp, s: usize, k: usize) -> Knot {
    path(app).subpaths[s].knots[k]
}

/// The measured state: only the out-handle comes out, the in-handle stays on the anchor and the
/// point is a corner, in one History step that undo takes back.
#[test]
fn alt_drag_from_a_corner_pulls_out_only_the_out_handle() {
    let mut app = app(Tool::ConvertPoint);
    let before = path(&app);
    let steps = added(&mut app, |a| drag(a, CORNER, DRAG_TO, Modifiers::ALT));
    let k = knot(&app, 0, 1);
    assert_eq!((k.anchor, k.out_ctrl, k.in_ctrl, k.smooth), (p(300.0, 300.0), p(350.0, 330.0), p(300.0, 300.0), false), "{steps:?}");
    for other in [[0usize, 0usize], [0, 2], [0, 3]] {
        assert_eq!(knot(&app, other[0], other[1]), before.subpaths[other[0]].knots[other[1]], "the neighbour {other:?} stays");
    }
    assert_eq!(path(&app).subpaths[1..], before.subpaths[1..], "the other subpaths stay");
    assert_eq!(steps, ["Change Anchor Point"], "one history step");
    assert!(app.session.undo());
    assert_eq!(path(&app), before);
}

/// A drag without ⌥ keeps the symmetric pair: the out-handle at the pointer, the in-handle its
/// mirror, the point smooth.
#[test]
fn a_drag_without_alt_keeps_the_mirrored_pair() {
    let mut app = app(Tool::ConvertPoint);
    assert_eq!(added(&mut app, |a| drag(a, CORNER, DRAG_TO, Modifiers::NONE)), ["Change Anchor Point"]);
    let k = knot(&app, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(350.0, 330.0), p(250.0, 270.0), true), "{k:?}");
}

/// The drag preview is the same one-handle result the committed step writes.
#[test]
fn the_drag_preview_shows_the_same_one_handle_result() {
    let mut app = app(Tool::ConvertPoint);
    let mods = Modifiers::ALT;
    pointer(&mut app, ToolEvent::Down { x: CORNER[0], y: CORNER[1], pressure: 1.0 }, mods);
    pointer(&mut app, ToolEvent::Move { x: DRAG_TO[0], y: DRAG_TO[1], pressure: 1.0 }, mods);
    let preview = match &app.ui.direct_selection.drag {
        Some(Drag::Edit { preview: Some(preview), .. }) => preview.clone(),
        other => panic!("expected a running edit drag with a preview: {other:?}"),
    };
    let k = preview.subpaths[0].knots[1];
    assert_eq!((k.anchor, k.out_ctrl, k.in_ctrl, k.smooth), (p(300.0, 300.0), p(350.0, 330.0), p(300.0, 300.0), false), "the preview");
    tool_event(&mut app, ToolEvent::Up { x: DRAG_TO[0], y: DRAG_TO[1] }, mods);
    assert_eq!(path(&app), preview, "the step is what the preview showed");
}

/// The point is not smooth: dragging its out-handle afterwards moves only that handle.
#[test]
fn dragging_a_handle_after_the_alt_drag_moves_only_that_handle() {
    let mut app = app(Tool::ConvertPoint);
    drag(&mut app, CORNER, DRAG_TO, Modifiers::ALT);
    assert_eq!(added(&mut app, |a| drag(a, DRAG_TO, [360.0, 340.0], Modifiers::NONE)), ["Corner Drag"]);
    let k = knot(&app, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(360.0, 340.0), p(300.0, 300.0), false), "{k:?}");
}

/// `path.convertPoint` rejects a bad `independent` as an error, and `false` keeps today's edit.
#[test]
fn the_convert_command_refuses_a_bad_value_for_the_new_option() {
    let mut app = app(Tool::ConvertPoint);
    let before = path(&app);
    for bad in [json!("yes"), json!(7), json!([true]), json!({"a": 1})] {
        let params = json!({"name": "Path 1", "subpath": 0, "knot": 1, "out": [350, 330], "independent": bad});
        let e = app.run("path.convertPoint", params).unwrap_err();
        assert!(e.contains("independent"), "{bad}: {e}");
    }
    assert_eq!(path(&app), before, "a refused param changes nothing");
    let params = json!({"name": "Path 1", "subpath": 0, "knot": 1, "out": [350, 330], "independent": false});
    app.run("path.convertPoint", params).unwrap();
    let k = knot(&app, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(350.0, 330.0), p(250.0, 270.0), true), "false is today's edit: {k:?}");
}

/// Not measured, so today's behaviour: an ⌥-drag from an anchor that is already smooth still
/// replaces both handles with the pointer's handle and its mirror.
#[test]
fn an_alt_drag_from_a_smooth_anchor_keeps_the_mirrored_pair() {
    let mut app = app(Tool::ConvertPoint);
    let smooth = json!({"subpaths": [{"closed": true, "knots": [
        [100, 300],
        {"anchor": [300, 300], "in": [200, 300], "out": [400, 300], "smooth": true},
        [300, 500],
        [100, 500],
    ]}]});
    app.run("path.set", json!({"name": "Path 1", "path": smooth})).unwrap();
    app.sync_views();
    assert_eq!(added(&mut app, |a| drag(a, CORNER, DRAG_TO, Modifiers::ALT)), ["Change Anchor Point"]);
    let k = knot(&app, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(350.0, 330.0), p(250.0, 270.0), true), "{k:?}");
}

/// Not measured, so today's behaviour: an ⌥-drag from the end anchor of an open subpath.
#[test]
fn an_alt_drag_from_an_open_subpaths_end_anchor_keeps_both_handles() {
    let mut app = app(Tool::ConvertPoint);
    assert_eq!(added(&mut app, |a| drag(a, [700.0, 300.0], [750.0, 330.0], Modifiers::ALT)), ["Change Anchor Point"]);
    let k = knot(&app, 1, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(750.0, 330.0), p(650.0, 270.0), true), "{k:?}");
}

/// Not measured, so today's behaviour: ⇧ together with ⌥, and the Pen's ⌥ (the same conversion
/// reached another way).
#[test]
fn an_alt_drag_with_shift_or_on_the_pen_keeps_the_mirrored_pair() {
    let mut shifted = app(Tool::ConvertPoint);
    assert_eq!(added(&mut shifted, |a| drag(a, CORNER, DRAG_TO, Modifiers::SHIFT | Modifiers::ALT)), ["Change Anchor Point"], "⇧⌥");
    let k = knot(&shifted, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(350.0, 330.0), p(250.0, 270.0), true), "⇧⌥: {k:?}");
    let mut pen = app(Tool::Pen);
    assert_eq!(added(&mut pen, |a| drag(a, CORNER, DRAG_TO, Modifiers::ALT)), ["Convert Point"], "the Pen's ⌥");
    let k = knot(&pen, 0, 1);
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(350.0, 330.0), p(250.0, 270.0), true), "Pen ⌥: {k:?}");
}

/// An ⌥-drag on a handle end is the plain handle drag it already is: the tool moves that handle
/// alone, with ⌥ or without.
#[test]
fn an_alt_drag_on_a_handle_end_is_the_plain_handle_drag() {
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        let mut app = app(Tool::ConvertPoint);
        drag(&mut app, CORNER, DRAG_TO, Modifiers::NONE);
        assert_eq!(added(&mut app, |a| drag(a, DRAG_TO, [360.0, 340.0], mods)), ["Corner Drag"], "{mods:?}");
        let k = knot(&app, 0, 1);
        assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (p(360.0, 340.0), p(250.0, 270.0), false), "{mods:?}: {k:?}");
    }
}

/// Not measured, so today's behaviour: an ⌥-click without a drag still leaves a corner with both
/// handles retracted.
#[test]
fn an_alt_click_without_a_drag_leaves_a_plain_corner() {
    let mut app = app(Tool::ConvertPoint);
    assert_eq!(added(&mut app, |a| drag(a, CORNER, CORNER, Modifiers::ALT)), ["Change Anchor Point"], "⌥-click");
    assert_eq!(knot(&app, 0, 1), Knot::corner(300.0, 300.0), "both handles retracted");
}
