use photocraft_color::PixelFormat;
use photocraft_doc::{
    LayerId, LayerMask,
    mask_paint::{EffectKind, Paint, Stack},
};
use photocraft_raster::Surface;
use serde_json::json;

use super::{cancel, cpu_document, display_doc, gpu_display_doc, set, set_fill, value};
use crate::{PhotocraftApp, Services};

fn app() -> (PhotocraftApp, LayerId) {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
    let id = session.active().unwrap().active_layer.unwrap();
    let state = session.active_mut().unwrap();
    let base = Surface::with_default(PixelFormat::GRAY8, &[0.2]);
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base,
        layers: vec![Paint {
            name: "Opacity".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        }],
        active: 0,
    });
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(id).unwrap().mask = Some(mask);
    (PhotocraftApp::new(session, Services::default()), id)
}

#[test]
fn opacity_preview_is_gpu_document_transient_then_cpu_materializes_when_needed() {
    let (mut app, layer) = app();
    let (revision, history) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());

    set(&mut app, layer, 0, 0.25);
    let (shown, key) = display_doc(&mut app, 0).expect("transient shown document");
    assert_eq!(shown.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.25);
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 1.0);
    assert_eq!((app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len()), (revision, history));
    let (again, same_key) = display_doc(&mut app, 0).unwrap();
    assert!(std::sync::Arc::ptr_eq(&shown, &again) && key == same_key);
    assert_eq!(value(&app, layer, 0, 0.0), 0.25);

    let cpu = cpu_document(&mut app, shown, key);
    let mask = cpu.layer(layer).unwrap().mask.as_ref().unwrap();
    let base = Surface::with_default(PixelFormat::GRAY8, &[0.2]).default_pixel()[0];
    let source = Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]).default_pixel();
    let alpha = source[3] * 0.25;
    let expected = Surface::with_default(PixelFormat::GRAY8, &[source[0] * alpha + base * (1.0 - alpha)]).default_pixel();
    assert_eq!(mask.surface.default_pixel(), expected);
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.default_pixel(), vec![0.0]);

    // Any command that consumes or changes the document commits the pending value first.
    app.run("layer.select", json!({"layer":layer.0})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.25);
    assert!(app.uv_mask_paint_preview.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);

    set(&mut app, layer, 0, 0.75);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.25);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.75);

    set(&mut app, layer, 0, 0.45);
    let _ = app.run("file.save", json!({}));
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.45);

    // A direct tab switch is handled by committing into the originating document and restoring
    // the user's selected tab before the new command runs.
    set(&mut app, layer, 0, 0.6);
    let old_doc = app.session.active().unwrap().doc.id;
    app.session.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    let new_layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer":new_layer.0})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(new_layer).unwrap().id, new_layer);
    let old_index = app.session.documents().iter().position(|state| state.doc.id == old_doc).unwrap();
    assert_eq!(app.session.documents()[old_index].doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.6);

    // A genuinely stale revision is reported and retained until explicitly canceled.
    app.session.set_active(old_index);
    set(&mut app, layer, 0, 0.4);
    app.session.active_mut().unwrap().revision += 1;
    assert!(app.run("layer.select", json!({"layer":layer.0})).is_err());
    assert!(app.uv_mask_paint_preview.is_some(), "a stale pending value must not disappear silently");
    cancel(&mut app);
    assert!(app.uv_mask_paint_preview.is_none());
}

#[test]
fn returning_opacity_to_canonical_value_cancels_preview_without_consuming_undo() {
    let (mut app, layer) = app();
    app.run("uv.maskPaint.modify", json!({"layer":layer.0,"index":0,"opacity":0.5})).unwrap();
    let state = app.session.active().unwrap();
    let (revision, history) = (state.revision, state.history.past_len());

    set(&mut app, layer, 0, 0.25);
    assert!(app.uv_mask_paint_preview.is_some());
    set(&mut app, layer, 0, 0.5);

    assert!(app.uv_mask_paint_preview.is_none());
    let state = app.session.active().unwrap();
    assert_eq!((state.revision, state.history.past_len()), (revision, history));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 1.0);
}

