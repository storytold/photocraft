//! Channel-edit regressions retained across the shared UI migration.
use super::*;

#[test]
fn colour_set_mask_view_is_preserved_by_transform_copy_and_cancel() {
    for selected in [false, true] {
        for linked in [false, true] {
            for mode in ["gray", "overlay"] {
                for landed in [false, true] {
                    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
                    app.run("layer.layerMask.hideAll", json!({})).unwrap();
                    app.run("layer.layerMask.linked", json!({"linked": linked})).unwrap();
                    let source = app.session.active().unwrap().active_layer.unwrap();
                    app.session
                        .edit("mask patch", |doc, _| {
                            doc.layer_mut(source).unwrap().mask.as_mut().unwrap().surface.fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0]);
                            Ok(())
                        })
                        .unwrap();
                    if selected {
                        app.run("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16, "feather": 1})).unwrap();
                    }
                    app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
                    app.run("view.layerMask", json!({"mode": mode})).unwrap();
                    let before = app.session.active().unwrap().doc.clone();
                    let view = app.session.active().unwrap().channel_view.clone();
                    let history = app.session.active().unwrap().history.past_len();
                    begin_copy(&mut app, &egui::Context::default()).unwrap();
                    assert_eq!(app.ui.transform.as_ref().unwrap().target, Some(json!("mask")), "{selected}/{linked}/{mode}: capture the original mask target");
                    let copy = app.session.active().unwrap().active_layer.unwrap();
                    assert_ne!(copy, source);
                    assert_eq!(
                        app.session.active().unwrap().doc.layer(copy).unwrap().surface(),
                        before.layer(source).unwrap().surface(),
                        "copying a mask must not crop its host pixels"
                    );
                    let mut expected_view = view.clone();
                    expected_view.layer_mask.as_mut().unwrap().layer = copy;
                    assert_eq!(app.session.active().unwrap().channel_view, expected_view, "start preserves channel eyes and the viewed mask");
                    app.run("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
                    expected_view = app.session.active().unwrap().channel_view.clone();
                    crate::channel_transform_preview::edit(&mut app, |t| t.quad = t.quad.map(|[x, y]| [x + 4.0, y + 0.0]));
                    if landed {
                        commit(&mut app);
                        let st = app.session.active().unwrap();
                        assert!(st.doc.layer(source) == before.layer(source));
                        assert_eq!(st.doc.layer(copy).unwrap().surface(), before.layer(source).unwrap().surface());
                        assert_ne!(st.doc.layer(copy).unwrap().mask.as_ref().unwrap().surface, before.layer(source).unwrap().mask.as_ref().unwrap().surface);
                        assert_eq!(st.history.past_len(), history + 1);
                        assert_eq!(st.channel_view, expected_view);
                        assert!(app.session.undo());
                        assert!(app.session.active().unwrap().doc.layers == before.layers);
                    } else {
                        cancel(&mut app);
                        expected_view.layer_mask.as_mut().unwrap().layer = source;
                        let st = app.session.active().unwrap();
                        assert!(Arc::ptr_eq(&st.doc, &before));
                        assert_eq!(st.history.past_len(), history);
                        assert_eq!(st.channel_view, expected_view, "cancel restores the source mask while retaining current eyes");
                    }
                }
            }
        }
    }
}

#[test]
fn a_large_placed_transform_prepares_without_capturing_the_selected_channels() {
    for cancel_preparation in [false, true] {
        let mut app = app_with_square(4096, photocraft_geom::Rect::new(0, 0, 4096, 4096));
        app.session.execute("layer.duplicate", json!({"inPlace":true})).unwrap();
        app.sync_views();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        let st = app.session.active().unwrap();
        let (placed, eyes) = (st.active_layer.unwrap(), st.channel_view.clone());
        app.background_jobs = true;
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput { max_texture_side: Some(1024), ..Default::default() });
        begin_placed(&mut app, &ctx).unwrap();
        let job = app.transform_preparing.as_ref().expect("the large layer prepares on a worker").job;
        assert!(app.ui.transform.is_none());
        assert!(begin(&mut app, &ctx).unwrap_err().contains("still preparing"));
        assert_eq!(app.transform_preparing.as_ref().unwrap().job, job);
        assert_eq!(app.session.active().unwrap().channel_view, eyes);
        assert!(app.session.active().unwrap().channel_transform.is_none());
        if cancel_preparation {
            assert!(app.session.cancel_job(job));
        }
        land_jobs(&mut app);
        assert!(app.transform_preparing.is_none());
        if cancel_preparation {
            assert!(app.ui.transform.is_none());
        } else {
            let t = app.ui.transform.as_ref().expect("the completed job opens the box");
            assert_eq!(t.layer, placed.0);
            assert!(t.target.is_none(), "the place transforms its whole layer");
            assert_eq!(app.ui.transform.as_ref().unwrap().made, Some(MadeLayer::Place));
            assert_eq!(app.session.active().unwrap().channel_view, eyes);
            cancel(&mut app);
        }
        assert!(app.session.active().unwrap().doc.layer(placed).is_none(), "cancel removes the placed layer");
        assert!(app.session.active().unwrap().channel_transform.is_none());
        ctx.end_pass().textures_delta.clear();
    }
}

