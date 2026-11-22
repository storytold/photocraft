//! Pre-cutover baseline pins for `filter.cameraRaw` (pixel layer, smart filter, alpha, identity; #587 / #778).

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use serde_json::json;

fn active_pixel(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    layer.surface().unwrap().rgba(x, y)
}

#[test]
fn camera_raw_filter_preserves_identity_alpha_and_smart_filter_parity() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#4080c0", "opacity": 50.0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    let before = active_pixel(&s, 8, 8);
    assert!((before[3] - 0.5).abs() < 0.02, "expected ~50% alpha: {before:?}");

    // Identity filter leaves non-RAW pixels bit-identical.
    s.execute("filter.cameraRaw", json!({})).unwrap();
    assert_eq!(active_pixel(&s, 8, 8), before);

    // Non-trivial filter preserves alpha and matches Smart Filter rendering.
    let params = json!({
        "temperature": 15.0,
        "tint": -10.0,
        "exposure": 0.5,
        "contrast": 20.0,
        "highlights": -25.0,
        "shadows": 30.0,
        "vibrance": 15.0,
    });

    let mut smart_session = Session::new();
    smart_session.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    smart_session.execute("layer.new.layer", json!({})).unwrap();
    smart_session.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16})).unwrap();
    smart_session.execute("edit.fill", json!({"contents": "color", "color": "#4080c0", "opacity": 50.0})).unwrap();
    smart_session.execute("select.deselect", json!({})).unwrap();
    smart_session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();

    s.execute("filter.cameraRaw", params.clone()).unwrap();
    smart_session.execute("filter.cameraRaw", params).unwrap();

    let direct = active_pixel(&s, 8, 8);
    assert!((direct[3] - before[3]).abs() < 1e-4, "alpha must be preserved: {before:?} -> {direct:?}");
    assert_ne!(direct[0..3], before[0..3], "RGB must change under non-default settings");

    let smart_st = smart_session.active().unwrap();
    let smart_layer = smart_st.active_layer.and_then(|id| smart_st.doc.layer(id)).unwrap();
    let LayerContent::Smart(so) = &smart_layer.content else {
        panic!("expected smart object");
    };
    assert_eq!(so.smart_filters.len(), 1);
    assert_eq!(so.smart_filters[0].command, "filter.cameraRaw");
    let rendered = smart_layer.surface().unwrap().rgba(8, 8);
    for c in 0..4 {
        assert!((rendered[c] - direct[c]).abs() < 1e-3, "channel {c}: smart {rendered:?} vs direct {direct:?}");
    }
}

#[test]
fn camera_raw_filter_legacy_baseline_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#4080c0"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#c06020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    s.execute(
        "filter.cameraRaw",
        json!({
            "temperature": 15.0,
            "tint": -10.0,
            "exposure": 0.5,
            "contrast": 20.0,
            "highlights": -25.0,
            "shadows": 30.0,
            "whites": 10.0,
            "blacks": -10.0,
            "vibrance": 15.0,
            "saturation": -5.0,
        }),
    )
    .unwrap();

    let quantize = |px: [f32; 4]| px.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16);
    let got = [quantize(active_pixel(&s, 2, 2)), quantize(active_pixel(&s, 8, 8)), quantize(active_pixel(&s, 13, 13))];
    assert_eq!(got, [[19650, 39061, 56622, 65535], [57965, 29220, 10084, 65535], [57965, 29220, 10084, 65535]]);
}

#[test]
fn camera_raw_smart_filter_migrates_to_shared_develop_settings() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "prophoto-compat"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute(
        "filter.cameraRaw",
        json!({
            "temperature": 18.0,
            "tint": -12.0,
            "exposure": 0.75,
            "contrast": 22.0,
            "highlights": -35.0,
            "shadows": 40.0,
            "whites": 15.0,
            "blacks": -10.0,
            "texture": 12.0,
            "clarity": 16.0,
            "dehaze": 8.0,
            "vibrance": 20.0,
            "saturation": -5.0,
            "curveSplits": [25.0, 50.0, 75.0],
            "pointCurve": [[0.0, 0.0], [64.0, 52.0], [255.0, 255.0]],
            "hslHue": [5.0, -10.0, 0.0, 12.0, -4.0, 8.0, 0.0, -3.0],
            "gradeShadows": {"hue": 210.0, "sat": 25.0, "lum": -5.0},
            "vignetteAmount": -20.0,
            "vignetteStyle": "highlightPriority",
            "shadowTint": -6.0,
            "calibrationHue": [4.0, -3.0, 2.0]
        }),
    )
    .unwrap();

    let st = s.active().unwrap();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let LayerContent::Smart(so) = &layer.content else {
        panic!("expected smart object");
    };
    assert_eq!(so.smart_filters[0].params["encodingGamma"], json!(1.8));
    let base = craft_develop_model::DevelopSettings::default();
    let migrated = photocraft_algo::camera_raw::migrate_to_develop_settings(&base, &so.smart_filters[0].params).unwrap();
    assert!(migrated.unmapped_fields.is_empty(), "unexpected unmapped fields: {:?}", migrated.unmapped_fields);
    assert_eq!(migrated.settings.process, base.process);
    assert_eq!(migrated.settings.light.exposure, 0.75);
    assert_eq!(migrated.settings.light.contrast, 22.0);
    assert_eq!(migrated.settings.wb.tint, -12.0);
    assert_eq!(migrated.settings.vignette.style, craft_develop_model::VignetteStyle::HighlightPriority);
}

