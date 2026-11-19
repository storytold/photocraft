//! Saved-path positions must not survive a list change or document-history restoration.
//! These regressions exercise the real command/action routes, without retaining old documents.
use crate::{
    PhotocraftApp, Services,
    state::{PathSelection, Tool},
};
use photocraft_doc::NamedPath;
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::{Value, json};

fn path(x: i32) -> Value {
    json!({"subpaths":[{"knots":[[x,10],[x+10,10],[x+10,20]]}]})
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width":100,"height":100})).unwrap();
    for (name, x) in [("A", 10), ("B", 40), ("C", 70)] {
        app.run("path.set", json!({"name":name,"path":path(x)})).unwrap();
    }
    app.ui.tool = Tool::PathSelection;
    app
}

fn paths(app: &PhotocraftApp) -> Vec<NamedPath> {
    app.session.active().unwrap().doc.paths.clone()
}

fn select(app: &mut PhotocraftApp, index: usize) {
    app.ui.selected_path = Some(PathSelection::Saved { index });
}

fn assert_cleared(app: &PhotocraftApp) {
    let actual = app.ui.selected_path.as_ref().and_then(|p| p.saved_index(&app.session.active().unwrap().doc));
    assert!(app.ui.selected_path.is_none(), "stale selection {:?} now targets {:?}", app.ui.selected_path, actual.map(|i| paths(app)[i].name.clone()));
}

fn install_replace(app: &mut PhotocraftApp, fail: bool) {
    let mut steps = vec![("path.delete".into(), json!({"pathIndex":0})), ("path.set".into(), json!({"name":"D","path":path(80)}))];
    if fail {
        steps.push(("path.delete".into(), json!({"pathIndex":99})));
    }
    app.session.actions.list.push(action("Replace", steps)).unwrap();
}

