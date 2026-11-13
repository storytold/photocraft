//! #13705, #13706: with the Line or the Ellipse tool in Shape mode, the canvas menu keeps Copy Fill
//! and Paste Fill live over a pixel layer, as measured, and greys only Copy Complete Stroke and
//! Paste Complete Stroke there. Over a shape layer the fill rows copy the layer's fill and paste it
//! onto another shape layer.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::line_menu_tests::greyed;
use super::tests::{app, harness, right_click, screen_of};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The shape tools whose Shape-mode menu the issues measured.
const TOOLS: [Tool; 2] = [Tool::Line, Tool::EllipseShape];

/// The menu's Copy and Paste groups.
const STYLE_ROWS: [&str; 4] = ["Copy Fill", "Copy Complete Stroke", "Paste Fill", "Paste Complete Stroke"];

/// `tool` draws a shape from `from` to `to` (document points) filled with the foreground colour
/// `rgb`, with no stroke; its new shape layer is active. Returns that layer.
fn draw(app: &mut PhotocraftApp, tool: Tool, rgb: [f32; 3], from: [f32; 2], to: [f32; 2]) -> LayerId {
    app.ui.tool = tool;
    app.ui.tool_options.shape_mode = "shape".into();
    app.session.tools.foreground = [rgb[0], rgb[1], rgb[2], 1.0];
    crate::vector_ui::finish_shape(app, tool, from, to, Modifiers::NONE);
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(fill_of(app, id).is_some(), "{tool:?} drew a filled shape layer");
    id
}

/// The fill of shape layer `id`.
fn fill_of(app: &PhotocraftApp, id: LayerId) -> Option<Fill> {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.fill.clone(),
        other => panic!("layer {id:?} is a {} layer", other.kind_name()),
    }
}

/// A red pixel layer added above the others, active, with `tool` picked in Shape mode and no path.
fn pixel_layer(mut app: PhotocraftApp, tool: Tool) -> PhotocraftApp {
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.ui.tool = tool;
    app.ui.tool_options.shape_mode = "shape".into();
    app
}

/// Right-click the middle of the document: the greyed rows of the menu's Copy and Paste groups.
fn greyed_style_rows(h: &mut Harness<'static, PhotocraftApp>, tool: Tool) -> Vec<String> {
    let p = screen_of(h.state(), [16.0, 16.0]);
    right_click(h, p, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!(format!("{tool:?}")));
    let entries = inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default();
    greyed(&entries).into_iter().filter(|label| STYLE_ROWS.contains(&label.as_str())).collect()
}

/// Click the open menu's row `label`, which closes the menu.
fn pick(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "{label} closes the menu");
}

#[test]
fn over_a_pixel_layer_only_the_stroke_rows_are_greyed() {
    for tool in TOOLS {
        // Nothing copied yet: Paste Fill stays live all the same, as measured.
        let mut h = harness(pixel_layer(app(), tool));
        assert!(h.state().session.path_fill_clipboard.is_none());
        assert_eq!(greyed_style_rows(&mut h, tool), ["Copy Complete Stroke", "Paste Complete Stroke"], "{tool:?}");
    }
}

/// Over a pixel layer the fill rows reach their commands, which need a shape layer: the status bar
/// says so, and neither the pixels, the copied fill nor the history change.
#[test]
fn over_a_pixel_layer_the_fill_rows_report_the_layer_and_change_nothing() {
    for tool in TOOLS {
        let mut app = app();
        draw(&mut app, tool, [0.0, 0.0, 1.0], [4.0, 4.0], [12.0, 12.0]);
        app.run("path.style.copyFill", json!({})).unwrap();
        let copied = app.session.path_fill_clipboard.clone();
        assert!(copied.is_some());
        let app = pixel_layer(app, tool);
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let pixels = st.doc.layer(layer).unwrap().content.clone();
        let mut h = harness(app);
        for label in ["Copy Fill", "Paste Fill"] {
            assert_eq!(greyed_style_rows(&mut h, tool), ["Copy Complete Stroke", "Paste Complete Stroke"], "{tool:?}");
            let before = h.state().session.journal.len();
            pick(&mut h, label);
            let app = h.state();
            assert!(app.ui.status_error, "{tool:?}: {label}");
            assert!(app.ui.status.contains("not a shape layer"), "{tool:?}: {label}: {}", app.ui.status);
            assert_eq!(app.session.journal.len(), before, "{tool:?}: {label} records nothing");
            assert_eq!(app.session.path_fill_clipboard, copied, "{tool:?}: {label} keeps the copied fill");
            let st = app.session.active().unwrap();
            assert_eq!(st.active_layer, Some(layer));
            assert!(st.doc.layer(layer).unwrap().content == pixels, "{tool:?}: {label} leaves the pixels");
        }
    }
}

/// Over a shape layer Copy Fill copies that layer's fill, and Paste Fill, live once a fill is
/// copied, gives it to the shape layer the menu opens over.
#[test]
fn over_a_shape_layer_the_fill_rows_copy_and_paste_the_layer_fill() {
    for tool in TOOLS {
        let mut app = app();
        let red = draw(&mut app, tool, [1.0, 0.0, 0.0], [4.0, 4.0], [12.0, 12.0]);
        let blue = draw(&mut app, tool, [0.0, 0.0, 1.0], [18.0, 20.0], [28.0, 28.0]);
        let blue_fill = fill_of(&app, blue);
        assert_ne!(fill_of(&app, red), blue_fill);
        let mut h = harness(app);

        // No stroke and nothing copied yet.
        assert_eq!(greyed_style_rows(&mut h, tool), ["Copy Complete Stroke", "Paste Fill", "Paste Complete Stroke"], "{tool:?}");
        pick(&mut h, "Copy Fill");
        assert!(!h.state().ui.status_error, "{tool:?}: {}", h.state().ui.status);
        assert_eq!(h.state().session.path_fill_clipboard, blue_fill, "{tool:?}: the blue layer's fill is copied");

        h.state_mut().run("layer.select", json!({"layer": red.0})).unwrap();
        assert_eq!(greyed_style_rows(&mut h, tool), ["Copy Complete Stroke", "Paste Complete Stroke"], "{tool:?}");
        pick(&mut h, "Paste Fill");
        let app = h.state();
        assert!(!app.ui.status_error, "{tool:?}: {}", app.ui.status);
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.style.pasteFill"));
        assert_eq!(fill_of(app, red), blue_fill, "{tool:?}: the red layer takes the copied fill");
        assert_eq!(fill_of(app, blue), blue_fill, "{tool:?}: the source keeps its fill");
    }
}
