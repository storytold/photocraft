//! Independently framed source records, consumed through the public importer.
use super::*;
use crate::xcf::tests::{W, hierarchy};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, SmartSource,
    import_conversion::{ConversionDisposition, ConversionReason, ImportConversion},
};
use photocraft_geom::Size;
use photocraft_raster::Surface;

fn fixture(refusal: Option<&str>, stage_mask: bool) -> Vec<u8> {
    let mut w = W::new(26);
    for value in [3, 3, 0, 600] {
        w.u32(value);
    } // linear F32 RGB
    w.prop(17, &[0]);
    w.end();
    let layer_slot = w.slot();
    for _ in 0..3 {
        w.ptr(0);
    }
    w.patch(layer_slot, w.out.len());
    for value in [2, 1, 1] {
        w.u32(value);
    }
    w.string("Tone owner");
    w.prop(15, &[(-1i32).to_be_bytes(), 1i32.to_be_bytes()].concat());
    w.prop(33, &0.75f32.to_be_bytes());
    w.prop_u32(8, 1);
    w.prop_u32(7, 28);
    w.end();
    let pixels_slot = w.slot();
    let mask_slot = w.slot();
    let contrast_slot = w.slot();
    let exposure_slot = w.slot();
    w.ptr(0);
    let raw = [-0.25f32, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>();
    let pixels = hierarchy(&mut w, 2, 1, 16, &raw, craft_xcf::Compression::None);
    w.patch(pixels_slot, pixels);
    w.patch(mask_slot, w.out.len());
    w.u32(2);
    w.u32(1);
    w.string("Outer mask");
    w.end();
    let mask_pixels = w.slot();
    let mask = hierarchy(&mut w, 2, 1, 4, &[0.5f32.to_be_bytes(), 1f32.to_be_bytes()].concat(), craft_xcf::Compression::None);
    w.patch(mask_pixels, mask);
    fn effect(w: &mut W, operation: &str, version: &str, args: &[(&str, f32)], mask: bool) {
        for text in ["Tone stage", "", operation, version] {
            w.string(text);
        }
        for (name, value) in args {
            let mut argument = W { out: Vec::new(), wide: true };
            argument.string(name);
            argument.u32(3);
            argument.out.extend_from_slice(&value.to_be_bytes());
            w.prop(45, &argument.out);
        }
        w.prop_u32(7, 62);
        w.prop_u32(8, 1);
        w.prop(33, &1f32.to_be_bytes());
        w.end();
        w.ptr(if mask { 8 } else { 0 });
    }
    w.patch(contrast_slot, w.out.len());
    effect(&mut w, "gegl:brightness-contrast", "0:0", &[("brightness", 0.25), ("contrast", 0.5)], stage_mask);
    w.patch(exposure_slot, w.out.len());
    let operation = if refusal == Some("invalid-value") { "gegl:exposure" } else { refusal.unwrap_or("gegl:exposure") };
    let exposure = if refusal == Some("invalid-value") { f32::NAN } else { 1. };
    effect(&mut w, operation, "1:0", &[("black-level", 0.05), ("exposure", exposure)], false);
    w.out
}
fn report(document: &Document) -> &photocraft_doc::import_conversion::ConversionReport {
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("conversion evidence missing") };
    report
}

#[test]
fn complete_tone_sequence_installs_real_native_data_and_reopens_with_source_evidence() {
    fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
        Ok(())
    }
    let _scope = crate::native_xcf_tones::enter_import(allow);
    let bytes = fixture(None, false);
    let original = bytes.clone();
    let imported = crate::import("native-tone.xcf", &bytes).unwrap();
    let layer = &imported.document.layers[0];
    let LayerContent::Smart(smart) = &layer.content else { panic!("complete tone sequence was not installed") };
    assert_eq!(smart.transform, photocraft_doc::Affine::translate(-1., 1.));
    assert_eq!(
        smart.smart_filters.iter().map(|filter| filter.command.as_str()).collect::<Vec<_>>(),
        ["image.adjustments.exposure", "image.adjustments.brightnessContrast", "image.adjustments.brightnessContrast"]
    );
    let cache = smart.cache.as_ref().unwrap();
    // Top-first source stack executes Exposure before legacy gain and additive
    // brightness. Native F32 legacy contrast has its documented127/255 pivot.
    for (x, input) in [(-1, [-0.25, 0.25, 2., 0.5]), (0, [0.75, 1.5, -0.5, 1.])] {
        let got = cache.rgba(x, 1);
        for channel in 0..3 {
            let expected = (input[channel] * 2. - 0.1 - 127. / 255.) * 0.5 + 127. / 255. + 0.25;
            assert!((got[channel] - expected).abs() < 0.000001, "{got:?} vs {expected}");
        }
        assert_eq!(got[3], input[3]);
    }
    assert_eq!(layer.opacity, 0.75);
    assert!(layer.mask.is_some());
    let evidence = report(&imported.document);
    assert_eq!(evidence.entries.len(), 2);
    assert_eq!(evidence.attachments.len(), 2);
    assert!(
        evidence.entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation && entry.reason == ConversionReason::NativePolicy)
    );
    assert_eq!(evidence.entries[0].source.ordinal, 0);
    assert_eq!(evidence.entries[0].source_description, "gegl:brightness-contrast");
    assert_eq!(evidence.entries[1].source_description, "gegl:exposure");
    assert!(evidence.entries.iter().all(|entry| entry.targets[0].layer_id == Some(layer.id)));
    assert!(imported.warnings.iter().any(|warning| warning.contains("editable native tone")));
    assert!(!imported.warnings.iter().any(|warning| warning.contains("not applied")));
    let SmartSource::Embedded { bytes: source, .. } = &smart.source else { panic!("native source missing") };
    let contents = photocraft_format::load_from_bytes(source).unwrap();
    assert_eq!(contents.layers[0].surface().unwrap().rgba(0, 0), [-0.25, 0.25, 2., 0.5]);
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    assert_eq!(reopened.layers[0].mask, layer.mask);
    let LayerContent::Smart(reopened_smart) = &reopened.layers[0].content else { panic!("native controls missing after reopen") };
    assert_eq!(reopened_smart.smart_filters, smart.smart_filters);
    assert_eq!(reopened_smart.cache, smart.cache);
    assert_eq!(bytes, original);
}

