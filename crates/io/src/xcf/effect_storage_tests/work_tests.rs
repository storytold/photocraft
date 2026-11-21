use super::*;

fn probe(bytes: &[u8], pointer: usize, malformed: bool) -> (usize, usize) {
    let mut work = effects::DecodeWork::with_limit(usize::MAX);
    let mut storage = PROBE_ALLOWANCE;
    let result = effects::decode_with_work(&Cursor { b: bytes, pos: 0, wide: true }, pointer, 26, &mut storage, &mut work, &Interrupt::NONE);
    if malformed {
        assert!(result.unwrap_err().to_string().contains("terminator"));
    } else {
        assert!(result.is_ok());
    }
    let (remaining, config_bytes) = work.stats();
    assert!(config_bytes > 0);
    (usize::MAX - remaining, config_bytes)
}

#[test]
fn rejected_effect_work_is_not_refunded_across_layer_and_coverage_owners() {
    let (bytes, effect, later) = fixture(true);
    let (per_record, config_bytes) = probe(&bytes, effect, true);
    let mut image = image(&bytes, PROBE_ALLOWANCE);
    image.effect_decode_work = effects::DecodeWork::with_limit(per_record * 2 + per_record / 2);
    let mut document = Document::new("Bounded repeated rejection", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    for ordinal in 0..12 {
        let layer = Layer::raster("Transparent owner", PixelFormat::RGBA8);
        let id = layer.id;
        document.layers.push(layer);
        let source = owner(&mut image, ordinal, Some(id));
        image.pending_effects.push(PendingLayerEffects { layer: id, owner: source, bounds: Rect::new(0, 0, 1, 1), pointers: vec![effect] });
    }
    let storage = image.meta_budget;
    import_native_effects(&mut document, &mut image, &Interrupt::NONE).unwrap();
    assert_eq!(image.meta_budget, storage, "all discarded effect storage is still refunded exactly");
    let (remaining, parsed) = image.effect_decode_work.stats();
    assert_eq!(remaining, 0);
    assert!(parsed >= config_bytes * 2 && parsed <= config_bytes * 3, "repeated records cannot repeatedly parse their configs: {parsed}");
    let source = owner(&mut image, 12, None);
    image.inventory_coverage_effects(source, &[effect], &Interrupt::NONE).unwrap();
    assert_eq!(image.effect_decode_work.stats(), (0, parsed), "coverage shares the spent layer allowance");
    document.layers.push(admit_later_raster(&mut image, later));
    let conversion = image.outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("durable report") };
    for feature in ["xcf.layer_effect", "xcf.coverage_effect"] {
        assert!(
            report.entries.iter().any(|entry| entry.feature_code == feature
                && entry.reason == ConversionReason::RetentionLimit
                && entry.source_description.contains("decode-work allowance")),
            "{feature}"
        );
    }
    document.metadata.import_conversion = Some(Arc::new(conversion));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), document);
}

#[test]
fn distinct_effect_offsets_share_cumulative_work_in_coverage_inventory() {
    let (original, effect, later) = fixture(true);
    let record = &original[effect..later];
    let mut bytes = vec![0; 4];
    let pointers: Vec<_> = (0..12)
        .map(|_| {
            let pointer = bytes.len();
            bytes.extend_from_slice(record);
            pointer
        })
        .collect();
    let (per_record, config_bytes) = probe(&bytes, pointers[0], true);
    let mut image = image(&bytes, PROBE_ALLOWANCE);
    image.effect_decode_work = effects::DecodeWork::with_limit(per_record * 2 + per_record / 2);
    for (ordinal, pointer) in pointers.iter().enumerate() {
        let source = owner(&mut image, ordinal, None);
        image.inventory_coverage_effects(source, &[*pointer], &Interrupt::NONE).unwrap();
    }
    assert_eq!(image.effect_decode_work.stats().0, 0);
    assert!(image.effect_decode_work.stats().1 <= config_bytes * 3, "unique offsets cannot bypass the shared byte-work allowance");
    assert_eq!(image.meta_budget, PROBE_ALLOWANCE - pointers.len() * 14 * std::mem::size_of::<usize>(), "only traversal storage remains charged");
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("durable report") };
    assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit));
}

#[test]
fn successful_effects_consume_work_and_keep_existing_storage_charges() {
    let (bytes, effect, _) = fixture(false);
    let (per_record, config_bytes) = probe(&bytes, effect, false);
    let storage_per_record = decoded_charge(&bytes, effect, false);
    let mut work = effects::DecodeWork::with_limit(per_record * 3);
    let mut storage = PROBE_ALLOWANCE;
    for _ in 0..2 {
        let decoded = effects::decode_with_work(&Cursor { b: &bytes, pos: 0, wide: true }, effect, 26, &mut storage, &mut work, &Interrupt::NONE).unwrap();
        assert!(matches!(&decoded.arguments[0].value, effects::ArgumentValue::Config(_)));
    }
    assert_eq!(storage, PROBE_ALLOWANCE - storage_per_record * 2);
    assert_eq!(work.stats(), (per_record, config_bytes * 2));
}

