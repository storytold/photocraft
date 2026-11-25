//! Real panel controls compose the menu-item commands with consent and retained selection. These trials read the
//! dialog host (`app.framework`), so they live in the crate; the ones that use only the public API stay in
//! `tests/it/actions_panel_menu.rs`.
use photocraft_engine::actions_cmds::ActionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands};
use craft::ui::dialog::DialogId;
use craft::ui::runtime::commands::CommandRef;
use egui_kittest::Harness;
use photocraft_engine::actions_cmds::{Action, ActionStep};
use photocraft_engine::prefs::PlaybackPerformance;
use photocraft_engine::{EngineError, Session, atn_cmds};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};

/// A real dialog host over one selected action in a set.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":16,"height":12})).unwrap();
    let mut a = Action::new("A", vec![]);
    a.set = Some("S".into());
    app.session.actions_mut().list.push(a).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app.ui.actions.panel.expanded.insert("A".into());
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1100.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// One control request through the public channel; the reply is the request's answer.
fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _reply) = ControlRequest::new(method, params);
    match handle(app, ctx, &request) {
        Reply::Done(reply) => reply,
        _ => panic!("{method} answers at once"),
    }
}

/// Fire the panel menu row of `command` as a click on it would.
fn fire(h: &mut Harness<'static, Box<PhotocraftApp>>, command: ActionsPanelCommands) -> Value {
    let ctx = h.ctx.clone();
    let reply = control(h.state_mut(), &ctx, "ui.menu.invoke", json!({"id": CommandRef::from(command).id}));
    h.run_steps(3);
    reply
}

/// The modal the last fired row opened.
fn top_dialog(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("a dialog is open")
}

/// Fire the row of `command` and return the dialog it opened.
fn open(h: &mut Harness<'static, Box<PhotocraftApp>>, command: ActionsPanelCommands) -> DialogId {
    fire(h, command);
    top_dialog(h.state())
}

/// The dialog a play paused on, as the play's result reports it.
fn paused_dialog(played: &Value) -> DialogId {
    serde_json::from_value(played["paused"]["dialog"].clone()).expect("the play paused on a dialog")
}

/// The command key of open dialog `id`.
fn dialog_key(app: &PhotocraftApp, id: DialogId) -> Option<String> {
    app.framework.dialogs.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.key)
}

/// Sets one field of dialog `id` as an agent would.
fn set_field(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, field: &str, value: Value) -> Value {
    let ctx = h.ctx.clone();
    control(h.state_mut(), &ctx, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}))
}

