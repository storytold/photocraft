//! #14172: Edit > Check Spelling... chosen from the menu bar in a document without type layers is
//! enabled and opens nothing: no dialog, no message, no history state. With no document open the
//! item is greyed.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

/// An app with `doc` open (or none): a white 800 x 500 document with an empty layer on top.
pub(super) fn app(doc: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if doc {
        app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).unwrap();
        app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    }
    app.sync_views();
    app
}

/// The menu bar and the type panels' windows (the Check Spelling dialog among them).
pub(super) fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
            super::windows(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Opens Edit and returns whether its Check Spelling... row is enabled.
pub(super) fn open_edit_menu(h: &mut Harness<'static, PhotocraftApp>) -> bool {
    h.get_by_label("Edit").click();
    h.run_steps(4);
    !h.get_by_label_contains("Check Spelling").accesskit_node().is_disabled()
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn check_spelling_without_type_layers_is_enabled_and_opens_nothing() {
    let mut h = harness(app(true));
    let before = history(h.state());
    assert!(open_edit_menu(&mut h), "Check Spelling... is enabled without type layers");
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    let app = h.state();
    assert!(app.ui.type_panels.spell.is_none(), "no Check Spelling dialog");
    assert!(app.ui.dialogs.is_empty(), "no other dialog either");
    assert!(h.query_by_label("Not in Dictionary:").is_none() && h.query_by_label("Done").is_none(), "nothing on screen");
    assert!(app.ui.status.is_empty() && !app.ui.status_error, "no message: {:?}", app.ui.status);
    assert_eq!(history(app), before, "no history state");
}

#[test]
fn check_spelling_is_greyed_without_a_document() {
    let mut h = harness(app(false));
    assert!(!open_edit_menu(&mut h));
}

/// Lock All on the only type layer is not measured: the menu keeps today's dialog, which opens
/// with nothing listed (the hidden and locked layer prompts are #14175).
#[test]
fn check_spelling_with_only_a_lock_all_type_layer_opens_an_empty_dialog() {
    let mut app = app(true);
    let id = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh end"})).unwrap()["layer"].clone();
    app.run("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    let mut h = harness(app);
    assert!(open_edit_menu(&mut h), "Check Spelling... is enabled");
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    let spell = h.state().ui.type_panels.spell.as_ref().expect("the Check Spelling dialog opens");
    assert!(spell.items.is_empty(), "with nothing listed: {:?}", spell.items);
    assert!(h.query_by_label("Done").is_some());
}
