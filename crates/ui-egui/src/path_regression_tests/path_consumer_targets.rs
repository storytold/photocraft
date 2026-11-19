//! Saved-path consumers must preserve row identity and reject unusable geometry.
use crate::app::framework::BuildApp as _;
use crate::{
    PhotocraftApp, Services, canvas_tool_menu, dialogs,
    state::{PathSelection, Tool},
};
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    symmetry_cmds::{PaintingSymmetry, SymmetryAxis},
};
use serde_json::{Value, json};
use std::sync::Arc;

fn geometry(x: i32) -> Value {
    json!({"subpaths":[{"closed":true,"knots":[[x,30],[x+40,30],[x+40,70],[x,70]]}]})
}
fn app(duplicates: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.edit_prefs(|p| p.interface.language = "en".into());
    app.run("file.new", json!({"width":400,"height":260,"background":"white"})).unwrap();
    for (name, x) in [("A", 30), ("B", 150), ("C", 280), ("work", 90)] {
        app.run("path.set", json!({"name":name,"path":geometry(x)})).unwrap();
    }
    if duplicates {
        for path in &mut Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths {
            path.name = "Duplicate".into();
        }
    }
    app.sync_views();
    app
}
fn harness(app: PhotocraftApp, actions: bool) -> Harness<'static, PhotocraftApp> {
    harness_with_renderer(app, actions, false)
}
fn harness_with_renderer(app: PhotocraftApp, actions: bool, render: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_context(app.ctx().clone());
    let builder = if render { builder.wgpu() } else { builder };
    let mut h =
        builder.with_size(vec2(700., 750.)).with_max_steps(64).with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default())).build_app(
            move |ui, app: &mut PhotocraftApp| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if actions {
                        crate::panels::actions::panel(app, ui);
                    } else {
                        crate::symmetry_ui::menu(ui, app);
                    }
                });
            },
            app,
        );
    h.run_steps(5);
    h
}
fn capture(app: &mut PhotocraftApp, target: PathSelection) {
    app.session.actions.list.push(action("Capture", vec![])).unwrap();
    app.ui.actions.panel.selected = Some(craft::automation::ui::actions::ActionSelection::Action(0));
    app.ui.selected_path = Some(target);
}
#[test]
fn insert_path_rejects_ambiguous_legacy_missing_and_out_of_bounds_targets() {
    for target in [PathSelection::Name("Duplicate".into()), PathSelection::Name("Missing".into()), PathSelection::Saved { index: 99 }] {
        let mut app = app(true);
        capture(&mut app, target);
        insert_path(&mut app, false);
        assert!(app.session.actions.list.action(0).unwrap().steps.is_empty());
    }
}
#[test]
fn insert_path_keeps_indexed_unique_and_work_targets() {
    for target in [PathSelection::Saved { index: 1 }, PathSelection::Name("B".into()), PathSelection::Name("work".into())] {
        let mut app = app(matches!(target, PathSelection::Saved { .. }));
        let expected = target.path(&app.session.active().unwrap().doc).unwrap().clone();
        capture(&mut app, target);
        insert_path(&mut app, true);
        let steps = &app.session.actions.list.action(0).unwrap().steps;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].params["path"], photocraft_engine::vector_cmds::path_json(&expected));
    }
}
#[test]
fn insert_path_keeps_active_layer_vector_mask() {
    let mut app = app(false);
    app.run("layer.vectorMask.fromPath", json!({"name":"work"})).unwrap();
    let expected = photocraft_engine::actions_edit_cmds::selected_recording_path(&app.session, "layer").unwrap().clone();
    capture(&mut app, PathSelection::Name("layer".into()));
    insert_path(&mut app, true);
    assert_eq!(app.session.actions.list.action(0).unwrap().steps[0].params["path"], photocraft_engine::vector_cmds::path_json(&expected));
}
fn open_symmetry(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("Set painting symmetry options").click();
    h.run_steps(4);
}
fn choose_row(h: &mut Harness<'_, PhotocraftApp>, name: &str, index: usize) {
    let mut rows: Vec<_> = h.get_all_by_label(name).map(|n| n.rect()).collect();
    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let pos = rows[index].center();
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
}
fn checked(h: &Harness<'_, PhotocraftApp>, name: &str) -> Vec<bool> {
    let mut rows: Vec<_> = h.get_all_by_label(name).map(|n| (n.rect().top(), n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, v)| v).collect()
}
fn segments(app: &PhotocraftApp) -> Vec<[[f64; 2]; 2]> {
    let Some(PaintingSymmetry::Path(axis)) = &app.session.active().unwrap().symmetry else { panic!("no path symmetry") };
    axis.segments.clone()
}
#[test]
fn symmetry_second_duplicate_uses_its_geometry_and_only_its_checkmark() {
    let app = app(true);
    let expected = SymmetryAxis::from_path("Duplicate".into(), &app.session.active().unwrap().doc.paths[1].path).unwrap().segments;
    let mut h = harness(app, false);
    open_symmetry(&mut h);
    choose_row(&mut h, "Duplicate", 1);
    assert_eq!(segments(h.state()), expected);
    open_symmetry(&mut h);
    assert_eq!(checked(&h, "Duplicate"), [false, true, false]);
}
#[test]
fn symmetry_legacy_name_still_chooses_first_duplicate_only() {
    let mut app = app(true);
    app.run("paint.symmetryFromPath", json!({"name":"Duplicate"})).unwrap();
    let expected = SymmetryAxis::from_path("Duplicate".into(), &app.session.active().unwrap().doc.paths[0].path).unwrap().segments;
    assert_eq!(segments(&app), expected);
    let mut h = harness(app, false);
    open_symmetry(&mut h);
    assert_eq!(checked(&h, "Duplicate"), [true, false, false]);
}
#[test]
fn symmetry_reserved_saved_names_do_not_target_temporary_paths() {
    for name in ["work", "layer"] {
        let mut app = app(false);
        Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths[1].name = name.into();
        let expected = SymmetryAxis::from_path(name.into(), &app.session.active().unwrap().doc.paths[1].path).unwrap().segments;
        let mut h = harness(app, false);
        open_symmetry(&mut h);
        choose_row(&mut h, name, 0);
        assert_eq!(segments(h.state()), expected);
        open_symmetry(&mut h);
        assert_eq!(checked(&h, name), [true]);
        assert_eq!(checked(&h, "Work Path"), [false]);
    }
}
#[test]
fn symmetry_list_edits_and_history_drop_stale_checkmarks_but_keep_sampled_axis() {
    let mut app = app(true);
    app.run("paint.symmetryFromPath", json!({"pathIndex":1})).unwrap();
    let expected = segments(&app);
    let mut h = harness(app, false);
    for cmd in ["path.delete", "edit.undo", "edit.redo"] {
        h.state_mut().run(cmd, json!({"pathIndex":0})).unwrap();
        h.run_steps(4);
        assert_eq!(segments(h.state()), expected);
        open_symmetry(&mut h);
        assert!(checked(&h, "Duplicate").iter().all(|v| !*v));
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
    }
}
#[test]
fn symmetry_menu_closes_when_document_paths_change() {
    let mut h = harness(app(true), false);
    open_symmetry(&mut h);
    assert_eq!(checked(&h, "Duplicate").len(), 3);
    h.state_mut().run("path.delete", json!({"pathIndex":0})).unwrap();
    h.run_steps(4);
    assert!(h.query_all_by_label("Duplicate").next().is_none());
    assert!(h.state().session.active().unwrap().symmetry.is_none());
}
#[test]
fn empty_inline_shape_cannot_add_or_overwrite_a_preset() {
    let mut app = app(false);
    app.run("edit.defineCustomShape", json!({"name":"Logo","path":"work"})).unwrap();
    let before = app.session.edit_state.custom_shapes.clone();
    for name in ["Logo", "New"] {
        assert!(app.run("edit.defineCustomShape", json!({"name":name,"path":{"subpaths":[]}})).is_err());
        assert_eq!(app.session.edit_state.custom_shapes, before);
    }
}
#[test]
fn canvas_custom_shape_keeps_inline_second_duplicate_geometry() {
    let mut app = app(true);
    app.ui.tool = Tool::Pen;
    app.ui.selected_path = Some(PathSelection::Saved { index: 1 });
    let expected = app.session.active().unwrap().doc.paths[1].path.clone();
    let ctx = egui::Context::default();
    assert!(canvas_tool_menu::open_at(&mut app, Tool::Pen, craft::keymap::Mods::NONE, egui::pos2(10., 10.), [10., 10.], None));
    canvas_tool_menu::choose(&mut app, &ctx, "edit.defineCustomShape");
    let dialog = app.framework.dialogs.get_mut::<HostedParamDialog>().unwrap();
    assert_eq!(dialog.fields().number("pathIndex"), Some(1.));
    dialog.set_field("name", serde_json::from_value(json!("Second")).unwrap());
    let id = dialog.id();
    confirm(&mut app, id).unwrap();
    assert_eq!(app.session.edit_state.custom_shapes[0].path, expected);
}

