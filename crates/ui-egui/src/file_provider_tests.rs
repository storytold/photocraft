//! Provider transport and native file UI share the real Save and close-command paths.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};

use crate::file_providers::{self, Host};
use crate::{PhotocraftApp, Services, menus};
use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::StackDialog;
use craft::ui::patterns::form_dialogs::HostedFormatOptions;

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

#[cfg(all(not(target_arch = "wasm32"), target_pointer_width = "64"))]
use crate::psd_open_tests::recovery_psd;

fn app(connected: bool) -> (Box<PhotocraftApp>, Host, Written) {
    app_with_size(connected, 2, 2)
}

fn app_with_size(connected: bool, width: u32, height: u32) -> (Box<PhotocraftApp>, Host, Written) {
    let provider = Host::default();
    provider.set_session("test-provider-instance-0001".into()).unwrap();
    if connected {
        provider.connect("test-storage".into(), "Example Storage".into(), "account-a".into()).unwrap();
    }
    let written: Written = Default::default();
    let writes = written.clone();
    let services = Services {
        file_provider: Some(provider.clone()),
        automation_command: Some(Box::new(|id, _| {
            if matches!(id, "file.save" | "file.close" | "file.closeAll") { Ok(()) } else { Err(format!("unexpected automation command: {id}")) }
        })),
        export: Some(Box::new(|_, _, _| Ok((vec![10, 20, 30], Vec::new())))),
        write: Some(Box::new(move |name, bytes| {
            writes.borrow_mut().push((name.into(), bytes.to_vec()));
            Ok(())
        })),
        files: Some(std::sync::Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["Drawing.psd".into()])]))),
        write_unconfirmed: true,
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": width, "height": height, "name": "Drawing"})).unwrap();
    let document = app.session.active_mut().unwrap();
    document.revision = 7;
    document.saved_revision = 2;
    (app, provider, written)
}

fn event(provider: &Host, kind: &str) -> Value {
    provider.take_events().into_iter().find(|event| event.metadata["type"] == kind).unwrap().metadata
}

fn resolve_save(app: &mut PhotocraftApp, provider: &Host, _ctx: &egui::Context) -> String {
    app.poll_file_dialog();
    let dialog = event(provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.psd").unwrap();
    app.poll_file_dialog();
    event(provider, "save")["requestId"].as_str().unwrap().into()
}

fn start_save(app: &mut PhotocraftApp, provider: &Host, ctx: &egui::Context, command: &str) -> String {
    menus::invoke(app, ctx, command, json!({})).unwrap();
    resolve_save(app, provider, ctx)
}

/// One real canvas-editor input frame, with provider delivery in logic before the canvas takes
/// its input and the save continuation after it. Reuse the canvas driver's context and clock.
pub(crate) fn provider_editor_frame(app: &mut PhotocraftApp, events: Vec<egui::Event>, mut before_canvas: impl FnMut(&PhotocraftApp)) {
    let ctx = app.ctx().clone();
    let mut frames = app.canvas_tools.frames.take().expect("warm up the canvas editor before sending input");
    app.framework_before_input(&ctx);
    frames.frame_without_input(events, |ui| {
        file_providers::tick(app, ui.ctx());
        before_canvas(app);
        crate::shortcut_dispatch::record_view_commands(app);
        crate::shortcuts::handle(app, &ui.ctx().clone());
        egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        crate::dialogs::run(app, ui);
        crate::shortcuts::escape_keys(app, &ui.ctx().clone());
        crate::discard_ui::finish_provider_save(app, ui.ctx());
    });
    app.canvas_tools.frames = Some(frames);
}

#[test]
fn no_registered_provider_keeps_the_local_writer_and_menu() {
    let (mut app, provider, written) = app(false);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), 1);
    assert_eq!(app.session.active().unwrap().saved_revision, 2, "a browser download has no durable acknowledgement");
    assert!(!app.saving());
    assert!(provider.take_events().is_empty());
    assert!(!menus::base_rows(&app).iter().any(|item| item.command.id.starts_with("file.openProviderRecent.")));
}

#[test]
fn disabled_provider_keeps_new_saves_local() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    provider.set_enabled(false).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), 1);
    assert!(provider.take_events().is_empty());
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
}

#[test]
fn denied_control_save_keeps_the_close_prompt_without_a_provider_request() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    app.services.automation_command = Some(Box::new(|_, _| Err("automation Save denied".into())));

    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert_eq!(crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap_err(), "automation Save denied");
    app.poll_file_dialog();
    assert!(crate::discard_ui::pending_command(&app).is_some());
    assert!(app.session.active().unwrap().is_dirty());
    assert!(!provider.wants_dialog());
    assert!(provider.take_events().is_empty());
    assert!(written.borrow().is_empty());
}

/// The admitted capture must still belong to the first chooser before any durable ACK.
fn assert_queued_save_capture_is_owned_by_first_document(
    app: &PhotocraftApp,
    provider: &Host,
    written: &Written,
    capture: &file_providers::Event,
    first: photocraft_doc::DocId,
    second: photocraft_doc::DocId,
) {
    assert_eq!(capture.metadata["type"], "save");
    assert_eq!(capture.metadata["documentId"], first.0.to_string());
    assert_eq!(capture.metadata["revision"], "7");
    assert_eq!(capture.metadata["filename"], "first.psd");
    assert_eq!(capture.metadata["destination"], "provider");
    assert_eq!(capture.bytes.as_deref(), Some([10, 20, 30].as_slice()));
    assert!(provider.pending_for(first));
    assert!(!provider.pending_for(second));
    assert_eq!(app.session.documents().iter().find(|state| state.doc.id == first).unwrap().saved_revision, 2);
    assert!(written.borrow().is_empty());
}

