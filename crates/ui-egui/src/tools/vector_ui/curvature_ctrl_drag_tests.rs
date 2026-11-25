//! #12757: with the Curvature Pen and a path still being drawn, ⌘/Ctrl-dragging one of its anchors moves that
//! anchor with its handles and leaves every other knot exactly as it was: the move does not solve the curve again.
//! Every value expected here is read from the app's own draft before the gesture and compared with what the
//! gesture made of it (the drag's own delta, and the knots it did not touch), so this file states no handle
//! position of its own.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The five anchors of the path under test, at 100 % zoom on an 800 x 600 document.
const ANCHORS: [[f64; 2]; 5] = [[100.0, 300.0], [250.0, 150.0], [400.0, 300.0], [550.0, 150.0], [700.0, 300.0]];
/// The middle anchor: the one the gesture drags.
const DRAGGED: usize = 2;
/// Where the drag lets go, so the drag's own delta is (0, 100).
const RELEASE: [f64; 2] = [400.0, 400.0];
/// The anchor the path carries on from after the drag, well clear of every anchor drawn and inside the document.
const NEXT: [f64; 2] = [780.0, 420.0];

/// The Curvature Pen in Path mode, the five anchors clicked, and the path still being drawn.
fn drawing() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.ui.tool = Tool::CurvaturePen;
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    for at in ANCHORS {
        testing::click(&mut app, at, Modifiers::NONE);
    }
    assert_eq!(draft(&app).len(), ANCHORS.len(), "the five anchors are the draft being drawn");
    app
}

/// The same path finished with Esc: no draft, the Work Path holds it.
fn finished() -> Box<PhotocraftApp> {
    let mut app = drawing();
    testing::key(&mut app, egui::Key::Escape, Modifiers::NONE);
    assert!(app.ui.pen.is_none(), "Esc ended the path");
    app
}

/// The draft's knots as anchor, in handle and out handle.
fn draft(app: &PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    app.ui.pen.as_ref().expect("the path is still being drawn").triples()
}

/// The Work Path's knots the same way.
fn work(app: &PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    let path = app.session.active().unwrap().doc.work_path.clone().expect("the finished path is the Work Path");
    path.subpaths[0].knots.iter().map(|k| [[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]]).collect()
}

/// How far `after` is from `before`.
fn delta(before: [f64; 2], after: [f64; 2]) -> [f64; 2] {
    [after[0] - before[0], after[1] - before[1]]
}

