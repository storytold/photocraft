use crate::native_xcf_tones::{
    AuthorizationError, enter_import,
    fixtures::{MaskedExposureOptions, masked_exposure},
};
use photocraft_doc::{
    LayerContent,
    import_conversion::{ConversionDisposition, ConversionReason, ImportConversion},
};
fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), AuthorizationError> {
    Ok(())
}

#[test]
fn one_masked_exposure_keeps_independent_opacity_coverage_and_source_provenance() {
    let _scope = enter_import(allow);
    let fixture = masked_exposure(Default::default());
    let imported = crate::import("synthetic-mask.xcf", &fixture.bytes).unwrap();
    let layer = &imported.document.layers[0];
    let LayerContent::Smart(smart) = &layer.content else { panic!("proven masked exposure must be native") };
    assert_eq!(smart.smart_filters.len(), 1);
    assert_eq!(smart.smart_filters[0].opacity, 0.65);
    let mask = smart.filter_mask.as_ref().unwrap();
    assert_eq!(mask.surface.format().sample, photocraft_color::SampleType::F32);
    assert_eq!(mask.surface.sample_channel(1, 0, 0), 64f32 / 255.);
    assert_eq!(mask.surface.sample_channel(2, 0, 0), 192f32 / 255.);
    assert_eq!(mask.surface.sample_channel(3, 0, 0), 1.);
    assert_eq!(layer.opacity, 0.75);
    assert_eq!(smart.transform, photocraft_doc::Affine::translate(1., 0.));
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("source evidence missing") };
    let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.effect_mask").unwrap();
    assert_eq!(entry.source.record_offset, Some(fixture.mask_offset));
    assert_eq!(entry.source.name, "Independent coverage");
    assert_eq!(entry.disposition, ConversionDisposition::NativeApproximation);
    assert_eq!(entry.reason, ConversionReason::NativePolicy);
    assert!(!entry.attachments.is_empty());
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native controls lost") };
    assert_eq!(again.smart_filters, smart.smart_filters);
    assert_eq!(again.filter_mask, smart.filter_mask);
    assert_eq!(again.cache, smart.cache);
}

#[test]
fn unknown_policy_invalid_mask_and_unproven_layouts_keep_original_raster() {
    let _scope = enter_import(allow);
    for (options, reason) in [
        (MaskedExposureOptions { unknown_policy: true, ..Default::default() }, ConversionReason::NativeLimitation),
        (MaskedExposureOptions { invalid_mask: true, ..Default::default() }, ConversionReason::InvalidOptionalData),
        (MaskedExposureOptions { partial_alpha: true, ..Default::default() }, ConversionReason::NativeLimitation),
        (MaskedExposureOptions { float_owner: true, ..Default::default() }, ConversionReason::NativeLimitation),
    ] {
        let expected = if options.float_owner {
            vec![0.125, 0.375, 0.625, 1., 0.75, 0.5, 0.25, 1.]
        } else {
            [32u8, 96, 160, if options.partial_alpha { 128 } else { 255 }, 192, 128, 64, 255].into_iter().map(|value| f32::from(value) / 255.).collect()
        };
        let fixture = masked_exposure(options);
        let imported = crate::import("unapplied-mask.xcf", &fixture.bytes).unwrap();
        let layer = &imported.document.layers[0];
        let LayerContent::Raster(original) = &layer.content else { panic!("refused source must remain raster") };
        assert_eq!(original.read_region(photocraft_geom::Rect::from_xywh(1, 0, 2, 1)), expected);
        assert_eq!(layer.opacity, 0.75);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("fallback evidence missing") };
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
        assert!(report.entries.iter().any(|entry| entry.reason == reason));
        assert!(imported.warnings.iter().any(|warning| warning.contains("not applied")));
    }
}

#[test]
fn mask_admission_keeps_storage_atomic_and_attempted_work_nonrefundable() {
    use super::coverage::{self, Admission, Failure};
    use photocraft_raster::Interrupt;
    for truncated in [false, true] {
        let mut fixture = masked_exposure(Default::default());
        if truncated {
            fixture.bytes.pop();
        }
        let xcf = craft_xcf::Xcf::parse(&fixture.bytes).unwrap();
        let mut effects = craft_xcf::effects::Budget::default();
        let list = xcf.layer_effects(0, &mut effects, &|| true).unwrap();
        let effect = craft_xcf::effects::Reader::new(&fixture.bytes, 22).read(list.owner, list.offsets[0], &mut effects, &|| true).unwrap();
        let mut retained = 16 << 20;
        let before = retained;
        let mut work = 32 << 20;
        let work_before = work;
        let mut source = 16 << 20;
        let source_before = source;
        let mut metadata = craft_xcf::records::Budget::default();
        let mut global_retained = before;
        let mut admission = Admission { retained: &mut global_retained, source: &mut source, metadata: &mut metadata, options: Default::default() };
        let result = coverage::decode(&xcf, &effect.record, &mut admission, &mut retained, &mut work, &Interrupt::NONE);
        assert!(work < work_before);
        assert!(source < source_before, "source visitation must be prepaid even on truncated samples");
        assert_eq!(global_retained, before, "only the caller may publish retained storage");
        if truncated {
            assert!(matches!(result, Err(Failure::Invalid)));
            assert_eq!(retained, before, "failed private surfaces are not retained");
        } else {
            assert!(result.is_ok());
            assert!(retained < before);
        }
    }
}

