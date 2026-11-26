use super::*;
use photocraft_color::Color;
use photocraft_doc::{Effect, Fill, FxCommon, FxPaint, Knockout, LayerMask, StrokeFx, StrokePosition};

fn shadow_fixture(depth: u64, distance: u64, angle: u64, width: i32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": width, "height": H, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("synthetic shape", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(12, 16, 20, 24), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute(
        "layer.layerStyle.dropShadow",
        json!({"color": "#ff0000", "opacity": 100, "blend": "normal", "useGlobalLight": false,
            "angle": angle, "distance": distance, "size": 0, "spread": 0}),
    )
    .unwrap();
    s
}

#[test]
fn conversion_preserves_long_off_canvas_shadows_at_every_depth() {
    for depth in DEPTHS {
        for (distance, angle, width) in [(320, 180, 128), (400, 180, 128), (400, 0, 128), (200, 180, 128), (400, 180, 1024)] {
            let mut s = shadow_fixture(depth, distance, angle, width);
            let region = Rect::new(-512, 0, 1536, H);
            let before = s.active().unwrap().doc.clone();
            let expected = photocraft_compose::render(&before, region).into_px();
            assert_eq!(expected.iter().filter(|p| p[0] > 0.99 && p[1] < 0.01 && p[3] > 0.99).count(), 64, "direct shadow control");

            convert(&mut s);
            let actual = photocraft_compose::render(&s.active().unwrap().doc, region).into_px();
            assert!(max_diff(&actual, &expected) < 1e-6, "cropped shadow: depth {depth}, distance {distance}, angle {angle}, width {width}");
            let sm = active_smart(&s);
            let SmartSource::Embedded { file_name, bytes } = &sm.source else { panic!("expected embedded contents") };
            let child = decode_source(file_name, bytes).unwrap();
            assert_eq!(child.size, Size::new(distance as u32 + 8, 8), "the embedded canvas includes the complete shadow");

            let after = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, after);

            let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert!(max_diff(&photocraft_compose::render(&reopened, region).px, &expected) < 1e-6, "native reopen retains the off-canvas shadow");

            s.execute("layer.smartObjects.editContents", json!({})).unwrap();
            s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
            s.set_active(0);
            assert!(
                max_diff(&photocraft_compose::render(&s.active().unwrap().doc, region).px, &expected) < 1e-6,
                "saving editable contents retains the shadow"
            );
        }
    }
}

#[test]
fn conversion_preserves_long_shadows_on_groups_and_their_children() {
    for on_group in [false, true] {
        let mut s = shadow_fixture(8, 400, 180, 128);
        if on_group {
            s.edit("disable child effects", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().effects.enabled = false;
                Ok(())
            })
            .unwrap();
        }
        s.execute("layer.groupLayers", json!({})).unwrap();
        if on_group {
            s.execute(
                "layer.layerStyle.dropShadow",
                json!({"color": "#ff0000", "opacity": 100, "blend": "normal", "useGlobalLight": false,
                    "angle": 180, "distance": 400, "size": 0, "spread": 0}),
            )
            .unwrap();
        }
        let region = Rect::new(0, 0, 800, H);
        let expected = photocraft_compose::render(&s.active().unwrap().doc, region).into_px();
        assert_eq!(expected.iter().filter(|p| p[0] > 0.99 && p[1] < 0.01 && p[3] > 0.99).count(), 64, "direct shadow control");
        convert(&mut s);
        assert!(max_diff(&photocraft_compose::render(&s.active().unwrap().doc, region).px, &expected) < 1e-6, "shadow on group: {on_group}");
    }
}

#[test]
fn conversion_succeeds_for_deeply_nested_styled_groups_within_compositor_reach() {
    let mut s = shadow_fixture(8, 400, 180, 128);
    let mut layer = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().clone();
    let effects = layer.effects.clone();
    for _ in 0..10 {
        layer = Layer::group("nested shadow", vec![layer]);
        layer.blend = BlendMode::Normal;
        layer.effects = effects.clone();
    }
    let id = layer.id;
    s.edit("nested effects", |doc, _| {
        doc.layers = vec![layer].into();
        Ok(())
    })
    .unwrap();
    s.select_layer(id).unwrap();
    let region = Rect::new(-512, -512, 640, 560);
    let expected = photocraft_compose::render(&s.active().unwrap().doc, region).into_px();
    convert(&mut s);
    assert!(
        max_diff(&photocraft_compose::render(&s.active().unwrap().doc, region).px, &expected) < 1e-6,
        "deeply nested styled groups stay bounded by the compositor's effect region and preserve their output"
    );
}

