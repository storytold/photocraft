//! Draft-anchor history composes with finished-path recovery through the shared command route.
use super::*;

fn app(curvature: bool, name: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":200,"height":200})).unwrap();
    if name != "work" {
        app.run("path.set", json!({"name":name,"path":{"subpaths":[{"knots":[[140,140],[180,140]]}]}})).unwrap();
        app.ui.selected_path = Some(name.into());
    }
    if name == "work" {
        app.run("path.set", json!({"name":"Other","path":{"subpaths":[{"knots":[[140,140],[180,140]]}]}})).unwrap();
    }
    app.ui.tool = if curvature { Tool::CurvaturePen } else { Tool::Pen };
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
fn menu(app: &mut PhotocraftApp, id: &str) {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).unwrap();
}
fn recovered(curvature: bool, name: &str) -> PhotocraftApp {
    let mut app = app(curvature, name);
    assert!(pen_commit(&mut app, false));
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
    app
}

#[test]
fn recovered_anchor_future_enables_and_dispatches_redo_before_finish() {
    for curvature in [false, true] {
        for name in ["work", "Saved"] {
            let mut app = recovered(curvature, name);
            let before = app.ui.pen.clone();
            menu(&mut app, "edit.undo");
            let doc = app.session.active().unwrap().doc.clone();
            assert!(crate::menus::is_enabled(&app, "edit.redo"), "anchor future must enable Redo");
            menu(&mut app, "edit.redo");
            assert_eq!(app.ui.pen, before, "Redo restores an anchor rather than consuming the draft");
            assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
            menu(&mut app, "edit.redo");
            assert!(app.ui.pen.is_none(), "only the next Redo finishes the restored draft");
        }
    }
}

#[test]
fn exhausted_recovered_draft_redoes_each_anchor_before_document_finish() {
    for curvature in [false, true] {
        for name in ["work", "Saved"] {
            let mut app = recovered(curvature, name);
            let before = app.ui.pen.clone();
            for _ in 0..3 {
                menu(&mut app, "edit.undo");
            }
            assert!(app.ui.pen.is_none());
            let doc = app.session.active().unwrap().doc.clone();
            for count in 1..=3 {
                assert!(crate::menus::is_enabled(&app, "edit.redo"));
                menu(&mut app, "edit.redo");
                assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(count), "draft anchors precede finish Redo");
                assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
            }
            assert_eq!(app.ui.pen, before);
            menu(&mut app, "edit.redo");
            assert!(app.ui.pen.is_none());
        }
    }
}

#[test]
fn step_forward_restores_draft_anchor_with_matching_enabled_state() {
    for curvature in [false, true] {
        let mut app = app(curvature, "work");
        let before = app.ui.pen.clone();
        menu(&mut app, "history.stepBackward");
        assert!(crate::menus::is_enabled(&app, "history.stepForward"));
        menu(&mut app, "history.stepForward");
        assert_eq!(app.ui.pen, before);
    }
}

#[test]
fn anchor_redo_preserves_unrelated_direct_selection() {
    let mut app = app(false, "work");
    assert!(pen_undo_last_point(&mut app));
    app.ui.direct_selection.target = Some(crate::direct_select::PathRef::Saved(0));
    app.ui.direct_selection.anchors = vec![[0, 0]];
    let selection = app.ui.direct_selection.clone();
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.direct_selection, selection);
}

#[test]
fn transform_undo_and_redo_leave_the_anchor_future_available() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":200,"height":200})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20., 20.), (80., 20.)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let before = app.ui.pen.clone();
    menu(&mut app, "edit.undo");
    let count = app.ui.pen_redo.len();
    crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
    key(&mut app, egui::Modifiers::COMMAND);
    key(&mut app, egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND });
    assert_eq!(app.ui.pen_redo.len(), count);
    for id in ["edit.undo", "edit.redo", "history.stepBackward", "history.stepForward"] {
        if id.starts_with("history.") {
            assert!(!crate::menus::is_enabled(&app, id));
        }
        let result = crate::menus::invoke(&mut app, &egui::Context::default(), id, json!({}));
        if id.starts_with("history.") {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert_eq!(app.ui.pen_redo.len(), count);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 1);
    }
    crate::transform_tool::cancel(&mut app);
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen, before);
}

