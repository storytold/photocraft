//! Modern presentation and its actual runtime ownership/navigation seams.
use super::*;
use crate::{FileDialogRequest, actions::new_set_dialog};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":8,"height":6,"background":"#203040"})).unwrap();
    app
}

fn draft(app: &mut PhotocraftApp, steps: &[(&str, Value)]) -> u64 {
    let id = open(app, json!({"name":"Draft"})).unwrap()["draft"].as_u64().unwrap();
    for (index, (command, params)) in steps.iter().enumerate() {
        app.run("actions.editor.step", json!({"draft":id,"op":"insert","step":index,"id":command,"params":params})).unwrap();
    }
    id
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(480.0, 740.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                panel(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

#[test]
fn defaults_layout_commands_and_validated_preferences_are_shell_only() {
    let mut app = app();
    assert_eq!(app.ui.actions_layout, ActionsLayout::Modern);
    assert_eq!(app.ui.actions_list_size, ActionsListSize::Large);
    let actions = app.session.actions.clone();
    app.run("actions.layout.classic", json!({})).unwrap();
    app.ui.actions_list_size = ActionsListSize::Small;
    persist(&mut app, &egui::Context::default());
    assert_eq!(app.session.prefs().actions_panel, json!({"layout":"classic","size":"small"}));
    assert_eq!(app.session.actions, actions);
    assert_eq!(layout_checked(&app, "actions.layout.classic"), Some(true));
    assert!(photocraft_engine::shell_commands::SHELL_COMMANDS.iter().any(|entry| entry.0 == "actions.layout.modern"));
    let mut fresh = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    fresh.session.prefs.edit(|prefs| prefs.actions_panel = json!({"layout":"classic","size":"small","draft":99}));
    restore(&mut fresh);
    assert_eq!(fresh.ui.actions_layout, ActionsLayout::Classic);
    assert_eq!(fresh.ui.actions_list_size, ActionsListSize::Small);
    assert!(fresh.actions_runtime.page.is_none());
    for bad in [Value::Null, json!([]), json!({"layout":"future","size":-1})] {
        fresh.session.prefs.edit(|prefs| prefs.actions_panel = bad);
        restore(&mut fresh);
        assert_eq!(fresh.ui.actions_layout, ActionsLayout::Modern);
        assert_eq!(fresh.ui.actions_list_size, ActionsListSize::Large);
    }
}

#[test]
fn modern_empty_page_has_search_creation_and_required_panel_menu() {
    let mut h = harness(app());
    h.get_by_label("Search actions");
    h.get_by_label("Yours");
    h.get_by_label("You haven't created any actions");
    h.get_by_label("Create new action").click();
    h.run_steps(3);
    assert!(h.state().actions_runtime.page.is_some());
    assert!(h.state().session.actions.list.is_empty(), "New opens an unpublished draft");
    assert_eq!(inspect(h.state())["layout"], "modern");
}

#[test]
fn dirty_layout_leave_cancel_keeps_page_and_capture_then_discard_resumes_destination() {
    let mut app = app();
    let id = draft(&mut app, &[("select.all", json!({}))]);
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    let take = app.session.actions.take;
    app.run("actions.layout.classic", json!({})).unwrap();
    assert!(app.actions_runtime.pending.is_some());
    let mut h = harness(app);
    h.get_by_label("Cancel and continue editing").click();
    h.run_steps(3);
    assert!(h.state().actions_runtime.pending.is_none());
    assert_eq!(h.state().actions_runtime.page.as_ref().unwrap().draft(), id);
    assert!(h.state().session.actions.recording_active());
    assert_eq!(h.state().session.actions.take, take);
    h.state_mut().run("actions.layout.classic", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.actions_layout, ActionsLayout::Classic);
    assert!(h.state().actions_runtime.page.is_none());
    assert!(h.state().session.actions.editor.is_none());
    assert!(h.state().session.actions.list.is_empty());
}

#[test]
fn delete_confirmation_is_one_use_and_rejects_a_changed_target() {
    let mut app = app();
    app.session.actions.list.push(Action::new("Delete me", Vec::new()));
    let id = ask_delete(&mut app, Some(0), None).unwrap()["dialog"].as_u64().unwrap();
    app.session.actions.list[0].description = "Changed after the question".into();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.session.actions.list.len(), 1);
    assert!(app.actions_runtime.delete.is_none());
    let id = ask_delete(&mut app, Some(0), None).unwrap()["dialog"].as_u64().unwrap();
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.actions.list.len(), 1);
    let id = ask_delete(&mut app, Some(0), None).unwrap()["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.actions.list.is_empty());
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
}

#[test]
fn empty_group_delete_question_rejects_a_changed_library_revision() {
    let mut app = app();
    app.run("actions.newSet", json!({"name":"Empty"})).unwrap();
    let id = ask_delete(&mut app, None, Some("Empty".into())).unwrap()["dialog"].as_u64().unwrap();
    app.run("actions.newSet", json!({"name":"Later"})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(photocraft_engine::actions_edit_cmds::set_names(&app.session).contains(&"Empty".to_string()));
}

#[test]
fn forced_small_ticks_show_progress_keep_parent_tail_budget_gate_and_capture() {
    let mut app = app();
    app.session.actions.list.push(Action::new("Child", vec![("layer.new.layer".into(), json!({"name":"Child one"})), ("select.all".into(), json!({}))]));
    let id = draft(&mut app, &[("actions.play", json!({"action":"Child"})), ("layer.new.layer", json!({"name":"Parent tail"}))]);
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    let before = actions_cmds::editor::inspect(&app.session, id).unwrap()["action"].clone();
    let take = app.session.actions.take;
    start_preview(&mut app, id, 0).unwrap();
    assert!(app.session.execute("actions.editor.update", json!({"draft":id,"name":"Intruder"})).is_err());
    assert!(app.session.start("filter.blur.gaussianBlur", json!({"radius":1})).is_err());
    let ctx = egui::Context::default();
    for _ in 0..3 {
        tick_with_quantum(&mut app, &ctx, 1);
    }
    assert_eq!(inspect(&app)["playing"]["progress"], 0.5);
    assert_eq!(app.actions_runtime.task.as_ref().unwrap().used, 3);
    let mut h = harness(app);
    h.get_by_label("Draft"); // The real panel displays the active action, not the list.
    assert_eq!(inspect(h.state())["playing"]["progress"], 0.5);
    tick_with_quantum(h.state_mut(), &ctx, 1);
    assert_eq!(inspect(h.state())["playing"]["progress"], 1.0);
    assert!(h.state().session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "Parent tail"));
    assert_eq!(h.state().session.actions.take, take);
    assert!(h.state().session.actions.recording_active());
    assert_eq!(actions_cmds::editor::inspect(&h.state().session, id).unwrap()["action"], before);
    assert!(h.state().session.actions.held_playback.is_none());

    fn deny(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "layer.new.layer" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "original gate".into() }) } else { Ok(()) }
    }
    tick_with_quantum(h.state_mut(), &ctx, 1); // Release the completed card.
    h.state_mut().automation_input = true;
    h.state_mut().services.automation_authorize = Some(deny);
    start_preview(h.state_mut(), id, 0).unwrap();
    h.state_mut().automation_input = false;
    tick_with_quantum(h.state_mut(), &ctx, 1);
    tick_with_quantum(h.state_mut(), &ctx, 1);
    assert!(h.state().ui.status.contains("original gate"));
    assert!(h.state().session.actions.recording_active());
    assert!(h.state().session.actions.held_playback.is_none());

    h.state_mut().services.automation_authorize = None;
    start_preview(h.state_mut(), id, 0).unwrap();
    h.state_mut().actions_runtime.task.as_mut().unwrap().used = actions_cmds::MAX_PLAYBACK_STEPS - 1;
    tick_with_quantum(h.state_mut(), &ctx, 1);
    tick_with_quantum(h.state_mut(), &ctx, 1);
    assert!(h.state().ui.status.contains("step limit"));
    assert!(h.state().session.actions.recording_active());
    assert!(h.state().session.actions.held_playback.is_none());
}

