//! #13346: the Rectangle tool's Align Edges checkbox, at the options bar's right end after the
//! gear. On (the default) a drag's box snaps to whole pixels; off a rectangle keeps the drag's
//! fractional coordinates. It is greyed outside Shape mode, where the box still snaps.

use egui::Modifiers;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::add_anchor_options_bar_tests::harness;
use super::{finish_shape, shape_geometry};
use crate::PhotocraftApp;
use crate::state::Tool;

const START: [f64; 2] = [10.3, 10.6];
const END: [f64; 2] = [40.7, 30.2];
const GEAR: &str = "Set additional shape and path options";

/// A 200 × 100 document with the Rectangle tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app
}

/// The `[x, y, w, h]` box a Rectangle drag from `START` to `END` draws, or None.
fn drag_box(app: &PhotocraftApp, mods: Modifiers) -> Option<[f64; 4]> {
    let g = shape_geometry(&app.ui.tool_options, Tool::Rectangle, START, END, mods)?;
    serde_json::from_value(g["rect"].clone()).ok()
}

/// The active shape layer's live rectangle `[x, y, w, h]` and its path's `[x0, y0, x1, y1]`.
fn drawn_layer(app: &mut PhotocraftApp) -> ([f64; 4], [f64; 4]) {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let path = match &st.doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Shape(sh) => sh.path.clone(),
        other => panic!("not a shape layer: {other:?}"),
    };
    let (x0, y0, x1, y1) = path.control_bounds().unwrap();
    let info = app.session.execute("shape.info", json!({"layer": id.0})).unwrap();
    let live = serde_json::from_value(info["live"]["rect"].clone()).unwrap_or_else(|_| panic!("a live rectangle: {}", info["live"]));
    (live, [x0, y0, x1, y1])
}

/// `[x0, y0, x1, y1]` of the work path.
fn work_path_bounds(app: &PhotocraftApp) -> [f64; 4] {
    let (x0, y0, x1, y1) = app.session.active().unwrap().doc.work_path.as_ref().unwrap().control_bounds().unwrap();
    [x0, y0, x1, y1]
}

fn assert_close(got: [f64; 4], want: [f64; 4]) {
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "{got:?} vs {want:?}");
    }
}

#[test]
fn the_box_snaps_with_align_edges_and_keeps_fractions_without() {
    let mut app = app();
    assert!(app.ui.tool_options.align_edges, "on by default");
    assert_eq!(drag_box(&app, Modifiers::NONE), Some([10.0, 11.0, 30.0, 20.0]));
    app.ui.tool_options.align_edges = false;
    assert_close(drag_box(&app, Modifiers::NONE).unwrap(), [10.3, 10.6, 30.4, 19.6]);
    // Shift squares on the drag's shorter side and Alt draws from the centre, both before the box
    // snaps or not.
    assert_close(drag_box(&app, Modifiers::SHIFT).unwrap(), [10.3, 10.6, 19.6, 19.6]);
    assert_close(drag_box(&app, Modifiers::ALT).unwrap(), [-20.1, -9.0, 60.8, 39.2]);
    app.ui.tool_options.align_edges = true;
    assert_eq!(drag_box(&app, Modifiers::SHIFT), Some([10.0, 11.0, 20.0, 20.0]));
    assert_eq!(drag_box(&app, Modifiers::ALT), Some([-20.0, -9.0, 61.0, 39.0]));
}

#[test]
fn a_rounded_rectangle_and_the_gears_fixed_size_follow_the_checkbox_too() {
    let mut app = app();
    app.ui.tool_options.corner_radius = 4.0;
    app.ui.tool_options.align_edges = false;
    let g = shape_geometry(&app.ui.tool_options, Tool::Rectangle, START, END, Modifiers::NONE).unwrap();
    assert_eq!(g["kind"], "roundedRect");
    assert_close(serde_json::from_value(g["rect"].clone()).unwrap(), [10.3, 10.6, 30.4, 19.6]);
    // Fixed Size puts W × H at the release point.
    app.ui.tool_options.corner_radius = 0.0;
    app.ui.tool_options.rect_constraint = "fixedSize".into();
    (app.ui.tool_options.rect_fixed_width, app.ui.tool_options.rect_fixed_height) = (20.5, 10.25);
    assert_close(drag_box(&app, Modifiers::NONE).unwrap(), [40.7, 30.2, 20.5, 10.25]);
    app.ui.tool_options.align_edges = true;
    assert_eq!(drag_box(&app, Modifiers::NONE), Some([41.0, 30.0, 21.0, 10.0]));
}

#[test]
fn the_drawn_shape_layer_follows_the_checkbox() {
    for (align, live, bounds) in [(true, [10.0, 11.0, 30.0, 20.0], [10.0, 11.0, 40.0, 31.0]), (false, [10.3, 10.6, 30.4, 19.6], [10.3, 10.6, 40.7, 30.2])] {
        let mut app = app();
        app.ui.tool_options.align_edges = align;
        let layers = app.session.active().unwrap().doc.layers.len();
        finish_shape(&mut app, Tool::Rectangle, START, END, Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "one new shape layer");
        let (got_live, got_bounds) = drawn_layer(&mut app);
        assert_close(got_live, live);
        assert_close(got_bounds, bounds);
    }
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_and_pixels_mode() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    for mode in ["path", "pixels"] {
        app.ui.tool_options.shape_mode = mode.into();
        assert_eq!(drag_box(&app, Modifiers::NONE), Some([10.0, 11.0, 30.0, 20.0]), "{mode}");
    }
    app.ui.tool_options.shape_mode = "path".into();
    finish_shape(&mut app, Tool::Rectangle, START, END, Modifiers::NONE);
    assert_close(work_path_bounds(&app), [10.0, 11.0, 40.0, 31.0]);
}

#[test]
fn a_sub_pixel_drag_still_draws_nothing() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    for end in [[10.9, 40.0], [40.0, 10.5]] {
        assert!(shape_geometry(&app.ui.tool_options, Tool::Rectangle, [10.0, 10.0], end, Modifiers::NONE).is_none());
        let layers = app.session.active().unwrap().doc.layers.len();
        finish_shape(&mut app, Tool::Rectangle, [10.0, 10.0], end, Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
    }
}

#[test]
fn reset_tool_turns_it_back_on() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.align_edges);
}

#[test]
fn options_bar_shows_align_edges_at_its_right_end() {
    let mut h = harness(Tool::Rectangle);
    // The Radius caption and its field.
    let radius = h.query_all_by_label("Radius:").map(|n| n.rect().right()).fold(f32::MIN, f32::max);
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

    for mode in ["path", "pixels"] {
        h.state_mut().ui.tool_options.shape_mode = mode.into();
        h.run_steps(4);
        assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in {mode} mode");
    }
}