/// Answers OK on dialog `id`; the error is the reason the dialog refused.
fn confirm(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> Result<(), String> {
    let ctx = h.ctx.clone();
    let reply = control(h.state_mut(), &ctx, "ui.dialog.confirm", json!({"dialog": id}));
    h.run_steps(3);
    if reply["ok"] == true { Ok(()) } else { Err(reply["error"].as_str().unwrap_or("refused").to_string()) }
}

/// Cancels dialog `id`.
fn cancel(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) {
    let ctx = h.ctx.clone();
    control(h.state_mut(), &ctx, "ui.dialog.cancel", json!({"dialog": id}));
    h.run_steps(3);
}

/// The steps of action `A`.
fn steps(app: &PhotocraftApp) -> Vec<ActionStep> {
    app.session.actions().list.action(0).unwrap().steps.clone()
}

/// Menu insertion uses an actual chooser and never samples or executes current command settings.
#[test]
fn insert_menu_item_chooser_cancels_or_records_descriptor_free_filter_event() {
    let mut h = harness();
    let revision = h.state().session.active().unwrap().revision;
    let dialog = open(&mut h, ActionsPanelCommands::InsertMenuItem);
    cancel(&mut h, dialog);
    assert!(steps(h.state()).is_empty());
    let dialog = open(&mut h, ActionsPanelCommands::InsertMenuItem);
    assert_eq!(set_field(&mut h, dialog, "id", json!("filter.blur.gaussianBlur"))["ok"], true);
    confirm(&mut h, dialog).unwrap();
    assert_eq!(steps(h.state())[0], ActionStep::new(atn_cmds::MENU_ITEM, json!({"id":"filter.blur.gaussianBlur","eventKind":"name"})));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().ui.actions.panel.selected, Some(ActionSelection::Step(0, 0)));
    let played = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = paused_dialog(&played);
    assert_eq!(dialog_key(h.state(), dialog).as_deref(), Some("filter.blur.gaussianBlur"));
    cancel(&mut h, dialog);
    h.state_mut().run("paint.stroke", json!({"points":[[6,6]],"size":3,"hardness":1,"color":"#000000"})).unwrap();
    let original = photocraft_compose::flatten(&h.state().session.active().unwrap().doc).into_px();
    let mut expected = Session::new();
    expected.add_document((*h.state().session.active().unwrap().doc).clone(), None);
    expected.execute("filter.blur.gaussianBlur", json!({"radius":2})).unwrap();
    let expected = photocraft_compose::flatten(&expected.active().unwrap().doc).into_px();
    assert_ne!(expected, original, "the Gaussian fixture has a visible effect");
    h.state_mut().session.actions_mut().list.insert_step(0, 1, ActionStep::new("select.all", json!({}))).unwrap();
    let paused = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = paused_dialog(&paused);
    assert_eq!(set_field(&mut h, dialog, "radius", json!(2))["ok"], true);
    confirm(&mut h, dialog).unwrap();
    assert_eq!(photocraft_compose::flatten(&h.state().session.active().unwrap().doc).into_px(), expected);
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "confirmation resumes the following step");
}

/// The control protocol distinguishes direct playback from a menu click, even with a step gate.
#[test]
fn control_playback_opens_menu_settings_only_for_menu_invocation() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.run("actions.insertMenuItem", json!({"action":"A","id":"filter.blur.gaussianBlur"})).unwrap();
    app.session.actions_mut().list.insert_step(0, 0, ActionStep::new("select.all", json!({}))).unwrap();
    app.session.actions_mut().list.insert_step(0, 2, ActionStep::new("select.deselect", json!({}))).unwrap();
    app.session.edit_prefs(|prefs| prefs.actions.performance = PlaybackPerformance::StepByStep);

    // Starting at the menu step checks the original dialog bug. Starting one step earlier
    // also verifies Step by Step cannot defer the menu step past the control request.
    for (from, ran) in [(1, 0), (0, 1)] {
        let reply = control(app, &ctx, "engine.execute", json!({"command":"actions.play","params":{"action":"A","from":from}}));
        assert_eq!(reply["ok"], true, "{reply}");
        let played = &reply["result"];
        assert_eq!(played["ran"], ran, "{reply}");
        assert_eq!(played["failed"]["step"], 1, "{reply}");
        assert_eq!(played["failed"]["id"], atn_cmds::MENU_ITEM, "{reply}");
        assert!(played["failed"]["error"].as_str().unwrap().contains("interactive-required"), "{reply}");
        assert!(played.get("paused").is_none(), "{reply}");
        assert!(!app.dialog_open());
    }

    app.services.automation_authorize = Some(|_, _| Ok(()));
    let reply = control(app, &ctx, "ui.menu.invoke", json!({"id":"actions.play","params":{"action":"A","from":1}}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply["result"]["paused"]["dialog"].is_u64(), "menu playback opens Gaussian Blur settings: {reply}");
    let dialog = paused_dialog(&reply["result"]);
    assert_eq!(dialog_key(app, dialog).as_deref(), Some("filter.blur.gaussianBlur"));
    let cancelled = control(app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert!(!app.dialog_open());
    assert!(app.session.active().unwrap().doc.selection.is_some(), "cancel must not run the following step");
}

/// One pass of the preview routes the canvas draws: the filter route, then the adjustment route. The canvas asks them
/// both before it draws, and each checks the retained caller itself.
fn preview_routes(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::canvas::ensure_filter_preview(app, 0, ctx);
    crate::dialogs::adjust_preview::display_doc(app, 0);
}

static MENU_PREVIEW_CHECKS: AtomicUsize = AtomicUsize::new(0);
static MENU_PREVIEW_DENIALS: AtomicUsize = AtomicUsize::new(0);

/// Count actual menu preview checks and deny an edited radius (the step's own radius is 1).
fn deny_menu_radius(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" {
        MENU_PREVIEW_CHECKS.fetch_add(1, Ordering::SeqCst);
        if params.get("radius").and_then(Value::as_f64).is_some_and(|radius| radius > 2.0) {
            MENU_PREVIEW_DENIALS.fetch_add(1, Ordering::SeqCst);
            return Err(EngineError::Other("radius denied".into()));
        }
    }
    Ok(())
}

