//! #16878: the pasteboard right-click menu lists Select Custom Color… directly
//! under Custom, with no separator between them.

use egui::{Modifiers, vec2};
use serde_json::json;

use super::canvas_tool_menu::tests::{harness, right_click};
use crate::PhotocraftApp;
use crate::state::Tool;

#[test]
fn pasteboard_menu_has_no_separator_between_custom_and_select_custom_color() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::RectMarquee;
    let mut h = harness(app);
    let pasteboard = h.state().last_canvas_rect.min + vec2(5.0, 5.0);
    right_click(&mut h, pasteboard, Modifiers::NONE);

    let inspected = crate::control::inspect(h.state(), &h.ctx);
    let entries = inspected["canvasToolMenu"]["entries"].as_array().expect("pasteboard menu entries");
    let rows: Vec<Option<&str>> = entries.iter().map(|e| if e["separator"] == json!(true) { None } else { e["label"].as_str() }).collect();

    assert_eq!(
        rows,
        vec![Some("Default"), Some("Black"), Some("Dark Gray"), Some("Medium Gray"), Some("Light Gray"), Some("Custom"), Some("Select Custom Color…"),],
        "expected no separator between Custom and Select Custom Color…"
    );
}