#[test]
fn changing_preview_target_commits_previous_value_then_keeps_new_preview() {
    let (mut app, old_layer) = app();
    set(&mut app, old_layer, 0, 0.25);
    let old_doc = app.session.active().unwrap().doc.id;

    app.session.execute("file.new", json!({"width":16,"height":12})).unwrap();
    let new_layer = app.session.active().unwrap().active_layer.unwrap();
    let state = app.session.active_mut().unwrap();
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.2]),
        layers: vec![Paint {
            name: "Second opacity".into(),
            visible: true,
            opacity: 0.8,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        }],
        active: 0,
    });
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(new_layer).unwrap().mask = Some(mask);

    set(&mut app, new_layer, 0, 0.3);

    assert_eq!(app.session.active().unwrap().doc.layer(new_layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.8);
    let preview = app.uv_mask_paint_preview.as_ref().unwrap();
    assert_eq!((preview.doc, preview.layer, preview.index, preview.opacity), (app.session.active().unwrap().doc.id, new_layer, 0, 0.3));
    let old_index = app.session.documents().iter().position(|state| state.doc.id == old_doc).unwrap();
    assert_eq!(app.session.documents()[old_index].doc.layer(old_layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.25);
}

#[test]
fn changing_paint_index_commits_old_preview_and_refreshes_same_document_revision() {
    let (mut app, layer) = app();
    let state = app.session.active_mut().unwrap();
    let paint = state.doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].clone();
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask.as_mut().unwrap().paint.as_mut().unwrap().layers.push(Paint {
        name: "Second opacity".into(),
        visible: true,
        opacity: 0.8,
        blend: photocraft_color::BlendMode::Normal,
        surface: paint.surface,
        fill_map: None,
        fill_map_size: [0, 0],
        fill_bounds: None,
        kind: EffectKind::Paint,
    });
    set(&mut app, layer, 0, 0.25);

    set(&mut app, layer, 1, 0.3);

    let state = app.session.active().unwrap();
    let stack = state.doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.layers[0].opacity, 0.25);
    assert_eq!(stack.layers[1].opacity, 0.8);
    let pending = app.uv_mask_paint_preview.as_ref().unwrap();
    assert_eq!((pending.doc, pending.revision, pending.layer, pending.index, pending.opacity), (state.doc.id, state.revision, layer, 1, 0.3));
    assert_eq!(value(&app, layer, 1, stack.layers[1].opacity), 0.3, "the new preview must not be stale after committing the previous value");
}

#[test]
fn stale_foreign_preview_is_retained_and_does_not_apply_new_target() {
    let (mut app, old_layer) = app();
    set(&mut app, old_layer, 0, 0.25);
    let old_doc = app.session.active().unwrap().doc.id;
    app.session.execute("file.new", json!({"width":16,"height":12})).unwrap();
    let new_layer = app.session.active().unwrap().active_layer.unwrap();
    let state = app.session.active_mut().unwrap();
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.2]),
        layers: vec![Paint {
            name: "Second opacity".into(),
            visible: true,
            opacity: 0.8,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        }],
        active: 0,
    });
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(new_layer).unwrap().mask = Some(mask);
    let old_index = app.session.documents().iter().position(|state| state.doc.id == old_doc).unwrap();
    let new_index = app.session.active_index().unwrap();
    app.session.set_active(old_index);
    app.session.active_mut().unwrap().revision += 1;
    app.session.set_active(new_index);

    set(&mut app, new_layer, 0, 0.3);

    let pending = app.uv_mask_paint_preview.as_ref().unwrap();
    assert_eq!((pending.doc, pending.layer, pending.index, pending.opacity), (old_doc, old_layer, 0, 0.25));
    assert!(app.ui.status.contains("stale"));
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().doc.layer(new_layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].opacity, 0.8);
}

