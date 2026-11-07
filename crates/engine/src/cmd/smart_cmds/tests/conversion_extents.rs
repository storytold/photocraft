use super::*;
use photocraft_color::Color;
use photocraft_doc::{Effect, FxCommon, FxPaint, Knockout, LayerMask, StrokeFx, StrokePosition};

fn shadow_fixture(depth: u64, distance: u64, angle: u64, width: i32) -> Session {
    let mut s = Session::new();
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
fn conversion_review_regression_preserves_transparent_shape_strokes() {
    for depth in DEPTHS {
        let mut s = Session::new();
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
fn conversion_review_regression_reports_size_limit_without_changing_document() {
    let mut s = shadow_fixture(8, 0, 180, 128);
    s.edit("huge sparse canvas", |doc, _| {
        doc.size = Size::new(40_000, 40_000);
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    let error = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap_err().to_string();
    assert!(error.contains("limit"), "conversion should identify its size limit: {error}");
    assert!(!error.contains("layer is empty"), "nonempty content was rejected for size");
    assert_eq!(s.active().unwrap().doc, before);
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

#[test]
fn conversion_rejects_nested_effect_working_memory_before_rendering() {
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
        doc.layers = vec![layer];
        Ok(())
    })
    .unwrap();
    s.select_layer(id).unwrap();
    let before = s.active().unwrap().doc.clone();
    let error = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap_err().to_string();
    assert!(error.contains("working-memory limit"), "nested effects must be rejected before allocating their expanded composite: {error}");
    assert_eq!(s.active().unwrap().doc, before);
}
