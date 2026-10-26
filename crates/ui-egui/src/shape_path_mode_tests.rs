//! Shape/Path creation shares upstream geometry, never a second path representation.
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use photocraft_doc::vector::{PathOp, Subpath};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 240, "height": 180, "background": "white"})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app
}
fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}
fn draw(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    for event in
        [ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, ToolEvent::Move { x: 120.0, y: 80.0, pressure: 1.0 }, ToolEvent::Up { x: 120.0, y: 80.0 }]
    {
        tool_event(app, event, egui::Modifiers::NONE);
    }
}
fn fields(app: &mut PhotocraftApp, tool: Tool) -> serde_json::Map<String, Value> {
    let id = crate::shape_dialog::open(app, tool, [20.0, 20.0]).unwrap();
    app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

#[test]
fn shape_is_default_and_pen_mode_reset_and_serialization_are_independent() {
    let mut app = app();
    assert_eq!(app.ui.tool_options.shape_mode, "shape");
    app.ui.tool_options.vector_mode = "shape".into();
    app.ui.tool_options.shape_mode = "path".into();
    for tool in [Tool::Rectangle, Tool::EllipseShape] {
        app.ui.tool_options.shape_mode = "path".into();
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        assert_eq!(app.ui.tool_options.shape_mode, "shape");
        assert_eq!(app.ui.tool_options.vector_mode, "shape");
    }
    for tool in [Tool::Pen, Tool::CurvaturePen, Tool::FreeformPen] {
        app.ui.tool_options.shape_mode = "path".into();
        app.ui.tool_options.vector_mode = "shape".into();
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        assert_eq!(app.ui.tool_options.vector_mode, "path");
        assert_eq!(app.ui.tool_options.shape_mode, "path");
    }
    app.ui.tool_options.shape_mode = "path".into();
    let encoded = serde_json::to_value(&app.ui.tool_options).unwrap();
    let decoded: ToolOptions = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.shape_mode, "path");
    let mut legacy = encoded;
    legacy.as_object_mut().unwrap().remove("shape_mode");
    let decoded: ToolOptions = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.shape_mode, "shape");
}

#[test]
fn rectangle_and_ellipse_append_without_layers_or_pixels_and_undo_one_step_each() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    let layers = app.session.active().unwrap().doc.layers.clone();
    let pixels = photocraft_compose::flatten(&app.session.active().unwrap().doc);
    let history = app.session.active().unwrap().history.past_len();
    draw(&mut app, Tool::Rectangle);
    let first = work(&app);
    assert_eq!(first, photocraft_vector::shapes::rect(20.0, 20.0, 100.0, 60.0));
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    draw(&mut app, Tool::EllipseShape);
    let both = work(&app);
    assert_eq!(both.subpaths.len(), 2);
    assert_eq!(both.subpaths[0], first.subpaths[0]);
    assert_eq!(both.subpaths[1], photocraft_vector::shapes::ellipse(20.0, 20.0, 100.0, 60.0).subpaths[0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 2);
    assert_eq!(app.session.active().unwrap().doc.layers, layers);
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc), pixels);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(work(&app), first);
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(work(&app), first);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(work(&app), both);
}

#[test]
fn prior_subpath_handles_operations_and_order_are_preserved() {
    let mut app = app();
    let mut original = photocraft_vector::shapes::ellipse(5.0, 6.0, 80.0, 40.0);
    original.subpaths[0].op = PathOp::Subtract;
    original.subpaths.push(Subpath::polyline(&[(1.0, 2.0), (4.0, 8.0)]));
    app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&original)})).unwrap();
    let original = work(&app);
    let bytes = serde_json::to_vec(&original.subpaths).unwrap();
    app.ui.tool_options.shape_mode = "path".into();
    draw(&mut app, Tool::Rectangle);
    let after = work(&app);
    assert_eq!(after.subpaths.len(), 3);
    assert_eq!(serde_json::to_vec(&after.subpaths[..2]).unwrap(), bytes);
}

