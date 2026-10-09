//! Free Transform must preview through the layer stack, rather than paint pixels over it.
use super::*;
use photocraft_doc::{Layer, LayerId, LayerMask};
use serde_json::json;
use std::sync::Arc;

fn scene(depth: u32, grouped: bool) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
    let lower = LayerId(app.run("layer.new.layer", json!({"name": "lower red"})).unwrap()["layer"].as_u64().unwrap());
    app.session
        .edit("test pixels", |doc, _| {
            doc.layer_mut(lower).unwrap().surface_mut().unwrap().fill_rect(DRect::new(8, 8, 40, 48), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.new.layer", json!({"name": "upper blue"})).unwrap();
    app.session
        .edit("upper pixels", |doc, active| {
            let upper = doc.layer_mut(active.unwrap()).unwrap();
            upper.surface_mut().unwrap().fill_rect(DRect::new(24, 16, 56, 40), &[0.0, 0.0, 1.0, 1.0]);
            // Opaque on the left, partially transparent in the centre, transparent on the right.
            let mut mask = LayerMask::hide_all();
            mask.surface.fill_rect(DRect::new(24, 16, 32, 40), &[1.0]);
            mask.surface.fill_rect(DRect::new(32, 16, 44, 40), &[0.5]);
            upper.mask = Some(mask);
            if grouped {
                let index = doc.layers.iter().position(|l| l.id == lower).unwrap();
                let lower_layer = doc.layers.remove(index);
                let mut group = Layer::group("masked group", vec![lower_layer]);
                group.opacity = 0.75;
                let mut mask = LayerMask::reveal_all();
                mask.surface.fill_rect(DRect::new(0, 0, 64, 12), &[0.0]);
                group.mask = Some(mask);
                doc.layers.insert(index, group);
            }
            Ok(())
        })
        .unwrap();
    app.run("layer.select", json!({"layer": lower.0})).unwrap();
    app.sync_views();
    (app, lower)
}

fn pixels(doc: &Document) -> Vec<u8> {
    photocraft_compose::render(doc, doc.bounds()).to_rgba8().pixels
}

#[test]
fn transform_composition_unchanged_preview_matches_the_document() {
    for depth in [8, 16, 32] {
        for grouped in [false, true] {
            let (mut app, _) = scene(depth, grouped);
            let original = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
            let (shown, key) = display_doc(&mut app, 0);
            assert!(pixels(&shown) == pixels(&original), "depth={depth}, grouped={grouped}: the preview must match the original composite");
            let (cached, cached_key) = display_doc(&mut app, 0);
            assert!(Arc::ptr_eq(&shown, &cached), "unchanged frames reuse their preview");
            assert_eq!(key, cached_key);
            assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
            assert_eq!(history, app.session.active().unwrap().history.past_len());
            crate::transform_tool::cancel(&mut app);
            assert_eq!(pixels(&display_doc(&mut app, 0).0), pixels(&original));
        }
    }
}

#[test]
fn transform_composition_moved_preview_matches_commit_and_preserves_history() {
    for grouped in [false, true] {
        let (mut app, _) = scene(8, grouped);
        let original = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
        let initial_key = display_doc(&mut app, 0).1;
        let t = app.ui.transform.as_mut().unwrap();
        t.quad = t.quad.map(|[x, y]| [x + 8.0, y + 4.0]);
        t.interpolation = "nearest".into();
        let (shown, key) = display_doc(&mut app, 0);
        assert_ne!(initial_key, key, "movement invalidates the canvas cache");
        assert_ne!(pixels(&shown), pixels(&original));
        let (cached, cached_key) = display_doc(&mut app, 0);
        assert!(Arc::ptr_eq(&shown, &cached));
        assert_eq!(key, cached_key);
        assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
        assert_eq!(history, app.session.active().unwrap().history.past_len());
        crate::transform_tool::commit(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(pixels(&shown), pixels(&app.session.active().unwrap().doc));
        assert_eq!(history + 1, app.session.active().unwrap().history.past_len());
        app.session.undo();
        assert_eq!(pixels(&app.session.active().unwrap().doc), pixels(&original));
        app.session.redo();
        assert_eq!(pixels(&app.session.active().unwrap().doc), pixels(&shown));
    }
}

#[test]
fn transform_composition_selection_perspective_and_warp_match_commit() {
    for depth in [8, 16, 32] {
        for kind in ["selection", "perspective", "warp"] {
            let (mut app, lower) = scene(depth, true);
            // An upper adjustment must affect the moving pixels too.
            app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
            app.run("layer.select", json!({"layer": lower.0})).unwrap();
            if kind == "selection" {
                app.run("select.rect", json!({"x": 12, "y": 20, "width": 16, "height": 16})).unwrap();
            }
            let original = app.session.active().unwrap().doc.clone();
            crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
            let t = app.ui.transform.as_mut().unwrap();
            if kind == "warp" {
                t.warp = Some(photocraft_geom::warp::Warp::preset(photocraft_geom::warp::WarpStyle::Arc, 30.0, t.rect));
            } else {
                t.quad[0][0] += 7.5;
                t.quad[1][1] += 3.5;
                t.quad[2][0] -= 2.5;
            }
            let (shown, key) = display_doc(&mut app, 0);
            assert!(pixels(&shown) != pixels(&original), "{kind}, depth {depth}: the preview must show the changed geometry");
            // Changing interpolation must invalidate the preview even with unchanged geometry.
            app.ui.transform.as_mut().unwrap().interpolation = "nearest".into();
            let (shown, next_key) = display_doc(&mut app, 0);
            assert_ne!(key, next_key);
            assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
            crate::transform_tool::commit(&mut app);
            assert!(!app.ui.status_error, "{}", app.ui.status);
            assert!(pixels(&shown) == pixels(&app.session.active().unwrap().doc), "{kind}, depth {depth}: preview must match commit");
        }
    }
}

#[test]
fn transform_composition_integer_and_fractional_moves_match_commit_at_every_depth() {
    for depth in [8, 16, 32] {
        for offset in [[10.0, -3.0], [0.25, 0.5], [-5.0, 8.0]] {
            let (mut app, lower) = scene(depth, true);
            // Non-uniform, translucent pixels catch filtering/rounding differences.
            app.session
                .edit("translucent detail", |doc, _| {
                    let surface = doc.layer_mut(lower).unwrap().surface_mut().unwrap();
                    surface.fill_rect(DRect::new(12, 18, 18, 22), &[0.2, 0.8, 0.3, 0.375]);
                    surface.fill_rect(DRect::new(20, 24, 22, 28), &[0.7, 0.1, 0.9, 0.625]);
                    Ok(())
                })
                .unwrap();
            let original = app.session.active().unwrap().doc.clone();
            crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
            let t = app.ui.transform.as_mut().unwrap();
            t.quad = t.quad.map(|[x, y]| [x + offset[0], y + offset[1]]);
            let shown = display_doc(&mut app, 0).0;
            assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
            crate::transform_tool::commit(&mut app);
            assert!(!app.ui.status_error, "{}", app.ui.status);
            assert!(pixels(&shown) == pixels(&app.session.active().unwrap().doc), "depth {depth}, offset {offset:?}: preview must match filtered commit");
        }
    }
}

#[test]
fn transform_composition_pointer_frame_shows_current_move_scale_and_rotation() {
    for (from, to) in [([20.0, 28.0], [27.0, 31.0]), ([40.0, 48.0], [48.0, 56.0]), ([50.0, 50.0], [52.0, 35.0])] {
        let (mut app, _) = scene(8, false);
        app.ui.tool = Tool::Move;
        app.ui.extras.rulers = false;
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(512.0, 512.0));
        let xf = ViewXform { rect, zoom: 4.0, center: [32.0, 32.0], flip: false, rotation: 0.0 };
        let frame = |app: &mut PhotocraftApp, events| {
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let view = View { zoom: xf.zoom, center: xf.center, doc_size: [64, 64], fit_pending: false, ..Default::default() };
                    canvas_view(app, ui, 0, rect, view, true);
                });
            });
            output.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        let start = xf.to_screen(from[0], from[1]);
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![egui::Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
        let initial_quad = app.ui.transform.as_ref().unwrap().quad;
        frame(&mut app, vec![egui::Event::PointerMoved(xf.to_screen(to[0], to[1]))]);
        assert_ne!(app.ui.transform.as_ref().unwrap().quad, initial_quad, "the pointer must change the transform");
        let (shown, key) = display_doc(&mut app, 0);
        let (_, display_key) = canvas_display(&app, &shown, None);
        let cache = app.canvases.get(&cache_key(shown.id, None)).unwrap();
        assert_eq!(cache.tex_preview_key, key ^ display_key, "{from:?} → {to:?}: canvas must show the pointer's current frame");
    }
}