#[test]
fn locked_mask_does_not_start_transient_opacity_preview() {
    let (mut app, layer) = app();
    let state = app.session.active_mut().unwrap();
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().locks.all = true;
    let (revision, history) = (state.revision, state.history.past_len());

    set(&mut app, layer, 0, 0.25);

    assert!(app.uv_mask_paint_preview.is_none());
    assert_eq!((app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len()), (revision, history));
}

#[test]
fn cpu_fallback_materializes_nested_transient_paint_masks() {
    let (mut app, layer) = app();
    let active = app.session.active_mut().unwrap();
    let doc = std::sync::Arc::make_mut(&mut active.doc);
    let child = doc.layers.pop().unwrap();
    doc.layers.push(photocraft_doc::Layer::group("Nested", vec![child]));
    set(&mut app, layer, 0, 0.25);
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    let cpu = crate::gpu_canvas::materialize_paint_masks(&shown);
    let flattened = cpu.layer(layer).unwrap().mask.as_ref().unwrap().surface.default_pixel();
    assert_eq!(flattened, cpu_document(&mut app, shown.clone(), 0).layer(layer).unwrap().mask.as_ref().unwrap().surface.default_pixel());
    assert!((flattened[0] - 0.3).abs() < 0.01, "fallback must use transient opacity, got {flattened:?}");
    assert_eq!(shown.layer(layer).unwrap().mask.as_ref().unwrap().surface.default_pixel(), vec![0.0], "GPU preview source remains unflattened");
}

#[test]
fn partial_cpu_materialization_matches_full_mask_values_with_feather_and_stays_damage_bounded() {
    let (mut app, layer) = app();
    let doc = {
        let state = app.session.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(&mut state.doc);
        let canvas = doc.bounds();
        doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(canvas, &[1.0, 0.0, 0.0, 1.0]);
        let mask = doc.layer_mut(layer).unwrap().mask.as_mut().unwrap();
        mask.feather = 2.0;
        let stack = mask.paint.as_mut().unwrap();
        stack.layers[0].kind = EffectKind::Fill { value: 0.9, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false };
        stack.layers[0].fill_bounds = Some([8, 6, 24, 20]);
        state.doc.as_ref().clone()
    };
    let damage = photocraft_geom::Rect::new(12, 10, 18, 16);
    let partial = crate::gpu_canvas::materialize_paint_masks_region(&doc, damage);
    let full = crate::gpu_canvas::materialize_paint_masks(&doc);
    let p = &partial.layer(layer).unwrap().mask.as_ref().unwrap().surface;
    let f = &full.layer(layer).unwrap().mask.as_ref().unwrap().surface;
    for y in damage.y0..damage.y1 {
        for x in damage.x0..damage.x1 {
            assert!((p.sample_channel(x, y, 0) - f.sample_channel(x, y, 0)).abs() < 1e-6, "mask pixel ({x},{y})");
        }
    }
    assert_eq!(p.sample_channel(9, 7, 0), f.sample_channel(9, 7, 0), "the feather halo includes nearby mask pixels");
    let partial_render = photocraft_compose::render(&partial, damage);
    let full_render = photocraft_compose::render(&full, damage);
    for (i, (a, b)) in partial_render.px.iter().zip(&full_render.px).enumerate() {
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "composited feathered pixel {i}: {a:?} != {b:?}");
    }
    assert!(full_render.px.iter().any(|px| px[0] > 0.05 && px[3] > 0.05), "the red layer is visible through the materialized mask");
    let unmaterialized_render = photocraft_compose::render(&doc, damage);
    assert!(full_render.px.iter().zip(&unmaterialized_render.px).any(|(a, b)| (a[0] - b[0]).abs() > 0.1), "materialization changes the rendered mask result");

    let mut huge = doc.clone();
    huge.size.width = 100_000;
    huge.size.height = 100_000;
    let tiny = crate::gpu_canvas::materialize_paint_masks_region(&huge, photocraft_geom::Rect::new(12, 10, 18, 16));
    let tiles = tiny.layer(layer).unwrap().mask.as_ref().unwrap().surface.tiles().count();
    assert!(tiles <= 1, "a tiny damaged rect on a huge canvas materializes at most its one tile, got {tiles}");
}

