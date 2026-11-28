use egui::{Modifiers, vec2};
use serde_json::json;

use super::canvas_tool_menu::MenuKind;
use super::canvas_tool_menu::tests::{harness, right_click};
use crate::PhotocraftApp;
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = tool;
    app
}

#[test]
fn tool_right_clicks_on_the_pasteboard_open_its_color_menu() {
    // Brush, and the ordinary on-image routes for several of these tools, are in context_popup_table.
    for tool in [Tool::SpotHealing, Tool::CloneStamp, Tool::Eraser, Tool::Blur, Tool::Dodge, Tool::Pen, Tool::PathSelection, Tool::QuickSelection] {
        let mut h = harness(app(tool));
        let pasteboard = h.state().last_canvas_rect.min + vec2(5.0, 5.0);
        right_click(&mut h, pasteboard, Modifiers::NONE);

        let menu = h.state().ui.canvas_tool_menu.as_ref().map(|menu| &menu.kind);
        assert!(matches!(menu, Some(MenuKind::Pasteboard)), "{tool:?}: got {menu:?}");
        assert!(h.state().ui.brush_picker.is_none(), "{tool:?}: the picker must not open");
    }
}

#[test]
fn block_eraser_uses_its_mode_menu_only_on_the_image() {
    let mut app = app(Tool::Eraser);
    app.ui.tool_options.eraser_mode = "block".into();
    let mut outside = harness(app);
    let pasteboard = outside.state().last_canvas_rect.min + vec2(5.0, 5.0);
    right_click(&mut outside, pasteboard, Modifiers::NONE);
    assert!(matches!(outside.state().ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::Pasteboard)));

    let mut app = self::app(Tool::Eraser);
    app.ui.tool_options.eraser_mode = "block".into();
    let mut inside = harness(app);
    let image = inside.state().last_canvas_rect.center();
    right_click(&mut inside, image, Modifiers::NONE);
    assert!(matches!(inside.state().ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::BrushMode { plain: true })));
}

#[test]
fn pasteboard_clicks_preserve_erase_and_shift_mode_gestures() {
    let mut app = app(Tool::Brush);
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    let mut h = harness(app);
    let pasteboard = h.state().last_canvas_rect.min + vec2(5.0, 5.0);

    right_click(&mut h, pasteboard, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none());
    assert!(h.state().ui.brush_picker.is_none());

    right_click(&mut h, pasteboard, Modifiers::SHIFT);
    assert!(matches!(h.state().ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::BrushMode { plain: false })));
}

#[test]
fn tools_outside_the_pasteboard_change_keep_their_context_picker() {
    let mut h = harness(app(Tool::Pencil));
    let pasteboard = h.state().last_canvas_rect.min + vec2(5.0, 5.0);
    right_click(&mut h, pasteboard, Modifiers::NONE);

    assert!(h.state().ui.brush_picker.is_some());
    assert!(h.state().ui.canvas_tool_menu.is_none());
}
