use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};

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
