use super::*;
use egui::{Pos2, RawInput, Rect, vec2};
use photocraft_engine::Session;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":100,"height":80})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Brush;
    app
}

fn open(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "ui.symmetryTransform", json!({})).unwrap();
}

fn is_valid(app: &PhotocraftApp) -> bool {
    app.ui.symmetry_transform.as_ref().is_some_and(|t| valid(app, t))
}

fn canvas_pass(app: &mut PhotocraftApp, focused: bool) {
    let ctx = app.ctx().clone();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
    ctx.run_ui(RawInput { focused, screen_rect: Some(rect), ..Default::default() }, |ui| canvas_controls(ui, app, rect)).textures_delta.clear();
}

#[test]
fn cancel_competing_edits_tool_document_and_focus_changes_drop_only_the_preview() {
    let mut app = app();
    app.run("paint.setSymmetry", json!({"mode":"vertical"})).unwrap();
    open(&mut app);
    assert!(app.ui.symmetry_transform.as_ref().and_then(|t| session_tool(&app, t)).is_some());
    app.run("document.inspect", json!({})).unwrap();
    assert!(is_valid(&app));
    finish_tool(&mut app, vec![SymmetryEvent::Discard]);
    assert!(app.ui.symmetry_transform.is_none());
    assert_eq!(preset(&app).unwrap().center, [50.0, 40.0]);

    open(&mut app);
    app.run("paint.setSymmetry", json!({"mode":"horizontal"})).unwrap();
    assert!(app.ui.symmetry_transform.is_none());

    for change in 0..3 {
        open(&mut app);
        match change {
            0 => app.ui.tool = Tool::Pencil,
            1 => {
                app.session.execute("layer.new.layer", json!({})).unwrap();
            }
            _ => {
                app.session.execute("file.new", json!({"width":10,"height":10})).unwrap();
            }
        }
        assert!(!is_valid(&app), "change {change} invalidates the session");
        app.ui.tool = Tool::Brush;
        if change == 2 {
            assert!(app.session.set_active(0));
        }
        app.ui.symmetry_transform = None;
    }

    open(&mut app);
    canvas_pass(&mut app, true);
    assert!(app.ui.symmetry_transform.is_some());
    canvas_pass(&mut app, false);
    assert!(app.ui.symmetry_transform.is_none());
    assert_eq!(preset(&app).unwrap().mode, SymmetryMode::Horizontal);
}

#[test]
fn menu_selects_all_modes_resets_positions_and_preserves_work_path_choice() {
    let mut app = app();
    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"knots":[[20,0],[20,80]]}]}})).unwrap();
    let ctx = app.ctx().clone();
    for mode in [FwMode::Vertical, FwMode::Horizontal, FwMode::Dual, FwMode::Diagonal] {
        pick(&mut app, &ctx, SymmetryCommand::Mode(mode));
        let p = preset(&app).unwrap();
        assert_eq!(Some(p.mode), mode_from_framework(mode));
        assert_eq!(p.center, [50.0, 40.0]);
    }
    pick(&mut app, &ctx, SymmetryCommand::FromPath("work".into()));
    assert_eq!(app.session.active().unwrap().symmetry.as_ref().unwrap().path_source(), Some("work"));
    assert!(!can_transform(&app));
    pick(&mut app, &ctx, SymmetryCommand::Off);
    assert!(preset(&app).is_none());
}
