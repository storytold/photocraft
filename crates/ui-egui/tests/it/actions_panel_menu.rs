//! Real panel controls compose the remaining commands with consent and retained selection.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action, actions_edit_cmds, atn_cmds};
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, PhotocraftApp, actions, canvas, dialogs};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{cell::RefCell, rc::Rc, sync::atomic::AtomicUsize};

/// A real panel and its dialogs with one selected action in a set.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":16,"height":12})).unwrap();
    let mut a = Action::new("A", vec![]);
    a.set = Some("S".into());
    app.session.actions.list.push(a);
    app.ui.actions.selected = Some(0);
    app.ui.actions.expanded = vec![true];
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
        |ui, app| {
            actions::panel_menu(app, ui);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// Click the actual menu entry and return its modal identity.
fn open(h: &mut Harness<'static, PhotocraftApp>, label: &str) -> u64 {
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state().ui.dialogs.last().unwrap().id
}

/// The checkbox changes recording policy rather than disabling actual painting.
#[test]
fn allow_tool_recording_checkbox_records_only_enabled_strokes() {
    let mut h = harness();
    h.state_mut().run("actions.record", json!({"action":"A"})).unwrap();
    for enabled in [true, false] {
        h.get_by_label("Allow Tool Recording").click();
        h.run_steps(3);
        assert_eq!(h.state().session.actions.record_tools, enabled);
        h.state_mut().run("paint.stroke", json!({"points":[[2,3],[10,3]],"size":2,"color":if enabled {"#ff0000"} else {"#00ff00"}})).unwrap();
    }
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(h.state().session.actions.list[0].steps.len(), 1);
    assert_eq!(h.state().session.actions.list[0].steps[0].1["color"], "#ff0000");
    assert_eq!(h.state().session.journal.iter().filter(|s| s.0 == "paint.stroke").count(), 2);
}

/// Folder selection dispatches set duplication and keeps the new folder selected, empty ones included.
#[test]
fn duplicate_selected_set_keeps_folder_and_disclosure_ownership() {
    let mut h = harness();
    h.state_mut().ui.actions.selected = None;
    h.state_mut().ui.actions.selected_set = Some("S".into());
    h.get_by_label("Duplicate").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.actions.selected, None);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("S copy"));
    assert_eq!(actions_edit_cmds::set_names(&h.state().session), ["S", "S copy"]);
    assert_eq!(h.state().ui.actions.expanded, [true, false]);
    h.state_mut().run("actions.newSet", json!({"name":"Empty"})).unwrap();
    h.state_mut().ui.actions.selected_set = Some("Empty".into());
    h.get_by_label("Duplicate").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Empty copy"));
}

/// Menu insertion uses an actual chooser and never samples or executes current command settings.
#[test]
fn insert_menu_item_chooser_cancels_or_records_descriptor_free_filter_event() {
    let mut h = harness();
    let revision = h.state().session.active().unwrap().revision;
    open(&mut h, "Insert Menu Item…");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().session.actions.list[0].steps.is_empty());
    let dialog = open(&mut h, "Insert Menu Item…");
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("id".into(), json!("filter.blur.gaussianBlur"));
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list[0].steps[0], (atn_cmds::MENU_ITEM.into(), json!({"id":"filter.blur.gaussianBlur","eventKind":"name"})));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().ui.actions.selected_step, Some(0));
    let played = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = played["paused"]["dialog"].as_u64().unwrap();
    assert_eq!(h.state_mut().ui.dialog_mut(dialog).unwrap().fields["__command"], "filter.blur.gaussianBlur");
    dialogs::cancel(h.state_mut(), dialog).unwrap();
}

/// The selected path is snapshotted without applying it to the document.
#[test]
fn insert_path_uses_actual_selected_saved_path() {
    let mut h = harness();
    h.get_by_label("Insert Path");
    assert!(h.state_mut().run("actions.insertPath", json!({"action":"A"})).is_err());
    h.state_mut().run("path.set", json!({"name":"Curve","path":{"subpaths":[{"closed":false,"knots":[{"anchor":[1,2],"out":[4,5]},[9,8]]}]}})).unwrap();
    h.state_mut().ui.selected_path = Some("Curve".into());
    h.run_steps(3);
    let revision = h.state().session.active().unwrap().revision;
    h.get_by_label("Insert Path").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list[0].steps[0].0, "path.set");
    assert_eq!(h.state().session.actions.list[0].steps[0].1["path"]["subpaths"][0]["knots"][0]["out"], json!([4.0, 5.0]));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
}

static REVOKED: AtomicBool = AtomicBool::new(false);

/// A caller may lose permission while a destructive warning is open.
fn revocable(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if matches!(id, "actions.clearAll" | "actions.reset") && REVOKED.load(Ordering::SeqCst) { Err(EngineError::Other("revoked".into())) } else { Ok(()) }
}

