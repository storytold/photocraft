//! Play a descriptor-free Image Size through its actual dialog and Actions continuation.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action, atn_cmds};
use photocraft_ui_egui::{PhotocraftApp, dialogs};
use serde_json::{Value, json};

/// Import a menu step and append a selection step to expose continuation ordering.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let action = Action::new("Menu", vec![(atn_cmds::MENU_ITEM.into(), json!({"id": "image.imageSize"}))]);
    let (bytes, _) = atn_cmds::export("Menu", &[&action], &[], false).unwrap();
    app.session.actions.list = atn_cmds::import(&bytes).unwrap().actions;
    app.session.actions.list[0].steps.push(("select.all".into(), json!({})));
    app
}

/// The real sizing dialog cancels without edits and resumes only after confirmation.
#[test]
fn image_size_menu_item_opens_real_dialog_cancel_stops_and_confirm_resumes() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(|ui, app: &mut PhotocraftApp| dialogs::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let revision = h.state().session.active().unwrap().revision;
    let definition = h.state().session.actions.list[0].steps.clone();
    let result = h.state_mut().run("actions.play", json!({"action": "Menu"})).unwrap();
    let dialog = result["paused"]["dialog"].as_u64().unwrap();
    h.run_steps(3);
    let fields = &h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields;
    assert_eq!(fields["__command"], "image.imageSize");
    assert_eq!(fields["__sizing"], true, "the populated sizing dialog, not a generic confirmation");
    assert_eq!(fields["width"], 8.0);
    assert_eq!(fields["height"], 6.0);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert!(h.state().session.active().unwrap().doc.selection.is_none());

    let result = h.state_mut().run("actions.play", json!({"action": "Menu"})).unwrap();
    let dialog = result["paused"]["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("width".into(), json!(4));
    fields.insert("height".into(), json!(3));
    fields.insert("resample".into(), json!("nearest"));
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.size, photocraft_doc::Size::new(4, 3));
    assert!(doc.selection.is_some(), "the following step runs only after confirmation");
    assert_eq!(h.state().session.actions.list[0].steps, definition, "menu playback does not record the entered settings into the definition");
}

/// Permit the wrapper while denying the concrete resize command it would open.
fn deny_resize(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" { Err(EngineError::Other("resize denied".into())) } else { Ok(()) }
}

/// Authorization of the actual menu command precedes any dialog or mutation.
#[test]
fn menu_item_gates_its_actual_command_before_opening_a_dialog() {
    let mut app = app();
    let revision = app.session.active().unwrap().revision;
    app.session.authorize = Some(deny_resize);
    let result = app.run("actions.play", json!({"action": "Menu"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["id"], "image.imageSize");
    assert!(result["failed"]["error"].as_str().is_some_and(|s| s.contains("resize denied")), "{result}");
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.session.active().unwrap().doc.selection.is_none());
}

/// Allow opening without settings, but deny the settings entered at confirmation.
fn deny_changed_width(id: &str, p: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" && p.get("width").is_some() { Err(EngineError::Other("entered resize denied".into())) } else { Ok(()) }
}

/// Final dialog settings remain subject to the gate that began playback.
#[test]
fn menu_item_confirmation_keeps_the_playback_gate_after_the_request_ends() {
    let mut app = app();
    let revision = app.session.active().unwrap().revision;
    app.session.authorize = Some(deny_changed_width);
    let result = app.run("actions.play", json!({"action": "Menu"})).unwrap();
    app.session.authorize = None;
    let dialog = result["paused"]["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("width".into(), json!(4));
    let error = dialogs::confirm(&mut app, dialog).unwrap_err();
    assert!(error.contains("entered resize denied"), "{error}");
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.session.active().unwrap().doc.selection.is_none());
}
