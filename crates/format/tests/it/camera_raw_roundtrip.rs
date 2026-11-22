//! Pins `.pcraft` serialization of `filter.cameraRaw` Smart Filters (pre-cutover baseline, #587 / #778).

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Size, SmartContentsId, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::Affine;

#[test]
fn pcraft_roundtrips_camera_raw_smart_filter_params() {
    let mut doc = Document::new("CameraRawPin", Size::new(16, 16), ColorMode::Rgb, SampleType::U16);
    let params: serde_json::Value = serde_json::from_str(
        r#"{
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
        "curveShadows": 10.0,
        "curveDarks": -8.0,
        "curveLights": 12.0,
        "curveHighlights": -6.0,
        "curveSplits": [25.0, 50.0, 75.0],
        "pointCurve": [[0.0, 0.0], [64.0, 52.0], [192.0, 204.0], [255.0, 255.0]],
        "hslHue": [5.0, -10.0, 0.0, 12.0, -4.0, 8.0, 0.0, -3.0],
        "hslSat": [10.0, 5.0, -8.0, 0.0, 6.0, -12.0, 4.0, 0.0],
        "hslLum": [-4.0, 8.0, 2.0, -6.0, 0.0, 10.0, -2.0, 3.0],
        "gradeShadows": {"hue": 210.0, "sat": 25.0, "lum": -5.0},
        "gradeMidtones": {"hue": 45.0, "sat": 10.0, "lum": 0.0},
        "gradeHighlights": {"hue": 35.0, "sat": 20.0, "lum": 4.0},
        "gradeGlobal": {"hue": 0.0, "sat": 0.0, "lum": 2.0},
        "gradeBlending": 60.0,
        "gradeBalance": -15.0,
        "sharpenAmount": 40.0,
        "sharpenRadius": 1.2,
        "sharpenDetail": 30.0,
        "sharpenMasking": 20.0,
        "noiseLuminance": 15.0,
        "noiseLuminanceDetail": 50.0,
        "noiseColor": 25.0,
        "noiseColorDetail": 50.0,
        "grainAmount": 12.0,
        "grainSize": 25.0,
        "grainRoughness": 50.0,
        "vignetteAmount": -20.0,
        "vignetteMidpoint": 45.0,
        "vignetteFeather": 55.0,
        "vignetteRoundness": 10.0,
        "vignetteHighlights": 15.0,
        "vignetteStyle": "highlightPriority",
        "shadowTint": -6.0,
        "calibrationHue": [4.0, -3.0, 2.0],
        "calibrationSat": [8.0, -5.0, 6.0],
        "encodingGamma": 1.8
    }"#,
    )
    .unwrap();
    let smart = Layer::new(
        "RawSmart",
        LayerContent::Smart(SmartObject {
            contents_id: SmartContentsId::fresh(),
            vector_preview: None,
            source: SmartSource::Embedded { file_name: "inner.png".into(), bytes: std::sync::Arc::new(vec![1, 2, 3, 4]) },
            transform: Affine::IDENTITY,
            smart_filters: vec![SmartFilter {
                command: "filter.cameraRaw".into(),
                params: params.clone(),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible: true,
            }],
            cache: None,
            psd_raw: None,
            filters_enabled: true,
            filter_mask: None,
            warp: None,
            stack_mode: None,
            perspective: None,
        }),
    );
    doc.layers.push(smart);

    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
    let LayerContent::Smart(so) = &back.layers[0].content else {
        panic!("expected SmartObject after .pcraft roundtrip");
    };
    assert_eq!(so.smart_filters.len(), 1);
    assert_eq!(so.smart_filters[0].command, "filter.cameraRaw");
    assert_eq!(so.smart_filters[0].params, params);
}
