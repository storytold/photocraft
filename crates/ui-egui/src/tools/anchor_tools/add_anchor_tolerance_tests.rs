//! #12732: Add Anchor Point reaches a segment within 5 points, as measured at display scale 1: at
//! 100 % a click 5 px from a segment adds an anchor and one 6 px away does nothing; at 200 % that
//! is 2.5 document px. The reach is the overlay's (storytold/craft-libs#2282); these tests drive it
//! through the canvas. Delete Anchor Point keeps its 6 points.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The measured document: 800 × 600 px with an open straight subpath from (400, 300) to (700, 300)
/// in the saved path "Path 1", selected in the Paths panel, shown at `zoom`.
fn app(tool: Tool, zoom: f32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ui.views[0].clear_pending();
    app.ui.tool = tool;
    let path = json!({"subpaths": [{"closed": false, "knots": [[400, 300], [700, 300]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    testing::clear_of_dialogs(&mut app, [550.0, 300.0]);
    app
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::click(app, at, Modifiers::NONE);
}

/// Does a click `dy` document px below the segment's middle add an anchor there, in one History
/// step? A click that adds nothing must leave the path and the History alone.
fn adds_at(zoom: f32, dy: f64) -> bool {
    let mut app = app(Tool::AddAnchorPoint, zoom);
    let (before, n) = (saved(&app), steps(&app));
    click(&mut app, [550.0, 300.0 + dy]);
    let after = saved(&app);
    if after == before {
        assert_eq!(steps(&app), n, "a click that adds nothing adds no step ({dy} px)");
        return false;
    }
    assert_eq!(after.subpaths[0].knots.len(), 3, "{dy} px");
    let added = after.subpaths[0].knots[1].anchor;
    assert!((added.x - 550.0).abs() < 0.01 && (added.y - 300.0).abs() < 1e-6, "the anchor goes on the segment: {added:?}");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    true
}

#[test]
fn at_100_percent_a_click_5_px_from_a_segment_adds_an_anchor_and_6_px_does_nothing() {
    for dy in [2.0, 4.0, 5.0] {
        assert!(adds_at(1.0, dy), "{dy} px below adds");
        assert!(adds_at(1.0, -dy), "{dy} px above adds");
    }
    assert!(!adds_at(1.0, 6.0));
    assert!(!adds_at(1.0, -6.0));
}

#[test]
fn the_reach_is_in_screen_pixels_so_at_200_percent_it_is_2_5_document_px() {
    assert!(adds_at(2.0, 2.5));
    assert!(!adds_at(2.0, 3.0));
    // And 10 document px at 50 %.
    assert!(adds_at(0.5, 10.0));
    assert!(!adds_at(0.5, 12.0));
}

#[test]
fn delete_anchor_point_still_reaches_an_anchor_within_6_points() {
    let mut app = app(Tool::DeleteAnchorPoint, 1.0);
    let path = json!({"subpaths": [{"closed": false, "knots": [[400, 300], [550, 300], [700, 300]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    click(&mut app, [550.0, 305.5]);
    assert_eq!(saved(&app).subpaths[0].knots.len(), 2, "5.5 px from the middle anchor deletes it");
}
