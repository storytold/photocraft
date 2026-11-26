//! #14235: Edit › Assign Profile on a document with layers asks first, with OK, Cancel and an unticked "Don't show
//! again". OK opens the Assign Profile dialog, Cancel changes nothing; a flat document and a run with params never ask;
//! a ticked box with OK keeps the question away. #16683: the question shows a warning icon beside its text.

use craft::ui::dialog::{DialogId, StackDialog as _, StandardPrompt};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::alerts;
use super::assign_profile_layers_alert::{AssignProfileAsk, PREF};
use crate::PhotocraftApp;

/// An RGB document; with `layers`, a pixel layer above the Background.
fn app(layers: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    if layers {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    app
}

fn menu(app: &mut PhotocraftApp) -> Value {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "edit.assignProfile", json!({})).unwrap()
}

fn question(app: &PhotocraftApp) -> Option<DialogId> {
    alerts::prompt_tagged::<AssignProfileAsk>(app)
}

fn assign_dialog_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.get::<HostedParamDialog>().is_some()
}

fn answer(app: &mut PhotocraftApp, id: DialogId, choice: &str, skip: bool) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "choice", json!(choice)).unwrap();
    app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(skip)).unwrap();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_document_with_layers_asks_before_the_dialog() {
    let mut app = app(true);
    let before = steps(&app);
    menu(&mut app);
    assert!(question(&app).is_some(), "the question");
    assert!(!assign_dialog_open(&app), "the dialog waits for the answer");
    let fields = app.framework.dialogs.get::<StandardPrompt>().and_then(|p| p.json_fields()).unwrap();
    assert_eq!(fields["skip"], json!(false), "Don't show again starts unticked");
    assert_eq!(steps(&app), before);
}

#[test]
fn the_question_shows_a_warning_icon_beside_its_text() {
    let app = app(true);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    menu(h.state_mut());
    h.run_steps(4);
    let title = h.get_by_role_and_label(Role::Heading, "Assign Profile").rect();
    let message = h.get_by_label_contains("Assign a profile anyway?").rect();
    // The prompt draws the icon unnamed, so it shows in the layout: the title starts at the content's start, the
    // message after the icon on its row.
    assert!(message.left() > title.left() + 8.0, "the message starts after the icon: {message:?} vs title {title:?}");
}

#[test]
fn ok_opens_the_assign_profile_dialog() {
    let mut app = app(true);
    menu(&mut app);
    let id = question(&app).unwrap();
    answer(&mut app, id, "proceed", false);
    assert!(question(&app).is_none() && assign_dialog_open(&app));
}

#[test]
fn cancel_ends_the_command_with_nothing_changed() {
    let mut app = app(true);
    let before = steps(&app);
    menu(&mut app);
    let id = question(&app).unwrap();
    answer(&mut app, id, "cancel", false);
    assert!(question(&app).is_none() && !assign_dialog_open(&app), "no dialog");
    assert_eq!(steps(&app), before, "no history state");
    menu(&mut app);
    assert!(question(&app).is_some(), "a Cancel keeps nothing: it asks again");
}

#[test]
fn a_flat_document_and_a_run_with_params_never_ask() {
    let mut app = app(false);
    menu(&mut app);
    assert!(question(&app).is_none() && assign_dialog_open(&app), "only a Background: the dialog opens at once");
    let mut app = self::app(true);
    app.run("edit.assignProfile", json!({"profile": "none"})).unwrap();
    assert!(question(&app).is_none(), "a scripted run doesn't ask");
}

#[test]
fn dont_show_again_with_ok_keeps_the_question_away() {
    let mut app = app(true);
    menu(&mut app);
    let id = question(&app).unwrap();
    answer(&mut app, id, "proceed", true);
    assert!(alerts::dismissed(&app, PREF));
    let ctx = app.ctx().clone();
    for d in app.framework.dialogs.open_dialogs() {
        let _ = crate::dialogs::cancel(&mut app, &ctx, d.id);
    }
    assert!(!assign_dialog_open(&app));
    menu(&mut app);
    assert!(question(&app).is_none() && assign_dialog_open(&app), "the dialog opens without asking");
}