/// A descriptor-free step retains its playback gate and the identity of its opened editor.
#[test]
fn paused_menu_preview_keeps_the_requesting_gate_and_rejects_retargeting() {
    let mut h = harness();
    h.state_mut().run("actions.insertMenuItem", json!({"action":"A","id":"filter.blur.gaussianBlur"})).unwrap();
    h.state_mut().session.authorize = Some(deny_menu_radius);
    let paused = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = paused_dialog(&paused);
    h.state_mut().session.authorize = None;

    let context = h.ctx.clone();
    let checks = MENU_PREVIEW_CHECKS.load(Ordering::SeqCst);
    preview_routes(h.state_mut(), &context);
    assert!(MENU_PREVIEW_CHECKS.load(Ordering::SeqCst) > checks, "the retained gate authorizes the real preview");

    let revision = h.state().session.active().unwrap().revision;
    let before = steps(h.state());
    set_field(&mut h, dialog, "radius", json!(8));
    let denials = MENU_PREVIEW_DENIALS.load(Ordering::SeqCst);
    preview_routes(h.state_mut(), &context);
    assert!(MENU_PREVIEW_DENIALS.load(Ordering::SeqCst) > denials, "the retained gate denies the edited radius");

    // The stored step now names another command than the editor opened for.
    h.state_mut()
        .session
        .actions_mut()
        .list
        .replace_step(0, 0, ActionStep::new(atn_cmds::MENU_ITEM, json!({"id":"filter.distort.displace","eventKind":"name"})))
        .unwrap();
    let error = confirm(&mut h, dialog).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    // A refused OK keeps the editor open with the reason; cancelling it ends the paused play.
    cancel(&mut h, dialog);
    assert!(!h.state().dialog_open());
    assert!(!h.state().file_dialog_open());
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_ne!(steps(h.state()), before, "only the test's own replacement is in the list");
    assert_eq!(steps(h.state())[0].params["id"], "filter.distort.displace");
}

/// Deny the default while allowing insertion of another mapped menu item.
fn deny_image_size_choice(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" || id == "actions.insertMenuItem" && params["id"] == "image.imageSize" {
        return Err(EngineError::Other("choice denied".into()));
    }
    Ok(())
}

/// The chooser retains its caller and checks only the choice the caller confirms.
#[test]
fn menu_chooser_authorizes_the_selected_command_without_default_probe() {
    let mut h = harness();
    h.state_mut().session.authorize = Some(deny_image_size_choice);
    let dialog = open(&mut h, ActionsPanelCommands::InsertMenuItem);
    set_field(&mut h, dialog, "id", json!("filter.blur.gaussianBlur"));
    h.state_mut().session.authorize = None;
    confirm(&mut h, dialog).unwrap();
    let inserted = steps(h.state());
    assert_eq!(inserted, [ActionStep::new(atn_cmds::MENU_ITEM, json!({"id":"filter.blur.gaussianBlur","eventKind":"name"}))]);

    h.state_mut().session.authorize = Some(deny_image_size_choice);
    let dialog = open(&mut h, ActionsPanelCommands::InsertMenuItem);
    h.state_mut().session.authorize = None;
    let error = confirm(&mut h, dialog).unwrap_err();
    assert!(error.contains("choice denied"), "{error}");
    assert_eq!(steps(h.state()), inserted);
}
