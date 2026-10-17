use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::LiveShape;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":200,"height":160,"background":"transparent"})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app
}
fn draw(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::vector_ui::finish_shape(app, tool, [20.0, 20.0], [100.0, 80.0], Default::default());
    assert!(!app.ui.status_error, "{}", app.ui.status);
}
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    harness_size(app, egui::vec2(1200.0, 600.0))
}
fn harness_size(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::i18n::with_language(crate::i18n::lang_from_tag("en").unwrap(), || {
                    ui.horizontal(|ui| crate::vector_ui::options_bar(app, ui, app.ui.tool));
                });
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    h
}

#[test]
fn narrow_toolbar_can_scroll_to_and_open_path_options() {
    let mut a = app();
    draw(&mut a, Tool::Polygon);
    let before = a.session.active().unwrap().doc.clone();
    let mut h = harness_size(a, egui::vec2(400.0, 100.0));
    h.hover_at(egui::pos2(200.0, 20.0));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-2000.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(12);
    let gear = h.get_by_label("Path Options");
    assert!(gear.rect().left() >= 0.0 && gear.rect().right() <= 400.0);
    gear.click();
    h.run_steps(3);
    h.get_by_label("Show Path").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.shape.show_path);
    assert_eq!(h.state().session.active().unwrap().doc, before);
}

#[test]
fn every_shape_tool_draws_each_mode_with_one_undo_step() {
    for mode in ["shape", "path", "pixels"] {
        for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape] {
            let mut a = app();
            a.ui.tool_options.shape.mode = mode.into();
            let before = a.session.active().unwrap().doc.clone();
            let history = a.session.active().unwrap().history.past_len();
            draw(&mut a, tool);
            let drawn = a.session.active().unwrap().doc.clone();
            assert_ne!(drawn, before, "{mode} {tool:?}");
            assert_eq!(a.session.active().unwrap().history.past_len(), history + 1, "{mode} {tool:?}");
            match mode {
                "shape" => assert_eq!(drawn.layers.len(), before.layers.len() + 1),
                "path" => {
                    assert_eq!(drawn.layers.len(), before.layers.len());
                    assert!(drawn.work_path.is_some());
                }
                _ => {
                    assert_eq!(drawn.layers.len(), before.layers.len());
                    assert!(drawn.work_path.is_none());
                }
            }
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(a.session.active().unwrap().doc, before);
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(a.session.active().unwrap().doc, drawn);
        }
    }
}

#[test]
fn drawing_uses_the_active_shapes_displayed_fill_and_complete_stroke() {
    let mut a = app();
    a.run("shape.create",json!({"rect":[4,4,20,20],"fill":{"gradient":{"stops":[[0,"#aa2233"],[1,"#2266cc"]],"angle":45}},"stroke":{"color":"#118833","width":7,"dashes":[2,1],"cap":"round","opacity":40}})).unwrap();
    let st = a.session.active().unwrap();
    let LayerContent::Shape(original) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    let original = original.clone();
    let colors = (a.session.tools.foreground, a.session.tools.background);
    draw(&mut a, Tool::EllipseShape);
    let st = a.session.active().unwrap();
    let LayerContent::Shape(next) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    assert_eq!(next.fill, original.fill);
    assert_eq!(next.stroke, original.stroke);
    assert_eq!((a.session.tools.foreground, a.session.tools.background), colors);
}

#[test]
fn path_conversion_buttons_make_selection_mask_and_shape() {
    let mut a = app();
    a.ui.tool_options.shape.mode = "path".into();
    draw(&mut a, Tool::Rectangle);
    let work = a.session.active().unwrap().doc.work_path.clone().unwrap();
    let pixel = a.session.active().unwrap().active_layer.unwrap();
    let mut h = harness(a);
    h.get_by_label("Selection").click();
    h.run_steps(3);
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    h.get_by_label("Mask").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layer(pixel).unwrap().vector_mask.as_ref().unwrap().path, work);
    h.get_by_label("Shape").click();
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    assert_eq!(sh.path, work);
    assert_eq!(h.state().ui.tool_options.shape.mode, "shape");
}

