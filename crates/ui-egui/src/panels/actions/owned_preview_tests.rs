//! Windowless consumers of the real preview, confirmation and deferred-picker routes.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
    app
}

fn draft(app: &mut PhotocraftApp, steps: &[(&str, Value)]) -> u64 {
    let id = app.run("actions.editor.open", json!({"name":"Unpublished"})).unwrap()["draft"].as_u64().unwrap();
    for (index, (command, params)) in steps.iter().enumerate() {
        app.run("actions.editor.step", json!({"draft":id,"op":"insert","step":index,"id":command,"params":params})).unwrap();
    }
    id
}

fn get(app: &mut PhotocraftApp, id: u64) -> Value {
    app.run("actions.editor.get", json!({"draft":id})).unwrap()
}

fn has_layer(app: &PhotocraftApp, name: &str) -> bool {
    app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == name)
}

fn dialog(app: &PhotocraftApp) -> u64 {
    app.ui.dialogs.last().unwrap().id
}

fn blur_draft(app: &mut PhotocraftApp) -> u64 {
    let id = draft(app, &[("filter.blur.gaussianBlur", json!({"radius":1})), ("layer.new.layer", json!({"name":"Suffix"}))]);
    app.run("actions.editor.step", json!({"draft":id,"op":"dialog","step":0,"enabled":true})).unwrap();
    id
}

#[test]
fn unpublished_preview_preserves_snapshot_indices_flags_and_metadata() {
    let mut app = app();
    let id = draft(&mut app, &[("layer.new.layer", json!({"name":"Disabled"})), ("layer.new.layer", json!({"name":"Visible"}))]);
    app.run("actions.editor.update", json!({"draft":id,"description":"Uncommitted description","icon":"UnknownGlyph"})).unwrap();
    app.run("actions.editor.step", json!({"draft":id,"op":"enabled","step":0,"enabled":false})).unwrap();
    let before = get(&mut app, id);
    let (library, revision, prefs) = (app.session.actions.list.clone(), app.session.actions.rev, app.session.prefs().clone());
    let played = preview_draft(&mut app, id, 0).unwrap();
    assert_eq!(played["ran"], 1);
    assert!(!has_layer(&app, "Disabled"));
    assert!(has_layer(&app, "Visible"));
    assert_eq!(get(&mut app, id), before);
    assert_eq!(app.session.actions.list, library);
    assert_eq!(app.session.actions.rev, revision);
    assert_eq!(app.session.prefs(), &prefs);
    assert!(app.session.actions.held_playback.is_none());
}

#[test]
fn preview_resumes_its_capture_without_a_self_call_or_expanded_steps() {
    let mut app = app();
    let id = draft(&mut app, &[("layer.renameLayer", json!({"name":"Preview name"}))]);
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    app.run("layer.new.layer", json!({"name":"Captured before"})).unwrap();
    let before = get(&mut app, id)["action"].clone();
    let take = app.session.actions.take;
    let journal = app.session.journal.len();
    preview_draft(&mut app, id, 0).unwrap();
    assert_eq!(get(&mut app, id)["action"], before);
    assert_eq!(get(&mut app, id)["recording"], true);
    assert_eq!(app.session.actions.take, take);
    assert!(app.session.journal.len() > journal);
    assert!(!app.session.journal[journal..].iter().any(|(id, _)| id == "actions.play"));
    app.run("select.all", json!({})).unwrap();
    let stopped = app.run("actions.editor.stop", json!({"draft":id})).unwrap();
    assert_eq!(stopped["action"]["steps"].as_array().unwrap().len(), 3);
    assert!(app.session.actions.list.is_empty());
}

