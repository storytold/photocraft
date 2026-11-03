//! Play a descriptor-free Image Size through its actual dialog and Actions continuation.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action, atn_cmds};
use photocraft_ui_egui::{PhotocraftApp, actions, canvas, control, dialogs};
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

/// Direct loads and delayed picker replies cannot replace the action a non-recording dialog owns.
#[test]
fn loading_actions_waits_for_a_paused_menu_step_without_recording() {
    use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply};
    use std::{cell::RefCell, rc::Rc};

    let replacement = Action::new("Menu", vec![("select.deselect".into(), json!({}))]);
    let bytes = atn_cmds::export("Menu", &[&replacement], &[], false).unwrap().0;
    for delayed in [false, true] {
        for confirm in [false, true] {
            let mut app = app();
            let waiting: Rc<RefCell<Option<FileDialogReply>>> = Default::default();
            let reply = waiting.clone();
            app.services.file_dialog = Some(Box::new(move |_, _, answer| *reply.borrow_mut() = Some(answer)));
            let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(
                |ui, app| {
                    actions::panel_menu(app, ui);
                    app.poll_file_dialog(ui.ctx(), None);
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(2);
            if delayed {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Load Actions…").click();
                h.run_steps(3);
                assert!(waiting.borrow().is_some());
            }
            let ctx = h.ctx.clone();
            let app = h.state_mut();
            let paused = app.run("actions.play", json!({"action":"Menu"})).unwrap();
            let dialog = paused["paused"]["dialog"].as_u64().unwrap();
            let before = app.session.actions.clone();
            let document = app.session.active().unwrap().doc.clone();
            let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
            assert!(before.recording.is_none(), "this pause has no suspended recording");
            if delayed {
                waiting.borrow_mut().take().unwrap().send(Some(FileDialogAnswer::Contents("replacement.atn".into(), bytes.clone())));
                app.poll_file_dialog(&ctx, None);
                assert!(app.ui.status_error);
                assert!(app.ui.status.contains("Confirm or cancel"), "{}", app.ui.status);
            } else {
                let error = app.run("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap_err();
                assert!(error.contains("Confirm or cancel"), "{error}");
            }
            for command in ["actions.clearAll", "actions.replace", "actions.reset"] {
                let error = app.run(command, json!({"confirm":true,"mode":"replace"})).unwrap_err();
                assert!(error.contains("Confirm or cancel"), "{command}: {error}");
            }
            assert_eq!(app.session.actions, before);
            assert_eq!(app.session.active().unwrap().doc, document);
            assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields, fields);
            if confirm {
                dialogs::confirm(app, dialog).unwrap();
                assert!(app.session.active().unwrap().doc.selection.is_some(), "the original action's tail resumes");
            } else {
                dialogs::cancel(app, dialog).unwrap();
                assert!(app.session.active().unwrap().doc.selection.is_none());
            }
            app.run("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap();
            assert_eq!(app.session.actions.list[0].steps, replacement.steps, "loading works after the pause ends");
        }
    }
}

/// The real sizing dialog cancels without edits and resumes only after confirmation.
#[test]
fn image_size_menu_item_opens_real_dialog_cancel_stops_and_confirm_resumes() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(|ui, app: &mut PhotocraftApp| dialogs::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let revision = h.state().session.active().unwrap().revision;
    let definition = h.state().session.actions.list[0].steps.clone();
    assert!(definition[0].1.get("_atnEvent").is_some(), "the imported definition keeps save-only metadata");
    let result = h.state_mut().run("actions.play", json!({"action": "Menu"})).unwrap();
    let dialog = result["paused"]["dialog"].as_u64().unwrap();
    h.run_steps(3);
    let fields = &h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields;
    assert_eq!(fields["__command"], "image.imageSize");
    assert_eq!(fields["__sizing"], true, "the populated sizing dialog, not a generic confirmation");
    assert!(fields.get("_atnEvent").is_none(), "save-only metadata never enters the command editor");
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
    assert!(h.state().session.journal.iter().all(|(_, p)| p.get("_atnEvent").is_none()), "save-only metadata stays out of executed commands");
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

/// A retained policy does not itself make native playback programmatic.
fn permit(_: &str, _: &Value) -> photocraft_engine::Result<()> {
    Ok(())
}

/// Real engine.execute requests refuse unconfigured menu intent even without an installed gate.
#[test]
fn control_menu_intent_requires_interaction_with_default_and_gated_services() {
    for gated in [false, true] {
        let mut app = app();
        app.background_jobs = false;
        if gated {
            app.services.automation_authorize = Some(permit);
        }
        let revision = app.session.active().unwrap().revision;
        let ctx = egui::Context::default();
        let (request, _) = control::ControlRequest::new("engine.execute", json!({"command":"actions.play","params":{"action":"Menu"}}));
        let control::Outcome::Done(response) = control::handle(&mut app, &ctx, &request) else { panic!("immediate action response") };
        assert_eq!(response["ok"], true, "{response}");
        let result = &response["result"];
        assert!(result["failed"]["error"].as_str().is_some_and(|e| e.contains("interactive-required")), "{response}");
        assert!(result.get("paused").is_none());
        assert!(app.ui.dialogs.is_empty());
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.session.authorize = Some(permit);
        let paused = app.run("actions.play", json!({"action":"Menu"})).unwrap();
        dialogs::cancel(&mut app, paused["paused"]["dialog"].as_u64().unwrap()).unwrap();
    }
}

/// Recorded settings remain executable by the actual headless request path.
#[test]
fn control_recorded_settings_ignore_the_native_dialog_toggle() {
    let mut app = app();
    app.background_jobs = false;
    app.session.actions.list[0].steps[0] = ("image.imageSize".into(), json!({"width":4,"height":3,"resample":"nearest"}));
    app.session.actions.list[0].dialogs.insert(0);
    let (request, _) = control::ControlRequest::new("engine.execute", json!({"command":"actions.play","params":{"action":"Menu"}}));
    let control::Outcome::Done(result) = control::handle(&mut app, &egui::Context::default(), &request) else { panic!("immediate action response") };
    assert_eq!(result["result"]["ran"], 2, "{result}");
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(4, 3));
}

/// Window Arrange's real cell activation must not retarget a paused Image Size confirmation.
#[test]
fn arranged_document_switch_cannot_retarget_paused_menu_confirmation() {
    let mut app = app();
    app.background_jobs = false;
    app.run("file.new", json!({"width":20,"height":10})).unwrap();
    app.session.set_active(0);
    app.ui.view.arrange = "twoUpVertical".into();
    app.ui.tool = photocraft_ui_egui::state::Tool::Hand;
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // The harness draws once before setup_context installs the app's named fonts.
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            canvas::document_area(app, ui);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    let paused = h.state_mut().run("actions.play", json!({"action":"Menu"})).unwrap();
    let dialog = paused["paused"]["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("width".into(), json!(4));
    fields.insert("height".into(), json!(3));
    let before: Vec<_> = h.state().session.documents().iter().map(|s| (s.doc.id, s.doc.size, s.revision)).collect();
    let pos = egui::pos2(1050.0, 850.0);
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(h.state().session.active_index(), Some(1), "the real arranged-cell producer activated the other document");
    assert!(dialogs::confirm(h.state_mut(), dialog).is_err());
    assert_eq!(h.state().session.documents().iter().map(|s| (s.doc.id, s.doc.size, s.revision)).collect::<Vec<_>>(), before);
    assert!(h.state().session.documents().iter().all(|s| s.doc.selection.is_none()));
    h.state_mut().session.set_active(0);
    dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.documents()[0].doc.size, photocraft_doc::Size::new(4, 3));
    assert!(h.state().session.documents()[0].doc.selection.is_some());
}

/// A stale resume cannot run its following selection on another document; later intentional New remains valid.
#[test]
fn paused_resume_checks_owner_before_later_document_changing_steps() {
    let mut app = app();
    app.background_jobs = false;
    app.run("file.new", json!({"width":20,"height":10})).unwrap();
    app.session.set_active(0);
    app.session.actions.list[0].steps.insert(1, ("file.new".into(), json!({"width":3,"height":2})));
    let paused = app.run("actions.play", json!({"action":"Menu"})).unwrap();
    let dialog = paused["paused"]["dialog"].as_u64().unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    app.session.set_active(1);
    assert!(actions::resume(&mut app, &fields).is_err());
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.session.active().unwrap().doc.selection.is_none());
    app.session.set_active(0);
    dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(3, 2));
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

static PREVIEW_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Count actual preview evaluations while retaining permission for the originating document.
fn count_preview(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" {
        PREVIEW_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// A paused filter preview cannot evaluate its parameters on a different active document.
#[test]
fn paused_filter_preview_stays_with_its_document() {
    use std::sync::atomic::Ordering;
    let mut app = app();
    app.background_jobs = false;
    app.run("file.new", json!({"width":20,"height":10})).unwrap();
    app.session.set_active(0);
    app.session.actions.list[0].steps[0].1["id"] = json!("filter.blur.gaussianBlur");
    app.session.authorize = Some(count_preview);
    let paused = app.run("actions.play", json!({"action":"Menu"})).unwrap();
    let dialog = paused["paused"]["dialog"].as_u64().unwrap();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    PREVIEW_CALLS.store(0, Ordering::SeqCst);
    assert!(canvas::ensure_texture(&mut app, &ctx, 0, None).is_some());
    assert!(PREVIEW_CALLS.load(Ordering::SeqCst) > 0, "same-owner preview evaluated");
    app.session.set_active(1);
    PREVIEW_CALLS.store(0, Ordering::SeqCst);
    canvas::ensure_texture(&mut app, &ctx, 1, None);
    assert_eq!(PREVIEW_CALLS.load(Ordering::SeqCst), 0, "foreign document must not evaluate the paused filter");
    assert!(dialogs::confirm(&mut app, dialog).is_err());
}
