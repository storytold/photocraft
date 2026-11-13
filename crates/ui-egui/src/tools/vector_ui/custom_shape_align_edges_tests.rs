//! #13470: the Custom Shape tool's Align Edges checkbox, at the options bar's right end after the
//! Shape picker. On (the default) a drag's box snaps to whole pixels; off a custom shape keeps the
//! drag's fractional coordinates. It is greyed outside Shape mode, where the box still snaps.

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

/// A 200 × 100 document with the Custom Shape tool and its Heart (path bounds 0,0 to 100,92).
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::CustomShape;
    app.ui.presets_ui.custom_shape = "Heart".into();
    app
}

/// The `[x, y, w, h]` box a Custom Shape drag from `START` to `END` draws, or None.
fn drag_box(app: &PhotocraftApp, mods: Modifiers) -> Option<[f64; 4]> {
    let g = shape_geometry(&app.ui.tool_options, Tool::CustomShape, START, END, mods)?;
    serde_json::from_value(g["rect"].clone()).ok()
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
fn the_drawn_shape_layer_follows_the_checkbox() {
    for (align, want) in [(true, [10.0, 11.0, 40.0, 31.0]), (false, [10.3, 10.6, 40.7, 30.2])] {
        let mut app = app();
        app.ui.tool_options.align_edges = align;
        let layers = app.session.active().unwrap().doc.layers.len();
        finish_shape(&mut app, Tool::CustomShape, START, END, Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "one new shape layer");
        assert_close(drawn_bounds(&app, false), want);
    }
}

#[test]
fn a_shape_added_to_the_active_layer_keeps_its_fractions_too() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    finish_shape(&mut app, Tool::CustomShape, [100.0, 50.0], [150.0, 96.0], Modifiers::NONE);
    app.ui.tool_options.shape_path_op = "combine".into();
    let layers = app.session.active().unwrap().doc.layers.len();
    finish_shape(&mut app, Tool::CustomShape, START, END, Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "joined the active layer");
    assert_close(drawn_bounds(&app, false), [10.3, 10.6, 150.0, 96.0]);
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_mode() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    app.ui.tool_options.shape_mode = "path".into();
    assert_eq!(drag_box(&app, Modifiers::NONE), Some([10.0, 11.0, 30.0, 20.0]));
    finish_shape(&mut app, Tool::CustomShape, START, END, Modifiers::NONE);
    assert_close(drawn_bounds(&app, true), [10.0, 11.0, 40.0, 31.0]);
}

#[test]
fn a_sub_pixel_drag_still_draws_nothing() {
    let mut app = app();
    app.ui.tool_options.align_edges = false;
    for end in [[10.9, 40.0], [40.0, 10.5]] {
        assert!(shape_geometry(&app.ui.tool_options, Tool::CustomShape, [10.0, 10.0], end, Modifiers::NONE).is_none());
        let layers = app.session.active().unwrap().doc.layers.len();
        finish_shape(&mut app, Tool::CustomShape, [10.0, 10.0], end, Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
    }
}

#[test]
fn reset_tool_turns_it_back_on_and_keeps_the_shape() {
    let mut app = app();
    app.ui.presets_ui.custom_shape = "Ring".into();
    app.ui.tool_options.align_edges = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.align_edges);
    assert_eq!(app.ui.presets_ui.shape(), "Ring", "Reset Tool keeps the chosen shape");
}

#[test]
fn options_bar_shows_align_edges_after_the_shape_picker() {
    let mut h = harness(Tool::CustomShape);
    let picker = h.get_by_label("Shape:").rect();
    let node = h.get_by_label("Align Edges");
    // The picker's button is 36 px wide, right of its caption.
    assert!(node.rect().left() > picker.right() + 36.0, "{:?} after {picker:?}", node.rect());
    assert!((node.rect().center().y - picker.center().y).abs() < 4.0, "on the picker's row");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));

    h.state_mut().ui.tool_options.shape_mode = "path".into();
    h.run_steps(4);
    assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in Path mode");
    assert!(h.query_by_label("Shape:").is_some(), "the Shape picker stays in Path mode");
}