/// Only the first capture's checkpoint may change when its provider ACK is consumed.
fn assert_queued_save_ack_preserves_other_document(
    app: &PhotocraftApp,
    provider: &Host,
    written: &Written,
    first: photocraft_doc::DocId,
    second: photocraft_doc::DocId,
    second_document: &std::sync::Arc<photocraft_doc::Document>,
) {
    let saved = app.session.documents().iter().find(|state| state.doc.id == first).unwrap();
    assert_eq!(saved.revision, 7);
    assert_eq!(saved.saved_revision, 7);
    assert!(!saved.is_dirty());
    assert_eq!(saved.doc.name, "first.psd");
    assert!(saved.path.is_none());
    assert!(provider.bound(first));
    assert!(!provider.bound(second));
    assert!(!provider.pending_for(first));
    assert!(!provider.pending_for(second));
    let selected = app.session.active().unwrap();
    assert_eq!(selected.doc.id, second);
    assert!(std::sync::Arc::ptr_eq(&selected.doc, second_document));
    assert_eq!(selected.revision, 11);
    assert_eq!(selected.saved_revision, 3);
    assert!(selected.is_dirty());
    assert!(selected.path.is_none());
    assert!(app.ui.recent_files.is_empty());
    assert!(written.borrow().is_empty());
    assert!(!app.file_dialog_open());
    assert!(!app.saving());
}

#[test]
fn a_rejected_queued_save_cannot_retarget_the_first_provider_dialog() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    let first = app.session.active().unwrap().doc.id;
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    let initial = provider.take_events();
    assert_eq!(initial.len(), 1, "the native picker starts its provider request immediately");
    let dialog = initial[0].metadata.clone();
    assert_eq!(dialog["type"], "saveDialog");

    // The second request arrives before polling the first chooser's answer.
    app.run("file.new", json!({"width": 2, "height": 2, "name": "Other"})).unwrap();
    let second = app.session.active().unwrap().doc.id;
    assert_ne!(first, second);
    app.session.active_mut().unwrap().revision = 11;
    app.session.active_mut().unwrap().saved_revision = 3;
    let second_document = app.session.active().unwrap().doc.clone();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap_err(), "a file dialog is already open");
    assert!(app.file_dialog_open());
    assert!(provider.take_events().is_empty());
    assert!(written.borrow().is_empty());

    app.poll_file_dialog();
    assert!(provider.take_events().is_empty(), "polling cannot start a replacement chooser");
    assert_eq!(dialog["documentId"], first.0.to_string(), "the queued chooser must keep the first document's owner");
    assert_eq!(dialog["intent"], "saveAs");
    assert!(provider.pending_for(first));
    assert!(!provider.pending_for(second));
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "first.psd").unwrap();
    app.poll_file_dialog();
    let events = provider.take_events();
    assert_eq!(events.len(), 1);
    let capture = events.into_iter().next().unwrap();
    assert_queued_save_capture_is_owned_by_first_document(&app, &provider, &written, &capture, first, second);

    provider.result(capture.metadata["requestId"].as_str().unwrap().into(), "saved".into(), "resource-first".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert_queued_save_ack_preserves_other_document(&app, &provider, &written, first, second, &second_document);
}

#[test]
fn provider_ack_owns_its_revision_without_becoming_a_local_path() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    app.push_recent("/device/drawing.psd");
    let document = app.session.active().unwrap().doc.id;
    let request = start_save(&mut app, &provider, &ctx, "file.saveAs");
    assert!(written.borrow().is_empty());
    assert!(app.saving());
    app.session.active_mut().unwrap().revision = 8;
    app.run("file.new", json!({"width": 2, "height": 2, "name": "Other"})).unwrap();
    app.session.active_mut().unwrap().path = Some("/device/drawing.psd".into());
    let selected = app.session.active().unwrap().doc.id;
    let document_names = app.session.documents_generation();
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert_ne!(app.session.documents_generation(), document_names);
    let saved = app.session.documents().iter().find(|state| state.doc.id == document).unwrap();
    assert_eq!(saved.saved_revision, 7);
    assert!(saved.is_dirty());
    assert_eq!(saved.path, None);
    assert_eq!(saved.doc.name, "drawing.psd");
    assert_eq!(app.session.active().unwrap().doc.id, selected);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/device/drawing.psd"));
    assert_eq!(app.ui.recent_files.as_slice(), ["/device/drawing.psd"]);
    assert!(provider.bound(document));
}

#[test]
fn save_a_copy_does_not_use_the_provider_or_mark_the_editable_saved() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), 1);
    assert!(provider.take_events().is_empty());
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(app.session.active().unwrap().path.is_none());
}

fn assert_retry_chooser_preserves_intent(app: &mut PhotocraftApp, provider: &Host, ctx: &egui::Context, command: &str, document_id: &str) {
    menus::invoke(app, ctx, command, json!({})).unwrap();
    app.poll_file_dialog();
    let retry = event(provider, "saveDialog");
    assert_eq!(retry["documentId"].as_str(), Some(document_id));
    assert_eq!(retry["intent"], if command == "file.save" { "save" } else { "saveAs" });
    provider.resolve_dialog(retry["requestId"].as_str().unwrap(), "cancel", "").unwrap();
    app.poll_file_dialog();
}

