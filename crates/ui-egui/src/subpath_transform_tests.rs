use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 320, "height": 200, "background": "white"})).unwrap();
    let mut path = photocraft_vector::shapes::rect(10.0, 10.0, 40.0, 40.0);
    path.subpaths.extend(photocraft_vector::shapes::ellipse(100.0, 20.0, 80.0, 60.0).subpaths);
    path.subpaths.push(photocraft_doc::Subpath::polyline(&[(220.0, 100.0), (280.0, 140.0)]));
    app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.sync_views();
    crate::work_path_drag::press(&mut app, [140.0, 20.0]);
    crate::work_path_drag::finish(&mut app, [140.0, 20.0], [140.0, 20.0]).unwrap().unwrap();
    app
}

fn work(app: &PhotocraftApp) -> photocraft_doc::Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn set_quad(app: &mut PhotocraftApp, matrix: photocraft_geom::Affine) {
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = corners(t.rect).map(|p| {
        let p = matrix.apply(photocraft_geom::Point::new(p[0], p[1]));
        [p.x, p.y]
    });
}

#[test]
fn component_box_pivot_and_preview_exclude_unselected_geometry() {
    let mut app = app();
    let before = work(&app);
    begin(&mut app, &egui::Context::default()).unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.rect, [100.0, 20.0, 180.0, 80.0]);
    assert_eq!(t.pivot, [140.0, 50.0]);
    assert_eq!(t.path.as_ref().unwrap()["subpaths"], json!([1]));
    let pv = app.transform_preview.as_ref().unwrap();
    assert_eq!(pv.path.as_ref().unwrap().subpaths, vec![before.subpaths[1].clone()]);
    let stationary = stationary_paths(t, &pv.doc).unwrap();
    assert_eq!(stationary.subpaths, vec![before.subpaths[0].clone(), before.subpaths[2].clone()]);
    set_quad(&mut app, photocraft_geom::Affine::translate(30.0, 10.0));
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(stationary_paths(t, &app.transform_preview.as_ref().unwrap().doc).unwrap(), stationary);
    assert_eq!(work(&app), before, "preview never edits document geometry");
}

#[test]
fn component_move_scale_rotate_commit_once_undo_redo_and_preserve_other_paths() {
    for matrix in [
        photocraft_geom::Affine::translate(12.0, 8.0),
        photocraft_geom::Affine::translate(-140.0, -50.0)
            .then(&photocraft_geom::Affine { m: [2.0, 0.0, 0.0, 0.5, 0.0, 0.0] })
            .then(&photocraft_geom::Affine::translate(140.0, 50.0)),
        photocraft_geom::Affine::translate(-140.0, -50.0)
            .then(&photocraft_geom::Affine::rotate(std::f64::consts::FRAC_PI_2))
            .then(&photocraft_geom::Affine::translate(140.0, 50.0)),
    ] {
        let mut app = app();
        let before = work(&app);
        let pixels = photocraft_compose::flatten(&app.session.active().unwrap().doc);
        let history = app.session.active().unwrap().history.past_len();
        begin(&mut app, &egui::Context::default()).unwrap();
        set_quad(&mut app, matrix);
        commit(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let after = work(&app);
        assert_eq!(after.subpaths[0], before.subpaths[0]);
        assert_eq!(after.subpaths[2], before.subpaths[2]);
        let expected = photocraft_doc::Path::new(vec![before.subpaths[1].clone()]).transform(&matrix);
        assert_eq!(after.subpaths[1].knots.len(), expected.subpaths[0].knots.len());
        for (a, b) in after.subpaths[1].knots.iter().zip(&expected.subpaths[0].knots) {
            for (a, b) in [(a.anchor, b.anchor), (a.in_ctrl, b.in_ctrl), (a.out_ctrl, b.out_ctrl)] {
                assert!((a.x - b.x).hypot(a.y - b.y) < 1e-6);
            }
        }
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc), pixels);
        assert_eq!(crate::work_path_drag::selected_component(&app), Some(1));
        let (_, component) = crate::work_path_drag::component_transform(&app).unwrap().unwrap();
        let bounds = component.control_bounds().unwrap();
        begin(&mut app, &egui::Context::default()).unwrap();
        let pivot = app.ui.transform.as_ref().unwrap().pivot;
        assert!((pivot[0] - (bounds.0 + bounds.2) * 0.5).abs() < 1e-9 && (pivot[1] - (bounds.1 + bounds.3) * 0.5).abs() < 1e-9);
        cancel(&mut app);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(work(&app), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(work(&app), after);
    }
}

