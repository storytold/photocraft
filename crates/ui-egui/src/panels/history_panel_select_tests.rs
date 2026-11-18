//! What the History panel's events do (#2966): a click on a state returns the document there with one command
//! (`history.selectState`), not one Undo / Redo per state, so the canvas refreshes once and only where the jump
//! changed pixels. The framework's `HistoryPanel` reports `HistoryEvent`s; these tests run them through
//! `history::apply` and read the rows, the deletable rule and the reveal rule it is built from.

use craft::keymap::Mods;
use craft::ui::patterns::panels::{HistoryEvent, HistoryKey};
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use serde_json::json;

use super::history::{self, apply, can_delete, rows};
use super::*;

/// New (the opening state of File › New), then New Layer, Fill, New Layer, Fill.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    for (cmd, p) in
        [("layer.new.layer", json!({})), ("edit.fill", json!({"color": "#ff0000"})), ("layer.new.layer", json!({})), ("edit.fill", json!({"color": "#00ff00"}))]
    {
        app.run(cmd, p).unwrap();
    }
    app
}

/// New, New Layer, Fill red, a snapshot, Fill blue.
fn with_snapshot() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("history.newSnapshot", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app
}

/// (past states, redo states, revision) of the active document.
fn state(app: &PhotocraftApp) -> (usize, usize, u64) {
    let st = app.session.active().unwrap();
    (st.history.past_len(), st.history.redo_labels().count(), st.revision)
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The key of history row `row`: 0 is the oldest state, then the current one, then the redo states.
fn state_key(app: &PhotocraftApp, row: usize) -> HistoryKey {
    HistoryKey::State(app.session.active().unwrap().history.row_id(row).expect("a row"))
}

/// The key of the current state's row.
fn current_key(app: &PhotocraftApp) -> HistoryKey {
    state_key(app, app.session.active().unwrap().history.past_len())
}

fn snapshots(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().snapshots.len()
}

fn last_snapshot(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().snapshots.last().unwrap().id
}

fn top_pixel(app: &PhotocraftApp) -> [f32; 4] {
    app.session.active().unwrap().doc.layers.last().unwrap().surface().unwrap().rgba(1, 1)
}

/// Whether the newest open dialog is New Snapshot.
fn snapshot_dialog_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.open_dialogs().last().is_some_and(|d| d.key.as_deref().is_some_and(crate::snapshot_dialog::owns))
}

#[test]
fn clicking_a_state_jumps_there_with_one_command() {
    let mut app = app();
    let (past, redo, rev) = state(&app);
    assert_eq!((past, redo), (4, 0));
    // Back three states: one command, one revision (the canvas sees one change).
    let event = HistoryEvent::Select(state_key(&app, 1));
    apply(&mut app, event);
    assert_eq!(state(&app), (1, 3, rev + 1));
    assert_eq!(app.perf.last_command, "history.selectState");
    // Forward to the last (dimmed) state, again in one step.
    let event = HistoryEvent::Select(state_key(&app, 4));
    apply(&mut app, event);
    assert_eq!(state(&app), (4, 0, rev + 2));
    // The current state: nothing to do.
    let event = HistoryEvent::Select(state_key(&app, 4));
    apply(&mut app, event);
    assert_eq!(state(&app), (4, 0, rev + 2));
}

/// #14029: a document from File › New lists its opening state as "New" (an opened file's reads "Open"), first among
/// the states, with the opening state's icon.
#[test]
fn a_new_documents_opening_state_reads_new_with_the_opening_icon() {
    let app = app();
    let listed = rows(app.session.active().unwrap());
    assert!(listed.iter().all(|r| r.label.as_str() != "Open"), "no row reads Open");
    let first_state = listed.iter().find(|r| matches!(r.key, HistoryKey::State(_))).expect("the opening state");
    assert_eq!(first_state.label.as_str(), "New");
    assert_eq!(first_state.icon, Some(history::step_icon("Open")), "the opening state's icon");
}

