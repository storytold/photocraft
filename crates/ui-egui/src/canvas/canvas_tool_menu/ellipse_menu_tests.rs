//! #13232: the Ellipse tool's Shape-mode canvas right-click opens the shape menu, in the reference
//! application's order: on a pixel layer with no path and on an ellipse shape layer, inside the
//! ellipse or off it. Its rows run what the menu bar or the Pen's menu runs. Path and Pixels mode,
//! and other layer kinds, open no menu, as before.

use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::available;
use super::line_menu_tests::{greyed, menu, shown};
use super::tests::{any_dialog, app, harness, open_here, pick, right_click, screen_of};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;
use craft::draw::ui::VectorMode;

const PIXEL_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

const SHAPE_LAYER_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform Path ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

/// The Ellipse tool picked in `mode`.
fn ellipse_tool(mut app: Box<PhotocraftApp>, mode: VectorMode) -> Box<PhotocraftApp> {
    app.ui.tool = Tool::EllipseShape;
    app.ui.tool_options.shape_mode = mode;
    app
}

/// A pixel layer filled red above the Background, active, and no path.
fn pixel_layer() -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    ellipse_tool(app, VectorMode::Shape)
}

/// An ellipse the Ellipse tool drew over the middle of a 200 px document, with a stroke: its shape
/// layer is active. The live shape's handles take a press near them, so the points this file
/// right-clicks, (10, 10) off the ellipse and (100, 100) inside it, keep clear of them.
fn ellipse_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let mut app = ellipse_tool(app, VectorMode::Shape);
    // The measured ellipse had a stroke; without one Copy Complete Stroke greys, as in the Line's menu.
    app.ui.tool_options.stroke_width = 2.0;
    testing::press(&mut app, [40.0, 40.0], Modifiers::NONE);
    testing::move_to(&mut app, [100.0, 100.0], Modifiers::NONE);
    testing::move_to(&mut app, [160.0, 160.0], Modifiers::NONE);
    testing::release(&mut app, [160.0, 160.0], Modifiers::NONE);
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, photocraft_doc::LayerContent::Shape(_)), "the ellipse made a shape layer");
    app
}

/// Right-click document point `doc`: the open menu's entries as `ui.inspect` lists them.
fn entries_at(h: &mut Harness<'static, Box<PhotocraftApp>>, doc: [f32; 2]) -> Vec<Value> {
    let p = screen_of(h.state(), doc);
    right_click(h, p, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!("EllipseShape"));
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// What a right-click at `doc` opens, as `ui.inspect` lists it: the tool menu and the layer menu.
fn opened_at(h: &mut Harness<'static, Box<PhotocraftApp>>, doc: [f32; 2]) -> (Value, Value) {
    let p = screen_of(h.state(), doc);
    right_click(h, p, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    (inspected["canvasToolMenu"].clone(), inspected["layerMenu"].clone())
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
    let app = h.state_mut();
    pick(app, "edit.freeTransform");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.transform.is_some(), "Free Transform opens the box");
}

#[test]
fn on_an_ellipse_layer_the_menu_is_the_same_inside_and_off_the_ellipse() {
    let mut h = harness(ellipse_layer());
    let off = entries_at(&mut h, [10.0, 10.0]);
    assert_eq!(shown(&off), menu(SHAPE_LAYER_MENU));
    let before = h.state().session.journal.len();
    pick(h.state_mut(), "path.style.copyFill");
    assert!(h.state().session.journal[before..].iter().any(|(id, _)| id == "path.style.copyFill"));
    // With a fill copied, as in the measured session, the greyed rows are the reference's.
    let off = entries_at(&mut h, [10.0, 10.0]);
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
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Escape closes the menu");
    assert_eq!(entries_at(&mut h, [100.0, 100.0]), off, "inside the ellipse");
}

/// A row that acts on the path opens the dialog or runs the command the Pen's menu does for the
/// same layer.
#[test]
fn path_rows_run_as_from_the_pen() {
    for command in ["path.transform", "path.style.copyFill", "path.style.copyStroke", "select.isolateLayers"] {
        let mut by_tool = Vec::new();
        for tool in [Tool::Pen, Tool::EllipseShape] {
            let mut app = ellipse_layer();
            app.ui.tool = tool;
            assert!(open_here(&mut app, tool, [10.0, 10.0]), "{tool:?}");
            let menu = app.ui.canvas_tool_menu.clone().unwrap();
            assert!(available(&app, &menu, command), "{tool:?}: {command} is enabled");
            let before = app.session.journal.len();
            pick(&mut app, command);
            assert!(!app.ui.status_error, "{tool:?}: {command}: {}", app.ui.status);
            let ran: Vec<_> = app.session.journal[before..].iter().map(|(id, _)| id.clone()).collect();
            by_tool.push((any_dialog(&app), ran, app.session.path_fill_clipboard.is_some()));
        }
        assert_eq!(by_tool[0], by_tool[1], "{command}");
    }
}

/// The layer rows run the menu bar's command.
#[test]
fn layer_rows_run_the_menu_bar_command() {
    let mut app = ellipse_layer();
    assert!(open_here(&mut app, Tool::EllipseShape, [10.0, 10.0]));
    pick(&mut app, "layer.layerStyle.blendingOptions");
    assert!(any_dialog(&app), "the menu bar's Layer Style dialog");

    for command in ["view.newGuidesFromShape", "layer.rasterize.layer"] {
        let mut app = ellipse_layer();
        assert!(open_here(&mut app, Tool::EllipseShape, [10.0, 10.0]));
        let before = app.session.journal.len();
        pick(&mut app, command);
        assert!(app.session.journal[before..].iter().any(|(id, _)| id == command), "{command} runs");
    }
    let mut app = ellipse_layer();
    assert!(open_here(&mut app, Tool::EllipseShape, [10.0, 10.0]));
    pick(&mut app, "layer.rasterize.layer");
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Raster(_)), "the ellipse is rasterized");
}

#[test]
fn path_and_pixels_mode_open_no_menu() {
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        // One app at a time: an array of built apps keeps them all on the test thread's stack.
        for make in [pixel_layer as fn() -> Box<PhotocraftApp>, ellipse_layer] {
            let mut h = harness(ellipse_tool(make(), mode));
            assert_eq!(opened_at(&mut h, [16.0, 16.0]), (Value::Null, Value::Null), "{mode:?} mode");
        }
    }
}

/// Over the layer kinds the reference wasn't measured on, Shape mode opens no menu, as before.
#[test]
fn other_layer_kinds_open_no_menu() {
    let kinds: [(&str, Value); 3] =
        [("layer.new.group", json!({})), ("layer.newAdjustmentLayer.invert", json!({})), ("type.create", json!({"x": 4, "y": 20, "text": "A", "size": 12}))];
    for (command, params) in kinds {
        let mut app = app();
        app.run(command, params).unwrap();
        let mut h = harness(ellipse_tool(app, VectorMode::Shape));
        assert_eq!(opened_at(&mut h, [2.0, 2.0]), (Value::Null, Value::Null), "{command}");
    }
}
