//! Menus keep the path they were opened for, or refuse an invalidated target.
use crate::app::framework::BuildApp as _;
use crate::{
    PhotocraftApp, Services, canvas_tool_menu, dialogs,
    state::{PathSelection, Tool},
    vector_ui,
};
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::{Value, json};
use std::sync::Arc;

fn geometry(x: i32) -> Value {
    json!({"subpaths":[{"closed":true,"knots":[[x,30],[x+40,30],[x+40,70],[x,70]]}]})
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.edit_prefs(|p| p.interface.language = "en".into());
    app.run("file.new", json!({"width":400,"height":260,"background":"white"})).unwrap();
    for (name, x) in [("A", 30), ("B", 150), ("C", 280), ("work", 90)] {
        app.run("path.set", json!({"name":name,"path":geometry(x)})).unwrap();
    }
    app.ui.tool = Tool::PathSelection;
    app.sync_views();
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(700.0, 750.0))
        .with_max_steps(64)
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_app(
            |ui, app: &mut PhotocraftApp| {
                egui::CentralPanel::default().show(ui, |ui| vector_ui::paths_panel(app, ui));
            },
            app,
        );
    h.run_steps(5);
    h
}

fn open_row(h: &mut Harness<'_, PhotocraftApp>, name: &str) {
    let pos = h.get_by_role_and_label(egui::accesskit::Role::ListItem, name).rect().center();
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
    assert!(h.query_by_label("Delete Path").is_some());
}

fn assert_menu_closed(h: &mut Harness<'_, PhotocraftApp>) {
    let before = h.state().session.active().unwrap().doc.clone();
    h.run_steps(4);
    assert!(h.query_by_label("Delete Path").is_none(), "the old row menu must not retarget the new occupant of its slot");
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert_eq!(h.state().session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
fn row_menu_closes_after_an_earlier_path_is_deleted() {
    let mut h = harness(app());
    open_row(&mut h, "B");
    h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
    assert_menu_closed(&mut h);
    assert_eq!(h.state().session.active().unwrap().doc.paths.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B", "C"]);
    // A fresh menu remains usable and still targets the clicked row, not the selected one.
    h.state_mut().ui.selected_path = Some(PathSelection::Saved { index: 0 });
    open_row(&mut h, "C");
    h.get_by_label("Delete Path").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().doc.paths.len(), 1);
    assert_eq!(h.state().session.active().unwrap().doc.paths[0].name, "B");
}

#[test]
fn row_menu_closes_after_grouped_same_count_replacement() {
    let mut app = app();
    app.session.edit_prefs(|p| p.actions.create_single_history_state = true);
    app.session
        .actions
        .list
        .push(action("Replace", vec![("path.delete".into(), json!({"pathIndex":0})), ("path.set".into(), json!({"name":"D","path":geometry(240)}))]))
        .unwrap();
    let mut h = harness(app);
    open_row(&mut h, "B");
    let result = h.state_mut().run("actions.play", json!({"action":"Replace"})).unwrap();
    assert!(result.get("failed").is_none());
    assert_eq!(h.state().session.active().unwrap().doc.paths.len(), 3);
    assert_menu_closed(&mut h);
}

#[test]
fn row_menu_closes_on_history_changes_and_between_frame_roundtrips() {
    let mut h = harness(app());
    let original = h.state().session.active().unwrap().doc.paths.clone();
    open_row(&mut h, "B");
    h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
    // Both commands happen between UI frames and restore the original path list.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.paths, original);
    assert_menu_closed(&mut h);
    for command in ["edit.redo", "history.undo", "history.redo"] {
        open_row(&mut h, "B");
        h.state_mut().run(command, json!({})).unwrap();
        assert_menu_closed(&mut h);
    }
}

#[test]
fn row_menu_closes_on_same_count_revert_and_its_undo_redo() {
    let mut app = app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("menu.pcraft");
    app.run("file.saveACopy", json!({"path":file})).unwrap();
    app.run("file.openAs", json!({"path":file})).unwrap();
    app.run("path.set", json!({"pathIndex":0,"path":geometry(10)})).unwrap();
    let mut h = harness(app);
    for command in ["file.revert", "edit.undo", "edit.redo"] {
        open_row(&mut h, "B");
        h.state_mut().run(command, json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths.len(), 3);
        assert_menu_closed(&mut h);
    }
}

#[test]
fn row_menu_closes_on_document_change() {
    let mut h = harness(app());
    open_row(&mut h, "B");
    h.state_mut().run("file.new", json!({"width":400,"height":260})).unwrap();
    for name in ["A", "B", "C"] {
        h.state_mut().run("path.set", json!({"name":name,"path":geometry(10)})).unwrap();
    }
    assert_menu_closed(&mut h);
}

#[test]
fn row_menu_survives_selection_change_and_deletes_only_its_row() {
    let mut h = harness(app());
    open_row(&mut h, "B");
    h.state_mut().ui.selected_path = Some(PathSelection::Saved { index: 2 });
    h.run_steps(4);
    h.get_by_label("Delete Path").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().doc.paths.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A", "C"]);
}

#[test]
fn stale_row_menu_does_not_close_an_unrelated_popup() {
    let mut h = harness(app());
    open_row(&mut h, "B");
    let other = egui::Id::unique("unrelated-test-popup");
    egui::Popup::open_id(&h.ctx, other);
    h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
    h.run_steps(1);
    assert!(egui::Popup::is_id_open(&h.ctx, other));
}

fn pen_draft(app: &mut PhotocraftApp) {
    app.ui.tool = Tool::Pen;
    for (x, y) in [(180.0, 100.0), (210.0, 100.0), (210.0, 130.0)] {
        crate::canvas::testing::click(app, [x, y], Modifiers::NONE);
    }
    assert!(app.ui.pen.as_ref().unwrap().draft.knots.len() >= 2);
}

fn choose_pen(app: &mut PhotocraftApp, command: &str) {
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10.0, 10.0], None));
    canvas_tool_menu::choose(app, &ctx, command);
}

