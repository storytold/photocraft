use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 200, 300], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [200, 0, 200, 300], "fill": "#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

/// Every drag pick applies, not only the release: the swatch follows the pointer.
#[test]
fn eyedropper_drag_updates_the_sampled_colour() {
    let mut app = app(Tool::Eyedropper);
    let none = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, none);
    assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "the press picked the red half");
    tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 100.0, pressure: 1.0 }, none);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "the drag picked the green half before the release");
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 100.0 }, none);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
    // Alt picks the background, and leaves the foreground.
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, egui::Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, egui::Modifiers::ALT);
    assert_eq!(app.session.tools.background, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
}
