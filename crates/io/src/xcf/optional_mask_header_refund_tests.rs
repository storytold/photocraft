use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

const META_ALLOWANCE: usize = 64 * 1024;

fn image(bytes: &[u8]) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: 16 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &PixelFormat::RGBA8)).unwrap(),
        meta_budget: META_ALLOWANCE,
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

fn owner(image: &mut Image<'_>, ordinal: usize) -> usize {
    image
        .outcomes
        .push(ConversionEntry {
            feature_code: "xcf.channel".into(),
            source: conversion::source("channel", ordinal, 0, "Coverage", &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: "Saved native coverage".into(),
            native_description: "Native coverage retained".into(),
            attachments: Vec::new(),
        })
        .unwrap()
}

fn effect(w: &mut W, mask: usize) -> usize {
    let at = w.out.len();
    for text in ["Coverage", "", "gegl:nop", "1"] {
        w.string(text);
    }
    w.end();
    w.ptr(mask);
    at
}

fn mask(w: &mut W, children: &[usize], terminated: bool) -> usize {
    let at = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Mask");
    w.end();
    w.ptr(usize::MAX); // Auxiliary pixels must never be followed.
    for child in children {
        w.ptr(*child);
    }
    if terminated {
        w.ptr(0);
    }
    at
}

fn fixtures() -> (Vec<u8>, usize, usize, usize, usize) {
    let mut w = W { out: vec![0; 4], wide: true };
    let plain = effect(&mut w, 0);
    let affected = effect(&mut w, 0);
    let mask_slot = w.out.len() - 8;
    let layer = w.out.len();
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string("Later raster");
    w.end();
    let pixels = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(pixels, at);
    // Exactly 1,024 distinct child pointers consume provisional storage before
    // the next pointer read reaches EOF. Repeat this same malformed header per owner.
    let malformed = mask(&mut w, &(1..=1024).map(|value| 100_000 + value).collect::<Vec<_>>(), false);
    w.patch(mask_slot, malformed);
    (w.out, plain, affected, layer, malformed)
}

#[test]
fn malformed_auxiliary_header_refunds_only_provisional_metadata_for_repeated_owners() {
    let (bytes, plain, affected, layer, malformed) = fixtures();
    let mut broken = image(&bytes);
    let mut control = image(&bytes);
    for ordinal in 0..4 {
        let owner = owner(&mut broken, ordinal);
        let control_owner = self::owner(&mut control, ordinal);
        broken.inventory_coverage_effects(owner, &[affected], &Interrupt::NONE).unwrap();
        control.inventory_coverage_effects(control_owner, &[plain], &Interrupt::NONE).unwrap();
        assert!(control.meta_budget < META_ALLOWANCE, "outer traversal and decoded effect charges remain");
        assert_eq!(broken.meta_budget, control.meta_budget, "failed header retains no allocation on owner {ordinal}");
    }
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let (layers, _) = broken.import_layers(&[layer], PixelFormat::RGBA8, format, &Interrupt::NONE).unwrap();
    assert_eq!(layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(broken.budget, 0, "later raster retains its exact pixel allowance");
    let conversion = broken.outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("typed report") };
    report.validate().unwrap();
    let masks: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect_mask").collect();
    assert_eq!(masks.len(), 4);
    assert!(masks.iter().all(|entry| entry.reason == ConversionReason::InvalidOptionalData
        && entry.disposition == ConversionDisposition::SkippedNonessential
        && entry.source.record_offset == Some(malformed as u64)));
    let mut document = Document::new("Refunded optional header", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers;
    document.metadata.import_conversion = Some(Arc::new(conversion));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().metadata.import_conversion, document.metadata.import_conversion);
}

#[test]
fn successful_auxiliary_header_keeps_its_child_and_nested_traversal_charges() {
    let mut w = W { out: vec![0; 4], wide: true };
    let child = effect(&mut w, 0);
    let header = mask(&mut w, &[child], true);
    let parent = effect(&mut w, header);
    let plain_parent = effect(&mut w, 0);
    let mut nested = image(&w.out);
    let mut control = image(&w.out);
    let nested_owner = owner(&mut nested, 0);
    let control_owner = owner(&mut control, 0);
    nested.inventory_coverage_effects(nested_owner, &[parent], &Interrupt::NONE).unwrap();
    control.inventory_coverage_effects(control_owner, &[plain_parent, child], &Interrupt::NONE).unwrap();
    let header_cost = 4 * std::mem::size_of::<usize>().max(std::mem::size_of::<u64>()) + "Mask".len() + std::mem::size_of::<CoverageMaskHeader<'_>>();
    assert_eq!(nested.meta_budget, control.meta_budget - header_cost);
    let ImportConversion::Report(report) = nested.outcomes.finish().unwrap() else { panic!("typed report") };
    assert_eq!(report.entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").count(), 2);
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect" && entry.source.record_offset == Some(child as u64)));
}

#[test]
fn cancellation_during_auxiliary_pointer_read_propagates_without_refunding_or_fallback() {
    let (bytes, plain, affected, _, _) = fixtures();
    let mut cancelled = image(&bytes);
    let mut control = image(&bytes);
    let owner = owner(&mut cancelled, 0);
    let control_owner = self::owner(&mut control, 0);
    control.inventory_coverage_effects(control_owner, &[plain], &Interrupt::NONE).unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let stop = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 127;
    assert!(matches!(cancelled.inventory_coverage_effects(owner, &[affected], &Interrupt::cancel_only(&stop)), Err(IoError::Cancelled)));
    assert!(cancelled.meta_budget < control.meta_budget, "cancelled decode is not the handled optional error refund");
    let ImportConversion::Report(report) = cancelled.outcomes.finish().unwrap() else { panic!("typed report") };
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect_mask"), "cancellation is never an optional-data fallback");
}