#[test]
fn empty_and_missing_work_paths_accept_creation() {
    for empty in [false, true] {
        let mut app = app();
        if empty {
            app.run("path.set", json!({"path":{"subpaths":[]}})).unwrap();
        }
        app.ui.tool_options.shape_mode = "path".into();
        draw(&mut app, Tool::EllipseShape);
        assert_eq!(work(&app).subpaths.len(), 1);
    }
}

#[test]
fn rounded_rectangle_uses_upstream_radii_for_drag_and_independent_dialog_corners() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    app.ui.tool_options.corner_radius = 12.0;
    draw(&mut app, Tool::Rectangle);
    let expected = photocraft_engine::vector_cmds::shape_path(&json!({"kind":"roundedRect","rect":[20,20,100,60],"radii":[12,12,12,12]})).unwrap();
    assert_eq!(work(&app), expected);
    assert!(expected.subpaths[0].knots.len() > 4);
    assert!(expected.subpaths[0].knots.iter().any(|k| k.in_ctrl != k.anchor || k.out_ctrl != k.anchor));
    let mut f = fields(&mut app, Tool::Rectangle);
    f.insert("radii".into(), json!([5, 10, 15, 20]));
    f.insert("width".into(), json!(100));
    f.insert("height".into(), json!(60));
    let geometry = crate::shape_dialog::geometry(&f).unwrap();
    assert_eq!(geometry["radii"], json!([5.0, 10.0, 15.0, 20.0]));
    let expected = photocraft_engine::vector_cmds::shape_path(&geometry).unwrap();
    crate::shape_dialog::confirm(&mut app, &f).unwrap();
    let actual = work(&app);
    assert_eq!(actual.subpaths.len(), 2);
    assert_eq!(actual.subpaths[1], expected.subpaths[0]);
}

#[test]
fn original_shape_layer_creation_and_live_corners_still_work() {
    for tool in [Tool::Rectangle, Tool::EllipseShape] {
        let mut app = app();
        app.ui.tool_options.corner_radius = 9.0;
        let before = app.session.active().unwrap().doc.layers.len();
        draw(&mut app, tool);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers.len(), before + 1);
        assert!(st.doc.work_path.is_none());
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Shape(shape) = &layer.content else {
            assert!(matches!(layer.content, photocraft_doc::LayerContent::Shape(_)));
            return;
        };
        assert!(shape.fill.is_some());
        if tool == Tool::Rectangle {
            assert!(matches!(&shape.live,Some(photocraft_doc::LiveShape::Rect{radii,..}) if *radii==[9.0;4]));
        }
    }
}

