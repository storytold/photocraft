//! #14200: Edit › Auto-Align Layers from the menu, with nothing to align, opens an alert with the message, OK and a
//! "Don't show again" checkbox (off), instead of only a status-bar message and a notice. The document is unchanged.
//! Once "Don't show again" was ticked, the message shows in the status bar only. Engine callers still get the error.

use serde_json::json;

use super::filter_dialog::AUTO_ALIGN_NO_MATCH;
use crate::PhotocraftApp;

/// Two layers with nothing to match: a full red layer and a small blue rectangle, both selected.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"layer": red.0, "mode": "add"})).unwrap();
    app
}

/// Edit › Auto-Align Layers… and OK in its dialog.
fn auto_align_from_the_menu(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, "edit.autoAlignLayers", json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    // The failure is the dialog's answer, not an `Err` of the confirm.
    let _ = crate::dialogs::confirm(app, &ctx, id);
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn no_match_from_the_menu_opens_an_alert_and_changes_nothing() {
    let mut app = app();
    let before = history(&app);
    auto_align_from_the_menu(&mut app);
    assert_eq!(super::alerts::alerts(&app).len(), 1, "an alert opens");
    assert_eq!(history(&app), before, "the document is unchanged");
    assert!(!crate::notices::shown_titled(&app, photocraft_engine::photo_cmds::NO_DETAIL), "the alert replaces the notice");
}

#[test]
fn after_dont_show_again_the_status_bar_says_it_alone() {
    let mut app = app();
    super::alerts::remember_skip(&mut app, AUTO_ALIGN_NO_MATCH);
    auto_align_from_the_menu(&mut app);
    assert!(super::alerts::alerts(&app).is_empty(), "no alert once ticked");
    assert_eq!(app.ui.status, photocraft_engine::photo_cmds::NO_DETAIL);
    assert!(!crate::notices::shown_titled(&app, photocraft_engine::photo_cmds::NO_DETAIL), "the status bar only, no notice");
}

#[test]
fn engine_callers_still_get_the_error() {
    let mut app = app();
    assert_eq!(app.run("edit.autoAlignLayers", json!({})).unwrap_err(), photocraft_engine::photo_cmds::NO_DETAIL);
}