#[test]
fn crop_box_history_never_consumes_the_anchor_future() {
    let mut app = app(false, "work");
    let before = app.ui.pen.clone();
    menu(&mut app, "edit.undo");
    let count = app.ui.pen_redo.len();
    let draft = app.ui.pen.clone();
    // Install a crop modal context directly: ordinary leaving-Pen commit semantics are not under test.
    app.ui.tool = Tool::Crop;
    for event in [
        crate::canvas::ToolEvent::Down { x: 30., y: 30., pressure: 1. },
        crate::canvas::ToolEvent::Move { x: 150., y: 150., pressure: 1. },
        crate::canvas::ToolEvent::Up { x: 150., y: 150. },
    ] {
        crate::crop_ui::pointer(&mut app, event, egui::Modifiers::NONE);
    }
    assert!(crate::crop_ui::pending(&app));
    for id in ["edit.undo", "edit.redo", "history.stepBackward", "history.stepForward"] {
        assert!(super::box_steps(&app, id));
        if id.starts_with("history.") {
            assert!(!crate::menus::is_enabled(&app, id));
        }
        let result = crate::menus::invoke(&mut app, &egui::Context::default(), id, json!({}));
        if id.starts_with("history.") {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert_eq!(app.ui.pen_redo.len(), count);
        assert_eq!(app.ui.pen, draft);
    }
    crate::crop_ui::cancel(&mut app);
    app.ui.tool = Tool::Pen;
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen, before);
}

fn key(app: &mut PhotocraftApp, modifiers: egui::Modifiers) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers },
        ],
        ..Default::default()
    });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn keyboard_redo_restores_recovered_anchors_before_finishing() {
    for curvature in [false, true] {
        let mut app = recovered(curvature, "work");
        let before = app.ui.pen.clone();
        key(&mut app, egui::Modifiers::COMMAND);
        key(&mut app, egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND });
        assert_eq!(app.ui.pen, before);
    }
}

#[test]
fn anchor_redo_composes_with_close_enter_and_escape_for_work_and_saved_paths() {
    for curvature in [false, true] {
        for name in ["work", "Saved"] {
            for finish in ["close", "enter", "escape"] {
                let mut app = app(curvature, name);
                match finish {
                    "close" => {
                        if curvature {
                            curvature_down(&mut app, 20., 20.);
                            curvature_up(&mut app);
                        } else {
                            pen_down(&mut app, 20., 20., false);
                            pen_up(&mut app);
                        }
                    }
                    "enter" => pen_enter(&mut app),
                    _ => pen_end(&mut app),
                }
                assert!(app.ui.pen.is_none());
                let finished = app.session.active().unwrap().doc.clone();
                menu(&mut app, "edit.undo");
                let draft = app.ui.pen.clone();
                assert!(draft.is_some());
                menu(&mut app, "edit.undo");
                menu(&mut app, "edit.redo");
                assert_eq!(app.ui.pen, draft, "{curvature}, {name}, {finish}");
                menu(&mut app, "edit.redo");
                assert!(app.ui.pen.is_none());
                let doc = &app.session.active().unwrap().doc;
                assert_eq!(doc.work_path, finished.work_path);
                assert_eq!(doc.paths, finished.paths);
            }
        }
    }
}

#[test]
fn new_drawing_after_recovered_anchor_undo_invalidates_only_the_anchor_future() {
    let mut app = recovered(false, "work");
    menu(&mut app, "edit.undo");
    pen_down(&mut app, 120., 140., false);
    pen_up(&mut app);
    let draft = app.ui.pen.clone();
    assert!(!pen_can_redo(&app));
    assert!(!crate::menus::is_enabled(&app, "edit.redo"));
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen, draft, "stale document finish cannot consume the new draft");
}

#[test]
fn recovered_resumed_work_and_saved_drafts_restore_before_their_finish() {
    for name in ["work", "Saved"] {
        let mut app = app(false, name);
        assert!(pen_commit(&mut app, false));
        pen_down(&mut app, 80., 90., false);
        pen_up(&mut app);
        pen_down(&mut app, 120., 140., false);
        pen_up(&mut app);
        assert!(app.ui.pen.as_ref().unwrap().resumes.is_some());
        assert!(pen_commit(&mut app, false));
        let finished = app.session.active().unwrap().doc.clone();
        menu(&mut app, "edit.undo");
        let before = app.ui.pen.clone();
        assert!(before.as_ref().unwrap().resumes.is_some());
        menu(&mut app, "edit.undo");
        menu(&mut app, "edit.undo");
        assert!(app.ui.pen.is_none());
        let source = app.session.active().unwrap().doc.clone();
        menu(&mut app, "edit.redo");
        assert!(app.ui.pen.as_ref().unwrap().resumes.is_some());
        assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
        menu(&mut app, "edit.redo");
        assert_eq!(app.ui.pen, before);
        menu(&mut app, "edit.redo");
        assert!(app.ui.pen.is_none());
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.work_path, finished.work_path);
        assert_eq!(doc.paths, finished.paths);
    }
}
