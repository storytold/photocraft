//! #16869: the Star tool's Align Edges checkbox, at the options bar's right end as on the other
//! shape tools. On (the default) a star's box snaps to whole pixels; off it keeps the drag's
//! fractional coordinates. It is greyed in Path and Pixels mode, where the box still snaps.

use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::ShapeKind;
use egui::Modifiers;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::super::test_input::harness;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

const START: [f64; 2] = [10.3, 10.6];
const END: [f64; 2] = [40.7, 30.2];
/// The box `[x, y, w, h]` from `START` to `END` as dragged.
const FRACTIONAL: [f64; 4] = [10.3, 10.6, 30.4, 19.6];

/// The live star box a Star drag from `START` to `END` draws with the options `set` gives, `None`
/// when it made no layer. The pixel-grid snap of vector tools is off, so only Align Edges rounds.
fn star_box(set: impl FnOnce(&mut ToolOptions)) -> Option<[f64; 4]> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 16.0;
    app.ui.tool = Tool::Star;
    set(&mut app.ui.tool_options);
    let before = app.session.active().unwrap().doc.layers.len();
    let none = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: START[0], y: START[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: (START[0] + END[0]) / 2.0, y: (START[1] + END[1]) / 2.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: END[0], y: END[1], pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: END[0], y: END[1] }, none);
    let st = app.session.active().unwrap();
    if st.doc.layers.len() == before {
        return None;
    }
    let id = st.active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["live"]["kind"].as_str(), Some("star"));
    serde_json::from_value(info["live"]["rect"].clone()).ok()
}

#[test]
fn on_by_default_a_star_snaps_and_off_it_keeps_the_drags_fractions() {
    assert!(ToolOptions::default().align_edges);
    let on = star_box(|_| {}).unwrap();
    assert!(on.iter().all(|v| v.fract() == 0.0), "on: {on:?}");
    let off = star_box(|o| o.align_edges = false).unwrap();
    assert!(off.iter().zip(FRACTIONAL).all(|(g, w)| (g - w).abs() < 1e-9), "off: {off:?}, want {FRACTIONAL:?}");
}

#[test]
fn path_and_pixels_mode_keep_snapping_with_the_greyed_checkbox_off() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool_options.align_edges = false;
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        app.ui.tool_options.shape_mode = mode;
        assert!(super::snaps_to_pixels(&app, ShapeKind::Star), "{mode:?}");
    }
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    assert!(!super::snaps_to_pixels(&app, ShapeKind::Star), "Shape mode with Align Edges off keeps fractions");
}

#[test]
fn the_bar_ends_with_align_edges_on_and_greyed_outside_shape_mode() {
    let mut h = harness(Tool::Star);
    let arrange = h.get_by_label("Path arrangement").rect();
    let node = h.get_by_label("Align Edges");
    assert!(node.rect().left() > arrange.right(), "{:?} after {arrange:?}", node.rect());
    assert!((node.rect().center().y - arrange.center().y).abs() < 4.0, "on the bar's row");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    node.click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "a click turns it off");

    for (mode, greyed) in [(VectorMode::Path, true), (VectorMode::Pixels, true), (VectorMode::Shape, false)] {
        h.state_mut().ui.tool_options.shape_mode = mode;
        h.run_steps(4);
        assert_eq!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), greyed, "{mode:?} mode");
    }
}
