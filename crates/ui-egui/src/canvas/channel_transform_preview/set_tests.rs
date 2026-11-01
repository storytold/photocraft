use super::*;
use photocraft_geom::warp::{BezierMesh, Warp};

#[test]
fn colour_set_box_locks_the_set_and_keeps_all_eyes_through_commit_cancel_and_undo() {
    for warp in [false, true] {
        for commit in [false, true] {
            let mut a = super::tests::app();
            a.run("select.rect", json!({"x":4,"y":4,"width":12,"height":12})).unwrap();
            a.run("select.modify.feather", json!({"radius":1})).unwrap();
            a.run("channel.target", json!({"colors":[0,2]})).unwrap();
            a.run("channel.setVisible", json!({"channel":"green","visible":true})).unwrap();
            let before = a.session.active().unwrap().doc.clone();
            let eyes = a.session.active().unwrap().channel_view.clone();
            let history = a.session.active().unwrap().history.past_len();
            let ctx = egui::Context::default();
            crate::transform_tool::begin(&mut a, &ctx).unwrap();
            assert_eq!(a.session.active().unwrap().channel_view, eyes);
            let t = a.ui.transform.as_mut().unwrap();
            assert_eq!(t.target, Some(json!({"colors":[0,2]})));
            t.quad = t.quad.map(|[x, y]| [x + 3.0, y]);
            if warp {
                crate::transform_tool::enter_warp(&mut a, &ctx);
                let t = a.ui.transform.as_mut().unwrap();
                assert!(t.warp.is_some());
                t.warp = Some(Warp::custom(BezierMesh::identity(t.rect, 1, 1).map_points(|p| [p[0] + 3.0, p[1]]), t.rect));
            }
            let t = a.ui.transform.clone().unwrap();
            let pv = a.transform_preview.as_mut().unwrap().channel.as_mut().unwrap();
            pv.update(&t).unwrap();
            let shown = pv.shown.clone();
            for (cmd, p) in [
                ("channel.target", json!({"colors":[1,2]})),
                ("channel.target", json!({"channel":"red"})),
                ("layer.select", json!({"layer":t.layer})),
                ("layer.new.layer", json!({})),
            ] {
                assert!(a.run(cmd, p).is_err(), "{cmd}");
            }
            for channel in ["red", "green", "blue", "composite"] {
                a.run("channel.setVisible", json!({"channel":channel,"visible":false})).unwrap();
                assert_eq!(a.ui.transform, Some(t.clone()));
                let pv = a.transform_preview.as_mut().unwrap().channel.as_mut().unwrap();
                pv.update(&t).unwrap();
                assert!(Arc::ptr_eq(&pv.shown, &shown));
                a.run("channel.setVisible", json!({"channel":channel,"visible":true})).unwrap();
            }
            a.run("channel.setVisible", json!({"channel":"red","visible":false})).unwrap();
            a.run("channel.setVisible", json!({"channel":"blue","visible":false})).unwrap();
            let final_eyes = a.session.active().unwrap().channel_view.clone();
            assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
            if commit {
                crate::transform_tool::commit(&mut a);
            } else {
                crate::transform_tool::cancel(&mut a);
            }
            assert!(!a.ui.status_error, "{}", a.ui.status);
            let st = a.session.active().unwrap();
            assert!(st.channel_transform.is_none());
            assert_eq!(st.history.past_len(), history + usize::from(commit));
            assert_eq!(st.channel_view, final_eyes);
            if commit {
                assert_eq!(st.doc.layers[0].surface(), shown.layers[0].surface());
                a.session.undo();
                assert_eq!(a.session.active().unwrap().doc.layers[0].surface(), before.layers[0].surface());
                assert_eq!(a.session.active().unwrap().channel_view, final_eyes);
                a.session.redo();
                assert_eq!(a.session.active().unwrap().doc.layers[0].surface(), shown.layers[0].surface());
            } else {
                assert!(Arc::ptr_eq(&before, &st.doc));
            }
        }
    }
}

#[test]
fn colour_set_refuses_start_if_any_edited_component_is_hidden() {
    for hidden in ["red", "blue"] {
        let mut a = super::tests::app();
        a.run("channel.target", json!({"colors":[0,2]})).unwrap();
        a.run("channel.setVisible", json!({"channel":hidden,"visible":false})).unwrap();
        let before = a.session.active().unwrap().channel_view.clone();
        let error = crate::transform_tool::begin(&mut a, &egui::Context::default()).unwrap_err();
        assert!(error.contains(photocraft_engine::channel_transform::HIDDEN));
        assert!(a.ui.transform.is_none());
        assert_eq!(a.session.active().unwrap().channel_view, before);
        assert!(!photocraft_engine::channel_transform::locked(&a.session));
    }
}

#[test]
fn differently_sparse_colour_sets_use_scalar_outlines_or_the_cpu_oracle() {
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    use photocraft_geom::Rect;
    let _lock = photocraft_testkit::gpu_lock();
    let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
    let gpu = crate::gpu_canvas::GpuCanvas::new(&rs);
    let mut doc = Document::with_background("different scalar outlines", Size::new(32, 24), ColorMode::Rgb, SampleType::F32, Color::WHITE);
    doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(2, 2, 8, 8), &[0.2, 1.0, 1.0, 1.0]);
    let layer = Some(doc.layers[0].id);
    let mut a = super::tests::app();
    a.session.add_document(doc.clone(), None);
    a.run("channel.target", json!({"colors":[0,2]})).unwrap();
    crate::transform_tool::begin(&mut a, &egui::Context::default()).unwrap();
    let view = a.session.active().unwrap().channel_view.clone();
    let mut t = a.ui.transform.clone().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 2.0, y]);
    let mut fast = crate::channel_transform_gpu::Preview::new(&gpu, &doc, layer, [1.0; 4]).unwrap();
    assert!(fast.texture(&t, &view, false, false, 1).is_none(), "different affine edge-repeat bounds retain the scalar oracle");
    t.warp = Some(Warp::custom(BezierMesh::identity(t.rect, 1, 1).map_points(|p| [p[0] + 2.0, p[1]]), t.rect));
    assert!(fast.texture(&t, &view, false, false, 1).is_some(), "a full-frame Warp has common coverage");
    let mut cpu = Preview::new(&doc, layer, [1.0; 4], false);
    cpu.update(&t).unwrap();
    let expected = cpu.image(&view, crate::channel_view::Look::default(), None);
    let actual = fast.readback();
    assert!(actual.iter().zip(&expected.pixels).all(|(a, b)| a.to_array().into_iter().zip(b.to_array()).all(|(a, b)| a.abs_diff(b) <= 2)));
    assert!(gpu.health().is_ok());
}
