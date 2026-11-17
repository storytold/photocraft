#![cfg(feature = "uv-workbench")]
use photocraft_doc::SampleType;
use photocraft_engine::Session;
use serde_json::json;

fn fixture(depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.import", json!({"path":source})).unwrap();
    s.execute("uv.texture.create", json!({"width":32,"height":32,"depth":depth})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s
}
fn value(s: &Session) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0)
}
fn paint(s: &mut Session, color: &str) {
    s.execute(
        "paint.stroke",
        json!({"target":"mask","points":[[8.5,8.5,1.0]],"size":12,"color":color,"brush":{"hardness":1.0,"opacity":1.0,"flow":1.0},"seed":42}),
    )
    .unwrap();
}
#[test]
fn paint_stack_visibility_opacity_order_delete_and_undo_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = fixture(depth);
        s.execute("uv.maskPaint.add", json!({"name":"Black"})).unwrap();
        paint(&mut s, "#000000");
        assert!(value(&s) < 0.01);
        s.execute("paint.stroke", json!({"target":"mask","points":[[8.5,8.5]],"size":12,"erase":true,"brush":{"hardness":1.0},"seed":42})).unwrap();
        assert!(value(&s) > 0.99, "Eraser removes this Paint layer's coverage");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(value(&s) < 0.01);
        s.execute("uv.maskPaint.modify", json!({"index":0,"opacity":0.5})).unwrap();
        assert!((value(&s) - 0.5).abs() < 0.02);
        s.execute("uv.maskPaint.modify", json!({"index":0,"visible":false})).unwrap();
        assert!(value(&s) > 0.99);
        s.execute("edit.undo", json!({})).unwrap();
        assert!((value(&s) - 0.5).abs() < 0.02);
        s.execute("uv.maskPaint.modify", json!({"index":0,"opacity":1.0})).unwrap();
        s.execute("uv.maskPaint.add", json!({"name":"White"})).unwrap();
        paint(&mut s, "#ffffff");
        assert!(value(&s) > 0.99);
        s.execute("uv.maskPaint.modify", json!({"index":1,"to":0})).unwrap();
        assert!(value(&s) < 0.01);
        s.execute("uv.maskPaint.modify", json!({"index":1,"remove":true})).unwrap();
        assert!(value(&s) > 0.99);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(value(&s) < 0.01);
        s.execute("edit.redo", json!({})).unwrap();
        assert!(value(&s) > 0.99);
    }
}
#[test]
fn live_brush_preview_and_container_keep_editable_layers() {
    let mut s = fixture(16);
    s.execute("uv.maskPaint.add", json!({"name":"Live black"})).unwrap();
    let p = json!({"target":"mask","points":[[8.5,8.5,1.0]],"size":12,"color":"#000000","brush":{"hardness":1.0},"seed":42});
    let live = photocraft_engine::brush_cmds::LiveStroke::begin(&s, &p).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    assert!(live.doc.layer(id).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0) < 0.01);
    assert!(value(&s) > 0.99, "Preview must not change history's document");
    s.offer_live_stroke(live).unwrap();
    s.execute("paint.stroke", p).unwrap();
    assert!(value(&s) < 0.01);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paint.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert!(value(&restored) < 0.01);
    let st = restored.active().unwrap();
    let stack = st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.layers[0].name, "Live black");
    assert_eq!(stack.layers[0].surface.format().sample, SampleType::U16);
    restored.execute("uv.maskPaint.modify", json!({"index":0,"remove":true})).unwrap();
    assert!(value(&restored) > 0.99);
}
#[test]
fn invalid_stack_edits_preserve_document() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for p in [json!({"index":999}), json!({"index":0,"opacity":-1}), json!({"index":0,"to":999}), json!({"index":0,"repeat_u":false})] {
        assert!(s.execute("uv.maskPaint.modify", p).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
    s.execute("uv.maskPaint.modify", json!({"index":0,"visible":false})).unwrap();
    assert!(s.execute("paint.stroke", json!({"target":"mask","points":[[8,8]],"size":12})).is_err());
}

#[test]
fn removing_an_effect_below_the_active_effect_keeps_the_same_effect_selected() {
    let mut s = fixture(8);
    for name in ["First", "Second", "Third"] {
        s.execute("uv.maskPaint.add", json!({"name":name})).unwrap();
    }
    s.execute("uv.maskPaint.modify", json!({"index":2})).unwrap();
    s.execute("uv.maskPaint.modify", json!({"index":0,"remove":true})).unwrap();
    let stack = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.active, 1);
    assert_eq!(stack.layers[stack.active].name, "Third");
}

