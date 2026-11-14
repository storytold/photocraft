//! #12732: Add Anchor Point reaches a segment within 5 points, as measured at display scale 1: at
//! 100 % a click 5 px from a segment adds an anchor and one 6 px away does nothing; at 200 % that
//! is 2.5 document px. Other display scales weren't measured and scale it as every canvas
//! tolerance. Delete Anchor Point, Direct Selection and the Pen's Auto Add/Delete keep their 6 points.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: 800 × 600 px with an open straight subpath from (400, 300) to (700, 300)
/// in the saved path "Path 1", selected in the Paths panel, shown at `zoom` (device pixels per
/// document pixel) on a display of `ppp` device pixels per point.
fn app(tool: Tool, zoom: f32, ppp: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ppp = ppp;
    app.ui.tool = tool;
    let path = json!({"subpaths": [{"closed": false, "knots": [[400, 300], [700, 300]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn click(app: &mut PhotocraftApp, [x, y]: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

/// Does a click `dy` document px below the segment's middle add an anchor there, in one History
/// step, Add Anchor Point? A click that adds nothing must leave the path and the History alone.
fn adds_at(app: &mut PhotocraftApp, dy: f64) -> bool {
    let (before, n) = (saved(app), steps(app));
    let p = [550.0, 300.0 + dy];
    let hover = crate::anchor_tools::adds_anchor_at(app, Tool::AddAnchorPoint, p);
    click(app, p);
    let after = saved(app);
    if after == before {
        assert_eq!(steps(app), n, "a click that adds nothing adds no step ({dy} px)");
        assert!(!hover, "the cursor shows + only where a click adds ({dy} px)");
        return false;
    }
    assert_eq!(after.subpaths[0].knots.len(), 3, "{dy} px");
    let added = after.subpaths[0].knots[1].anchor;
    assert!((added.x - 550.0).abs() < 1e-6 && (added.y - 300.0).abs() < 1e-6, "the anchor goes on the segment: {added:?}");
    assert_eq!(steps(app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(hover, "the cursor shows + where a click adds ({dy} px)");
    true
}

#[test]
fn at_100_percent_a_click_5_px_from_a_segment_adds_an_anchor_and_6_px_does_nothing() {
    for dy in [2.0, 3.0, 4.0, 5.0] {
        assert!(adds_at(&mut app(Tool::AddAnchorPoint, 1.0, 1.0), dy), "{dy} px below adds");
        assert!(adds_at(&mut app(Tool::AddAnchorPoint, 1.0, 1.0), -dy), "{dy} px above adds");
    }
    assert!(!adds_at(&mut app(Tool::AddAnchorPoint, 1.0, 1.0), 6.0));
    assert!(!adds_at(&mut app(Tool::AddAnchorPoint, 1.0, 1.0), -6.0));
}

#[test]
fn the_reach_is_in_screen_pixels_so_at_200_percent_it_is_2_5_document_px() {
    assert!(adds_at(&mut app(Tool::AddAnchorPoint, 2.0, 1.0), 2.5));
    assert!(!adds_at(&mut app(Tool::AddAnchorPoint, 2.0, 1.0), 3.0));
    // And 10 document px at 50 %.
    assert!(adds_at(&mut app(Tool::AddAnchorPoint, 0.5, 1.0), 10.0));
    assert!(!adds_at(&mut app(Tool::AddAnchorPoint, 0.5, 1.0), 11.0));
}

/// Not measured: the reach stays in points, so 5 points are 6.25 px at display scale 1.25 and
/// 10 px at 2 (6 points were 7.5 and 12 px).
#[test]
fn the_reach_is_5_points_on_any_display_scale() {
    for (ppp, inside, outside) in [(1.25, 6.25, 6.5), (2.0, 10.0, 11.0)] {
        assert!(adds_at(&mut app(Tool::AddAnchorPoint, 1.0, ppp), inside), "{inside} px at display scale {ppp}");
        assert!(!adds_at(&mut app(Tool::AddAnchorPoint, 1.0, ppp), outside), "{outside} px at display scale {ppp}");
    }
}

/// Near an anchor nothing changes: within 6 points of one, Add Anchor Point hits the anchor and
/// adds nothing, as before.
#[test]
fn a_click_near_an_anchor_still_adds_nothing() {
    let mut app = app(Tool::AddAnchorPoint, 1.0, 1.0);
    let (before, n) = (saved(&app), steps(&app));
    for p in [[403.0, 303.0], [705.5, 300.0], [400.0, 305.5]] {
        click(&mut app, p);
        assert_eq!(saved(&app), before, "{p:?}");
    }
    assert_eq!(steps(&app), n);
}

#[test]
fn delete_anchor_point_still_reaches_an_anchor_within_6_points() {
    let mut app = app(Tool::DeleteAnchorPoint, 1.0, 1.0);
    let path = json!({"subpaths": [{"closed": false, "knots": [[400, 300], [550, 300], [700, 300]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    click(&mut app, [550.0, 305.5]);
    assert_eq!(saved(&app).subpaths[0].knots.len(), 2, "5.5 px from the middle anchor deletes it");
}

/// The Pen's Auto Add/Delete has its own tolerance (`pen_auto`) and keeps 6 points.
#[test]
fn the_pen_still_adds_an_anchor_within_6_points() {
    let mut app = app(Tool::Pen, 1.0, 1.0);
    app.ui.selected_path = None;
    let path = json!({"subpaths": [{"closed": false, "knots": [[400, 300], [700, 300]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    click(&mut app, [550.0, 305.5]);
    let work = app.session.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(work.subpaths[0].knots.len(), 3, "5.5 px below the segment adds with the Pen");
}

/// Direct Selection's segment hit (`hit_target`) keeps 6 points.
#[test]
fn direct_selection_still_hits_a_segment_within_6_points() {
    let app = app(Tool::DirectSelection, 1.0, 1.0);
    let hit = crate::direct_select::hit_target(&app, [550.0, 305.5], false);
    assert!(matches!(hit, Some((_, _, photocraft_vector::edit::Hit::Segment(..)))), "{hit:?}");
}
