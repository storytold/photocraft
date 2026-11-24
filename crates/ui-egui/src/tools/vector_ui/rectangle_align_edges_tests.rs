//! #13346: the Rectangle tool's Align Edges checkbox, at the options bar's right end after the
//! gear. On (the default) a drag's box snaps to whole pixels; off a rectangle keeps the drag's
//! fractional coordinates. It is greyed outside Shape mode, where the box still snaps.

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

const START: [f64; 2] = [10.3, 10.6];
const END: [f64; 2] = [40.7, 30.2];
const GEAR: &str = "Set additional shape and path options";

/// A 200 × 100 document with the Rectangle tool and its options set by `set`.
fn app(set: impl FnOnce(&mut ToolOptions)) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Rectangle;
    set(&mut app.ui.tool_options);
    app
}

/// A Rectangle drag the framework tool committed as the box from `from` to `to` with corner radius
/// `radius`: the exact fractions go to `finish_drag` as the tool hands them over.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], radius: f64) {
    let rect = craft::behavior::kurbo::Rect::new(from[0], from[1], to[0], to[1]);
    super::shapes::finish_drag(app, ShapeKind::Rectangle, &ShapeGeometry::Rect { rect, radii: [radius; 4] }, Mods::default());
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// The active shape layer's live rectangle `[x, y, w, h]`, its kind and its path's `[x0, y0, x1, y1]`.
fn drawn_layer(app: &mut PhotocraftApp) -> ([f64; 4], String, [f64; 4]) {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let path = match &st.doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Shape(sh) => sh.path.clone(),
        other => panic!("not a shape layer: {other:?}"),
    };
    let (x0, y0, x1, y1) = path.control_bounds().unwrap();
    let info = app.session.execute("shape.info", json!({"layer": id.0})).unwrap();
    let live = serde_json::from_value(info["live"]["rect"].clone()).unwrap_or_else(|_| panic!("a live rectangle: {}", info["live"]));
    (live, info["live"]["kind"].as_str().unwrap_or_default().to_owned(), [x0, y0, x1, y1])
}

fn assert_close(got: [f64; 4], want: [f64; 4]) {
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "{got:?} vs {want:?}");
    }
}

#[test]
fn the_drawn_shape_layer_follows_the_checkbox() {
    assert!(ToolOptions::default().align_edges, "on by default");
    for (align, live, bounds) in [(true, [10.0, 11.0, 30.0, 20.0], [10.0, 11.0, 40.0, 31.0]), (false, [10.3, 10.6, 30.4, 19.6], [10.3, 10.6, 40.7, 30.2])] {
        let mut app = app(|o| o.align_edges = align);
        let layers = layer_count(&app);
        drag(&mut app, START, END, 0.0);
        assert_eq!(layer_count(&app), layers + 1, "one new shape layer");
        let (got_live, kind, got_bounds) = drawn_layer(&mut app);
        assert_eq!(kind, "rect");
        assert_close(got_live, live);
        assert_close(got_bounds, bounds);
    }
}

#[test]
fn a_rounded_rectangle_follows_the_checkbox_too() {
    for (align, want) in [(true, [10.0, 11.0, 30.0, 20.0]), (false, [10.3, 10.6, 30.4, 19.6])] {
        let mut app = app(|o| o.align_edges = align);
        drag(&mut app, START, END, 4.0);
        let (live, _, _) = drawn_layer(&mut app);
        assert_close(live, want);
        let st = app.session.active().unwrap();
        let radii = match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            photocraft_doc::LayerContent::Shape(sh) => match sh.live {
                Some(photocraft_doc::LiveShape::Rect { radii, .. }) => radii,
                ref other => panic!("a live rectangle, got {other:?}"),
            },
            _ => panic!("a shape layer"),
        };
        assert_eq!(radii, [4.0; 4], "the corners stay rounded");
    }
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_and_pixels_mode() {
    let mut app = app(|o| o.align_edges = false);
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        app.ui.tool_options.shape_mode = mode;
        assert!(super::shapes::snaps_to_pixels(&app, ShapeKind::Rectangle), "{mode:?}");
    }
    app.ui.tool_options.shape_mode = VectorMode::Path;
    drag(&mut app, START, END, 0.0);
    let (x0, y0, x1, y1) = app.session.active().unwrap().doc.work_path.as_ref().unwrap().control_bounds().unwrap();
    assert_close([x0, y0, x1, y1], [10.0, 11.0, 40.0, 31.0]);
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    assert!(!super::shapes::snaps_to_pixels(&app, ShapeKind::Rectangle), "Shape mode with Align Edges off keeps fractions");
}

#[test]
fn a_sub_pixel_drag_still_draws_nothing() {
    let mut app = app(|o| o.align_edges = false);
    for end in [[10.9, 40.0], [40.0, 10.5]] {
        let layers = layer_count(&app);
        drag(&mut app, [10.0, 10.0], end, 0.0);
        assert_eq!(layer_count(&app), layers, "{end:?}");
    }
}

#[test]
fn reset_tool_turns_it_back_on() {
    let mut app = app(|o| o.align_edges = false);
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.align_edges);
}

#[test]
fn options_bar_shows_align_edges_at_its_right_end() {
    let mut h = harness(Tool::Rectangle);
    // The Radius field.
    let radius = h.query_all_by_label("Radius").map(|n| n.rect().right()).fold(f32::MIN, f32::max);
    let gear = h.get_by_label(GEAR).rect();
    let node = h.get_by_label("Align Edges");
    assert!(node.rect().left() > gear.right() && gear.left() > radius, "{:?} after {gear:?} after the radius ({radius})", node.rect());
    assert!((node.rect().center().y - gear.center().y).abs() < 4.0, "on the gear's row");
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