#[test]
fn transform_of_mask_cache_is_rejected_without_partially_mutating_the_document() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"name":"Paint"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let result = s.execute("edit.transform", json!({"dx":4,"dy":2,"target":"mask"}));
    assert!(result.is_err(), "legacy mask transforms must not be silently lost behind the editable stack");
    assert_eq!(s.active().unwrap().doc, before, "rejection leaves document and history untouched");

    let layer = s.active().unwrap().active_layer.unwrap();
    s.edit("Link test mask", |doc, _| {
        doc.layer_mut(layer).unwrap().mask.as_mut().unwrap().linked = true;
        Ok(serde_json::Value::Null)
    })
    .unwrap();
    let before_linked_transform = s.active().unwrap().doc.clone();
    let result = s.execute("edit.transform", json!({"dx":4,"dy":2}));
    assert!(result.is_err(), "linked layer transforms must not detach the editable stack from its cache");
    assert_eq!(s.active().unwrap().doc, before_linked_transform, "linked transform rejection is atomic too");
}

#[test]
fn fill_and_levels_layers_compose_modify_undo_and_round_trip() {
    let mut s = fixture(16);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.25})).unwrap();
    s.execute("uv.maskPaint.modify", json!({"index":0,"rotation":37.0,"repeat_u":false,"repeat_v":true,"mirror_u":true,"mirror_v":false})).unwrap();
    assert!((value(&s) - 0.25).abs() < 0.01);
    s.execute("uv.maskPaint.add", json!({"kind":"levels","input_black":0.0,"gamma":1.0,"input_white":0.5,"output_black":0.0,"output_white":1.0})).unwrap();
    assert!((value(&s) - 0.5).abs() < 0.01);
    s.execute("uv.maskPaint.modify", json!({"index":1,"output_white":0.8,"opacity":0.5})).unwrap();
    assert!((value(&s) - 0.325).abs() < 0.02);
    let before_invalid = s.active().unwrap().doc.clone();
    for params in [json!({"index":1,"gamma":0.0}), json!({"index":1,"input_black":0.8}), json!({"index":0,"gamma":2.0}), json!({"index":0,"rotation":36001.0})]
    {
        assert!(s.execute("uv.maskPaint.modify", params).is_err());
        assert_eq!(s.active().unwrap().doc, before_invalid);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert!((value(&s) - 0.5).abs() < 0.01);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("effects.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert!((value(&restored) - 0.5).abs() < 0.01);
    let stack = restored.active().unwrap().doc.layer(restored.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert!(matches!(&stack.layers[0].kind, photocraft_doc::mask_paint::EffectKind::Fill { value, .. } if (*value - 0.25).abs() < f32::EPSILON));
    assert!(matches!(
        &stack.layers[0].kind,
        photocraft_doc::mask_paint::EffectKind::Fill { repeat_u: false, repeat_v: true, mirror_u: true, mirror_v: false, .. }
    ));
    assert!(matches!(&stack.layers[0].kind, photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } if (*rotation - 37.0).abs() < f32::EPSILON));
    assert!(matches!(&stack.layers[1].kind, photocraft_doc::mask_paint::EffectKind::Levels { input_white, .. } if (*input_white - 0.5).abs() < f32::EPSILON));
}

#[test]
fn paint_and_fill_blend_modes_are_grayscale_only_and_blur_stays_nondestructive() {
    let mut s = fixture(16);
    s.execute("uv.maskPaint.add", json!({"blend":"multiply","name":"Multiply paint"})).unwrap();
    paint(&mut s, "#000000");
    assert!(value(&s) < 0.01);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.5,"blend":"screen"})).unwrap();
    assert!(value(&s) > 0.45);
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4,"blend":"screen"})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"levels"})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(
        s.execute("paint.stroke", json!({"target":"mask","points":[[8.5,8.5]],"size":8,"color":"#000000","brush":{"hardness":1.0}})).is_err(),
        "selected Blur effects cannot receive brush strokes"
    );
    assert_eq!(s.active().unwrap().doc, before, "rejected brush edits leave the document unchanged");

    for invalid in [
        json!({"index":0,"blend":"hue"}),
        json!({"index":0,"blend":"color_burn"}),
        json!({"index":0,"blend":"overlay"}),
        json!({"index":3,"blend":"multiply"}),
        json!({"index":2,"radius":65}),
    ] {
        assert!(s.execute("uv.maskPaint.modify", invalid).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
    let stack = s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.layers[0].blend, photocraft_color::BlendMode::Multiply);
    assert!(matches!(stack.layers[2].kind, photocraft_doc::mask_paint::EffectKind::Blur { radius: 4 }));
    assert_eq!(stack.layers[2].blend, photocraft_color::BlendMode::Screen, "Blur accepts scalar blend modes on creation");

    s.execute("uv.maskPaint.modify", json!({"index":2,"blend":"multiply"})).unwrap();
    let stack = s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.layers[2].blend, photocraft_color::BlendMode::Multiply, "Blur accepts scalar mask blend modes");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mask-blur.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    let stack = restored.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
    assert_eq!(stack.layers[0].blend, photocraft_color::BlendMode::Multiply);
    assert!(matches!(stack.layers[2].kind, photocraft_doc::mask_paint::EffectKind::Blur { radius: 4 }));
    assert_eq!(stack.layers[2].blend, photocraft_color::BlendMode::Multiply, "Blur blend survives native save/load");
}