#[test]
fn tick_step_dialog_confirm_cancel_and_replayed_token_finish_ownership_once() {
    let mut app = app();
    let id = draft(&mut app, &[("filter.blur.gaussianBlur", json!({"radius":1})), ("layer.new.layer", json!({"name":"After dialog"}))]);
    app.run("actions.editor.step", json!({"draft":id,"op":"dialog","step":0,"enabled":true})).unwrap();
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    let ctx = egui::Context::default();
    start_preview(&mut app, id, 0).unwrap();
    tick_with_quantum(&mut app, &ctx, 1);
    let dialog = app.ui.dialogs.last().unwrap().id;
    let fields = app.ui.dialogs.last().unwrap().fields.clone();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.actions_runtime.task.is_some());
    tick_with_quantum(&mut app, &ctx, 1);
    assert!(app.session.actions.recording_active());
    assert!(resume(&mut app, &fields).is_err());
    assert!(app.session.actions.recording_active());
    tick_with_quantum(&mut app, &ctx, 1);
    start_preview(&mut app, id, 0).unwrap();
    tick_with_quantum(&mut app, &ctx, 1);
    let dialog = app.ui.dialogs.last().unwrap().id;
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(app.actions_runtime.progress.is_none());
    assert!(app.session.actions.recording_active());
    assert!(app.session.actions.held_playback.is_none());
}

