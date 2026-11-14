//! Rejected optional effect metadata must not consume pixels or permanently spend discarded storage.
use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

fn image(bytes: &[u8], metadata: usize) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: MAX_PIXEL_BYTES,
        meta_budget: metadata,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: 72.,
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}

fn pointer_list(w: &mut W, pointers: &[usize]) -> usize {
    let offset = w.out.len();
    for &pointer in pointers {
        w.ptr(pointer);
    }
    w.ptr(0);
    offset
}

fn channel(w: &mut W, pointers: &[usize]) -> usize {
    let offset = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Coverage");
    w.end();
    let raster = w.slot();
    pointer_list(w, pointers);
    let at = hierarchy(w, 1, 1, 1, &[128], Compression::None);
    w.patch(raster, at);
    offset
}

fn layer(w: &mut W, mask: usize, pointers: &[usize]) -> usize {
    let offset = w.out.len();
    for word in [1, 1, 1] {
        w.u32(word);
    }
    w.string("Raster");
    w.end();
    let raster = w.slot();
    w.ptr(mask);
    pointer_list(w, pointers);
    let at = hierarchy(w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(raster, at);
    offset
}

fn persist(document: &mut Document, image: Image<'_>) {
    document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish().unwrap()));
    let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), *document);
}

#[test]
fn duplicate_terminated_effect_lists_preserve_raster_mask_channel_and_native_pixels() {
    let mut w = W { out: vec![0; 4], wide: true };
    let mask = channel(&mut w, &[1, 1]);
    let alpha = channel(&mut w, &[2, 2]);
    let first = layer(&mut w, mask, &[3, 3]);
    let later = layer(&mut w, 0, &[]);
    let mut image = image(&w.out, 4096);
    let (layers, _) = image.import_layers(&[first, later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    let (channels, selection) = image.import_channels(&[alpha], PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert!(selection.is_none());
    assert_eq!(layers.len(), 2);
    for layer in &layers {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    assert_eq!(layers.iter().find_map(|layer| layer.mask.as_ref()).unwrap().surface.pixel(0, 0), vec![128. / 255.]);
    assert_eq!(channels[0].surface.pixel(0, 0), vec![128. / 255.]);
    let mut document = Document::new("Usable pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers;
    document.channels = channels;
    persist(&mut document, image);
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("report") };
    let lists: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.effect_list").collect();
    assert_eq!(lists.len(), 3);
    assert!(lists.iter().all(|entry| entry.reason == ConversionReason::InvalidOptionalData && entry.source_description.contains("repeated")));
    assert_eq!(lists.iter().filter_map(|entry| entry.source.record_offset).collect::<std::collections::HashSet<_>>().len(), 3);
    assert!(!report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit));
}

#[test]
fn optional_pointer_error_refunds_only_rejected_list_and_preserves_accepted_and_cancelled_charges() {
    let mut w = W { out: vec![0; 4], wide: true };
    let duplicate = pointer_list(&mut w, &[1, 1]);
    let accepted = pointer_list(&mut w, &[1, 2]);
    let mut image = image(&w.out, 1024);
    let mut duplicate_cursor = image.c.at(duplicate);
    let (pointers, limited) = image.optional_effect_pointers(&mut duplicate_cursor, &Interrupt::NONE).unwrap();
    assert!(pointers.is_empty() && !limited);
    assert_eq!(image.meta_budget, 1024);
    let mut accepted_cursor = image.c.at(accepted);
    let (pointers, limited) = image.optional_effect_pointers(&mut accepted_cursor, &Interrupt::NONE).unwrap();
    assert_eq!(pointers, [1, 2]);
    assert!(!limited);
    let charge = 4 * std::mem::size_of::<usize>().max(std::mem::size_of::<u64>());
    assert_eq!(image.meta_budget, 1024 - 2 * charge);
    let mut second_duplicate = image.c.at(duplicate);
    image.optional_effect_pointers(&mut second_duplicate, &Interrupt::NONE).unwrap();
    assert_eq!(image.meta_budget, 1024 - 2 * charge, "a later rejected list cannot refund an earlier accepted allocation");
    assert_eq!(pointers, [1, 2]);
    let mut truncated = self::image(&w.out[..accepted + 8 + 3], 1024);
    let mut truncated_cursor = truncated.c.at(accepted);
    let (rejected, limited) = truncated.optional_effect_pointers(&mut truncated_cursor, &Interrupt::NONE).unwrap();
    assert!(rejected.is_empty() && !limited);
    assert_eq!(truncated.meta_budget, 1024, "unreadable optional framing is an accurate invalid-data omission too");
    let mut cancelled = self::image(&w.out, 1024);
    let checks = std::sync::atomic::AtomicUsize::new(0);
    // Three pre-scan checks, then one admitted pointer, then cancellation.
    let stop = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4;
    let mut cancelled_cursor = cancelled.c.at(accepted);
    assert!(matches!(cancelled.optional_effect_pointers(&mut cancelled_cursor, &Interrupt::cancel_only(&stop)), Err(IoError::Cancelled)));
    assert_eq!(cancelled.meta_budget, 1024 - charge, "typed cancellation does not become a handled parse-error refund");
    let ImportConversion::Report(report) = cancelled.outcomes.finish().unwrap() else { panic!("report") };
    assert!(report.entries.is_empty(), "cancelled source did not become an invalid-data disclosure");
}

fn effect(w: &mut W, values: &[(&str, bool)]) -> usize {
    let offset = w.out.len();
    for value in ["Optional effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    for &(raw, trailing) in values {
        let mut value = W { out: Vec::new(), wide: false };
        value.string("settings");
        value.u32(6);
        value.string(raw);
        if trailing {
            value.out.push(99);
        }
        w.prop(45, &value.out);
    }
    w.end();
    w.ptr(0);
    offset
}

fn decode_cost(bytes: &[u8], offset: usize) -> usize {
    let mut remaining = usize::MAX;
    effects::decode(&Cursor { b: bytes, pos: 0, wide: true }, offset, 26, &mut remaining, &Interrupt::NONE).unwrap();
    usize::MAX - remaining
}

#[test]
fn discarded_invalid_syntax_and_trailing_config_charge_only_retained_diagnostics() {
    let mut w = W { out: vec![0; 4], wide: true };
    let small_bad = effect(&mut w, &[("(", false)]);
    let large_bad_raw = format!("(label \"{}\") (", "x".repeat(128));
    let large_bad = effect(&mut w, &[(&large_bad_raw, false)]);
    assert_eq!(decode_cost(&w.out, small_bad), decode_cost(&w.out, large_bad), "discarded malformed tree storage is not retained");
    let small_tail = effect(&mut w, &[("", true)]);
    let raw = "(label \"owned configuration\")";
    let large_tail = effect(&mut w, &[(raw, true)]);
    assert_eq!(decode_cost(&w.out, small_tail), decode_cost(&w.out, large_tail), "a successfully parsed tree discarded for trailing bytes is not retained");
    let empty = effect(&mut w, &[("", false)]);
    let accepted = effect(&mut w, &[(raw, false)]);
    let mut remaining = usize::MAX;
    effects::parse_config(raw, &mut remaining).unwrap();
    assert_eq!(
        decode_cost(&w.out, accepted),
        decode_cost(&w.out, empty) + (usize::MAX - remaining),
        "accepted config retains its unchanged tree/string charge"
    );
    let cursor = Cursor { b: &w.out, pos: 0, wide: true };
    let parsed = effects::decode(&cursor, large_bad, 26, &mut 4096, &Interrupt::NONE).unwrap();
    assert!(matches!(&parsed.arguments[0].value, effects::ArgumentValue::Invalid { .. }));
    assert!(matches!(effects::decode(&cursor, accepted, 26, &mut 1, &Interrupt::NONE), Err(IoError::XcfMetadataLimit)));
    assert!(matches!(effects::decode(&cursor, accepted, 26, &mut 4096, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
}

#[test]
fn repeated_discarded_configs_leave_the_exact_retained_allowance_for_accepted_data_and_later_pixels() {
    let mut w = W { out: vec![0; 4], wide: true };
    let raw = "(label \"bounded owned configuration\")";
    let none = effect(&mut w, &[]);
    let short_tail = effect(&mut w, &[("", true)]);
    let accepted = effect(&mut w, &[(raw, false)]);
    let repeated = effect(&mut w, &[(raw, true), (raw, true), (raw, true), (raw, false)]);
    let later = layer(&mut w, 0, &[]);
    let header = decode_cost(&w.out, none);
    let invalid_cost = decode_cost(&w.out, short_tail) - header;
    let accepted_cost = decode_cost(&w.out, accepted) - header;
    let expected_retained = header + 3 * invalid_cost + accepted_cost;
    let mut remaining = usize::MAX;
    effects::parse_config(raw, &mut remaining).unwrap();
    let provisional = usize::MAX - remaining;
    let budget = expected_retained + provisional;
    let cursor = Cursor { b: &w.out, pos: 0, wide: true };
    let mut remaining = budget;
    let decoded = effects::decode(&cursor, repeated, 26, &mut remaining, &Interrupt::NONE).unwrap();
    assert_eq!(budget - remaining, expected_retained);
    assert_eq!(decoded.arguments.len(), 4);
    assert!(decoded.arguments.iter().take(3).all(|argument| matches!(&argument.value, effects::ArgumentValue::Invalid { .. })));
    assert!(matches!(&decoded.arguments[3].value, effects::ArgumentValue::Config(_)));
    let allowance = 6 * std::mem::size_of::<usize>() + 8 * std::mem::size_of::<usize>() + budget;
    let mut image = image(&w.out, allowance);
    let owner = image
        .outcomes
        .push(ConversionEntry {
            feature_code: "xcf.channel".into(),
            source: conversion::source("channel", 0, 0, "Owner", &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: "Stored coverage".into(),
            native_description: "Native coverage".into(),
            attachments: Vec::new(),
        })
        .unwrap();
    image.inventory_coverage_effects(owner, &[repeated], &Interrupt::NONE).unwrap();
    let (layers, _) = image.import_layers(&[later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert_eq!(layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    let mut document = Document::new("Later raster", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers;
    persist(&mut document, image);
}