#[test]
fn cancelling_component_box_is_non_destructive() {
    let mut app = app();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    begin(&mut app, &egui::Context::default()).unwrap();
    set_quad(&mut app, photocraft_geom::Affine::translate(40.0, 30.0));
    cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(crate::work_path_drag::selected_component(&app), Some(1));
}

#[test]
fn component_advanced_modes_and_command_distortion_are_refused() {
    for id in ["edit.transform.skew", "edit.transform.distort", "edit.transform.perspective", "edit.transform.warp", "edit.freeTransformCopy"] {
        let mut app = app();
        let before = work(&app);
        let ctx = egui::Context::default();
        assert!(!crate::menus::is_enabled(&app, id));
        assert!(crate::menus::invoke(&mut app, &ctx, id, json!({})).is_err(), "{id}");
        assert!(app.ui.transform.is_none());
        assert_eq!(work(&app), before);
    }
    let mut app = app();
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    let (_, mods) = drag_mods(&app, t, Hit::Corner(0), egui::Modifiers::COMMAND);
    assert!(!mods.command);
    let (_, mods) = drag_mods(&app, t, Hit::Edge(0), egui::Modifiers::COMMAND);
    assert!(!mods.command, "command-edge must not introduce shear");
    assert!(begin_warp(&mut app, &ctx).is_err());
    enter_warp(&mut app, &ctx);
    assert!(app.ui.transform.as_ref().unwrap().warp.is_none());
}

