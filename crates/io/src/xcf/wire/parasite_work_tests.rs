use super::*;
use crate::xcf::tests::{TestImage, TestLayer};
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
use photocraft_raster::Interrupt;

fn record(out: &mut Vec<u8>, name: &[u8], payload: &[u8]) -> usize {
    out.extend_from_slice(&u32::try_from(name.len() + 1).unwrap().to_be_bytes());
    out.extend_from_slice(name);
    out.push(0);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    out.extend_from_slice(payload);
    24 + name.len() + 1 + payload.len()
}

fn source(data: &[u8], owners: usize) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    for _ in 0..owners {
        let mut layer = TestLayer::rgba("Owner", 1, 1, vec![17, 31, 57, 255]);
        layer.props.push((21, data.to_vec()));
        image.layers.push(layer);
    }
    image.write()
}

fn import(bytes: &[u8], limit: usize) -> crate::ImportResult {
    import_with_limits(bytes, limit, limit)
}

fn import_with_limits(bytes: &[u8], native: usize, report: usize) -> crate::ImportResult {
    crate::xcf::import_with_sample_order_and_parasite_work(
        "work.xcf",
        bytes,
        None,
        ParasiteWork::with_limit(native),
        ParasiteWork::with_limit(report),
        &Interrupt::NONE,
    )
    .unwrap()
}

fn assert_pixels_and_loss(imported: &crate::ImportResult, owners: usize) {
    assert_eq!(imported.document.layers.len(), owners);
    for layer in &imported.document.layers {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("durable report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.parasite_work" && entry.reason == ConversionReason::RetentionLimit));
    assert!(imported.warnings.iter().any(|warning| warning.contains("work allowance") || warning.contains("retention limits")));
    let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), imported.document);
}

#[test]
fn parasite_work_exact_boundary_preserves_borrowed_data_and_bounds_empty_records() {
    let mut bytes = Vec::new();
    let cost = record(&mut bytes, b"k", &[42]);
    let mut work = ParasiteWork::with_limit(cost);
    let mut seen = Vec::new();
    let scan = scan_parasites_with_work(&bytes, &mut work, &Interrupt::NONE, |record| {
        assert_eq!(record.name, b"k");
        assert_eq!(record.payload, [42]);
        assert_eq!(record.raw, bytes.as_slice());
        assert_eq!(record.start, 0);
        seen.push(record.payload.as_ptr());
        Ok(true)
    })
    .unwrap();
    assert!(!scan.malformed && !scan.limited);
    assert_eq!(work.remaining(), 0);
    assert_eq!(seen, [bytes.last().unwrap() as *const u8]);
    let scan = scan_parasites_with_work(&bytes, &mut work, &Interrupt::NONE, |_| panic!("spent work must not visit another record")).unwrap();
    assert!(scan.limited && !scan.malformed);
    let mut empty = Vec::new();
    let cost = record(&mut empty, b"", b"");
    let mut flood = empty.repeat(100);
    flood.extend_from_slice(&[0, 0]);
    let mut work = ParasiteWork::with_limit(cost * 4);
    PARASITE_RECORD_READS.with(|count| count.set(0));
    let scan = scan_parasites_with_work(&flood, &mut work, &Interrupt::NONE, |_| Ok(true)).unwrap();
    assert!(scan.limited && !scan.malformed);
    assert_eq!(PARASITE_RECORD_READS.with(std::cell::Cell::get), 4);
}

#[test]
fn parasite_scan_work_is_shared_across_aliases_and_distinct_inputs() {
    let mut bytes = Vec::new();
    let cost = record(&mut bytes, b"same", &[7; 64]);
    for distinct in [false, true] {
        let mut work = ParasiteWork::with_limit(cost * 3);
        let mut visited = 0;
        PARASITE_RECORD_READS.with(|count| count.set(0));
        for index in 0..16 {
            let mut distinct_bytes = bytes.clone();
            if distinct {
                *distinct_bytes.last_mut().unwrap() = index;
            }
            let input = if distinct { &distinct_bytes } else { &bytes };
            let scan = scan_parasites_with_work(input, &mut work, &Interrupt::NONE, |_| {
                visited += 1;
                Ok(true)
            })
            .unwrap();
            assert_eq!(scan.limited, index >= 3);
        }
        assert_eq!(visited, 3);
        assert_eq!(PARASITE_RECORD_READS.with(std::cell::Cell::get), 3);
        assert_eq!(work.remaining(), 0);
    }
}