fn assert_refused_pending_save_keeps_local_copy(pending_command: &str, refused_command: &str) {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    let request = start_save(&mut app, &provider, &ctx, pending_command);
    let state = app.session.active().unwrap();
    let document = state.doc.id;
    let before = (document, state.revision, state.saved_revision, state.is_dirty(), state.path.clone(), state.doc.name.clone());
    let journal = app.session.journal.clone();
    let status = (app.ui.status.clone(), app.ui.status_error);
    assert!(provider.pending_for(document) && app.saving());
    assert!(!app.file_dialog_open());
    assert!(written.borrow().is_empty());
    // begin consumed the admitted capture's intent. Observe the same routing decision
    // pick_save uses, without introducing another chooser or replacing that capture.
    provider.prepare_dialog(Some(document));
    assert!(!provider.wants_dialog());
    file_providers::capture(&mut app);
    let pending = provider.snapshot();
    let error = menus::invoke(&mut app, &ctx, refused_command, json!({})).unwrap_err();
    assert_eq!(error, "Wait for the pending save to finish before saving again");
    file_providers::capture(&mut app);
    assert!(!provider.wants_dialog(), "a refused save must not install an ownerless filename-chooser intent");
    assert_eq!(provider.snapshot(), pending, "the original request and document still own the pending capture");
    let state = app.session.active().unwrap();
    assert_eq!((state.doc.id, state.revision, state.saved_revision, state.is_dirty(), state.path.clone(), state.doc.name.clone()), before);
    assert_eq!(app.session.journal, journal);
    assert_eq!((app.ui.status.clone(), app.ui.status_error), status);
    assert!(provider.pending_for(document) && app.saving());
    assert!(!app.file_dialog_open() && !app.save_options_open());
    assert!(provider.take_events().is_empty());
    assert!(written.borrow().is_empty());

    // A failed upload keeps the exact immutable capture available for retry.
    provider.result(request.clone(), "failed".into(), String::new()).unwrap();
    assert!(provider.pending_for(document));
    file_providers::tick(&mut app, &ctx);
    assert!(provider.pending_for(document) && app.saving());
    assert!(!provider.wants_dialog());
    // Explicit cancellation retires that original capture without acknowledging persistence.
    provider.result(request, "cancel".into(), String::new()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert!(!provider.pending_for(document) && !app.saving());
    assert!(!provider.wants_dialog());
    let state = app.session.active().unwrap();
    assert_eq!((state.doc.id, state.revision, state.saved_revision, state.is_dirty(), state.path.clone(), state.doc.name.clone()), before);
    // The normal copy entry point clears idle intents itself; preserve its actual local route.
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), 1);
    assert_eq!(written.borrow()[0].1, [10, 20, 30]);
    assert!(provider.take_events().is_empty());
    let state = app.session.active().unwrap();
    assert_eq!((state.doc.id, state.revision, state.saved_revision, state.is_dirty(), state.path.clone(), state.doc.name.clone()), before);
    assert!(!provider.pending_for(document));

    // Once idle, the same Save/Save As entry point still records its intended route.
    let document_id = document.0.to_string();
    assert_retry_chooser_preserves_intent(&mut app, &provider, &ctx, refused_command, &document_id);
    assert!(!provider.pending_for(document) && !app.saving());
    assert_eq!(written.borrow().len(), 1);
}

#[test]
fn refused_pending_saves_leave_no_new_intent_and_keep_local_copies_local() {
    for pending_command in ["file.save", "file.saveAs"] {
        for refused_command in ["file.save", "file.saveAs"] {
            assert_refused_pending_save_keeps_local_copy(pending_command, refused_command);
        }
    }
}

#[test]
fn save_before_close_waits_for_ack_and_for_any_newer_edits() {
    for edit_after_capture in [false, true] {
        let (mut app, provider, _) = app(true);
        let ctx = app.ctx().clone();
        menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
        crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
        let request = resolve_save(&mut app, &provider, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.saving());
        assert!(app.has_unsaved_work());
        assert!(crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").is_err());
        if edit_after_capture {
            app.session.active_mut().unwrap().revision = 8;
        }
        provider.result(request, "saved".into(), "resource-one".into()).unwrap();
        file_providers::tick(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1, "logic cannot close before the UI pass consumes its input");
        crate::discard_ui::finish_provider_save(&mut app, &ctx);
        assert!(!app.saving());
        if edit_after_capture {
            assert_eq!(app.session.documents().len(), 1);
            assert!(app.session.active().unwrap().is_dirty());
            assert!(crate::discard_ui::pending_command(&app).is_some());
        } else {
            assert!(app.session.documents().is_empty());
        }
    }
}

#[test]
fn provider_ack_waits_for_raw_type_input_from_the_same_frame_before_closing() {
    let (mut app, provider, _) = app_with_size(true, 400, 200);
    let ctx = app.ctx().clone();
    let text = app.run("type.create", json!({"text":"Before", "size":12, "x":20, "y":80})).unwrap()["layer"].as_u64().unwrap();
    crate::type_tool::edit_active(&mut app, &ctx).unwrap();
    let edit = app.ui.text_edit.as_mut().unwrap();
    edit.anchor = edit.caret;
    // The real canvas editor must have shown and taken focus before the ACK/input frame.
    crate::canvas::testing::frame(&mut app);
    crate::canvas::testing::frame(&mut app);
    let doc = app.session.active().unwrap().doc.id;
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let request = resolve_save(&mut app, &provider, &ctx);
    let captured = app.session.active().unwrap().revision;
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();

    let mut observed_logic = false;
    provider_editor_frame(&mut app, vec![egui::Event::Text(" after".into())], |app| {
        if !observed_logic {
            // Logic applies the ACK before this same frame's shared canvas editor types.
            assert_eq!(app.session.active().unwrap().doc.id, doc);
            assert_eq!(app.session.active().unwrap().revision, captured);
            assert_eq!(app.session.active().unwrap().saved_revision, captured);
            assert!(app.has_unsaved_work(), "the parked close remains guarded until UI settles");
            observed_logic = true;
        }
    });
    assert!(observed_logic);
    let state = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Text(layer) = &state.doc.layer(photocraft_doc::LayerId(text)).unwrap().content else { panic!("text layer") };
    assert_eq!(layer.text, "Before after");
    assert_eq!(state.saved_revision, captured);
    assert_ne!(state.revision, captured);
    assert!(state.is_dirty());
    assert!(crate::discard_ui::pending_command(&app).is_some());
}

#[test]
fn provider_ack_with_only_pointer_input_closes_after_a_settled_ui_pass() {
    let (mut app, provider, _) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let request = resolve_save(&mut app, &provider, &ctx);
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();
    let raw = egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0))], ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| {
        file_providers::tick(&mut app, ui.ctx());
        crate::discard_ui::finish_provider_save(&mut app, ui.ctx());
    });
    output.textures_delta.clear();
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.has_unsaved_work());
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| crate::discard_ui::finish_provider_save(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert!(app.session.documents().is_empty());
    assert!(!app.has_unsaved_work());
}