#[test]
fn transform_composition_gpu_pointer_frame_is_current() {
    for (from, to) in [([20.0, 28.0], [27.0, 31.0]), ([40.0, 48.0], [48.0, 56.0]), ([50.0, 50.0], [52.0, 35.0])] {
        let built = std::panic::catch_unwind(|| {
            egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 800.0)).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let (mut app, _) = scene(8, false);
                app.set_wgpu(cc.wgpu_render_state.as_ref().unwrap().clone());
                app.ui.tool = Tool::Move;
                app.ui.extras.rulers = false;
                app.ui.views[0] = View { zoom: 4.0, center: [32.0, 32.0], doc_size: [64, 64], fit_pending: false, ..Default::default() };
                crate::transform_tool::begin(&mut app, &cc.egui_ctx).unwrap();
                app
            })
        });
        let Ok(mut h) = built else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        h.run_steps(4);
        let screen = |h: &egui_kittest::Harness<'_, PhotocraftApp>, p: [f32; 2]| {
            let view = &h.state().ui.views[0];
            ViewXform { rect: h.state().last_canvas_rect, zoom: view.zoom, center: view.center, flip: false, rotation: view.rotation }.to_screen(p[0], p[1])
        };
        let start = screen(&h, from);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        h.event(egui::Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        let before = h.state().ui.transform.as_ref().unwrap().quad;
        h.event(egui::Event::PointerMoved(screen(&h, to)));
        h.step();
        assert_ne!(h.state().ui.transform.as_ref().unwrap().quad, before);
        let (shown, key) = display_doc(h.state_mut(), 0);
        let (display, _) = canvas_display(h.state(), &shown, None);
        let cache = h.state().canvases.get(&(shown.id, GPU_OUTPUT)).unwrap();
        assert!(cache.on_gpu);
        assert_eq!(cache.preview_key, key ^ texture_key(display.as_deref()), "GPU pixels must use the current pointer frame");
        h.render().unwrap();
    }
}

