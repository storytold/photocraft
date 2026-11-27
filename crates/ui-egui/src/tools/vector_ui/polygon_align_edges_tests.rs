//! #13326: the Polygon tool's Align Edges checkbox, at the options bar's right end after the path
//! arrangement. On (the default) a drag's box snaps to whole pixels; off a polygon keeps the drag's
//! fractional coordinates. It is greyed in Path mode (and in a pixels mode), where the box snaps.

use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::ShapeKind;
use craft::keymap::Mods;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::test_input::harness;
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The drag every test draws, from a fractional press to a fractional release.
const DRAG: [[f64; 2]; 2] = [[10.3, 10.6], [40.7, 30.2]];
/// Its box `[x, y, w, h]` as dragged, and snapped to whole pixels.
const FRACTIONAL: [f64; 4] = [10.3, 10.6, 30.4, 19.6];
const SNAPPED: [f64; 4] = [10.0, 11.0, 30.0, 20.0];

/// A 200 × 100 document after a Polygon drag the framework tool committed as the box from `from`
/// to `to`, with the tool options `set` gives: the exact fractions go to `finish_drag` as the tool
/// hands them over.
fn drawn(set: impl FnOnce(&mut ToolOptions), from: [f64; 2], to: [f64; 2]) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Polygon;
    set(&mut app.ui.tool_options);
    let rect = craft::behavior::kurbo::Rect::new(from[0], from[1], to[0], to[1]);
    super::shapes::finish_drag(&mut app, ShapeKind::Polygon, &ShapeGeometry::Polygon { rect, sides: 5, star_ratio: 1.0 }, Mods::default());
    app
}

/// The live polygon box `[x, y, w, h]` of the active layer after the drag, `None` when it made no layer.
fn polygon_box(app: &mut PhotocraftApp) -> Option<[f64; 4]> {
    let st = app.session.active().unwrap();
    if st.doc.layers.len() == 1 {
        return None;
    }
    let id = st.active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    assert_eq!((info["live"]["kind"].as_str(), info["live"]["sides"].as_u64()), (Some("polygon"), Some(5)));
    serde_json::from_value(info["live"]["rect"].clone()).ok()
}

/// The box a Polygon drag along `DRAG` with Align Edges `align` draws.
fn drag_box(align: bool) -> Option<[f64; 4]> {
    polygon_box(&mut drawn(|o| o.align_edges = align, DRAG[0], DRAG[1]))
}

fn assert_near(got: [f64; 4], want: [f64; 4], what: &str) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-9), "{what}: {got:?}, want {want:?}");
}

#[test]
fn on_by_default_the_box_snaps_and_off_it_keeps_the_drags_fractions() {
    assert!(ToolOptions::default().align_edges);
    assert_eq!(polygon_box(&mut drawn(|_| {}, DRAG[0], DRAG[1])), Some(SNAPPED));
    assert_near(drag_box(false).unwrap(), FRACTIONAL, "off");
}

#[test]
fn path_and_pixels_mode_keep_snapping_with_the_greyed_checkbox_off() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool_options.align_edges = false;
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        app.ui.tool_options.shape_mode = mode;
        assert!(super::shapes::snaps_to_pixels(&app, ShapeKind::Polygon), "{mode:?}");
    }
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    assert!(!super::shapes::snaps_to_pixels(&app, ShapeKind::Polygon), "Shape mode with Align Edges off keeps fractions");
}

#[test]
fn a_drag_under_a_pixel_still_draws_nothing() {
    for end in [[11.2, 30.0], [40.0, 11.5]] {
        let mut app = drawn(|o| o.align_edges = false, DRAG[0], end);
        assert_eq!(polygon_box(&mut app), None, "{end:?}");
    }
}

#[test]
fn a_path_mode_outline_snaps_with_the_checkbox_off() {
    let app = drawn(
        |o| {
            o.align_edges = false;
            o.shape_mode = VectorMode::Path;
        },
        DRAG[0],
        DRAG[1],
    );
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1, "no shape layer in Path mode");
    let (x0, y0, x1, y1) = doc.work_path.as_ref().unwrap().control_bounds().unwrap();
    // The pentagon in the snapped box [10, 11, 30, 20] (centre 25,21, radii 15 and 10): its top
    // vertex on the box's whole-pixel top edge, the others at 72° steps around the centre.
    let (sin72, cos36) = (72f64.to_radians().sin(), 36f64.to_radians().cos());
    assert_near([x0, y0, x1, y1], [25.0 - 15.0 * sin72, 11.0, 25.0 + 15.0 * sin72, 21.0 + 10.0 * cos36], "work path");
}

#[test]
fn reset_tool_turns_it_back_on_and_keeps_the_default_sides() {
    let mut app = drawn(|o| o.align_edges = false, DRAG[0], DRAG[1]);
    app.ui.tool_options.polygon_sides = 8;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.align_edges);
    assert_eq!(app.ui.tool_options.polygon_sides, 5);
}

#[test]
fn the_bar_ends_with_align_edges_enabled_only_in_shape_mode() {
    let mut h = harness(Tool::Polygon);
    // The Sides field, then the path menus, the last of which is Path arrangement.
    let sides = h.query_all_by_label("Sides").map(|n| n.rect().right()).fold(f32::MIN, f32::max);
    let arrange = h.get_by_label("Path arrangement").rect();
    let node = h.get_by_label("Align Edges");
    assert!(arrange.left() > sides && node.rect().left() > arrange.right(), "{:?} after {arrange:?} after Sides ({sides})", node.rect());
    assert!((node.rect().center().y - arrange.center().y).abs() < 4.0, "on the bar's row");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    node.click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "a click turns it off");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));

    for (mode, greyed) in [(VectorMode::Path, true), (VectorMode::Shape, false)] {
        h.state_mut().ui.tool_options.shape_mode = mode;
        h.run_steps(4);
        assert_eq!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), greyed, "{mode:?} mode");
    }
}