#[test]
fn provider_ack_does_not_discard_an_uncommitted_preview_or_its_later_commit() {
    let (mut app, provider, _) = app(true);
    let ctx = app.ctx().clone();
    let layer = app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let request = resolve_save(&mut app, &provider, &ctx);
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();
    app.live_adjust = Some((photocraft_doc::LayerId(layer), json!({"preview":"not committed"})));
    file_providers::tick(&mut app, &ctx);
    crate::discard_ui::finish_provider_save(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.has_unsaved_work());
    assert!(crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").is_err());
    app.live_adjust = None;
    app.run("layer.new.layer", json!({"name":"Committed after capture"})).unwrap();
    crate::discard_ui::finish_provider_save(&mut app, &ctx);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(crate::discard_ui::pending_command(&app).is_some());
}

#[test]
fn provider_ack_waits_for_live_gradient_edit_without_pointer_or_dialog() {
    for commit in [false, true] {
        let (mut app, provider, _) = app_with_size(true, 200, 120);
        let ctx = app.ctx().clone();
        app.run("layer.new.layer", json!({})).unwrap();
        menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
        crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
        let request = resolve_save(&mut app, &provider, &ctx);
        let captured = app.session.active().unwrap().revision;
        app.ui.tool = crate::state::Tool::Gradient;
        app.ui.tool_options.gradient_classic = false;
        // Retain a draft in its current owner without a held physical pointer: a latent
        // preview alone must guard the ACK, independently of the canvas's capture state.
        app.canvas_tools.gradient.from = Some([20.0, 20.0]);
        app.canvas_tools.gradient.to = [100.0, 80.0];
        assert!(crate::gradient_ui::editing(&app));
        assert_eq!(app.session.active().unwrap().revision, captured, "the live gradient is only a preview");
        assert!(!ctx.input(|input| input.pointer.any_down()));
        assert!(!crate::canvas::testing::in_gesture(&app));
        assert!(!app.dialog_open());
        provider.result(request, "saved".into(), "resource-one".into()).unwrap();
        file_providers::tick(&mut app, &ctx);
        crate::discard_ui::finish_provider_save(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1, "the ACK must wait for the uncommitted gradient");
        assert!(app.has_unsaved_work());
        if commit {
            crate::canvas::testing::press(&mut app, [20.0, 20.0], egui::Modifiers::NONE);
            crate::canvas::testing::move_to(&mut app, [100.0, 80.0], egui::Modifiers::NONE);
            crate::canvas::testing::release(&mut app, [100.0, 80.0], egui::Modifiers::NONE);
            assert!(!crate::gradient_ui::editing(&app));
            assert_ne!(app.session.active().unwrap().revision, captured);
            crate::discard_ui::finish_provider_save(&mut app, &ctx);
            assert!(app.session.active().unwrap().is_dirty());
            assert!(crate::discard_ui::pending_command(&app).is_some());
        } else {
            app.canvas_tools.gradient = Default::default();
            app.gradient = Default::default();
            assert!(!crate::gradient_ui::editing(&app));
            crate::discard_ui::finish_provider_save(&mut app, &ctx);
            assert!(app.session.documents().is_empty(), "cancelling the preview leaves the acknowledged capture unchanged");
            assert!(!app.has_unsaved_work());
        }
    }
}

#[test]
fn a_deferred_provider_close_cannot_cross_an_account_connection() {
    let (mut app, provider, _) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let request = resolve_save(&mut app, &provider, &ctx);
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    provider.disconnect().unwrap();
    provider.connect("test-storage".into(), "Example Storage".into(), "account-b".into()).unwrap();
    crate::discard_ui::finish_provider_save(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert!(crate::discard_ui::pending_command(&app).is_some());
}

#[test]
fn provider_save_is_not_recorded_as_a_local_action_or_relabels_an_earlier_local_save() {
    for command in ["file.save", "file.saveAs"] {
        let (mut app, provider, written) = app(true);
        let ctx = app.ctx().clone();
        let previous = ("file.saveAs".into(), json!({"path":"drawing.psd"}));
        app.session.journal.push(previous.clone());
        let before = app.session.journal.clone();
        let request = start_save(&mut app, &provider, &ctx, command);
        assert_eq!(app.session.journal, before, "a provider capture cannot become an ordinary local replay step");
        assert!(written.borrow().is_empty());
        provider.result(request, "saved".into(), "one".into()).unwrap();
        file_providers::tick(&mut app, &ctx);
        assert_eq!(app.session.journal.last(), Some(&previous));
    }
}

#[test]
fn provider_saves_with_format_options_are_not_recorded_as_local_actions() {
    for extension in ["tga", "dds", "jpg", "tif"] {
        let (mut app, provider, written) = app(true);
        let ctx = app.ctx().clone();
        if extension == "tif" {
            app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_layered_tiff = true);
            app.run("layer.new.layer", json!({"name":"Second layer"})).unwrap();
        }
        let before = app.session.journal.clone();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog();
        let dialog = event(&provider, "saveDialog");
        let name = format!("drawing.{extension}");
        provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", &name).unwrap();
        app.poll_file_dialog();
        assert!(app.save_options_open(), "{extension} needs its format answer before serialization");
        let mut h = egui_kittest::Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(egui::vec2(800.0, 600.0))
            .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
            .build_ui_state(
                move |ui, app| {
                    crate::dialogs::run(app, ui);
                    file_providers::capture(app);
                },
                app,
            );
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(!h.state().save_options_open());
        assert_eq!(h.state().session.journal, before, "{extension} cannot become a local action step");
        assert!(written.borrow().is_empty());
        assert_eq!(event(&provider, "save")["destination"], "provider");
    }
}

#[test]
fn a_pending_provider_read_does_not_block_an_unrelated_discard_answer() {
    let (mut app, provider, _) = app(true);
    let ctx = app.ctx().clone();
    provider
        .set_resources(
            provider.snapshot()["connectionId"].as_str().unwrap(),
            vec![file_providers::Resource { id: "one".into(), name: "one.psd".into(), updated_at: String::new() }],
        )
        .unwrap();
    let read = provider.request_open("one").unwrap();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert!(crate::discard_ui::pending_command(&app).is_some());
    assert!(provider.busy());
    assert!(!app.saving());
    crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").unwrap();
    assert!(app.session.documents().is_empty());
    provider.cancel_open(read, String::new()).unwrap();
    assert!(provider.busy(), "the terminal still owns its original connection");
    assert!(!provider.saving(), "a cancelled read does not become a save operation");
    provider.publish_events_with(|_, _| Ok::<_, ()>(()), |_| {});
    assert!(!provider.busy());
}

/// A memory refusal cannot both finish the provider request and schedule an unowned local open.
#[cfg(all(not(target_arch = "wasm32"), target_pointer_width = "64"))]
#[test]
fn provider_memory_approval_refusal_cannot_launch_an_unowned_open_job() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    app.services.import = Some(Box::new(|name, bytes, opts| {
        photocraft_io::import_with_options(name, bytes, opts).map(|result| (result.document, result.warnings)).map_err(|error| error.to_string())
    }));
    let original = app.session.active().unwrap().doc.clone();
    let background_jobs = app.background_jobs;
    let bytes = recovery_psd::make_psd(false);
    let gb = 1_u64 << 30;
    let memory = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(memory, || {
            let options = photocraft_engine::jobs::import_options(app.session.prefs());
            assert!(matches!(photocraft_io::import_for_open("memory.psd", &bytes, &options), Err(photocraft_io::IoError::PsdApprovalRequired { .. })));
            provider
                .set_resources(
                    provider.snapshot()["connectionId"].as_str().unwrap(),
                    vec![file_providers::Resource { id: "memory".into(), name: "memory.psd".into(), updated_at: String::new() }],
                )
                .unwrap();
            let request = provider.request_open("memory").unwrap();
            provider.take_events();
            provider.open(request.clone(), "memory.psd".into(), bytes).unwrap();
            file_providers::tick(&mut app, &ctx);
            assert!(app.jobs.opens.is_empty(), "a failed provider request must not leave a local open job");
            assert!(app.session.jobs().is_empty());
            assert!(app.held_memory_prompts.is_empty() && app.held_opens.is_empty() && app.prompt_order.is_empty());
            assert_eq!(app.session.documents().len(), 1);
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
            assert_eq!(app.background_jobs, background_jobs);
            let events = provider.take_events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].metadata["type"], "openFailed");
            assert_eq!(events[0].metadata["requestId"], request);
            assert_eq!(events[0].metadata["resourceId"], "memory");
            assert!(!provider.busy());
            assert!(written.borrow().is_empty());
            assert!(provider.request_open("memory").is_ok(), "the correlated failure permits a fresh retry");
        });
    });
}