#[test]
fn pen_menu_uses_committed_second_duplicate_for_selection_and_preserves_work_path() {
    let mut app = app();
    for path in &mut Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths {
        path.name = "??".into();
    }
    app.sync_views();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let before = app.session.active().unwrap().doc.clone();
    pen_draft(&mut app);
    choose_pen(&mut app, "path.toSelection");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.work_path, before.work_path);
    assert_eq!(st.doc.paths[0], before.paths[0]);
    assert_eq!(st.doc.paths[1].path.subpaths.len(), before.paths[1].path.subpaths.len() + 1);
    let dialog = app.framework.dialogs.get::<HostedParamDialog>().unwrap();
    assert_eq!(dialog.fields().number("pathIndex"), Some(1.));
    assert!(dialog.fields().get("name").is_none());
    let id = dialog.id();
    confirm(&mut app, id).unwrap();
    let bounds = app.run("document.inspect", json!({})).unwrap()["selectionBounds"].clone();
    assert_eq!(bounds, json!([150, 30, 60, 100]));
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.run("document.inspect", json!({})).unwrap()["selectionBounds"], bounds);
}

#[test]
fn pen_menu_direct_delete_targets_committed_saved_row_with_undo_redo() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let before = app.session.active().unwrap().doc.clone();
    pen_draft(&mut app);
    choose_pen(&mut app, "path.delete");
    assert_eq!(app.session.active().unwrap().doc.paths, vec![before.paths[0].clone(), before.paths[2].clone()]);
    assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths.len(), 3);
    assert_eq!(app.session.active().unwrap().doc.paths[1].path.subpaths.len(), 2);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths.len(), 2);
    assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
}

#[test]
fn pen_menu_keeps_work_and_shape_layer_draft_targets() {
    for shape in [false, true] {
        let mut app = app();
        app.ui.tool_options.vector_mode = if shape { craft::draw::ui::VectorMode::Shape } else { craft::draw::ui::VectorMode::Path };
        app.ui.selected_path = Some("work".into());
        pen_draft(&mut app);
        choose_pen(&mut app, "path.toSelection");
        let dialog = app.framework.dialogs.get::<HostedParamDialog>().unwrap();
        assert_eq!(dialog.fields().text("name"), Some(if shape { "layer" } else { "work" }));
        assert!(dialog.fields().get("pathIndex").is_none());
        let id = dialog.id();
        confirm(&mut app, id).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }
}

#[test]
fn pen_menu_without_a_draft_keeps_its_captured_saved_target() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10.0, 10.0], None));
    app.ui.selected_path = Some(PathSelection::Saved { index: 0 });
    canvas_tool_menu::choose(&mut app, &ctx, "path.toSelection");
    assert_eq!(app.framework.dialogs.get::<HostedParamDialog>().unwrap().fields().number("pathIndex"), Some(1.));
}