#[test]
fn symmetry_work_layer_off_and_preset_remain_usable() {
    let mut app = app(false);
    app.run("layer.vectorMask.fromPath", json!({"name":"work"})).unwrap();
    let layer_name = crate::vector_ui::path_rows(&app.session.active().unwrap().doc, app.session.active().unwrap().active_layer)
        .into_iter()
        .find(|row| row.kind == crate::vector_ui::PathRow::Layer)
        .unwrap()
        .name;
    let mut h = harness(app, false);
    for (label, name) in [("Work Path", "work"), (layer_name.as_str(), "layer")] {
        open_symmetry(&mut h);
        choose_row(&mut h, label, 0);
        assert_eq!(h.state().session.active().unwrap().symmetry.as_ref().unwrap().path_source(), Some(name));
        open_symmetry(&mut h);
        assert_eq!(checked(&h, label), [true]);
        h.get_by_label("Symmetry Off").click();
        h.run_steps(4);
        assert!(h.state().session.active().unwrap().symmetry.is_none());
    }
    h.state_mut().run("paint.symmetryFromPath", json!({"pathIndex":1})).unwrap();
    h.state_mut().run("paint.setSymmetry", json!({"mode":"vertical"})).unwrap();
    h.run_steps(4);
    open_symmetry(&mut h);
    assert_eq!(checked(&h, "B"), [false]);
    assert!(h.state().session.active().unwrap().symmetry.as_ref().unwrap().preset().is_some());
}