#[test]
fn pending_save_as_arms_leave_guard_even_when_the_old_revision_is_clean() {
    let (mut app, provider, _) = app(true);
    let ctx = app.ctx().clone();
    app.session.active_mut().unwrap().saved_revision = 7;
    assert!(!app.has_unsaved_work());
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    assert!(app.has_unsaved_work());
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 1, "a clean Save As still owns an external side effect");
    assert!(crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").is_err());
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "cancel", "").unwrap();
    app.poll_file_dialog();
    assert!(!app.has_unsaved_work());
}

#[test]
fn cancelling_format_options_releases_the_destination_and_reopens_the_chooser() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.tga").unwrap();
    app.poll_file_dialog();
    file_providers::capture(&mut app);
    assert!(matches!(app.parked_save.as_ref().map(|save| &save.format), Some(crate::format_prompts::ParkedFormat::Targa)));
    assert!(app.saving(), "the destination remains reserved while format options are open");
    assert!(menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).is_err());
    assert!(provider.take_events().is_empty(), "Save a Copy cannot reuse the pending save's intent");
    let mut h = egui_kittest::Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(800.0, 600.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_ui_state(
            |ui, app| {
                crate::dialogs::run(app, ui);
                file_providers::capture(app);
            },
            app,
        );
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().parked_save.is_none());
    assert!(!h.state().saving());
    assert!(written.borrow().is_empty());
    assert_eq!(h.state().session.active().unwrap().saved_revision, 2);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    h.state_mut().poll_file_dialog();
    let next = event(&provider, "saveDialog");
    assert_ne!(next["requestId"], dialog["requestId"]);
    assert_eq!(next["intent"], "save");
}