#[test]
fn component_revision_change_during_preview_cannot_mutate_any_geometry() {
    let mut app = app();
    begin(&mut app, &egui::Context::default()).unwrap();
    set_quad(&mut app, photocraft_geom::Affine::translate(20.0, 10.0));
    let replacement = photocraft_engine::vector_cmds::path_json(&work(&app));
    app.run("path.set", json!({"path":replacement})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(app.ui.transform.is_some(), "refused commit remains open for cancellation");
    cancel(&mut app);
}

#[test]
fn numeric_ui_commands_inherit_component_scope_and_whole_path_remains_available() {
    let mut app = app();
    let before = work(&app);
    app.run("path.transform", json!({"translateX":10})).unwrap();
    let after = work(&app);
    assert_eq!(after.subpaths[0], before.subpaths[0]);
    assert_eq!(after.subpaths[2], before.subpaths[2]);
    assert_ne!(after.subpaths[1], before.subpaths[1]);
    crate::work_path_drag::select_whole(&mut app);
    app.run("path.transform", json!({"translateY":5})).unwrap();
    assert_eq!(work(&app), after.transform(&photocraft_geom::Affine::translate(0.0, 5.0)));
}

#[test]
fn rectangle_and_open_component_use_their_own_bounds() {
    for (at, index, rect) in [([10.0, 10.0], 0, [10.0, 10.0, 50.0, 50.0]), ([220.0, 100.0], 2, [220.0, 100.0, 280.0, 140.0])] {
        let mut app = app();
        crate::work_path_drag::press(&mut app, at);
        crate::work_path_drag::finish(&mut app, at, at).unwrap().unwrap();
        let before = work(&app);
        begin(&mut app, &egui::Context::default()).unwrap();
        assert_eq!(app.ui.transform.as_ref().unwrap().rect, rect);
        set_quad(&mut app, photocraft_geom::Affine::translate(6.0, 9.0));
        commit(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        for (i, subpath) in before.subpaths.iter().enumerate() {
            if i != index {
                assert_eq!(&work(&app).subpaths[i], subpath);
            }
        }
        assert_eq!(work(&app).subpaths[index].closed, before.subpaths[index].closed);
    }
}

#[test]
fn moving_original_component_does_not_move_attached_copied_path_text() {
    let mut app = app();
    let layer = app.run("type.create", json!({"text":"copied geometry","size":12})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.attachToPath", json!({"layer":layer,"subpath":1})).unwrap();
    let before = match &app.session.active().unwrap().doc.layer(LayerId(layer)).unwrap().content {
        LayerContent::Text(t) => (t.shape.clone(), t.text.clone(), t.transform),
        _ => panic!("expected text"),
    };
    crate::work_path_drag::press(&mut app, [140.0, 20.0]);
    crate::work_path_drag::finish(&mut app, [140.0, 20.0], [140.0, 20.0]).unwrap().unwrap();
    app.run("path.transform", json!({"translateX":20})).unwrap();
    let after = match &app.session.active().unwrap().doc.layer(LayerId(layer)).unwrap().content {
        LayerContent::Text(t) => (t.shape.clone(), t.text.clone(), t.transform),
        _ => panic!("expected text"),
    };
    assert_eq!(before, after);
}

#[test]
fn cmd_t_shortcut_targets_selected_component_and_escape_cancels() {
    let mut app = app();
    let original = work(&app);
    let history = app.session.active().unwrap().history.past_len();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    input.events.push(egui::Event::Key { key: egui::Key::T, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    ctx.run_ui(input, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
    let t = app.ui.transform.as_ref().expect("Cmd+T opens a component transform");
    assert_eq!(t.rect, [100.0, 20.0, 180.0, 80.0]);
    assert_eq!(t.path.as_ref().unwrap()["subpaths"], json!([1]));
    set_quad(&mut app, photocraft_geom::Affine::translate(9.25, 7.125));
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    input.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    ctx.run_ui(input, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.ui.transform.is_none());
    assert_eq!(work(&app), original);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn tiny_and_degenerate_component_bounds_keep_a_finite_centered_pivot() {
    for points in [vec![(100.0, 100.0)], vec![(100.0, 100.0), (100.0, 100.0001)], vec![(100.0, 100.0), (100.0001, 100.0002)]] {
        let mut app = app();
        let mut path = work(&app);
        path.subpaths[1] = photocraft_doc::Subpath::polyline(&points);
        app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        crate::work_path_drag::press(&mut app, [100.0, 100.0]);
        crate::work_path_drag::finish(&mut app, [100.0, 100.0], [100.0, 100.0]).unwrap().unwrap();
        let (_, component) = crate::work_path_drag::component_transform(&app).unwrap().unwrap();
        let bounds = component.control_bounds().unwrap();
        let before = work(&app);
        begin(&mut app, &egui::Context::default()).unwrap();
        let t = app.ui.transform.as_ref().unwrap();
        assert!(t.rect.iter().chain(t.pivot.iter()).all(|v| v.is_finite()));
        assert!(t.rect[2] - t.rect[0] >= 1.0 && t.rect[3] - t.rect[1] >= 1.0);
        assert!((t.pivot[0] - (bounds.0 + bounds.2) * 0.5).abs() < 1e-9);
        assert!((t.pivot[1] - (bounds.1 + bounds.3) * 0.5).abs() < 1e-9);
        set_quad(&mut app, photocraft_geom::Affine::translate(3.25, 4.5));
        commit(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let after = work(&app);
        assert_eq!(after.subpaths[0], before.subpaths[0]);
        assert_eq!(after.subpaths[2], before.subpaths[2]);
        assert_eq!(crate::work_path_drag::selected_component(&app), Some(1));
    }
}

#[test]
fn stale_component_scope_cannot_open_a_whole_work_path_box() {
    let mut app = app();
    app.session.active_mut().unwrap().revision += 1;
    let before = app.session.active().unwrap().doc.clone();
    assert!(begin(&mut app, &egui::Context::default()).is_err());
    assert!(app.ui.transform.is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
    crate::work_path_drag::reconcile(&mut app, true);
    assert!(begin(&mut app, &egui::Context::default()).is_err());
    crate::work_path_drag::select_whole(&mut app);
    begin(&mut app, &egui::Context::default()).unwrap();
    assert!(app.ui.transform.as_ref().unwrap().path.as_ref().unwrap().get("subpaths").is_none());
    cancel(&mut app);
}

#[test]
fn shape_and_explicit_saved_contexts_do_not_inherit_a_component_selector() {
    let mut app = app();
    app.run("path.set", json!({"name":"Saved","path":{"subpaths":[{"knots":[[5,5],[10,20]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    crate::work_path_drag::clear_selection(&mut app);
    assert!(crate::work_path_drag::component_transform(&app).unwrap().is_none());
    app.run("shape.create", json!({"kind":"rect","rect":[10,20,40,30]})).unwrap();
    app.ui.selected_path = Some("layer".into());
    assert!(crate::work_path_drag::component_transform(&app).unwrap().is_none());
    let before = work(&app);
    begin(&mut app, &egui::Context::default()).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().path.as_ref().unwrap()["name"], json!("layer"));
    cancel(&mut app);
    assert_eq!(work(&app), before);
}

#[test]
fn direct_run_refuses_component_advanced_commands_before_any_mutation() {
    for id in
        ["edit.transform.skew", "edit.transform.distort", "edit.transform.perspective", "edit.transform.warp", "edit.freeTransformCopy", "edit.transform.again"]
    {
        let mut app = app();
        let original = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let result = app.run(id, json!({}));
        assert_eq!(result.unwrap_err(), COMPONENT_AFFINE_ONLY, "{id}");
        assert_eq!(app.session.active().unwrap().doc, original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert!(app.ui.transform.is_none());
    }
}

#[test]
fn component_transform_after_sampler_change_uses_current_revision() {
    let mut app = app();
    let original = work(&app);
    app.run("view.colorSamplers.add", json!({"x":2,"y":3})).unwrap();
    app.run("path.transform", json!({"translateX":6.25,"translateY":3.5})).unwrap();
    let moved = work(&app);
    assert_eq!(moved.subpaths[0], original.subpaths[0]);
    assert_ne!(moved.subpaths[1], original.subpaths[1]);
    assert_eq!(moved.subpaths[2], original.subpaths[2]);
    assert_eq!(crate::work_path_drag::selected_component(&app), Some(1));
}

#[test]
fn explicit_component_selector_is_independent_of_stale_gui_selection() {
    for invalidated in [false, true] {
        let mut app = app();
        app.session.active_mut().unwrap().revision += 1;
        if invalidated {
            crate::work_path_drag::reconcile(&mut app, true);
        }
        assert!(crate::work_path_drag::component_transform(&app).is_err());
        let st = app.session.active().unwrap();
        let params = json!({"subpaths":[0], "document":st.doc.id.0, "revision":st.revision, "translateX":6.25, "translateY":3.5});
        let history = st.history.past_len();
        let mut expected = work(&app);
        expected.subpaths[0] =
            photocraft_doc::Path::new(vec![expected.subpaths[0].clone()]).transform(&photocraft_geom::Affine::translate(6.25, 3.5)).subpaths.remove(0);
        app.run("path.transform", params).expect("a current explicit selector must reach engine validation despite stale GUI state");
        assert_eq!(work(&app), expected);
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    }
}

#[test]
fn explicit_component_selector_errors_still_reach_engine_validation() {
    for (key, value, message) in [
        ("document", json!(0), "stale component target"),
        ("revision", json!(0), "stale component target"),
        ("document", json!(null), "stale component target"),
        ("revision", json!(null), "stale component target"),
        ("subpaths", json!(null), "exactly one component index"),
        ("subpaths", json!([]), "exactly one component index"),
        ("subpaths", json!([0, 1]), "exactly one component index"),
        ("subpaths", json!([-1]), "invalid component index"),
        ("subpaths", json!(["0"]), "invalid component index"),
        ("subpaths", json!([99]), "no such Work Path component"),
    ] {
        let mut app = app();
        app.session.active_mut().unwrap().revision += 1;
        let st = app.session.active().unwrap();
        let original = st.doc.clone();
        let history = st.history.past_len();
        let revision = st.revision;
        let mut params = json!({"subpaths":[0], "document":st.doc.id.0, "revision":revision, "translateX":6.25});
        if value.is_null() && key != "subpaths" {
            params.as_object_mut().unwrap().remove(key);
        } else {
            params[key] = value;
        }
        assert_eq!(crate::work_path_drag::transform_params(&app, params.clone()).unwrap(), params);
        let error = app.run("path.transform", params).unwrap_err();
        assert!(error.contains(message), "{key}: {error}");
        assert_eq!(app.session.active().unwrap().doc, original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(app.session.active().unwrap().revision, revision);
    }
}

#[test]
fn implicit_component_selector_keeps_gui_scope_and_stale_guard() {
    for stale in [false, true] {
        let mut app = app();
        if stale {
            app.session.active_mut().unwrap().revision += 1;
        }
        let original = work(&app);
        let history = app.session.active().unwrap().history.past_len();
        let result = app.run("path.transform", json!({"translateX":6.25, "translateY":3.5}));
        if stale {
            assert!(result.unwrap_err().contains("Work Path selection changed"));
            assert_eq!(work(&app), original);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        } else {
            result.unwrap();
            let mut expected = original;
            expected.subpaths[1] =
                photocraft_doc::Path::new(vec![expected.subpaths[1].clone()]).transform(&photocraft_geom::Affine::translate(6.25, 3.5)).subpaths.remove(0);
            assert_eq!(work(&app), expected);
            assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        }
    }
}
