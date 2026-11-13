use super::*;
use photocraft_doc::import_conversion::MAX_CONVERSION_STRING;

fn owner(name: &str) -> ConversionEntry {
    ConversionEntry {
        feature_code: "xcf.layer".into(),
        source: source("layer", 0, 20, name, &[], None),
        targets: Vec::new(),
        disposition: ConversionDisposition::NativeEquivalent,
        reason: ConversionReason::NativePolicy,
        source_description: String::new(),
        native_description: String::new(),
        attachments: Vec::new(),
    }
}

/// Exercise the actual native-format validation/serialization boundary, not only the builder's counters.
fn assert_truncated_report_persists(conversion: ImportConversion) {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    conversion.validate().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("typed report") };
    assert!(report.entries.len() <= MAX_CONVERSION_ENTRIES);
    assert!(serde_json::to_vec(report).unwrap().len() <= MAX_CONVERSION_BYTES);
    let expected_entries = report.entries.len();
    let expected_attachments = report.attachments.len();
    let notices: Vec<_> = report.entries.iter().filter(|e| e.feature_code == "xcf.report_truncated").collect();
    assert_eq!(notices.len(), 1);
    let notice = notices[0];
    assert_eq!((notice.disposition, notice.reason), (ConversionDisposition::SkippedNonessential, ConversionReason::RetentionLimit));
    assert!(notice.native_description.contains("incomplete"));
    assert!(notice.targets.is_empty());
    assert!(notice.attachments.is_empty());
    let mut document = Document::new("Usable native document", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut pixels = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    pixels.write_region(Rect::new(0, 0, 1, 1), &[1., 0., 0., 1.]);
    document.layers.push(Layer::new("Saved pixels", LayerContent::Raster(pixels)));
    document.metadata.import_conversion = Some(std::sync::Arc::new(conversion));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    drop(document);
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
    let Some(ImportConversion::Report(report)) = reopened.metadata.import_conversion.as_deref() else { panic!("durable report") };
    report.validate().unwrap();
    assert_eq!((report.entries.len(), report.attachments.len()), (expected_entries, expected_attachments));
    assert_eq!(report.entries.iter().filter(|e| e.feature_code == "xcf.report_truncated").count(), 1);
}

#[test]
fn report_entry_cap_is_nonfatal_and_persists_one_immutable_notice() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let original = owner("Owner");
    let first = outcomes.push(original.clone()).unwrap();
    outcomes.property(first, 999, 30, b"retained configuration").unwrap();
    // Two entries already exist; fill the actual ceiling, without large pixel fixtures.
    for _ in 2..MAX_CONVERSION_ENTRIES - 1 {
        outcomes.push(original.clone()).unwrap();
    }
    outcomes.property(first, 998, 40, b"queued tail attachment must be discarded with its owner").unwrap();
    let last = MAX_CONVERSION_ENTRIES - 1;
    assert_eq!(outcomes.report.entries.len(), MAX_CONVERSION_ENTRIES, "a complete report at the exact count ceiling still fits");
    let omitted = outcomes.push(original.clone()).expect("optional report count cannot reject native content");
    assert_eq!(outcomes.entry(omitted).unwrap().feature_code, "xcf.report_truncated");
    assert_eq!(outcomes.entry(last).unwrap().feature_code, "xcf.report_truncated", "the evicted owner must never alias the summary");
    assert!(outcomes.entry(usize::MAX - 1).is_err(), "unissued IDs remain errors");
    let summary = outcomes.entry(omitted).unwrap().clone();
    let mut warnings = crate::ora::Warnings::default();
    for id in [omitted, last] {
        outcomes.detail(id, "xcf.must_not_mutate_summary", ConversionReason::MissingMapping, "source".into(), "native".into()).unwrap();
        outcomes.property(id, 998, 50, b"must not attach to summary").unwrap();
        outcomes.parasites(id, 60, b"malformed optional configuration", &[]).unwrap();
        outcomes
            .notice(id, "xcf.must_not_mutate_summary", ConversionReason::NativeLimitation, ("source".into(), "native".into()), Some("omitted"), &mut warnings)
            .unwrap();
    }
    assert_eq!(outcomes.entry(omitted).unwrap(), &summary);
    assert_eq!(warnings.finish(), ["An imported item omitted"]);
    let conversion = outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("report") };
    assert_eq!(report.attachments.len(), 1, "valid prefix attachments remain coherent");
    assert_eq!(report.entries[1].attachments, vec![0]);
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.source_property.998"));
    assert_truncated_report_persists(conversion);
}