#[test]
fn mask_blend_commands_accept_the_compact_scalar_set() {
    let modes = [
        ("normal", photocraft_color::BlendMode::Normal),
        ("darken", photocraft_color::BlendMode::Darken),
        ("multiply", photocraft_color::BlendMode::Multiply),
        ("lighten", photocraft_color::BlendMode::Lighten),
        ("screen", photocraft_color::BlendMode::Screen),
        ("linear_dodge", photocraft_color::BlendMode::LinearDodge),
        ("subtract", photocraft_color::BlendMode::Subtract),
        ("difference", photocraft_color::BlendMode::Difference),
    ];
    for (name, expected) in modes {
        let mut s = fixture(8);
        s.execute("uv.maskPaint.add", json!({"blend":name})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let stack = s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
        assert_eq!(stack.layers[0].blend, expected, "{name}");
    }
    assert_eq!(photocraft_doc::mask_paint::MASK_BLEND_MODES.len(), modes.len());
}

#[test]
fn mask_blend_change_undo_redo_recomposes_the_grayscale_stack() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.5})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.5})).unwrap();
    assert!((value(&s) - 0.5).abs() < 0.01);

    s.execute("uv.maskPaint.modify", json!({"index":1,"blend":"multiply"})).unwrap();
    assert!((value(&s) - 0.25).abs() < 0.01, "multiply blends the fill into the existing white mask");

    s.execute("edit.undo", json!({})).unwrap();
    assert!((value(&s) - 0.5).abs() < 0.01, "undo restores Normal blending");
    s.execute("edit.redo", json!({})).unwrap();
    assert!((value(&s) - 0.25).abs() < 0.01, "redo restores Multiply blending");
}

#[test]
fn paint_damage_includes_downstream_blur_and_hiding_or_deleting_blur_invalidates_canvas() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"paint","name":"Source"})).unwrap();
    paint(&mut s, "#ffffff");
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let doc = s.active().unwrap().doc.clone();
    let canvas = doc.bounds();
    let source = &doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface;
    let source_bounds = source.content_bounds();

    s.execute("uv.maskPaint.select", json!({"index":0})).unwrap();
    s.execute("uv.maskPaint.modify", json!({"index":0,"visible":false})).unwrap();
    let expected = source_bounds.inflate(4).intersect(&canvas);
    assert_eq!(s.active().unwrap().last_damage, Some(expected), "Paint edits report their downstream Blur halo");
    assert_mask_cache_matches_full(&s);

    s.execute("uv.maskPaint.modify", json!({"index":1,"visible":false})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(canvas), "hiding a Blur can change its entire output area");
    assert_mask_cache_matches_full(&s);

    s.execute("uv.maskPaint.modify", json!({"index":1,"remove":true})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(canvas), "deleting a Blur invalidates the full mask canvas");
    assert_mask_cache_matches_full(&s);
}

#[test]
fn adding_mask_effect_invalidates_gray_preview_even_when_layer_pixels_are_empty() {
    let mut s = fixture(8);
    let canvas = s.active().unwrap().doc.bounds();
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":8})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(canvas));
}

#[test]
fn unrelated_document_edits_do_not_rebuild_a_bound_blur_mask_cache() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.4})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.clone();

    // This ordinary pixel edit leaves the non-destructive stack untouched. The bound mask's
    // cached result must stay stable and the edit must not demand a new topology shell map.
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();

    let after = s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap();
    let previous = before.layer(layer).unwrap().mask.as_ref().unwrap();
    assert_eq!(after.surface.read_region(before.bounds()), previous.surface.read_region(before.bounds()));
    assert_eq!(after.paint, previous.paint);
}

