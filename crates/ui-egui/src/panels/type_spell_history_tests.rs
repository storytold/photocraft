//! #14177: every change made in one Check Spelling run is one history state named "Spell Check",
//! and one undo restores every changed layer. A run without a change adds no state; headless
//! corrections keep one state each, under the same name.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use super::tests::app_with_text;
use crate::PhotocraftApp;

/// The last word is left unchecked, so Done ends a run in its middle.
const BOTTOM: &str = "Teh cat xjqvzkwp";

/// `BOTTOM` on the bottom type layer and `top` on the active one above it.
fn two_layers(top: &str) -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = app_with_text(BOTTOM);
    let bottom = app.session.active().unwrap().active_layer.unwrap();
    app.run("type.create", json!({"x": 10, "y": 100, "text": top})).unwrap();
    let top = app.session.active().unwrap().active_layer.unwrap();
    (app, bottom, top)
}

/// The type panels' windows of `app`, with Edit > Check Spelling... opened from the menu.
fn open(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    harness(app)
}

/// The type panels' windows of `app`, whose Check Spelling dialog is open.
fn harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    // The fonts are set up after the harness's first frame: the window appears after it.
    let spell = app.ui.type_panels.spell.take();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| super::windows(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.type_panels.spell = spell;
    h.run_steps(3);
    h
}

fn press(h: &mut Harness<'static, PhotocraftApp>, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(3);
}

/// Change To set to `to`, then `button` (Change or Change All).
fn change(h: &mut Harness<'static, PhotocraftApp>, button: &str, to: &str) {
    h.state_mut().ui.type_panels.spell.as_mut().unwrap().change_to = to.into();
    press(h, button);
}

fn text(app: &PhotocraftApp, id: LayerId) -> String {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn rows(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn changes_in_one_run_are_one_spell_check_state_and_one_undo_restores_every_layer() {
    let (app, bottom, top) = two_layers("Teh brwon brwon");
    let mut h = open(app);
    let before = rows(h.state());
    change(&mut h, "Change", "The");
    change(&mut h, "Change All", "brown");
    change(&mut h, "Change", "The");
    press(&mut h, "Done");
    let app = h.state_mut();
    assert!(app.ui.type_panels.spell.is_none());
    assert_eq!((text(app, top), text(app, bottom)), ("The brown brown".into(), "The cat xjqvzkwp".into()));
    assert_eq!(rows(app).len(), before.len() + 1, "{:?}", rows(app));
    assert_eq!(rows(app).last().map(String::as_str), Some("Spell Check"));
    assert_eq!(app.session.active().unwrap().active_layer, Some(top), "the active layer stays");
    assert!(app.session.undo());
    assert_eq!((text(app, top), text(app, bottom)), ("Teh brwon brwon".into(), BOTTOM.into()));
    assert_eq!(rows(app), before);
    assert!(app.session.redo());
    assert_eq!((text(app, top), text(app, bottom)), ("The brown brown".into(), "The cat xjqvzkwp".into()));
}

#[test]
fn a_run_with_ignore_ignore_all_and_add_adds_no_state() {
    let (app, _, _) = two_layers("Teh brwon brwon xjqvzkwp");
    let mut h = open(app);
    let before = rows(h.state());
    let revision = h.state().session.active().unwrap().revision;
    press(&mut h, "Ignore");
    press(&mut h, "Ignore All");
    press(&mut h, "Add");
    press(&mut h, "Done");
    let app = h.state();
    assert!(app.ui.type_panels.spell.is_none());
    assert_eq!(rows(app), before);
    assert_eq!(app.session.active().unwrap().revision, revision, "the document did not change");
}

#[test]
fn each_run_is_a_state_of_its_own() {
    let (app, bottom, top) = two_layers("Teh dog");
    let mut h = open(app);
    let before = rows(h.state()).len();
    change(&mut h, "Change", "The");
    press(&mut h, "Done");
    super::menu(h.state_mut(), "edit.checkSpelling", &json!({})).unwrap().unwrap();
    h.run_steps(3);
    change(&mut h, "Change", "The");
    press(&mut h, "Done");
    let app = h.state_mut();
    assert_eq!(rows(app)[before..], ["Spell Check", "Spell Check"]);
    assert!(app.session.undo());
    assert_eq!((text(app, top), text(app, bottom)), ("The dog".into(), BOTTOM.into()), "one undo takes back only the later run");
}

#[test]
fn an_edit_between_corrections_keeps_its_own_state() {
    let (app, _, top) = two_layers("Teh brwon");
    let mut h = open(app);
    let before = rows(h.state()).len();
    change(&mut h, "Change", "The");
    h.state_mut().run("layer.renameLayer", json!({"layer": top.0, "name": "Renamed"})).unwrap();
    change(&mut h, "Change", "brown");
    press(&mut h, "Done");
    let app = h.state_mut();
    let labels = &rows(app)[before..];
    assert_eq!(labels.len(), 3, "{labels:?}");
    assert_eq!((labels[0].as_str(), labels[2].as_str()), ("Spell Check", "Spell Check"), "{labels:?}");
    assert!(app.session.undo());
    assert_eq!(text(app, top), "The brwon", "the rename is not part of the later correction's state");
    assert_eq!(app.session.active().unwrap().doc.layer(top).unwrap().name, "Renamed");
}

#[test]
fn headless_changes_keep_one_spell_check_state_each() {
    let (mut app, bottom, top) = two_layers("Teh dog");
    let before = rows(&app).len();
    for layer in [top, bottom] {
        app.run("edit.checkSpelling", json!({"action": "change", "layer": layer.0, "start": 0, "end": 3, "word": "Teh", "replace": "The"})).unwrap();
    }
    assert_eq!(rows(&app)[before..], ["Spell Check", "Spell Check"]);
}

/// A playback paused at a Check Spelling step keeps the history it has today (#10965): one
/// `Action:` state when it groups its history, and one state per correction when it does not.
#[test]
fn a_paused_check_spelling_step_keeps_the_playback_history() {
    for single in [true, false] {
        let (mut app, _, top) = two_layers("Teh brwon xjqvzkwp");
        app.run("actions.setHistory", json!({"singleState": single})).unwrap();
        let before = rows(&app).len();
        let mut action = photocraft_engine::actions_cmds::Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false}))]);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
        let paused = crate::actions::play(&mut app, &json!({"action": "Spell"})).unwrap();
        assert_eq!(paused["paused"]["dialog"], super::SPELLING, "{paused}");
        let mut h = harness(app);
        change(&mut h, "Change", "The");
        change(&mut h, "Change", "brown");
        press(&mut h, "Done");
        let app = h.state();
        assert!(app.ui.status.starts_with("Played"), "Done ended the playback: {}", app.ui.status);
        assert_eq!(text(app, top), "The brown xjqvzkwp");
        let labels = rows(app)[before..].to_vec();
        if single {
            assert_eq!(labels, ["Action: Spell"]);
        } else {
            assert_eq!(labels, ["Spell Check", "Spell Check"]);
        }
    }
}

