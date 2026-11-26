//! #13885: with the Frame tool a right-click on the canvas opens the layer picker: the layers under
//! the pointer, topmost first, a frame's image indented under its frame, then a separator and
//! Isolate Layers. Off the image it keeps the pasteboard colour menu. The Move tool's picker and
//! the ⌘/Ctrl+right-click picker stay as they are.
use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing;
use crate::canvas_tool_menu::{self, MenuKind};

/// The issue's document: 600 × 400 white, Red (100 × 80 at 50,50) put in a frame ("Red Frame"),
/// Blue (120 × 60 at 300,200) and an empty layer on top. Returns the app and Red's id.
fn app(tool: Tool) -> (Box<PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    let mut red = 0;
    for (name, x, y, w, h, color) in [("Red", 50, 50, 100, 80, "#ff0000"), ("Blue", 300, 200, 120, 60, "#0000ff")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        if name == "Red" {
            red = app.session.active().unwrap().active_layer.unwrap().0;
        }
    }
    app.run("layer.new.frame", json!({"x": 50, "y": 50, "width": 100, "height": 80, "layer": red})).unwrap();
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    (app, red)
}

/// The open picker's rows as the menu shows them (labels, a separator as "-"), and the row ids.
fn rows(app: &PhotocraftApp) -> (Vec<String>, Vec<String>) {
    let menu = app.ui.canvas_tool_menu.as_ref().filter(|m| matches!(m.kind, MenuKind::Layers { .. })).expect("the layer picker");
    let entries = canvas_tool_menu::entries(app, menu);
    let labels = entries.iter().map(|e| if e.is_separator() { "-".to_string() } else { e.label().as_str().to_string() }).collect();
    (labels, entries.iter().filter_map(|e| e.value().cloned()).collect())
}

#[test]
fn on_empty_canvas_background_then_isolate_layers() {
    let (mut app, _) = app(Tool::Frame);
    testing::secondary_click(&mut app, [500.0, 350.0], Modifiers::NONE);
    let (labels, ids) = rows(&app);
    assert_eq!(labels, ["Background", "-", "Isolate Layers"]);
    assert!(app.session.active().unwrap().isolated_layers.is_empty());
    let ctx = app.ctx().clone();
    canvas_tool_menu::choose(&mut app, &ctx, ids.last().unwrap());
    assert!(app.ui.canvas_tool_menu.is_none(), "choosing closes the picker");
    let st = app.session.active().unwrap();
    assert_eq!(st.isolated_layers, st.selected_layers(), "select.isolateLayers ran on the selected layers");
    assert!(!st.isolated_layers.is_empty());
}

#[test]
fn on_the_frame_its_image_is_indented_under_it() {
    let (mut app, red) = app(Tool::Frame);
    testing::secondary_click(&mut app, [100.0, 90.0], Modifiers::NONE);
    let (labels, ids) = rows(&app);
    assert_eq!(labels, ["Red Frame", "    Red", "Background", "-", "Isolate Layers"]);
    let ctx = app.ctx().clone();
    canvas_tool_menu::choose(&mut app, &ctx, &row_id(red));
    assert!(ids.contains(&row_id(red)));
    assert!(app.ui.canvas_tool_menu.is_none());
    assert_eq!(app.session.active().unwrap().active_layer.map(|l| l.0), Some(red), "choosing selects the layer");
}

#[test]
fn off_the_image_the_frame_tool_keeps_the_pasteboard_menu() {
    let (mut app, _) = app(Tool::Frame);
    testing::secondary_click(&mut app, [-20.0, -20.0], Modifiers::NONE);
    let kind = app.ui.canvas_tool_menu.as_ref().map(|m| m.kind.clone());
    assert_eq!(kind, Some(MenuKind::Pasteboard), "the pasteboard colour menu, no layer picker");
}

#[test]
fn the_move_tools_picker_and_command_right_click_stay_as_they_are() {
    for (tool, mods) in [(Tool::Move, Modifiers::NONE), (Tool::Brush, Modifiers::COMMAND)] {
        let (mut app, _) = app(tool);
        testing::secondary_click(&mut app, [100.0, 90.0], mods);
        assert_eq!(rows(&app).0, ["Red", "Background"], "{tool:?}: no indent, no Isolate Layers");
    }
    assert!(!is_gesture(Tool::Brush, Mods::NONE) && is_gesture(Tool::Frame, Mods::NONE));
}
