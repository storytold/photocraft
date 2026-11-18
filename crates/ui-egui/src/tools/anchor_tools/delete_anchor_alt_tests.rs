//! #12770: ⌥-click with Delete Anchor Point on an anchor of the targeted path selects its whole
//! subpath and edits nothing; on a segment ⌥ still adds an anchor (Add Anchor Point).

use egui::Modifiers;
use serde_json::json;

use super::add_anchor_lock_tests::click_with_modifiers;
use super::delete_anchor_targeted_path_tests::{setup, steps};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::direct_select::PathRef;
use crate::state::Tool;

/// Delete Anchor Point with the saved path "Path 1", a closed corner square
/// (100,300)(300,300)(300,500)(100,500), targeted in the Paths panel.
fn square() -> PhotocraftApp {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn knots(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.paths[0].path.subpaths[0].knots.len()
}

#[test]
fn alt_click_on_an_anchor_selects_the_whole_subpath_and_records_nothing() {
    let mut app = square();
    let n = steps(&app);
    click_with_modifiers(&mut app, [300.0, 500.0], Modifiers::ALT);
    app.sync_views();
    let ds = &app.ui.direct_selection;
    assert_eq!(ds.target, Some(PathRef::Saved(0)));
    assert_eq!(ds.anchors, vec![[0, 0], [0, 1], [0, 2], [0, 3]]);
    assert_eq!(knots(&app), 4);
    assert_eq!(steps(&app), n);
}

#[test]
fn alt_click_on_a_segment_still_adds_an_anchor() {
    let mut app = square();
    let n = steps(&app);
    click_with_modifiers(&mut app, [200.0, 300.0], Modifiers::ALT);
    assert_eq!(knots(&app), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

#[test]
fn alt_drag_from_an_anchor_selects_nothing() {
    let mut app = square();
    let n = steps(&app);
    testing::press(&mut app, [300.0, 500.0], Modifiers::ALT);
    testing::move_to(&mut app, [360.0, 560.0], Modifiers::ALT);
    testing::release(&mut app, [360.0, 560.0], Modifiers::ALT);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(knots(&app), 4);
    assert_eq!(steps(&app), n);
}
