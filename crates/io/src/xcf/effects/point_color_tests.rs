use super::*;
use crate::native_xcf_tones::{
    AuthorizationError, enter_import,
    fixtures::point::{WireValue, fixture},
};
use photocraft_color::SampleType;
use photocraft_doc::{
    LayerContent,
    import_conversion::{ConversionDisposition, ConversionReason, ImportConversion},
};
use photocraft_geom::Rect;
fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), AuthorizationError> {
    Ok(())
}

#[test]
fn scalar_color_records_install_native_controls_and_persist_evidence() {
    let _scope = enter_import(allow);
    for (op, args) in [
        ("gegl:invert-linear", vec![]),
        ("gimp:posterize", vec![("levels", WireValue::Int(8))]),
        ("gegl:vibrance", vec![("vibrance", WireValue::Float(25.)), ("saturation", WireValue::Float(3.))]),
        ("gegl:saturation", vec![("scale", WireValue::Float(2.)), ("colorspace", WireValue::Enum(0))]),
        ("gegl:mono-mixer", vec![("red", WireValue::Float(0.5)), ("green", WireValue::Float(0.25)), ("blue", WireValue::Float(0.25))]),
        ("gegl:channel-mixer", vec![("rr-gain", WireValue::Float(0.5)), ("rg-gain", WireValue::Float(0.5))]),
        ("gegl:sepia", vec![("scale", WireValue::Float(0.5)), ("srgb", WireValue::Bool(false))]),
    ] {
        let bytes = fixture(op, &args, SampleType::F32, None);
        let imported = crate::import("color.xcf", &bytes).unwrap();
        let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("{op} not installed") };
        assert!(!smart.smart_filters.is_empty());
        assert!(smart.smart_filters.len() <= 4);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("missing native policy") };
        assert!(report.entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
        assert!(!report.attachments.is_empty());
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("controls lost") };
        assert_eq!(again.smart_filters, smart.smart_filters);
        assert_eq!(again.cache, smart.cache);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}
#[test]
fn unsafe_or_unverified_scalar_color_inputs_preserve_original_pixels() {
    let _scope = enter_import(allow);
    for (op, args, refusal) in [
        ("gimp:posterize", vec![("levels", WireValue::Int(256))], None),
        ("gimp:posterize", vec![("levels", WireValue::Float(8.5))], None),
        ("gegl:vibrance", vec![("vibrance", WireValue::Float(f32::NAN))], None),
        ("gegl:saturation", vec![("colorspace", WireValue::Enum(99))], None),
        ("gegl:saturation", vec![("colorspace", WireValue::Bool(false))], None),
        ("gegl:mono-mixer", vec![("preserve-luminosity", WireValue::Bool(true))], None),
        ("gegl:channel-mixer", vec![("rr-gain", WireValue::Float(3.))], None),
        ("gegl:sepia", vec![("srgb", WireValue::Int(0))], None),
        ("gegl:vibrance", vec![("vibrance", WireValue::Float(1.)), ("vibrance", WireValue::Float(2.))], None),
        ("gegl:invert-linear", vec![], Some("policy")),
        ("gegl:invert-linear", vec![], Some("opacity")),
        ("gegl:invert-linear", vec![], Some("mask")),
        ("gegl:invert-linear", vec![], Some("version")),
    ] {
        let bytes = fixture(op, &args, SampleType::F32, refusal);
        let imported = crate::import("unapplied.xcf", &bytes).unwrap();
        let LayerContent::Raster(actual) = &imported.document.layers[0].content else { panic!("unproven {op} activated") };
        assert_eq!(actual.read_region(Rect::from_xywh(-1, 1, 2, 1)), [-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.]);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("fallback missing") };
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
        assert!(report.entries.iter().any(|entry| matches!(entry.reason, ConversionReason::NativeLimitation | ConversionReason::InvalidOptionalData)));
    }
}
#[test]
fn point_color_cancelled_mapping_never_generates_native_data() {
    let bytes = fixture("gegl:sepia", &[], SampleType::F32, None);
    let xcf = craft_xcf::Xcf::parse(&bytes).unwrap();
    let mut budget = craft_xcf::effects::Budget::default();
    let list = xcf.layer_effects(0, &mut budget, &|| true).unwrap();
    let source = craft_xcf::effects::Reader::new(&bytes, 26).read(list.owner, list.offsets[0], &mut budget, &|| true).unwrap();
    assert!(matches!(map(&source, &Interrupt::cancel_only(&|| true)), Err(Failure::Cancelled)));
}
