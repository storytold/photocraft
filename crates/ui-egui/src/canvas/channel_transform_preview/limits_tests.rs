use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn retained_channel_source_preflight_tracks_memory_usage_changes() {
    const CHILD: &str = "PHOTOCRAFT_CHANNEL_SOURCE_BUDGET_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Memory Usage is process-wide; keep this test independent of parallel UI sessions.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "canvas::channel_transform_preview::limits_tests::retained_channel_source_preflight_tracks_memory_usage_changes", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        return;
    }
    use photocraft_algo::transform::Homography;
    use photocraft_engine::{channel_transform::AffinePreviewBounds, transform_cmds, warp_cmds};
    use photocraft_geom::warp::{BezierMesh, Warp};
    let mut doc = Document::with_background("source budget", Size::new(12, 8), ColorMode::Rgb, SampleType::F32, Color::WHITE);
    doc.size = Size::new(2048, 2048);
    let layer = Some(doc.layers[0].id);
    let doc = Arc::new(doc);
    let params = json!({"target":{"colors":[0,2]}});
    let rect = [0.0, 0.0, 2048.0, 2048.0];
    let warp = Warp::custom(BezierMesh::identity(rect, 1, 1).map_points(|p| [p[0] / 256.0, p[1] / 256.0]), rect);
    let h = Homography([1.0 / 256.0, 0.0, 0.0, 0.0, 1.0 / 256.0, 0.0, 0.0, 0.0, 1.0]);
    transform_cmds::set_budget(64 << 20);
    let mut affine = AffinePreviewBounds::new(doc.clone(), layer, &params, [1.0; 4]).unwrap();
    affine.check(&h).unwrap();
    warp_cmds::check_channel_warp_geometry(&doc, layer, &params, rect, &warp).unwrap();
    transform_cmds::set_budget(16 << 20);
    assert!(affine.check(&h).unwrap_err().to_string().contains("Memory Usage"));
    assert!(warp_cmds::check_channel_warp_geometry(&doc, layer, &params, rect, &warp).unwrap_err().to_string().contains("Memory Usage"));
    let single = json!({"target":{"color":0}});
    warp_cmds::check_channel_warp_geometry(&doc, layer, &single, rect, &warp).unwrap();
    transform_cmds::set_budget(64 << 20);
    affine.check(&h).unwrap();
    warp_cmds::check_channel_warp_geometry(&doc, layer, &params, rect, &warp).unwrap();
    let mut large = (*doc).clone();
    large.size = Size::new(6000, 6000);
    transform_cmds::set_budget(512 << 20);
    photocraft_engine::channel_transform::source_bounds(&large, layer, &json!({"target":{"colors":[0,1,2]}})).unwrap();
}

