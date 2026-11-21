//! #12046: a paragraph box dragged with the Horizontal or Vertical Type tool starts with the whole
//! Lorem ipsum paragraph, selected, so typing replaces it. A click keeps "Lorem Ipsum". With the
//! placeholder preference off both start empty. The Horizontal Type Mask's box follows the Type
//! tool's (#12072); the Vertical Type Mask still starts empty.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::mask_replace_selection_tests::new_app;
use super::super::{PARAGRAPH_PLACEHOLDER, PLACEHOLDER, current_text};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

const PARAGRAPH: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Quis ipsum suspendisse ultrices gravida. Risus commodo viverra maecenas accumsan lacus vel facilisis.";

fn app_with(tool: Tool, placeholder: bool) -> Box<PhotocraftApp> {
    let mut app = new_app();
    app.run("prefs.set", json!({"path": "type.fillNewTypeLayersWithPlaceholder", "value": placeholder})).unwrap();
    app.ui.tool = tool;
    app
}

fn drag(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 200.0, y: 120.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 320.0, y: 220.0 }, Modifiers::NONE);
}

fn click(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 100.0, y: 200.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 100.0, y: 200.0 }, Modifiers::NONE);
}

/// The new text, and the selected range as (anchor, caret).
fn started(app: &PhotocraftApp) -> (String, (usize, usize)) {
    let ed = app.ui.text_edit.as_ref().expect("editing new text");
    (current_text(app, LayerId(ed.layer)).unwrap_or_default(), (ed.anchor, ed.caret))
}

#[test]
fn a_dragged_box_starts_with_the_whole_paragraph_selected() {
    assert_eq!(PARAGRAPH_PLACEHOLDER, PARAGRAPH, "the measured text");
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = app_with(tool, true);
        drag(&mut app);
        assert_eq!(started(&app), (PARAGRAPH.to_string(), (0, PARAGRAPH.chars().count())), "{tool:?}");
    }
}

#[test]
fn a_click_keeps_lorem_ipsum() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = app_with(tool, true);
        click(&mut app);
        assert_eq!(started(&app), (PLACEHOLDER.to_string(), (0, PLACEHOLDER.chars().count())), "{tool:?}");
    }
}

#[test]
fn with_the_preference_off_both_start_empty() {
    let mut app = app_with(Tool::Type, false);
    drag(&mut app);
    assert_eq!(started(&app).0, "");
    let mut app = app_with(Tool::Type, false);
    click(&mut app);
    assert_eq!(started(&app).0, "");
}

#[test]
fn the_type_mask_tools_follow_the_type_tool_as_today() {
    // #12072 measured the Horizontal Type Mask's box starting with the Type tool's box text.
    let mut app = app_with(Tool::HorizontalTypeMask, true);
    drag(&mut app);
    assert_eq!(started(&app).0, PARAGRAPH);
    let mut app = app_with(Tool::VerticalTypeMask, true);
    drag(&mut app);
    assert_eq!(started(&app).0, "", "the Vertical Type Mask still starts empty");
}