#[test]
fn normal_shared_effects_allow_inventory_and_materialization_with_exact_storage_accounting() {
    let (bytes, effect, _) = fixture(false);
    let (per_record, config_bytes) = probe(&bytes, effect, false);
    let retained = decoded_charge(&bytes, effect, false);
    assert_eq!(effects::DecodeWork::default().stats().0, MAX_METADATA_BYTES * 4);
    assert!(per_record <= retained * 4, "ordinary quoted configuration stays within the work-to-storage allowance, including envelope overhead");
    const OWNERS: usize = 8;
    let mut image = image(&bytes, PROBE_ALLOWANCE);
    image.effect_decode_work = effects::DecodeWork::with_limit(per_record * OWNERS * 2);
    let mut document = Document::new("Shared valid effects", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    for ordinal in 0..OWNERS {
        let coverage = owner(&mut image, ordinal, None);
        image.inventory_coverage_effects(coverage, &[effect], &Interrupt::NONE).unwrap();
        let layer = Layer::raster("Native owner", PixelFormat::RGBA8);
        let id = layer.id;
        document.layers.push(layer);
        let native = owner(&mut image, ordinal, Some(id));
        image.pending_effects.push(PendingLayerEffects { layer: id, owner: native, bounds: Rect::new(0, 0, 1, 1), pointers: vec![effect] });
    }
    import_native_effects(&mut document, &mut image, &Interrupt::NONE).unwrap();
    assert_eq!(image.effect_decode_work.stats(), (0, config_bytes * OWNERS * 2));
    assert_eq!(image.meta_budget, PROBE_ALLOWANCE - retained * OWNERS * 2 - OWNERS * 14 * std::mem::size_of::<usize>());
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("durable report") };
    assert!(
        !report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit),
        "the exact shared work boundary accepts the final normal record"
    );
}

#[test]
fn auxiliary_mask_names_share_work_even_when_retention_uses_a_short_fallback() {
    let name = "x".repeat(PROBE_ALLOWANCE + 1024);
    let mut w = W { out: vec![0; 4], wide: true };
    let mask = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string(&name);
    w.end();
    w.ptr(usize::MAX); // Header inventory must not decode the hierarchy.
    w.ptr(0);
    let mut image = image(&w.out, PROBE_ALLOWANCE);
    let per_record = 64 + name.len() + 4 + 8 + 32;
    image.effect_decode_work = effects::DecodeWork::with_limit(per_record * 2 + per_record / 2);
    let mut admitted = 0;
    for _ in 0..12 {
        match image.coverage_mask_header(mask, &Interrupt::NONE) {
            Ok(record) => {
                assert!(record.name_limited);
                assert_eq!(record.name, "Mask");
                admitted += 1;
            }
            Err(IoError::XcfMetadataLimit) => {}
            _ => panic!("mask-header work should be admitted or limited"),
        }
    }
    assert_eq!(admitted, 2);
    assert_eq!(image.effect_decode_work.stats(), (0, 0));
}

fn late_mask_failure(kind: u8, distinct: bool) -> (Vec<u8>, Vec<usize>) {
    let (original, effect, later) = fixture(false);
    let record = &original[effect..later];
    let mut w = W { out: vec![0; 4], wide: true };
    let mut parents = Vec::new();
    for _ in 0..if distinct { 12 } else { 1 } {
        let parent = w.out.len();
        w.out.extend_from_slice(record);
        let mask = w.out.len();
        w.patch(parent + record.len() - 8, mask);
        w.u32(1);
        w.u32(1);
        w.string("Mask");
        if kind == 0 {
            for _ in 0..1024 {
                w.prop(8, &1u32.to_be_bytes());
            }
            w.u32(99);
            w.u32((MAX_PAYLOAD + 1) as u32); // Late invalid length, independent of subsequent records.
        } else {
            w.end();
            w.ptr(usize::MAX); // Never decode auxiliary hierarchy samples.
            for pointer in 1..=32 {
                w.ptr(pointer);
            }
            if kind == 1 {
                w.ptr(1); // Late duplicate: HashSet lookup, not a quadratic Vec scan.
            } else {
                w.out.extend_from_slice(&[0, 0, 0]); // Truncated wide pointer at EOF.
            }
        }
        parents.push(parent);
    }
    if !distinct {
        parents.resize(12, parents[0]);
    }
    (w.out, parents)
}

