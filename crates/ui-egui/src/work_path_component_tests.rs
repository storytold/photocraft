use super::*;
use crate::canvas::{ToolEvent, tool_event};
use photocraft_doc::vector::Subpath;
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":400,"height":200,"background":"white"})).unwrap();
    let mut p = photocraft_vector::shapes::rect(10.0, 20.0, 50.0, 50.0);
    p.subpaths.extend(photocraft_vector::shapes::ellipse(100.0, 20.0, 80.0, 60.0).subpaths);
    p.subpaths.push(Subpath::polyline(&[(220.0, 40.0), (300.0, 60.0)]));
    app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&p)})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.ui.view.extras = true;
    app.ui.view.show.target_path = true;
    app.sync_views();
    app
}
fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}
fn down(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
}
fn moving(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
}
fn up(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, egui::Modifiers::NONE);
}

#[test]
fn first_middle_last_outline_click_selects_only_that_component_without_history() {
    for (i, p) in [(0, [10.0, 20.0]), (1, [140.0, 20.0]), (2, [220.0, 40.0])] {
        let mut app = app();
        let original = work(&app);
        let history = app.session.active().unwrap().history.past_len();
        down(&mut app, p);
        up(&mut app, p);
        assert_eq!(selected_component(&app), Some(i));
        assert_eq!(work(&app), original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}
#[test]
fn fractional_component_preview_commit_and_undo_leave_neighbours_byte_identical() {
    let mut app = app();
    let original = work(&app);
    let h = app.session.active().unwrap().history.past_len();
    let revision = app.session.active().unwrap().revision;
    let pixels = photocraft_compose::flatten(&app.session.active().unwrap().doc);
    down(&mut app, [140.0, 20.0]);
    for end in [[141.125, 20.0625], [151.375, 24.125]] {
        moving(&mut app, end);
        assert_eq!(selected_component(&app), Some(1));
        assert_eq!(preview(&app), Some([end[0] - 140.0, end[1] - 20.0]));
        assert_eq!(work(&app), original);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active().unwrap().history.past_len(), h);
    }
    up(&mut app, [151.375, 24.125]);
    let after = work(&app);
    let mut expected = original.clone();
    let anchors = photocraft_vector::edit::subpath_anchors(&expected, 1);
    photocraft_vector::edit::move_anchors(&mut expected, &anchors, [11.375, 4.125]).unwrap();
    assert_eq!(after, expected);
    for i in [0, 2] {
        assert_eq!(serde_json::to_vec(&after.subpaths[i]).unwrap(), serde_json::to_vec(&original.subpaths[i]).unwrap());
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), h + 1);
    assert_eq!(selected_component(&app), Some(1));
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc), pixels);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(work(&app), original);
    assert!(selected_component(&app).is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(work(&app), after);
}
#[test]
fn paths_panel_whole_selection_and_following_component_click_both_work() {
    let mut app = app();
    select_whole(&mut app);
    let original = work(&app);
    down(&mut app, [10.0, 20.0]);
    moving(&mut app, [17.125, 23.5]);
    assert_eq!(preview(&app), Some([7.125, 3.5]));
    assert!(selected_component(&app).is_none());
    up(&mut app, [17.125, 23.5]);
    assert_eq!(work(&app), original.transform(&Affine::translate(7.125, 3.5)));
    down(&mut app, [147.125, 23.5]);
    up(&mut app, [147.125, 23.5]);
    assert_eq!(selected_component(&app), Some(1));
}
#[test]
fn invalid_component_index_never_becomes_whole_path() {
    let mut app = app();
    down(&mut app, [140.0, 20.0]);
    app.ui.work_path_drag.selection.as_mut().unwrap().component = Some(99);
    let original = work(&app);
    let h = app.session.active().unwrap().history.past_len();
    assert!(preview(&app).is_none());
    assert!(finish(&mut app, [140.0, 20.0], [151.0, 24.0]).unwrap().is_err());
    assert_eq!(work(&app), original);
    assert_eq!(app.session.active().unwrap().history.past_len(), h);
}
#[test]
fn stale_revisions_document_switch_replacement_and_named_target_cancel() {
    for cause in ["revision", "document", "geometry", "saved", "tool"] {
        let mut app = app();
        down(&mut app, [140.0, 20.0]);
        moving(&mut app, [151.0, 24.0]);
        match cause {
            "revision" => app.session.active_mut().unwrap().revision += 1,
            "document" => {
                app.run("file.new", json!({"width":80,"height":80})).unwrap();
            }
            "geometry" => {
                std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).work_path = Some(photocraft_vector::shapes::rect(5.0, 5.0, 20.0, 20.0))
            }
            "saved" => app.ui.selected_path = Some("Saved".into()),
            "tool" => app.ui.tool = Tool::DirectSelection,
            _ => assert!(matches!(cause, "revision" | "document" | "geometry" | "saved" | "tool")),
        }
        reconcile(&mut app, true);
        let before = app.session.active().unwrap().doc.work_path.clone();
        let h = app.session.active().unwrap().history.past_len();
        assert!(preview(&app).is_none());
        assert!(selected_component(&app).is_none());
        assert!(finish(&mut app, [140.0, 20.0], [151.0, 24.0]).unwrap().is_err());
        assert_eq!(app.session.active().unwrap().doc.work_path, before);
        assert_eq!(app.session.active().unwrap().history.past_len(), h);
    }
}
#[test]
fn escape_cancels_component_drag_through_actual_shortcut() {
    let mut app = app();
    let original = work(&app);
    let h = app.session.active().unwrap().history.past_len();
    down(&mut app, [140.0, 20.0]);
    moving(&mut app, [151.0, 24.0]);
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    ctx.run_ui(input, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.drag.is_none());
    assert!(preview(&app).is_none());
    assert!(finish(&mut app, [140.0, 20.0], [151.0, 24.0]).unwrap().is_err());
    assert_eq!(work(&app), original);
    assert_eq!(app.session.active().unwrap().history.past_len(), h);
}
fn overlay(app: &PhotocraftApp) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let xf = crate::canvas::ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
        zoom: 1.0,
        center: [0.0, 0.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::background());
        crate::vector_ui::draw_overlay(app, &painter, &xf, &app.session.active().unwrap().doc);
    });
    out.textures_delta.clear();
    out.shapes.into_iter().map(|s| s.shape).collect()
}
#[test]
fn renderer_translates_only_selected_outline_and_anchors_without_ghosts() {
    let mut app = app();
    down(&mut app, [140.0, 20.0]);
    let before = overlay(&app);
    moving(&mut app, [151.25, 24.5]);
    let during = overlay(&app);
    let paths = |s: &[egui::Shape]| {
        s.iter()
            .filter_map(|s| match s {
                egui::Shape::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let a = paths(&before);
    let b = paths(&during);
    assert_eq!(a.len(), 3);
    assert_eq!(b.len(), 3);
    assert_eq!(a[0], b[0]);
    assert_eq!(a[2], b[2]);
    for (a, b) in a[1].points.iter().zip(&b[1].points) {
        assert!((*b - *a - egui::vec2(11.25, 4.5)).length() < 0.001);
    }
    let anchors = |s: &[egui::Shape]| {
        s.iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) => Some(r.rect.center()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let a = anchors(&before);
    let b = anchors(&during);
    assert_eq!(a.len(), 20);
    assert_eq!(b.len(), 20);
    for (i, (a, b)) in a.iter().zip(&b).enumerate() {
        let delta = if (8..16).contains(&i) { egui::vec2(11.25, 4.5) } else { egui::Vec2::ZERO };
        assert!((*b - *a - delta).length() < 0.001);
    }
}
#[test]
fn saved_paths_and_direct_selection_keep_their_existing_behaviour() {
    let mut app = app();
    down(&mut app, [140.0, 20.0]);
    up(&mut app, [140.0, 20.0]);
    app.ui.tool = Tool::DirectSelection;
    let original = work(&app);
    for event in [ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, ToolEvent::Move { x: 14.0, y: 26.0, pressure: 1.0 }, ToolEvent::Up { x: 14.0, y: 26.0 }]
    {
        assert!(crate::direct_select::pointer(&mut app, event, egui::Modifiers::NONE));
    }
    let moved = work(&app);
    assert_ne!(moved.subpaths[0].knots[0].anchor, original.subpaths[0].knots[0].anchor);
    assert_eq!(moved.subpaths[1], original.subpaths[1]);
    app.run("path.set", json!({"name":"Saved","path":{"subpaths":[{"knots":[[5,5],[15,15]]}]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.ui.selected_path = Some("Saved".into());
    let before = app.session.active().unwrap().doc.clone();
    press(&mut app, [5.0, 5.0]);
    assert!(finish(&mut app, [5.0, 5.0], [10.0, 10.0]).is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
}
#[test]
fn overlapping_and_open_components_are_deterministic_transient_and_persist_geometry() {
    let mut app = app();
    let mut p = work(&app);
    p.subpaths.push(p.subpaths[0].clone());
    app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&p)})).unwrap();
    down(&mut app, [10.0, 20.0]);
    up(&mut app, [10.0, 20.0]);
    let chosen = selected_component(&app);
    assert!(chosen.is_some());
    down(&mut app, [10.0, 20.0]);
    up(&mut app, [10.0, 20.0]);
    assert_eq!(selected_component(&app), chosen);
    down(&mut app, [220.0, 40.0]);
    moving(&mut app, [222.125, 43.5]);
    up(&mut app, [222.125, 43.5]);
    assert_eq!(selected_component(&app), Some(2));
    assert!(!work(&app).subpaths[2].closed);
    assert!(serde_json::to_value(&app.ui).unwrap().get("work_path_drag").is_none());
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().work_path, Some(work(&app)));
}

#[test]
fn empty_canvas_release_is_a_silent_non_mutating_miss() {
    let mut app = app();
    let original = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    down(&mut app, [350.0, 170.0]);
    up(&mut app, [350.0, 170.0]);
    assert!(!app.ui.status_error, "an ordinary miss is not a cancellation: {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc, original);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(selected_component(&app).is_none());
}

#[test]
fn cancelling_or_losing_focus_after_a_miss_keeps_late_release_owned() {
    for focus_loss in [false, true] {
        let mut app = app();
        down(&mut app, [350.0, 170.0]);
        if focus_loss {
            reconcile(&mut app, false);
        } else {
            cancel(&mut app);
        }
        assert!(finish(&mut app, [350.0, 170.0], [350.0, 170.0]).unwrap().is_err());
        assert!(!owns_drag(&app));
    }
}

#[test]
fn sampler_only_revisions_preserve_selected_component_and_real_edits_invalidate_it() {
    let mut app = app();
    down(&mut app, [140.0, 20.0]);
    up(&mut app, [140.0, 20.0]);
    assert_eq!(selected_component(&app), Some(1));
    let original = work(&app);
    let history = app.session.active().unwrap().history.past_len();
    for (id, params) in [
        ("view.colorSamplers.add", json!({"x":1,"y":2})),
        ("view.colorSamplers.move", json!({"index":0,"x":3,"y":4})),
        ("view.colorSamplers.delete", json!({"index":0})),
        ("view.colorSamplers.clear", json!({})),
    ] {
        app.run(id, params).unwrap();
        assert_eq!(selected_component(&app), Some(1), "{id} must not invalidate unchanged Work Path geometry");
        assert_eq!(work(&app), original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
    app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&original)})).unwrap();
    assert!(selected_component(&app).is_none(), "a real path command still invalidates even identical geometry");
}

#[test]
fn sampler_revision_refresh_does_not_resurrect_an_already_stale_selection() {
    let mut app = app();
    down(&mut app, [140.0, 20.0]);
    up(&mut app, [140.0, 20.0]);
    app.session.active_mut().unwrap().revision += 1;
    app.run("view.colorSamplers.clear", json!({})).unwrap();
    assert!(selected_component(&app).is_none());
}
