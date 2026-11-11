//! #12767: with Delete Anchor Point, a drag that starts on a segment moves it as a temporary
//! Direct Selection does (history step **Drag Segment**); an anchor click still deletes.

use egui::Modifiers;
use serde_json::{Value, json};

use super::delete_anchor_targeted_path_tests::{click, setup, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The item's test document: a saved path "Path 1", selected in the Paths panel, with a closed
/// corner square, an open straight line and an open S-curve.
fn document(tool: Tool) -> PhotocraftApp {
    let (mut app, _) = setup(tool);
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [{"anchor": [400, 450], "out": [500, 350]}, {"anchor": [700, 450], "in": [600, 550]}]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn knots(app: &mut PhotocraftApp) -> Vec<Value> {
    app.run("path.info", json!({"name": "Path 1"})).unwrap()["path"]["subpaths"][0]["knots"].as_array().unwrap().clone()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

fn anchors(app: &mut PhotocraftApp) -> Vec<[f64; 2]> {
    let anchor = |k: &Value| {
        let a = k.get("anchor").unwrap_or(k);
        [a[0].as_f64().unwrap(), a[1].as_f64().unwrap()]
    };
    knots(app).iter().map(anchor).collect()
}

#[test]
fn dragging_the_bottom_edge_moves_both_its_anchors_in_one_drag_segment_step() {
    let mut app = document(Tool::DeleteAnchorPoint);
    let n = steps(&app);
    drag(&mut app, [200.0, 500.0], [200.0, 540.0]);
    assert_eq!(anchors(&mut app), vec![[100.0, 300.0], [300.0, 300.0], [300.0, 540.0], [100.0, 540.0]]);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Drag Segment"));
}

#[test]
fn a_click_on_a_segment_without_movement_does_nothing() {
    let mut app = document(Tool::DeleteAnchorPoint);
    let (before, n) = (knots(&mut app), steps(&app));
    click(&mut app, [200.0, 500.0]);
    assert_eq!(knots(&mut app), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn a_click_on_an_anchor_still_deletes_it_in_one_step() {
    let mut app = document(Tool::DeleteAnchorPoint);
    let n = steps(&app);
    click(&mut app, [300.0, 500.0]);
    assert_eq!(knots(&mut app).len(), 3);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn add_anchor_point_clicks_keep_adding_an_anchor() {
    let mut app = document(Tool::AddAnchorPoint);
    let n = steps(&app);
    click(&mut app, [200.0, 500.0]);
    assert_eq!(knots(&mut app).len(), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}