/// The rows are the snapshots, then the states with the later ones dimmed; a state's key survives a step back.
#[test]
fn rows_list_snapshots_then_states_then_redo_states() {
    let mut app = with_snapshot();
    app.run("edit.undo", json!({})).unwrap();
    let before = rows(app.session.active().unwrap());
    let kinds: Vec<(bool, bool)> = before.iter().map(|r| (matches!(r.key, HistoryKey::Snapshot(_)), r.redo)).collect();
    // The first snapshot, "Snapshot 1", then New, New Layer, Fill (current), Fill (redo).
    assert_eq!(kinds, [(true, false), (true, false), (false, false), (false, false), (false, false), (false, true)]);
    app.run("edit.redo", json!({})).unwrap();
    let after = rows(app.session.active().unwrap());
    assert_eq!(after[5].key, before[5].key, "the state keeps its key when it becomes current");
    assert!(!after[5].redo);
}

/// While Free Transform is open, clicking a History state leaves the document alone: the transform owns
/// Undo. Without a transform the same click steps back as usual.
#[test]
fn history_rows_wait_for_an_open_transform() {
    for transforming in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        if transforming {
            let app_ctx = app.ctx().clone();
            crate::transform_tool::begin(&mut app, &app_ctx).unwrap();
        }
        let before = steps(&app);
        let event = HistoryEvent::Select(state_key(&app, 0));
        apply(&mut app, event);
        let after = steps(&app);
        if transforming {
            assert_eq!(after, before, "the document's history is untouched");
            assert!(app.ui.transform.is_some());
        } else {
            assert!(after < before, "the click steps back: {before} -> {after}");
        }
    }
}

