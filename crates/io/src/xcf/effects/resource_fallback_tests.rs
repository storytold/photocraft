//! Former 6339 resource mappings must stay inert until resource generation has
//! an explicit exact-parameter authority and transactional retention contract.
use crate::native_xcf_tones::{
    AuthorizationError, enter_import,
    fixtures::point::{WireValue as W, fixture},
    preparation_count,
};
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
fn unadmitted_color_resources_keep_original_record_pixels_and_native_reopen_evidence() {
    let _scope = enter_import(allow);
    for (operation, args) in [
        ("gegl:hue-chroma", vec![("hue", W::Float(30.)), ("chroma", W::Float(10.)), ("lightness", W::Float(5.))]),
        ("gegl:color-temperature", vec![("original-temperature", W::Float(5000.)), ("intended-temperature", W::Float(6500.))]),
    ] {
        let bytes = fixture(operation, &args, SampleType::F32, None);
        let xcf = craft_xcf::Xcf::parse(&bytes).unwrap();
        let mut budget = craft_xcf::effects::Budget::default();
        let inventory = xcf.layer_effects(0, &mut budget, &|| true).unwrap();
        let source = craft_xcf::effects::Reader::new(&bytes, 26).read(inventory.owner, inventory.offsets[0], &mut budget, &|| true).unwrap();
        let before = preparation_count();
        let imported = crate::import("resource.xcf", &bytes).unwrap();
        assert_eq!(preparation_count(), before, "unadmitted resources must not enter native generation");
        let LayerContent::Raster(surface) = &imported.document.layers[0].content else { panic!("resource was silently installed") };
        assert_eq!(surface.read_region(Rect::from_xywh(-1, 1, 2, 1)), [-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.]);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("resource fallback undisclosed") };
        let entry = report.entries.iter().find(|e| e.feature_code == "xcf.layer_effect").unwrap();
        assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(entry.reason, ConversionReason::MissingMapping);
        assert_eq!(entry.source_description, operation);
        let attachment = &report.attachments[entry.attachments[0] as usize];
        assert_eq!(attachment.bytes.as_slice(), source.record.raw);
        assert!(imported.warnings.iter().any(|w| w.contains("not applied")));
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.layers[0].surface(), Some(surface));
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}