#[test]
fn limited_text_parasite_flags_keep_pixels_for_repeated_and_distinct_owners() {
    let mut data = Vec::new();
    record(&mut data, b"plug_in_gdyntext/data", br"GDT10{legacy}{1}{0}{0}{0}{ff2200}{0}{-*-sans-medium-r-normal-*-20-*-*-*-*-*-iso8859-1}");
    for _ in 0..64 {
        record(&mut data, b"ignored", b"x");
    }
    record(&mut data, b"gimp-text-layer", br#"(text "modern wins")"#);
    for aliases in [false, true] {
        let mut bytes = source(&data, 16);
        if aliases {
            let (mut cursor, header) = crate::xcf::read_header(&bytes).unwrap();
            props_version(&mut cursor, Some(header.version)).unwrap();
            let table = cursor.pos;
            let first = bytes[table..table + 8].to_vec();
            for index in 1..16 {
                bytes[table + index * 8..table + (index + 1) * 8].copy_from_slice(&first);
            }
        }
        PARASITE_RECORD_READS.with(|count| count.set(0));
        let imported = import(&bytes, 300);
        assert!(PARASITE_RECORD_READS.with(std::cell::Cell::get) <= 600 / 24);
        assert_pixels_and_loss(&imported, 16);
    }
}

#[test]
fn handled_report_parasites_use_an_independent_bounded_scan() {
    let mut one = Vec::new();
    let cost = record(&mut one, b"gimphoto-clipped", b"");
    let data = one.repeat(8);
    PARASITE_RECORD_READS.with(|count| count.set(0));
    let imported = import_with_limits(&source(&data, 1), cost * 8, cost * 2);
    assert_eq!(PARASITE_RECORD_READS.with(std::cell::Cell::get), 10, "eight flag records and two independent report records");
    assert!(imported.document.layers[0].clipped, "known flags remain usable when report scanning is limited");
    assert_pixels_and_loss(&imported, 1);
}

#[test]
fn limited_text_selection_does_not_choose_a_prefix_before_a_later_modern_record() {
    let mut data = Vec::new();
    let prefix = record(&mut data, b"plug_in_gdyntext/data", br"GDT10{legacy}{1}{0}{0}{0}{ff2200}{0}{-*-sans-medium-r-normal-*-20-*-*-*-*-*-iso8859-1}");
    let modern = br#"(text "modern wins")"#;
    let last = record(&mut data, b"gimp-text-layer", modern);
    let full_scan = prefix + last;
    // Flag selection completes, but text selection cannot establish the last native
    // configuration. Using the valid legacy prefix would guess a different document.
    let imported = import(&source(&data, 1), full_scan + prefix);
    assert_pixels_and_loss(&imported, 1);
    let complete = import(&source(&data, 1), full_scan * 3);
    let photocraft_doc::LayerContent::Text(text) = &complete.document.layers[0].content else { panic!("complete selection remains editable") };
    assert_eq!(text.text, "modern wins");
}

#[test]
fn limited_image_selection_omits_partial_known_values_and_discloses_unknown_icc_assignment() {
    let mut data = Vec::new();
    let prefix = record(&mut data, b"gimp-comment", b"stale");
    record(&mut data, b"gimp-comment", b"final");
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.image_props.push((21, data));
    image.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![17, 31, 57, 255]));
    let imported = import(&image.write(), prefix);
    assert!(imported.document.metadata.text.is_empty(), "a prefix is not the final metadata assignment");
    assert!(imported.document.icc_profile.is_some(), "unknown assignment opens with a disclosed tagged implicit profile");
    assert_pixels_and_loss(&imported, 1);
}

#[test]
fn borrowed_report_names_preserve_non_utf8_unknown_record_classification() {
    let mut data = Vec::new();
    record(&mut data, &[0xff], b"unknown");
    let imported = import(&source(&data, 1), 4096);
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.source_property.21" && entry.reason == ConversionReason::MissingMapping));
    assert!(report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == data.as_slice()));
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.parasite_framing"));
}

#[test]
fn parasite_work_limits_stay_distinct_from_malformed_prefixes_and_cancellation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut data = Vec::new();
    record(&mut data, b"known", b"prefix");
    data.extend_from_slice(&[0, 0]);
    let mut seen = 0;
    let status = scan_parasites_with_work(&data, &mut ParasiteWork::default(), &Interrupt::NONE, |_| {
        seen += 1;
        Ok(true)
    })
    .unwrap();
    assert!(status.malformed && !status.limited);
    assert_eq!(seen, 1);
    let cancel = || true;
    for data in [data.as_slice(), &[]] {
        assert!(matches!(
            scan_parasites_with_work(data, &mut ParasiteWork::with_limit(0), &Interrupt::cancel_only(&cancel), |_| Ok(true)),
            Err(crate::IoError::Cancelled)
        ));
    }
    let data = data[..data.len() - 2].repeat(32);
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
    let mut work = ParasiteWork::default();
    assert!(matches!(scan_parasites_with_work(&data, &mut work, &Interrupt::cancel_only(&cancel), |_| Ok(true)), Err(crate::IoError::Cancelled)));
    assert!(work.remaining() < super::super::MAX_METADATA_BYTES * 4);
}

#[test]
fn image_and_earlier_layer_report_scans_cannot_spend_later_text_admission() {
    for image_owner in [false, true] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = 26;
        let mut large = Vec::new();
        let large_cost = record(&mut large, b"unused", &[0; 512]);
        let mut text_property = Vec::new();
        let text_cost = record(&mut text_property, b"gimp-text-layer", br#"(text "Editable") (font-size 12)"#);
        let mut text = TestLayer::rgba("Text", 1, 1, vec![17, 31, 57, 255]);
        text.props.push((21, text_property));
        image.layers.push(text);
        if image_owner {
            image.image_props.push((21, large));
        } else {
            let mut earlier = TestLayer::rgba("Earlier report", 1, 1, vec![0, 0, 0, 255]);
            earlier.props.push((21, large));
            image.layers.push(earlier);
        }
        let native = large_cost * if image_owner { 2 } else { 1 } + text_cost * 2;
        let imported = import_with_limits(&image.write(), native, 64);
        let layer = imported.document.layers.iter().find(|layer| layer.name == "Text").unwrap();
        let photocraft_doc::LayerContent::Text(text) = &layer.content else { panic!("report work discarded native text for image owner {image_owner}") };
        assert_eq!(text.text, "Editable");
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("report") };
        assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit), "the separate report allowance was exhausted");
    }
}