/// Snapshot rows (#2966): selecting one returns to it as a new state and selects it, a rename keeps the
/// states, and a delete removes it.
#[test]
fn snapshot_rows_select_rename_and_delete() {
    let mut app = with_snapshot();
    let id = last_snapshot(&app);
    let before = steps(&app);
    apply(&mut app, HistoryEvent::Select(HistoryKey::Snapshot(id)));
    assert_eq!(app.perf.last_command, "history.selectSnapshot");
    assert_eq!(steps(&app), before + 1);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("Snapshot 1"));
    assert_eq!(top_pixel(&app), [1.0, 0.0, 0.0, 1.0], "back to red");
    assert_eq!(selected_snapshot(&app), Some(id), "the snapshot shows selected");

    // Clicking the current state while a snapshot is selected only clears the selection.
    let revision = state(&app).2;
    let event = HistoryEvent::Select(state_key(&app, state(&app).0));
    apply(&mut app, event);
    assert_eq!(selected_snapshot(&app), None);
    assert_eq!(state(&app).2, revision, "and runs no command");

    apply(&mut app, HistoryEvent::rename(id, "Lesson start"));
    assert_eq!(app.perf.last_command, "history.renameSnapshot");
    let names: Vec<String> = app.session.active().unwrap().snapshots.iter().skip(1).map(|s| s.name.clone()).collect();
    assert_eq!(names, ["Lesson start"]);

    let kept = steps(&app);
    apply(&mut app, HistoryEvent::Delete(Some(HistoryKey::Snapshot(id))));
    assert_eq!(snapshots(&app), 1, "the snapshot is gone, the first one stays");
    assert_eq!(steps(&app), kept, "deleting leaves the history alone");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Clicking a snapshot selects it, and the trash then deletes the snapshot instead of a state (#5324). Any
/// other step deselects it, and the trash deletes the current state again.
#[test]
fn the_trash_deletes_the_selected_snapshot() {
    let mut app = with_snapshot();
    let event = HistoryEvent::Select(HistoryKey::Snapshot(last_snapshot(&app)));
    apply(&mut app, event);
    let before = steps(&app);
    apply(&mut app, HistoryEvent::Delete(None));
    assert_eq!(snapshots(&app), 1, "the trash deleted the snapshot, leaving the first one");
    assert_eq!(steps(&app), before, "and left the states alone");

    // An edit after the click deselects the snapshot: the trash deletes the state.
    let mut app = with_snapshot();
    let event = HistoryEvent::Select(HistoryKey::Snapshot(last_snapshot(&app)));
    apply(&mut app, event);
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    assert_eq!(selected_snapshot(&app), None, "an edit deselects the snapshot");
    let before = steps(&app);
    apply(&mut app, HistoryEvent::Delete(None));
    assert_eq!(snapshots(&app), 2, "the snapshot is kept, after the first one");
    assert_eq!(steps(&app), before - 1, "the current state is deleted");
}

/// A snapshot or a past state dragged onto the trash is deleted (#5324); the oldest state is not deletable.
#[test]
fn rows_dragged_onto_the_trash_are_deleted() {
    let mut app = with_snapshot();
    let before = steps(&app);
    assert!(can_delete(&app, &HistoryKey::Snapshot(last_snapshot(&app))));
    let event = HistoryEvent::Delete(Some(HistoryKey::Snapshot(last_snapshot(&app))));
    apply(&mut app, event);
    assert_eq!(snapshots(&app), 1, "the dragged snapshot is deleted, leaving the first one");
    assert_eq!(steps(&app), before, "no state is");

    // The second state: it and the states after it go, as with Delete. The states are New, New Layer, Fill, Fill.
    let mut app = with_snapshot();
    assert_eq!(steps(&app), 4);
    assert!(!can_delete(&app, &state_key(&app, 0)), "the oldest state stays");
    let event = HistoryEvent::Delete(Some(state_key(&app, 0)));
    apply(&mut app, event);
    assert_eq!(steps(&app), 4, "a refused row deletes nothing");
    let key = state_key(&app, 1);
    assert!(can_delete(&app, &key));
    apply(&mut app, HistoryEvent::Delete(Some(key)));
    assert_eq!(app.perf.last_command, "history.deleteState");
    assert_eq!(steps(&app), 1, "the dragged state and the ones after it are deleted");
    assert_eq!(snapshots(&app), 2, "the snapshot is kept, after the first one");
}

/// With Allow Non-Linear History and only the oldest state current, a click on the trash has nothing to delete
/// (the oldest state stays, as in Photoshop); a later (dimmed) state dragged onto it is still deleted, and once
/// a past state exists again the click works.
#[test]
fn non_linear_trash_click_never_targets_the_oldest_state() {
    let mut app = with_snapshot();
    let last = |app: &PhotocraftApp| state_key(app, steps(app) + app.session.active().unwrap().history.redo_labels().count() - 1);
    assert!(!can_delete(&app, &HistoryKey::State(u64::MAX)), "a state that is not held is not deletable");
    app.run("history.options", json!({"allowNonLinearHistory": true})).unwrap();
    for _ in 0..3 {
        app.run("edit.undo", json!({})).unwrap();
    }
    let counts = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        (st.history.past_len(), st.history.redo_labels().count())
    };
    assert_eq!(counts(&app), (0, 3), "New current, New Layer, Fill, Fill dimmed");
    assert!(app.session.is_enabled("history.deleteState"), "the engine still offers a later state to delete");
    assert!(!can_delete(&app, &state_key(&app, 0)), "the oldest state is never deletable");
    assert!(can_delete(&app, &last(&app)), "a later state is, with the option on");

    apply(&mut app, HistoryEvent::Delete(None));
    assert_eq!(counts(&app), (0, 3), "the click deletes nothing");
    assert!(!app.ui.status_error, "and reports no error: {}", app.ui.status);

    // The last dimmed Fill dragged onto the trash: only it goes.
    let event = HistoryEvent::Delete(Some(last(&app)));
    apply(&mut app, event);
    assert_eq!(counts(&app), (0, 2), "the dragged later state is deleted");

    // A past state again: the click deletes the current one.
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(counts(&app).0, 1);
    apply(&mut app, HistoryEvent::Delete(None));
    assert_eq!(counts(&app).0, 0, "the click deleted the current state");
}

/// Without Allow Non-Linear History a later (dimmed) state is not deletable.
#[test]
fn a_later_state_is_not_deletable_without_the_option() {
    let mut app = with_snapshot();
    app.run("edit.undo", json!({})).unwrap();
    let redo = state_key(&app, steps(&app));
    assert!(!can_delete(&app, &redo));
    apply(&mut app, HistoryEvent::Delete(Some(redo)));
    assert_eq!(app.session.active().unwrap().history.redo_labels().count(), 1, "nothing was deleted");
}