#[test]
fn channel_transform_warp_is_enabled_in_both_menus_without_retargeting_or_committing() {
    for target in [json!("green"), json!("mask"), json!(0), json!("quickMask")] {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        let expected = if target == "mask" {
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("channel.target", json!({"channel":"mask"})).unwrap();
            app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
            json!("mask")
        } else if target == 0 {
            app.run("channel.new", json!({})).unwrap();
            app.run("channel.target", json!({"channel":0})).unwrap();
            json!({"channel":0})
        } else if target == "quickMask" {
            app.run("select.editInQuickMaskMode", json!({"on":true})).unwrap();
            json!("quickMask")
        } else {
            app.run("channel.target", json!({"channel":"green"})).unwrap();
            json!({"color":1})
        };
        app.run("select.rect", json!({"x":8,"y":8,"width":16,"height":16})).unwrap();
        let ctx = egui::Context::default();
        begin(&mut app, &ctx).unwrap_or_else(|e| panic!("{target}: {e}"));
        let t = crate::channel_transform_preview::edit(&mut app, |t| {
            t.quad = t.quad.map(|[x, y]| [x + 3.0, y + 2.0]);
        });
        let before = t.clone();
        let st = app.session.active().unwrap();
        let (source, eyes, history) = (st.doc.clone(), st.channel_view.clone(), st.history.past_len());
        assert_eq!(before.target, Some(expected.clone()));
        assert!(crate::menus::is_enabled(&app, "edit.transform.warp"), "main menu: {target}");
        let warp_row_enabled = |app: &PhotocraftApp| {
            app.ui
                .transform
                .as_ref()
                .unwrap()
                .session
                .menu(&TransformMenuWords::english())
                .iter()
                .find(|entry| matches!(entry.value(), Some(TransformChoice::Warp)))
                .unwrap()
                .is_enabled()
        };
        assert!(warp_row_enabled(&app), "shared transform context menu: {target}");
        crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
        let t = crate::channel_transform_preview::Geometry::of(app.ui.transform.as_ref().unwrap());
        assert!(t.warp.is_some(), "{target}");
        assert_eq!((t.session, t.rect, t.quad, t.target.as_ref()), (before.session, before.rect, before.quad, before.target.as_ref()));
        let [x, y] = before.quad[0];
        let mapped = t.warp.as_ref().unwrap().map(before.rect[0], before.rect[1]);
        assert!((mapped.0 - x).abs() < 1e-6 && (mapped.1 - y).abs() < 1e-6, "the prior drag carries into Warp");
        let st = app.session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&source, &st.doc));
        assert_eq!(st.channel_view, eyes);
        assert_eq!(st.history.past_len(), history);
        assert_eq!(st.channel_transform.as_ref().unwrap().target, expected);
        assert!(!crate::menus::is_enabled(&app, "edit.transform.warp"));
        let menu = app.ui.transform.as_ref().unwrap().session.menu(&TransformMenuWords::english());
        assert!(
            menu.iter().find(|entry| matches!(entry.value(), Some(TransformChoice::Warp))).unwrap().is_checked(),
            "the shared context menu marks its current mode"
        );
        cancel(&mut app);
    }
}

