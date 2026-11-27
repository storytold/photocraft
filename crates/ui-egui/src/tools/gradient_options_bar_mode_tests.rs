//! #12007: a live (Gradient mode) drag makes a Normal, 100 % fill layer and the options change
//! never changes a selected fill layer's blend or opacity.

use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120, "background": "white"})).unwrap();
    app.ui.tool = Tool::Gradient;
    app
}

fn drag(app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]) {
    let none = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, none);
}

#[test]
fn the_bar_no_longer_changes_a_selected_fill_layers_blend() {
    let mut a = app();
    drag(&mut a, [20.0, 60.0], [180.0, 60.0]);
    let steps = a.session.active().unwrap().history.past_len();
    let before = a.ui.tool_options.clone();
    a.ui.tool_options.gradient_blend_mode = BlendMode::Difference;
    crate::gradient_ui::options_changed(&mut a, &before);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.layers.last().unwrap().blend, BlendMode::Normal);
    assert_eq!(st.history.past_len(), steps, "no history step");
}
