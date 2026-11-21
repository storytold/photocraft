//! #13237: the Ellipse tool's options bar has Align Edges, on by default and saved with the tool
//! options. It is enabled in Shape and Path mode and greyed in Pixels mode. On, the drawn ellipse's
//! box snaps to whole pixels; off, it keeps the drag's fractional coordinates.

use craft::behavior::kurbo::Rect;
use craft::draw::geometry::shape::ShapeGeometry;
use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::ShapeKind;
use craft::keymap::Mods;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::test_input::harness;
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The box an Ellipse drag that the framework hands over as `rect` (x0, y0, x1, y1) commits, in
/// `mode` with Align Edges `on`, read back from the new layer's live ellipse.
fn committed_box(on: bool, rect: [f64; 4]) -> [f64; 4] {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::EllipseShape;
    app.ui.tool_options.align_edges = on;
    let geometry = ShapeGeometry::Ellipse { rect: Rect::new(rect[0], rect[1], rect[2], rect[3]) };
    super::shapes::finish_drag(&mut app, ShapeKind::Ellipse, &geometry, Mods::NONE);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    serde_json::from_value(info["live"]["rect"].clone()).unwrap()
}

#[test]
fn on_by_default_and_saved() {
    assert!(ToolOptions::default().align_edges);
    let off = ToolOptions { align_edges: false, ..Default::default() };
    let back: ToolOptions = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
    assert!(!back.align_edges);
}

#[test]
fn on_snaps_the_box_and_off_keeps_the_fractions() {
    let drag = [12.3, 8.6, 57.7, 41.2];
    assert_eq!(committed_box(true, drag), [12.0, 9.0, 45.0, 33.0], "on: whole pixels");
    let off = committed_box(false, drag);
    let want = [12.3, 8.6, 45.4, 32.6];
    assert!(off.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "off: the drag's fractions, {off:?}");
}

#[test]
fn it_applies_in_shape_and_path_mode_not_pixels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool_options.align_edges = false;
    for (mode, snaps) in [(VectorMode::Shape, false), (VectorMode::Path, false), (VectorMode::Pixels, true)] {
        app.ui.tool_options.shape_mode = mode;
        assert_eq!(super::shapes::snaps_to_pixels(&app, ShapeKind::Ellipse), snaps, "{mode:?}");
    }
}

#[test]
fn the_options_bar_shows_it_enabled_in_shape_and_path_mode_and_greyed_in_pixels_mode() {
    let mut h = harness(Tool::EllipseShape);
    let node = h.get_by_label("Align Edges");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    for (mode, disabled) in [(VectorMode::Path, false), (VectorMode::Pixels, true)] {
        h.state_mut().ui.tool_options.shape_mode = mode;
        h.run_steps(4);
        assert_eq!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), disabled, "{mode:?}");
    }
}