#[test]
fn late_mask_property_and_pointer_failures_spend_work_before_refundable_storage() {
    let (original, effect, _) = fixture(false);
    let (effect_work, _) = probe(&original, effect, false);
    let retained = decoded_charge(&original, effect, false);
    for (kind, distinct) in [(0, false), (0, true), (1, false), (1, true), (2, false)] {
        let (bytes, parents) = late_mask_failure(kind, distinct);
        let mask_work = if kind == 0 { 64 + 8 + 1024 * 12 + 8 } else { 64 + 8 + 8 + 8 + 33 * 24 };
        let mut image = image(&bytes, PROBE_ALLOWANCE);
        image.effect_decode_work = effects::DecodeWork::with_limit(2 * (effect_work + mask_work) + 64);
        wire::PROPERTY_READS.with(|count| count.set(0));
        effects::POINTER_READS.with(|count| count.set(0));
        for (ordinal, pointer) in parents.iter().enumerate() {
            let source = owner(&mut image, ordinal, None);
            image.inventory_coverage_effects(source, &[*pointer], &Interrupt::NONE).unwrap();
        }
        assert_eq!(image.effect_decode_work.stats().0, 0);
        let properties = wire::PROPERTY_READS.with(std::cell::Cell::get);
        let pointers = effects::POINTER_READS.with(std::cell::Cell::get);
        if kind == 0 {
            assert_eq!(properties, 2 * 1025, "late invalid properties: distinct={distinct}");
            assert_eq!(pointers, 0);
        } else {
            assert_eq!(properties, 2);
            assert_eq!(pointers, 2 * 33, "late duplicate/truncated pointers: kind={kind}, distinct={distinct}");
        }
        assert_eq!(
            image.meta_budget,
            PROBE_ALLOWANCE - 2 * retained - parents.len() * 14 * std::mem::size_of::<usize>(),
            "failed mask storage stays fully refunded"
        );
        let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("durable report") };
        assert!(
            report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit && entry.source_description.contains("decode-work allowance"))
        );
    }
}

#[test]
fn valid_mask_properties_and_pointer_terminator_succeed_at_the_exact_work_boundary() {
    let mut w = W { out: vec![0; 4], wide: true };
    let mask = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Mask");
    for _ in 0..4 {
        w.prop(8, &1u32.to_be_bytes());
    }
    w.end();
    w.ptr(usize::MAX);
    for pointer in [1, 2, 3, 0] {
        w.ptr(pointer);
    }
    let mut image = image(&w.out, PROBE_ALLOWANCE);
    image.effect_decode_work = effects::DecodeWork::with_limit(64 + 8 + 4 * 12 + 8 + 8 + 4 * 24);
    let record = image.coverage_mask_header(mask, &Interrupt::NONE).unwrap();
    assert_eq!(record.name, "Mask");
    assert_eq!(record.properties.len(), 4);
    assert_eq!(record.children, [1, 2, 3]);
    assert_eq!(image.effect_decode_work.stats().0, 0);
    assert_eq!(
        image.meta_budget,
        PROBE_ALLOWANCE - record.name.capacity() - std::mem::size_of::<CoverageMaskHeader<'_>>() - 4 * std::mem::size_of::<Prop<'_>>() - 3 * 32
    );
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(image.coverage_mask_header(mask, &cancelled), Err(IoError::Cancelled)));
}

#[test]
fn effect_decode_work_preserves_early_and_mid_record_cancellation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (bytes, effect, _) = fixture(false);
    for mid_record in [false, true] {
        let checks = AtomicUsize::new(0);
        let cancel = || !mid_record || checks.fetch_add(1, Ordering::Relaxed) >= 2;
        let ctl = Interrupt::cancel_only(&cancel);
        let mut work = effects::DecodeWork::with_limit(PROBE_ALLOWANCE);
        let mut storage = PROBE_ALLOWANCE;
        let result = effects::decode_with_work(&Cursor { b: &bytes, pos: 0, wide: true }, effect, 26, &mut storage, &mut work, &ctl);
        assert!(matches!(result, Err(IoError::Cancelled)));
        if mid_record {
            assert!(work.stats().0 < PROBE_ALLOWANCE && work.stats().1 > 0, "completed work stays charged on cancellation");
        } else {
            assert_eq!(work.stats(), (PROBE_ALLOWANCE, 0));
            assert_eq!(storage, PROBE_ALLOWANCE);
        }
    }
    let mut spent = effects::DecodeWork::with_limit(0);
    let mut storage = PROBE_ALLOWANCE;
    assert!(
        matches!(
            effects::decode_with_work(&Cursor { b: &bytes, pos: 0, wide: true }, effect, 26, &mut storage, &mut spent, &Interrupt::cancel_only(&|| true),),
            Err(IoError::Cancelled)
        ),
        "cancellation wins even after the work allowance is spent"
    );
    assert_eq!(storage, PROBE_ALLOWANCE);
}
