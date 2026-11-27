//! #16869: the Star tool's Align Edges checkbox, at the options bar's right end as on the other
//! shape tools. On (the default) a star's box snaps to whole pixels; off it keeps the drag's
//! fractional coordinates. It is greyed in Path and Pixels mode, where the box still snaps.

use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::ShapeKind;
use craft::keymap::Mods;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::super::test_input::harness;
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The box `[x, y, w, h]` of the drag every test draws, from a fractional press to a fractional
/// release, as dragged and snapped to whole pixels.
const FRACTIONAL: [f64; 4] = [10.3, 10.6, 30.4, 19.6];
const SNAPPED: [f64; 4] = [10.0, 11.0, 30.0, 20.0];

/// The live box of the star a Star drag over `FRACTIONAL` draws with the tool options `set` gives:
/// the exact fractions go to `finish_drag` as the framework tool hands them over.
fn star_box(set: impl FnOnce(&mut ToolOptions)) -> [f64; 4] {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Star;
    set(&mut app.ui.tool_options);
    let [x, y, w, h] = FRACTIONAL;
    let rect = craft::behavior::kurbo::Rect::new(x, y, x + w, y + h);
    super::finish_drag(&mut app, ShapeKind::Star, &ShapeGeometry::Polygon { rect, sides: 5, star_ratio: 0.5 }, Mods::default());
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["kind"].as_str(), Some("star"));
    serde_json::from_value(info["live"]["rect"].clone()).unwrap()
}

#[test]
fn on_by_default_a_star_snaps_and_off_it_keeps_the_drags_fractions() {
    assert!(ToolOptions::default().align_edges);
    assert_eq!(star_box(|_| {}), SNAPPED);
    let off = star_box(|o| o.align_edges = false);
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