#[test]
fn repeated_megabyte_names_hit_retained_byte_cap_without_failing_native_save() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let entry = owner(&"n".repeat(MAX_CONVERSION_STRING));
    for _ in 0..40 {
        let id = outcomes.push(entry.clone()).expect("optional repeated names cannot reject usable content");
        outcomes.detail(id, "xcf.mask", ConversionReason::NativePolicy, "Saved mask".into(), "Native mask".into()).unwrap();
        outcomes.property(id, 999, 30, b"inert configuration").unwrap();
    }
    assert_truncated_report_persists(outcomes.finish().unwrap());
}

#[test]
fn escaped_megabyte_names_hit_encoded_byte_cap_without_failing_native_save() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let entry = owner(&"\u{1}".repeat(MAX_CONVERSION_STRING));
    for _ in 0..8 {
        let id = outcomes.push(entry.clone()).expect("JSON escaping cannot make optional reporting fatal");
        outcomes.property(id, 999, 30, b"inert configuration").unwrap();
    }
    assert_truncated_report_persists(outcomes.finish().unwrap());
}

#[test]
fn below_report_caps_owner_targets_and_optional_configuration_are_unchanged() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let mut entry = owner("Owner");
    entry.targets.push(NativeTargetRef { label: "Native layer".into(), ..Default::default() });
    let id = outcomes.push(entry.clone()).unwrap();
    outcomes.property(id, 999, 30, b"original configuration").unwrap();
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.entries[0], entry);
    assert_eq!(report.entries[1].source.name, "Owner");
    assert_eq!(report.entries[1].targets, entry.targets);
    assert_eq!(report.entries[1].attachments, vec![0]);
    assert_eq!(report.attachments[0].bytes.as_slice(), b"original configuration");
    report.validate().unwrap();
}

#[test]
fn human_notices_survive_the_detail_that_truncates_reporting_and_later_omitted_owners() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let id = outcomes.push(owner("Actual owner")).unwrap();
    let mut warnings = crate::ora::Warnings::default();
    outcomes
        .notice(
            id,
            "xcf.blend",
            ConversionReason::NativeLimitation,
            ("source blend".into(), "native approximation".into()),
            Some("uses an approximate blend"),
            &mut warnings,
        )
        .unwrap();
    assert!(outcomes.recording());
    outcomes
        .notice(
            id,
            "xcf.coverage_clamped",
            ConversionReason::NumericalConversion,
            ("x".repeat(MAX_CONVERSION_STRING + 1), "coverage clamped".into()),
            Some("has clamped coverage"),
            &mut warnings,
        )
        .unwrap();
    assert!(!outcomes.recording());
    outcomes
        .notice(
            id,
            "xcf.precision_underflow",
            ConversionReason::NumericalConversion,
            ("tiny source".into(), "native zero".into()),
            Some("has tiny samples rounded to zero"),
            &mut warnings,
        )
        .unwrap();
    let omitted = outcomes.push(owner("Label unavailable after cap")).unwrap();
    for (feature, human) in [("xcf.text", "has modified text pixels"), ("xcf.link", "has an unavailable linked resource")] {
        outcomes.notice(omitted, feature, ConversionReason::NativeLimitation, ("source".into(), "native raster".into()), Some(human), &mut warnings).unwrap();
    }
    let warnings = warnings.finish();
    assert_eq!(warnings.len(), 5);
    for human in [
        "uses an approximate blend",
        "has clamped coverage",
        "has tiny samples rounded to zero",
        "has modified text pixels",
        "has an unavailable linked resource",
    ] {
        assert!(warnings.iter().any(|warning| warning.contains(human)), "{human}: {warnings:?}");
    }
    assert_eq!(warnings.iter().filter(|warning| warning.contains("Actual owner")).count(), 3);
    assert!(warnings.iter().all(|warning| !warning.contains("XCF conversion report") && !warning.contains("Label unavailable after cap")));
    assert_truncated_report_persists(outcomes.finish().unwrap());
}

#[test]
fn omitted_owner_warning_labels_do_not_accumulate_or_mutate_the_summary() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let mut oversized = owner("Oversized source");
    oversized.source_description = "x".repeat(MAX_CONVERSION_STRING + 1);
    let omitted = outcomes.push(oversized).unwrap();
    let summary = outcomes.entry(omitted).unwrap().clone();
    let mut warnings = crate::ora::Warnings::default();
    for index in 0..128 {
        let id = outcomes.push(owner(&format!("Unretained source {index}"))).unwrap();
        outcomes
            .notice(id, "xcf.text", ConversionReason::NativeLimitation, ("source".into(), "native".into()), Some("has modified text pixels"), &mut warnings)
            .unwrap();
    }
    assert!(outcomes.warning(usize::MAX - 1, "invalid owner", &mut warnings).is_err());
    assert_eq!(warnings.finish(), ["An imported item has modified text pixels"]);
    assert_eq!(outcomes.entry(omitted).unwrap(), &summary);
    assert_truncated_report_persists(outcomes.finish().unwrap());
}