#[test]
fn conversion_preserves_transparent_shape_strokes() {
    for depth in DEPTHS {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        s.execute("shape.create", json!({"kind": "rect", "rect": [2, 2, 8, 8], "fill": [0.0, 0.0, 0.0, 0.0]})).unwrap();
        s.edit("outline a transparent shape", |doc, active| {
            let layer = doc.layer_mut(active.unwrap()).unwrap();
            assert!(layer.surface().unwrap().content_bounds().is_empty(), "the effect source has no visible fill pixels");
            layer.effects.items.push(Effect::Stroke(StrokeFx {
                common: FxCommon::new(BlendMode::Normal, 1.0),
                size: 12.0,
                position: StrokePosition::Outside,
                paint: FxPaint::Color(Color::rgb(1.0, 0.0, 0.0)),
                overprint: false,
            }));
            Ok(())
        })
        .unwrap();
        let region = Rect::new(-16, -16, 32, 32);
        let expected = photocraft_compose::render(&s.active().unwrap().doc, region);
        assert_eq!(expected.get(-8, 6), [1.0, 0.0, 0.0, 1.0], "direct off-canvas stroke control");
        let before = s.active().unwrap().doc.clone();
        convert(&mut s);
        let actual = photocraft_compose::render(&s.active().unwrap().doc, region);
        assert_eq!(actual.get(-8, 6), expected.get(-8, 6), "the solid off-canvas stroke must survive conversion");
        // Conversion stores the floating compositor result at the document's sample depth.
        // Antialiased stroke edges can round by one sample; a clipped stroke differs by 1.0.
        let tolerance = match depth {
            8 => 1.0 / 255.0 + 1e-6,
            16 => 1.0 / 65535.0 + 1e-6,
            _ => 1e-6,
        };
        let difference = max_diff(&actual.px, &expected.px);
        assert!(difference <= tolerance, "cropped transparent-shape stroke: depth {depth}, max difference {difference}, tolerance {tolerance}");
        let after = s.active().unwrap().doc.clone();
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let actual = photocraft_compose::render(&reopened, region);
        assert_eq!(actual.get(-8, 6), expected.get(-8, 6), "native reopen retains the solid off-canvas stroke");
        assert!(max_diff(&actual.px, &expected.px) <= tolerance);
    }
}

