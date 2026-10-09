//! #1119: aspect correction is a view transform, not a document edit.
use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.views[0].fit_pending = false;
    app.ui.views[0].center = [60.0, 40.0];
    app
}

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, ppp: f32) -> egui::FullOutput {
    let rect = Rect::from_min_size(pos2(20.0, 30.0), vec2(640.0, 480.0));
    app.last_canvas_rect = rect;
    let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
    let mut output = ctx.run_ui(input, |ui| {
        let v = app.ui.views[0].clone();
        app.ui.views[0] = canvas_view(app, ui, 0, rect, v, true);
    });
    output.textures_delta.clear();
    output
}

#[test]
fn aspect_correction_extent_inverse_rotation_and_persistence() {
    for ppp in [1.0, 2.0] {
        for (preset, ratio) in [("square", 1.0), ("anamorphic2To1", 2.0), ("d1DvNtsc", 0.91), ("custom:1.25:Test", 1.25)] {
            for correction in [false, true] {
                let mut app = app();
                app.ui.view.pixel_aspect = preset.into();
                app.ui.view.pixel_aspect_correction = correction;
                let saved = serde_json::to_value(&app.ui).unwrap();
                app.ui = serde_json::from_value(saved).unwrap();
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, Default::default());
                let aspect = if correction { ratio } else { 1.0 };
                for (rotation, flip) in [(0.0_f32, false), (37.0, false), (90.0, true)] {
                    app.ui.views[0].rotation = rotation;
                    app.ui.view.flip_horizontal = flip;
                    let output = frame(&mut app, &ctx, ppp);
                    let xf = ViewXform::active(&app).unwrap();
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    let sign = if flip { -1.0 } else { 1.0 };
                    let expected = vec2((20.0 * aspect * cos - 10.0 * sin) * sign, 20.0 * aspect * sin + 10.0 * cos);
                    let at = xf.rect.center() + expected;
                    assert!(xf.to_screen(80.0, 50.0).distance(at) < 0.001, "{preset}, correction={correction}, ppp={ppp}, rotation={rotation}");
                    let doc = xf.to_doc(at);
                    assert!((doc[0] - 80.0).abs() < 0.001 && (doc[1] - 50.0).abs() < 0.001);
                    assert_eq!(app.ui.views[0].zoom, 1.0);
                    assert!((app.current_zoom() - 1.0).abs() < 0.001);
                    let id = app.session.active().unwrap().doc.id;
                    let tex = app.canvases[&cache_key(id, None)].texture.as_ref().unwrap().id();
                    let mesh = output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Mesh(m) if m.texture_id == tex => Some(m),
                            _ => None,
                        })
                        .unwrap();
                    let want = vec2(120.0 * aspect * cos.abs() + 80.0 * sin.abs(), 120.0 * aspect * sin.abs() + 80.0 * cos.abs());
                    assert!((mesh.calc_bounds().size() - want).length() < 0.002);
                    let before = xf.to_doc(at);
                    zoom_about(&mut app.ui.views[0], &xf, at, 2.0, false);
                    assert!(ViewXform::active(&app).unwrap().to_screen(before[0] as f32, before[1] as f32).distance(at) < 0.002);
                    app.ui.views[0].zoom = 1.0;
                    app.ui.views[0].center = [60.0, 40.0];
                }
                assert_eq!(app.session.active().unwrap().doc.size.width, 120);
            }
        }
    }
}

