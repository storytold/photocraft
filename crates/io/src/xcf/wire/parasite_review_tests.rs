use super::*;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicUsize, Ordering};

fn record(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    out.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

#[test]
fn parasite_collection_bounds_bookkeeping_and_keeps_final_duplicates_after_the_cap() {
    let mut data = Vec::new();
    record(&mut data, "keep", b"old");
    for n in 0..200 {
        record(&mut data, &format!("unused-{n}"), b"unused");
    }
    record(&mut data, "keep", b"final");
    let result = collect_parasites(&data, 512, 1, &Interrupt::NONE).unwrap();
    assert!(result.limited && !result.malformed);
    assert_eq!(result.values, vec![("keep".into(), b"final".to_vec())]);
    let zero = collect_parasites(&data, 0, MAX_PARASITES, &Interrupt::NONE).unwrap();
    assert!(zero.limited && zero.values.is_empty());
    record(&mut data, "keep", b"");
    let result = collect_parasites(&data, 512, 1, &Interrupt::NONE).unwrap();
    assert_eq!(result.values, vec![("keep".into(), Vec::new())], "zero-length final values replace old payloads");
}

#[test]
fn oversized_final_duplicate_is_omitted_instead_of_restoring_stale_metadata() {
    let mut data = Vec::new();
    record(&mut data, "keep", b"old");
    record(&mut data, "keep", &[42; 512]);
    let result = collect_parasites(&data, 256, 1, &Interrupt::NONE).unwrap();
    assert!(result.limited && result.values.is_empty());
}

#[test]
fn borrowed_image_icc_scan_ignores_a_name_flood_and_preserves_empty_record_semantics() {
    let mut first = Vec::new();
    record(&mut first, "icc-profile", b"first property");
    let mut second = Vec::new();
    record(&mut second, "icc-profile", b"stale");
    for n in 0..5000 {
        record(&mut second, &format!("unused-{n}"), b"never copied");
    }
    record(&mut second, "icc-profile", b"final property");
    let image = |second: &[u8]| {
        let mut bytes = Vec::new();
        for data in [&first[..], second] {
            bytes.extend_from_slice(&21u32.to_be_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
            bytes.extend_from_slice(data);
        }
        bytes.extend_from_slice(&[0; 8]);
        let mut cursor = Cursor { b: &bytes, pos: 0, wide: false };
        crate::xcf::read_image_metadata(&mut cursor, 26, &Interrupt::NONE).unwrap().icc
    };
    assert_eq!(image(&second), Some(b"final property".to_vec()));
    record(&mut second, "icc-profile", b"");
    assert_eq!(image(&second), Some(b"first property".to_vec()), "an empty winning ICC only suppresses its own property's earlier duplicate");
}

#[test]
fn borrowed_and_owned_parasite_scans_cancel_mid_flood_and_distinguish_malformed_tails() {
    let mut data = Vec::new();
    for _ in 0..100 {
        record(&mut data, "same", b"value");
    }
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 17;
    assert!(matches!(collect_parasites(&data, 1024, 1, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
    assert_eq!(polls.load(Ordering::Relaxed), 18);
    let mut valid = Vec::new();
    record(&mut valid, "kept", b"value");
    valid.extend_from_slice(&[0, 0]);
    let result = collect_parasites(&valid, 1024, 1, &Interrupt::NONE).unwrap();
    assert!(result.malformed && !result.limited);
    assert_eq!(result.values, vec![("kept".into(), b"value".to_vec())]);
}

#[test]
fn image_metadata_reads_known_values_after_flood_without_spending_native_budget_on_unknowns() {
    use crate::xcf::tests::{TestImage, TestLayer};
    use photocraft_doc::import_conversion::ImportConversion;
    let mut data = Vec::new();
    record(&mut data, "gimp-comment", b"stale");
    for n in 0..5000 {
        record(&mut data, &format!("unused-{n}"), b"unused");
    }
    record(&mut data, "gimp-comment", b"final");
    data.extend_from_slice(&[0, 0]);
    let mut source = TestImage::rgb8(1, 1);
    source.image_props.push((21, data));
    source.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![17, 31, 57, 255]));
    let result = crate::import("parasites.xcf", &source.write()).unwrap();
    assert_eq!(result.document.metadata.text, vec![("Comment".into(), "final".into())]);
    assert_eq!(result.document.layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert!(result.warnings.iter().any(|warning| warning.contains("malformed optional metadata")));
    let Some(ImportConversion::Report(report)) = result.document.metadata.import_conversion.as_deref() else { panic!("report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.parasite_framing"));
    let saved = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
    let restored = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(restored.metadata, result.document.metadata);
    assert_eq!(restored.layers, result.document.layers);
}
#[test]
fn borrowed_metadata_scan_leaves_tiny_native_allowance_for_the_final_known_value_and_reports_real_exhaustion() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ImportConversion, NativeTargetRef};
    let mut data = Vec::new();
    record(&mut data, "gimp-comment", b"stale");
    for n in 0..100 {
        record(&mut data, &format!("unknown-{n}"), b"unused");
    }
    record(&mut data, "gimp-comment", b"final");
    let retained = std::mem::size_of::<(String, String)>() + "Comment".len() + "final".len();
    for allowance in [retained - 1, retained] {
        let mut doc = photocraft_doc::Document::new(
            "Small allowance",
            photocraft_geom::Size::new(1, 1),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        );
        let mut outcomes = crate::xcf::conversion::Outcomes::new(26).unwrap();
        let owner = outcomes
            .push(ConversionEntry {
                feature_code: "xcf.canvas".into(),
                source: crate::xcf::conversion::source("image", 0, 0, "", &[], None),
                targets: vec![NativeTargetRef { label: "Canvas".into(), ..Default::default() }],
                disposition: ConversionDisposition::NativeEquivalent,
                reason: ConversionReason::NativePolicy,
                source_description: "Canvas".into(),
                native_description: "Canvas".into(),
                attachments: Vec::new(),
            })
            .unwrap();
        let props = [Prop { offset: 0, kind: 21, data: &data }];
        outcomes.parasites(owner, 0, &data, &[]).unwrap();
        let mut budget = allowance;
        let mut warnings = crate::ora::Warnings::default();
        crate::xcf::metadata::image(&mut doc, &props, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut warnings, &Interrupt::NONE).unwrap();
        let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
        if allowance == retained {
            assert_eq!(budget, 0, "unknown names and replaced values consume no native metadata allocation");
            assert_eq!(doc.metadata.text, vec![("Comment".into(), "final".into())]);
            assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.metadata_partial"));
        } else {
            let warnings = warnings.finish();
            assert!(warnings.iter().any(|warning| warning.contains("native metadata limits")));
            assert!(warnings.iter().all(|warning| warning.len() < 512));
            assert_eq!(budget, allowance);
            assert!(doc.metadata.text.is_empty());
            assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.metadata_partial" && entry.reason == ConversionReason::RetentionLimit));
            let mut final_record = Vec::new();
            record(&mut final_record, "gimp-comment", b"final");
            assert!(
                report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == final_record.as_slice()),
                "the final optional source record stays available under separate report allowance"
            );
        }
    }
}

#[test]
fn editable_text_after_unknown_layer_parasites_is_not_lost_to_collection_limits() {
    use crate::xcf::tests::{TestImage, TestLayer};
    let mut data = Vec::new();
    record(&mut data, "gimp-text-layer", br#"(text "stale")"#);
    for n in 0..5000 {
        record(&mut data, &format!("unused-{n}"), b"unused");
    }
    record(&mut data, "gimp-text-layer", br#"(text "final") (color (color-rgba 0.25 0.5 0.75 0.5))"#);
    let mut source = TestImage::rgb8(1, 1);
    let mut layer = TestLayer::rgba("Text", 1, 1, vec![17, 31, 57, 255]);
    layer.props.push((21, data));
    source.layers.push(layer);
    let result = crate::import("text-parasites.xcf", &source.write()).unwrap();
    let photocraft_doc::LayerContent::Text(text) = &result.document.layers[0].content else { panic!("unknown source names must not force pixel fallback") };
    assert_eq!(text.text, "final");
    assert_eq!(text.runs[0].style.color.alpha, 0.5);
    assert!(text.cache.is_none());
}
#[test]
fn assigned_icc_after_a_flood_is_preserved_and_header_scan_cancels_during_records() {
    use crate::xcf::tests::{TestImage, TestLayer};
    let mut data = Vec::new();
    let profile = craft_cms::Builtin::DisplayP3.profile().to_bytes();
    record(&mut data, "icc-profile", b"superseded invalid profile");
    for n in 0..5000 {
        record(&mut data, &format!("unused-{n}"), b"unused");
    }
    record(&mut data, "icc-profile", &profile);
    let mut source = TestImage::rgb8(1, 1);
    source.image_props.push((21, data.clone()));
    source.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![17, 31, 57, 255]));
    let result = crate::import("assigned-after-flood.xcf", &source.write()).unwrap();
    assert_eq!(result.document.icc_profile, Some(profile));
    let mut properties = Vec::new();
    properties.extend_from_slice(&21u32.to_be_bytes());
    properties.extend_from_slice(&(data.len() as u32).to_be_bytes());
    properties.extend_from_slice(&data);
    properties.extend_from_slice(&[0; 8]);
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 32;
    let mut cursor = Cursor { b: &properties, pos: 0, wide: false };
    assert!(matches!(crate::xcf::read_image_metadata(&mut cursor, 26, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
    assert_eq!(polls.load(Ordering::Relaxed), 33, "cancellation must stop during the borrowed header scan, before copying the selected ICC");
}