#[test]
fn fill_bounds_preview_uses_low_res_proxy_and_commits_once_on_release() {
    let (mut app, layer) = app();
    {
        let state = app.session.active_mut().unwrap();
        let paint = std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask.as_mut().unwrap().paint.as_mut().unwrap();
        paint.layers[0].kind = EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false };
    }
    let history = app.session.active().unwrap().history.past_len();
    let next = [4, 4, 20, 16];
    set_fill(&mut app, layer, 0, next, 20.0);

    let (shown, key) = display_doc(&mut app, 0).expect("canonical canvas stays available during Fill manipulation");
    assert_eq!(key, 0);
    assert!(shown.layer(layer).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0) < 0.01, "pointer moves do not recompose full-resolution pixels");
    let now = std::time::Instant::now();
    let (mut proxy, _) = super::fill_mask_proxy_at(&mut app, 0, now);
    let deadline = now + std::time::Duration::from_secs(2);
    while proxy.as_ref().is_none_or(|proxy| proxy.key != super::current_fill_key(&app, 0).unwrap()) {
        std::thread::sleep(std::time::Duration::from_millis(1));
        (proxy, _) = super::fill_mask_proxy_at(&mut app, 0, std::time::Instant::now());
        assert!(std::time::Instant::now() < deadline, "bounded Fill proxy completes off-thread");
    }
    let proxy = proxy.unwrap();
    assert!(proxy.size[0] <= 256 && proxy.size[1] <= 256);
    assert!(proxy.values.iter().any(|value| *value > 0.95), "proxy includes the selected Fill");
    assert_eq!(app.session.active().unwrap().history.past_len(), history, "preview is transient");

    let commit = super::pending(&app).expect("latest Fill preview is pending");
    app.run("uv.maskPaint.modify", commit.params()).unwrap();
    assert!(app.uv_mask_paint_preview.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "gesture creates one history entry");
    let committed = app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].clone();
    assert_eq!(committed.fill_bounds, Some(next));
    assert!(matches!(committed.kind, EffectKind::Fill { rotation, .. } if rotation == 20.0));
    app.run("edit.undo", json!({})).unwrap();
    let undone = app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].clone();
    assert_eq!(undone.fill_bounds, None);
    assert!(matches!(undone.kind, EffectKind::Fill { rotation, .. } if rotation == 0.0));
}

#[test]
fn gray_fill_preview_skips_full_cpu_worker_for_both_mask_target_forms() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":512,"height":129})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    let state = session.active_mut().unwrap();
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.2]),
        layers: vec![Paint {
            name: "Fill".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: Some([0, 0, 32, 32]),
            kind: EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
        }],
        active: 0,
    });
    photocraft_compose::mask_paint::refresh(&mut mask, Some(state.doc.bounds()));
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask = Some(mask);
    state.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::LayerMask;
    state.channel_view.layer_mask = None;
    let mut app = PhotocraftApp::new(session, Services::default());
    set_fill(&mut app, layer, 0, [8, 0, 40, 32], 0.0);
    assert!(super::current_fill_gray_mask(&app, 0), "the target-only mask selection is opaque Gray");
    assert!(super::display_doc(&mut app, 0).is_some());
    let ui_ctx = egui::Context::default();
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(2);
    let proxy = loop {
        let _ = crate::channel_view::ensure(&mut app, &ui_ctx, 0);
        if let Some(proxy) = super::sampled_fill_proxy(&app, 0)
            && proxy.key == super::current_fill_key(&app, 0).unwrap()
        {
            break proxy;
        }
        assert!(std::time::Instant::now() < deadline, "2D preview worker publishes its first proxy");
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert!(proxy.size[0] <= 256 && proxy.size[1] <= 256);
    assert!(crate::channel_view::ensure(&mut app, &ui_ctx, 0).is_some());
    let doc = app.session.active().unwrap().doc.clone();
    let cache = app.channel_views.get(&doc.id.0).unwrap();
    assert_eq!(cache.preview_key, proxy.key, "the texture key matches the sampled transform");
    let mut view = app.session.active().unwrap().channel_view.clone();
    // `ensure` normalizes target-only mask selection to this explicit display descriptor.
    view.layer_mask =
        Some(photocraft_engine::mask_view_cmds::LayerMaskView { layer, filter: false, mode: photocraft_engine::mask_view_cmds::MaskViewMode::Gray });
    let pixels = crate::channel_view::render_fill_mask_proxy(
        &doc,
        &view,
        photocraft_geom::Rect::new(0, 0, proxy.size[0] as i32, proxy.size[1] as i32),
        proxy.size,
        crate::channel_view::Look::default(),
        &proxy,
    )
    .unwrap();
    let px = (16 * proxy.size[1] as usize / 129) * proxy.size[0] as usize + (24 * proxy.size[0] as usize / 512);
    assert_eq!(pixels[px], egui::Color32::WHITE);

    let state = app.session.active_mut().unwrap();
    state.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
    state.channel_view.layer_mask =
        Some(photocraft_engine::mask_view_cmds::LayerMaskView { layer, filter: false, mode: photocraft_engine::mask_view_cmds::MaskViewMode::Gray });
    assert!(super::current_fill_gray_mask(&app, 0), "the explicit mask-view selection is also opaque Gray");
    assert!(super::display_doc(&mut app, 0).is_some());
    assert!(crate::channel_view::ensure(&mut app, &ui_ctx, 0).is_some());
}