/// Record changed-geometry cost separately from the cached frames. Run in release mode.
#[test]
#[ignore]
fn transform_composition_bench() {
    use std::time::Instant;
    for (width, height) in [(800, 450), (6000, 4000)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": width, "height": height})).unwrap();
        app.run("layer.new.layer", json!({"name": "benchmark lower"})).unwrap();
        app.session
            .edit("benchmark pixels", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(DRect::new(0, 0, width, height), &[0.8, 0.2, 0.1, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let before = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(app.transform_preview.as_ref().unwrap().doc.clone());
        }
        eprintln!("{width}x{height} legacy document lookup: {:.3} ms/frame (mesh remains separate)", before.elapsed().as_secs_f64() * 10.0);
        let start = Instant::now();
        display_doc(&mut app, 0);
        eprintln!("{width}x{height} initial composite preview: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
        for kind in ["translation", "rotation", "scaling"] {
            let t = app.ui.transform.as_mut().unwrap();
            let [cx, cy] = [f64::from(width) / 2.0, f64::from(height) / 2.0];
            if kind == "translation" {
                t.quad = t.quad.map(|[x, y]| [x + 10.0, y + 10.0]);
            } else if kind == "rotation" {
                t.quad = t.quad.map(|[x, y]| {
                    let (s, c) = 0.05_f64.sin_cos();
                    [cx + (x - cx) * c - (y - cy) * s, cy + (x - cx) * s + (y - cy) * c]
                });
            } else {
                t.quad = t.quad.map(|[x, y]| [cx + (x - cx) * 1.03, cy + (y - cy) * 1.03]);
            }
            let start = Instant::now();
            let shown = display_doc(&mut app, 0).0;
            eprintln!("{width}x{height} {kind} preview rebuild: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
            let cached = Instant::now();
            for _ in 0..100 {
                assert!(Arc::ptr_eq(&shown, &display_doc(&mut app, 0).0));
            }
            eprintln!("{width}x{height} {kind} cached preview: {:.3} ms/frame", cached.elapsed().as_secs_f64() * 10.0);
        }
    }
}