#[test]
fn path_shape_options_edit_live_geometry_but_never_a_different_document() {
    let mut a = app();
    a.ui.tool_options.shape.mode = "path".into();
    a.ui.tool_options.corner_radius = 4.0;
    draw(&mut a, Tool::Rectangle);
    assert!(matches!(path_shape(&a).unwrap().live, Some(LiveShape::Rect { radii: [4.0, 4.0, 4.0, 4.0], .. })));
    edit_path(&mut a, json!({"radii":9,"rect":[20,20,120,70]})).unwrap();
    assert!(matches!(path_shape(&a).unwrap().live, Some(LiveShape::Rect { radii: [9.0, 9.0, 9.0, 9.0], rect: [20.0, 20.0, 120.0, 70.0] })));
    let path = a.session.active().unwrap().doc.work_path.clone().unwrap();
    a.run("file.new", json!({"width":200,"height":160})).unwrap();
    a.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    assert!(path_shape(&a).unwrap().live.is_none());
    assert!(edit_path(&mut a, json!({"radii":2})).is_err());
}

#[test]
fn path_options_change_view_only() {
    let mut a = app();
    draw(&mut a, Tool::Line);
    let before = a.session.active().unwrap().doc.clone();
    let mut h = harness(a);
    h.get_by_label("Path Options").click();
    h.run_steps(3);
    h.get_by_label("Show Path").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.shape.show_path);
    assert_eq!(h.state().session.active().unwrap().doc, before);
}

#[test]
fn control_shape_options_validate_before_applying_any_field() {
    let mut a = app();
    let ctx = egui::Context::default();
    let call = |a: &mut PhotocraftApp, p| {
        let (request, _) = crate::control::ControlRequest::new("ui.set", p);
        crate::control::handle(a, &ctx, &request)
    };
    call(&mut a, json!({"shapeOptions":{"mode":"pixels","opacity":25,"pathWidth":3}}));
    assert_eq!(a.ui.tool_options.shape.mode, "pixels");
    assert_eq!(a.ui.tool_options.shape.opacity, 25.0);
    let before = a.ui.clone();
    for p in [
        json!({"tool":"polygon","shapeOptions":{"mode":"bad"}}),
        json!({"tool":"polygon","shapeOptions":{"opacity":-1}}),
        json!({"tool":"polygon","shapeOptions":{"pathWidth":1e100}}),
        json!({"tool":"polygon","shapeOptions":{"unknown":true}}),
        json!({"tool":"polygon","shapeOptions":{"sizeMode":"bad"}}),
        json!({"tool":"polygon","shapeOptions":{"sizeWidth":0}}),
        json!({"tool":"polygon","shapeOptions":{"sizeHeight":300001}}),
    ] {
        call(&mut a, p);
        assert_eq!(a.ui, before);
    }
}

