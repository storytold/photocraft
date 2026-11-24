//! #13370: the Triangle tool's Align Edges checkbox, every shape tool's one checkbox as in the
//! reference application. On (the default) the drawn box snaps to whole pixels; off it keeps the
//! drag's fractional coordinates. It is greyed outside Shape mode.

use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::ShapeKind;
use egui::Modifiers;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::test_input::harness;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

const START: [f64; 2] = [10.3, 10.6];
const END: [f64; 2] = [40.7, 30.2];

/// The box a `tool` drag from `START` to `END` draws, `None` when nothing is drawn. The pixel-grid
/// snap of vector tools is off, so only Align Edges rounds.
fn rect(set: impl FnOnce(&mut ToolOptions), tool: Tool, start: [f64; 2], end: [f64; 2]) -> Option<[f64; 4]> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 16.0;
    app.ui.tool = tool;
    set(&mut app.ui.tool_options);
    let before = app.session.active().unwrap().doc.layers.len();
    let none = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: (start[0] + end[0]) / 2.0, y: (start[1] + end[1]) / 2.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, none);
    let st = app.session.active().unwrap();
    if st.doc.layers.len() == before {
        return None;
    }
    let id = st.active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    serde_json::from_value(info["live"]["rect"].clone()).ok()
}

#[test]
fn align_edges_is_on_by_default_and_saved() {
    assert!(ToolOptions::default().align_edges);
    let off = ToolOptions { align_edges: false, ..Default::default() };
    let back: ToolOptions = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
    assert!(!back.align_edges, "a saved off survives a round trip");
    let empty: ToolOptions = serde_json::from_str("{}").unwrap();
    assert!(empty.align_edges, "an older file without the field gets the default");
}

#[test]
fn other_shape_tools_always_snap() {
    for tool in [Tool::EllipseShape, Tool::Star] {
        let r = rect(|o| o.align_edges = false, tool, START, END).unwrap();
        assert!(r.iter().all(|v| v.fract() == 0.0), "{tool:?}: {r:?}");
    }
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_mode() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool_options.align_edges = false;
    app.ui.tool_options.shape_mode = VectorMode::Path;
    assert!(super::shapes::snaps_to_pixels(&app, ShapeKind::Triangle), "Path mode still snaps");
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    assert!(!super::shapes::snaps_to_pixels(&app, ShapeKind::Triangle), "Shape mode with Align Edges off keeps fractions");
}

#[test]
fn options_bar_shows_align_edges_for_the_triangle() {
    let mut h = harness(Tool::Triangle);
    let node = h.get_by_label("Align Edges");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));

    h.state_mut().ui.tool_options.shape_mode = VectorMode::Path;
    h.run_steps(4);
    assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in Path mode");
}

/// The reference application has one Align Edges checkbox on every shape tool's and the Pen's
/// options bar, on after Reset Tool: here too, exactly one, on, enabled in Shape mode.
#[test]
fn every_shape_tool_and_the_pen_show_one_align_edges_checkbox() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape, Tool::Pen] {
        let mut h = harness(tool);
        h.state_mut().ui.tool_options.vector_mode = VectorMode::Shape;
        h.run_steps(3);
        assert_eq!(h.query_all_by_label("Align Edges").count(), 1, "{tool:?}");
        let node = h.get_by_label("Align Edges");
        assert!(!node.accesskit_node().is_disabled(), "{tool:?}: enabled in Shape mode");
        assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True), "{tool:?}");
    }
    let opts = ToolOptions { align_edges: false, ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool_options = opts;
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Line, Tool::Pen] {
        app.ui.tool = tool;
        app.ui.tool_options.align_edges = false;
        crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
        assert!(app.ui.tool_options.align_edges, "{tool:?}: Reset Tool turns Align Edges back on");
    }
}
