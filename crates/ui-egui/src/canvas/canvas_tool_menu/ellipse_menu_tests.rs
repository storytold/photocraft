//! #13232: the Ellipse tool's Shape-mode canvas right-click opens the shape menu, in the reference
//! application's order: on a pixel layer with no path and on an ellipse shape layer, inside the
//! ellipse or off it. Its rows run what the menu bar or the Pen's menu runs. Path and Pixels mode,
//! and other layer kinds, open no menu, as before.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::line_menu_tests::{greyed, menu, shown};
use super::tests::{app, harness, right_click, screen_of};
use crate::PhotocraftApp;
use crate::state::Tool;

const PIXEL_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

const SHAPE_LAYER_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform Path ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

/// The Ellipse tool picked in `mode`.
fn ellipse_tool(mut app: PhotocraftApp, mode: &str) -> PhotocraftApp {
    app.ui.tool = Tool::EllipseShape;
    app.ui.tool_options.shape_mode = mode.into();
    app
}

/// A pixel layer filled red above the Background, active, and no path.
fn pixel_layer() -> PhotocraftApp {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    ellipse_tool(app, "shape")
}

/// An ellipse the Ellipse tool drew over the document's middle, with a stroke: its shape layer is
/// active.
fn ellipse_layer() -> PhotocraftApp {
    let mut app = ellipse_tool(app(), "shape");
    // The measured ellipse had a stroke; without one Copy Complete Stroke greys, as in the Line's menu.
    app.ui.tool_options.stroke_width = 2.0;
    crate::vector_ui::finish_shape(&mut app, Tool::EllipseShape, [8.0, 8.0], [24.0, 24.0], Modifiers::NONE);
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(layer.content.kind_name(), "Shape");
    app
}

/// Right-click document point `doc`: the open menu's entries as `ui.inspect` lists them.
fn entries_at(h: &mut Harness<'static, PhotocraftApp>, doc: [f32; 2]) -> Vec<Value> {
    let p = screen_of(h.state(), doc);
    right_click(h, p, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!("EllipseShape"));
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// Pick the row `label` of the open menu.
fn pick(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "{label} closes the menu");
}

#[test]
fn on_a_pixel_layer_the_menu_lists_the_reference_rows() {
    let mut h = harness(pixel_layer());
    let entries = entries_at(&mut h, [16.0, 16.0]);
    assert_eq!(shown(&entries), menu(PIXEL_MENU));
    // The measured greyed rows, and two the reference keeps live that need a shape layer here:
    // Copy Fill and Paste Fill refuse a pixel layer, so their rows grey out as in the Line's menu.
    assert_eq!(
        greyed(&entries),
        [
            "Define Custom Shape…",
            "Make Selection…",
            "Fill Path…",
            "Stroke Path…",
            "Rasterize Layer",
            "Rasterize Layer Style",
            "Rasterize Vector Mask",
            "Unite Shapes",
            "Subtract Front Shape",
            "Unite Shapes at Overlap",
            "Subtract Shapes at Overlap",
            "Copy Fill",
            "Copy Complete Stroke",
            "Paste Fill",
            "Paste Complete Stroke",
        ]
    );
    // With no path, Free Transform is the menu bar's: a box around the layer.
    pick(&mut h, "Free Transform");
    assert!(h.state().ui.transform.is_some(), "Free Transform opens the box");
}

#[test]
fn on_an_ellipse_layer_the_menu_is_the_same_inside_and_off_the_ellipse() {
    let mut h = harness(ellipse_layer());
    let off = entries_at(&mut h, [2.0, 2.0]);
    assert_eq!(shown(&off), menu(SHAPE_LAYER_MENU));
    let before = h.state().session.journal.len();
    pick(&mut h, "Copy Fill");
    assert!(h.state().session.journal[before..].iter().any(|(id, _)| id == "path.style.copyFill"));
    // With a fill copied, as in the measured session, the greyed rows are the reference's.
    let off = entries_at(&mut h, [2.0, 2.0]);
    assert_eq!(
        greyed(&off),
        [
            "Fill Path…",
            "Stroke Path…",
            "Rasterize Layer Style",
            "Rasterize Vector Mask",
            "Unite Shapes",
            "Subtract Front Shape",
            "Unite Shapes at Overlap",
            "Subtract Shapes at Overlap",
            "Paste Complete Stroke",
        ]
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(entries_at(&mut h, [16.0, 16.0]), off, "inside the ellipse");
}

#[test]
fn the_rows_run_the_menu_bar_and_pen_menu_commands() {
    let mut h = harness(ellipse_layer());
    entries_at(&mut h, [2.0, 2.0]);
    pick(&mut h, "Free Transform Path");
    let dialog = h.state().ui.dialogs.last().and_then(|d| d.fields.get("__command")).cloned();
    assert_eq!(dialog, Some(json!("path.transform")), "the Pen's Free Transform Path dialog");
    h.state_mut().ui.dialogs.clear();
    entries_at(&mut h, [2.0, 2.0]);
    pick(&mut h, "Blending Options…");
    let kind = h.state().ui.dialogs.last().map(|d| &d.kind);
    assert!(matches!(kind, Some(crate::state::DialogKind::LayerStyle)), "the menu bar's Layer Style dialog");
    h.state_mut().ui.dialogs.clear();
    entries_at(&mut h, [2.0, 2.0]);
    pick(&mut h, "Isolate Layers");
    assert!(!h.state().session.active().unwrap().isolated_layers.is_empty(), "Isolate Layers isolates the ellipse layer");
    for (label, command) in
        [("New Guides From Shape", "view.newGuidesFromShape"), ("Copy Complete Stroke", "path.style.copyStroke"), ("Rasterize Layer", "layer.rasterize.layer")]
    {
        entries_at(&mut h, [2.0, 2.0]);
        let before = h.state().session.journal.len();
        pick(&mut h, label);
        assert!(h.state().session.journal[before..].iter().any(|(id, _)| id == command), "{label} runs {command}");
    }
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name(), "Pixel", "the ellipse is rasterized");
}

#[test]
fn path_and_pixels_mode_open_no_menu() {
    for mode in ["path", "pixels"] {
        for (layer, app) in [("pixel", pixel_layer()), ("ellipse", ellipse_layer())] {
            let mut h = harness(ellipse_tool(app, mode));
            let p = screen_of(h.state(), [16.0, 16.0]);
            right_click(&mut h, p, Modifiers::NONE);
            assert!(h.state().ui.canvas_tool_menu.is_none(), "{mode} mode, {layer} layer");
            assert!(h.state().ui.layer_menu.is_none(), "{mode} mode, {layer} layer");
        }
    }
}

/// Over the layer kinds the reference wasn't measured on, Shape mode opens no menu, as before.
#[test]
fn other_layer_kinds_open_no_menu() {
    let kinds: [(&str, &str, Value); 3] = [
        ("Group", "layer.new.group", json!({})),
        ("Adjustment", "layer.newAdjustmentLayer.invert", json!({})),
        ("Type", "type.create", json!({"x": 4, "y": 20, "text": "A", "size": 12})),
    ];
    for (kind, command, params) in kinds {
        let mut app = app();
        app.run(command, params).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name(), kind);
        let mut h = harness(ellipse_tool(app, "shape"));
        let p = screen_of(h.state(), [2.0, 2.0]);
        right_click(&mut h, p, Modifiers::NONE);
        assert!(h.state().ui.canvas_tool_menu.is_none(), "{kind} layer");
        assert!(h.state().ui.layer_menu.is_none(), "{kind} layer");
    }
}
