//! #13252: the Line tool's Align Edges checkbox, after Weight, on by default and saved with the tool options. On, a
//! drag snaps as before (the ends on pixel centres, a 1 px horizontal or vertical line on one pixel row or column); off,
//! the line keeps the drag's fractional ends and weight. The checkbox is enabled in Shape mode and greyed in Path and
//! Pixels mode, and the Line tool's setting is its own.

use craft::draw::ui::VectorMode;
use egui::Modifiers;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::test_input::harness;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

/// The live line a Line-tool drag from `start` to `end` draws (`from`, `to`, `weight`), with Align Edges `on`. The
/// pixel-grid snap of vector tools is off, so only Align Edges rounds.
fn line(on: bool, weight: f32, start: [f64; 2], end: [f64; 2]) -> Value {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Line;
    app.ui.tool_options.line_weight = weight;
    app.ui.tool_options.line_align_edges = on;
    let none = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: (start[0] + end[0]) / 2.0, y: (start[1] + end[1]) / 2.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, none);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    info["live"].clone()
}

fn ends(live: &Value) -> ([f64; 2], [f64; 2]) {
    (serde_json::from_value(live["from"].clone()).unwrap(), serde_json::from_value(live["to"].clone()).unwrap())
}

#[test]
fn line_align_edges_is_on_by_default_and_saved() {
    assert!(ToolOptions::default().line_align_edges);
    let off = ToolOptions { line_align_edges: false, ..Default::default() };
    let back: ToolOptions = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
    assert!(!back.line_align_edges, "a saved off survives a round trip");
    let empty: ToolOptions = serde_json::from_str("{}").unwrap();
    assert!(empty.line_align_edges, "an older file without the field gets the default");
    let mut reset = ToolOptions { line_align_edges: false, ..Default::default() };
    crate::tool_reset::reset_options(&mut reset, Tool::Line);
    assert!(reset.line_align_edges, "Reset Tool turns it back on");
}

#[test]
fn on_a_horizontal_and_a_vertical_line_land_on_pixel_centres() {
    assert_eq!(ends(&line(true, 1.0, [400.0, 300.0], [600.0, 300.0])), ([400.0, 299.5], [600.0, 299.5]));
    assert_eq!(ends(&line(true, 1.0, [400.0, 300.0], [400.0, 500.0])), ([399.5, 300.0], [399.5, 500.0]));
}

#[test]
fn off_a_line_keeps_its_fractional_ends_and_weight() {
    let (start, end) = ([100.3, 50.6], [300.7, 120.2]);
    let live = line(false, 1.5, start, end);
    let (from, to) = ends(&live);
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
    assert!(close(from, start) && close(to, end), "the drag's points: {from:?} {to:?}");
    assert_eq!(live["weight"].as_f64(), Some(1.5), "the weight is not rounded");
    // On, the same drag rounds the ends.
    let (from, _) = ends(&line(true, 1.5, start, end));
    assert!(from.iter().all(|v| v.fract() == 0.0), "{from:?}");
}

#[test]
fn the_checkbox_is_the_line_tools_own_and_greyed_outside_shape_mode() {
    let mut h = harness(Tool::Line);
    let node = h.get_by_label("Align Edges");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.line_align_edges, "clicking toggles the Line tool's option");
    assert!(h.state().ui.tool_options.align_edges, "the Triangle's stays as it was");
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        h.state_mut().ui.tool_options.shape_mode = mode;
        h.run_steps(4);
        assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in {mode:?} mode");
    }
}