#[test]
fn one_unsupported_stage_or_unresolved_mask_preserves_the_whole_raster_owner() {
    fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
        Ok(())
    }
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for (unknown, mask) in [(Some("example:unknown-tone"), false), (Some("invalid-value"), false), (None, true)] {
        let bytes = fixture(unknown, mask);
        let imported = crate::import("retained-tones.xcf", &bytes).unwrap();
        let layer = &imported.document.layers[0];
        let source = layer.surface().expect("partial sequences must preserve raster owner");
        assert_eq!(source.rgba(-1, 1), [-0.25, 0.25, 2., 0.5]);
        assert!(layer.mask.is_some());
        assert_eq!(layer.opacity, 0.75);
        let evidence = report(&imported.document);
        assert!(evidence.entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
        assert!(evidence.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
        if unknown == Some("invalid-value") {
            assert!(evidence.entries.iter().any(|entry| entry.reason == ConversionReason::InvalidOptionalData));
        }
        assert!(imported.warnings.iter().any(|warning| warning.contains("2 layer effect(s)")));
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.layers[0].surface(), layer.surface());
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}

#[test]
fn native_admission_refusal_keeps_owner_and_marks_no_installed_entry() {
    fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
        Ok(())
    }
    let _scope = crate::native_xcf_tones::enter_import(allow);
    let bytes = fixture(None, false);
    let xcf = craft_xcf::Xcf::parse(&bytes).unwrap();
    let mut document = Document::new("Native context", Size::new(3, 3), ColorMode::Rgb, SampleType::F32);
    let mut pixels = Surface::new(document.pixel_format());
    pixels.write_pixel(-1, 1, &[-0.25, 0.25, 2., 0.5]);
    let mut owner = Layer::new("Unchanged", LayerContent::Raster(pixels));
    let original = owner.clone();
    let mut cancelled = crate::xcf::report::Report::new(&bytes, 26).with_native_context(&document);
    let mut warnings = crate::ora::Warnings::default();
    let mut retained = 1;
    let mut source_budget = crate::xcf::MAX_PIXEL_BYTES;
    let mut metadata_budget = craft_xcf::records::Budget::default();
    assert!(matches!(
        cancelled.layer_effects(
            &xcf,
            0,
            &mut owner,
            &mut crate::xcf::effects::coverage::Admission {
                retained: &mut retained,
                source: &mut source_budget,
                metadata: &mut metadata_budget,
                options: Default::default()
            },
            &mut warnings,
            &Interrupt::cancel_only(&|| true)
        ),
        Err(crate::IoError::Cancelled)
    ));
    assert_eq!(owner, original);
    assert_eq!(retained, 1);
    let mut reporter = crate::xcf::report::Report::new(&bytes, 26).with_native_context(&document);
    reporter
        .layer_effects(
            &xcf,
            0,
            &mut owner,
            &mut crate::xcf::effects::coverage::Admission {
                retained: &mut retained,
                source: &mut source_budget,
                metadata: &mut metadata_budget,
                options: Default::default(),
            },
            &mut warnings,
            &Interrupt::NONE,
        )
        .unwrap();
    assert_eq!(owner, original);
    assert_eq!(retained, 1);
    reporter.attach(&mut document, &mut warnings);
    assert!(
        report(&document)
            .entries
            .iter()
            .all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied && entry.reason == ConversionReason::RetentionLimit)
    );
}

#[test]
fn direct_io_is_inert_and_a_late_denied_command_prevents_all_materialization() {
    let bytes = fixture(None, false);
    let before = crate::native_xcf_tones::preparation_count();
    let direct = crate::import("inert.xcf", &bytes).unwrap();
    assert!(matches!(direct.document.layers[0].content, LayerContent::Raster(_)));
    assert!(report(&direct.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    assert_eq!(crate::native_xcf_tones::preparation_count(), before);
    fn deny_second(command: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
        if command == "image.adjustments.brightnessContrast" { Err(crate::native_xcf_tones::AuthorizationError::Denied) } else { Ok(()) }
    }
    let _scope = crate::native_xcf_tones::enter_import(deny_second);
    assert!(matches!(crate::import("denied.xcf", &bytes), Err(crate::IoError::NativeXcfToneDenied { .. })));
    assert_eq!(crate::native_xcf_tones::preparation_count(), before, "all native commands must pass before even the first stage is materialized");
}