#[test]
fn a_paused_named_library_call_captures_one_call_in_the_draft() {
    let mut app = app();
    let mut child = Action::new("Child", vec![("filter.blur.gaussianBlur".into(), json!({"radius":1})), ("layer.new.layer".into(), json!({"name":"Suffix"}))]);
    child.dialogs.insert(0);
    app.session.actions.list.push(child);
    let id = draft(&mut app, &[]);
    app.run("actions.editor.record", json!({"draft":id})).unwrap();
    let played = play(&mut app, &json!({"action":"Child"})).unwrap();
    let dialog = played["paused"]["dialog"].as_u64().unwrap();
    for (command, params) in [
        ("actions.editor.save", json!({"draft":id})),
        ("actions.editor.discard", json!({"draft":id})),
        ("actions.editor.record", json!({"draft":id})),
        ("actions.record", json!({"name":"Intruder"})),
    ] {
        assert!(app.run(command, params.clone()).is_err());
        assert!(app.session.execute(command, params).is_err(), "direct Session calls also respect the owner");
    }
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let stopped = app.run("actions.editor.stop", json!({"draft":id})).unwrap();
    assert_eq!(stopped["action"]["steps"], json!([["actions.play",{"action":"Child"}]]));
    assert!(has_layer(&app, "Suffix"));
    assert_eq!(app.session.actions.list.len(), 1);
}