/// Runs a command the way the command dispatch does: a history mark before it, the reveal filed after.
fn run_filed(app: &mut PhotocraftApp, cmd: &str, params: serde_json::Value) {
    let before = history::mark(app);
    app.run(cmd, params).unwrap();
    history::file(app, before);
}

/// Reveal asks for the current row when the history moved, never for a change that adds no state (#5909).
#[test]
fn scrolling_away_survives_a_change_that_adds_no_state() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    run_filed(&mut app, "file.new", json!({"width": 64, "height": 48}));
    assert_eq!(history::take(&mut app), Some(current_key(&app)), "a document that came up shows its current state");
    assert_eq!(history::take(&mut app), None, "the draw took it");
    run_filed(&mut app, "layer.new.layer", json!({}));
    run_filed(&mut app, "edit.fill", json!({"color": "#ff0000"}));
    let _ = history::take(&mut app);
    // Selecting a layer bumps the revision but adds no state: the list stays where it is.
    let id = app.session.active().unwrap().doc.layers[0].id;
    let before = history::mark(&app);
    app.session.select_layer(id).unwrap();
    history::file(&mut app, before);
    assert_eq!(history::take(&mut app), None, "a layer click must not scroll History");
    // A new state brings the current row back into view.
    run_filed(&mut app, "layer.new.layer", json!({}));
    let key = history::take(&mut app);
    assert_eq!(key, Some(current_key(&app)), "a new state scrolls into view");
    // Undo moves the history: the current row (and the dimmed one after it) is shown.
    run_filed(&mut app, "edit.undo", json!({}));
    assert_eq!(history::take(&mut app), Some(current_key(&app)));
}

/// At the history limit a new step drops the oldest: the counts and the (same) name stay as they were, but the
/// row id of the current state changes.
#[test]
fn a_new_state_at_the_history_limit_still_scrolls_into_view() {
    let mut app = app();
    let st = app.session.active_mut().unwrap();
    st.history.max_states = st.history.past_len();
    let past = app.session.active().unwrap().history.past_len();
    run_filed(&mut app, "layer.new.layer", json!({}));
    assert_eq!(app.session.active().unwrap().history.past_len(), past, "the oldest state was dropped");
    assert!(history::take(&mut app).is_some(), "a new state at the limit scrolls into view");
}

/// A selected snapshot keeps the list where the user put it.
#[test]
fn a_selected_snapshot_asks_for_no_reveal() {
    let mut app = with_snapshot();
    let before = history::mark(&app);
    let event = HistoryEvent::Select(HistoryKey::Snapshot(last_snapshot(&app)));
    apply(&mut app, event);
    history::file(&mut app, before);
    assert_eq!(history::take(&mut app), None, "selecting a snapshot adds a state but shows the snapshot row");
}

#[test]
fn a_new_state_at_the_history_limit_deselects_the_snapshot() {
    let mut app = app();
    app.run("history.newSnapshot", json!({})).unwrap();
    // What a click on the snapshot row does: the command, then the selection at the state it made.
    let id = app.session.active().unwrap().snapshots[0].id;
    app.run("history.selectSnapshot", json!({"snapshot": id})).unwrap();
    select_snapshot(&mut app, id);
    assert_eq!(selected_snapshot(&app), Some(id), "the snapshot is selected");
    // At the limit, selecting the snapshot again by command adds a state with the same name and
    // drops the oldest: only the newest state tells the two apart.
    let st = app.session.active_mut().unwrap();
    st.history.max_states = st.history.past_len();
    let past = st.history.past_len();
    app.run("history.selectSnapshot", json!({"snapshot": id})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), past, "the oldest state was dropped");
    assert_eq!(selected_snapshot(&app), None, "a new state deselects the snapshot");
}