/// Are two points the same to within a thousandth of a pixel? The pointer's travel is measured through the view, so
/// the drag's delta is only that exact.
fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::press(app, from, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    testing::move_to(app, mid, mods);
    testing::move_to(app, to, mods);
    testing::release(app, to, mods);
    // Let go of the keys: a frame keeps the modifiers of the last event, and the Pen's pass reads them to decide
    // whether ⌘ still makes the pointer Direct Selection.
    testing::modifiers(app, Modifiers::NONE);
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The dragged anchor went to the point the pointer let go, and every other knot of `after` is `before`'s.
fn assert_only_the_dragged_anchor_moved(before: &[[[f64; 2]; 3]], after: &[[[f64; 2]; 3]]) -> [f64; 2] {
    let by = delta(before[DRAGGED][0], after[DRAGGED][0]);
    assert!(same(by, [RELEASE[0] - ANCHORS[DRAGGED][0], RELEASE[1] - ANCHORS[DRAGGED][1]]), "the anchor followed the pointer to {RELEASE:?}, moved {by:?}");
    for side in 1..3 {
        let handle = delta(before[DRAGGED][side], after[DRAGGED][side]);
        assert!(same(handle, by), "handle {side} of the dragged knot moved with it by {by:?}, not {handle:?}");
    }
    for k in 0..ANCHORS.len() {
        if k == DRAGGED {
            continue;
        }
        assert_eq!(after[k], before[k], "knot {k} is exactly what it was: the move solved the curve again");
    }
    by
}

#[test]
fn a_ctrl_drag_of_a_draft_anchor_moves_it_with_its_handles_and_leaves_the_curve_solved_as_it_was() {
    let mut app = drawing();
    let before = draft(&app);
    drag(&mut app, ANCHORS[DRAGGED], RELEASE, Modifiers::COMMAND);
    assert_only_the_dragged_anchor_moved(&before, &draft(&app));
}

#[test]
fn a_ctrl_drag_of_a_draft_anchor_adds_no_step_while_the_path_is_a_draft() {
    // The Curvature Pen writes its path only at the finish, and its step names are #12752's: the drag is an edit of
    // the draft, so it neither adds a step of its own nor writes the document.
    let mut app = drawing();
    let (before, path) = (steps(&app), app.session.active().unwrap().doc.work_path.clone());
    drag(&mut app, ANCHORS[DRAGGED], RELEASE, Modifiers::COMMAND);
    assert_eq!(steps(&app), before, "the drag of a draft is no step yet");
    assert_eq!(app.session.active().unwrap().doc.work_path, path, "and wrote nothing to the document");
    testing::key(&mut app, egui::Key::Enter, Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "the finish is still the one step it is today");
}

#[test]
fn a_plain_drag_of_a_draft_anchor_still_solves_the_whole_curve() {
    let mut app = drawing();
    let before = draft(&app);
    drag(&mut app, ANCHORS[DRAGGED], RELEASE, Modifiers::NONE);
    let after = draft(&app);
    assert!(same(after[DRAGGED][0], RELEASE), "the Pen's own drag put the anchor at the release point");
    for k in [DRAGGED - 1, DRAGGED + 1] {
        assert_ne!(after[k], before[k], "the neighbouring knot {k} was solved again under the drag");
    }
    // The draft is exactly the solve of its own anchors: solving it again changes nothing.
    let mut again = app.ui.pen.as_ref().expect("still drawing").draft.clone();
    again.reflow();
    for (k, knot) in again.knots.iter().enumerate() {
        let solved = [[knot.anchor.x, knot.anchor.y], [knot.in_ctrl.x, knot.in_ctrl.y], [knot.out_ctrl.x, knot.out_ctrl.y]];
        for side in 0..3 {
            assert!(same(after[k][side], solved[side]), "a plain drag left knot {k} part {side} off its own solve");
        }
    }
}

#[test]
fn a_ctrl_drag_of_a_finished_path_moves_the_anchor_with_its_handles_in_one_step() {
    let mut app = finished();
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, ANCHORS[DRAGGED], RELEASE, Modifiers::COMMAND);
    assert_only_the_dragged_anchor_moved(&before, &work(&app));
    assert_eq!(steps(&app), n + 1, "the whole drag is one step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Drag Anchor Point"), "and that step is named as the finished-path drag is today");
}

#[test]
fn drawing_on_from_the_last_anchor_keeps_the_moved_anchor_where_the_drag_put_it() {
    // The next click of the Curvature Pen solves the handles of the knots it has again, and whether it keeps the
    // handles a ⌘/Ctrl-drag gave the moved anchor is one of the cases the issue marks not measured: it stays as it
    // is today, so only the anchor's place is pinned here.
    let mut app = drawing();
    let before = draft(&app);
    drag(&mut app, ANCHORS[DRAGGED], RELEASE, Modifiers::COMMAND);
    let moved = draft(&app);
    testing::click(&mut app, NEXT, Modifiers::NONE);
    let after = draft(&app);
    assert_eq!(after.len(), ANCHORS.len() + 1, "the click added the next anchor");
    assert_eq!(after[DRAGGED][0], moved[DRAGGED][0], "the moved anchor stayed where the drag put it");
    assert_eq!(after[DRAGGED + 1][0], before[DRAGGED + 1][0], "and the last anchor did not move");
}
