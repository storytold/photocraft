//! #12081: while a Type Mask is being edited the options bar's text-colour well has no colour to show, and a colour
//! chosen from it changes nothing. Idle with the mask tool, the plain Type tool's bar, and a mask click that edits an
//! existing text layer keep the well as it is today.
//!
//! The well is drawn by the framework's type bar (`craft::text::ui::type_bar::TypeBar`); the bar here owns which state
//! it asks the framework for and what a pick may do. These tests drive the real bar in a harness and click its well by
//! name, so the guard is click-tested and the untouched cases keep the picker they open today.

use egui::Modifiers;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::mask_replace_selection_tests::{click, new_app};
use super::super::text_layer;
use super::super::vertical_align_buttons_tests::bar;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The issue's document: 800x400, the mask tool in hand, type size 48 pt.
fn with_tool(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = new_app();
    app.ui.tool = tool;
    app
}

/// Starts a Type Mask session with a click and returns the app with its working layer.
fn mask_session(tool: Tool) -> (Box<PhotocraftApp>, LayerId) {
    let mut app = with_tool(tool);
    click(&mut app, 400.0, 100.0, Modifiers::NONE);
    let work = LayerId(app.ui.text_edit.as_ref().filter(|ed| ed.mask).expect("a mask session").layer);
    (app, work)
}

/// The mask's working layer: the runs its glyphs are laid out from.
fn runs(app: &PhotocraftApp, work: LayerId) -> Vec<photocraft_doc::text::TextRun> {
    text_layer(&app.session.active().unwrap().doc, work).expect("the mask's working layer").char_runs()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The bar's colour well, clicked, with the passes that answer its request run.
fn click_well(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut h = bar(app);
    h.get_by_label("Text color").click();
    h.run_steps(2);
    h
}

/// The measured row: a mask edit has no colour to choose, so the well's click starts no picker and leaves the working
/// layer, the history and the tool colours as they were.
#[test]
fn a_colour_chosen_while_a_mask_is_edited_changes_nothing() {
    let (app, work) = mask_session(Tool::HorizontalTypeMask);
    let before = runs(app.as_ref(), work);
    let history = steps(app.as_ref());
    let tools = (app.session.tools.foreground, app.session.tools.background);
    let h = click_well(app);
    assert!(h.state().framework.dialogs.stack().top().is_none(), "the well of a mask edit starts no colour picker");
    assert_eq!(runs(h.state(), work), before, "no text colour and no style change on the mask's glyphs");
    assert_eq!(steps(h.state()), history, "no history step");
    assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), tools, "no tool colour change");
}

/// Both mask tools are held to the same word: the Vertical Type Mask row keeps the horizontal one's answer.
#[test]
fn either_mask_tool_edit_starts_no_picker() {
    for tool in [Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let (app, work) = mask_session(tool);
        let before = runs(app.as_ref(), work);
        let history = steps(app.as_ref());
        let h = click_well(app);
        assert!(h.state().framework.dialogs.stack().top().is_none(), "{tool:?}: no colour picker");
        assert_eq!(runs(h.state(), work), before, "{tool:?}: the runs are untouched");
        assert_eq!(steps(h.state()), history, "{tool:?}: no history step");
    }
}

/// Idle with a mask tool, a plain Type edit, and a mask click that edits an existing text layer keep both the colour
/// the well shows and the picker its click opens today.
#[test]
fn every_other_case_keeps_the_wells_colour_and_its_picker() {
    // Idle with the mask tool: no session in hand.
    let h = click_well(with_tool(Tool::HorizontalTypeMask));
    assert!(h.state().framework.dialogs.stack().top().is_some(), "an idle mask tool still opens its picker");

    // A plain Type edit, the well naming the selected character.
    let mut app = with_tool(Tool::Type);
    let id = LayerId(app.run("type.create", json!({"text": "TXT", "x": 400, "y": 100, "color": "#ff0000"})).unwrap()["layer"].as_u64().unwrap());
    app.ui.text_edit =
        Some(crate::state::TextEdit { layer: id.0, document: None, caret: 1, anchor: 0, session: "mask-colour-type-edit".into(), created: false, mask: false });
    let mut h = bar(app);
    assert_eq!(
        h.get_by_label("Text color").accesskit_node().value().map(|value| value.to_string()).as_deref(),
        Some("#ff0000"),
        "the Type edit shows the layer's colour"
    );
    h.get_by_label("Text color").click();
    h.run_steps(2);
    assert!(h.state().framework.dialogs.stack().top().is_some(), "a plain Type edit still opens its picker");

    // A mask click on an existing text layer's glyphs edits that layer as the Type tool does (#12075), and that edit
    // keeps its picker: the mask's own colour is the only case the well stops offering.
    let mut app = with_tool(Tool::Type);
    app.run("select.deselect", json!({})).unwrap();
    click(&mut app, 100.0, 200.0, Modifiers::NONE);
    super::super::host::type_text(&mut app, "TXT");
    super::super::commit(&mut app);
    let id = app.session.active().unwrap().active_layer.expect("the typed layer is active");
    app.ui.tool = Tool::HorizontalTypeMask;
    let x = (100..200)
        .map(f64::from)
        .find(|&x| super::super::layout::hit_layer(&mut app, x, 185.0) == Some(id) && super::super::layout::offset_at(&mut app, id, x, 185.0) == 1)
        .expect("a point after the first letter");
    click(&mut app, x, 185.0, Modifiers::NONE);
    let h = click_well(app);
    assert!(h.state().ui.text_edit.as_ref().is_some_and(|ed| !ed.mask), "the mask click edits the layer itself, no mask session");
    assert!(h.state().framework.dialogs.stack().top().is_some(), "that edit keeps its picker");
}