fn play_replace(app: &mut PhotocraftApp) {
    install_replace(app, false);
    let r = app.run("actions.play", json!({"action":"Replace"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
}

#[test]
fn coalesced_delete_invalidates_a_still_in_range_index() {
    let mut app = app();
    app.run("path.rename", json!({"pathIndex":2,"to":"C1","coalesce":"paths"})).unwrap();
    select(&mut app, 1);
    let before = paths(&app);
    let steps = app.session.active().unwrap().history.past_len();
    app.run("path.delete", json!({"pathIndex":0,"coalesce":"paths"})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "the deletion really coalesced");
    assert_eq!(paths(&app), before[1..]);
    assert_cleared(&app);
}

#[test]
fn action_delete_then_append_invalidates_even_when_final_count_is_unchanged() {
    for grouped in [false, true] {
        for engine_route in [false, true] {
            let mut app = app();
            app.session.edit_prefs(|p| p.actions.create_single_history_state = grouped);
            install_replace(&mut app, false);
            // Also cover an action nested inside another action.
            app.session.actions.list.push(action("Outer", vec![("actions.play".into(), json!({"action":"Replace"}))])).unwrap();
            select(&mut app, 1);
            let r = if engine_route {
                let r = app.session.execute("actions.play", json!({"action":"Outer"})).unwrap();
                app.sync_views();
                r
            } else {
                app.run("actions.play", json!({"action":"Outer"})).unwrap()
            };
            assert!(r.get("failed").is_none(), "{r}");
            assert_eq!(paths(&app).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B", "C", "D"]);
            assert_cleared(&app);
        }
    }
}

#[test]
fn a_failed_actions_applied_prefix_still_invalidates_selection() {
    let mut app = app();
    install_replace(&mut app, true);
    select(&mut app, 1);
    let r = app.run("actions.play", json!({"action":"Replace"})).unwrap();
    assert!(r.get("failed").is_some());
    assert_eq!(paths(&app).len(), 3);
    assert_cleared(&app);
}

fn grouped_replacement() -> (PhotocraftApp, Vec<NamedPath>, Vec<NamedPath>) {
    let mut app = app();
    app.session.edit_prefs(|p| p.actions.create_single_history_state = true);
    let before = paths(&app);
    play_replace(&mut app);
    let after = paths(&app);
    (app, before, after)
}

fn history_route(command: &str, forward: bool, route: &str) {
    let (mut app, before, after) = grouped_replacement();
    if forward {
        app.run("edit.undo", json!({})).unwrap();
    }
    select(&mut app, 0);
    if route == "action" {
        app.session.actions.list.push(action("Navigate", vec![(command.into(), json!({}))])).unwrap();
        let r = app.run("actions.play", json!({"action":"Navigate"})).unwrap();
        assert!(r.get("failed").is_none(), "{command}: {r}");
    } else if route == "menu" {
        crate::menus::invoke(&mut app, &egui::Context::default(), command, json!({})).unwrap();
    } else {
        app.run(command, json!({})).unwrap();
    }
    assert_eq!(paths(&app), if forward { after } else { before }, "{command}");
    assert_cleared(&app);
}

macro_rules! history_case {
    ($name:ident, $command:literal, $forward:literal) => {
        #[test]
        fn $name() {
            for route in ["command", "action", "menu"] {
                history_route($command, $forward, route);
            }
        }
    };
}
history_case!(edit_undo, "edit.undo", false);
history_case!(edit_redo, "edit.redo", true);
history_case!(history_undo, "history.undo", false);
history_case!(history_redo, "history.redo", true);
history_case!(step_backward, "history.stepBackward", false);
history_case!(step_forward, "history.stepForward", true);
history_case!(toggle_backward, "edit.toggleLastState", false);
history_case!(toggle_forward, "edit.toggleLastState", true);

#[test]
fn history_state_jumps_invalidate_in_both_directions() {
    let (mut app, before, after) = grouped_replacement();
    let current = app.session.active().unwrap().history.past_len();
    for (state, expected) in [(current - 1, before), (current, after)] {
        select(&mut app, 0);
        app.run("history.selectState", json!({"state":state})).unwrap();
        assert_eq!(paths(&app), expected);
        assert_cleared(&app);
    }
}

#[test]
fn restoring_a_snapshot_with_the_same_path_count_invalidates_selection() {
    let mut app = app();
    let before = paths(&app);
    let snapshot = app.run("history.newSnapshot", json!({"name":"Paths"})).unwrap()["snapshot"].clone();
    play_replace(&mut app);
    select(&mut app, 0);
    app.run("history.selectSnapshot", json!({"snapshot":snapshot})).unwrap();
    assert_eq!(paths(&app), before);
    assert_cleared(&app);
}

#[test]
fn deleting_current_history_state_invalidates_in_linear_and_non_linear_mode() {
    for non_linear in [false, true] {
        let (mut app, before, _) = grouped_replacement();
        app.run("history.options", json!({"allowNonLinearHistory":non_linear})).unwrap();
        select(&mut app, 0);
        app.run("history.deleteState", json!({})).unwrap();
        assert_eq!(paths(&app), before);
        assert_cleared(&app);
    }
}

#[test]
fn direct_session_undo_redo_and_rollback_invalidate_on_sync() {
    for operation in ["undo", "redo", "rollback"] {
        let (mut app, before, after) = grouped_replacement();
        if operation == "redo" {
            app.run("edit.undo", json!({})).unwrap();
        }
        select(&mut app, 0);
        assert!(match operation {
            "undo" => app.session.undo(),
            "redo" => app.session.redo(),
            _ => app.session.rollback(),
        });
        app.sync_views();
        assert_eq!(paths(&app), if operation == "redo" { after } else { before });
        assert_cleared(&app);
    }
}

#[test]
fn no_op_or_rejected_history_commands_preserve_selection() {
    let mut app = app();
    select(&mut app, 1);
    let before = paths(&app);
    let state = app.session.active().unwrap().history.past_len();
    app.run("history.selectState", json!({"state":state})).unwrap();
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    for (id, params) in [
        ("edit.redo", json!({})),
        ("history.selectState", json!({"state":-1})),
        ("history.stepForward", json!({})),
        ("history.selectSnapshot", json!({"snapshot":999})),
    ] {
        assert!(app.run(id, params).is_err());
        assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    }
    // Clearing/restoring the history rows changes metadata, not the document's paths.
    app.run("history.clear", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    assert_eq!(paths(&app), before);
}

#[test]
fn ordinary_edits_preserve_a_valid_saved_path_selection() {
    let mut app = app();
    select(&mut app, 1);
    for (id, params) in [
        ("path.rename", json!({"pathIndex":1,"to":"Renamed","coalesce":"paths"})),
        ("path.set", json!({"pathIndex":1,"path":path(45),"coalesce":"paths"})),
        ("select.rect", json!({"x":0,"y":0,"width":5,"height":5})),
    ] {
        app.run(id, params).unwrap();
        assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    }
}

#[test]
fn deleting_a_non_current_history_row_preserves_selection() {
    let (mut app, _, before) = grouped_replacement();
    app.run("history.options", json!({"allowNonLinearHistory":true})).unwrap();
    select(&mut app, 1);
    app.run("history.deleteState", json!({"state":1})).unwrap();
    assert_eq!(paths(&app), before);
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
}

#[test]
fn grouped_action_history_round_trip_does_not_restore_the_invalidation_token() {
    let mut app = app();
    app.session.edit_prefs(|p| p.actions.create_single_history_state = true);
    let before = paths(&app);
    let steps = app.session.active().unwrap().history.past_len();
    app.session.actions.list.push(action("Round trip", vec![("path.delete".into(), json!({"pathIndex":0})), ("edit.undo".into(), json!({}))])).unwrap();
    select(&mut app, 1);
    let r = app.run("actions.play", json!({"action":"Round trip"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(paths(&app), before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_cleared(&app);
}

#[test]
fn out_of_range_ui_selection_is_cleared_without_a_document_change() {
    let mut app = app();
    let before = paths(&app);
    select(&mut app, usize::MAX);
    app.sync_views();
    assert_cleared(&app);
    assert_eq!(paths(&app), before);
}

#[test]
fn pen_draft_undo_does_not_invalidate_saved_path_positions() {
    use crate::canvas::testing::{ToolEvent, tool_event};
    for command in ["edit.undo", "history.stepBackward"] {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        select(&mut app, 1);
        for (x, y) in [(25.0, 60.0), (65.0, 60.0)] {
            tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
        }
        assert_eq!(app.ui.pen.as_ref().unwrap().draft.knots.len(), 2);
        let before = paths(&app);
        let steps = app.session.active().unwrap().history.past_len();
        crate::menus::invoke(&mut app, &egui::Context::default(), command, json!({})).unwrap();
        app.sync_views();
        assert_eq!(app.ui.pen.as_ref().unwrap().draft.knots.len(), 1);
        assert_eq!(paths(&app), before);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    }
}

#[test]
fn revert_same_count_invalidates_saved_index() {
    for (in_action, grouped) in [(false, false), (true, false), (true, true)] {
        let mut app = app();
        app.session.edit_prefs(|p| p.actions.create_single_history_state = grouped);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("saved.pcraft");
        app.run("file.saveACopy", json!({"path":file})).unwrap();
        app.run("file.openAs", json!({"path":file})).unwrap();
        let saved = paths(&app);
        play_replace(&mut app);
        let edited = paths(&app);
        select(&mut app, 0);
        if in_action {
            app.session.actions.list.push(action("Revert", vec![("file.revert".into(), json!({}))])).unwrap();
            let r = app.run("actions.play", json!({"action":"Revert"})).unwrap();
            assert!(r.get("failed").is_none(), "{r}");
        } else {
            app.run("file.revert", json!({})).unwrap();
        }
        assert_eq!(paths(&app), saved);
        assert_cleared(&app);
        // Revert is itself one undoable step, and neither direction can reuse its selection.
        for (command, expected) in [("edit.undo", &edited), ("edit.redo", &saved)] {
            select(&mut app, 0);
            app.run(command, json!({})).unwrap();
            assert_eq!(&paths(&app), expected);
            assert_cleared(&app);
        }
    }
}

#[test]
fn identical_geometry_component_selection_is_cleared_after_shift() {
    use crate::canvas::testing::{ToolEvent, tool_event};
    let mut app = app();
    for index in 0..3 {
        app.run("path.set", json!({"pathIndex":index,"path":path(40)})).unwrap();
    }
    select(&mut app, 1);
    tool_event(&mut app, ToolEvent::Down { x: 45.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 45.0, y: 12.0 }, egui::Modifiers::NONE);
    assert!(!app.ui.direct_selection.anchors.is_empty());
    app.run("path.delete", json!({"pathIndex":0})).unwrap();
    assert_cleared(&app);
    assert!(app.ui.direct_selection.target.is_none(), "stale component selection {:?}", app.ui.direct_selection.target);
}

#[test]
fn history_snapshot_and_delete_inside_actions() {
    for command in ["history.selectState", "history.selectSnapshot", "history.deleteState"] {
        for non_linear in [false, true] {
            let mut app = app();
            app.run("history.options", json!({"allowNonLinearHistory":non_linear})).unwrap();
            app.session.edit_prefs(|p| p.actions.create_single_history_state = true);
            let before = paths(&app);
            let state = app.session.active().unwrap().history.past_len();
            let snap = app.run("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
            play_replace(&mut app);
            select(&mut app, 0);
            let params = match command {
                "history.selectState" => json!({"state":state}),
                "history.selectSnapshot" => json!({"snapshot":snap}),
                _ => json!({}),
            };
            app.session.actions.list.push(action("Navigate extra", vec![(command.into(), params)])).unwrap();
            let r = app.run("actions.play", json!({"action":"Navigate extra"})).unwrap();
            assert!(r.get("failed").is_none(), "{command}: {r}");
            assert_eq!(paths(&app), before, "{command}");
            assert_cleared(&app);
        }
    }
}

#[test]
fn reused_index_does_not_transfer_component_selection_to_another_row() {
    use crate::canvas::testing::{ToolEvent, tool_event};
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    for had_components in [false, true] {
        let mut app = app();
        let geom = json!({"subpaths":[
            {"closed":true,"knots":[[10,10],[30,10],[30,30],[10,30]]},
            {"closed":true,"knots":[[60,10],[80,10],[80,30],[60,30]]}
        ]});
        for index in 0..3 {
            app.run("path.set", json!({"pathIndex":index,"path":geom})).unwrap();
        }
        crate::panels::panel_docking::show_panel(&mut app, "paths", Some(true)).unwrap();
        let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app
        });
        h.run_steps(6);
        h.get_all_by_label("B").find(|n| n.accesskit_node().is_selected().is_some()).unwrap().click();
        h.run_steps(3);
        if had_components {
            tool_event(h.state_mut(), ToolEvent::Down { x: 70.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(h.state_mut(), ToolEvent::Up { x: 70.0, y: 20.0 }, egui::Modifiers::NONE);
            assert!(h.state().ui.direct_selection.anchors.iter().all(|a| a[0] == 1));
            assert!(!h.state().ui.direct_selection.anchors.is_empty());
        }
        h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
        h.run_steps(3);
        assert_cleared(h.state());
        // A fresh row click targets C; it must not inherit B's selected second component.
        h.get_by_role_and_label(egui::accesskit::Role::ListItem, "C").click();
        h.run_steps(3);
        let before = paths(h.state());
        for pressed in [true, false] {
            h.event(egui::Event::Key { key: egui::Key::Minus, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(3);
        assert_eq!(paths(h.state()), before, "Minus edited C through B's stale component selection; had_components={had_components}");
        // A new canvas selection can still edit C, and the edit has normal undo/redo semantics.
        tool_event(h.state_mut(), ToolEvent::Down { x: 70.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(h.state_mut(), ToolEvent::Up { x: 70.0, y: 20.0 }, egui::Modifiers::NONE);
        for pressed in [true, false] {
            h.event(egui::Event::Key { key: egui::Key::Minus, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(3);
        let edited = paths(h.state());
        assert_eq!(edited[0], before[0]);
        assert_eq!(edited[1].path.subpaths[1].op, photocraft_doc::PathOp::Subtract);
        for (command, expected) in [("edit.undo", &before), ("edit.redo", &edited)] {
            h.state_mut().run(command, json!({})).unwrap();
            h.run_steps(3);
            assert_eq!(&paths(h.state()), expected);
            assert!(h.state().ui.direct_selection.target.is_none());
        }
    }
}

#[test]
fn failed_revert_keeps_the_document_and_saved_selection() {
    let mut app = app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("saved.pcraft");
    app.run("file.saveACopy", json!({"path":file})).unwrap();
    app.run("file.openAs", json!({"path":file})).unwrap();
    select(&mut app, 1);
    let before = paths(&app);
    let revision = app.session.active().unwrap().revision;
    let epoch = app.session.active().unwrap().saved_paths_revision();
    std::fs::remove_file(file).unwrap();
    assert!(app.run("file.revert", json!({})).is_err());
    app.session.actions.list.push(action("Missing revert", vec![("file.revert".into(), json!({}))])).unwrap();
    let reply = app.run("actions.play", json!({"action":"Missing revert"})).unwrap();
    assert!(reply.get("failed").is_some());
    app.sync_views();
    assert_eq!(paths(&app), before);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().saved_paths_revision(), epoch);
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
}

fn select_saved_component(app: &mut PhotocraftApp) {
    use crate::canvas::testing::{ToolEvent, tool_event};
    app.ui.tool = Tool::PathSelection;
    select(app, 1);
    tool_event(app, ToolEvent::Down { x: 45.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 45.0, y: 12.0 }, egui::Modifiers::NONE);
    assert!(!app.ui.direct_selection.anchors.is_empty());
}

#[test]
fn history_clears_saved_components_even_when_their_geometry_is_unchanged() {
    for (command, forward) in [
        ("edit.undo", false),
        ("edit.redo", true),
        ("history.undo", false),
        ("history.redo", true),
        ("history.stepBackward", false),
        ("history.stepForward", true),
        ("edit.toggleLastState", false),
        ("edit.toggleLastState", true),
    ] {
        let mut app = app();
        // A change elsewhere leaves B's geometry identical on both sides of history.
        app.run("path.rename", json!({"pathIndex":0,"to":"Renamed"})).unwrap();
        if forward {
            app.run("edit.undo", json!({})).unwrap();
        }
        select_saved_component(&mut app);
        app.run(command, json!({})).unwrap();
        assert_cleared(&app);
        assert!(app.ui.direct_selection.target.is_none(), "{command}");
        assert!(app.ui.direct_selection.anchors.is_empty(), "{command}");
    }
}

#[test]
fn ordinary_rename_and_unrelated_edits_keep_saved_components() {
    let mut app = app();
    select_saved_component(&mut app);
    let target = app.ui.direct_selection.target;
    let anchors = app.ui.direct_selection.anchors.clone();
    for (command, params) in [("path.rename", json!({"pathIndex":1,"to":"Renamed"})), ("select.rect", json!({"x":0,"y":0,"width":5,"height":5}))] {
        app.run(command, params).unwrap();
        assert_eq!(app.ui.direct_selection.target, target);
        assert_eq!(app.ui.direct_selection.anchors, anchors);
    }
}

#[test]
fn stale_saved_anchor_drag_cannot_commit_after_indices_shift() {
    use crate::canvas::testing::{ToolEvent, tool_event};
    let mut app = app();
    app.ui.tool = Tool::DirectSelection;
    select(&mut app, 1);
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 45.0, y: 15.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(crate::direct_select::editing(&app));
    app.run("path.delete", json!({"pathIndex":0})).unwrap();
    let before = paths(&app);
    let steps = app.session.active().unwrap().history.past_len();
    assert!(!crate::direct_select::editing(&app));
    tool_event(&mut app, ToolEvent::Up { x: 45.0, y: 15.0 }, egui::Modifiers::NONE);
    assert_eq!(paths(&app), before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

#[test]
fn alt_drag_duplicates_only_the_selected_same_named_path_and_undo_redo_restore_it() {
    use crate::canvas::testing::{ToolEvent, tool_event};
    let mut app = app();
    app.ui.extras.snap = false;
    app.run("path.set", json!({"pathIndex":1,"path":{"subpaths":[{"closed":true,"knots":[[20,40],[80,40],[80,80],[20,80]]}]}})).unwrap();
    // Imported documents can contain duplicate names even though Rename refuses to create them.
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths[1].name = "A".into();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    select(&mut app, 1);
    let before = paths(&app);
    let steps = app.session.active().unwrap().history.past_len();
    for event in [ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: 58.0, y: 45.0, pressure: 1.0 }, ToolEvent::Up { x: 58.0, y: 45.0 }]
    {
        tool_event(&mut app, event, egui::Modifiers::ALT);
    }
    let after = paths(&app);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[2]);
    assert_eq!(after[1].path.subpaths.len(), 2);
    assert_eq!(after[1].path.subpaths[0], before[1].path.subpaths[0]);
    for (copy, original) in after[1].path.subpaths[1].knots.iter().zip(&before[1].path.subpaths[0].knots) {
        assert!((copy.anchor.x - original.anchor.x - 8.0).abs() < 1e-6, "copy={copy:?}, original={original:?}");
        assert!((copy.anchor.y - original.anchor.y - 5.0).abs() < 1e-6, "copy={copy:?}, original={original:?}");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 0], [1, 1], [1, 2], [1, 3]]);
    for (command, expected) in [("edit.undo", before), ("edit.redo", after)] {
        app.run(command, json!({})).unwrap();
        assert_eq!(paths(&app), expected);
        assert_cleared(&app);
        assert!(app.ui.direct_selection.target.is_none());
    }
}

fn action(name: &str, steps: Vec<(String, Value)>) -> Action {
    Action::new(name, steps.into_iter().map(|(id, params)| craft::automation::actions::ActionStep::new(id, params)).collect())
}

#[test]
fn script_reopens_same_id_then_edits_before_selection_sync() {
    let mut app = app();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("paths.pcraft");
    app.run("file.saveACopy", json!({"path":file})).unwrap();
    app.run("file.closeAll", json!({})).unwrap();
    app.run("file.openAs", json!({"path":file})).unwrap();
    let id = app.session.active().unwrap().doc.id;
    let revision = app.session.active().unwrap().revision;
    select(&mut app, 1);
    let reply = app
        .run(
            "file.scripts.browse",
            json!({"steps":[
                ["file.closeAll",{}], ["file.openAs",{"path":file}],
                ["path.rename",{"pathIndex":1,"to":"Replacement"}]
            ]}),
        )
        .unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.id, id);
    assert!(st.revision > revision);
    assert_eq!(st.saved_paths_revision(), 0);
    assert_cleared(&app);
    assert!(crate::vector_ui::with_active_path(&app, "layer.newFillLayer.solidColor", json!({})).get("path").is_none());
}

#[test]
fn failed_record_again_clears_the_shifted_row_before_the_next_fill() {
    let mut app = app();
    select(&mut app, 1);
    let original = paths(&app);
    app.session.actions.list.push(action("Script", vec![("file.scripts.browse".into(), json!({}))])).unwrap();
    // The nested script deletes A. Changing the action library makes the enclosing
    // Record Again fail after the deletion, before the next frame can reconcile the panel.
    let error = app
        .run(
            "actions.recordAgain",
            json!({"action":"Script","step":0,"params":{"steps":[
                ["path.delete",{"pathIndex":0}],["actions.rename",{"action":"Script","name":"Changed"}]
            ]}}),
        )
        .unwrap_err();
    assert!(error.contains("action changed"), "{error}");
    assert_eq!(app.ui.status, error);
    assert!(app.ui.status_error);
    assert_eq!(paths(&app), original[1..]);
    assert_cleared(&app);
    app.run("layer.newFillLayer.solidColor", json!({"color":"#808080"})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().vector_mask.is_none());
}
