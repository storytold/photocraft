//! #11837: the Quick Selection's options bar carries its own New / Add / Subtract buttons, and
//! picking one sets the tool's mode alone: the other selection tools keep the document's
//! `ui.selection_mode`, which the Quick Selection has no Intersect entry for.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The selections the bar offers, in Photoshop's words.
const NEW: &str = "New selection";
const ADD: &str = "Add to selection";
const SUBTRACT: &str = "Subtract from selection";
const INTERSECT: &str = "Intersect with selection";

/// A 100 × 100 document with the Quick Selection, while the other selection tools sit on
/// Intersect — a mode the Quick Selection doesn't have.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.ui.tool = Tool::QuickSelection;
    app.ui.selection_mode = 3;
    app
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    use crate::app::framework::BuildApp;
    let mut h = Harness::builder().with_size(vec2(1400.0, 120.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn the_quick_selection_bar_offers_its_own_three_modes() {
    let h = harness(app());
    for label in [NEW, ADD, SUBTRACT] {
        assert!(h.query_by_label(label).is_some(), "the bar shows {label}");
    }
    assert!(h.query_by_label(INTERSECT).is_none(), "the Quick Selection has no Intersect mode");
    assert_eq!(h.state().ui.selection_mode, 3, "the bar leaves the other selection tools' mode alone");
    assert!(
        h.get_by_label(SUBTRACT).rect().right() < h.get_by_label("Toggle the Brush Settings panel").rect().left(),
        "the mode buttons sit before the brush controls"
    );
    assert!(h.query_by_label("Select and Mask…").is_some(), "Select and Mask… is at the right end of the bar");
}

#[test]
fn picking_a_mode_sets_the_tools_own_mode_only() {
    let mut h = harness(app());
    h.get_by_label(SUBTRACT).click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.quick_selection_mode, 2, "Subtract is the tool's own mode");
    assert_eq!(h.state().ui.selection_mode, 3, "the other selection tools keep Intersect");
    // And the bar shows the tool's mode again on the next pass, not the document's.
    assert_eq!(h.state().ui.tool_options.quick_selection_mode, 2);
}

#[test]
fn another_selection_tool_keeps_the_document_mode() {
    let mut app = app();
    app.ui.tool = Tool::MagicWand;
    let mut h = harness(app);
    h.get_by_label(ADD).click();
    h.run_steps(2);
    assert_eq!(h.state().ui.selection_mode, 1, "the Magic Wand writes the document's mode");
    assert_eq!(h.state().ui.tool_options.quick_selection_mode, 0, "and leaves the Quick Selection's own mode alone");
}
