//! Provider transport and native file UI share the real Save and close-command paths.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};

use crate::file_providers::{self, Host};
use crate::{FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services, menus};

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn app(connected: bool) -> (PhotocraftApp, Host, Written) {
    let provider = Host::default();
    provider.set_session("test-provider-instance-0001".into()).unwrap();
    if connected {
        provider.connect("test-storage".into(), "Example Storage".into(), "account-a".into()).unwrap();
    }
    let written: Written = Default::default();
    let writes = written.clone();
    let dialog_provider = provider.clone();
    let services = Services {
        file_provider: Some(provider.clone()),
        export: Some(Box::new(|_, _, _| Ok((vec![10, 20, 30], Vec::new())))),
        write: Some(Box::new(move |name, bytes| {
            writes.borrow_mut().push((name.into(), bytes.to_vec()));
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _, reply| match request {
            FileDialogRequest::Save { suggested } if dialog_provider.wants_dialog() => dialog_provider.dialog(suggested, reply),
            FileDialogRequest::Save { suggested } => reply.send(Some(FileDialogAnswer::SaveTo(suggested))),
            _ => reply.send(None),
        })),
        write_unconfirmed: true,
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 2, "height": 2, "name": "Drawing"})).unwrap();
    let document = app.session.active_mut().unwrap();
    document.revision = 7;
    document.saved_revision = 2;
    (app, provider, written)
}

fn event(provider: &Host, kind: &str) -> Value {
    provider.take_events().into_iter().find(|event| event.metadata["type"] == kind).unwrap().metadata
}

fn resolve_save(app: &mut PhotocraftApp, provider: &Host, ctx: &egui::Context) -> String {
    app.poll_file_dialog(ctx, None);
    let dialog = event(provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.psd").unwrap();
    app.poll_file_dialog(ctx, None);
    event(provider, "save")["requestId"].as_str().unwrap().into()
}

fn start_save(app: &mut PhotocraftApp, provider: &Host, ctx: &egui::Context, command: &str) -> String {
    menus::invoke(app, ctx, command, json!({})).unwrap();
    resolve_save(app, provider, ctx)
}

#[test]
fn no_registered_provider_keeps_the_local_writer_and_menu() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(false);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 1);
    assert_eq!(app.session.active().unwrap().saved_revision, 2, "a browser download has no durable acknowledgement");
    assert!(!app.saving());
    assert!(provider.take_events().is_empty());
    assert!(!menus::menu_items(&app).iter().any(|item| item.id.starts_with("file.openProviderRecent.")));
}

#[test]
fn disabled_provider_keeps_new_saves_local() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(true);
    provider.set_enabled(false).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 1);
    assert!(provider.take_events().is_empty());
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
}

#[test]
fn provider_ack_owns_its_revision_without_becoming_a_local_path() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(true);
    app.push_recent("/device/drawing.psd");
    let document = app.session.active().unwrap().doc.id;
    let request = start_save(&mut app, &provider, &ctx, "file.saveAs");
    assert!(written.borrow().is_empty());
    assert!(app.saving());
    app.session.active_mut().unwrap().revision = 8;
    app.run("file.new", json!({"width": 2, "height": 2, "name": "Other"})).unwrap();
    app.session.active_mut().unwrap().path = Some("/device/drawing.psd".into());
    let selected = app.session.active().unwrap().doc.id;
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    let saved = app.session.documents().iter().find(|state| state.doc.id == document).unwrap();
    assert_eq!(saved.saved_revision, 7);
    assert!(saved.is_dirty());
    assert_eq!(saved.path, None);
    assert_eq!(saved.doc.name, "drawing.psd");
    assert_eq!(app.session.active().unwrap().doc.id, selected);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/device/drawing.psd"));
    assert_eq!(app.ui.recent_files, ["/device/drawing.psd"]);
    assert!(provider.bound(document));
}

#[test]
fn save_a_copy_does_not_use_the_provider_or_mark_the_editable_saved() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(true);
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 1);
    assert!(provider.take_events().is_empty());
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(app.session.active().unwrap().path.is_none());
}

#[test]
fn save_before_close_waits_for_ack_and_for_any_newer_edits() {
    for edit_after_capture in [false, true] {
        let ctx = egui::Context::default();
        let (mut app, provider, _) = app(true);
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
            assert!(crate::discard_ui::inspect(&app).is_some());
        } else {
            assert!(app.session.documents().is_empty());
        }
    }
}