#[test]
fn conversion_preserves_off_canvas_pixels_of_a_shapeless_knockout() {
    let mut s = session(8);
    s.edit("Knockout with off-canvas pixels", |doc, active| {
        let layer = doc.layer_mut(active.unwrap()).unwrap();
        layer.advanced.transparency_shapes = false;
        layer.advanced.knockout = Knockout::Shallow;
        let surface = layer.surface_mut().unwrap();
        surface.fill_rect(Rect::new(12, 16, 20, 24), &[0.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::new(400, 16, 408, 24), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let region = Rect::new(0, 0, 416, H);
    let before = photocraft_compose::render(&s.active().unwrap().doc, region);
    assert_eq!(before.get(400, 16), [0.0, 0.0, 1.0, 1.0]);
    convert(&mut s);
    let after = photocraft_compose::render(&s.active().unwrap().doc, region);
    assert_eq!(after.get(400, 16), before.get(400, 16));
}

#[test]
fn conversion_preserves_effects_from_an_empty_groups_mask() {
    let mut s = session(8);
    let mut group = Layer::group("Masked effects", Vec::new());
    group.blend = BlendMode::Normal;
    group.advanced.transparency_shapes = false;
    let mut mask = LayerMask::hide_all();
    mask.surface.fill_rect(Rect::new(12, 16, 20, 24), &[1.0]);
    group.mask = Some(mask);
    let id = group.id;
    s.edit("Add masked group", |doc, _| {
        doc.layers.push(group);
        Ok(())
    })
    .unwrap();
    s.select_layer(id).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#00ff00", "opacity": 100, "blend": "normal"})).unwrap();
    s.execute(
        "layer.layerStyle.dropShadow",
        json!({"color": "#ff0000", "opacity": 100, "blend": "normal", "useGlobalLight": false,
            "angle": 180, "distance": 400, "size": 0, "spread": 0}),
    )
    .unwrap();

    let region = Rect::new(0, 0, 432, H);
    let before = photocraft_compose::render(&s.active().unwrap().doc, region);
    assert_eq!(before.get(12, 16), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(before.get(412, 16), [1.0, 0.0, 0.0, 1.0]);
    convert(&mut s);
    let after = photocraft_compose::render(&s.active().unwrap().doc, region);
    assert_eq!(after.get(12, 16), before.get(12, 16));
    assert_eq!(after.get(412, 16), before.get(412, 16));
}

#[test]
fn conversion_ignores_hidden_off_canvas_descendants() {
    let mut s = shadow_fixture(8, 0, 180, 128);
    let visible = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().clone();
    let mut hidden = Layer::raster("far hidden", PixelFormat::RGBA8);
    hidden.visible = false;
    hidden.surface_mut().unwrap().fill_rect(Rect::new(100_000, 100_000, 100_008, 100_008), &[1.0, 0.0, 0.0, 1.0]);
    let mut clipped_to_hidden = Layer::raster("clipped far", PixelFormat::RGBA8);
    clipped_to_hidden.clipped = true;
    clipped_to_hidden.surface_mut().unwrap().fill_rect(Rect::new(120_000, 120_000, 120_008, 120_008), &[0.0, 1.0, 0.0, 1.0]);
    let group = Layer::group("with hidden", vec![visible, hidden, clipped_to_hidden]);
    let id = group.id;
    s.edit("group with hidden far layers", |doc, _| {
        doc.layers = vec![group].into();
        Ok(())
    })
    .unwrap();
    s.select_layer(id).unwrap();
    convert(&mut s);
    let sm = active_smart(&s);
    assert_eq!(sm.cache.as_ref().unwrap().content_bounds(), Rect::new(12, 16, 20, 24));
}

#[test]
fn conversion_ignores_clipped_off_canvas_children_of_hidden_base_in_styled_group() {
    let mut s = shadow_fixture(8, 0, 180, 128);
    let visible = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().clone();
    let mut hidden = Layer::raster("far hidden", PixelFormat::RGBA8);
    hidden.visible = false;
    hidden.surface_mut().unwrap().fill_rect(Rect::new(100_000, 100_000, 100_008, 100_008), &[1.0, 0.0, 0.0, 1.0]);
    let mut clipped_to_hidden = Layer::raster("clipped far", PixelFormat::RGBA8);
    clipped_to_hidden.clipped = true;
    clipped_to_hidden.surface_mut().unwrap().fill_rect(Rect::new(120_000, 120_000, 120_008, 120_008), &[0.0, 1.0, 0.0, 1.0]);
    let mut group = Layer::group("styled with hidden clip base", vec![visible, hidden, clipped_to_hidden]);
    group.effects.enabled = true;
    group.effects.items.push(Effect::default_drop_shadow());
    let id = group.id;
    s.edit("styled group with hidden clipped descendant", |doc, _| {
        doc.layers = vec![group].into();
        Ok(())
    })
    .unwrap();
    s.select_layer(id).unwrap();
    convert(&mut s);
    let bounds = active_smart(&s).cache.as_ref().unwrap().content_bounds();
    assert!(bounds.width() < 256 && bounds.height() < 256, "clipped child of hidden base must not expand styled group bounds: {bounds:?}");
}

#[test]
fn conversion_preserves_long_off_canvas_shadows_on_editable_text() {
    for depth in [8u64, 32u64] {
        let tolerance = if depth == 8 { 1.0 / 255.0 + 1e-6 } else { 1e-6 };
        for distance in [320u64, 400u64] {
            let mut s = crate::session();
            s.execute("file.new", json!({"width": 128, "height": 128, "depth": depth, "background": "transparent"})).unwrap();
            s.execute("type.create", json!({"text": "H", "size": 30, "x": 16, "y": 48})).unwrap();
            s.execute(
                "layer.layerStyle.dropShadow",
                json!({"color": "#ff0000", "opacity": 100, "blend": "normal", "useGlobalLight": false,
                    "angle": 180, "distance": distance, "size": 0, "spread": 0}),
            )
            .unwrap();

            let region = Rect::new(-64, 0, 640, 128);
            let shadow_region = Rect::new(300, 0, 640, 128);
            let before = s.active().unwrap().doc.clone();
            let expected = photocraft_compose::render(&before, region).into_px();
            let expected_shadow = photocraft_compose::render(&before, shadow_region).into_px();
            let red_pixels = expected_shadow.iter().filter(|p| p[0] > 0.9 && p[1] < 0.1 && p[3] > 0.5).count();
            assert!(red_pixels > 0, "direct editable-text shadow control must paint past the old 256 px margin: depth {depth}, distance {distance}");

            convert(&mut s);
            let actual = photocraft_compose::render(&s.active().unwrap().doc, region).into_px();
            let diff = max_diff(&actual, &expected);
            assert!(diff <= tolerance, "cropped editable-text shadow: depth {depth}, distance {distance}, diff {diff}, tolerance {tolerance}");
            let actual_shadow = photocraft_compose::render(&s.active().unwrap().doc, shadow_region).into_px();
            let actual_red = actual_shadow.iter().filter(|p| p[0] > 0.9 && p[1] < 0.1 && p[3] > 0.5).count();
            assert_eq!(actual_red, red_pixels, "off-canvas editable-text shadow pixel count must match: depth {depth}, distance {distance}");

            let sm = active_smart(&s);
            let SmartSource::Embedded { file_name, bytes } = &sm.source else { panic!("expected embedded contents") };
            let child = decode_source(file_name, bytes).unwrap();
            assert!(matches!(&child.layers[0].content, LayerContent::Text(t) if t.text == "H"), "conversion must keep the embedded Type layer editable");
            assert!(child.size.width > distance as u32, "embedded text canvas must hold the {distance} px shadow: {:?}", child.size);

            let after = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, after);

            let saved = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let reopen_diff = max_diff(&photocraft_compose::render(&reopened, region).px, &expected);
            assert!(reopen_diff <= tolerance, "native reopen retains the editable-text shadow: diff {reopen_diff}");

            s.execute("layer.smartObjects.editContents", json!({})).unwrap();
            s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
            s.set_active(0);
            let save_diff = max_diff(&photocraft_compose::render(&s.active().unwrap().doc, region).px, &expected);
            assert!(save_diff <= tolerance, "saving editable text Smart Object contents retains the shadow: diff {save_diff}");
        }
    }
}

#[test]
fn conversion_preserves_custom_cmyk_profile_across_source_cache_rebuild() {
    let params =
        craft_cms::synth::CmykParams { description: "Test Uncoated".into(), tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() };
    let profile = craft_cms::synth::cmyk_profile(&params).unwrap().to_bytes();

    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": "cmyk", "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("custom CMYK profile and layer", |doc, active| {
        doc.icc_profile = Some(profile.clone());
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[0.15, 0.72, 0.65, 0.08, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute(
        "layer.layerStyle.dropShadow",
        json!({"color": "#d05a33", "opacity": 100, "blend": "normal", "useGlobalLight": false,
            "angle": 180, "distance": 40, "size": 0, "spread": 0}),
    )
    .unwrap();

    convert(&mut s);
    let sm = active_smart(&s).clone();
    let initial_cache = sm.cache.as_ref().unwrap().clone();
    let SmartSource::Embedded { file_name, bytes } = &sm.source else { panic!("expected embedded contents") };
    let fmt = s.active().unwrap().doc.pixel_format();

    // Force a cold rebuild from the embedded .pcraft document (as after reopening or purging caches).
    super::super::purge_source_cache();
    let rebuilt = super::super::source::source_image_classified(file_name, bytes, fmt, false).unwrap();
    let child = decode_source(file_name, bytes).unwrap();
    assert_eq!(child.icc_profile.as_deref().map(Vec::as_slice), Some(profile.as_slice()), "embedded CMYK source must retain its ICC profile");

    // Translate the initial cache to source space and compare bit-for-bit against the rebuilt cache.
    let origin_x = sm.transform.m[4].round() as i32;
    let origin_y = sm.transform.m[5].round() as i32;
    let initial_in_source = crate::layer_multi_cmds::shift_surface(&initial_cache, -origin_x, -origin_y);
    assert_eq!(
        rebuilt.surface.read_region(rebuilt.bounds),
        initial_in_source.read_region(rebuilt.bounds),
        "cold source-cache rebuild must preserve the document's custom CMYK ICC profile"
    );

    // Confirm the custom profile actually changed the rebuilt CMYK separation compared to default coated CMYK.
    let default_encoded = super::super::source::buffer_to_surface(&photocraft_compose::flatten(&child), fmt);
    assert_ne!(rebuilt.surface.pixel(4, 4), default_encoded.pixel(4, 4), "rebuilt custom-profile CMYK pixels must differ from default coated CMYK");
}

#[test]
fn conversion_accepts_a_36_megapixel_rgb8_canvas() {
    let mut s = shadow_fixture(8, 400, 180, 128);
    s.edit("Paint a 36 MP layer", |doc, active| {
        doc.size = Size::new(6_000, 6_000);
        let bounds = doc.bounds();
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    convert(&mut s);
    let sm = active_smart(&s);
    let cache = sm.cache.as_ref().unwrap();
    assert_eq!(cache.pixel(12, 16), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(cache.pixel(6_399, 16), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn conversion_handles_sparse_layer_on_huge_canvas_and_rejects_oversized_fill() {
    // A small layer on a 40,000 x 40,000 canvas only needs its own bounds plus effect reach.
    let mut s = shadow_fixture(8, 0, 180, 128);
    s.edit("huge sparse canvas", |doc, _| {
        doc.size = Size::new(40_000, 40_000);
        Ok(())
    })
    .unwrap();
    convert(&mut s);
    assert_eq!(active_smart(&s).cache.as_ref().unwrap().content_bounds(), Rect::new(12, 16, 20, 24));

    // A full-canvas Fill layer on the same 40,000 x 40,000 canvas genuinely exceeds the limit and
    // must be rejected cleanly without modifying the document.
    let mut fill_session = crate::session();
    fill_session.execute("file.new", json!({"width": 128, "height": 128, "depth": 8})).unwrap();
    let fill = Layer::new("Huge Fill", LayerContent::Fill(Fill::Solid(Color::BLACK)));
    let fill_id = fill.id;
    fill_session
        .edit("huge fill canvas", |doc, _| {
            doc.size = Size::new(40_000, 40_000);
            doc.layers.push(fill);
            Ok(())
        })
        .unwrap();
    fill_session.select_layer(fill_id).unwrap();
    let before = fill_session.active().unwrap().doc.clone();
    let error = fill_session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap_err().to_string();
    assert!(error.contains("limit"), "conversion should identify its size limit: {error}");
    assert!(!error.contains("layer is empty"), "nonempty content was rejected for size");
    assert_eq!(fill_session.active().unwrap().doc, before);
}

#[test]
fn conversion_rejects_effects_above_the_renderer_build_limit() {
    let mut s = shadow_fixture(8, 400, 180, 128);
    s.edit("Oversized effect maps on sparse content", |doc, active| {
        doc.size = Size::new(6_000, 6_000);
        let layer = doc.layer_mut(active.unwrap()).unwrap();
        layer.advanced.transparency_shapes = false;
        let shadow = layer.effects.items.first().unwrap().clone();
        layer.effects.items.extend(std::iter::repeat_n(shadow, 24));
        // Dissolve requires the whole effect region even when local maps could be clipped.
        layer.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Dissolve, 1.0), color: Color::BLACK });
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    let error = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap_err().to_string();
    assert!(error.contains("effect maps exceed the renderer's working-memory limit"), "{error}");
    assert_eq!(s.active().unwrap().doc, before);
}
