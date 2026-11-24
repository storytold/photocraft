//! #13204: arrow-key nudges of the anchors Direct Selection has selected are one History step
//! "Nudge Path" while they follow each other; one Undo takes them all back. Another anchor
//! selection or another edit in between starts a new step, and a segment-only selection is not
//! nudged.

use egui::{Key, Modifiers};
use photocraft_geom::Point;
use serde_json::json;

use super::convert_point_history_tests::{added, click, drag};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// A square work path with corners at (40, 40), (120, 40), (120, 120) and (40, 120).
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    let path = json!({"subpaths": [{"closed": true, "knots": [[40, 40], [120, 40], [120, 120], [40, 120]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

fn anchor(app: &PhotocraftApp, knot: usize) -> Point {
    app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[knot].anchor
}

fn nudge(app: &mut PhotocraftApp, mods: Modifiers) {
    testing::key(app, Key::ArrowRight, mods);
}

#[test]
fn consecutive_nudges_are_one_nudge_path_step() {
    let mut app = app();
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 3]]);
    let start = anchor(&app, 3);
    let steps = added(&mut app, |a| nudge(a, Modifiers::NONE));
    assert_eq!(steps, ["Nudge Path"]);
    let one = anchor(&app, 3).x - start.x;
    assert!(one > 0.0, "the plain nudge moved the anchor right: {:?}", anchor(&app, 3));
    let more = added(&mut app, |a| {
        nudge(a, Modifiers::SHIFT);
        nudge(a, Modifiers::NONE);
    });
    assert!(more.is_empty(), "the next nudges joined the first step: {more:?}");
    let end = anchor(&app, 3);
    assert!((end.x - start.x - 12.0 * one).abs() < 1e-6 && end.y == start.y, "⇧ went ten times as far as a plain nudge: {end:?}");
    assert!(app.session.undo());
    assert_eq!(anchor(&app, 3), start, "one Undo took back the whole run");
}

#[test]
fn another_selection_or_edit_starts_a_new_step() {
    let mut app = app();
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    nudge(&mut app, Modifiers::NONE);
    // Another anchor selected and then the first one again: its next nudge is a step of its own.
    let first = anchor(&app, 3);
    click(&mut app, [120.0, 120.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]]);
    click(&mut app, [first.x, first.y], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 3]]);
    assert_eq!(added(&mut app, |a| nudge(a, Modifiers::NONE)), ["Nudge Path"], "the first anchor again");
    click(&mut app, [120.0, 120.0], Modifiers::NONE);
    assert_eq!(added(&mut app, |a| nudge(a, Modifiers::NONE)), ["Nudge Path"], "another anchor");
    let at = anchor(&app, 2);
    let steps = added(&mut app, |a| {
        drag(a, [at.x, at.y], [at.x, at.y + 20.0], Modifiers::NONE);
        nudge(a, Modifiers::NONE);
    });
    assert_eq!(steps, ["Drag Anchor Point", "Nudge Path"], "a drag in between");
}

#[test]
fn a_segment_only_selection_is_not_nudged() {
    let mut app = app();
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty(), "only the segment is selected");
    let before = app.session.active().unwrap().doc.work_path.clone();
    let steps = added(&mut app, |a| {
        nudge(a, Modifiers::NONE);
        nudge(a, Modifiers::NONE);
    });
    assert!(steps.is_empty(), "{steps:?}");
    assert_eq!(app.session.active().unwrap().doc.work_path, before);
}