#[test]
fn fill_sampler_throttles_samples_but_commit_keeps_latest_raw_transform() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":512,"height":129})).unwrap();
    session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
    let layer = session.active().unwrap().active_layer.unwrap();
    let state = session.active_mut().unwrap();
    let mut base = Surface::with_default(PixelFormat::GRAY8, &[0.2]);
    base.write_region(photocraft_geom::Rect::new(300, 20, 301, 21), &[0.8]);
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base,
        layers: vec![Paint {
            name: "Fill".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: Some([0, 0, 32, 32]),
            kind: EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
        }],
        active: 0,
    });
    photocraft_compose::mask_paint::refresh(&mut mask, Some(state.doc.bounds()));
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask = Some(mask);
    let mut app = PhotocraftApp::new(session, Services::default());
    let t0 = std::time::Instant::now();
    set_fill(&mut app, layer, 0, [8, 0, 40, 32], 0.0);
    let _ = super::fill_mask_proxy_at(&mut app, 0, t0);
    let first_key = super::current_fill_key(&app, 0).unwrap();
    set_fill(&mut app, layer, 0, [24, 0, 56, 32], 17.0);
    let (still_unready, next) = super::fill_mask_proxy_at(&mut app, 0, t0 + std::time::Duration::from_millis(20));
    let latest_key = super::current_fill_key(&app, 0).unwrap();
    assert!(still_unready.is_none_or(|proxy| proxy.key != latest_key), "no result can claim parameters that have not been sampled yet");
    assert!(next.is_some(), "a repaint is scheduled while the worker or cadence is pending");
    assert_ne!(super::current_fill_key(&app, 0).unwrap(), first_key);
    let latest = super::pending(&app).unwrap();
    assert_eq!(latest.bounds, Some([24, 0, 56, 32]));
    assert_eq!(latest.rotation, Some(17.0), "release keeps the raw latest manipulator parameters");

    let t50 = t0 + std::time::Duration::from_millis(50);
    let (mut proxy, _) = super::fill_mask_proxy_at(&mut app, 0, t50);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while proxy.as_ref().is_none_or(|proxy| proxy.key != latest_key) {
        std::thread::sleep(std::time::Duration::from_millis(1));
        (proxy, _) = super::fill_mask_proxy_at(&mut app, 0, std::time::Instant::now());
        assert!(std::time::Instant::now() < deadline, "worker publishes the latest coalesced transform");
    }
    assert_eq!(proxy.unwrap().key, latest_key);
    assert!(!super::fill_preview_pending(&app, layer, 0));
    app.run("uv.maskPaint.modify", latest.params()).unwrap();
    let committed = app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].clone();
    assert_eq!(committed.fill_bounds, Some([24, 0, 56, 32]));
    assert!(matches!(committed.kind, EffectKind::Fill { rotation, .. } if rotation == 17.0));
}

