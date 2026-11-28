//! #14177: every change made in one Check Spelling run is one history state named "Spell Check",
//! and one undo restores every changed layer. A run without a change adds no state; headless
//! corrections keep one state each, under the same name. The dialog is the hosted one, driven
//! through its buttons.

use craft::automation::actions::ActionStep;
use craft::ui::dialog::StackDialog as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::actions_cmds::ActionsExt as _;
use serde_json::json;

use super::*;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The last word is left unchecked, so Done ends a run in its middle.
const BOTTOM: &str = "Teh cat xjqvzkwp";

/// `BOTTOM` on the bottom type layer and `top` on the active one above it.
fn two_layers(top: &str) -> (Box<PhotocraftApp>, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 120, "background": "white"})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 60, "text": BOTTOM})).unwrap();
    let bottom = app.session.active().unwrap().active_layer.unwrap();
    app.run("type.create", json!({"x": 10, "y": 100, "text": top})).unwrap();
    let top = app.session.active().unwrap().active_layer.unwrap();
    app.sync_views();
    (app, bottom, top)
}

/// The dialog host of `app`, with Edit > Check Spelling... opened as the menu does.
fn open(mut app: Box<PhotocraftApp>) -> H {
    reopen(&mut app);
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

fn reopen(app: &mut PhotocraftApp) {
    super::super::menu(app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
}

fn press(h: &mut H, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(3);
}

/// Change To set to `to`, then `button` (Change or Change All).
fn change(h: &mut H, button: &str, to: &str) {
    let ctx = h.ctx.clone();
    let id = h.state().framework.dialogs.get::<CheckSpellingDialog>().expect("the dialog").id();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "changeTo", json!(to)).unwrap();
    h.run_steps(1);
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
    reopen(h.state_mut());
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
    let action = app.session.actions().list.action(0).unwrap();
    assert_eq!(action.steps, vec![ActionStep::new("edit.checkSpelling", json!({"allLayers": true}))], "no coalesce key in the step");
    assert_eq!(action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
    assert_eq!(rows(app)[before..], ["Spell Check"]);
}

/// #5496: Escape, like the close box, is Done, so it keeps the run's changes as one state.
#[test]
fn escape_keeps_the_run_as_one_state() {
    let (app, _, top) = two_layers("Teh brwon");
    let mut h = open(app);
    let before = rows(h.state()).len();
    change(&mut h, "Change", "The");
    change(&mut h, "Change", "brown");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    let app = h.state();
    assert!(app.ui.type_panels.spell.is_none(), "Escape closed Check Spelling");
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