#[test]
fn symmetry_invalid_indices_preserve_axis_and_transform_preserves_source() {
    let mut app = app(true);
    app.run("paint.symmetryFromPath", json!({"pathIndex":1})).unwrap();
    let before = app.session.active().unwrap().symmetry.clone().unwrap();
    for params in [json!({"pathIndex":99}), json!({"pathIndex":-1}), json!({"pathIndex":"1"}), json!({"pathIndex":1,"name":"Duplicate"})] {
        assert!(app.run("paint.symmetryFromPath", params).is_err());
        assert_eq!(app.session.active().unwrap().symmetry.as_ref(), Some(&before));
    }
    let source = before.saved_path_source();
    assert_eq!(source, Some((1, app.session.active().unwrap().saved_paths_revision())));
    let moved = before.transform(&photocraft_geom::Affine::translate(5., 7.)).unwrap();
    assert_eq!(moved.saved_path_source(), source);
    let (PaintingSymmetry::Path(original), PaintingSymmetry::Path(transformed)) = (before, moved) else { panic!("expected sampled axes") };
    assert_eq!(original.segments.len(), transformed.segments.len());
    for (a, b) in original.segments.iter().flatten().zip(transformed.segments.iter().flatten()) {
        assert_eq!([a[0] + 5., a[1] + 7.], *b);
    }
}

