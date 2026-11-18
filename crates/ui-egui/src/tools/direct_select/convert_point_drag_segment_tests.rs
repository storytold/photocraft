//! #12741: a drag that starts on a segment with the Convert Point tool works as a temporary
//! Direct Selection, as in the reference: with nothing selected the straight segment moves
//! ("Drag Segment"), with the whole path selected the whole path moves ("Drag Path").

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use super::convert_point_history_tests::{added, drag};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

const BOTTOM_EDGE: [f64; 2] = [200.0, 500.0];

/// The reference document: 800×600 px with a saved path, targeted in the Paths panel: a closed
/// corner square, an open straight line and an open S-curve of two smooth points.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"knots": [[400, 300], [700, 300]]},
        {"knots": [
            {"anchor": [400, 450], "in": [300, 550], "out": [500, 350], "smooth": true},
            {"anchor": [700, 450], "in": [600, 550], "out": [800, 350], "smooth": true},
        ]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.sync_views();
    app.ui.tool = tool;
    app
}

fn path(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths[0].path.clone()
}

/// `p` with knots `which` (all when `None`) moved by `d`, handles included.
fn moved(p: &Path, which: Option<&[[usize; 2]]>, d: [f64; 2]) -> Path {
    let mut p = p.clone();
    for (s, sub) in p.subpaths.iter_mut().enumerate() {
        for (k, kn) in sub.knots.iter_mut().enumerate() {
            if which.is_none_or(|w| w.contains(&[s, k])) {
                for q in [&mut kn.anchor, &mut kn.in_ctrl, &mut kn.out_ctrl] {
                    *q = Point::new(q.x + d[0], q.y + d[1]);
                }
            }
        }
    }
    p
}

/// `a` and `b` are the same path to within the view's `f32` mapping of a pointer to document pixels: a drag's
/// delta carries rounding below a thousandth of a pixel.
#[track_caller]
fn assert_same_path(a: &Path, b: &Path, what: &str) {
    assert_eq!(a.subpaths.len(), b.subpaths.len(), "{what}: subpaths");
    for (sa, sb) in a.subpaths.iter().zip(&b.subpaths) {
        assert_eq!((sa.closed, sa.knots.len()), (sb.closed, sb.knots.len()), "{what}: shape");
        for (ka, kb) in sa.knots.iter().zip(&sb.knots) {
            assert_eq!(ka.smooth, kb.smooth, "{what}: smooth");
            for (pa, pb) in [(ka.anchor, kb.anchor), (ka.in_ctrl, kb.in_ctrl), (ka.out_ctrl, kb.out_ctrl)] {
                assert!((pa.x - pb.x).abs() < 1e-3 && (pa.y - pb.y).abs() < 1e-3, "{what}: {pa:?} vs {pb:?}");
            }
        }
    }
}

/// Every anchor of the path selected, by a ⌘-marquee (Direct Selection in this tool).
fn select_all(app: &mut PhotocraftApp) {
    drag(app, [0.0, 0.0], [800.0, 600.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors.len(), 8);
}

#[test]
fn with_nothing_selected_a_segment_drag_moves_only_that_segment() {
    let mut app = app(Tool::ConvertPoint);
    let before = path(&app);
    let steps = added(&mut app, |a| drag(a, BOTTOM_EDGE, [200.0, 540.0], Modifiers::NONE));
    assert_eq!(steps, ["Drag Segment"]);
    let after = path(&app);
    assert_same_path(&after, &moved(&before, Some(&[[0, 2], [0, 3]]), [0.0, 40.0]), "the other anchors stay");
    assert!(app.session.undo());
    assert_eq!(path(&app), before);
}

#[test]
fn the_closing_segment_of_a_closed_subpath_moves_too() {
    let mut app = app(Tool::ConvertPoint);
    let before = path(&app);
    assert_eq!(added(&mut app, |a| drag(a, [100.0, 400.0], [60.0, 400.0], Modifiers::NONE)), ["Drag Segment"]);
    assert_same_path(&path(&app), &moved(&before, Some(&[[0, 3], [0, 0]]), [-40.0, 0.0]), "the closing segment");
}

#[test]
fn with_the_whole_path_selected_a_segment_drag_moves_the_whole_path() {
    let mut app = app(Tool::ConvertPoint);
    select_all(&mut app);
    let before = path(&app);
    let steps = added(&mut app, |a| drag(a, BOTTOM_EDGE, [200.0, 540.0], Modifiers::NONE));
    assert_eq!(steps, ["Drag Path"]);
    assert_same_path(&path(&app), &moved(&before, None, [0.0, 40.0]), "all three subpaths moved");
    assert_eq!(app.ui.direct_selection.anchors.len(), 8, "the selection stays");
}

#[test]
fn direct_selection_segment_drags_keep_their_behaviour() {
    for all in [false, true] {
        let mut app = app(Tool::DirectSelection);
        if all {
            select_all(&mut app);
        }
        let before = path(&app);
        assert_eq!(added(&mut app, |a| drag(a, BOTTOM_EDGE, [200.0, 540.0], Modifiers::NONE)), ["Drag Segment"]);
        assert_same_path(&path(&app), &moved(&before, Some(&[[0, 2], [0, 3]]), [0.0, 40.0]), &format!("all selected: {all}"));
    }
}

#[test]
fn a_command_anchor_drag_released_without_command_keeps_its_own_step_name() {
    let mut app = app(Tool::ConvertPoint);
    select_all(&mut app);
    testing::press(&mut app, [300.0, 300.0], Modifiers::COMMAND);
    testing::move_to(&mut app, [320.0, 300.0], Modifiers::COMMAND);
    let steps = added(&mut app, |a| testing::release(a, [320.0, 300.0], Modifiers::NONE));
    assert_eq!(steps, ["Drag Anchor Points"]);
}

#[test]
fn move_anchors_takes_the_drag_path_step_name_and_refuses_any_other() {
    let mut app = app(Tool::ConvertPoint);
    let params = |name: serde_json::Value| json!({"name": "Path 1", "anchors": [[0, 0]], "move": [1, 1], "stepName": name});
    assert_eq!(added(&mut app, |a| a.run("path.moveAnchors", params(json!("Drag Path"))).map(|_| ()).unwrap()), ["Drag Path"]);
    let before = path(&app);
    for bad in [json!("Drag Segment"), json!("Drag Path "), json!(1)] {
        let e = app.run("path.moveAnchors", params(bad.clone())).unwrap_err();
        assert!(e.contains("stepName"), "{bad}: {e}");
    }
    assert_eq!(path(&app), before);
}