#[test]
fn canvas_fill_snapshot_stays_canonical_while_commit_keeps_raw_latest_parameters() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":512,"height":129})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    let state = session.active_mut().unwrap();
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.2]),
        layers: vec![Paint {
            name: "Fill".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: Some([0, 0, 32, 32]),
            kind: EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
        }],
        active: 0,
    });
    photocraft_compose::mask_paint::refresh(&mut mask, Some(state.doc.bounds()));
    std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask = Some(mask);
    let mut app = PhotocraftApp::new(session, Services::default());
    let next_bounds = [24, 8, 56, 40];
    set_fill(&mut app, layer, 0, next_bounds, 31.0);

    let (gpu, gpu_key) = gpu_display_doc(&mut app, 0).expect("canvas keeps its canonical document");
    assert_eq!(gpu_key, 0);
    let gpu_paint = &gpu.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
    assert_eq!(gpu_paint.fill_bounds, Some([0, 0, 32, 32]), "canvas source is not the raw manipulator snapshot");
    assert!(matches!(gpu_paint.kind, EffectKind::Fill { rotation, .. } if rotation == 0.0));
    assert_eq!(gpu_paint.surface.sample_channel(30, 12, 0), 0.0, "canonical flattened cache is left untouched during manipulation");
    let commit = super::pending(&app).unwrap();
    assert_eq!(commit.bounds, Some(next_bounds));
    assert_eq!(commit.rotation, Some(31.0));
}

#[test]
fn fill_proxy_scheduler_coalesces_to_latest_key_at_twenty_hz() {
    let t0 = std::time::Instant::now();
    let mut schedule = super::FillSampleSchedule::default();
    assert!(schedule.accept(t0, 10), "first sample is immediate");
    assert!(!schedule.accept(t0 + std::time::Duration::from_millis(12), 11));
    assert!(!schedule.accept(t0 + std::time::Duration::from_millis(35), 12));
    assert_eq!(schedule.latest_key, Some(10), "intermediate pointer states are coalesced");
    assert_eq!(schedule.until_next(t0 + std::time::Duration::from_millis(35)), Some(std::time::Duration::from_millis(15)));
    assert!(schedule.accept(t0 + std::time::Duration::from_millis(50), 12), "the latest pointer state is accepted at 20 Hz");
    assert_eq!(schedule.latest_key, Some(12));
    assert!(!schedule.accept(t0 + std::time::Duration::from_millis(100), 12), "an unchanged state is not resampled");
}

#[test]
fn sampled_fill_proxy_is_rejected_after_revision_change_or_cancel() {
    let (mut app, layer) = app();
    {
        let state = app.session.active_mut().unwrap();
        let paint = std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask.as_mut().unwrap().paint.as_mut().unwrap();
        paint.layers[0].kind = EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false };
    }
    set_fill(&mut app, layer, 0, [2, 3, 20, 18], 0.0);
    let key = super::current_fill_key(&app, 0).unwrap();
    let preview = app.uv_mask_paint_preview.as_mut().unwrap();
    let proxy = super::FillMaskProxy {
        generation: preview.generation,
        doc: preview.doc,
        revision: preview.revision,
        layer,
        index: 0,
        key,
        size: [1, 1],
        values: std::sync::Arc::new(vec![1.0]),
    };
    preview.mask_sampler.ready = Some(proxy);
    assert!(super::sampled_fill_proxy(&app, 0).is_some());
    app.session.active_mut().unwrap().revision += 1;
    assert!(super::sampled_fill_proxy(&app, 0).is_none(), "result from an earlier document revision is ignored");
    app.session.active_mut().unwrap().revision -= 1;
    cancel(&mut app);
    assert!(super::sampled_fill_proxy(&app, 0).is_none(), "cancelling drops the sampler generation");
}