#[test]
fn aspect_fit_fill_and_unavailable_overlays() {
    let mut app = app();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    frame(&mut app, &ctx, 2.0);
    crate::menus::invoke(&mut app, &ctx, "view.pixelAspectRatio.anamorphic2To1", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "view.pixelAspectRatioCorrection", json!({"on": true})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    fit_view(&mut app.ui.views[0], &doc, vec2(100.0, 80.0));
    assert!((app.ui.views[0].zoom - 0.25).abs() < 0.001);
    fill_view(&mut app.ui.views[0], &doc, vec2(240.0, 80.0));
    assert!((app.ui.views[0].zoom - 1.0).abs() < 0.001);
    for id in ["view.show.brushPreview", "view.show.artboardGuides"] {
        assert!(!crate::menus::is_enabled(&app, id), "{id}");
        assert!(crate::menus::invoke(&mut app, &ctx, id, json!({"on": false})).is_err(), "{id} must reject automation too");
    }
}

#[test]
#[ignore = "requires a wgpu adapter; run explicitly for aspect visual validation"]
fn aspect_rendered_cpu_gpu_extent_at_both_display_scales() {
    for gpu in [false, true] {
        for ppp in [1.0, 2.0] {
            for (preset, aspect, rotation) in [
                ("square", 1.0_f32, 0.0),
                ("anamorphic2To1", 2.0, 0.0),
                ("d1DvNtsc", 0.91, 0.0),
                ("custom:1.25:Test", 1.25, 0.0),
                ("anamorphic2To1", 2.0, 90.0),
            ] {
                let mut h = egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                    let mut app = app();
                    app.ui.views[0].zoom = 1.0 / ppp;
                    app.run("edit.fill", json!({"color": "#11df47"})).unwrap();
                    app.ui.view.pixel_aspect = preset.into();
                    app.ui.view.pixel_aspect_correction = true;
                    app.ui.views[0].rotation = rotation;
                    if gpu {
                        app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
                    }
                    app
                });
                h.run_steps(5);
                assert_eq!(h.state().perf.gpu, gpu);
                let pixels = h.render().unwrap();
                let canvas = ViewXform::active(h.state()).unwrap().rect;
                let mut min = [u32::MAX; 2];
                let mut max = [0; 2];
                for (x, y, p) in pixels.enumerate_pixels() {
                    if canvas.contains(pos2((x as f32 + 0.5) / ppp, (y as f32 + 0.5) / ppp))
                        && p[0].abs_diff(17) <= 2
                        && p[1].abs_diff(223) <= 2
                        && p[2].abs_diff(71) <= 2
                    {
                        min = [min[0].min(x), min[1].min(y)];
                        max = [max[0].max(x), max[1].max(y)];
                    }
                }
                assert_ne!(min[0], u32::MAX);
                let extent = [max[0] - min[0] + 1, max[1] - min[1] + 1];
                let want = if rotation == 0.0 { [(120.0 * aspect).round() as u32, 80] } else { [80, (120.0 * aspect).round() as u32] };
                assert!(
                    extent[0].abs_diff(want[0]) <= 1 && extent[1].abs_diff(want[1]) <= 1,
                    "gpu={gpu}, ppp={ppp}, {preset}, rotation={rotation}: {extent:?} != {want:?}"
                );
                if let Some(dir) = std::env::var_os("PHOTOCRAFT_ASPECT_SNAPSHOTS") {
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    pixels.save(dir.join(format!("aspect-{}-{ppp}-{aspect}-{rotation}.png", if gpu { "gpu" } else { "cpu" }))).unwrap();
                }
            }
        }
    }
}

#[test]
fn aspect_pointer_paints_native_pixel_and_shape_bounds_match() {
    for ppp in [1.0, 2.0] {
        let mut app = app();
        app.ui.views[0].zoom = 1.0 / ppp;
        app.ui.view.pixel_aspect = "anamorphic2To1".into();
        app.ui.view.pixel_aspect_correction = true;
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#11df47"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 1, "hardness": 1.0}})).unwrap();
        app.ui.tool = Tool::Pencil;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 600.0)).with_pixels_per_point(ppp).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    egui::CentralPanel::default().show(ui, |ui| document_area(app, ui));
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        let at = h.state().last_canvas_rect.center() + vec2(41.0, 10.5) / ppp;
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
        }
        let st = h.state().session.active().unwrap();
        let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert!(surface.rgba(80, 50)[3] > 0.9, "ppp {ppp}: pointer paints original document coordinates");
        let xf = ViewXform::active(h.state()).unwrap();
        let cursor = pencil_cursor_rect(&xf, [80.5, 50.5], 1.0, ppp);
        assert!((cursor.size() * ppp - vec2(2.0, 1.0)).length() < 0.001);
        let app = h.state_mut();
        app.run("shape.create", json!({"kind": "rect", "rect": [20, 10, 60, 40], "fill": "#11df47"})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_show_transform = true;
        let r = transform_controls_rect(app, &xf).unwrap();
        assert!((r.size() * ppp - vec2(120.0, 40.0)).length() < 0.001);
    }
}

#[test]
fn aspect_supported_mesh_and_pins_toggle_the_real_puppet_overlay() {
    let mut app = app();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap();
    app.run("edit.fill", json!({"color": "#11df47"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    crate::puppet_ui::begin(&mut app, &ctx, "edit.puppetWarp").unwrap();
    crate::puppet_ui::pointer(&mut app, ToolEvent::Down { x: 45.0, y: 35.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::puppet_ui::pointer(&mut app, ToolEvent::Up { x: 45.0, y: 35.0 }, egui::Modifiers::NONE);
    let count = |app: &mut PhotocraftApp| {
        let out = frame(app, &ctx, 1.0);
        let mesh_edges =
            out.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if (stroke.width - 0.6).abs() < 0.001)).count();
        let pins = out.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.fill == Color32::from_rgb(255, 214, 10))).count();
        (mesh_edges, pins)
    };
    for (id, on) in [("view.show.mesh", true), ("view.show.editPins", true)] {
        crate::menus::invoke(&mut app, &ctx, id, json!({"on": on})).unwrap();
    }
    let shown = count(&mut app);
    assert!(shown.0 > 0 && shown.1 > 0);
    crate::menus::invoke(&mut app, &ctx, "view.show.mesh", json!({"on": false})).unwrap();
    assert_eq!(count(&mut app), (0, shown.1));
    crate::menus::invoke(&mut app, &ctx, "view.show.editPins", json!({"on": false})).unwrap();
    assert_eq!(count(&mut app), (0, 0));
    crate::menus::invoke(&mut app, &ctx, "view.show.all", json!({})).unwrap();
    assert_eq!(count(&mut app), shown);
    crate::menus::invoke(&mut app, &ctx, "view.extras", json!({"on": false})).unwrap();
    assert_eq!(count(&mut app), (0, 0));
    assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins.len(), 1, "visibility doesn't delete pins");
}