#[test]
fn recording_a_run_records_one_step_and_one_spell_check_state() {
    let (mut app, _, _) = two_layers("Teh brwon brwon");
    app.run("actions.record", json!({"name": "Spell"})).unwrap();
    let mut h = open(app);
    let before = rows(h.state()).len();
    change(&mut h, "Change", "The");
    change(&mut h, "Change All", "brown");
    press(&mut h, "Done");
    let app = h.state_mut();
    app.run("actions.stop", json!({})).unwrap();
    let action = &app.session.actions.list[0];
    assert_eq!(action.steps, vec![("edit.checkSpelling".to_string(), json!({"allLayers": true}))], "no coalesce key in the step");
    assert_eq!(action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
    assert_eq!(rows(app)[before..], ["Spell Check"]);
}

/// #5496: the close box is Done, so it keeps the run's changes as one state. None on macOS.
#[test]
fn the_close_box_keeps_the_run_as_one_state() {
    if !crate::dialogs::CLOSE_BOX {
        return;
    }
    let (app, _, top) = two_layers("Teh brwon");
    let mut h = open(app);
    let before = rows(h.state()).len();
    change(&mut h, "Change", "The");
    change(&mut h, "Change", "brown");
    let x = h.get_by_label("Close").rect();
    h.hover_at(x.center());
    h.drag_at(x.center());
    h.drop_at(x.center());
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.type_panels.spell.is_none(), "the close box closed Check Spelling");
    assert_eq!(text(app, top), "The brown");
    assert_eq!(rows(app)[before..], ["Spell Check"]);
}

#[test]
fn an_undo_in_the_run_drops_that_change_and_leaves_one_state() {
    let (app, _, top) = two_layers("Teh brwon");
    let mut h = open(app);
    let before = rows(h.state());
    change(&mut h, "Change", "The");
    assert!(h.state_mut().session.undo());
    h.run_steps(1);
    change(&mut h, "Change", "brown");
    press(&mut h, "Done");
    let app = h.state_mut();
    assert_eq!(text(app, top), "Teh brown", "the undone change stays undone");
    assert_eq!(rows(app)[..before.len()], before[..]);
    assert_eq!(rows(app)[before.len()..], ["Spell Check"]);
    assert!(!app.session.redo(), "the new correction dropped the undone one");
}