#[test]
fn pen_menu_uses_draft_owner_after_options_change() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    pen_draft(&mut app);
    // Options may change while a draft is pending; its captured owner still receives it.
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
    choose_pen(&mut app, "path.toSelection");
    assert_eq!(app.session.active().unwrap().doc.paths[1].path.subpaths.len(), 2);
    assert_eq!(app.framework.dialogs.get::<HostedParamDialog>().unwrap().fields().number("pathIndex"), Some(1.));
}

#[test]
fn pen_menu_failed_commit_does_not_open_a_dialog_or_change_a_path() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    pen_draft(&mut app);
    app.session.execute("path.delete", json!({"pathIndex":0})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    choose_pen(&mut app, "path.toSelection");
    assert!(app.framework.dialogs.is_empty());
    assert!(app.ui.pen.is_some());
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
fn saved_canvas_menu_cannot_delete_a_reused_index_before_another_frame() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10.0, 10.0], None));
    app.run("path.delete", json!({"pathIndex":0})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    canvas_tool_menu::choose(&mut app, &ctx, "path.delete");
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

fn reopen_with_same_identity(app: &mut PhotocraftApp, swap: bool) {
    let mut doc = app.session.active().unwrap().doc.as_ref().clone();
    if swap {
        doc.paths.swap(0, 1);
    }
    app.session.close(app.session.active_index().unwrap());
    app.session.add_document(doc, None);
    app.sync_views();
}

#[test]
fn row_menu_rejects_reopened_document_with_reused_identity_and_revision() {
    let mut app = app();
    reopen_with_same_identity(&mut app, false);
    let mut h = harness(app);
    open_row(&mut h, "B");
    let stamp = (h.state().session.active().unwrap().doc.id, h.state().session.active().unwrap().revision);
    reopen_with_same_identity(h.state_mut(), true);
    assert_eq!((h.state().session.active().unwrap().doc.id, h.state().session.active().unwrap().revision), stamp);
    assert_menu_closed(&mut h);
}

#[test]
fn canvas_menu_rejects_reopened_document_before_another_frame() {
    let mut app = app();
    reopen_with_same_identity(&mut app, false);
    app.ui.tool = Tool::Pen;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    reopen_with_same_identity(&mut app, true);
    let before = app.session.active().unwrap().doc.clone();
    canvas_tool_menu::choose(&mut app, &ctx, "path.delete");
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
fn saved_path_dialog_rejects_actual_file_close_and_reopen() {
    let mut app = app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("paths.pcraft");
    app.run("file.saveACopy", json!({"path":file})).unwrap();
    app.run("file.closeAll", json!({})).unwrap();
    app.run("file.openAs", json!({"path":file})).unwrap();
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    choose_pen(&mut app, "path.transform");
    let dialog = app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap();
    dialog.set_field("translateX", serde_json::from_value(json!(20.)).unwrap());
    let id = dialog.id();
    let stamp = (app.session.active().unwrap().doc.id, app.session.active().unwrap().revision);
    let mut replacement = app.session.active().unwrap().doc.as_ref().clone();
    replacement.paths.swap(0, 1);
    let mut writer = Session::new();
    writer.add_document(replacement, None);
    writer.execute("file.saveACopy", json!({"path":file})).unwrap();
    app.run("file.closeAll", json!({})).unwrap();
    app.run("file.openAs", json!({"path":file})).unwrap();
    assert_eq!(stamp, (app.session.active().unwrap().doc.id, app.session.active().unwrap().revision));
    let before = app.session.active().unwrap().doc.clone();
    assert!(confirm(&mut app, id).is_err());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
fn every_saved_path_parameter_dialog_rejects_a_reopened_source() {
    for command in ["path.transform", "path.toSelection", "path.fill", "path.stroke", "path.clippingPath.set", "edit.defineCustomShape"] {
        let mut app = app();
        reopen_with_same_identity(&mut app, false);
        app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
        choose_pen(&mut app, command);
        let id = app.framework.dialogs.get::<HostedParamDialog>().expect(command).id();
        reopen_with_same_identity(&mut app, true);
        let before = app.session.active().unwrap().doc.clone();
        let presets = app.session.edit_state.custom_shapes.clone();
        assert!(confirm(&mut app, id).is_err(), "{command}");
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref(), "{command}");
        assert_eq!(app.session.edit_state.custom_shapes, presets, "{command}");
    }
}

#[test]
fn restored_path_dialog_requires_a_fresh_source_and_live_dialog_ignores_removed_marker() {
    for restored in [false, true] {
        let mut app = app();
        reopen_with_same_identity(&mut app, false);
        app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
        choose_pen(&mut app, "path.transform");
        let dialog = app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap();
        dialog.set_field("translateX", serde_json::from_value(json!(20.)).unwrap());
        let id = dialog.id();
        if restored {
            // Restoring form fields cannot recreate the runtime owner.
            app.framework.path_dialog_targets.clear();
        } else {
            dialog.clear_field("__pathSource");
            reopen_with_same_identity(&mut app, true);
        }
        let before = app.session.active().unwrap().doc.clone();
        assert!(confirm(&mut app, id).is_err());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    }
}

#[test]
fn saved_draft_menu_enablement_follows_owner_after_mode_and_mask_option_changes() {
    for command in ["path.delete", "path.fill", "path.stroke", "layer.vectorMask.fromPath"] {
        let mut app = app();
        app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
        app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
        let before = app.session.active().unwrap().doc.clone();
        pen_draft(&mut app);
        app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
        app.ui.vector_mask_target = true;
        let ctx = egui::Context::default();
        assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
        assert!(canvas_tool_menu::available(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), command), "{command}");
        canvas_tool_menu::choose(&mut app, &ctx, command);
        assert!(app.ui.pen.is_none(), "{command}: the captured saved draft committed");
        assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
        if matches!(command, "path.fill" | "path.stroke") {
            assert_eq!(app.framework.dialogs.get::<HostedParamDialog>().unwrap().fields().number("pathIndex"), Some(1.));
        } else if command == "path.delete" {
            assert_eq!(app.session.active().unwrap().doc.paths, vec![before.paths[0].clone(), before.paths[2].clone()]);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.paths[1].path.subpaths.len(), 2);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.paths.len(), 2);
        } else {
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().vector_mask.as_ref().unwrap().path, st.doc.paths[1].path);
        }
    }
}