#[test]
fn aspect_invalid_restored_custom_ratios_cannot_poison_geometry() {
    for value in ["custom:NaN:bad", "custom:inf:bad", "custom:0:bad", "custom:-2:bad", "custom:10000000:bad"] {
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(value), 1.0, "{value}");
    }
}

#[test]
fn aspect_scroll_clamp_uses_the_rotated_corrected_extent() {
    let mut app = app();
    app.ui.view.pixel_aspect = "anamorphic2To1".into();
    app.ui.view.pixel_aspect_correction = true;
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    frame(&mut app, &ctx, 1.0);
    let v = &mut app.ui.views[0];
    v.doc_size = [1000, 800];
    v.center = [0.0, 0.0];
    crate::scrollbars::clamp_view(v, vec2(400.0, 300.0));
    assert!((v.center[0] - 100.0).abs() < 0.001 && (v.center[1] - 150.0).abs() < 0.001);
    v.rotation = 90.0;
    v.center = [0.0, 0.0];
    crate::scrollbars::clamp_view(v, vec2(400.0, 300.0));
    assert!((v.center[0] - 75.0).abs() < 0.001 && (v.center[1] - 200.0).abs() < 0.001, "{:?}", v.center);
}

#[test]
fn aspect_unavailable_extras_options_reject_cosmetic_changes() {
    let mut app = app();
    let ctx = egui::Context::default();
    let before = app.ui.view.show;
    for field in ["brush_preview", "artboard_guides"] {
        let mut params = json!({});
        params[field] = json!(false);
        assert!(crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", params).is_err());
        assert_eq!(app.ui.view.show, before);
    }
}

#[test]
fn aspect_grid_and_ruler_positions_use_document_coordinates() {
    for ppp in [1.0, 2.0] {
        let mut app = app();
        app.ui.views[0].zoom = 1.0 / ppp;
        app.ui.view.pixel_aspect = "anamorphic2To1".into();
        app.ui.view.pixel_aspect_correction = true;
        app.ui.extras.rulers = true;
        app.ui.extras.grid = true;
        app.session.prefs.edit(|p| p.units_and_rulers.rulers = photocraft_engine::prefs::Unit::Pixels);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let output = frame(&mut app, &ctx, ppp);
        let center = ViewXform::active(&app).unwrap().rect.center();
        // Default major grid spacing is one inch = 72 native pixels.
        let x = center.x + (72.0 - 60.0) * 2.0 / ppp;
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment { points, .. }
            if (points[0].x - x).abs() < 0.01 && (points[1].x - x).abs() < 0.01
            && ((points[1].y - points[0].y).abs() * ppp - 80.0).abs() < 0.01)),
            "grid at native x=72, ppp={ppp}"
        );
        let label_x = center.x + (100.0 - 60.0) * 2.0 / ppp + 2.0;
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t)
            if t.galley.job.text == "100" && (t.pos.x - label_x).abs() < 1.0)),
            "ruler label 100, ppp={ppp}"
        );
    }
}

#[test]
#[ignore = "requires a wgpu adapter; validates physical pixel grid thresholds"]
fn aspect_grid_rendered_cpu_gpu_threshold_uses_smaller_physical_axis() {
    for gpu in [false, true] {
        for ppp in [1.0, 2.0] {
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = app();
                app.run("edit.fill", json!({"color": "#11df47"})).unwrap();
                app.ui.view.pixel_aspect = "d1DvNtsc".into();
                app.ui.view.pixel_aspect_correction = true;
                if gpu {
                    app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
                }
                app
            });
            for (zoom, shown) in [(5.25, false), (6.0, true)] {
                h.state_mut().ui.views[0].zoom = zoom / ppp;
                h.run_steps(5);
                assert_eq!(h.state().perf.gpu, gpu);
                let pixels = h.render().unwrap();
                let center = ViewXform::active(h.state()).unwrap().rect.center() * ppp;
                let mut grid_pixels = 0;
                for (x, y, p) in pixels.enumerate_pixels() {
                    if (x as f32 - center.x).abs() < 20.0
                        && (y as f32 - center.y).abs() < 20.0
                        && (p[0].abs_diff(17) > 4 || p[1].abs_diff(223) > 4 || p[2].abs_diff(71) > 4)
                    {
                        grid_pixels += 1;
                    }
                }
                assert_eq!(grid_pixels > 0, shown, "gpu={gpu}, ppp={ppp}, zoom={zoom}, grid pixels={grid_pixels}");
            }
        }
    }
}

