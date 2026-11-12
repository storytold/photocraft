//! #12905 measured Pen defaults and legacy/unmeasured controls.
use super::*;
use photocraft_doc::vector::PathOp;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = Tool::Pen;
    app
}

fn square(app: &mut PhotocraftApp, x: f64, y: f64, side: f64) {
    for (dx, dy) in [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)] {
        pen_down(app, x + dx, y + dy, false);
        pen_up(app);
    }
    pen_down(app, x + 0.5, y + 0.5, false);
    pen_up(app);
}

#[test]
fn fresh_pen_path_defaults_to_exclude() {
    let app = app();
    assert_eq!(app.ui.tool_options.vector_mode, "path");
    assert_eq!(tool_path_op(&app), Some("exclude"));
}

#[test]
fn reset_pen_restores_path_exclude() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    crate::path_components::set_op(&mut app, "subtract").unwrap();
    app.ui.tool_options.vector_mode = "path".into();
    crate::path_components::set_op(&mut app, "intersect").unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert_eq!((app.ui.tool_options.pen_path_mode_op.as_ref(), app.ui.tool_options.pen_shape_mode_op.as_ref()), (None, None));
    assert_eq!(app.ui.tool_options.vector_mode, "path");
    assert_eq!(tool_path_op(&app), Some("exclude"));
}

#[test]
fn second_work_component_uses_exclude_by_default() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 100.0);
    square(&mut app, 30.0, 30.0, 10.0);
    let path = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
    assert_eq!(path.subpaths.len(), 2);
    assert_eq!(path.effective_op(0), PathOp::Combine);
    assert_eq!(path.subpaths[1].op, PathOp::Exclude);
}

#[test]
fn shape_and_path_picks_remain_independent() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    crate::path_components::set_op(&mut app, "subtract").unwrap();
    app.ui.tool_options.vector_mode = "path".into();
    crate::path_components::set_op(&mut app, "intersect").unwrap();
    app.ui.tool_options.vector_mode = "shape".into();
    assert_eq!(tool_path_op(&app), Some("subtract"));
    app.ui.tool_options.vector_mode = "path".into();
    assert_eq!(tool_path_op(&app), Some("intersect"));
}

#[test]
fn shape_default_and_reset_still_make_new_layers() {
    for reset in [false, true] {
        let mut app = app();
        if reset {
            crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Pen);
        }
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), None);
        let before = app.session.active().unwrap().doc.layers.len();
        square(&mut app, 20.0, 20.0, 100.0);
        let first = app.session.active().unwrap().active_layer;
        square(&mut app, 30.0, 30.0, 10.0);
        let state = app.session.active().unwrap();
        assert_ne!(state.active_layer, first);
        assert_eq!(state.doc.layers.len(), before + 2);
    }
}

#[test]
fn legacy_shared_settings_load_and_roundtrip() {
    for op in ["newLayer", "combine", "subtract", "intersect", "exclude"] {
        let options: ToolOptions = serde_json::from_value(json!({"pen_path_op": op})).unwrap();
        assert_eq!(options.pen_path_op, op);
        let loaded: ToolOptions = serde_json::from_value(serde_json::to_value(&options).unwrap()).unwrap();
        assert_eq!(loaded, options);
    }
}

#[test]
fn unmeasured_pen_variants_keep_shared_default_and_reset() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("combine"));
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), None);
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        app.ui.tool_options.vector_mode = "path".into();
        assert_eq!(tool_path_op(&app), Some("subtract"));
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        assert_eq!(tool_path_op(&app), Some("combine"));
    }
}