#[test]
fn click_dialog_captures_mode_both_directions_and_invalid_input_has_no_history() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    let f = fields(&mut app, Tool::EllipseShape);
    app.ui.tool_options.shape_mode = "shape".into();
    crate::shape_dialog::confirm(&mut app, &f).unwrap();
    assert_eq!(work(&app).subpaths.len(), 1);
    let f = fields(&mut app, Tool::Rectangle);
    app.ui.tool_options.shape_mode = "path".into();
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::shape_dialog::confirm(&mut app, &f).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    for geometry in
        [json!({"kind":"rect","rect":[0,0,0,10]}), json!({"kind":"ellipse","rect":[0,0,-1,10]}), json!({"kind":"roundedRect","rect":[0,0,"bad",10]})]
    {
        assert!(create_shape_in_mode(&mut app, geometry, true).is_err());
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
    let mut invalid = f;
    invalid.insert("width".into(), json!(-1));
    assert!(crate::shape_dialog::confirm(&mut app, &invalid).is_err());
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn path_preview_has_no_fill_or_stroke_and_cancel_does_not_mutate_document() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    for tool in [Tool::Rectangle, Tool::EllipseShape] {
        let p = shape_preview(&app, tool, [20.0, 20.0], [120.0, 80.0], egui::Modifiers::NONE).unwrap();
        assert!(p.fill.is_none() && p.stroke.is_none());
        assert!(!p.path.subpaths.is_empty());
    }
    // A creation dialog that is opened and dismissed without confirmation performs no edit.
    let dialog = crate::shape_dialog::open(&mut app, Tool::Rectangle, [20.0, 20.0]).unwrap();
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [20.0, 20.0], egui::Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn active_shape_or_mask_is_preserved_and_new_work_path_uses_legacy_release_movement() {
    for mask in [false, true] {
        let mut app = app();
        if mask {
            app.run("layer.vectorMask.revealAll", json!({})).unwrap();
        } else {
            draw(&mut app, Tool::Rectangle);
        }
        let layers = app.session.active().unwrap().doc.layers.clone();
        app.ui.tool_options.shape_mode = "path".into();
        draw(&mut app, Tool::EllipseShape);
        assert!(shape_work_path_selected(&app));
        assert!(!app.ui.vector_mask_target);
        assert_eq!(crate::direct_select::active_paths(&app)[0].0, PathRef::Work);
        let before = work(&app);
        let history = app.session.active().unwrap().history.past_len();
        app.ui.tool = Tool::PathSelection;
        for e in [ToolEvent::Down { x: 70.0, y: 20.0, pressure: 1.0 }, ToolEvent::Move { x: 77.0, y: 24.0, pressure: 1.0 }] {
            tool_event(&mut app, e, egui::Modifiers::NONE);
        }
        assert_eq!(work(&app), before);
        tool_event(&mut app, ToolEvent::Up { x: 77.0, y: 24.0 }, egui::Modifiers::NONE);
        assert_eq!(work(&app), before.transform(&Affine::translate(7.0, 4.0)));
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(app.session.active().unwrap().doc.layers, layers);
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer":layer.0})).unwrap();
        assert!(!shape_work_path_selected(&app));
    }
}

#[test]
fn saved_paths_pen_settings_and_native_roundtrip_remain_intact() {
    let mut app = app();
    app.run("path.set", json!({"name":"Saved","path":{"subpaths":[{"knots":[[1,2],[30,40]]}]}})).unwrap();
    let saved = app.session.active().unwrap().doc.paths.clone();
    let pen = app.ui.tool_options.vector_mode.clone();
    app.ui.selected_path = Some("Saved".into());
    app.ui.tool_options.shape_mode = "path".into();
    draw(&mut app, Tool::Rectangle);
    draw(&mut app, Tool::EllipseShape);
    assert_eq!(app.session.active().unwrap().doc.paths, saved);
    assert_eq!(app.ui.tool_options.vector_mode, pen);
    let original = work(&app);
    let st = app.session.active().unwrap();
    let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back.work_path, Some(original));
    assert_eq!(back.paths, saved);
}

#[test]
fn work_path_priority_applies_only_to_presses_on_its_outline() {
    for on_work in [false, true] {
        let mut app = app();
        let a = app.run("shape.create", json!({"rect":[10,10,40,30],"fill":"#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        let b = app.run("shape.create", json!({"rect":[140,100,40,30],"fill":"#0000ff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer":a})).unwrap();
        create_shape_in_mode(&mut app, json!({"kind":"ellipse","rect":[60,40,60,40]}), true).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let original_work = work(&app);
        let history = app.session.active().unwrap().history.past_len();
        app.ui.tool = Tool::PathSelection;
        app.ui.extras.snap = false;
        let start = if on_work { [90.0, 40.0] } else { [150.0, 110.0] };
        for ev in [
            ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 },
            ToolEvent::Move { x: start[0] + 9.0, y: start[1] + 7.0, pressure: 1.0 },
            ToolEvent::Up { x: start[0] + 9.0, y: start[1] + 7.0 },
        ] {
            tool_event(&mut app, ev, egui::Modifiers::NONE);
        }
        let after = app.session.active().unwrap();
        if on_work {
            assert_eq!(work(&app), original_work.transform(&Affine::translate(9.0, 7.0)));
            assert_eq!(after.doc.layers, before.layers);
        } else {
            assert_eq!(work(&app), original_work, "pressing another Shape must not move the selected Work Path");
            assert_eq!(after.active_layer, Some(photocraft_doc::LayerId(b)));
            assert_ne!(after.doc.layer(photocraft_doc::LayerId(b)), before.layer(photocraft_doc::LayerId(b)));
            assert_eq!(after.doc.layer(photocraft_doc::LayerId(a)), before.layer(photocraft_doc::LayerId(a)));
        }
        assert_eq!(after.history.past_len(), history + 1);
    }
}
