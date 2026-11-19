//! The Paths panel must keep two rows with identical names independent.
use crate::PhotocraftApp;
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::{Event, Key, Modifiers, PointerButton, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;
use std::sync::Arc;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":400,"height":260,"background":"white"})).unwrap();
        for (name, knots) in [("Left", json!([[30, 30], [130, 30], [80, 130]])), ("Right", json!([[220, 30], [320, 30], [320, 130], [220, 130]]))] {
            app.run("path.set", json!({"name":name,"path":{"subpaths":[{"knots":knots}]}})).unwrap();
        }
        for p in &mut Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths {
            p.name = "??".into();
        }
        crate::panels::panel_docking::show_panel(&mut app, "paths", Some(true)).unwrap();
        app.ui.tool = crate::state::Tool::PathSelection;
        app
    });
    h.run_steps(8);
    h
}

fn row(h: &Harness<'_, PhotocraftApp>, i: usize) -> egui::Rect {
    let mut rows: Vec<_> = h.get_all_by_role_and_label(egui::accesskit::Role::ListItem, "??").map(|n| n.rect()).collect();
    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert_eq!(rows.len(), 2);
    rows[i]
}

fn click(h: &mut Harness<'_, PhotocraftApp>, i: usize) {
    let p = row(h, i).center();
    h.hover_at(p);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.run_steps(4);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

#[test]
fn duplicate_paths_ui_select_and_load_each_geometry() {
    for index in [0, 1] {
        let mut h = harness();
        click(&mut h, index);
        assert_eq!(crate::vector_ui::selected_saved_path_index(h.state()).unwrap().0, index);
        let selected: Vec<_> = h.get_all_by_role_and_label(egui::accesskit::Role::ListItem, "??").map(|n| n.accesskit_node().is_selected()).collect();
        assert_eq!(selected.iter().filter(|s| **s == Some(true)).count(), 1);
        h.get_by_label("Load path as a selection").click();
        h.run_steps(4);
        let bounds = h.state_mut().run("document.inspect", json!({})).unwrap()["selectionBounds"].clone();
        assert_eq!(bounds, if index == 0 { json!([30, 30, 100, 100]) } else { json!([220, 30, 100, 100]) });
    }
}

#[test]
fn duplicate_paths_ui_rename_second_has_one_editor_and_undo_redo() {
    let mut h = harness();
    let p = row(&h, 1).center();
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "??").count(), 1, "one rename field, even with identical names");
    key(&mut h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("Right renamed".into()));
    key(&mut h, Key::Enter, Modifiers::NONE);
    let names = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.paths.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&h), ["??", "Right renamed"]);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(names(&h), ["??", "??"]);
    assert!(h.state().ui.selected_path.is_none(), "undo clears positional UI selection");
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(names(&h), ["??", "Right renamed"]);
}

#[test]
fn duplicate_paths_ui_delete_one_and_undo_redo_preserve_order() {
    for index in [0, 1] {
        let mut h = harness();
        let before = h.state().session.active().unwrap().doc.paths.clone();
        click(&mut h, index);
        h.get_by_label("Delete current path").click();
        h.run_steps(4);
        assert_eq!(h.state().session.active().unwrap().doc.paths, vec![before[1 - index].clone()]);
        assert!(h.state().ui.selected_path.is_none(), "deletion invalidates positional selection");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.paths, before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.paths, vec![before[1 - index].clone()]);
    }
}

#[test]
fn duplicate_paths_ui_direct_selection_commands_keep_the_second_target() {
    let mut h = harness();
    click(&mut h, 1);
    let before = h.state().session.active().unwrap().doc.paths.clone();
    let mut params = crate::direct_select::PathRef::Saved(1).params(h.state());
    params["anchors"] = json!([[0, 0]]);
    params["move"] = json!([5, 0]);
    h.state_mut().run("path.moveAnchors", params).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.paths[0], before[0]);
    assert_eq!(h.state().session.active().unwrap().doc.paths[1].path.subpaths[0].knots[0].anchor.x, 225.0);
}

#[test]
fn duplicate_paths_ui_document_switch_clears_the_target() {
    let mut h = harness();
    click(&mut h, 1);
    h.state_mut().run("file.new", json!({"width":20,"height":20})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.selected_path.is_none());
}

#[test]
fn duplicate_paths_ui_context_delete_uses_clicked_row_not_selected_row() {
    let mut h = harness();
    click(&mut h, 1);
    let expected = h.state().session.active().unwrap().doc.paths[1].clone();
    let p = row(&h, 0).center();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
    h.get_by_label("Delete Path").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().doc.paths, vec![expected]);
}