#[test]
fn camera_raw_migration_preserves_exposure_presence_after_native_reopen() {
    use std::sync::Arc;

    use craft_develop_model::{DevelopSettings, Process};
    use photocraft_color::{BlendMode, ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, Size, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::Affine;

    // Original synthetic content. Exercise the native serializer, not the PSD importer.
    let size = Size::new(2, 2);
    let mut source = Session::new();
    source.execute("file.new", json!({"width": 2, "height": 2, "depth": 16})).unwrap();
    let embedded = Arc::new(photocraft_format::save_to_bytes(&source.active().unwrap().doc, &Default::default()).unwrap());
    let mut base = DevelopSettings { process: Process::V8.version(), ..Default::default() };
    base.light.exposure = -0.75;
    base.light.contrast = 12.0;
    for (params, expected) in [(json!({"encodingGamma": 1.8}), -0.75), (json!({"encodingGamma": 1.8, "exposure": 0.0}), 0.0)] {
        let source = SmartSource::Embedded { file_name: "generated.pcraft".into(), bytes: embedded.clone() };
        let mut smart = SmartObject::new(source, Affine::IDENTITY, None);
        smart.smart_filters.push(SmartFilter {
            command: "filter.cameraRaw".into(),
            params: params.clone(),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        });
        let mut doc = Document::new("Saved Camera Raw exposure", size, ColorMode::Rgb, SampleType::U16);
        doc.layers.push(Layer::new("Stored filter", LayerContent::Smart(smart)));

        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("expected reopened Smart Object") };
        let [filter] = smart.smart_filters.as_slice() else { panic!("expected one stored Camera Raw filter") };
        assert_eq!(filter.command, "filter.cameraRaw");
        assert_eq!(filter.params, params);
        let migrated = photocraft_algo::camera_raw::migrate_to_develop_settings(&base, &filter.params).unwrap();
        assert!(migrated.unmapped_fields.is_empty(), "{:?}", migrated.unmapped_fields);
        assert_eq!(migrated.settings.process, base.process);
        assert_eq!(migrated.settings.light.exposure, expected);
        assert_eq!(migrated.settings.light.contrast, base.light.contrast);
    }
}

#[test]
fn reopened_camera_raw_smart_filter_keeps_legacy_close_curve_knots() {
    use std::sync::Arc;

    use photocraft_color::{BlendMode, ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, Rect, Size, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::Affine;
    use photocraft_raster::Surface;

    // Original synthetic neutral ramp, saved as real embedded PhotoCraft content.
    let size = Size::new(5, 3);
    let mut inner = Document::new("Gray ramp", size, ColorMode::Rgb, SampleType::U16);
    let mut ramp = Surface::new(inner.pixel_format());
    for (x, gray) in (0..5).zip([0.125, 0.25, 0.75, 0.875, 1.0]) {
        ramp.fill_rect(Rect::new(x, 0, x + 1, 3), &[gray, gray, gray, 1.0]);
    }
    inner.layers.push(Layer::new("Ramp", LayerContent::Raster(ramp)));
    let original = photocraft_format::save_to_bytes(&inner, &Default::default()).unwrap();
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "ramp.pcraft".into(), bytes: Arc::new(original) }, Affine::IDENTITY, None);
    // The modern command rejects this small gap. Store the historical settings as a
    // Smart Filter and drive production's lenient replay, not the new-edit validator.
    let near = f32::from_bits(127.5_f32.to_bits() + 33);
    let curve = [[0.0, 0.0], [127.5, 0.0], [near, 255.0], [255.0, 255.0]];
    let params = json!({"pointCurve": curve});
    smart.smart_filters.push(SmartFilter { command: "filter.cameraRaw".into(), params: params.clone(), blend: BlendMode::Normal, opacity: 1.0, visible: true });
    let mut document = Document::new("Stored Camera Raw curve", size, ColorMode::Rgb, SampleType::U16);
    document.layers.push(Layer::new("Stored filter", LayerContent::Smart(smart)));

    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("expected reopened Smart Object") };
    let [filter] = smart.smart_filters.as_slice() else { panic!("expected one stored Camera Raw filter") };
    assert_eq!(filter.params, params);
    let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("expected embedded ramp") };
    let placed = photocraft_format::load_from_bytes(bytes).unwrap();
    let LayerContent::Raster(ramp) = &placed.layers[0].content else { panic!("expected decoded ramp pixels") };
    assert!((ramp.rgba(2, 1)[0] - 0.75).abs() < 0.01);

    // Identity placement: the embedded raster is already the placed input. This
    // production entry point replays the stored stack without using layer cache pixels.
    let rendered = photocraft_engine::smart_cmds::apply_smart_filters(ramp, smart, reopened.bounds()).unwrap();
    let bright = rendered.rgba(2, 1);
    assert!(bright[..3].iter().all(|&v| v > 0.9), "legacy close-knot step must brighten the upper midtone: {bright:?}");
    let dark = rendered.rgba(1, 1);
    assert!(dark[..3].iter().all(|&v| v < 0.1), "the lower midtone remains below the step: {dark:?}");
}
