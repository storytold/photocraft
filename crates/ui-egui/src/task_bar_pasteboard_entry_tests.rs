//! #16757: with the Move tool, the pasteboard's right-click menu starts with Hide Contextual Task Bar
//! (Show Contextual Task Bar while the bar is off) and a separator, and the row toggles the same flag
//! as Window › Contextual Task Bar.

use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::canvas_tool_menu::tests::{harness, right_click};
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = tool;
    app
}

/// Right-clicks the pasteboard and returns the open menu's rows, a separator as `None`.
fn pasteboard_rows(h: &mut Harness<'static, Box<PhotocraftApp>>) -> Vec<Option<String>> {
    let pasteboard = h.state().last_canvas_rect.min + vec2(5.0, 5.0);
    right_click(h, pasteboard, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["pasteboard"], json!(true), "the pasteboard menu opens");
    let entries = inspected["canvasToolMenu"]["entries"].as_array().expect("pasteboard menu entries");
    entries.iter().map(|e| if e["separator"] == json!(true) { None } else { e["label"].as_str().map(str::to_owned) }).collect()
}

#[test]
fn the_move_tools_pasteboard_menu_starts_with_the_task_bar_row() {
    let mut marquee = harness(app(Tool::RectMarquee));
    let today = pasteboard_rows(&mut marquee);
    assert!(!today.iter().flatten().any(|label| label.contains("Contextual Task Bar")), "other tools keep today's menu: {today:?}");

    let mut h = harness(app(Tool::Move));
    let rows = pasteboard_rows(&mut h);
    assert_eq!(rows.first(), Some(&Some("Hide Contextual Task Bar".to_owned())), "{rows:?}");
    assert_eq!(rows.get(1), Some(&None), "a separator under it: {rows:?}");
    assert_eq!(rows[2..], today[..], "then today's colour rows");
}

#[test]
fn the_task_bar_row_hides_and_shows_the_bar_as_the_window_item_does() {
    let mut h = harness(app(Tool::Move));
    assert!(h.state().ui.panels.context_task_bar);

    pasteboard_rows(&mut h);
    h.get_by_label("Hide Contextual Task Bar").click();
    h.run_steps(2);
    assert!(!h.state().ui.panels.context_task_bar, "the row hides the bar");
    assert!(h.state().ui.canvas_tool_menu.is_none(), "and closes the menu");
    // The Window item toggles the same flag: one press turns the bar back on, and off again.
    let ctx = h.ctx.clone();
    let window_item =
        |h: &mut Harness<'static, Box<PhotocraftApp>>| crate::menus::invoke(h.state_mut(), &ctx, "window.panel.contextualTaskBar", json!({})).unwrap();
    assert_eq!(window_item(&mut h), json!(true));
    assert_eq!(window_item(&mut h), json!(false));

    let rows = pasteboard_rows(&mut h);
    assert_eq!(rows.first(), Some(&Some("Show Contextual Task Bar".to_owned())), "{rows:?}");
    h.get_by_label("Show Contextual Task Bar").click();
    h.run_steps(2);
    assert!(h.state().ui.panels.context_task_bar, "the row shows the bar again");
}
