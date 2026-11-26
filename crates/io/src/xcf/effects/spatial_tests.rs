use crate::native_xcf_tones::{
    AuthorizationError, enter_import,
    fixtures::point::{WireValue as W, fixture},
};
use photocraft_color::SampleType;
use photocraft_doc::{
    LayerContent,
    import_conversion::{ConversionDisposition, ImportConversion},
};
use photocraft_geom::Rect;
fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), AuthorizationError> {
    Ok(())
}
#[test]
fn spatial_records_install_native_filters_and_preserve_reopen_evidence() {
    let _scope = enter_import(allow);
    for (op, args) in [
        ("gegl:gaussian-blur", vec![("std-dev-x", W::Float(2.)), ("std-dev-y", W::Float(1.))]),
        ("gegl:box-blur", vec![("radius", W::Int(2))]),
        ("gegl:motion-blur-linear", vec![("length", W::Float(3.)), ("angle", W::Float(45.))]),
        ("gegl:unsharp-mask", vec![("std-dev", W::Float(2.)), ("scale", W::Float(1.5)), ("threshold", W::Float(0.25))]),
        ("gegl:high-pass", vec![("std-dev", W::Float(2.))]),
    ] {
        let bytes = fixture(op, &args, SampleType::F32, None);
        let imported = crate::import("spatial.xcf", &bytes).unwrap();
        let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("{op} not installed") };
        assert_eq!(smart.smart_filters.len(), 1);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("source report absent") };
        assert!(report.entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native filter lost") };
        assert_eq!(again.smart_filters, smart.smart_filters);
        assert_eq!(again.cache, smart.cache);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}
#[test]
fn spatial_invalid_unverified_and_unknown_policy_cases_preserve_source_pixels() {
    let _scope = enter_import(allow);
    for (op, args, refusal) in [
        ("gegl:gaussian-blur", vec![("std-dev-x", W::Float(0.)), ("std-dev-y", W::Float(0.))], None),
        ("gegl:gaussian-blur", vec![("filter", W::Enum(99))], None),
        ("gegl:gaussian-blur", vec![("clip-extent", W::Enum(0))], None),
        ("gegl:box-blur", vec![("radius", W::Float(1.5))], None),
        ("gegl:motion-blur-linear", vec![("length", W::Float(0.))], None),
        ("gegl:unsharp-mask", vec![("scale", W::Float(0.))], None),
        ("gegl:high-pass", vec![("contrast", W::Float(0.5))], None),
        ("gegl:gaussian-blur", vec![], Some("policy")),
        ("gegl:gaussian-blur", vec![], Some("mask")),
        ("gegl:gaussian-blur", vec![], Some("opacity")),
    ] {
        let bytes = fixture(op, &args, SampleType::F32, refusal);
        let imported = crate::import("unapplied.xcf", &bytes).unwrap();
        let LayerContent::Raster(actual) = &imported.document.layers[0].content else { panic!("unverified {op} activated") };
        assert_eq!(actual.read_region(Rect::from_xywh(-1, 1, 2, 1)), [-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.]);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("fallback absent") };
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
    }
}