#[test]
fn selected_scalar_channel_transform_opens_over_distant_sparse_mask_samples() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    for target in [json!("mask"), json!({"channel":0}), json!("quickMask")] {
        let mut app = app_with_square(64, Rect::new(8, 8, 24, 24));
        if target == "mask" {
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("channel.target", json!({"channel":"mask"})).unwrap();
            app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        } else if target == "quickMask" {
            app.run("select.editInQuickMaskMode", json!({"on":true})).unwrap();
        } else {
            app.run("channel.new", json!({})).unwrap();
            app.run("channel.target", json!({"channel":0})).unwrap();
        }
        app.session
            .edit("distant scalar samples", |doc, layer| {
                let surface = if target == "mask" {
                    &mut doc.layer_mut(layer.unwrap()).unwrap().mask.as_mut().unwrap().surface
                } else if target == "quickMask" {
                    &mut doc.quick_mask.as_mut().unwrap().surface
                } else {
                    &mut doc.channels[0].surface
                };
                *surface = Surface::new(surface.format());
                surface.write_pixel(8, 8, &[1.0]);
                surface.write_pixel(20_000, 6_000, &[0.5]);
                let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                selection.fill_rect(Rect::new(8, 8, 24, 24), &[1.0]);
                doc.selection = Some(selection);
                Ok(())
            })
            .unwrap();
        let st = app.session.active().unwrap();
        let (before, eyes) = (st.doc.clone(), st.channel_view.clone());
        let ctx = egui::Context::default();
        begin(&mut app, &ctx).unwrap_or_else(|e| panic!("{target}: {e}"));
        let transform = crate::channel_transform_preview::Geometry::of(app.ui.transform.as_ref().unwrap());
        assert_eq!(transform.rect, [8.0, 8.0, 24.0, 24.0]);
        assert_eq!(transform.target, Some(target));
        assert!(photocraft_engine::channel_transform::locked(&app.session));
        enter_warp(&mut app, &ctx);
        assert!(app.ui.transform.as_ref().unwrap().session.warp().is_some());
        cancel(&mut app);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().channel_view, eyes);
        assert!(!photocraft_engine::channel_transform::locked(&app.session));
    }
}

#[test]
fn free_transform_a_copy_of_a_colour_on_a_linked_layer_stays_isolated() {
    for selected in [false, true] {
        for landed in [false, true] {
            let (mut app, source, linked, _) = linked_layers_app();
            app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
            app.run("channel.target", json!({"channel": "red"})).unwrap();
            if selected {
                app.run("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16})).unwrap();
            }
            let before = app.session.active().unwrap().doc.clone();
            begin_copy(&mut app, &egui::Context::default()).unwrap();
            let copy = app.session.active().unwrap().active_layer.unwrap();
            assert_ne!(copy, source, "a colour target makes a copy even on a linked host");
            assert_eq!(app.session.active().unwrap().doc.layers.len(), before.layers.len() + 1);
            let t = crate::channel_transform_preview::Geometry::of(app.ui.transform.as_ref().unwrap());
            assert_eq!(t.target, Some(json!({"color": 0})));
            assert!(app.ui.transform.as_ref().unwrap().layers.is_empty(), "the linked group must not become the edit target");
            let copied = app.session.active().unwrap().doc.layer(copy).unwrap().surface().unwrap().clone();
            crate::channel_transform_preview::edit(&mut app, |t| t.quad = t.quad.map(|[x, y]| [x + 4.0, y + 0.0]));
            if landed {
                commit(&mut app);
                assert!(!app.ui.status_error, "{}", app.ui.status);
                let doc = &app.session.active().unwrap().doc;
                assert_eq!(doc.layer(source), before.layer(source));
                assert_eq!(doc.layer(linked), before.layer(linked));
                let result = doc.layer(copy).unwrap().surface().unwrap();
                for y in 0..64 {
                    for x in 0..64 {
                        for component in [1, 2, 3] {
                            assert_eq!(result.sample_channel(x, y, component), copied.sample_channel(x, y, component));
                        }
                    }
                }
                assert_ne!(result, &copied);
                app.session.undo();
            } else {
                cancel(&mut app);
            }
            assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        }
    }
}

#[test]
fn free_transform_a_copy_from_a_linked_filter_mask_view_boxes_the_link_group() {
    for mode in ["gray", "overlay"] {
        for colors in [vec![0], vec![0, 2]] {
            let (mut app, _, linked, _) = linked_layers_app();
            app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            let smart = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("filter mask", |doc, _| {
                    let LayerContent::Smart(content) = &mut doc.layer_mut(smart).unwrap().content else { panic!("not a smart object") };
                    content.filter_mask = Some(photocraft_doc::LayerMask::reveal_all());
                    Ok(())
                })
                .unwrap();
            app.run("layer.select", json!({"layer": linked.0, "mode": "add"})).unwrap();
            app.run("layer.linkLayers", json!({"linked": true})).unwrap();
            app.run("layer.select", json!({"layer": smart.0})).unwrap();
            app.run("channel.target", json!({"colors": colors})).unwrap();
            app.run("view.layerMask", json!({"mask": "filter", "mode": mode})).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            begin_copy(&mut app, &egui::Context::default()).unwrap();
            assert_eq!(app.session.active().unwrap().doc.layers.len(), before.layers.len(), "a filter-mask view still transforms the whole object");
            crate::channel_transform_preview::Geometry::of(app.ui.transform.as_ref().unwrap());
            assert_eq!(app.ui.transform.as_ref().unwrap().made, None);
            assert_eq!(app.ui.transform.as_ref().unwrap().layers.len(), 2);
            assert!(app.ui.transform.as_ref().unwrap().layers.contains(&smart.0) && app.ui.transform.as_ref().unwrap().layers.contains(&linked.0));
            assert!(app.transform_preview.as_ref().unwrap().channel.is_none());
            cancel(&mut app);
            assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        }
    }
}

#[test]
fn linked_mask_target_transforms_alone() {
    let mut app = masked_app(true);
    app.ui.mask_target = true;
    app.run("view.layerMask", json!({"mode":"overlay"})).unwrap();
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().target, Some(json!("mask")));
    assert_eq!(app.ui.transform.as_ref().unwrap().rect_array(), [0.0, 0.0, 16.0, 64.0]);
}

