//! #13470: the Custom Shape tool's Align Edges checkbox, at the options bar's right end after the
//! Shape picker. On (the default) a drag's box snaps to whole pixels; off a custom shape keeps the
//! drag's fractional coordinates. It is greyed outside Shape mode, where the box still snaps.
//!
//! The drag tool keeps the box on whole pixels or not by Align Edges (`ShapeDragOptions::whole_pixels`), so these
//! drags run through the canvas: the Triangle (#13370) and Rectangle (#13346) checkboxes act on a real drag too.

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

/// A 200 × 100 document shown at 1600 % with the pixel-grid snap off, `tool` picked (the Custom Shape tool with
/// its Heart, path bounds 0,0 to 100,92) and its options set by `set`.
fn app(tool: Tool, set: impl FnOnce(&mut ToolOptions)) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 16.0;
    app.ui.tool = tool;
    app.ui.presets_ui.custom_shape = "Heart".into();
    set(&mut app.ui.tool_options);
    app
}

/// A canvas drag from `from` to `to` with `mods` held: the press, two moves and the release.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

/// The active document's layer count.
fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// `[x0, y0, x1, y1]` of the active shape layer's path, or of the work path when `work`.
fn drawn_bounds(app: &PhotocraftApp, work: bool) -> [f64; 4] {
    let st = app.session.active().unwrap();
    let path = if work {
        st.doc.work_path.clone().unwrap()
    } else {
        match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            photocraft_doc::LayerContent::Shape(sh) => sh.path.clone(),
            other => panic!("not a shape layer: {other:?}"),
        }
    };
    let (x0, y0, x1, y1) = path.control_bounds().unwrap();
    [x0, y0, x1, y1]
}

/// Within a thousandth of a pixel: the pointer reaches the canvas through f32 screen points.
fn assert_close(got: [f64; 4], want: [f64; 4], what: &str) {
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-3, "{what}: {got:?} vs {want:?}");
    }
}

/// The bounds a `tool` drag from `START` to `END` draws as a new shape layer with Align Edges `align`.
fn drawn(tool: Tool, align: bool) -> [f64; 4] {
    let mut app = app(tool, |o| o.align_edges = align);
    let layers = layer_count(&app);
    drag(&mut app, START, END, Modifiers::NONE);
    assert_eq!(layer_count(&app), layers + 1, "{tool:?}: one new shape layer");
    drawn_bounds(&app, false)
}

#[test]
fn a_custom_shape_drag_snaps_with_align_edges_and_keeps_fractions_without() {
    assert!(ToolOptions::default().align_edges, "on by default");
    assert_eq!(drawn(Tool::CustomShape, true), [10.0, 11.0, 40.0, 31.0]);
    assert_close(drawn(Tool::CustomShape, false), [10.3, 10.6, 40.7, 30.2], "Align Edges off");
}

/// The Triangle's and the Rectangle's Align Edges reach a real drag as well: the drag tool used to round every box.
#[test]
fn the_triangle_and_rectangle_checkboxes_keep_a_real_drags_fractions() {
    for tool in [Tool::Triangle, Tool::Rectangle] {
        assert_eq!(drawn(tool, true), [10.0, 11.0, 40.0, 31.0], "{tool:?} with Align Edges");
        assert_close(drawn(tool, false), [10.3, 10.6, 40.7, 30.2], &format!("{tool:?} without Align Edges"));
    }
    // The other shape tools have no checkbox and always snap: their live box is on whole pixels.
    for tool in [Tool::EllipseShape, Tool::Polygon] {
        let mut app = app(tool, |o| o.align_edges = false);
        drag(&mut app, START, END, Modifiers::NONE);
        let id = app.session.active().unwrap().active_layer.unwrap().0;
        let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
        let r: [f64; 4] = serde_json::from_value(info["live"]["rect"].clone()).unwrap();
        assert!(r.iter().all(|v| v.fract() == 0.0), "{tool:?}: {r:?}");
    }
}

/// ⇧ keeps the shape's proportions (the Heart's 100 : 92, centred in the square of the drag's shorter side) and ⌥
/// draws from the centre, both before the box keeps its fractions.
#[test]
fn shift_and_alt_constrain_before_the_fractions_are_kept() {
    let heart = 19.6 * 0.92;
    let shift = [10.3, 10.6 + (19.6 - heart) / 2.0, 29.9, 10.6 + (19.6 + heart) / 2.0];
    for (mods, want) in [(Modifiers::SHIFT, shift), (Modifiers::ALT, [-20.1, -9.0, 40.7, 30.2])] {
        let mut app = app(Tool::CustomShape, |o| o.align_edges = false);
        drag(&mut app, START, END, mods);
        assert_close(drawn_bounds(&app, false), want, &format!("{mods:?}"));
    }
}

#[test]
fn a_shape_added_to_the_active_layer_keeps_its_fractions_too() {
    let mut app = app(Tool::CustomShape, |o| o.align_edges = false);
    drag(&mut app, [100.0, 50.0], [150.0, 96.0], Modifiers::NONE);
    let first = drawn_bounds(&app, false);
    app.ui.tool_options.shape_path_op = "combine".into();
    let layers = layer_count(&app);
    drag(&mut app, START, END, Modifiers::NONE);
    assert_eq!(layer_count(&app), layers, "joined the active layer");
    assert_close(drawn_bounds(&app, false), [10.3, 10.6, first[2], first[3]], "both components");
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_mode() {
    let mut app = app(Tool::CustomShape, |o| (o.align_edges, o.shape_mode) = (false, VectorMode::Path));
    assert!(super::shapes::snaps_to_pixels(&app, ShapeKind::Custom), "Path mode still snaps");
    drag(&mut app, START, END, Modifiers::NONE);
    assert_eq!(drawn_bounds(&app, true), [10.0, 11.0, 40.0, 31.0]);
}

#[test]
fn a_sub_pixel_drag_still_draws_nothing() {
    for end in [[10.9, 40.0], [40.0, 10.5]] {
        let mut app = app(Tool::CustomShape, |o| o.align_edges = false);
        let layers = layer_count(&app);
        drag(&mut app, [10.0, 10.0], end, Modifiers::NONE);
        assert_eq!(layer_count(&app), layers, "to {end:?}");
    }
}

#[test]
fn reset_tool_turns_it_back_on_and_keeps_the_shape() {
    let mut app = app(Tool::CustomShape, |o| o.align_edges = false);
    app.ui.presets_ui.custom_shape = "Ring".into();
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.align_edges);
    assert_eq!(app.ui.presets_ui.shape(), "Ring", "Reset Tool keeps the chosen shape");
}

#[test]
fn options_bar_shows_align_edges_after_the_shape_picker() {
    let mut h = harness(Tool::CustomShape);
    // The Shape picker is the right-most part named "Shape" (the mode menu's Shape comes first).
    let picker =
        |h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>| h.query_all_by_label("Shape").map(|n| n.rect()).max_by(|a, b| a.right().total_cmp(&b.right()));
    let picker = picker(&h).expect("the Shape picker");
    let shapes = h.query_all_by_label("Shape").count();
    let node = h.get_by_label("Align Edges");
    assert!(node.rect().left() > picker.right(), "{:?} after {picker:?}", node.rect());
    assert!((node.rect().center().y - picker.center().y).abs() < 4.0, "on the picker's row");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));

    h.state_mut().ui.tool_options.shape_mode = VectorMode::Path;
    h.run_steps(4);
    assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in Path mode");
    assert_eq!(h.query_all_by_label("Shape").count(), shapes, "the Shape picker stays in Path mode");
}