#[test]
fn provider_ack_waits_for_raw_type_input_from_the_same_frame_before_closing() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let (mut app, provider, _) = app(true);
    let text = app.run("type.create", json!({"text":"Before", "size":12})).unwrap()["layer"].as_u64().unwrap();
    crate::type_tool::edit_active(&mut app).unwrap();
    let edit = app.ui.text_edit.as_mut().unwrap();
    edit.anchor = edit.caret;
    let doc = app.session.active().unwrap().doc.id;
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
    let request = resolve_save(&mut app, &provider, &ctx);
    let captured = app.session.active().unwrap().revision;
    provider.result(request, "saved".into(), "resource-one".into()).unwrap();

    let raw = egui::RawInput { focused: true, events: vec![egui::Event::Text(" after".into())], ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| {
        // eframe runs logic before UI with this same raw input. This is the real Type
        // input handler: its engine edit must not be preempted by the save continuation.
        file_providers::tick(&mut app, ui.ctx());
        assert_eq!(app.session.active().unwrap().doc.id, doc);
        assert!(app.has_unsaved_work(), "the parked close remains guarded until UI settles");
        assert!(crate::type_tool::handle_keys(&mut app, ui.ctx()));
        crate::discard_ui::finish_provider_save(&mut app, ui.ctx());
        app.finish_type_reset_pass(ui.ctx());
    });
    output.textures_delta.clear(); // This input/protocol fixture does not paint.
    let state = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Text(layer) = &state.doc.layer(photocraft_doc::LayerId(text)).unwrap().content else { panic!("text layer") };
    assert_eq!(layer.text, "Before after");
    assert_eq!(state.saved_revision, captured);
    assert_ne!(state.revision, captured);
    assert!(state.is_dirty());
    assert!(crate::discard_ui::inspect(&app).is_some());
}

#[test]
fn provider_ack_with_only_pointer_input_closes_after_a_settled_ui_pass() {
    let ctx = egui::Context::default();
    let (mut app, provider, _) = app(true);
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
    let ctx = egui::Context::default();
    let (mut app, provider, _) = app(true);
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
    assert!(crate::discard_ui::inspect(&app).is_some());
}

#[test]
fn provider_ack_waits_for_live_gradient_edit_without_pointer_or_dialog() {
    for commit in [false, true] {
        let ctx = egui::Context::default();
        let (mut app, provider, _) = app(true);
        app.run("layer.new.layer", json!({})).unwrap();
        menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
        crate::discard_ui::answer_control(&mut app, &ctx, "save").unwrap();
        let request = resolve_save(&mut app, &provider, &ctx);
        let captured = app.session.active().unwrap().revision;
        app.ui.tool = crate::state::Tool::Gradient;
        app.ui.tool_options.gradient_classic = false;
        assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, egui::Modifiers::NONE,));
        assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Move { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE,));
        assert!(crate::gradient_ui::editing(&app));
        assert_eq!(app.session.active().unwrap().revision, captured, "the live gradient is only a preview");
        assert!(!ctx.input(|input| input.pointer.any_down()));
        assert!(app.ui.dialogs.is_empty());
        provider.result(request, "saved".into(), "resource-one".into()).unwrap();
        file_providers::tick(&mut app, &ctx);
        crate::discard_ui::finish_provider_save(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1, "the ACK must wait for the uncommitted gradient");
        assert!(app.has_unsaved_work());
        if commit {
            assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE));
            assert!(!crate::gradient_ui::editing(&app));
            assert_ne!(app.session.active().unwrap().revision, captured);
            crate::discard_ui::finish_provider_save(&mut app, &ctx);
            assert!(app.session.active().unwrap().is_dirty());
            assert!(crate::discard_ui::inspect(&app).is_some());
        } else {
            app.gradient = Default::default();
            crate::discard_ui::finish_provider_save(&mut app, &ctx);
            assert!(app.session.documents().is_empty(), "cancelling the preview leaves the acknowledged capture unchanged");
            assert!(!app.has_unsaved_work());
        }
    }
}

#[test]
fn a_deferred_provider_close_cannot_cross_an_account_connection() {
    let ctx = egui::Context::default();
    let (mut app, provider, _) = app(true);
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
    assert!(crate::discard_ui::inspect(&app).is_some());
}