#[test]
fn a_bound_native_project_keeps_its_format_and_resource_on_the_next_save() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.pcraft").unwrap();
    app.poll_file_dialog();
    let save = event(&provider, "save");
    provider.result(save["requestId"].as_str().unwrap().into(), "saved".into(), "resource-native".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert!(app.session.active().unwrap().path.is_none());
    app.session.active_mut().unwrap().revision = 8;
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    let next = event(&provider, "saveDialog");
    assert_eq!(next["filename"], "drawing.pcraft");
    assert_eq!(next["intent"], "save");
    provider.resolve_dialog(next["requestId"].as_str().unwrap(), "provider", "drawing.pcraft").unwrap();
    app.poll_file_dialog();
    let next_save = event(&provider, "save");
    assert_eq!(next_save["filename"], "drawing.pcraft");
    assert_eq!(next_save["resourceId"], "resource-native");
    assert_eq!(next_save["revision"], "8");
    assert!(written.borrow().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.session.active().unwrap().path.is_none());
}

#[test]
fn provider_bound_jpeg_with_transparency_is_offered_as_png() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 4, "height": 4, "background": "white"})).unwrap();
    app.run("layer.new.layerFromBackground", json!({})).unwrap();

    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.jpg").unwrap();
    app.poll_file_dialog();
    crate::format_prompts::confirm(&mut app, &ctx);
    let save = event(&provider, "save");
    provider.result(save["requestId"].as_str().unwrap().into(), "saved".into(), "resource-jpeg".into()).unwrap();
    file_providers::tick(&mut app, &ctx);

    app.run("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(event(&provider, "saveDialog")["filename"], "drawing.png");
    assert!(written.borrow().is_empty());
    assert!(app.session.active().unwrap().path.is_none());
}

// Original synthetic PhotoCraft raster/native-resource fixture, MIT OR Apache-2.0.
#[test]
fn provider_delivery_failure_preserves_native_resources_drafts_and_a_fresh_save_retry() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    app.services.export = Some(Box::new(|doc, name, settings| {
        photocraft_io::export(doc, name, &settings.io_options()).map(|result| (result.bytes, result.warnings)).map_err(|error| error.to_string())
    }));
    let image =
        photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgba, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 70, 90, 110, 255])
            .unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let raster = photocraft_io::import("original.png", &png).unwrap().document;
    {
        let state = app.session.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(state.doc_mut());
        doc.layers = raster.layers;
        doc.metadata.text.push(("Original".into(), "Native delivery fixture".into()));
        doc.metadata.psd_resources.push((4000, "Original embedded bytes".into(), std::sync::Arc::new(png)));
    }
    let doc = app.session.active().unwrap().doc.id;
    let captured = app.session.active().unwrap().doc.clone();
    let journal = app.session.journal.clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "delivery.pcraft").unwrap();
    app.poll_file_dialog();
    let mut request = String::new();
    let mut payload = Vec::new();
    provider.publish_events_with(
        |metadata, bytes| {
            assert_eq!(metadata["type"], "save");
            request = metadata["requestId"].as_str().unwrap().into();
            payload = bytes.unwrap();
            Err("synthetic byte-copy failure")
        },
        |_| {},
    );
    let reopened = photocraft_io::import("delivery.pcraft", &payload).unwrap().document;
    assert_eq!(reopened.layers, captured.layers);
    assert_eq!(reopened.metadata, captured.metadata);
    assert_eq!(reopened.size, captured.size);
    let layer = captured.layers.first().unwrap().id;
    app.live_adjust = Some((layer, json!({"draft":"later, uncommitted"})));
    app.session.active_mut().unwrap().revision = 8;
    file_providers::tick(&mut app, &ctx);
    assert_eq!(app.live_adjust, Some((layer, json!({"draft":"later, uncommitted"}))));
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &captured));
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(!provider.pending_for(doc));
    assert!(!provider.saving());
    assert!(!provider.bound(doc));
    assert!(written.borrow().is_empty());
    assert_eq!(app.session.journal, journal, "delivery recovery must not record a local save destination");
    assert!(provider.result(request.clone(), "saved".into(), "remote".into()).is_err());
    assert_eq!(event(&provider, "saveFailed")["requestId"], request);

    app.live_adjust = None; // Explicit fixture cancellation, never a recovery side effect.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "delivery.pcraft").unwrap();
    app.poll_file_dialog();
    let mut retry = String::new();
    provider.publish_events_with(
        |metadata, bytes| {
            retry = metadata["requestId"].as_str().unwrap().into();
            assert_ne!(retry, request);
            assert_eq!(metadata["revision"], "8");
            let decoded = photocraft_io::import("delivery.pcraft", &bytes.unwrap()).unwrap().document;
            assert_eq!(decoded.layers, captured.layers);
            assert_eq!(decoded.metadata, captured.metadata);
            provider.result(retry.clone(), "saved".into(), "remote".into()).unwrap();
            Ok::<_, ()>(())
        },
        |_| {},
    );
    app.session.active_mut().unwrap().revision = 9;
    file_providers::tick(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().saved_revision, 8);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(provider.bound(doc));
    assert!(written.borrow().is_empty());
    assert!(provider.result(retry, "saved".into(), "remote".into()).is_err());
}

