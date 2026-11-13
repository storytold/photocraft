use super::super::tests::{TestImage, TestLayer};
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ImportConversion, MAX_CONVERSION_STRING};
use photocraft_geom::Size;

fn pair(x: f32, y: f32) -> Vec<u8> {
    [x.to_be_bytes(), y.to_be_bytes()].concat()
}

#[test]
fn unequal_resolution_warns_and_preserves_durable_axis_detail_on_native_reopen() {
    let mut source = TestImage::rgb8(1, 1);
    source.image_props.push((19, pair(300., 150.)));
    source.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![255, 0, 0, 255]));
    let imported = super::super::import("unequal.xcf", &source.write(), &photocraft_raster::Interrupt::NONE).unwrap();
    let warning = crate::unequal_resolution_warning(300., 150.).unwrap();
    assert_eq!(imported.warnings.iter().filter(|message| **message == warning).count(), 1);
    assert_eq!(imported.document.resolution_dpi, 300.);
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("durable report") };
    let detail = report.entries.iter().find(|entry| entry.feature_code == "xcf.resolution").unwrap();
    assert_eq!(detail.reason, ConversionReason::NativeLimitation);
    assert_eq!(detail.source_description, "Horizontal 300 DPI; vertical 150 DPI");
    let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.resolution_dpi, 300.);
    assert_eq!(reopened.layers[0].surface().unwrap().pixel(0, 0), vec![1., 0., 0., 1.]);
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
}

#[test]
fn unequal_resolution_warning_is_independent_of_conversion_report_retention() {
    let entry = ConversionEntry {
        feature_code: "xcf.image".into(),
        source: conversion::source("image", 0, 0, "Image", &[], None),
        targets: Vec::new(),
        disposition: ConversionDisposition::NativeEquivalent,
        reason: ConversionReason::NativePolicy,
        source_description: String::new(),
        native_description: String::new(),
        attachments: Vec::new(),
    };
    let mut outcomes = conversion::Outcomes::new(26).unwrap();
    let owner = outcomes.push(entry.clone()).unwrap();
    let mut oversized = entry;
    oversized.source_description = "x".repeat(MAX_CONVERSION_STRING + 1);
    outcomes.push(oversized).unwrap();
    assert!(!outcomes.recording(), "exercise an actually exhausted report before processing resolution");
    let payload = pair(300., 150.);
    let properties = [Prop { kind: 19, data: &payload, offset: 0 }];
    let mut doc = Document::new("Usable pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut warnings = crate::ora::Warnings::default();
    let mut budget = 0; // Neither metadata retention nor report capacity may suppress the warning.
    image(&mut doc, &properties, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut warnings).unwrap();
    assert_eq!(doc.resolution_dpi, 300.);
    assert_eq!(warnings.finish(), vec![crate::unequal_resolution_warning(300., 150.).unwrap()]);
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("bounded report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.report_truncated"));
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.resolution"));
}

#[test]
fn equal_nearly_equal_and_invalid_resolution_pairs_do_not_add_unequal_warning() {
    for (payload, dpi) in
        [(pair(300., 300.), 300.), (pair(300., 300.001), 300.), (pair(300., f32::NAN), 72.), (pair(300., 0.), 72.), (300f32.to_be_bytes().to_vec(), 72.)]
    {
        let mut source = TestImage::rgb8(1, 1);
        source.image_props.push((19, payload));
        let imported = super::super::import("resolution-control.xcf", &source.write(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(imported.document.resolution_dpi, dpi);
        assert!(!imported.warnings.iter().any(|warning| warning.contains("vertical resolution")));
    }
}

#[test]
fn duplicate_resolution_properties_warn_only_for_the_effective_last_valid_pair() {
    for (name, pairs, effective) in [
        ("earlier unequal then equal", vec![(300., 150.), (72., 72.)], (72.0f32, 72.0f32)),
        ("equal then unequal", vec![(72., 72.), (300., 150.)], (300., 150.)),
        ("multiple unequal", vec![(300., 150.), (144., 96.)], (144., 96.)),
        ("invalid tail retains last valid", vec![(300., 150.), (144., 96.), (72., f32::NAN)], (144., 96.)),
    ] {
        let mut source = TestImage::rgb8(1, 1);
        for &(x, y) in &pairs {
            source.image_props.push((19, pair(x, y)));
        }
        source.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![255, 0, 0, 255]));
        let imported = super::super::import("effective-resolution.xcf", &source.write(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(imported.document.resolution_dpi, effective.0, "{name}");
        let actual: Vec<_> = imported.warnings.iter().filter(|warning| warning.contains("vertical resolution")).cloned().collect();
        let expected: Vec<_> = crate::unequal_resolution_warning(f64::from(effective.0), f64::from(effective.1)).into_iter().collect();
        assert_eq!(actual, expected, "{name}: only the effective pair may describe the document's DPI");

        // The summary warning follows final semantics, while durable per-record
        // details continue to describe each historical unequal property.
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("durable report") };
        let historical_unequal = pairs.iter().filter(|&&(x, y)| x.is_finite() && y.is_finite() && x != y).count();
        assert_eq!(
            report.entries.iter().filter(|entry| entry.feature_code == "xcf.resolution" && entry.reason == ConversionReason::NativeLimitation).count(),
            historical_unequal,
            "{name}"
        );
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.resolution_dpi, effective.0, "{name}");
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion, "{name}");
    }
}
