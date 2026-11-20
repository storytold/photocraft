//! #14172: Edit > Check Spelling... chosen from the menu in a document without type layers is
//! enabled and opens nothing: no dialog, no message, no history state. With no document open the
//! item is greyed.

use craft::text::ui::spelling::CheckSpellingDialog;
use serde_json::json;

use crate::PhotocraftApp;

/// An app with `doc` open (or none): a white 800 x 500 document with an empty layer on top.
pub(super) fn app(doc: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if doc {
        app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).unwrap();
        app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    }
    app.sync_views();
    app
}

/// Whether Edit > Check Spelling... is enabled in the menu.
pub(super) fn enabled(app: &PhotocraftApp) -> bool {
    crate::menus::is_enabled(app, "edit.checkSpelling")
}

/// Chooses Edit > Check Spelling... as the menu does.
pub(super) fn choose(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "edit.checkSpelling", json!({}))
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn check_spelling_without_type_layers_is_enabled_and_opens_nothing() {
    let mut app = app(true);
    let before = history(&app);
    assert!(enabled(&app), "Check Spelling... is enabled without type layers");
    choose(&mut app).unwrap();
    assert!(app.ui.type_panels.spell.is_none(), "no Check Spelling dialog");
    assert!(app.framework.dialogs.is_empty(), "no dialog at all");
    assert!(app.ui.status.is_empty() && !app.ui.status_error, "no message: {:?}", app.ui.status);
    assert_eq!(history(&app), before, "no history state");
}

#[test]
fn check_spelling_is_greyed_without_a_document() {
    assert!(!enabled(&app(false)));
}

/// Lock All on the only type layer is not measured: the menu keeps today's dialog, which opens
/// with nothing listed (the hidden and locked layer prompts are #14175).
#[test]
fn check_spelling_with_only_a_lock_all_type_layer_opens_an_empty_dialog() {
    let mut app = app(true);
    let id = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh end"})).unwrap()["layer"].clone();
    app.run("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    assert!(enabled(&app), "Check Spelling... is enabled");
    choose(&mut app).unwrap();
    assert!(app.ui.type_panels.spell.is_some(), "the Check Spelling dialog opens");
    let dialog = app.framework.dialogs.get::<CheckSpellingDialog>().expect("the hosted dialog");
    assert!(dialog.items().is_empty(), "with nothing listed: {:?}", dialog.items());
}