#[test]
fn failed_provider_chooser_delivery_releases_the_actual_dialog_without_a_local_write() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    let source = app.session.active().unwrap().doc.clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    let mut request = String::new();
    assert!(provider.publish_events_with(
        |metadata, _| {
            assert_eq!(metadata["type"], "saveDialog");
            request = metadata["requestId"].as_str().unwrap().into();
            Err("synthetic chooser dispatch failure")
        },
        |_| {}
    ));
    app.poll_file_dialog();
    file_providers::tick(&mut app, &ctx);
    assert!(!app.file_dialog_open());
    assert!(!provider.saving());
    assert!(!provider.busy());
    assert!(app.ui.status_error);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(written.borrow().is_empty());
    assert!(provider.resolve_dialog(&request, "provider", "late.psd").is_err());
    let retry = start_save(&mut app, &provider, &ctx, "file.saveAs");
    assert!(provider.pending_for(app.session.active().unwrap().doc.id));
    provider.result(retry, "cancel".into(), String::new()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert!(!provider.saving());
    assert!(written.borrow().is_empty());
}

#[test]
fn accepted_provider_chooser_callback_survives_a_later_dispatch_exception() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog();
    provider.publish_events_with(
        |metadata, _| {
            provider.resolve_dialog(metadata["requestId"].as_str().unwrap(), "provider", "drawing.psd").unwrap();
            Err("throw after accepted choice")
        },
        |_| {},
    );
    app.poll_file_dialog();
    let capture = event(&provider, "save");
    assert!(provider.pending_for(app.session.active().unwrap().doc.id));
    assert_eq!(capture["destination"], "provider");
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(written.borrow().is_empty());
    provider.result(capture["requestId"].as_str().unwrap().into(), "cancel".into(), String::new()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert!(!provider.saving());
}