#[test]
fn size_modes_and_pixel_alignment_share_preview_and_commit_geometry() {
    let mut a = app();
    a.ui.tool_options.shape.align_edges = false;
    let rect = |a: &PhotocraftApp, mods| crate::vector_ui::shape_geometry(a, Tool::Rectangle, [10.25, 20.25], [80.75, 45.75], mods).unwrap()["rect"].clone();
    assert_eq!(rect(&a, egui::Modifiers::NONE), json!([10.25, 20.25, 70.5, 25.5]));
    a.ui.tool_options.shape.align_edges = true;
    assert_eq!(rect(&a, egui::Modifiers::NONE), json!([10.0, 20.0, 71.0, 26.0]));
    a.ui.tool_options.shape.align_edges = false;
    a.ui.tool_options.shape.size_mode = "square".into();
    assert_eq!(rect(&a, egui::Modifiers::NONE), json!([10.25, 20.25, 70.5, 70.5]));
    a.ui.tool_options.shape.size_mode = "fixed".into();
    a.ui.tool_options.shape.size_width = 40.0;
    a.ui.tool_options.shape.size_height = 20.0;
    assert_eq!(rect(&a, egui::Modifiers::NONE), json!([10.25, 20.25, 40.0, 20.0]));
    assert_eq!(rect(&a, egui::Modifiers::ALT), json!([-9.75, 10.25, 40.0, 20.0]));
    a.ui.tool_options.shape.size_mode = "proportional".into();
    assert_eq!(rect(&a, egui::Modifiers::NONE), json!([10.25, 20.25, 70.5, 35.25]));
    let expected = rect(&a, egui::Modifiers::NONE);
    crate::vector_ui::finish_shape(&mut a, Tool::Rectangle, [10.25, 20.25], [80.75, 45.75], egui::Modifiers::NONE);
    let st = a.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    let Some(LiveShape::Rect { rect, .. }) = shape.live else { panic!() };
    assert_eq!(json!(rect), expected);
    // A horizontal drag with Shift still produces a square instead of a zero-height shape.
    assert!(crate::vector_ui::shape_geometry(&a, Tool::Rectangle, [10.0, 10.0], [50.0, 10.0], egui::Modifiers::SHIFT).is_some());
}

#[test]
fn align_edges_is_view_state_and_old_preferences_load_defaults() {
    let mut a = app();
    draw(&mut a, Tool::Rectangle);
    let before = a.session.active().unwrap().doc.clone();
    let mut h = harness(a);
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.shape.align_edges);
    assert_eq!(h.state().session.active().unwrap().doc, before);
    let old: ShapeToolOptions = serde_json::from_value(json!({"mode":"path","fromCenter":true})).unwrap();
    assert_eq!(old.size_mode, "unconstrained");
    assert!(old.align_edges);
}

#[test]
fn full_paint_defaults_are_atomic_view_state_and_seed_new_shapes() {
    let mut a = app();
    let ctx = egui::Context::default();
    let before = a.session.active().unwrap().doc.clone();
    let colors = (a.session.tools.foreground, a.session.tools.background);
    let call = |a: &mut PhotocraftApp, p| {
        let (request, _) = crate::control::ControlRequest::new("ui.set", p);
        let crate::control::Outcome::Done(response) = crate::control::handle(a, &ctx, &request) else {
            panic!("ui.set must return synchronously");
        };
        response
    };
    let response = call(
        &mut a,
        json!({"shapeFillPaint":{"gradient":{"stops":[[0,"#ff0000"],[1,"#0000ff"]],"opacityStops":[[0,0.25],[1,0.5]],"midpoints":[0.3],"offset":[0.1,-0.2],"align":false,"dither":true}},"shapeStrokePaint":[0.2,0.3,0.4,0.25]}),
    );
    assert_eq!(response["ok"], true, "full paint defaults are accepted: {response}");
    assert!(
        matches!(&a.ui.tool_options.shape_fill_paint,Some(photocraft_doc::Fill::Gradient { opacity_stops, midpoints, align:false, dither:true, ..}) if opacity_stops.len()==2 && midpoints==&[0.3])
    );
    assert_eq!(a.session.active().unwrap().doc, before);
    assert_eq!((a.session.tools.foreground, a.session.tools.background), colors);
    let options = a.ui.tool_options.clone();
    for invalid in
        [json!({"tool":"polygon","shapeFillPaint":{"gradient":{"stops":[[0,"bad"]]}}}), json!({"tool":"polygon","shapeStrokePaint":{"type":"unknown"}})]
    {
        let response = call(&mut a, invalid);
        assert_eq!(response["ok"], false, "invalid paints are rejected: {response}");
        assert_eq!(a.ui.tool_options, options);
        assert_eq!(a.ui.tool, Tool::Rectangle);
    }
    draw(&mut a, Tool::Rectangle);
    let st = a.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    assert_eq!(shape.fill, options.shape_fill_paint);
    assert_eq!(shape.stroke.as_ref().unwrap().paint, options.shape_stroke_paint.unwrap());
}