#[test]
#[ignore = "manual offscreen visual fixture; writes a synthetic screenshot"]
fn duplicate_symmetry_visual_fixture() {
    let _gpu = photocraft_testkit::gpu_lock();
    let mut h = harness_with_renderer(app(true), false, true);
    open_symmetry(&mut h);
    choose_row(&mut h, "Duplicate", 1);
    open_symmetry(&mut h);
    assert_eq!(checked(&h, "Duplicate"), [false, true, false]);
    let output = std::env::var_os("PHOTOCRAFT_PEN_VISUAL_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| "log/path-consumer-visual".into());
    std::fs::create_dir_all(&output).unwrap();
    h.render().unwrap().save(output.join("duplicate-symmetry.png")).unwrap();
}

#[test]
fn symmetry_menu_rejects_reopened_document_with_reused_identity_and_revision() {
    let mut app = app(true);
    let doc = app.session.active().unwrap().doc.as_ref().clone();
    app.session.close(0);
    app.session.add_document(doc.clone(), None);
    app.sync_views();
    let mut h = harness(app, false);
    open_symmetry(&mut h);
    h.state_mut().session.close(0);
    h.state_mut().session.add_document(doc, None);
    h.state_mut().sync_views();
    h.run_steps(4);
    assert!(h.query_all_by_label("Duplicate").next().is_none());
}

fn action(name: &str, steps: Vec<(String, Value)>) -> Action {
    Action::new(name, steps.into_iter().map(|(id, params)| craft::automation::actions::ActionStep::new(id, params)).collect())
}

#[test]
fn indexed_path_playback_dialog_preserves_target_and_continues_or_cancels_once() {
    for outcome in ["confirm", "cancel", "stale"] {
        let mut app = app(true);
        let before = app.session.active().unwrap().doc.clone();
        let mut recorded = action(
            "Path dialog",
            vec![("path.transform".into(), json!({"pathIndex":1,"translateX":5.0})), ("layer.renameLayer".into(), json!({"name":"After path"}))],
        );
        recorded.dialogs.insert(0);
        app.session.actions.list.push(recorded).unwrap();
        app.ui.selected_path = Some(PathSelection::Saved { index: 0 });
        let reply = app.run("actions.play", json!({"action":"Path dialog"})).unwrap();
        assert!(reply.get("paused").is_some(), "{reply}");
        crate::dialogs::testing::show(&mut app);
        let id = app.framework.dialogs.get::<HostedParamDialog>().unwrap().id();
        let params = crate::control::dialogs::hosted_command(&app, id).unwrap().1;
        assert_eq!(params["pathIndex"], 1);
        assert!(params.get("name").is_none());
        let ctx = app.ctx().clone();
        if outcome == "stale" {
            app.run("path.delete", json!({"pathIndex":0})).unwrap();
            let changed = app.session.active().unwrap().doc.clone();
            assert!(crate::actions::authorize_panel_preview(&app, id, "path.transform", &params).is_err());
            assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), changed.as_ref());
            crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
        } else if outcome == "cancel" {
            crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        } else {
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
            let doc = &app.session.active().unwrap().doc;
            assert_eq!(doc.paths[0], before.paths[0]);
            assert_eq!(doc.paths[1].path.subpaths[0].knots[0].anchor.x, before.paths[1].path.subpaths[0].knots[0].anchor.x + 5.0);
            assert!(doc.walk().iter().any(|(_, _, layer)| layer.name == "After path"));
            assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err());
        }
        assert!(crate::actions::token_of(&app, id).is_none());
        assert_eq!(app.session.actions.playing, 0);
    }
}

#[test]
fn consecutive_indexed_playback_dialogs_keep_the_new_openings_source() {
    for stale in [false, true] {
        let mut app = app(true);
        let before = app.session.active().unwrap().doc.paths.clone();
        let mut recorded = action(
            "Two path dialogs",
            vec![("path.transform".into(), json!({"pathIndex":1,"translateX":2.0})), ("path.transform".into(), json!({"pathIndex":1,"translateX":3.0}))],
        );
        recorded.dialogs.extend([0, 1]);
        app.session.actions.list.push(recorded).unwrap();
        app.run("actions.play", json!({"action":"Two path dialogs"})).unwrap();
        crate::dialogs::testing::show(&mut app);
        let id = app.framework.dialogs.get::<HostedParamDialog>().unwrap().id();
        let ctx = app.ctx().clone();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        crate::dialogs::testing::show(&mut app);
        assert!(crate::actions::token_of(&app, id).is_some(), "the next step is still paused");
        if stale {
            app.run("path.delete", json!({"pathIndex":0})).unwrap();
            let changed = app.session.active().unwrap().doc.clone();
            assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), changed.as_ref());
            crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
        } else {
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
            let paths = &app.session.active().unwrap().doc.paths;
            assert_eq!(paths[0], before[0]);
            assert_eq!(paths[1].path.subpaths[0].knots[0].anchor.x, before[1].path.subpaths[0].knots[0].anchor.x + 5.0);
        }
        assert_eq!(app.session.actions.playing, 0);
    }
}

fn confirm(app: &mut PhotocraftApp, id: egui::Id) -> Result<(), String> {
    let ctx = app.ctx().clone();
    dialogs::confirm(app, &ctx, id)
}

fn insert_path(app: &mut PhotocraftApp, enabled: bool) {
    use craft::automation::ui::actions::ActionsPanelCommands;
    use craft::ui::runtime::commands::CommandSet as _;
    let (command, state) = crate::panels::actions::menu::states(app)
        .into_iter()
        .find(|(id, _)| ActionsPanelCommands::from_ref(id) == Some(ActionsPanelCommands::InsertPath))
        .unwrap();
    assert_eq!(state.is_enabled(), enabled);
    let ctx = app.ctx().clone();
    assert!(crate::panels::actions::menu::run_fired(app, &ctx, &command));
}
