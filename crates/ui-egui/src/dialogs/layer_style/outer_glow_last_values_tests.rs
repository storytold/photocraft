//! #14945: a new Outer Glow uses the last settings confirmed in Layer Style for this app session.

use craft::imaging::layer_style::{EffectKind, StyleValue};
use craft::theme::Srgba;
use egui_kittest::kittest::Queryable as _;
use photocraft_engine::prefs::SavedPrefsExt;
use serde_json::{Value, json};

use super::tests::{app_with_layer, cancel, draft, edit, first_of, harness};
use super::{DialogId, style_params_json};
use crate::PhotocraftApp;

const COMMAND: &str = "layer.layerStyle.outerGlow";

fn open_glow(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, COMMAND, json!({})).unwrap();
    app.layer_style_target.unwrap().0
}

fn values(app: &PhotocraftApp, id: DialogId) -> Value {
    let glow = first_of(app, id, EffectKind::OuterGlow);
    style_params_json(&draft(app, id).style.effect(glow).unwrap().params)
}

fn set_glow(app: &mut PhotocraftApp, id: DialogId, opacity: f64) {
    let glow = first_of(app, id, EffectKind::OuterGlow);
    edit(app, id, |draft| {
        draft.set_param(glow, "blend", StyleValue::Text("Normal".into()));
        draft.set_param(glow, "color", StyleValue::Color(Srgba::from_rgb_array([0.0; 3])));
        draft.set_param(glow, "opacity", StyleValue::Number(opacity));
        draft.set_param(glow, "size", StyleValue::Number(10.0));
    });
}

fn confirm(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::confirm(app, id).unwrap();
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

fn other_document(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 40, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
}

fn remember_glow(app: &mut PhotocraftApp) -> Value {
    let id = open_glow(app);
    set_glow(app, id, 100.0);
    let remembered = values(app, id);
    confirm(app, id);
    other_document(app);
    remembered
}

#[test]
fn a_new_outer_glow_reuses_the_confirmed_values_on_another_document() {
    let mut app = app_with_layer(32);
    let remembered = remember_glow(&mut app);
    let id = open_glow(&mut app);
    assert_eq!(values(&app, id), remembered);
    confirm(&mut app, id);
    let id = open_glow(&mut app);
    assert_eq!(values(&app, id), remembered, "the seeded glow applies to the layer");
}

#[test]
fn cancelled_edits_and_an_unchanged_scripted_glow_leave_the_last_ok_alone() {
    let mut app = app_with_layer(32);
    let remembered = remember_glow(&mut app);
    let pending = open_glow(&mut app);
    set_glow(&mut app, pending, 40.0);
    cancel(&mut app, pending);

    app.run(COMMAND, json!({"blend": "Screen", "color": "#0000ff", "size": 21, "opacity": 60})).unwrap();
    let scripted = open_glow(&mut app);
    assert_eq!(values(&app, scripted)["color"], "#0000ff", "the existing layer effect supplies its own values");
    confirm(&mut app, scripted);

    other_document(&mut app);
    let next = open_glow(&mut app);
    assert_eq!(values(&app, next), remembered);
}

#[test]
fn editing_an_existing_outer_glow_updates_the_next_new_glow() {
    let mut app = app_with_layer(32);
    remember_glow(&mut app);
    app.run(COMMAND, json!({"size": 21})).unwrap();
    let id = open_glow(&mut app);
    set_glow(&mut app, id, 50.0);
    let edited = values(&app, id);
    confirm(&mut app, id);
    other_document(&mut app);
    let next = open_glow(&mut app);
    assert_eq!(values(&app, next), edited);
}

#[test]
fn reset_uses_factory_or_user_defaults_and_a_fresh_app_starts_at_factory() {
    let mut app = app_with_layer(32);
    let initial = open_glow(&mut app);
    let factory = values(&app, initial);
    cancel(&mut app, initial);
    remember_glow(&mut app);
    let id = open_glow(&mut app);
    let mut h = harness(app);
    h.get_by_label("Reset to Default").click();
    h.run_steps(3);
    assert_eq!(values(h.state(), id), factory);

    let saved_default = json!({"kind": "outerGlow", "params": {"opacity": 18.0}});
    h.state_mut().run("layer.layerStyle.makeDefault", saved_default).unwrap();
    // Reset must replace every remembered field, even when the saved default is partial.
    cancel(h.state_mut(), id);
    let id = open_glow(h.state_mut());
    h.run_steps(3);
    h.get_by_label("Reset to Default").click();
    h.run_steps(3);
    let mut user = factory.clone();
    user["opacity"] = json!(18.0);
    assert_eq!(values(h.state(), id), user);

    // A fresh app opens at factory values even with an existing Make Default entry.
    let saved = h.state().session.prefs_to_json();
    let mut restarted = app_with_layer(32);
    restarted.session.load_prefs_json(&saved).unwrap();
    let id = open_glow(&mut restarted);
    assert_eq!(values(&restarted, id), factory);
    let mut h = harness(restarted);
    h.get_by_label("Reset to Default").click();
    h.run_steps(3);
    assert_eq!(values(h.state(), id), user);
}

#[test]
fn make_default_in_the_current_session_seeds_new_glows() {
    let mut app = app_with_layer(32);
    remember_glow(&mut app);
    let id = open_glow(&mut app);
    set_glow(&mut app, id, 60.0);
    let user = values(&app, id);
    let mut h = harness(app);
    h.get_by_label("Make Default").click();
    h.run_steps(3);
    cancel(h.state_mut(), id);
    let next = open_glow(h.state_mut());
    assert_eq!(values(h.state(), next), user);
}

#[test]
fn disabled_or_refused_glows_do_not_replace_the_last_ok() {
    let mut app = app_with_layer(32);
    let remembered = remember_glow(&mut app);
    let disabled = open_glow(&mut app);
    set_glow(&mut app, disabled, 70.0);
    let glow = first_of(&app, disabled, EffectKind::OuterGlow);
    edit(&mut app, disabled, |draft| draft.toggle(glow));
    confirm(&mut app, disabled);

    other_document(&mut app);
    let refused = open_glow(&mut app);
    set_glow(&mut app, refused, 80.0);
    let glow = first_of(&app, refused, EffectKind::OuterGlow);
    edit(&mut app, refused, |draft| draft.set_param(glow, "fx", StyleValue::Other("{}".into())));
    crate::dialogs::testing::confirm(&mut app, refused).unwrap();
    assert!(app.ui.status_error);

    let next = open_glow(&mut app);
    assert_eq!(values(&app, next), remembered);
}