#[test]
fn full_report_coverage_inventory_still_warns_without_reading_optional_source() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let mut oversized = owner("Unretained coverage owner");
    oversized.source_description = "x".repeat(MAX_CONVERSION_STRING + 1);
    let omitted = outcomes.push(oversized).unwrap();
    let mut image = crate::xcf::Image {
        c: crate::xcf::Cursor { b: &[], pos: 0, wide: true },
        version: 26,
        precision: crate::xcf::Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: crate::xcf::Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: 0,
        meta_budget: 0,
        warnings: Default::default(),
        outcomes,
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: 72.,
        pending_effects: Vec::new(),
        path_tattoos: Default::default(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    };
    image.inventory_coverage_effects(omitted, &[], &photocraft_raster::Interrupt::NONE).unwrap();
    assert!(std::mem::take(&mut image.warnings).finish().is_empty());
    assert!(matches!(
        image.inventory_coverage_effects(omitted, &[usize::MAX], &photocraft_raster::Interrupt::cancel_only(&|| true)),
        Err(crate::IoError::Cancelled)
    ));
    assert!(std::mem::take(&mut image.warnings).finish().is_empty(), "cancellation still wins over warnings");
    image.inventory_coverage_effects(omitted, &[usize::MAX], &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!((image.budget, image.meta_budget), (0, 0));
    let warnings = image.warnings.finish();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("stored coverage effects"));
    assert!(warnings[0].starts_with("An imported item "));
    assert!(!warnings[0].contains("XCF conversion report"));
    assert_truncated_report_persists(image.outcomes.finish().unwrap());
}

#[test]
fn public_import_report_cap_preserves_all_layers_pixels_one_warning_and_native_save() {
    use crate::xcf::tests::{TestImage, TestLayer};
    use photocraft_doc::Document;
    use photocraft_geom::Rect;

    fn assert_pixels(document: &Document) {
        assert_eq!(document.layers.len(), 20);
        let mut visible = 0;
        for layer in &document.layers {
            let surface = layer.surface().expect("native raster layer");
            if layer.name == "Visible saved pixels" {
                visible += 1;
                assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
            } else {
                assert_eq!(layer.name, "Transparent stored layer");
                assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), vec![0., 0., 0., 0.]);
                assert_eq!(surface.tile_count(), 0, "transparent layers must remain prunable");
            }
        }
        assert_eq!(visible, 1);
        let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("durable report") };
        report.validate().unwrap();
        assert_eq!(report.entries.iter().filter(|entry| entry.feature_code == "xcf.report_truncated").count(), 1);
    }

    let mut source = TestImage::rgb8(1, 1);
    for i in 0..20 {
        let (name, pixel) = if i == 0 { ("Visible saved pixels", vec![255, 0, 0, 255]) } else { ("Transparent stored layer", vec![0, 0, 0, 0]) };
        let mut layer = TestLayer::rgba(name, 1, 1, pixel);
        // Each layer remains below the source parser's 4096-property bound. The
        // complete file is <1 MiB and uses only one retained native raster tile;
        // repeated empty optional records exercise the independent report cap.
        layer.props = vec![(777, Vec::new()); 4000];
        source.layers.push(layer);
    }
    let bytes = source.write();
    assert!(bytes.len() < 1 << 20);
    let imported = crate::import("original-report-cap.xcf", &bytes).expect("optional report saturation must not reject usable XCF layers");
    assert!(!imported.preview_only);
    assert_eq!(imported.warnings.iter().filter(|warning| warning.contains("conversion report is incomplete")).count(), 1);
    assert_pixels(&imported.document);
    let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    drop(imported);
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_pixels(&reopened);
}

#[test]
fn optional_attachment_respects_portable_entry_and_target_costs() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let owner = outcomes
        .push(ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: source("layer", 0, 20, "Owner", &[], None),
            targets: vec![NativeTargetRef::default(); 1000],
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: String::new(),
            native_description: String::new(),
            attachments: Vec::new(),
        })
        .unwrap();
    let bytes = vec![7; MAX_CONVERSION_BYTES - 250_000];
    outcomes.property(owner, 999, 30, &bytes).unwrap();
    let ImportConversion::Report(report) = outcomes.finish().expect("optional data must not make the import fail") else { panic!("typed report") };
    assert!(report.attachments.is_empty());
    assert_eq!(report.entries[1].disposition, ConversionDisposition::SkippedNonessential);
    assert_eq!(report.entries[1].reason, ConversionReason::RetentionLimit);
    report.validate().unwrap();
}