#[test]
fn mask_bounds_and_cancel_refuse_before_sample_work() {
    use super::coverage::{self, Admission, Failure};
    use photocraft_raster::Interrupt;
    let fixture = masked_exposure(Default::default());
    let xcf = craft_xcf::Xcf::parse(&fixture.bytes).unwrap();
    let mut effects = craft_xcf::effects::Budget::default();
    let list = xcf.layer_effects(0, &mut effects, &|| true).unwrap();
    let effect = craft_xcf::effects::Reader::new(&fixture.bytes, 22).read(list.owner, list.offsets[0], &mut effects, &|| true).unwrap();
    for cancelled in [false, true] {
        let mut retained = 1;
        let mut global_retained = 1;
        let mut source = 16 << 20;
        let source_before = source;
        let mut work = 32 << 20;
        let work_before = work;
        let mut metadata = craft_xcf::records::Budget::default();
        let mut admission = Admission { retained: &mut global_retained, source: &mut source, metadata: &mut metadata, options: Default::default() };
        let cancel = || cancelled;
        let ctl = Interrupt::cancel_only(&cancel);
        let result = coverage::decode(&xcf, &effect.record, &mut admission, &mut retained, &mut work, &ctl);
        if cancelled {
            assert!(matches!(result, Err(Failure::Cancelled)));
        } else {
            assert!(matches!(result, Err(Failure::Limit)));
        }
        assert_eq!(source, source_before);
        assert_eq!(work, work_before);
        assert_eq!(retained, 1);
        assert_eq!(global_retained, 1);
    }
}

#[test]
fn disabled_masked_exposure_retains_controls_without_applying_them() {
    let _scope = enter_import(allow);
    let fixture = masked_exposure(MaskedExposureOptions { disabled_stage: true, ..Default::default() });
    let imported = crate::import("disabled-mask.xcf", &fixture.bytes).unwrap();
    let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("disabled editable stage lost") };
    assert_eq!(smart.smart_filters.len(), 1);
    assert!(!smart.smart_filters[0].visible);
    assert_eq!(smart.smart_filters[0].opacity, 0.65);
    assert!(smart.filter_mask.is_some());
    assert_eq!(smart.cache.as_ref().unwrap().to_interleaved(photocraft_geom::Rect::from_xywh(1, 0, 2, 1)), [32, 96, 160, 255, 192, 128, 64, 255]);
}

#[test]
fn u16_masked_exposure_preserves_low_bits_and_source_mask_samples() {
    let _scope = enter_import(allow);
    let fixture = masked_exposure(MaskedExposureOptions { u16_owner: true, ..Default::default() });
    let imported = crate::import("u16-mask.xcf", &fixture.bytes).unwrap();
    assert_eq!(imported.document.depth, photocraft_color::SampleType::U16);
    let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("proven U16 stage not installed") };
    let mask = smart.filter_mask.as_ref().unwrap();
    assert_eq!(mask.surface.sample_channel(1, 0, 0), (16385f64 / 65535.) as f32);
    assert_eq!(mask.surface.sample_channel(2, 0, 0), (49153f64 / 65535.) as f32);
    let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("native source missing") };
    let original = photocraft_format::load_from_bytes(bytes).unwrap();
    let words = [8193u16, 24578, 40963, 65535, 49156, 32773, 16390, 65535];
    assert_eq!(
        original.layers[0].surface().unwrap().to_interleaved(photocraft_geom::Rect::from_xywh(0, 0, 2, 1)),
        words.into_iter().flat_map(u16::to_le_bytes).collect::<Vec<_>>()
    );
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native mask lost") };
    assert_eq!(again.filter_mask, smart.filter_mask);
    assert_eq!(again.smart_filters, smart.smart_filters);
    assert_eq!(again.cache, smart.cache);
}

#[test]
fn u16_partial_alpha_and_unknown_policy_remain_explicit_raster_fallbacks() {
    let _scope = enter_import(allow);
    for partial in [false, true] {
        let fixture = masked_exposure(MaskedExposureOptions { u16_owner: true, partial_alpha: partial, unknown_policy: !partial, ..Default::default() });
        let imported = crate::import("unproven-u16.xcf", &fixture.bytes).unwrap();
        let LayerContent::Raster(original) = &imported.document.layers[0].content else { panic!("unproven source activated") };
        let words = [8193u16, 24578, 40963, if partial { 32768 } else { 65535 }, 49156, 32773, 16390, 65535];
        assert_eq!(original.to_interleaved(photocraft_geom::Rect::from_xywh(1, 0, 2, 1)), words.into_iter().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("fallback report absent") };
        assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::NativeLimitation));
        assert!(report.entries.iter().all(|entry| entry.disposition != ConversionDisposition::NativeApproximation));
    }
}