/// ⌥-click on the camera opens New Snapshot (#2966): the name and From; OK takes the snapshot. No snapshot is
/// taken before.
#[test]
fn alt_click_on_the_camera_opens_new_snapshot() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    apply(&mut app, HistoryEvent::new_snapshot(Mods::ALT));
    assert_eq!(snapshots(&app), 1, "⌥-click opens the dialog instead");
    assert!(snapshot_dialog_open(&app));
}

/// History Options › Show New Snapshot Dialog by Default (#2966): a plain click on the camera opens New
/// Snapshot as ⌥-click does, and ⌥-click still opens it (Photoshop's button always prompts with the option
/// on). Off, a plain click takes the snapshot at once.
#[test]
fn show_new_snapshot_dialog_by_default_makes_a_click_open_the_dialog() {
    for (dialog_by_default, mods) in [(true, Mods::NONE), (true, Mods::ALT), (false, Mods::NONE)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("history.options", json!({"showNewSnapshotDialogByDefault": dialog_by_default})).unwrap();
        apply(&mut app, HistoryEvent::new_snapshot(mods));
        let dialog = snapshot_dialog_open(&app);
        if dialog_by_default {
            assert!(dialog, "{mods:?}: the dialog opens");
            assert_eq!(snapshots(&app), 1, "{mods:?}: no snapshot until OK");
        } else {
            assert!(!dialog, "off: no dialog");
            assert_eq!(snapshots(&app), 2, "off: the click takes the snapshot");
        }
    }
}

/// The top row is the first snapshot, the document as opened: selecting it brings that back (the Discord report
/// on #2966: "History original file click does not work").
#[test]
fn the_top_row_returns_to_the_document_as_opened() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let opened = app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(1, 1);
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let name = app.session.active().unwrap().doc.name.clone();
    let top = rows(app.session.active().unwrap()).remove(0);
    assert_eq!(top.label.as_str(), name, "the top row is named after the document");
    apply(&mut app, HistoryEvent::Select(top.key));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].surface().unwrap().rgba(1, 1), opened);
    assert_eq!(st.history.entries().last(), Some(&name));
}

/// History Options › Automatically Create First Snapshot: on, the panel's top row is a snapshot named after the
/// document; off, there is none above the opening state.
#[test]
fn the_first_snapshot_row_follows_the_option() {
    for first in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("history.options", json!({"automaticallyCreateFirstSnapshot": first})).unwrap();
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        let name = app.session.active().unwrap().doc.name.clone();
        let named = rows(app.session.active().unwrap()).iter().any(|r| matches!(r.key, HistoryKey::Snapshot(_)) && r.label.as_str() == name);
        assert_eq!(named, first, "a snapshot row named after the document: {first}");
    }
}

/// Snapshot thumbnails belong to the snapshot's document (#6087): a document reopened with the same id (a
/// `.pcraft` keeps it) gets fresh ones rather than the closed document's, even before the views are synced.
#[test]
fn snapshot_thumbnails_follow_their_document() {
    use crate::app::thumbs::SnapshotThumbs;

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let doc = (*app.session.active().unwrap().doc).clone();
    let doc_id = doc.id;
    let request = ThumbRequest::for_cell(32.0, 1.0);
    let revision = |app: &PhotocraftApp| SnapshotThumbs::new(app, doc_id).revision(&0, &request);
    let first = revision(&app);
    assert_eq!(first, revision(&app), "the same picture keeps its revision");
    // The revision follows the snapshot document's `Arc` address, so each closed document stays alive for the
    // test: a freed address handed to the next document would read as the same picture.
    let mut closed = vec![app.session.active().unwrap().snapshots[0].doc.clone()];
    // Closed and reopened with the same id.
    app.session.close(0);
    app.sync_views();
    app.session.add_document(doc.clone(), None);
    assert_eq!(app.session.active().unwrap().doc.id, doc_id, "same id");
    let reopened = revision(&app);
    assert_ne!(reopened, first);
    // Closed and reopened with no sync in between: still a fresh one.
    closed.push(app.session.active().unwrap().snapshots[0].doc.clone());
    app.session.close(0);
    app.session.add_document(doc, None);
    assert_ne!(revision(&app), reopened);
}
