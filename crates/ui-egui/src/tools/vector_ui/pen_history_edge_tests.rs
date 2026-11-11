//! Edge ownership checks shared with the exact upstream comparison.
use super::*;
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":200,"height":200})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
    app.ui.tool = Tool::Pen;
    app
}
fn menu(app: &mut PhotocraftApp, id: &str) {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).unwrap();
}
fn draw(app: &mut PhotocraftApp, curvature: bool) {
    app.ui.tool = if curvature { Tool::CurvaturePen } else { Tool::Pen };
    for (x, y) in [(20., 20.), (80., 20.), (80., 90.)] {
        if curvature {
            curvature_down(app, x, y);
            curvature_up(app);
        } else {
            pen_down(app, x, y, false);
            pen_up(app);
        }
    }
}
fn key(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}
#[test]
fn document_redo_must_not_discard_a_new_valid_owned_draft() {
    for curvature in [false, true] {
        let mut app = app();
        app.run("select.rect", json!({"x":5,"y":5,"width":10,"height":10})).unwrap();
        menu(&mut app, "edit.undo");
        draw(&mut app, curvature);
        let draft = app.ui.pen.clone();
        let doc = app.session.active().unwrap().doc.clone();
        menu(&mut app, "edit.redo");
        assert_eq!(app.ui.pen, draft, "unrelated document Redo must not erase a valid draft");
        assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
        assert!(!crate::menus::is_enabled(&app, "edit.redo"));
    }
}
#[test]
fn modal_transform_backspace_delete_leave_the_pen_draft_alone() {
    for curvature in [false, true] {
        for k in [egui::Key::Backspace, egui::Key::Delete] {
            let mut app = app();
            draw(&mut app, curvature);
            let draft = app.ui.pen.clone();
            crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
            key(&mut app, k);
            assert_eq!(app.ui.pen, draft, "{curvature}, {k:?}: modal box owns the gesture");
            assert!(app.ui.transform.is_some());
        }
    }
}
#[test]
fn non_pen_modal_history_keeps_menu_and_direct_command_routing_unchanged() {
    let (mut a, mut b) = (app(), app());
    for app in [&mut a, &mut b] {
        app.ui.tool = Tool::Move;
        crate::transform_tool::begin(app, &egui::Context::default()).unwrap();
    }
    let menu = crate::menus::invoke(&mut a, &egui::Context::default(), "history.stepBackward", json!({}));
    let command = b.run("history.stepBackward", json!({}));
    assert_eq!(menu.is_ok(), command.is_ok(), "Pen integration must not add a non-Pen-only menu restriction");
    assert_eq!(a.session.active().unwrap().history.past_len(), b.session.active().unwrap().history.past_len());
}
#[test]
fn crossing_older_document_history_redoes_the_document_before_the_finish() {
    let mut app = app();
    draw(&mut app, false);
    assert!(pen_commit(&mut app, false));
    let finished = app.session.active().unwrap().doc.work_path.clone();
    let count = app.session.active().unwrap().history.past_len();
    menu(&mut app, "edit.undo");
    for _ in 0..3 {
        menu(&mut app, "edit.undo");
    }
    assert!(app.ui.pen.is_none());
    menu(&mut app, "edit.undo");
    assert_eq!(app.session.active().unwrap().history.past_len(), count - 2);
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), count - 1);
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(app.session.active().unwrap().doc.work_path, finished);
    assert_eq!(app.session.active().unwrap().history.past_len(), count);
}

#[test]
fn extra_redo_after_anchor_replay_preserves_the_unfinished_draft() {
    for curvature in [false, true] {
        for name in ["work", "Saved"] {
            let mut app = app();
            if name != "work" {
                app.run("path.set", json!({"name":name,"path":{"subpaths":[]}})).unwrap();
            }
            app.run("select.rect", json!({"x":5,"y":5,"width":10,"height":10})).unwrap();
            menu(&mut app, "edit.undo");
            if name != "work" {
                app.ui.selected_path = Some(name.into());
            }
            draw(&mut app, curvature);
            let draft = app.ui.pen.clone();
            menu(&mut app, "edit.undo");
            menu(&mut app, "edit.redo");
            assert_eq!(app.ui.pen, draft);
            let doc = app.session.active().unwrap().doc.clone();
            menu(&mut app, "edit.redo");
            assert_eq!(app.ui.pen, draft, "{curvature}, {name}: extra Redo must not enter unrelated document history");
            assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
            assert!(!crate::menus::is_enabled(&app, "edit.redo"));
        }
    }
}

#[test]
fn modal_anchor_removal_keeps_the_saved_future() {
    for k in [egui::Key::Backspace, egui::Key::Delete] {
        let mut app = app();
        draw(&mut app, false);
        let full = app.ui.pen.clone();
        menu(&mut app, "edit.undo");
        menu(&mut app, "edit.undo");
        let draft = app.ui.pen.clone();
        let future = app.ui.pen_redo.clone();
        assert_eq!(future.len(), 2);
        crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
        key(&mut app, k);
        assert_eq!(app.ui.pen, draft);
        assert_eq!(app.ui.pen_redo, future);
        crate::transform_tool::cancel(&mut app);
        menu(&mut app, "edit.redo");
        menu(&mut app, "edit.redo");
        assert_eq!(app.ui.pen, full);
    }
}

#[test]
fn crossing_document_history_invalidates_the_anchor_future_instead_of_replaying_it() {
    let mut app = app();
    draw(&mut app, false);
    assert!(pen_commit(&mut app, false));
    menu(&mut app, "edit.undo");
    for _ in 0..3 {
        menu(&mut app, "edit.undo");
    }
    assert_eq!(app.ui.pen_redo.len(), 3);
    menu(&mut app, "edit.undo");
    assert!(!pen_can_redo(&app), "the source document changed");
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen_redo.is_empty(), "document Redo must discard the stale anchor future");
    assert!(app.ui.pen.is_none());
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.is_some());
}
#[test]
fn document_redo_still_runs_after_document_edits_invalidate_a_draft() {
    let mut app = app();
    draw(&mut app, false);
    app.run("select.rect", json!({"x":5,"y":5,"width":10,"height":10})).unwrap();
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.redo");
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert!(app.ui.pen.is_none(), "an invalidated draft must not block legitimate document history");
}