#[test]
fn menu_checks_the_measured_defaults_and_keeps_each_mode_pick() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                path_operations_menu(app, ui);
            }
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h.get_by_label("Path operations").click();
    h.run_steps(3);
    assert_eq!(h.get_by_label("Exclude Overlapping Shapes").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert!(h.get_by_label("New Layer").accesskit_node().is_disabled());
    if let Ok(dir) = std::env::var("PHOTOCRAFT_PEN_DEFAULT_SNAPSHOTS") {
        let _guard = photocraft_testkit::gpu_lock();
        h.render().unwrap().save(std::path::Path::new(&dir).join("path-default.png")).unwrap();
    }
    h.get_by_label("Subtract Front Shape").click();
    h.run_steps(3);
    h.state_mut().ui.tool_options.vector_mode = "shape".into();
    h.run_steps(2);
    h.get_by_label("Path operations").click();
    h.run_steps(3);
    assert_eq!(h.get_by_label("New Layer").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    if let Ok(dir) = std::env::var("PHOTOCRAFT_PEN_DEFAULT_SNAPSHOTS") {
        let _guard = photocraft_testkit::gpu_lock();
        h.render().unwrap().save(std::path::Path::new(&dir).join("shape-default.png")).unwrap();
    }
    h.get_by_label("Intersect Shape Areas").click();
    h.run_steps(3);
    h.state_mut().ui.tool_options.vector_mode = "path".into();
    h.run_steps(2);
    h.get_by_label("Path operations").click();
    h.run_steps(3);
    assert_eq!(h.get_by_label("Subtract Front Shape").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
}

fn control(app: &mut PhotocraftApp, params: Value) {
    let (request, _) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, &egui::Context::default(), &request) {
        crate::control::Outcome::Done(reply) => assert_eq!(reply["ok"], true, "{reply}"),
        _ => panic!("ui.set must complete immediately"),
    }
}

#[test]
fn control_picks_use_the_same_mode_slots_and_survive_settings_roundtrip() {
    let mut app = app();
    control(&mut app, json!({"tool":"pen", "penMode":"shape", "penPathOp":"subtract"}));
    control(&mut app, json!({"penMode":"path", "penPathOp":"intersect"}));
    app.ui.tool_options = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
    control(&mut app, json!({"penMode":"shape"}));
    assert_eq!(tool_path_op(&app), Some("subtract"));
    control(&mut app, json!({"penMode":"path"}));
    assert_eq!(tool_path_op(&app), Some("intersect"));
    assert_eq!(app.ui.tool_options.pen_path_op, "intersect", "the legacy shared last pick is retained");
}

#[test]
fn legacy_explicit_picks_keep_their_operation_in_both_pen_modes() {
    let mut app = app();
    for op in ["newLayer", "combine", "subtract", "intersect", "exclude"] {
        app.ui.tool_options = serde_json::from_value(json!({"pen_path_op":op})).unwrap();
        assert_eq!(tool_path_op(&app), Some(if op == "newLayer" { "exclude" } else { op }));
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), if op == "newLayer" { None } else { Some(op) });
    }
}

#[test]
fn variants_keep_the_shared_last_pick_after_pen_mode_picks() {
    let mut app = app();
    control(&mut app, json!({"tool":"pen", "penMode":"shape", "penPathOp":"subtract"}));
    control(&mut app, json!({"penMode":"path", "penPathOp":"intersect"}));
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("intersect"));
    }
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "shape".into();
    assert_eq!(tool_path_op(&app), Some("subtract"));
}

#[test]
fn pen_preview_and_commit_use_captured_mode_after_tool_changes() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 100.0);
    for (x, y) in [(30.0, 30.0), (40.0, 30.0), (40.0, 40.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let preview = draft_shape_path(&app, app.ui.pen.as_ref().unwrap()).unwrap().0;
    assert_eq!(preview["subpaths"][1]["op"], "exclude");
    app.ui.tool = Tool::CurvaturePen;
    assert!(pen_commit(&mut app, false));
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[1].op, PathOp::Exclude);
}

#[test]
fn curvature_and_freeform_default_commits_still_combine() {
    for tool in [Tool::CurvaturePen, Tool::FreeformPen] {
        let mut app = app();
        app.ui.tool = tool;
        for (x, y) in [(20.0, 20.0), (30.0, 30.0)] {
            if tool == Tool::CurvaturePen {
                for (a, b) in [(x, y), (x + 10.0, y), (x + 10.0, y + 10.0)] {
                    crate::vector_ui::curvature_down(&mut app, a, b);
                    crate::vector_ui::curvature_up(&mut app);
                }
                assert!(pen_commit(&mut app, false));
            } else {
                freeform_finish(&mut app, &[[x, y, 1.0], [x + 10.0, y, 1.0], [x + 10.0, y + 10.0, 1.0]], &[]);
            }
        }
        let path = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
        assert_eq!(path.subpaths.len(), 2, "{tool:?}");
        assert_eq!(path.subpaths[1].op, PathOp::Combine, "{tool:?}");
    }
}
