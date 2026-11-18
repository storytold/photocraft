//! Shift+Alt+Q selects Behind for Brush and Pencil (#11894).

use craft::ui::patterns::blend_keys::BlendShortcut;
use egui::{Context, RawInput};
use photocraft_color::BlendMode;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_mode_menu::{self, Item};
use crate::state::Tool;

/// Create a test document for the selected tool, optionally with a transparent layer.
fn app(tool: Tool, transparent_layer: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    if transparent_layer {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    app.ui.tool = tool;
    app
}

/// Fires the Behind command (Shift+Alt+Q) and asserts that the module takes it.
fn press_behind(app: &mut PhotocraftApp) {
    let command = craft::ui::runtime::commands::CommandRef::from(BlendShortcut::Behind);
    assert!(super::fired(app, &super::test_fired(command)));
}

/// Check whether the rendered options bar displays the Behind label.
fn options_show_behind(app: &mut PhotocraftApp, ctx: &Context) -> bool {
    /// Find the Behind text label in a shape or its descendants.
    fn has_behind(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.text() == "Behind",
            egui::Shape::Vec(shapes) => shapes.iter().any(has_behind),
            _ => false,
        }
    }
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 300.0)));
    let mut out = ctx.run_ui(RawInput { screen_rect, ..Default::default() }, |ui| crate::panels::options_bar(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().any(|shape| has_behind(&shape.shape))
}

/// The key leaves Clear, uses one menu-equivalent command, and changes the displayed Mode.
#[test]
fn brush_and_pencil_select_behind_in_one_command() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut app = app(tool, true);
        app.ui.tool_options.brush_clear = true;
        app.ui.tool_options.pencil_clear = true;
        app.session.tools.brush.mode = BlendMode::Multiply;
        let ctx = app.ctx().clone();
        PhotocraftApp::setup_context(&ctx, Default::default());
        assert!(brush_mode_menu::enabled(&app, Item::Behind));
        let (id, params) = brush_mode_menu::engine_call(&app, tool, Item::Behind).unwrap();
        assert_eq!((id, &params), ("tools.setBrush", &json!({"brush": {"mode": "Normal", "behind": true}})));
        let before = app.session.journal.len();
        press_behind(&mut app);
        assert_eq!(&app.session.journal[before..], &[(id.to_owned(), params)]);
        assert_eq!(app.ui.brush_tool, tool);
        assert_eq!(app.ui.tool_options.brush_clear, tool != Tool::Brush);
        assert_eq!(app.ui.tool_options.pencil_clear, tool != Tool::Pencil);
        let brush = app.session.execute("brush.get", json!({})).unwrap();
        assert_eq!(brush["behind"], true);
        assert_eq!(brush["mode"], "Normal");
        assert!(options_show_behind(&mut app, &ctx), "{tool:?}");
    }
}
