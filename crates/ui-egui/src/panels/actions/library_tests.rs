use photocraft_engine::actions_cmds::ActionsExt;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use craft::automation::ui::actions::ActionsEvent;
use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::{DialogId, StackDialog, StandardPrompt};
use serde_json::{Value, json};

use super::files_tests::action_set_bytes;
use super::library;
use crate::PhotocraftApp;

fn new_app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn sets(app: &PhotocraftApp) -> Vec<&str> {
    app.session.actions().list.sets()
}

/// The panel menu's `event`, then the question it opened.
fn ask(app: &mut PhotocraftApp, event: ActionsEvent) -> DialogId {
    super::apply_actions_events(app, vec![event]);
    question(app)
}

fn question(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.get::<StandardPrompt>().map(StackDialog::id).expect("the library question is open")
}

/// Cancel leaves the list, OK on a list changed since the question is refused, and Edit › Undo of
/// the library waits behind a newer document edit, from the menu and the command alike.
#[test]
fn clear_all_asks_refuses_a_changed_list_and_shares_edit_undo() {
    let mut app = new_app();
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.newSet", json!({"name": "Keep"})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    let id = ask(&mut app, ActionsEvent::ClearAll);
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert_eq!(sets(&app), ["Keep"]);
    let id = ask(&mut app, ActionsEvent::ClearAll);
    app.run("actions.newSet", json!({"name": "Newer"})).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(app.ui.status_error, "the stale answer says why nothing was cleared");
    assert_eq!(sets(&app), ["Keep", "Newer"]);
    let id = ask(&mut app, ActionsEvent::ClearAll);
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(sets(&app).is_empty());
    assert_eq!(library::edit_undo_label(&app), Some("Undo Clear All Actions"));
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(library::edit_undo_label(&app).is_none());
    crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(sets(&app).is_empty(), "the newer document edit was undone first");
    assert_eq!(library::edit_undo_label(&app), Some("Undo Clear All Actions"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sets(&app), ["Keep", "Newer"]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// Reset's Append keeps the list and adds the starting set; OK puts the starting set in its place.
#[test]
fn reset_appends_or_replaces_the_list() {
    let mut app = new_app();
    let ctx = app.ctx().clone();
    app.run("actions.newSet", json!({"name": "Mine"})).unwrap();
    let id = ask(&mut app, ActionsEvent::Reset);
    app.framework.dialogs.set_json_field(&ctx, id, "choice", json!("alternate")).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(sets(&app).contains(&"Mine") && sets(&app).contains(&photocraft_engine::actions_edit_cmds::BASICS_SET), "{:?}", sets(&app));
    let id = ask(&mut app, ActionsEvent::Reset);
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(sets(&app), [photocraft_engine::actions_edit_cmds::BASICS_SET]);
}

/// Replace reads and counts the chosen file before asking; the list stays until OK, which
/// replaces it with exactly what was read.
#[test]
fn replace_holds_the_read_file_until_ok() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Contents(vec![("replacement.atn".into(), action_set_bytes("Replacement", "Invert"))])]));
    let mut app = super::runner_tests::disk_app(files);
    let ctx = app.ctx().clone();
    app.run("actions.newSet", json!({"name": "Original"})).unwrap();
    super::apply_actions_events(&mut app, vec![ActionsEvent::Replace]);
    app.poll_file_dialog();
    let id = question(&app);
    assert_eq!(sets(&app), ["Original"]);
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(sets(&app), ["Replacement"]);
    assert_eq!(app.session.actions().list.actions().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Invert"]);
}

/// The chosen path and the bytes read need the automation policy, and the bytes need it again on OK.
#[test]
fn replace_checks_the_chosen_source_and_again_on_ok() {
    let files = Arc::new(ScriptedFiles::answering([
        Picked::Paths(vec!["blocked-actions.atn".into()]),
        Picked::Contents(vec![("replacement.atn".into(), action_set_bytes("Replacement", "Invert"))]),
    ]));
    let permit_data = Rc::new(Cell::new(true));
    let policy = Rc::clone(&permit_data);
    let automation_command = Box::new(move |id: &str, p: &Value| -> Result<(), String> {
        if id != "actions.replace" {
            return Ok(());
        }
        if p.get("path").is_some() {
            return Err("path denied by policy".into());
        }
        if p.get("data").is_some() && policy.get() { Ok(()) } else { Err("payload denied by policy".into()) }
    });
    let services = crate::Services { files: Some(files), automation_command: Some(automation_command), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("actions.newSet", json!({"name": "Keep"})).unwrap();
    for attempt in 0..2 {
        app.automation_input = true;
        super::apply_actions_events(&mut app, vec![ActionsEvent::Replace]);
        app.automation_input = false;
        app.poll_file_dialog();
        if attempt == 0 {
            assert!(app.ui.status.contains("path denied by policy"), "{}", app.ui.status);
            assert!(app.framework.dialogs.get::<StandardPrompt>().is_none());
        }
    }
    let id = question(&app);
    permit_data.set(false);
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(app.ui.status.contains("payload denied by policy"), "{}", app.ui.status);
    assert_eq!(sets(&app), ["Keep"]);
}
