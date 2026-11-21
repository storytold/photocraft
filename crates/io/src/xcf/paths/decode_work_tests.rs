use super::*;
use crate::xcf::*;
use photocraft_raster::Interrupt;

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn name(out: &mut Vec<u8>, value: &str) {
    word(out, value.len() as u32 + 1);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}
fn stroke(out: &mut Vec<u8>, knots: u32) {
    for value in [1, 0, 2, knots * 3] {
        word(out, value);
    }
    for _ in 0..knots {
        for (kind, x, y) in [(1, 0f32, 1f32), (0, 2., 3.), (1, 4., 5.)] {
            word(out, kind);
            word(out, x.to_bits());
            word(out, y.to_bits());
        }
    }
}
fn record(label: &str, knots: u32, trailing: bool) -> Vec<u8> {
    let mut payload = Vec::new();
    // A tattoo and selected marker test that incomplete identity is not guessed.
    for value in [20, 4, 72, 43, 0, 0, 0, 1, 1] {
        word(&mut payload, value);
    }
    stroke(&mut payload, knots);
    if trailing {
        word(&mut payload, 0);
    }
    let mut bytes = Vec::new();
    name(&mut bytes, label);
    word(&mut bytes, payload.len() as u32);
    bytes.extend(payload);
    bytes
}
fn cursor(bytes: &[u8]) -> Cursor<'_> {
    Cursor { b: bytes, pos: 0, wide: true }
}
fn reset() {
    PATH_DECODES.with(|n| n.set(0));
    PATH_STRINGS.with(|n| n.set(0));
    PATH_POINTS.with(|n| n.set(0));
}
fn counts() -> (usize, usize, usize) {
    (PATH_DECODES.with(std::cell::Cell::get), PATH_STRINGS.with(std::cell::Cell::get), PATH_POINTS.with(std::cell::Cell::get))
}

#[test]
fn failed_standalone_aliases_and_distinct_records_spend_work_before_names_and_geometry() {
    for distinct in [false, true] {
        let first = record("one", 1, true);
        let second = if distinct { record("two", 1, true) } else { first.clone() };
        let mut work = DecodeWork { bytes_left: first.len() + second.len() };
        reset();
        for bytes in [&first, &second] {
            let error = standalone_with_work(&cursor(bytes), 0, &mut work, &Interrupt::NONE).err().unwrap();
            assert!(error.to_string().contains("payload length mismatch"), "the first two attempts finish point decoding: {error}");
        }
        assert_eq!(counts(), (2, 2, 6));
        assert_eq!(work.bytes_left, 0);
        assert!(standalone_with_work(&cursor(&first), 0, &mut work, &Interrupt::NONE).unwrap().is_none());
        assert_eq!(counts(), (2, 2, 6), "a third alias must not clone a name or decode even one point");
    }
}

#[test]
fn successful_and_failed_paths_share_the_exact_nonrefundable_boundary() {
    let good = record("one", 1, false);
    let bad = record("two", 1, true);
    let mut work = DecodeWork { bytes_left: good.len() + bad.len() };
    reset();
    let path = standalone_with_work(&cursor(&good), 0, &mut work, &Interrupt::NONE).unwrap().unwrap();
    assert_eq!(path.named.name, "one");
    let knot = path.named.path.subpaths[0].knots[0];
    assert_eq!((knot.anchor, knot.in_ctrl, knot.out_ctrl), (Point::new(2., 3.), Point::new(0., 1.), Point::new(4., 5.)));
    assert!(standalone_with_work(&cursor(&bad), 0, &mut work, &Interrupt::NONE).is_err());
    assert_eq!((work.bytes_left, counts()), (0, (2, 2, 6)));
    assert!(standalone_with_work(&cursor(&good), 0, &mut work, &Interrupt::NONE).unwrap().is_none());
    let mut short = DecodeWork { bytes_left: good.len() - 1 };
    reset();
    assert!(standalone_with_work(&cursor(&good), 0, &mut short, &Interrupt::NONE).unwrap().is_none());
    assert_eq!(counts(), (0, 0, 0));
    assert_eq!(short.bytes_left, good.len() - 1);
}

#[test]
fn image_path_properties_and_standalone_records_use_one_work_allowance() {
    let mut vectors = Vec::new();
    for value in [1, 0, 1] {
        word(&mut vectors, value);
    }
    name(&mut vectors, "modern");
    for value in [0, 0, 0, 0, 1] {
        word(&mut vectors, value);
    }
    stroke(&mut vectors, 1);
    let mut legacy = Vec::new();
    for value in [0, 1] {
        word(&mut legacy, value);
    }
    name(&mut legacy, "legacy");
    word(&mut legacy, 0);
    legacy.push(0);
    for value in [0, 2, 1, 1, 0, 0, 2, 1, 1] {
        word(&mut legacy, value);
    }
    let standalone = record("standalone", 1, false);
    let mut work = DecodeWork { bytes_left: vectors.len() + legacy.len() + standalone.len() };
    reset();
    assert_eq!(vectors_with_work(&vectors, &mut work, &Interrupt::NONE).unwrap().unwrap()[0].named.name, "modern");
    assert_eq!(legacy_with_work(&legacy, &mut work, &Interrupt::NONE).unwrap().unwrap().0[0].named.name, "legacy");
    assert_eq!(standalone_with_work(&cursor(&standalone), 0, &mut work, &Interrupt::NONE).unwrap().unwrap().named.name, "standalone");
    assert_eq!((work.bytes_left, counts()), (0, (3, 3, 8)));
    assert!(vectors_with_work(&vectors, &mut work, &Interrupt::NONE).unwrap().is_none());
    assert!(legacy_with_work(&legacy, &mut work, &Interrupt::NONE).unwrap().is_none());
    assert_eq!(counts(), (3, 3, 8));
}

