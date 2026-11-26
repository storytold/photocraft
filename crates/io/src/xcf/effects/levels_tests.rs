use crate::native_xcf_tones::{AuthorizationError, enter_import, fixtures::levels::fixture};
use photocraft_color::SampleType;
use photocraft_doc::{
    LayerContent,
    import_conversion::{ConversionDisposition, ConversionReason, ImportConversion},
};
use photocraft_geom::Rect;
fn allow(_: &str, _: &serde_json::Value) -> Result<(), AuthorizationError> {
    Ok(())
}

#[test]
fn levels_installs_editable_native_data_and_durable_approximation_evidence() {
    let _scope = enter_import(allow);
    let depth = SampleType::F32;
    let source = fixture(depth, None);
    let imported = crate::import("levels.xcf", &source).unwrap();
    let layer = &imported.document.layers[0];
    let LayerContent::Smart(smart) = &layer.content else { panic!("native Levels missing") };
    assert_eq!(smart.smart_filters.len(), 1);
    assert_eq!(smart.smart_filters[0].command, "image.adjustments.levels");
    assert_eq!(smart.transform, photocraft_doc::Affine::translate(-1., 1.));
    assert_eq!(layer.opacity, 0.75);
    assert!(smart.filter_mask.is_none());
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("source report absent") };
    let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap();
    assert_eq!(entry.disposition, ConversionDisposition::NativeApproximation);
    assert!(!entry.attachments.is_empty());
    assert!(entry.native_description.contains("native"));
    assert!(imported.warnings.iter().any(|s| s.contains("integer quantization may differ")));
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native Levels lost") };
    assert_eq!(again.smart_filters, smart.smart_filters);
    assert_eq!(again.cache, smart.cache);
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
}

#[test]
fn levels_refusals_preserve_original_raster_and_source_evidence() {
    for refusal in ["policy", "narrow", "nonfinite", "mask", "opacity", "version"] {
        let source = fixture(SampleType::F32, Some(refusal));
        let inert = crate::import("inert.xcf", &source).unwrap();
        let _scope = enter_import(allow);
        let imported = crate::import("unapplied.xcf", &source).unwrap();
        let LayerContent::Raster(actual) = &imported.document.layers[0].content else { panic!("unproven Levels activated: {refusal}") };
        let area = Rect::from_xywh(-1, 1, 2, 1);
        assert_eq!(actual.read_region(area), [-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.]);
        assert_eq!(actual, inert.document.layers[0].surface().unwrap());
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("fallback disclosure absent") };
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
        assert!(report.entries.iter().any(|entry| matches!(entry.reason, ConversionReason::NativeLimitation | ConversionReason::InvalidOptionalData)));
        assert!(imported.warnings.iter().any(|s| s.contains("not applied")));
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        assert_eq!(reopened.layers[0].surface(), Some(actual));
    }
}

#[test]
fn integer_levels_keep_original_samples_without_unpriced_lut_generation() {
    let _scope = enter_import(allow);
    for (depth, expected) in [
        (SampleType::U8, vec![32, 96, 160, 128, 192, 128, 64, 255]),
        (SampleType::U16, [8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535].into_iter().flat_map(u16::to_le_bytes).collect()),
    ] {
        let source = fixture(depth, None);
        let imported = crate::import("integer-levels.xcf", &source).unwrap();
        let LayerContent::Raster(surface) = &imported.document.layers[0].content else { panic!("integer Levels activated without LUT admission") };
        assert_eq!(surface.to_interleaved(Rect::from_xywh(-1, 1, 2, 1)), expected);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else {
            panic!("unapplied integer evidence absent")
        };
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
        assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::NativeLimitation));
    }
}