#[test]
fn shape_draft_keeps_layer_restrictions_after_switching_to_path_mode() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
    app.ui.selected_path = Some("work".into());
    pen_draft(&mut app);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    for command in ["path.delete", "path.fill", "path.stroke", "layer.vectorMask.fromPath"] {
        assert!(!canvas_tool_menu::available(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), command), "{command}");
    }
    canvas_tool_menu::choose(&mut app, &ctx, "path.toSelection");
    let d = app.framework.dialogs.get::<HostedParamDialog>().unwrap();
    assert_eq!(d.fields().text("name"), Some("layer"));
    let id = d.id();
    confirm(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn vector_mask_draft_keeps_layer_restrictions_after_target_option_changes() {
    let mut app = app();
    app.run("layer.vectorMask.fromPath", json!({"name":"work"})).unwrap();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.selected_path = None;
    app.ui.vector_mask_target = true;
    pen_draft(&mut app);
    app.ui.vector_mask_target = false;
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    for command in ["path.delete", "path.fill", "path.stroke", "layer.vectorMask.fromPath"] {
        assert!(!canvas_tool_menu::available(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), command), "{command}");
    }
    canvas_tool_menu::choose(&mut app, &ctx, "path.toSelection");
    let d = app.framework.dialogs.get::<HostedParamDialog>().unwrap();
    assert_eq!(d.fields().text("name"), Some("layer"));
    let id = d.id();
    confirm(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn fresh_saved_path_dialog_transforms_only_its_target_with_undo_redo() {
    let mut app = app();
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let before = app.session.active().unwrap().doc.clone();
    choose_pen(&mut app, "path.transform");
    let d = app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap();
    d.set_field("translateX", serde_json::from_value(json!(20.)).unwrap());
    let id = d.id();
    confirm(&mut app, id).unwrap();
    let after = app.session.active().unwrap().doc.clone();
    assert_eq!(after.paths[0], before.paths[0]);
    assert_eq!(after.paths[2], before.paths[2]);
    assert_eq!(after.work_path, before.work_path);
    assert_eq!(
        photocraft_engine::vector_cmds::path_json(&after.paths[1].path),
        photocraft_engine::vector_cmds::path_json(&photocraft_engine::vector_cmds::parse_path(&geometry(170)).unwrap())
    );
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths, before.paths);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths, after.paths);
}

fn action(name: &str, steps: Vec<(String, Value)>) -> Action {
    Action::new(name, steps.into_iter().map(|(id, params)| craft::automation::actions::ActionStep::new(id, params)).collect())
}

fn confirm(app: &mut PhotocraftApp, id: egui::Id) -> Result<(), String> {
    let ctx = app.ctx().clone();
    dialogs::confirm(app, &ctx, id)
}