#[test]
fn rebinding_a_masked_document_rebuilds_blur_cache_for_new_shell_ownership() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.2})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let doc_id = s.active().unwrap().doc.id;

    // Bind the same texture to the second material. Shell ownership changes even though the
    // document and its stack do not, so the materialized Blur result must be refreshed.
    s.execute("uv.texture.bind", json!({"material":1,"channel":"baseColor"})).unwrap();
    let state = s.uv_documents().iter().find(|state| state.doc.id == doc_id).unwrap();
    let mask = state.doc.layer(layer).unwrap().mask.as_ref().unwrap();
    let scene = s.uv.scene.as_deref().unwrap();
    let topology = s.uv.topology.as_deref().unwrap();
    let shell_ids = topology.shell_id_map_for_materials(scene, state.doc.size.width, state.doc.size.height, &[0, 1], [0, 0]).unwrap();
    let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(state.doc.bounds(), &shell_ids).unwrap();
    let mut expected = mask.clone();
    assert!(photocraft_compose::mask_paint::refresh_in_canvas_with_shell_ids(&mut expected, state.doc.bounds(), shell_map));
    assert_eq!(mask.surface.read_region(state.doc.bounds()), expected.surface.read_region(state.doc.bounds()));
}

#[test]
fn undo_redo_and_history_jumps_reuse_valid_4k_blur_cache_snapshots() {
    let mut s = Session::new();
    s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.import", json!({"path":source})).unwrap();
    s.execute("uv.texture.create", json!({"material":0,"width":4096,"height":4096,"depth":8})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.35})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let before = std::sync::Arc::clone(&s.active().unwrap().doc);
    let before_state = s.active().unwrap().history.past_len();

    s.execute("layer.renameLayer", json!({"layer":layer.0,"name":"Renamed mask target"})).unwrap();
    let renamed = std::sync::Arc::clone(&s.active().unwrap().doc);
    let started = std::time::Instant::now();
    s.execute("edit.undo", json!({})).unwrap();
    let undo_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(
        std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc),
        "valid history snapshot should be restored without cloning/recompositing its mask surface"
    );

    let started = std::time::Instant::now();
    s.execute("edit.redo", json!({})).unwrap();
    let redo_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(std::sync::Arc::ptr_eq(&renamed, &s.active().unwrap().doc), "redo should reuse the same valid snapshot");

    let started = std::time::Instant::now();
    s.execute("history.selectState", json!({"state":before_state})).unwrap();
    let undo_jump_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc), "history panel jump should reuse the valid snapshot");
    let started = std::time::Instant::now();
    s.execute("history.selectState", json!({"state":before_state + 1})).unwrap();
    let redo_jump_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(std::sync::Arc::ptr_eq(&renamed, &s.active().unwrap().doc), "redo history jump should reuse the valid snapshot");
    println!("4K bound Blur history transition ms: undo={undo_ms:.2} redo={redo_ms:.2} select-undo={undo_jump_ms:.2} select-redo={redo_jump_ms:.2}");
}

#[test]
fn undo_recomposes_old_mask_snapshot_after_uv_topology_reimport() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.25})).unwrap();
    s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.renameLayer", json!({"layer":layer.0,"name":"After cached Blur"})).unwrap();

    // Reimport publishes a new topology and refreshes the current document only. Earlier
    // history snapshots must miss the cache validity key and be recomposed when restored.
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.reimport", json!({"path":source})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();

    let state = s.active().unwrap();
    let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
    let mask = layer.mask.as_ref().unwrap();
    let materials: Vec<_> = photocraft_uv::preview::Channel::MAPS
        .into_iter()
        .flat_map(|channel| s.uv.map_bindings(channel).iter().enumerate().filter_map(move |(material, id)| (*id == Some(state.doc.id)).then_some(material)))
        .collect();
    let shell_ids =
        s.uv.topology
            .as_ref()
            .unwrap()
            .shell_id_map_for_materials(s.uv.scene.as_ref().unwrap(), state.doc.size.width, state.doc.size.height, &materials, [0, 0])
            .unwrap();
    let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(state.doc.bounds(), &shell_ids).unwrap();
    let mut expected = mask.clone();
    assert!(photocraft_compose::mask_paint::refresh_in_canvas_with_shell_ids(&mut expected, state.doc.bounds(), shell_map));
    assert_eq!(mask.surface.read_region(state.doc.bounds()), expected.surface.read_region(state.doc.bounds()));
}

