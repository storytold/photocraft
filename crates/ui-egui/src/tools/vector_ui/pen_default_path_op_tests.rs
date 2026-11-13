//! #12905 shared Pen-family defaults, per-mode picks and legacy/control compatibility.
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
    square(&mut app, 20.0, 20.0, 100.0);
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
    square(&mut app, 20.0, 20.0, 100.0);
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
fn variants_share_exclude_default_and_reset() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("exclude"));
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), None);
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        app.ui.tool_options.vector_mode = "path".into();
        assert_eq!(tool_path_op(&app), Some("exclude"));
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        assert_eq!(tool_path_op(&app), Some("exclude"));
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
    app.ui.tool_options.vector_mode = "shape".into();
    square(&mut app, 20.0, 20.0, 100.0);
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
    app.ui.tool_options.vector_mode = "shape".into();
    square(&mut app, 20.0, 20.0, 100.0);
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
    app.ui.tool_options.vector_mode = "shape".into();
    square(&mut app, 20.0, 20.0, 100.0);
    control(&mut app, json!({"tool":"pen", "penMode":"shape", "penPathOp":"subtract"}));
    control(&mut app, json!({"penMode":"path", "penPathOp":"intersect"}));
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("intersect"));
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), Some("subtract"), "selected-Shape picks stay shared: {tool:?}");
        app.ui.tool_options.vector_mode = "path".into();
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
fn curvature_and_freeform_default_components_use_exclude() {
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
        assert_eq!(path.subpaths[1].op, PathOp::Exclude, "{tool:?}");
    }
}

// Revised shared-tool requirements: these probes must fail on the Pen-only implementation.
#[test]
fn revised_path_picks_are_immediately_shared_in_every_direction() {
    let mut app = app();
    for (tool, op) in [(Tool::Pen, "subtract"), (Tool::FreeformPen, "intersect"), (Tool::CurvaturePen, "combine")] {
        app.ui.tool = tool;
        crate::path_components::set_op(&mut app, op).unwrap();
        for other in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
            crate::tool_memory::select(&mut app, other);
            assert_eq!(tool_path_op(&app), Some(op), "{tool:?} -> {other:?}");
        }
    }
}

#[test]
fn revised_control_picks_are_shared_and_keep_shape_default_independent() {
    let mut app = app();
    for tool in ["pen", "freeformPen", "curvaturePen"] {
        control(&mut app, json!({"tool":tool, "penMode":"path", "penPathOp":"combine"}));
        for other in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
            app.ui.tool = other;
            assert_eq!(tool_path_op(&app), Some("combine"), "{tool} -> {other:?}");
            app.ui.tool_options.vector_mode = "shape".into();
            assert_eq!(tool_path_op(&app), None, "Shape mode must stay New Layer: {other:?}");
            app.ui.tool_options.vector_mode = "path".into();
        }
    }
}

#[test]
fn revised_reset_on_each_pen_restores_shared_exclude() {
    for reset_tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        crate::tool_reset::reset_options(&mut app.ui.tool_options, reset_tool);
        for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
            app.ui.tool = tool;
            assert_eq!(tool_path_op(&app), Some("exclude"), "reset {reset_tool:?}, view {tool:?}");
        }
    }
}

#[test]
fn revised_shape_subtract_without_a_shape_does_not_stick_or_change_path() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        crate::path_components::set_op(&mut app, "combine").unwrap();
        app.ui.tool_options.vector_mode = "shape".into();
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        assert_eq!(tool_path_op(&app), None, "{tool:?}: reopen Shape menu as New Layer");
        app.ui.tool_options.vector_mode = "path".into();
        assert_eq!(tool_path_op(&app), Some("combine"), "{tool:?}: Path pick survives");
    }
}

#[test]
fn mode_selection_is_shared_by_all_three_pen_tools() {
    let mut app = app();
    for (tool, mode) in [("pen", "shape"), ("freeformPen", "path"), ("curvaturePen", "shape")] {
        control(&mut app, json!({"tool":tool,"penMode":mode}));
        for other in ["pen", "freeformPen", "curvaturePen"] {
            control(&mut app, json!({"tool":other}));
            assert_eq!(app.ui.tool_options.vector_mode, mode, "{tool} -> {other}");
        }
    }
}

fn menu_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                path_operations_menu(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

#[test]
fn native_menu_picks_are_shared_after_pen_has_an_explicit_pick() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = menu_harness(app());
    for (tool, pick, label) in [
        (Tool::Pen, "Subtract Front Shape", "Exclude Overlapping Shapes"),
        (Tool::FreeformPen, "Intersect Shape Areas", "Subtract Front Shape"),
        (Tool::CurvaturePen, "Combine Shapes", "Intersect Shape Areas"),
    ] {
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        h.get_by_label("Path operations").click();
        h.run_steps(3);
        assert_eq!(h.get_by_label(label).accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
        h.get_by_label(pick).click();
        h.run_steps(3);
    }
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        h.get_by_label("Path operations").click();
        h.run_steps(3);
        assert_eq!(h.get_by_label("Combine Shapes").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "{tool:?}");
        h.get_by_label("Combine Shapes").click();
        h.run_steps(3);
    }
}