#[test]
fn failed_provider_delivery_keeps_save_before_close_retryable_and_preserves_a_later_edit() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    app.poll_file_dialog();
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.psd").unwrap();
    app.poll_file_dialog();
    provider.publish_events_with(|_, _| Err("synthetic close-save delivery failure"), |_| {});
    file_providers::tick(&mut app, &ctx);
    crate::discard_ui::finish_provider_save(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(crate::discard_ui::pending_command(&app).is_some());
    assert!(!app.saving());
    assert_eq!(event(&provider, "saveFailed")["revision"], "7");
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let retry = resolve_save(&mut app, &provider, &ctx);
    app.run("layer.new.layer", json!({"name":"After retry capture"})).unwrap();
    let later = app.session.active().unwrap().doc.clone();
    provider.result(retry, "saved".into(), "remote".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    crate::discard_ui::finish_provider_save(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.layers, later.layers, "normal ACK filename bookkeeping must preserve the later edit");
    assert_eq!(app.session.active().unwrap().saved_revision, 7);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(crate::discard_ui::pending_command(&app).is_some());
    assert!(written.borrow().is_empty());
}

#[test]
fn local_save_as_keeps_upstream_own_name_and_explicit_name_action_paths() {
    for (path, expected) in [("out/Drawing.psd", json!({"folder":"out", "extension":"psd"})), ("out/Renamed.psd", json!({"path":"out/Renamed.psd"}))] {
        let (mut app, provider, written) = app(false);
        app.save_as(Some(path.into())).unwrap();
        assert_eq!(app.session.journal.last(), Some(&("file.saveAs".into(), expected)));
        assert_eq!(written.borrow().last().unwrap().0, path);
        assert!(provider.take_events().is_empty());
    }
}

#[test]
fn local_plain_save_relabels_only_the_step_from_this_save() {
    let (mut app, _, _) = app(false);
    let earlier = ("file.saveAs".into(), json!({"folder":"out", "extension":"psd"}));
    app.session.journal.push(earlier.clone());
    let from = app.session.journal.len();
    app.journal_as_plain_save("out/Drawing.psd", from);
    assert_eq!(app.session.journal.last(), Some(&earlier));
    app.save_as(Some("out/Drawing.psd".into())).unwrap();
    app.journal_as_plain_save("out/Drawing.psd", from);
    assert_eq!(app.session.journal.last(), Some(&("file.save".into(), json!({}))));
    assert_eq!(app.session.journal.get(from - 1), Some(&earlier));
}

#[test]
fn provider_tiff_answer_preserves_native_options_and_copy_checkpoint() {
    for discard_layers in [false, true] {
        let (mut app, provider, written) = app(true);
        let ctx = app.ctx().clone();
        app.run("layer.new.layer", json!({"name":"Second layer"})).unwrap();
        let captured = app.session.active().unwrap().revision;
        let doc = app.session.active().unwrap().doc.id;
        let exports = Rc::new(RefCell::new(Vec::new()));
        let observed = exports.clone();
        app.services.export = Some(Box::new(move |doc, _, settings| {
            observed.borrow_mut().push((doc.id, settings.tiff_compression, settings.tiff_transparency, settings.tiff_pyramid, settings.tiff_layers));
            Ok((vec![10, 20, 30], Vec::new()))
        }));
        let journal = app.session.journal.clone();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog();
        let dialog = event(&provider, "saveDialog");
        provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.tif").unwrap();
        app.poll_file_dialog();
        assert!(matches!(app.parked_save.as_ref().map(|save| &save.format), Some(crate::format_prompts::ParkedFormat::Tiff(_))));
        let state = app.session.active().unwrap();
        assert!(photocraft_engine::file_cmds::tiff_would_write_layers(&state.doc));
        let transparency = state.doc.background().is_none().then_some(false);
        let prompt = app.framework.dialogs.get_mut::<HostedFormatOptions>().unwrap();
        prompt.set_json_field(&ctx, "compression", json!("zip")).unwrap();
        prompt.set_json_field(&ctx, "pyramid", json!(true)).unwrap();
        if transparency.is_some() {
            prompt.set_json_field(&ctx, "transparency", json!(false)).unwrap();
        }
        prompt.set_json_field(&ctx, "discardLayers", json!(discard_layers)).unwrap();
        crate::format_prompts::confirm(&mut app, &ctx);
        file_providers::capture(&mut app);
        assert_eq!(exports.borrow().as_slice(), &[(doc, photocraft_codecs::TiffCompression::Deflate, transparency, true, !discard_layers)]);
        assert_eq!(app.tiff_remembered.compression, photocraft_codecs::TiffCompression::Deflate);
        assert!(app.tiff_remembered.pyramid);
        assert_eq!(app.session.journal, journal);
        assert!(written.borrow().is_empty());
        let save = event(&provider, "save");
        assert_eq!(save["documentId"], doc.0.to_string());
        assert_eq!(save["revision"], captured.to_string());
        app.session.active_mut().unwrap().revision = captured + 1;
        provider.result(save["requestId"].as_str().unwrap().into(), "saved".into(), "tiff-resource".into()).unwrap();
        file_providers::tick(&mut app, &ctx);
        let saved = app.session.active().unwrap();
        assert_eq!(saved.saved_revision, if discard_layers { 2 } else { captured });
        assert!(saved.is_dirty(), "a copy or a later revision cannot be acknowledged as the current editable");
        assert_eq!(provider.bound(doc), !discard_layers);
        assert!(saved.path.is_none());
        assert!(app.ui.recent_files.is_empty());
        assert_eq!(app.session.journal, journal);
    }
}

#[test]
fn provider_capture_bypasses_the_native_save_commit_worker() {
    let (mut app, provider, written) = app(true);
    let ctx = app.ctx().clone();
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_called = called.clone();
    app.services.save_file = Some(std::sync::Arc::new(move |_, _, _, _| {
        worker_called.store(true, std::sync::atomic::Ordering::SeqCst);
        Err("provider capture must not enter the native save worker".into())
    }));
    app.background_jobs = true;
    let request = start_save(&mut app, &provider, &ctx, "file.saveAs");
    assert!(app.jobs.saves.is_empty());
    assert!(app.jobs.last_started.is_none());
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(written.borrow().is_empty());
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    provider.result(request, "saved".into(), "native-job-independent".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().saved_revision, 7);
    assert!(app.jobs.saves.is_empty());
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn provider_save_all_waits_for_each_durable_ack_before_closing_next_document() {
    let (mut app, provider, written) = app(true);
    let first = app.session.active().unwrap().doc.id;
    app.run("file.new", json!({"width": 2, "height": 2, "name": "Second"})).unwrap();
    let second = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().revision = 11;
    app.session.active_mut().unwrap().saved_revision = 3;
    let mut h = egui_kittest::Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(800.0, 600.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "file.closeAll", json!({})).unwrap();
    crate::discard_ui::answer_control(h.state_mut(), &ctx, "saveAll").unwrap();
    let first_save = resolve_save(h.state_mut(), &provider, &ctx);
    assert!(provider.pending_for(first));
    h.run_steps(2);
    assert_eq!(h.state().session.documents().len(), 2);
    assert!(!h.state().file_dialog_open(), "Apply to All cannot start the next chooser before this ACK");
    provider.result(first_save, "saved".into(), "first-resource".into()).unwrap();
    file_providers::tick(h.state_mut(), &ctx);
    assert_eq!(h.state().session.documents().len(), 2, "the ACK still waits for a settled editor pass");
    crate::discard_ui::finish_provider_save(h.state_mut(), &ctx);
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(h.state().session.active().unwrap().doc.id, second);
    h.run_steps(2);
    let second_save = resolve_save(h.state_mut(), &provider, &ctx);
    assert!(provider.pending_for(second));
    assert_eq!(h.state().session.active().unwrap().saved_revision, 3);
    provider.result(second_save, "saved".into(), "second-resource".into()).unwrap();
    file_providers::tick(h.state_mut(), &ctx);
    crate::discard_ui::finish_provider_save(h.state_mut(), &ctx);
    assert!(h.state().session.documents().is_empty());
    assert!(crate::discard_ui::pending_command(h.state()).is_none());
    assert!(h.state().ui.recent_files.is_empty());
    assert!(written.borrow().is_empty());
}