#[test]
fn duplicate_paths_ui_keyboard_selection_and_delete_target_the_second_row() {
    let mut h = harness();
    click(&mut h, 1);
    let command = if cfg!(target_os = "macos") {
        Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE }
    } else {
        Modifiers { command: true, ctrl: true, ..Modifiers::NONE }
    };
    key(&mut h, Key::Enter, command);
    assert_eq!(
        h.state_mut().run("document.inspect", json!({})).unwrap()["selectionBounds"],
        json!([220, 30, 100, 100]),
        "selected={:?}; status={}; journal={:?}",
        h.state().ui.selected_path,
        h.state().ui.status,
        h.state().session.journal.last()
    );
    // Exercise deletion from a newly selected row. A second click on an already selected
    // framework row starts inline rename, where Delete correctly edits the name instead.
    for deletion in [Key::Delete, Key::Backspace] {
        let mut h = harness();
        h.state_mut().ui.tool = crate::state::Tool::DirectSelection;
        click(&mut h, 1);
        let first = h.state().session.active().unwrap().doc.paths[0].clone();
        key(&mut h, deletion, Modifiers::NONE);
        assert_eq!(h.state().session.active().unwrap().doc.paths, vec![first]);
    }
}

#[test]
fn duplicate_paths_ui_stale_rename_is_cancelled_when_indices_shift() {
    let mut h = harness();
    let p = row(&h, 1).center();
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "??").count(), 1);
    h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "??").count(), 0);
    assert_eq!(h.state().session.active().unwrap().doc.paths[0].name, "??");
    assert!(h.state().ui.selected_path.is_none());
}

#[test]
fn duplicate_paths_ui_saved_reserved_names_are_not_temporary_paths() {
    for name in ["work", "layer", "Work Path", ""] {
        let mut h = harness();
        let second_row = row(&h, 1);
        for p in &mut Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc).paths {
            p.name = name.into();
        }
        h.run_steps(4);
        let p = second_row.center();
        h.hover_at(p);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        h.run_steps(4);
        assert_eq!(crate::vector_ui::selected_saved_path_index(h.state()).unwrap().0, 1, "{name}");
        h.get_by_label("Delete current path").click();
        h.run_steps(4);
        assert_eq!(h.state().session.active().unwrap().doc.paths.len(), 1);
        assert_eq!(h.state().session.active().unwrap().doc.paths[0].path.subpaths[0].knots.len(), 3);
    }
}

#[test]
fn duplicate_paths_ui_selection_state_roundtrips_and_legacy_names_require_uniqueness() {
    use crate::state::PathSelection;
    let mut h = harness();
    let selected = PathSelection::Saved { index: 1 };
    let value = serde_json::to_value(&selected).unwrap();
    assert_eq!(value, json!({"pathIndex":1}));
    assert_eq!(serde_json::from_value::<PathSelection>(value).unwrap(), selected);
    let legacy: PathSelection = serde_json::from_value(json!("??")).unwrap();
    assert_eq!(legacy.saved_index(&h.state().session.active().unwrap().doc), None);
    h.state_mut().run("path.rename", json!({"pathIndex":1,"to":"Right"})).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(legacy.saved_index(doc), Some(0));
    assert_eq!(PathSelection::Saved { index: 2 }.saved_index(doc), None);
    for value in [json!({"pathIndex":-1}), json!({"pathIndex":"1"}), json!({"pathIndex":null})] {
        assert!(serde_json::from_value::<PathSelection>(value).is_err());
    }
}

#[test]
fn duplicate_paths_ui_selection_dialog_captures_index_and_rejects_a_shifted_source() {
    for stale in [false, true] {
        let mut h = harness();
        click(&mut h, 1);
        crate::canvas_tool_menu::pen_dialogs::make_selection(h.state_mut()).unwrap();
        let dialog = h.state().framework.dialogs.get::<HostedParamDialog>().unwrap();
        assert_eq!(dialog.fields().number("pathIndex"), Some(1.));
        assert!(dialog.fields().get("name").is_none());
        let id = dialog.id();
        let ctx = h.ctx.clone();
        if stale {
            h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
            assert!(crate::dialogs::confirm(h.state_mut(), &ctx, id).is_err());
            assert!(h.state().session.active().unwrap().doc.selection.is_none());
        } else {
            crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
            assert_eq!(h.state_mut().run("document.inspect", json!({})).unwrap()["selectionBounds"], json!([220, 30, 100, 100]));
        }
    }
}