#[test]
fn runtime_shutdown_releases_an_owned_yield_without_saving_the_draft() {
    let mut app = app();
    let id = draft(&mut app, &[("select.all", json!({}))]);
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    start_preview(&mut app, id, 0).unwrap();
    shutdown(&mut app);
    assert!(app.session.actions.recording_active());
    assert!(app.session.actions.list.is_empty());
    assert!(app.session.actions.held_playback.is_none());
    shutdown(&mut app);
    assert!(app.session.actions.recording_active());
}

#[test]
fn modern_spelling_pause_survives_a_later_tick_then_done_or_cancel_finishes_its_owner() {
    for done in [true, false] {
        let mut app = app();
        app.run("type.create", json!({"x": 2, "y": 4, "text": "Teh cat", "size": 12})).unwrap();
        let id = draft(&mut app, &[("edit.checkSpelling", json!({"allLayers": false})), ("layer.new.layer", json!({"name": "After spelling"}))]);
        app.run("actions.editor.step", json!({"draft": id, "op": "dialog", "step": 0, "enabled": true})).unwrap();
        app.run("actions.editor.record", json!({"draft": id})).unwrap();
        let original = actions_cmds::editor::inspect(&app.session, id).unwrap()["action"].clone();
        let take = app.session.actions.take;
        let ctx = egui::Context::default();
        start_preview(&mut app, id, 0).unwrap();
        tick_with_quantum(&mut app, &ctx, 1);
        let token = app.ui.type_panels.spell.as_ref().unwrap().resume.unwrap();
        assert!(app.ui.dialogs.is_empty(), "Spelling retains its own non-modal panel");
        assert!(app.paused_playbacks.paused[&token].incremental);
        assert!(app.session.actions.held_playback.is_some());
        tick_with_quantum(&mut app, &ctx, 1);
        assert_eq!(app.ui.type_panels.spell.as_ref().unwrap().resume, Some(token), "a later tick must not orphan Spelling");
        assert!(app.paused_playbacks.paused.contains_key(&token));
        if done {
            crate::type_panels_ui::close_spelling(&mut app);
            assert!(app.actions_runtime.task.is_some());
            tick_with_quantum(&mut app, &ctx, 1);
        } else {
            cancel(&mut app);
            assert!(app.ui.type_panels.spell.is_none());
        }
        assert_eq!(app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "After spelling"), done);
        assert!(app.session.actions.recording_active());
        assert_eq!(app.session.actions.take, take);
        assert!(app.session.actions.held_playback.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(actions_cmds::editor::inspect(&app.session, id).unwrap()["action"], original, "preview never publishes or records itself");
        assert!(app.session.actions.list.is_empty());
        // Cancelling an already completed playback must not close a subsequently opened manual panel.
        crate::type_panels_ui::open_spelling(&mut app, true).unwrap();
        assert!(app.ui.type_panels.spell.as_ref().unwrap().resume.is_none());
        cancel(&mut app);
        assert!(app.ui.type_panels.spell.is_some());
    }
}

#[test]
fn draft_spelling_records_the_completed_choice_once_on_done() {
    let mut app = app();
    app.run("type.create", json!({"x": 2, "y": 4, "text": "Teh cat", "size": 12})).unwrap();
    let id = draft(&mut app, &[]);
    app.run("actions.editor.record", json!({"draft": id})).unwrap();
    crate::type_panels_ui::open_spelling(&mut app, false).unwrap();
    assert!(
        actions_cmds::editor::inspect(&app.session, id).unwrap()["action"]["steps"].as_array().unwrap().is_empty(),
        "opening/listing is not the recorded choice"
    );
    crate::type_panels_ui::close_spelling(&mut app);
    app.run("actions.editor.stop", json!({"draft": id})).unwrap();
    let value = actions_cmds::editor::inspect(&app.session, id).unwrap();
    assert_eq!(value["action"]["steps"], json!([["edit.checkSpelling", {"allLayers": false}]]));
    assert_eq!(value["action"]["dialogs"], json!([0]));
    crate::type_panels_ui::close_spelling(&mut app);
    assert_eq!(actions_cmds::editor::inspect(&app.session, id).unwrap(), value, "Done is one-use");
    assert!(app.session.actions.list.is_empty());
}