fn assert_mask_cache_matches_full(s: &Session) {
    let state = s.active().unwrap();
    let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
    let mask = layer.mask.as_ref().unwrap();
    let mut full = mask.clone();
    let materials: Vec<_> = photocraft_uv::preview::Channel::MAPS
        .into_iter()
        .flat_map(|channel| s.uv.map_bindings(channel).iter().enumerate().filter_map(move |(material, id)| (*id == Some(state.doc.id)).then_some(material)))
        .collect();
    let has_blur = mask.paint.as_ref().is_some_and(|stack| {
        stack.layers.iter().any(|effect| effect.visible && effect.opacity > 0.0 && matches!(effect.kind, photocraft_doc::mask_paint::EffectKind::Blur { .. }))
    });
    if !materials.is_empty() && has_blur {
        let shell_ids =
            s.uv.topology
                .as_ref()
                .unwrap()
                .shell_id_map_for_materials(s.uv.scene.as_ref().unwrap(), state.doc.size.width, state.doc.size.height, &materials, [0, 0])
                .unwrap();
        let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(state.doc.bounds(), &shell_ids).unwrap();
        assert!(photocraft_compose::mask_paint::refresh_in_canvas_with_shell_ids(&mut full, state.doc.bounds(), shell_map));
    } else {
        photocraft_compose::mask_paint::refresh_in_canvas(&mut full, state.doc.bounds());
    }
    assert_eq!(mask.surface.read_region(state.doc.bounds()), full.surface.read_region(state.doc.bounds()));
}

#[test]
fn blur_halo_budget_rejects_unbounded_stacks_without_mutation() {
    let mut s = fixture(8);
    for _ in 0..8 {
        s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":64})).unwrap();
    }
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("uv.maskPaint.add", json!({"kind":"blur","radius":1})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn removing_fill_image_clears_source_and_is_undoable() {
    let mut s = fixture(8);
    s.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.4})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.edit("Set test Fill image", |doc, _| {
        let fill = &mut doc.layer_mut(layer).unwrap().mask.as_mut().unwrap().paint.as_mut().unwrap().layers[0];
        fill.fill_map = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.7]));
        fill.fill_map_size = [16, 16];
        Ok(serde_json::Value::Null)
    })
    .unwrap();

    s.execute("uv.maskPaint.modify", json!({"index":0,"clear_fill_map":true})).unwrap();
    let fill = &s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
    assert!(fill.fill_map.is_none());
    assert_eq!(fill.fill_map_size, [0, 0]);
    s.execute("edit.undo", json!({})).unwrap();
    let fill = &s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
    assert!(fill.fill_map.is_some(), "undo restores the image source");
    assert_eq!(fill.fill_map_size, [16, 16]);
}

#[test]
fn opacity_drag_is_one_undo_and_does_not_rewrite_untouched_mask_tiles() {
    let mut s = fixture(16);
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("Dense base", |doc, _| {
        doc.size = photocraft_geom::Size::new(1024, 1024);
        let mask = doc.layer_mut(id).unwrap().mask.as_mut().unwrap();
        mask.surface.write_region(photocraft_geom::Rect::from_xywh(0, 0, 1024, 1024), &vec![0.75; 1024 * 1024]);
        Ok(())
    })
    .unwrap();
    s.execute("uv.maskPaint.add", json!({})).unwrap();
    paint(&mut s, "#000000");
    s.execute("uv.maskPaint.add", json!({"name":"Far Paint"})).unwrap();
    s.execute("paint.stroke", json!({"target":"mask","points":[[800.5,800.5]],"size":12,"color":"#000000","brush":{"hardness":1.0}})).unwrap();
    s.execute("uv.maskPaint.select", json!({"index":0})).unwrap();
    let distant = photocraft_geom::TileCoord::new(3, 3);
    let before = s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.tile(distant).unwrap().clone();
    let start = std::time::Instant::now();
    for opacity in [0.9, 0.8, 0.7] {
        s.execute("uv.maskPaint.modify", json!({"index":0,"opacity":opacity,"coalesce":"opacity-drag"})).unwrap();
    }
    println!("Three opacity updates on a dense 1K base: {:?}", start.elapsed());
    assert!(!s.active().unwrap().last_damage.unwrap().contains(768, 768), "Opacity must not invalidate the distant viewport region");
    let after = s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.tile(distant).unwrap().clone();
    assert!(std::sync::Arc::ptr_eq(&before, &after), "Opacity must preserve cache tiles outside Paint coverage");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(value(&s) < 0.01, "One undo restores opacity before the whole drag");
}