#[test]
fn hosted_path_dialog_fields_cannot_retarget_the_captured_saved_row() {
    for command in ["path.transform", "path.toSelection", "path.fill", "path.stroke", "edit.defineCustomShape"] {
        let mut app = app();
        app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
        choose_pen(&mut app, command);
        let dialog = app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap();
        dialog.set_field("pathIndex", 0_i64.into());
        if command != "edit.defineCustomShape" {
            dialog.set_field("name", "work".into());
        }
        let fields = dialog.fields().clone();
        let params = canvas_tool_menu::pen_dialogs::command_params(&app, command, &fields).unwrap();
        if command == "edit.defineCustomShape" {
            assert_eq!(params["path"], photocraft_engine::vector_cmds::path_json(&app.session.active().unwrap().doc.paths[1].path));
        } else {
            assert_eq!(params["pathIndex"], json!(1));
            assert!(params.get("name").is_none());
        }
        app.run("path.delete", json!({"pathIndex":0})).unwrap();
        assert!(
            canvas_tool_menu::pen_dialogs::command_params(&app, command, &fields).is_err(),
            "{command}: preview and confirmation reject the same stale source"
        );
    }
}

#[test]
fn hosted_path_dialog_cancel_and_reopen_gets_a_fresh_runtime_target() {
    let mut app = app();
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    choose_pen(&mut app, "path.transform");
    let id = app.framework.dialogs.get::<HostedParamDialog>().unwrap().id();
    let ctx = app.ctx().clone();
    dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(app.framework.path_dialog_targets.is_empty());
    app.run("path.delete", json!({"pathIndex":0})).unwrap();
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let before = app.session.active().unwrap().doc.clone();
    let id = dialogs::open_command_dialog(&mut app, "path.transform", "Transform Path").unwrap();
    app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap().set_field("translateX", 5.0.into());
    confirm(&mut app, id).unwrap();
    let after = &app.session.active().unwrap().doc;
    assert_eq!(after.paths[0], before.paths[0]);
    assert_ne!(after.paths[1], before.paths[1]);
    assert!(app.framework.path_dialog_targets.is_empty());
}

#[test]
fn restored_saved_canvas_menu_and_cancelled_draft_menu_cannot_retarget_work() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let ctx = app.ctx().clone();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    app.ui.canvas_tool_menu = Some(serde_json::from_value(serde_json::to_value(menu).unwrap()).unwrap());
    let before = app.session.active().unwrap().doc.clone();
    canvas_tool_menu::choose(&mut app, &ctx, "path.delete");
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());

    app.ui.selected_path = Some("work".into());
    pen_draft(&mut app);
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    app.run("path.delete", json!({"pathIndex":0})).unwrap();
    assert!(app.ui.pen.is_none());
    let before = app.session.active().unwrap().doc.clone();
    canvas_tool_menu::choose(&mut app, &ctx, "path.delete");
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
fn framework_panel_keeps_a_weak_source_between_document_close_and_reopen() {
    let mut h = harness(app());
    let source = h.state().framework.paths_source.clone().unwrap();
    let doc = h.state().session.active().unwrap().doc.as_ref().clone();
    h.state_mut().run("file.closeAll", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().framework.paths_source.as_ref(), Some(&source));
    h.state_mut().session.add_document(doc, None);
    h.state_mut().sync_views();
    h.run_steps(3);
    assert_ne!(h.state().framework.paths_source.as_ref(), Some(&source));
}

#[test]
fn returning_to_an_older_snapshot_does_not_restore_its_rename_editor() {
    let mut h = harness(app());
    let st = h.state().session.active().unwrap();
    let (original, revision) = (st.doc.clone(), st.revision);
    open_row(&mut h, "B");
    h.get_by_label("Rename Path…").click();
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "B").count(), 1);
    h.state_mut().run("path.rename", json!({"pathIndex":1,"to":"Changed"})).unwrap();
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "B").count(), 0);
    // Deliberately reuse even the exact Arc and revision, stronger than a reopened document
    // reusing an allocator address. The old editor must stay dismissed.
    let st = h.state_mut().session.active_mut().unwrap();
    st.doc = original;
    st.revision = revision;
    h.state_mut().sync_views();
    h.run_steps(4);
    assert_eq!(h.query_all_by_role_and_label(egui::accesskit::Role::TextInput, "B").count(), 0);
    assert_eq!(h.state().session.active().unwrap().doc.paths[1].name, "B");
}