#[test]
fn reduced_gpu_preview_reports_native_reach_before_any_commit_and_recovers() {
    let _lock = photocraft_testkit::gpu_lock();
    let Ok(rs) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())))
    else {
        return;
    };
    if !crate::channel_transform_gpu::test_supported(&rs) {
        return;
    }
    for colors in [vec![1], vec![0, 2]] {
        let mut a = super::tests::app();
        let mut doc = Document::with_background("native preview limits", Size::new(6000, 1000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let bounds = doc.bounds();
        doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[0.2, 0.4, 0.6, 1.0]);
        a.session.add_document(doc, None);
        a.set_wgpu(rs.clone());
        a.run("channel.target", json!({"colors":colors})).unwrap();
        let before = a.session.active().unwrap().doc.clone();
        let index = a.session.active_index().unwrap();
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut a, &ctx).unwrap();
        let layer = a.session.active().unwrap().active_layer;
        a.transform_preview.as_mut().unwrap().channel = Some(Preview::new(&before, layer, a.session.tools.background, true).unwrap());
        a.ui.transform.as_mut().unwrap().quad = a.ui.transform.as_ref().unwrap().quad.map(|[x, y]| [x + 16.0, y]);
        let good = a.ui.transform.as_ref().unwrap().quad;
        assert!(a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().factor > 1);
        let gpu = crate::channel_transform_gpu::test_wait(&rs, || texture(&mut a, &ctx, index, None)).unwrap();
        assert!(a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().gpu.is_some());
        let old = a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().shown.clone();
        let mut expected = Preview::new(&before, layer, a.session.tools.background, true).unwrap();
        expected.update(a.ui.transform.as_ref().unwrap()).unwrap();
        let offset = f64::from(photocraft_algo::transform::WARP_REACH) + 64.0;
        a.ui.transform.as_mut().unwrap().quad = good.map(|[x, y]| [x + offset, y]);
        let cpu = texture(&mut a, &ctx, index, None).unwrap();
        assert_ne!(cpu, gpu, "a first refused geometry never returns the GPU texture");
        assert!(a.ui.status_error && a.ui.status.contains("reach"), "{}", a.ui.status);
        let shown = &a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().shown;
        assert!(!Arc::ptr_eq(&old, shown), "the last valid GPU geometry must be materialized");
        assert_eq!(shown.layers[0].surface(), expected.shown.layers[0].surface());
        let message = a.ui.status.clone();
        a.run("channel.setVisible", json!({"channel":"composite","visible":false})).unwrap();
        assert_eq!(texture(&mut a, &ctx, index, None), Some(cpu));
        assert_eq!(a.ui.status, message);
        assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
        a.ui.transform.as_mut().unwrap().quad = good;
        assert_eq!(crate::channel_transform_gpu::test_wait(&rs, || texture(&mut a, &ctx, index, None)), Some(gpu));
        assert!(!a.ui.status_error, "{}", a.ui.status);
        crate::transform_tool::cancel(&mut a);
        assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
        assert_eq!(a.session.active().unwrap().channel_view.visible_colors(3), 0);
        ctx.tex_manager().write().take_delta().clear();
    }
}

#[test]
fn reduced_mask_saved_channel_and_quick_mask_refuse_native_reach() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer, LayerMask};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    for target in [json!("mask"), json!({"channel":0}), json!("quickMask")] {
        let mut doc = Document::new("scalar native limits", Size::new(3000, 2000), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(Layer::raster("pixels", doc.pixel_format()));
        let mut samples = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        samples.fill_rect(Rect::new(400, 400, 800, 800), &[0.5]);
        if target == "mask" {
            doc.layers[0].mask = Some(LayerMask { surface: samples, ..LayerMask::reveal_all() });
        } else if target == "quickMask" {
            doc.quick_mask = Some(AlphaChannel::new("Quick Mask", samples));
        } else {
            doc.channels.push(AlphaChannel::new("saved", samples));
        }
        let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        a.session.add_document(doc, None);
        if target == "mask" {
            a.run("channel.target", json!({"channel":"mask"})).unwrap();
            a.run("view.layerMask", json!({"mode":"overlay"})).unwrap();
        } else if target != "quickMask" {
            a.run("channel.target", json!({"channel":0})).unwrap();
        }
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut a, &ctx).unwrap();
        let before = a.session.active().unwrap().doc.clone();
        let layer = a.session.active().unwrap().active_layer;
        a.transform_preview.as_mut().unwrap().channel = Some(Preview::new(&before, layer, a.session.tools.background, true).unwrap());
        let mut t = a.ui.transform.as_ref().unwrap().clone();
        assert_eq!(t.target, Some(target));
        let preview = a.transform_preview.as_mut().unwrap().channel.as_mut().unwrap();
        assert!(preview.factor > 1);
        assert!(preview.check_native(&t).is_ok());
        let good = t.quad;
        t.quad = good.map(|[x, y]| [x + f64::from(photocraft_algo::transform::WARP_REACH), y]);
        assert!(preview.check_native(&t).unwrap_err().contains("reach"));
        t.quad = good;
        assert!(preview.check_native(&t).is_ok());
        assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
    }
}