#[test]
fn search_is_case_insensitive_and_new_group_keeps_classic_set_defaults() {
    let mut app = app();
    app.session.actions.list = vec![Action::new("Warm contrast", Vec::new()), Action::new("Cool shadows", Vec::new())];
    app.ui.actions.search = "WARM".into();
    let mut h = harness(app);
    h.get_by_label("Warm contrast");
    assert!(h.query_by_label("Cool shadows").is_none());
    h.get_by_label("Clear search").click();
    h.run_steps(3);
    h.get_by_label("Cool shadows");
    let id = new_group(h.state_mut()).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["name"], "Group 1");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let id = new_set_dialog(h.state_mut()).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["name"], "Set 1");
    crate::dialogs::cancel(h.state_mut(), id).unwrap();
}

#[test]
fn closing_a_group_with_a_hidden_editor_defers_and_discard_replays_the_real_close() {
    let mut app = app();
    app.run("window.panel.activate", json!({"panel":"actions"})).unwrap();
    app.run("window.panel.activate", json!({"panel":"history"})).unwrap();
    draft(&mut app, &[("select.all", json!({}))]);
    app.run("window.panel.group", json!({"panel":"history","operation":"close"})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().layout.contains(&"actions".into()));
    assert!(app.actions_runtime.pending.is_some());
    let mut h = Harness::builder().with_size(vec2(480.0, 740.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                modals(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert!(h.state().actions_runtime.page.is_none());
    assert!(!h.state().ui.docking.as_ref().unwrap().layout.contains(&"actions".into()));
    assert!(!h.state().ui.docking.as_ref().unwrap().layout.contains(&"history".into()));
}

#[test]
fn export_suggests_the_selected_action_only_with_a_portable_uppercase_extension() {
    let mut app = app();
    app.session.actions.list = vec![Action::new("Other", Vec::new()), Action::new("Web/Print:Mail", Vec::new())];
    app.ui.actions.selected = Some(1);
    let (file_dialog, asked) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("/virtual/selected.ATN".into()))]);
    app.services.file_dialog = Some(file_dialog);
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let output = written.clone();
    app.services.write = Some(Box::new(move |_, bytes| {
        *output.borrow_mut() = bytes.to_vec();
        Ok(())
    }));
    export(&mut app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(matches!(asked.borrow().as_slice(),[FileDialogRequest::Save{suggested}] if suggested=="Web_Print_Mail.ATN"));
    let parsed = photocraft_psd::atn::parse(&written.borrow()).unwrap();
    assert_eq!(parsed.actions.len(), 1);
    assert_eq!(parsed.actions[0].name.to_string_lossy(), "Web/Print:Mail");
}

#[test]
fn canonical_ungrouped_category_keeps_classic_loose_rows_flat_and_order_stable() {
    let mut app = app();
    let mut explicit = Action::new("Explicit category", Vec::new());
    explicit.set = Some(photocraft_engine::atn_cmds::UNGROUPED_SET.into());
    let mut named = Action::new("Named action", Vec::new());
    named.set = Some("Named group".into());
    app.session.actions.list = vec![named, Action::new("Legacy none", Vec::new()), explicit];
    let original = app.session.actions.list.clone();
    let mut h = Harness::builder().with_size(vec2(480.0, 740.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::actions::panel(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h.get_by_label("Legacy none");
    h.get_by_label("Explicit category");
    assert!(h.query_by_label(photocraft_engine::atn_cmds::UNGROUPED_SET).is_none());
    h.state_mut().run("actions.layout.classic", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label(photocraft_engine::atn_cmds::UNGROUPED_SET).is_none(), "ordinary loose actions keep the original flat presentation");
    assert!(h.get_by_label("Legacy none").rect().top() < h.get_by_label("Explicit category").rect().top());
    assert!(h.get_by_label("Explicit category").rect().top() < h.get_by_label("Named action").rect().top(), "loose actions precede named folders");
    assert_eq!(h.state().session.actions.list, original, "presentation never rewrites legacy None or stored names");
    h.state_mut().ui.actions.selected = None;
    h.state_mut().ui.actions.selected_set = Some(photocraft_engine::atn_cmds::UNGROUPED_SET.into());
    assert_eq!(super::super::playback_targets(h.state()), [1, 2]);
    // A real retained empty category remains visible after its last action is removed.
    h.state_mut().session.actions.list.clear();
    h.state_mut().run("actions.newSet", json!({"name":photocraft_engine::atn_cmds::UNGROUPED_SET})).unwrap();
    h.run_steps(3);
    h.get_by_label(photocraft_engine::atn_cmds::UNGROUPED_SET);
    h.state_mut().run("actions.layout.modern", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label(photocraft_engine::atn_cmds::UNGROUPED_SET);
}
