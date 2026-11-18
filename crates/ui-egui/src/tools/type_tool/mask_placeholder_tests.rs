//! #12072: the Type Mask tools start new text with the Type tool's placeholder, selected, so
//! typing replaces it. Committing the untouched placeholder selects its outline. The Vertical Type
//! Mask's placeholder is not measured yet, so it still starts empty. Clicks and drags go through
//! the canvas's tool events.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::commit;
use super::super::host::type_text;
use super::super::mask_escape_tests::escape;
use super::super::mask_replace_selection_tests::{bounds, click, new_app, selection};
use super::super::{PLACEHOLDER, current_text};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

const AT: (f64, f64) = (100.0, 200.0);

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

/// The new text, and the selected range as (anchor, caret).
fn started(app: &PhotocraftApp) -> (String, (usize, usize)) {
    let ed = app.ui.text_edit.as_ref().expect("editing new text");
    (current_text(app, LayerId(ed.layer)).unwrap_or_default(), (ed.anchor, ed.caret))
}

fn app_with(tool: Tool) -> PhotocraftApp {
    let mut app = new_app();
    app.run("prefs.set", json!({"path": "type.fillNewTypeLayersWithPlaceholder", "value": true})).unwrap();
    app.ui.tool = tool;
    app
}

#[test]
fn a_click_with_the_type_mask_tool_starts_with_the_placeholder_selected() {
    let mut app = app_with(Tool::HorizontalTypeMask);
    click(&mut app, AT.0, AT.1, Modifiers::NONE);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "typing a mask");
    assert_eq!(started(&app), (PLACEHOLDER.to_owned(), (0, PLACEHOLDER.chars().count())));
    type_text(&mut app, "W");
    assert_eq!(started(&app), ("W".to_owned(), (1, 1)), "typing replaces the placeholder");

    // A drag starts the Type tool's box text, selected: paragraph text, not point text.
    let (from, to) = ([450.0, 100.0], [750.0, 300.0]);
    let mut text = app_with(Tool::Type);
    drag(&mut text, from, to);
    let (want, _) = started(&text);
    assert!(!want.is_empty());
    let mut app = app_with(Tool::HorizontalTypeMask);
    drag(&mut app, from, to);
    assert_eq!(started(&app), (want.clone(), (0, want.chars().count())), "the Type tool's box text, selected");
    let id = app.ui.text_edit.as_ref().unwrap().layer;
    assert!(super::box_shape(&app, LayerId(id)).is_some(), "paragraph text, not point text");
}

#[test]
fn a_click_with_the_vertical_type_mask_tool_still_starts_empty() {
    let mut app = app_with(Tool::VerticalTypeMask);
    click(&mut app, AT.0, AT.1, Modifiers::NONE);
    assert_eq!(started(&app), (String::new(), (0, 0)));
}

#[test]
fn committing_the_untouched_placeholder_selects_its_outline() {
    let start = || {
        let mut app = app_with(Tool::HorizontalTypeMask);
        // `new_app` starts with a selection at x 600, away from the text; the commit replaces it.
        assert!(bounds(&selection(&app)).is_some_and(|r| r.x0 == 600), "{:?}", bounds(&selection(&app)));
        click(&mut app, AT.0, AT.1, Modifiers::NONE);
        // The editor shows and takes the keyboard in the next pass, so the Escape below reaches it.
        crate::canvas::testing::frame(&mut app);
        app
    };
    let mut app = start();
    commit(&mut app);
    assert!(app.ui.text_edit.is_none());
    let got = selection(&app);
    assert!(bounds(&got).is_some_and(|r| r.x1 > r.x0 && r.y1 > r.y0 && r.x1 < 600), "the placeholder alone: {:?}", bounds(&got));
    // The outline of a type layer made with the same click.
    let mut text = app_with(Tool::Type);
    click(&mut text, AT.0, AT.1, Modifiers::NONE);
    let id = text.ui.text_edit.as_ref().unwrap().layer;
    commit(&mut text);
    text.run("type.createSelection", json!({"layer": id, "operation": "new"})).unwrap();
    assert!(got == selection(&text), "{:?}, not the type layer's {:?}", bounds(&got), bounds(&selection(&text)));
    // Escape commits a Type Mask (#12070), to the same selection.
    let escaped = escape(start());
    assert!(escaped.ui.text_edit.is_none());
    assert!(got == selection(&escaped), "Escape: {:?}", bounds(&selection(&escaped)));
}