#[test]
fn sparse_selected_scalar_channel_transform_uses_a_bounded_native_display() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer, LayerMask};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for target in [json!("mask"), json!({"channel":0}), json!("quickMask")] {
            let mut doc = Document::new("sparse selected preview", Size::new(3000, 2000), ColorMode::Rgb, depth);
            doc.layers.push(Layer::raster("pixels", doc.pixel_format()));
            let mut samples = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
            samples.write_pixel(8, 8, &[1.0]);
            samples.write_pixel(100_000_000, 6000, &[0.5]);
            if target == "mask" {
                doc.layers[0].mask = Some(LayerMask { surface: samples, ..LayerMask::reveal_all() });
            } else if target == "quickMask" {
                doc.quick_mask = Some(AlphaChannel::new("Quick Mask", samples));
            } else {
                doc.channels.push(AlphaChannel::new("saved", samples));
            }
            let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            selection.fill_rect(Rect::new(8, 8, 24, 24), &[1.0]);
            doc.selection = Some(selection);
            // Assert the safe path before capture: a dense proxy here would try enormous rows.
            assert_eq!(preview_scales(&doc, true).unwrap(), (1, 2));
            let mut hostile = doc.clone();
            hostile.size = Size::new(u32::MAX, 2);
            assert!(preview_scales(&hostile, true).is_err());
            hostile.size = Size::new(300_000, 256);
            assert!(preview_scales(&hostile, true).is_err(), "the renderer's minimum tile-height band must remain bounded");
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.session.add_document(doc, None);
            if target == "mask" {
                app.run("channel.target", json!({"channel":"mask"})).unwrap();
                app.run("view.layerMask", json!({"mode":"overlay"})).unwrap();
            } else if target != "quickMask" {
                app.run("channel.target", json!({"channel":0})).unwrap();
            }
            let before = app.session.active().unwrap().doc.clone();
            let ctx = egui::Context::default();
            crate::transform_tool::begin(&mut app, &ctx).unwrap();
            let t = app.ui.transform.as_mut().unwrap();
            t.quad = t.quad.map(|[x, y]| [x + 2.0, y]);
            let index = app.session.active_index().unwrap();
            texture(&mut app, &ctx, index, None).unwrap();
            crate::transform_tool::enter_warp(&mut app, &ctx);
            texture(&mut app, &ctx, index, None).unwrap();
            app.run("channel.setVisible", json!({"channel":"green","visible":false})).unwrap();
            app.run("channel.setVisible", json!({"channel":"blue","visible":false})).unwrap();
            app.run("channel.setVisible", json!({"channel":"red","visible":true})).unwrap();
            texture(&mut app, &ctx, index, None).unwrap();
            let eyes = app.session.active().unwrap().channel_view.clone();
            let p = app.transform_preview.as_ref().unwrap().channel.as_ref().unwrap();
            assert_eq!((p.factor, p.display_factor), (1, 2));
            let image = p.image(
                &app.session.active().unwrap().channel_view,
                crate::channel_view::Look::of(&app.session),
                None,
                &crate::channel_view::Screen::default(),
            );
            assert_eq!(image.size, [1500, 1000]);
            let scalar = if target == "mask" {
                &p.shown.layers[0].mask.as_ref().unwrap().surface
            } else if target == "quickMask" {
                &p.shown.quick_mask.as_ref().unwrap().surface
            } else {
                &p.shown.channels[0].surface
            };
            assert_eq!(scalar.sample_channel(10, 8, 0), 1.0);
            assert!((scalar.sample_channel(100_000_000, 6000, 0) - 0.5).abs() < 0.005);
            assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
            crate::transform_tool::cancel(&mut app);
            assert_eq!(app.session.active().unwrap().channel_view, eyes);
            ctx.tex_manager().write().take_delta().clear();
        }
    }
}
