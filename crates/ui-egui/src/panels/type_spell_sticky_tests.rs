//! #14183: Check All Layers keeps the state Check Spelling was last closed with.

use craft::automation::actions::{Action, ActionStep};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

const ALL_LAYERS: &str = "Check All Layers";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A document with the type layer "Teh cat", under the dialog host.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 120, "background": "white"})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh cat"})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    h
}

/// Edit › Check Spelling… as the menu runs it.
fn open(h: &mut H, params: Value) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.checkSpelling", params).unwrap();
    h.run_steps(3);
}

fn ticked(h: &H) -> bool {
    h.state().framework.dialogs.get::<CheckSpellingDialog>().expect("the dialog is open").all_layers_checked()
}

fn click(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn the_next_open_keeps_check_all_layers_as_it_was_closed() {
    let mut h = harness();
    open(&mut h, json!({}));
    assert!(ticked(&h), "the first open is on");
    click(&mut h, ALL_LAYERS);
    assert!(!ticked(&h));
    click(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_none(), "Done closed the dialog");
    open(&mut h, json!({}));
    assert!(!ticked(&h), "unticked as it was closed");
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(ticked(&h), "ticked again as it was closed");
}

#[test]
fn an_explicit_all_layers_wins_over_the_remembered_state() {
    let mut h = harness();
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    open(&mut h, json!({"allLayers": true}));
    assert!(ticked(&h), "the param wins");
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(ticked(&h), "the param's run was closed ticked");
}

#[test]
fn a_played_check_spelling_step_neither_reads_nor_changes_the_remembered_state() {
    let mut h = harness();
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    let mut a = Action::new("Spell", vec![ActionStep::new("edit.checkSpelling", json!({"allLayers": true}))]);
    a.dialogs.insert(0);
    h.state_mut().session.actions.list.push(a).unwrap();
    crate::actions::play(h.state_mut(), &json!({"action": "Spell"})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.type_panels.spell.as_ref().is_some_and(|d| d.resume.is_some()), "the step paused at its dialog");
    assert!(ticked(&h), "the step opens with its own Check All Layers");
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(!ticked(&h), "the menu still opens with the state its own run was closed with");
}

#[test]
fn without_an_active_layer_a_remembered_off_opens_on_as_before() {
    let mut h = harness();
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    h.state_mut().run("select.deselectLayers", json!({})).unwrap();
    open(&mut h, json!({}));
    assert!(h.state().ui.type_panels.spell.is_some(), "the dialog opens");
    assert!(ticked(&h), "off needs a layer to check");
}

#[test]
fn the_menu_reopening_an_open_dialog_keeps_its_state() {
    let mut h = harness();
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    open(&mut h, json!({}));
    assert!(!ticked(&h));
}

#[test]
fn recording_with_a_remembered_off_records_it() {
    let mut h = harness();
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    h.state_mut().run("actions.record", json!({"name": "Spell"})).unwrap();
    open(&mut h, json!({}));
    click(&mut h, "Done");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let steps = &h.state().session.actions.list.action(0).unwrap().steps;
    assert_eq!(steps, &vec![ActionStep::new("edit.checkSpelling", json!({"allLayers": false}))]);
}