/// Actual append/replace/Cancel work, and retained or stale consent cannot clear the library.
#[test]
fn reset_placeholder_and_clear_all_keep_actual_consent_and_caller() {
    let mut h = harness();
    let original = h.state().session.actions.list.clone();
    open(&mut h, "Reset Actions…");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list, original);
    open(&mut h, "Reset Actions…");
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list, original);
    assert!(h.state().session.actions.sets.iter().any(|s| s.contains("placeholder")));
    open(&mut h, "Reset Actions…");
    h.get_by_label("Replace").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().session.actions.list.is_empty());
    assert_eq!(h.state().session.actions.sets, [actions_edit_cmds::DEFAULT_SET_PLACEHOLDER]);
    for label in ["Reset Actions…", "Clear All Actions"] {
        REVOKED.store(false, Ordering::SeqCst);
        let mut h = harness();
        h.state_mut().session.authorize = Some(revocable);
        let dialog = open(&mut h, label);
        h.state_mut().session.authorize = None;
        REVOKED.store(true, Ordering::SeqCst);
        assert!(dialogs::confirm(h.state_mut(), dialog).is_err());
        assert_eq!(h.state().session.actions.list.len(), 1);
        REVOKED.store(false, Ordering::SeqCst);
        h.run_steps(3);
        let dialog = open(&mut h, label);
        h.state_mut().run("actions.rename", json!({"action":"A","name":"Changed"})).unwrap();
        assert!(dialogs::confirm(h.state_mut(), dialog).is_err());
        assert_eq!(h.state().session.actions.list[0].name, "Changed");
    }
}

static PATH_CHECKS: AtomicUsize = AtomicUsize::new(0);

/// A retained caller rejects edited map paths after the current request's gate is cleared.
fn deny_map_path(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id.starts_with("filter.") {
        PATH_CHECKS.fetch_add(1, Ordering::SeqCst);
        if params.get("mapPath").and_then(Value::as_str).is_some_and(|p| !p.is_empty()) {
            return Err(EngineError::Other("map path denied".into()));
        }
    }
    Ok(())
}

/// Real menu previews and Displace's later picker callback keep fixed ownership and authority.
#[test]
fn paused_menu_previews_and_picker_callbacks_keep_the_requesting_gate() {
    let mut h = harness();
    h.state_mut().run("actions.insertMenuItem", json!({"action":"A","id":"filter.blur.gaussianBlur"})).unwrap();
    h.state_mut().session.authorize = Some(deny_map_path);
    let paused = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = paused["paused"]["dialog"].as_u64().unwrap();
    h.state_mut().session.authorize = None;
    let context = h.ctx.clone();
    PATH_CHECKS.store(0, Ordering::SeqCst);
    assert!(canvas::ensure_texture(h.state_mut(), &context, 0, None).is_some());
    assert!(PATH_CHECKS.load(Ordering::SeqCst) > 0, "the real permitted menu preview is authorized");
    let revision = h.state().session.active().unwrap().revision;
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("mapPath".into(), json!("/outside/map.psd"));
    PATH_CHECKS.store(0, Ordering::SeqCst);
    canvas::ensure_texture(h.state_mut(), &context, 0, None);
    assert!(PATH_CHECKS.load(Ordering::SeqCst) > 0, "edited path parameters are judged under the captured gate");
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__command".into(), json!("filter.distort.displace"));
    assert!(dialogs::confirm(h.state_mut(), dialog).is_err(), "the editor cannot retarget to a path-reading command");
    dialogs::cancel(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.active().unwrap().revision, revision);

    let mut h = harness();
    let waiting: Rc<RefCell<Option<FileDialogReply>>> = Default::default();
    let reply = waiting.clone();
    h.state_mut().services.file_dialog = Some(Box::new(move |_, _, answer| *reply.borrow_mut() = Some(answer)));
    let a = &mut h.state_mut().session.actions.list[0];
    a.steps.push(("filter.distort.displace".into(), json!({})));
    a.dialogs.insert(0);
    h.state_mut().session.authorize = Some(deny_map_path);
    let paused = h.state_mut().run("actions.play", json!({"action":"A"})).unwrap();
    let dialog = paused["paused"]["dialog"].as_u64().unwrap();
    h.state_mut().session.authorize = None;
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("mapDocument".into(), json!(-1));
    let revision = h.state().session.active().unwrap().revision;
    dialogs::confirm(h.state_mut(), dialog).unwrap();
    let context = h.ctx.clone();
    h.state_mut().poll_file_dialog(&context, None);
    waiting.borrow_mut().take().unwrap().send(Some(FileDialogAnswer::Paths(vec!["/outside/denied-map.psd".into()])));
    h.state_mut().poll_file_dialog(&context, None);
    assert!(h.state().ui.status.contains("map path denied"), "{}", h.state().ui.status);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.actions.list[0].steps[0].1, json!({}));
}