#[test]
fn aspect_review_red_eye_draws_native_search_ellipse() {
    for aspect in [2.0, 0.91] {
        for rotation in [0.0, 90.0] {
            for ppp in [1.0, 2.0] {
                let mut a = app();
                a.ui.views[0].zoom = 1.0 / ppp;
                a.ui.tool = Tool::RedEye;
                a.ui.view.pixel_aspect = format!("custom:{aspect}");
                a.ui.view.pixel_aspect_correction = true;
                a.ui.views[0].rotation = rotation;
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, Default::default());
                frame(&mut a, &ctx, ppp);
                let rect = a.last_canvas_rect;
                let pointer = rect.center();
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    events: vec![egui::Event::PointerMoved(pointer)],
                    ..Default::default()
                };
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
                let mut out = ctx.run_ui(input, |ui| {
                    let v = a.ui.views[0].clone();
                    a.ui.views[0] = canvas_view(&mut a, ui, 0, rect, v, true);
                });
                out.textures_delta.clear();
                let size = out
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_white_alpha(220)) && p.closed => {
                            Some(s.shape.visual_bounding_rect().size())
                        }
                        egui::Shape::Circle(p) if p.stroke.color == Color32::from_white_alpha(220) => Some(s.shape.visual_bounding_rect().size()),
                        _ => None,
                    })
                    .expect("Red Eye footprint");
                let d = 2.0 * photocraft_algo::redeye::search_radius(a.ui.tool_options.red_eye_pupil_size) / ppp;
                let expected = if rotation == 0.0 { vec2(d * aspect + 1.0, d + 1.0) } else { vec2(d + 1.0, d * aspect + 1.0) };
                assert!((size - expected).length() < 0.1, "{aspect} {rotation} {ppp}: {size:?} vs {expected:?}");
            }
        }
    }
}

#[test]
#[ignore = "requires a wgpu adapter; captures corrected secondary tool footprints"]
fn aspect_review_rendered_secondary_footprints_at_both_display_scales() {
    let dir = std::env::var_os("PHOTOCRAFT_ASPECT_SNAPSHOTS").map(std::path::PathBuf::from).expect("snapshot directory");
    std::fs::create_dir_all(&dir).unwrap();
    for ppp in [1.0, 2.0] {
        for rotation in [0.0, 90.0] {
            for tool in [Tool::Brush, Tool::RedEye] {
                let mut h = egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                    let mut a = app();
                    a.run("edit.fill", json!({"color":"#506080"})).unwrap();
                    a.run("tools.setBrush", json!({"brush":{"size":20.0}})).unwrap();
                    a.run("prefs.set", json!({"path":"cursors.painting","value":"standard"})).unwrap();
                    a.ui.view.pixel_aspect = "anamorphic2To1".into();
                    a.ui.view.pixel_aspect_correction = true;
                    a.ui.views[0].rotation = rotation;
                    a.ui.tool = tool;
                    a.last_stroke_end = Some((a.session.active().unwrap().doc.id, [20.0, 20.0]));
                    a
                });
                h.run_steps(5);
                let at = ViewXform::active(h.state()).unwrap().to_screen(70.0, 45.0);
                if tool == Tool::Brush {
                    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
                }
                h.event(egui::Event::PointerMoved(at));
                h.run_steps(3);
                h.render().unwrap().save(dir.join(format!("aspect-followup-{tool:?}-{ppp}-{rotation}.png"))).unwrap();
            }
        }
    }
}

#[test]
fn aspect_review_paste_visibility_excludes_rotated_bounding_box_corners() {
    let xf = ViewXform {
        rect: Rect::from_min_size(pos2(-100.0, -100.0), vec2(200.0, 200.0)),
        zoom: 1.0,
        aspect: 2.0,
        center: [0.0, 0.0],
        flip: false,
        rotation: 45.0,
    };
    let r = DRect::new(72, -20, 92, 20);
    assert!(xf.rect.intersects(xf.doc_rect(r)), "bounding boxes alone would report visible");
    assert!(!xf.sees_rect(r), "the copied pixels are all beyond the viewport corner");
    assert!(!ViewXform { flip: true, ..xf }.sees_rect(r));
    assert!(xf.sees_rect(DRect::new(60, -20, 80, 20)));
}
