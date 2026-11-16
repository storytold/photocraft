//! Saved-path consumers must preserve row identity and reject unusable geometry.
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
use photocraft_ui_egui::{
    PhotocraftApp, Services, canvas_tool_menu, dialogs,
    state::{PathSelection, Tool},
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
    let builder = Harness::builder();
    let builder = if render { builder.wgpu() } else { builder };
    let mut h =
        builder.with_size(vec2(700., 750.)).with_max_steps(64).with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default())).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if actions {
                        photocraft_ui_egui::panels::actions::panel_menu(app, ui);
                    } else {
                        photocraft_ui_egui::symmetry_ui::menu(app, ui);
                    }
                });
            },
            app,
        );
    h.run_steps(5);
    h
}
fn capture(app: &mut PhotocraftApp, target: PathSelection) {
    app.session.actions.list.push(Action::new("Capture", vec![]));
    app.ui.actions.selected = Some(0);
    app.ui.selected_path = Some(target);
}
#[test]
fn insert_path_rejects_ambiguous_legacy_missing_and_out_of_bounds_targets() {
    for target in [PathSelection::Name("Duplicate".into()), PathSelection::Name("Missing".into()), PathSelection::Saved { index: 99 }] {
        let mut app = app(true);
        capture(&mut app, target);
        let mut h = harness(app, true);
        assert!(h.get_by_label("Insert Path").accesskit_node().is_disabled());
        h.get_by_label("Insert Path").click();
        h.run_steps(4);
        assert!(h.state().session.actions.list[0].steps.is_empty());
    }
}
#[test]
fn insert_path_keeps_indexed_unique_and_work_targets() {
    for target in [PathSelection::Saved { index: 1 }, PathSelection::Name("B".into()), PathSelection::Name("work".into())] {
        let mut app = app(matches!(target, PathSelection::Saved { .. }));
        let expected = target.path(&app.session.active().unwrap().doc).unwrap().clone();
        capture(&mut app, target);
        let mut h = harness(app, true);
        assert!(!h.get_by_label("Insert Path").accesskit_node().is_disabled());
        h.get_by_label("Insert Path").click();
        h.run_steps(4);
        let steps = &h.state().session.actions.list[0].steps;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].1["path"], photocraft_engine::vector_cmds::path_json(&expected));
    }
}
#[test]
fn insert_path_keeps_active_layer_vector_mask() {
    let mut app = app(false);
    app.run("layer.vectorMask.fromPath", json!({"name":"work"})).unwrap();
    let expected = photocraft_engine::actions_edit_cmds::selected_recording_path(&app.session, "layer").unwrap().clone();
    capture(&mut app, PathSelection::Name("layer".into()));
    let mut h = harness(app, true);
    assert!(!h.get_by_label("Insert Path").accesskit_node().is_disabled());
    h.get_by_label("Insert Path").click();
    h.run_steps(4);
    assert_eq!(h.state().session.actions.list[0].steps[0].1["path"], photocraft_engine::vector_cmds::path_json(&expected));
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
        egui::Popup::close_all(&h.ctx);
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
    assert!(canvas_tool_menu::open(&mut app, &ctx, Tool::Pen, [10., 10.]));
    canvas_tool_menu::choose(&mut app, &ctx, "edit.defineCustomShape");
    let dialog = app.ui.dialogs.last_mut().unwrap();
    assert!(dialog.fields["path"].is_object());
    dialog.fields.insert("name".into(), json!("Second"));
    let id = dialog.id;
    dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.edit_state.custom_shapes[0].path, expected);
}

#[test]
fn symmetry_work_layer_off_and_preset_remain_usable() {
    let mut app = app(false);
    app.run("layer.vectorMask.fromPath", json!({"name":"work"})).unwrap();
    let layer_name = photocraft_ui_egui::vector_ui::path_rows(&app.session.active().unwrap().doc, app.session.active().unwrap().active_layer)
        .into_iter()
        .find(|row| row.kind == photocraft_ui_egui::vector_ui::PathRow::Layer)
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
