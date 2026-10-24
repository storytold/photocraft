use serde_json::json;

use super::*;
use crate::menus::invoke as menu;

/// One 4 × 2 inch document at 100 ppi, with the rulers in `unit`.
fn app(unit: &str) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
    app.sync_views();
    (app, egui::Context::default())
}

fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> u64 {
    menu(app, ctx, "view.newGuide", json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn set(app: &mut PhotocraftApp, d: u64, orientation: &str, position: &str) {
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("orientation".into(), json!(orientation));
    f.insert("position".into(), json!(position));
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.horizontal.clone(), g.vertical.clone())
}

#[test]
fn opens_in_the_ruler_unit_and_takes_photoshop_units() {
    let (mut app, ctx) = app("cm");
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert!(owns(f), "New Guide has its own dialog, not the generic one");
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 cm")));
    // Percent is of the width for a vertical guide.
    set(&mut app, d, "vertical", "50%");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![], vec![200.0]));
    // The last guide opens next time, in the ruler unit (200 px at 100 ppi is 5.08 cm); a bare
    // number is in the ruler unit, as in Photoshop.
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("vertical"), Some("5.08 cm")));
    set(&mut app, d, "horizontal", "2.54");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![100.0], vec![200.0]));
    // Any unit overrides the ruler unit.
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "36 pt");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![100.0, 50.0]);
}

#[test]
fn percent_rulers_open_at_zero_percent() {
    let (mut app, ctx) = app("percent");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], "0 %");
    set(&mut app, d, "horizontal", "25");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![50.0], "25% of the 200 px height");
}

#[test]
fn a_refused_position_reopens_the_dialog_without_a_guide() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "12 furlongs");
    let err = crate::dialogs::confirm(&mut app, d).unwrap_err();
    assert!(err.contains("not a length"), "{err}");
    assert_eq!(guides(&app), (vec![], vec![]));
    let again = app.ui.dialogs.iter().find(|d| owns(&d.fields)).expect("the dialog is open again");
    assert_eq!(again.fields["position"], "12 furlongs", "what was typed is kept");
    assert_eq!(again.fields["orientation"], "vertical");
    assert_eq!(again.fields["__error"].as_str(), Some(err.as_str()));
    // A refused position is not remembered.
    assert!(!app.session.prefs().dialogs.contains_key("view.newGuide"));
}

#[test]
fn remembered_values_that_no_longer_parse_are_ignored() {
    let (mut app, ctx) = app("mm");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"orientation": "diagonal", "positionPx": "far away"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 mm")));
}

fn color_of(app: &PhotocraftApp, vertical: bool, position: f64) -> Option<[u8; 3]> {
    app.session.active().unwrap().doc.guides.color(vertical, position)
}

#[test]
fn guides_store_the_chosen_colour_cyan_by_default() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "cyan");
    set(&mut app, d, "vertical", "10");
    crate::dialogs::confirm(&mut app, d).unwrap();
    // Stored on the guide itself, so a later change of the preference colour leaves it alone.
    assert_eq!(color_of(&app, true, 10.0), Some([0x4a, 0xff, 0xff]));
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "20");
    app.ui.dialog_mut(d).unwrap().fields.insert("color".into(), json!("magenta"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, true, 20.0), Some([0xff, 0x00, 0xff]));
    // Custom… sends the picked colour; the list's choice and the custom colour open next time.
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "magenta");
    set(&mut app, d, "horizontal", "30");
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("color".into(), json!("custom"));
    f.insert("customColor".into(), json!("#abcdef"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, false, 30.0), Some([0xab, 0xcd, 0xef]));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("custom"), Some("#abcdef")));
}

#[test]
fn remembered_colours_that_are_not_on_the_list_are_ignored() {
    let (mut app, ctx) = app("pixels");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"color": "puce", "customColor": "#zzzzzz"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("cyan"), Some("#4affff")));
}

#[test]
fn the_last_position_reopens_in_whatever_the_ruler_unit_is_now() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "1in");
    crate::dialogs::confirm(&mut app, d).unwrap();
    for (unit, shown) in [("pixels", "100 px"), ("inches", "1 in"), ("mm", "25.4 mm"), ("points", "72 pt"), ("picas", "6 pica"), ("percent", "50 %")] {
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
        let d = open(&mut app, &ctx);
        assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], shown, "{unit}");
        crate::dialogs::cancel(&mut app, d).unwrap();
    }
}

// These regressions exercise the ordinary dialog handlers, not a separate model.
fn edit_fixture() -> (PhotocraftApp, u64) {
    let (mut app, _) = app("pixels");
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    crate::guide_select::select(&mut app, true, 0);
    let id = open_edit(&mut app).unwrap();
    set(&mut app, id, "vertical", "30 px");
    (app, id)
}

#[test]
fn draft_cannot_edit_a_guide_in_another_document() {
    let (mut app, id) = edit_fixture();
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![10.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn draft_cannot_edit_the_guide_that_reused_a_deleted_index() {
    let (mut app, id) = edit_fixture();
    app.run("view.deleteGuide", json!({"orientation": "vertical", "index": 0})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn draft_unchanged_confirmation_preserves_redo() {
    let (mut app, id) = edit_fixture();
    crate::dialogs::cancel(&mut app, id).unwrap();
    app.run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 15})).unwrap();
    assert!(app.session.undo());
    crate::guide_select::select(&mut app, true, 0);
    let id = open_edit(&mut app).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(app.session.active().unwrap().history.can_redo());
}

#[test]
fn draft_preview_is_isolated_and_invalid_text_keeps_the_last_good_preview() {
    let (mut app, id) = edit_fixture();
    crate::guide_edit::sync(&mut app);
    let shown = |app: &PhotocraftApp| edit_preview(app, &app.session.active().unwrap().doc).unwrap().vertical.clone();
    assert_eq!(shown(&app), vec![30.0, 20.0]);
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), 2);
    set(&mut app, id, "vertical", "far");
    crate::guide_edit::sync(&mut app);
    assert_eq!(shown(&app), vec![30.0, 20.0]);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    let id = app.ui.dialogs.last().unwrap().id;
    assert!(app.ui.dialog_mut(id).unwrap().fields.contains_key("__error"));
    set(&mut app, id, "vertical", "40 px");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(guides(&app).1, vec![40.0, 20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), 3);
    assert_eq!(app.session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 1);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
}

#[test]
fn draft_reopening_retires_the_previous_dialog() {
    let (mut app, first) = edit_fixture();
    let second = open_edit(&mut app).unwrap();
    assert!(crate::dialogs::confirm(&mut app, first).is_err());
    crate::dialogs::cancel(&mut app, second).unwrap();
    assert!(app.guide_edit.is_none());
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
}

#[test]
fn draft_cannot_commit_after_guides_are_locked() {
    let (mut app, id) = edit_fixture();
    app.ui.extras.lock_guides = true;
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
    assert!(app.guide_edit.is_none());
}