static PARENT_GATED: AtomicBool = AtomicBool::new(false);
fn original_gate(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "layer.new.layer" && params["name"] == "Parent tail" {
        PARENT_GATED.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[test]
fn two_nested_pauses_keep_the_parent_tail_gate_and_one_use_tokens() {
    PARENT_GATED.store(false, Ordering::SeqCst);
    let mut app = app();
    let mut child = Action::new(
        "Child",
        vec![
            ("filter.blur.gaussianBlur".into(), json!({"radius":1})),
            ("filter.blur.gaussianBlur".into(), json!({"radius":2})),
            ("layer.new.layer".into(), json!({"name":"Child tail"})),
        ],
    );
    child.dialogs.extend([0, 1]);
    app.session.actions.list.push(child);
    let id = draft(
        &mut app,
        &[
            ("layer.new.layer", json!({"name":"Parent prefix"})),
            ("actions.play", json!({"action":"Child"})),
            ("layer.new.layer", json!({"name":"Parent tail"})),
        ],
    );
    let before = get(&mut app, id);
    app.session.authorize = Some(original_gate);
    preview_draft(&mut app, id, 0).unwrap();
    let first = dialog(&app);
    let fields = app.ui.dialog_mut(first).unwrap().fields.clone();
    app.session.authorize = None;
    crate::dialogs::confirm(&mut app, first).unwrap();
    let second = dialog(&app);
    assert_ne!(first, second);
    assert_ne!(fields[RESUME], app.ui.dialog_mut(second).unwrap().fields[RESUME]);
    let doc = app.session.active().unwrap().doc.clone();
    assert!(resume(&mut app, &fields).is_err());
    assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    crate::dialogs::confirm(&mut app, second).unwrap();
    assert!(has_layer(&app, "Parent prefix"));
    assert!(has_layer(&app, "Child tail"));
    assert!(has_layer(&app, "Parent tail"));
    assert!(PARENT_GATED.load(Ordering::SeqCst));
    assert_eq!(get(&mut app, id), before);
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.session.actions.held_playback.is_none());
}

#[test]
fn cancel_command_error_and_worker_error_restore_only_capture() {
    for exit in ["cancel", "command error", "worker error"] {
        let mut app = app();
        let id = draft(
            &mut app,
            &[
                ("layer.setProps", json!({"name":"Prefix","locks":{"pixels":exit == "worker error"}})),
                ("filter.blur.gaussianBlur", json!({"radius":1})),
                ("layer.new.layer", json!({"name":"Suffix"})),
            ],
        );
        app.run("actions.editor.step", json!({"draft":id,"op":"dialog","step":1,"enabled":true})).unwrap();
        app.run("actions.editor.record", json!({"draft":id})).unwrap();
        let before = get(&mut app, id)["action"].clone();
        app.background_jobs = exit == "worker error";
        preview_draft(&mut app, id, 0).unwrap();
        let dialog = dialog(&app);
        if exit == "cancel" {
            crate::dialogs::cancel(&mut app, dialog).unwrap();
        } else {
            if exit == "command error" {
                app.ui.dialog_mut(dialog).unwrap().fields.insert("radius".into(), json!("invalid"));
            }
            assert!(crate::dialogs::confirm(&mut app, dialog).is_err(), "{exit}");
        }
        assert!(has_layer(&app, "Prefix"), "prefix document work remains: {exit}");
        assert!(!has_layer(&app, "Suffix"));
        assert_eq!(get(&mut app, id)["action"], before);
        assert_eq!(get(&mut app, id)["recording"], true);
        assert!(app.session.actions.held_playback.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

#[test]
fn stale_owner_or_missing_and_replayed_tokens_cannot_run_the_dialog_first() {
    for fault in ["new take", "closed draft", "bad token", "missing token"] {
        let mut app = app();
        let id = blur_draft(&mut app);
        app.run("actions.editor.record", json!({"draft":id})).unwrap();
        preview_draft(&mut app, id, 0).unwrap();
        let dialog = dialog(&app);
        let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
        let doc = app.session.active().unwrap().doc.clone();
        match fault {
            "new take" => {
                app.session.actions.take += 1;
                app.session.actions.list.push(Action::new("Later recorder", Vec::new()));
                app.session.actions.recording = Some((app.session.journal.len(), 0));
            }
            "closed draft" => app.session.actions.editor = None,
            "bad token" => {
                app.ui.dialog_mut(dialog).unwrap().fields.insert(RESUME.into(), json!({"forged":true}));
            }
            _ => {
                app.ui.dialog_mut(dialog).unwrap().fields.remove(RESUME);
            }
        }
        let recording = app.session.actions.recording;
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err(), "{fault}");
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc), "{fault}");
        assert_eq!(app.session.actions.recording, recording, "never overwrite a later take");
        assert!(app.session.actions.held_playback.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert!(resume(&mut app, &fields).is_err());
    }
}

fn deny_map_path(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.distort.displace" && params.get("mapPath").is_some() {
        return Err(photocraft_engine::EngineError::Other("original gate refuses the map".into()));
    }
    Ok(())
}

#[test]
fn deferred_displace_checks_the_owner_and_gate_before_refocus_or_command() {
    for stale in [false, true] {
        let mut app = app();
        let id = draft(&mut app, &[("filter.distort.displace", json!({"horizontal":1,"vertical":1})), ("layer.new.layer", json!({"name":"Suffix"}))]);
        app.run("actions.editor.step", json!({"draft":id,"op":"dialog","step":0,"enabled":true})).unwrap();
        let (show, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec!["not-read.png".into()]))]);
        app.services.file_dialog = Some(show);
        app.session.authorize = Some(deny_map_path);
        preview_draft(&mut app, id, 0).unwrap();
        let dialog = dialog(&app);
        app.session.authorize = None;
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(app.file_dialog_open());
        if stale {
            app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
            app.session.actions.editor = None;
        }
        let target = app.session.active().unwrap().doc.clone();
        let active = app.session.active().unwrap().doc.id;
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_eq!(app.session.active().unwrap().doc.id, active, "stale callback must not refocus");
        assert!(std::sync::Arc::ptr_eq(&target, &app.session.active().unwrap().doc));
        assert!(app.ui.status_error);
        if !stale {
            assert!(app.ui.status.contains("original gate"), "{}", app.ui.status);
        }
        assert!(app.session.actions.held_playback.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

#[test]
fn a_resumed_preview_keeps_its_tree_step_budget() {
    let mut app = app();
    let id = blur_draft(&mut app);
    app.run("actions.editor.step", json!({"draft":id,"op":"insert","step":2,"id":"layer.new.layer","params":{"name":"Overflow"}})).unwrap();
    preview_draft(&mut app, id, 0).unwrap();
    let dialog = dialog(&app);
    let token = app.ui.dialog_mut(dialog).unwrap().fields[RESUME].as_u64().unwrap();
    app.paused_playbacks.paused.get_mut(&token).unwrap().used = actions_cmds::MAX_PLAYBACK_STEPS - 1;
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(has_layer(&app, "Suffix"));
    assert!(!has_layer(&app, "Overflow"));
    assert!(app.ui.status_error);
    assert_eq!(app.session.actions.playback_steps, actions_cmds::MAX_PLAYBACK_STEPS);
    assert!(app.session.actions.held_playback.is_none());
}