#[test]
fn provider_save_is_not_recorded_as_a_local_action_or_relabels_an_earlier_local_save() {
    for command in ["file.save", "file.saveAs"] {
        let ctx = egui::Context::default();
        let (mut app, provider, written) = app(true);
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
        let ctx = egui::Context::default();
        let (mut app, provider, written) = app(true);
        if extension == "tif" {
            app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_layered_tiff = true);
            app.run("layer.new.layer", json!({"name":"Second layer"})).unwrap();
        }
        let before = app.session.journal.clone();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        let dialog = event(&provider, "saveDialog");
        let name = format!("drawing.{extension}");
        provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", &name).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(app.save_options_open(), "{extension} needs its format answer before serialization");
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                move |ui, app| {
                    match extension {
                        "tga" => crate::targa_options_ui::show(app, ui.ctx()),
                        "dds" => crate::dds_options_ui::show(app, ui.ctx()),
                        "jpg" => crate::jpeg_options_ui::show(app, ui.ctx()),
                        _ => crate::tiff_options_ui::show(app, ui.ctx()),
                    }
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
    let ctx = egui::Context::default();
    let (mut app, provider, _) = app(true);
    provider
        .set_resources(
            provider.snapshot()["connectionId"].as_str().unwrap(),
            vec![file_providers::Resource { id: "one".into(), name: "one.psd".into(), updated_at: String::new() }],
        )
        .unwrap();
    let read = provider.request_open("one").unwrap();
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert!(crate::discard_ui::inspect(&app).is_some());
    assert!(provider.busy());
    assert!(!app.saving());
    crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").unwrap();
    assert!(app.session.documents().is_empty());
    provider.cancel_open(read, String::new()).unwrap();
    assert!(!provider.busy());
}

#[test]
fn pending_save_as_arms_leave_guard_even_when_the_old_revision_is_clean() {
    let ctx = egui::Context::default();
    let (mut app, provider, _) = app(true);
    app.session.active_mut().unwrap().saved_revision = 7;
    assert!(!app.has_unsaved_work());
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    let dialog = event(&provider, "saveDialog");
    assert!(app.has_unsaved_work());
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 1, "a clean Save As still owns an external side effect");
    assert!(crate::discard_ui::answer_control(&mut app, &ctx, "dontSave").is_err());
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "cancel", "").unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(!app.has_unsaved_work());
}

#[test]
fn cancelling_format_options_releases_the_destination_and_reopens_the_chooser() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(true);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.tga").unwrap();
    app.poll_file_dialog(&ctx, None);
    file_providers::capture(&mut app);
    assert!(app.targa_options.is_some());
    assert!(app.saving(), "the destination remains reserved while format options are open");
    assert!(menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).is_err());
    assert!(provider.take_events().is_empty(), "Save a Copy cannot reuse the pending save's intent");
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(
            |ui, app| {
                crate::targa_options_ui::show(app, ui.ctx());
                file_providers::capture(app);
            },
            app,
        );
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().targa_options.is_none());
    assert!(!h.state().saving());
    assert!(written.borrow().is_empty());
    assert_eq!(h.state().session.active().unwrap().saved_revision, 2);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    h.state_mut().poll_file_dialog(&ctx, None);
    let next = event(&provider, "saveDialog");
    assert_ne!(next["requestId"], dialog["requestId"]);
    assert_eq!(next["intent"], "save");
}

#[test]
fn a_bound_native_project_keeps_its_format_and_resource_on_the_next_save() {
    let ctx = egui::Context::default();
    let (mut app, provider, written) = app(true);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    let dialog = event(&provider, "saveDialog");
    provider.resolve_dialog(dialog["requestId"].as_str().unwrap(), "provider", "drawing.pcraft").unwrap();
    app.poll_file_dialog(&ctx, None);
    let save = event(&provider, "save");
    provider.result(save["requestId"].as_str().unwrap().into(), "saved".into(), "resource-native".into()).unwrap();
    file_providers::tick(&mut app, &ctx);
    assert!(app.session.active().unwrap().path.is_none());
    app.session.active_mut().unwrap().revision = 8;
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    let next = event(&provider, "saveDialog");
    assert_eq!(next["filename"], "drawing.pcraft");
    assert_eq!(next["intent"], "save");
    provider.resolve_dialog(next["requestId"].as_str().unwrap(), "provider", "drawing.pcraft").unwrap();
    app.poll_file_dialog(&ctx, None);
    let next_save = event(&provider, "save");
    assert_eq!(next_save["filename"], "drawing.pcraft");
    assert_eq!(next_save["resourceId"], "resource-native");
    assert_eq!(next_save["revision"], "8");
    assert!(written.borrow().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.session.active().unwrap().path.is_none());
}