#[test]
fn shape_menu_reopens_with_new_layer_after_subtract_without_a_shape() {
    use egui_kittest::kittest::{NodeT, Queryable};
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        crate::path_components::set_op(&mut app, "combine").unwrap();
        app.ui.tool_options.vector_mode = "shape".into();
        let mut h = menu_harness(app);
        h.get_by_label("Path operations").click();
        h.run_steps(3);
        h.get_by_label("Subtract Front Shape").click();
        h.run_steps(3);
        h.get_by_label("Path operations").click();
        h.run_steps(3);
        assert_eq!(h.get_by_label("New Layer").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "{tool:?}");
        h.get_by_label("New Layer").click();
        h.run_steps(3);
        h.state_mut().ui.tool_options.vector_mode = "path".into();
        assert_eq!(tool_path_op(h.state()), Some("combine"));
    }
}

fn draw_component(app: &mut PhotocraftApp, x: f64, y: f64) {
    match app.ui.tool {
        Tool::CurvaturePen => {
            for (a, b) in [(x, y), (x + 10.0, y), (x + 10.0, y + 10.0)] {
                crate::vector_ui::curvature_down(app, a, b);
                crate::vector_ui::curvature_up(app);
            }
            assert!(pen_commit(app, true));
        }
        Tool::FreeformPen => freeform_finish(app, &[[x, y, 1.0], [x + 10.0, y, 1.0], [x + 10.0, y + 10.0, 1.0], [x, y, 1.0]], &[]),
        _ => square(app, x, y, 10.0),
    }
}

#[test]
fn default_and_reset_shape_components_create_new_layers_for_all_three_tools() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        app.ui.tool_options.vector_mode = "shape".into();
        crate::path_components::set_op(&mut app, "subtract").unwrap(); // no shape: must not survive
        app.ui.tool_options = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
        let n = app.session.active().unwrap().doc.layers.len();
        draw_component(&mut app, 20.0, 20.0);
        let first = app.session.active().unwrap().active_layer;
        assert_eq!(tool_path_op(&app), None, "ignored pick must not reappear once a shape exists: {tool:?}");
        draw_component(&mut app, 40.0, 40.0);
        let st = app.session.active().unwrap();
        assert_ne!(first, st.active_layer, "{tool:?}");
        assert_eq!(st.doc.layers.len(), n + 2, "{tool:?}");
    }
}

#[test]
fn selected_shape_subtract_and_history_remain_available_for_all_three_tools() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        app.ui.tool_options.vector_mode = "shape".into();
        draw_component(&mut app, 20.0, 20.0);
        let id = app.session.active().unwrap().active_layer.unwrap();
        let n = app.session.active().unwrap().doc.layers.len();
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        app.ui.tool_options.vector_mode = "path".into();
        crate::path_components::set_op(&mut app, "intersect").unwrap();
        app.ui.tool_options.vector_mode = "shape".into();
        assert_eq!(tool_path_op(&app), Some("subtract"));
        draw_component(&mut app, 40.0, 40.0);
        let path = crate::vector_ui::active_shape_path(&app).unwrap().1;
        assert_eq!(path.subpaths.len(), 2, "{tool:?}");
        assert_eq!(path.subpaths[1].op, PathOp::Subtract);
        assert_eq!(app.session.active().unwrap().active_layer, Some(id));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), n);
        app.session.undo();
        assert_eq!(crate::vector_ui::active_shape_path(&app).unwrap().1.subpaths.len(), 1);
        app.session.redo();
        assert_eq!(crate::vector_ui::active_shape_path(&app).unwrap().1, path);
    }
}

#[test]
fn legacy_path_picks_load_for_all_three_while_shape_without_a_layer_shows_new_layer() {
    let mut app = app();
    for op in ["newLayer", "combine", "subtract", "intersect", "exclude"] {
        for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
            app.ui.tool = tool;
            app.ui.tool_options = serde_json::from_value(json!({"pen_path_op":op})).unwrap();
            assert_eq!(tool_path_op(&app), Some(if op == "newLayer" { "exclude" } else { op }));
            app.ui.tool_options.vector_mode = "shape".into();
            assert_eq!(tool_path_op(&app), None);
        }
    }
}

#[test]
fn control_updates_pen_slots_even_when_another_tool_is_active() {
    let mut app = app();
    control(&mut app, json!({"tool":"pen","penMode":"path","penPathOp":"subtract"}));
    control(&mut app, json!({"tool":"rectangle","penPathOp":"intersect"}));
    assert_eq!(app.ui.tool_options.shape_path_op, "newLayer", "Pen setter must not change Rectangle operations");
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("intersect"));
    }
}