#[test]
fn malformed_image_path_properties_do_not_refund_source_work() {
    let malformed = 0u32.to_be_bytes();
    let mut work = DecodeWork { bytes_left: malformed.len() * 2 };
    reset();
    assert!(vectors_with_work(&malformed, &mut work, &Interrupt::NONE).is_err());
    assert!(legacy_with_work(&malformed, &mut work, &Interrupt::NONE).is_err());
    assert_eq!((work.bytes_left, counts()), (0, (2, 0, 0)));
    assert!(vectors_with_work(&malformed, &mut work, &Interrupt::NONE).unwrap().is_none());
    assert_eq!(counts(), (2, 0, 0));
}

#[test]
fn cancellation_wins_before_admission_and_during_geometry_without_refunds() {
    let bytes = record("cancel", 2, false);
    let mut work = DecodeWork { bytes_left: bytes.len() };
    reset();
    let cancel = || true;
    assert!(matches!(standalone_with_work(&cursor(&bytes), 0, &mut work, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
    assert_eq!((work.bytes_left, counts()), (bytes.len(), (0, 0, 0)));
    let cancel = || PATH_POINTS.with(|count| count.get() >= 3);
    assert!(matches!(standalone_with_work(&cursor(&bytes), 0, &mut work, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
    assert_eq!((work.bytes_left, counts()), (0, (1, 1, 3)));
}

fn image(bytes: &[u8]) -> Image<'_> {
    Image {
        c: cursor(bytes),
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: MAX_PIXEL_BYTES,
        meta_budget: MAX_METADATA_BYTES,
        name_work_left: crate::xcf::MAX_METADATA_BYTES,
        vector_style_work_left: crate::xcf::MAX_METADATA_BYTES,
        link_work_left: crate::xcf::MAX_METADATA_BYTES,
        effect_decode_work: Default::default(),
        text_decode_work: Default::default(),
        parasite_work: Default::default(),
        report_parasite_work: Default::default(),
        point_text_work_left: crate::xcf::text::POINT_LAYOUT_WORK_UNITS,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        source_resolution: (72., 72.),
        pending_effects: Vec::new(),
        path_tattoos: std::collections::HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}

#[test]
fn file_path_work_fallback_preserves_saved_pixels_source_report_and_native_persistence() {
    use photocraft_doc::import_conversion::ImportConversion;
    let good = record("known", 1, false);
    let bad = record("failed", 1, true);
    let mut bytes = vec![0; 4];
    let good_offset = bytes.len();
    bytes.extend_from_slice(&good);
    let bad_offset = bytes.len();
    bytes.extend_from_slice(&bad);
    let mut work = DecodeWork { bytes_left: good.len() + 2 * bad.len() };
    let mut image = image(&bytes);
    let mut document = Document::new("Usable pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Saved raster", PixelFormat::RGBA8);
    let LayerContent::Raster(pixels) = &mut layer.content else { panic!("raster") };
    let expected = vec![17. / 255., 31. / 255., 57. / 255., 1.];
    pixels.write_region(document.bounds(), &expected);
    document.layers.push(layer);
    reset();
    import_paths_with_work(&mut document, &mut image, &[], &[good_offset, bad_offset, bad_offset, bad_offset], None, &mut work, &Interrupt::NONE).unwrap();
    assert_eq!((work.bytes_left, counts()), (0, (3, 3, 9)));
    assert_eq!(document.paths.len(), 1);
    assert_eq!(document.paths[0].name, "known");
    assert!(image.initial_view.selected_path.is_none());
    assert!(image.path_tattoos.values().all(Option::is_none), "an incomplete inventory cannot bind later vector owners");
    assert!(image.meta_budget > MAX_METADATA_BYTES / 2, "failure work cannot rely on retained native storage");
    assert!(image.warnings.finish().iter().any(|warning| warning.contains("admission limit")));
    let report = image.outcomes.finish().unwrap();
    let ImportConversion::Report(details) = &report else { panic!("report") };
    assert!(details.entries.iter().any(|entry| entry.feature_code == "xcf.path" && entry.reason == ConversionReason::RetentionLimit));
    assert!(details.attachments.iter().any(|attachment| attachment.bytes.windows(bad.len()).any(|part| part == bad.as_slice())));
    document.metadata.import_conversion = Some(std::sync::Arc::new(report));
    assert_eq!(document.layers[0].surface().unwrap().read_region(document.bounds()), expected);
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.paths, document.paths);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
}