#[test]
fn free_transform_a_copy_of_a_path_keeps_priority_over_a_hidden_colour() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [30, 10], [30, 20], [10, 20]]}]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.run("channel.target", json!({"channel": "red"})).unwrap();
    app.run("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let eyes = app.session.active().unwrap().channel_view.clone();
    begin_copy(&mut app, &egui::Context::default()).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().path, Some(json!({"name": "work"})));
    assert_eq!(app.session.active().unwrap().channel_view, eyes);
    cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
    assert_eq!(app.session.active().unwrap().channel_view, eyes);
}

#[test]
fn selected_layers_keep_a_channel_transform_isolated_and_warp_available() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let first = st.active_layer.unwrap();
    let mut other = photocraft_doc::Layer::raster("other", photocraft_color::PixelFormat::RGBA8);
    other.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(32, 8, 48, 24), &[0., 1., 0., 1.]);
    let second = other.id;
    Arc::make_mut(st.doc_mut()).layers.push(other);
    *st.selection_mut() = vec![first, second];
    app.run("channel.target", json!({"channel":"green"})).unwrap();
    app.run("channel.setVisible", json!({"channel":"composite","visible":true})).unwrap();
    let before = app.session.active().unwrap().doc.layer(second).unwrap().surface().unwrap().to_interleaved(photocraft_geom::Rect::new(0, 0, 64, 64));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert!(app.ui.transform.as_ref().unwrap().layers.is_empty());
    assert_eq!(app.ui.transform.as_ref().unwrap().target, Some(json!({"color":1})));
    begin_warp(&mut app, &ctx).unwrap();
    assert!(app.ui.transform.as_ref().unwrap().session.warp().is_some());
    app.run("channel.setVisible", json!({"channel":"green","visible":false})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.is_some());
    cancel(&mut app);
    assert!(!app.session.active().unwrap().channel_view.color_visible(1));
    assert_eq!(app.session.active().unwrap().doc.layer(second).unwrap().surface().unwrap().to_interleaved(photocraft_geom::Rect::new(0, 0, 64, 64)), before);
}

#[test]
fn channel_jobs_restore_the_guard_but_cancelled_jobs_close_the_box() {
    use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
    for cancelled in [false, true] {
        let mut a = app_with_square(64, photocraft_geom::Rect::new(4, 4, 28, 28));
        a.run("channel.target", json!({"colors":[0,2]})).unwrap();
        crate::transform_tool::begin(&mut a, &egui::Context::default()).unwrap();
        let t = crate::channel_transform_preview::edit(&mut a, |t| {
            t.quad = t.quad.map(|[x, y]| [x + 2.0, y]);
        });
        let geometry = t.clone();
        let before = a.session.active().unwrap().doc.clone();
        a.run("channel.setVisible", json!({"channel":"blue","visible":false})).unwrap();
        let eyes = a.session.active().unwrap().channel_view.clone();
        a.session.active_mut().unwrap().channel_transform = None;
        a.transform_preview.as_mut().unwrap().committing = Some(JobId(123));
        let event = JobEvent {
            id: JobId(123),
            command: "edit.transform.warp".into(),
            label: "Warp".into(),
            document: Some(before.id),
            outcome: if cancelled { JobOutcome::Cancelled } else { JobOutcome::Failed("synthetic refusal".into()) },
            error: (!cancelled).then(|| photocraft_engine::EngineError::Other("synthetic refusal".into())),
        };
        crate::transform_tool::on_job_event(&mut a, &event);
        assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
        assert_eq!(a.session.active().unwrap().channel_view, eyes);
        assert_eq!(a.ui.transform.is_none(), cancelled);
        assert_eq!(photocraft_engine::channel_transform::locked(&a.session), !cancelled);
        if !cancelled {
            assert_eq!(a.ui.transform.as_ref().map(crate::channel_transform_preview::Geometry::of), Some(geometry));
            assert!(a.run("channel.target", json!({"channel":"green"})).is_err());
            crate::transform_tool::cancel(&mut a);
            assert!(!photocraft_engine::channel_transform::locked(&a.session));
        }
    }
}
