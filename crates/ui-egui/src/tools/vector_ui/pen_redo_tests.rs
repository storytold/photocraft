use super::*;

fn draft(curvature: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = if curvature { Tool::CurvaturePen } else { Tool::Pen };
    app.ui.tool_options.vector_mode = "path".into();
    for (x, y) in [(20., 20.), (80., 20.), (80., 90.)] {
        if curvature {
            curvature_down(&mut app, x, y);
            curvature_up(&mut app);
        } else {
            pen_down(&mut app, x, y, false);
            pen_move(&mut app, x + 4., y + 6., false);
            pen_up(&mut app);
        }
    }
    app
}

#[test]
fn draft_anchor_redo_restores_exact_geometry_without_document_history() {
    for curvature in [false, true] {
        let mut app = draft(curvature);
        let before = app.ui.pen.clone().unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
        }
        assert!(app.ui.pen.is_none());
        assert!(crate::menus::is_enabled(&app, "edit.redo"));
        for _ in 0..3 {
            crate::menus::invoke(&mut app, &ctx, "edit.redo", json!({})).unwrap();
        }
        assert_eq!(app.ui.pen, Some(before));
        assert!(!pen_can_redo(&app));
        let st = app.session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&doc, &st.doc));
        assert_eq!(st.history.past_len(), history);
    }
}

#[test]
fn draft_anchor_redo_is_discarded_by_new_drawing_and_commit() {
    let mut app = draft(false);
    assert!(pen_undo_last_point(&mut app));
    pen_down(&mut app, 120., 120., false);
    pen_up(&mut app);
    assert!(!pen_redo_last_point(&mut app));
    assert!(pen_undo_last_point(&mut app));
    assert!(pen_commit(&mut app, false));
    assert!(!pen_redo_last_point(&mut app));
    assert!(app.ui.pen.is_none());
}

#[test]
fn draft_anchor_redo_refuses_changed_document_or_draft() {
    for change in ["document", "revision", "geometry", "tool"] {
        let mut app = draft(false);
        assert!(pen_undo_last_point(&mut app));
        match change {
            "document" => {
                app.session.execute("file.new", json!({"width": 50, "height": 50})).unwrap();
            }
            "revision" => {
                app.session.execute("path.set", json!({"name": "other", "path": {"subpaths": []}})).unwrap();
            }
            "geometry" => {
                app.ui.pen.as_mut().unwrap().knots[0][0][0] += 1.;
            }
            _ => app.ui.tool = Tool::Move,
        }
        let doc = app.session.active().unwrap().doc.clone();
        let pen = app.ui.pen.clone();
        assert!(!pen_redo_last_point(&mut app));
        assert_eq!(app.ui.pen, pen);
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    }
}

#[test]
fn draft_anchor_redo_keeps_work_and_saved_path_targets() {
    for name in ["work", "Saved"] {
        let mut app = draft(false);
        assert!(pen_commit(&mut app, false));
        let original = app.session.active().unwrap().doc.work_path.clone().unwrap();
        app.session.execute("path.set", json!({"name": "Saved", "path": photocraft_engine::vector_cmds::path_json(&original)})).unwrap();
        app.ui.selected_path = Some(name.into());
        pen_down(&mut app, 140., 140., false);
        pen_up(&mut app);
        pen_down(&mut app, 180., 180., false);
        pen_up(&mut app);
        let before = app.ui.pen.clone();
        let doc = app.session.active().unwrap().doc.clone();
        assert!(pen_undo_last_point(&mut app));
        assert!(pen_redo_last_point(&mut app));
        assert_eq!(app.ui.pen, before);
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
        assert!(pen_commit(&mut app, false));
        let doc = &app.session.active().unwrap().doc;
        let work = doc.work_path.as_ref().unwrap();
        let saved = &doc.paths.iter().find(|p| p.name == "Saved").unwrap().path;
        if name == "work" {
            assert_eq!(work.subpaths.len(), 2);
            assert_eq!(saved, &original);
        } else {
            assert_eq!(saved.subpaths.len(), 2);
            assert_eq!(work, &original);
        }
    }
}

#[test]
fn draft_anchor_redo_preserves_resumed_geometry() {
    let mut app = draft(false);
    assert!(pen_commit(&mut app, false));
    pen_down(&mut app, 80., 90., false);
    pen_up(&mut app);
    pen_down(&mut app, 140., 140., false);
    pen_up(&mut app);
    let pen = app.ui.pen.clone();
    let doc = app.session.active().unwrap().doc.clone();
    assert!(pen.as_ref().unwrap().resumes.is_some());
    assert!(pen_undo_last_point(&mut app));
    assert!(pen_undo_last_point(&mut app));
    assert!(app.ui.pen.is_none());
    assert!(pen_redo_last_point(&mut app));
    assert!(pen_redo_last_point(&mut app));
    assert_eq!(app.ui.pen, pen);
    assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
}

#[test]
fn draft_anchor_redo_uses_shortcut_dispatch_and_is_not_persisted() {
    let mut app = draft(false);
    let before = app.ui.pen.clone();
    assert!(pen_undo_last_point(&mut app));
    let serialized = serde_json::to_value(&app.ui).unwrap();
    assert!(serialized.get("pen_redo").is_none());
    crate::shortcut_dispatch::dispatch(&mut app, &egui::Context::default(), "edit.redo");
    assert_eq!(app.ui.pen, before);
    assert!(pen_undo_last_point(&mut app));
    app.ui.tool = Tool::Move;
    finish_pen_if_left(&mut app);
    app.ui.tool = Tool::Pen;
    assert!(!pen_redo_last_point(&mut app));
}
