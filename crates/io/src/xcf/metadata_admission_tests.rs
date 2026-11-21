//! Optional metadata admission must leave independently framed raster/coverage samples usable.
use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

fn image(bytes: &[u8], metadata: usize) -> Image<'_> {
    let mut image = super::tests::decoder_image(bytes);
    image.meta_budget = metadata;
    image
}

fn layer(w: &mut W, name: &str, properties: &[(u32, Vec<u8>)], mask: usize) -> usize {
    let at = w.out.len();
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string(name);
    for (kind, bytes) in properties {
        w.prop(*kind, bytes);
    }
    w.end();
    let raster = w.slot();
    w.ptr(mask);
    w.ptr(0);
    let at_pixels = hierarchy(w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(raster, at_pixels);
    at
}

fn channel(w: &mut W, name: &str, children: &[usize]) -> usize {
    let at = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string(name);
    w.end();
    let raster = w.slot();
    for child in children {
        w.ptr(*child);
    }
    w.ptr(0);
    let at_pixels = hierarchy(w, 1, 1, 1, &[128], Compression::None);
    w.patch(raster, at_pixels);
    at
}

fn vector_style() -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    for value in [0, 7, 1, 0] {
        w.u32(value);
    }
    for _ in 0..2 {
        w.u32(0);
        w.u32(1);
        w.string("R'G'B'A u8");
        w.u32(4);
        w.out.extend([255, 0, 0, 255]);
        w.u32(0);
        w.string("");
    }
    for value in [1f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
        w.u32(value);
    }
    w.out
}

fn coverage_owner(image: &mut Image<'_>) -> usize {
    image
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
        .unwrap()
}

#[test]
fn oversized_names_skip_validation_and_rejected_lossy_names_spend_work() {
    let mut w = W { out: Vec::new(), wide: true };
    w.string("Too long");
    let lossy = w.out.len();
    w.u32(5);
    w.out.extend([b'a', b'b', b'c', 0xff, 0]);
    let short = w.out.len();
    w.string("OK");
    let mut image = image(&w.out, 5);
    image.name_work_left = 6;

    let mut cursor = image.c.at(0);
    assert_eq!(image.metadata_name(&mut cursor, "Layer").unwrap(), ("Layer".into(), true));
    assert_eq!(cursor.pos, lossy);
    assert_eq!(image.name_work_left, 6, "over-budget names consume no validation work");

    assert_eq!(image.metadata_name(&mut cursor, "Layer").unwrap(), ("Layer".into(), true));
    assert_eq!(image.name_work_left, 2, "a lossy name may fail storage admission after validation");
    assert_eq!(image.meta_budget, 5);

    image.meta_budget = 32;
    let mut cursor = image.c.at(lossy);
    assert_eq!(image.metadata_name(&mut cursor, "Channel").unwrap(), ("Channel".into(), true));
    assert_eq!(image.name_work_left, 2, "repeated references cannot resume validation after a refusal");
    assert_eq!(cursor.pos, short);
    assert_eq!(image.metadata_name(&mut cursor, "Mask").unwrap(), ("OK".into(), false));
    assert_eq!(image.name_work_left, 0);
}

#[test]
fn auxiliary_mask_skips_full_wire_width_hierarchy_and_reads_children() {
    let mut w = W { out: Vec::new(), wide: true };
    w.u32(1);
    w.u32(1);
    w.string("Mask");
    w.end();
    let pointer = w.out.len();
    w.out.extend(u64::MAX.to_be_bytes());
    w.ptr(1);
    w.ptr(0);

    let record = image(&w.out, MAX_METADATA_BYTES).coverage_mask_header(0, &Interrupt::NONE).unwrap();
    assert_eq!(record.name, "Mask");
    assert_eq!(record.children, [1]);
    assert!(image(&w.out[..pointer + 7], MAX_METADATA_BYTES).coverage_mask_header(0, &Interrupt::NONE).is_err());
}

#[test]
fn optional_effect_frame_overflow_keeps_groups_without_decoding_hierarchy_or_queuing_effects() {
    fn group(w: &mut W, width: u32, height: u32, x: i32, y: i32) -> usize {
        let effect = effect(w, 0);
        let at = w.out.len();
        for value in [width, height, 1] {
            w.u32(value);
        }
        w.string("Group");
        w.prop(29, &[]);
        let offsets = [x.to_be_bytes(), y.to_be_bytes()].concat();
        w.prop(15, &offsets);
        w.end();
        w.ptr(usize::MAX); // Groups must never follow this deliberately invalid hierarchy.
        w.ptr(0);
        w.ptr(effect);
        w.ptr(0);
        at
    }
    for (width, height, x, y, message) in
        [(i32::MAX as u32 + 1, 1, 0, 0, "width"), (1, u32::MAX, 0, 0, "height"), (2, 1, i32::MAX - 1, 0, "right edge"), (1, 2, 0, i32::MAX - 1, "bottom edge")]
    {
        let mut w = W { out: vec![0; 4], wide: true };
        let ptr = group(&mut w, width, height, x, y);
        let mut image = image(&w.out, MAX_METADATA_BYTES);
        let before = image.budget;
        let (layers, _) = image.import_layers(&[ptr], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
        assert_eq!(layers.len(), 1, "{message}: the optional frame cannot discard its group");
        assert!(layers[0].is_group());
        assert_eq!(image.budget, before, "no group hierarchy pixels are admitted");
        assert!(image.pending_effects.is_empty(), "invalid frames must not queue their effects");
        assert!(image.warnings.finish().iter().any(|warning| warning.contains("effect frame") && warning.contains("pixels preserved")));
        let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("typed report") };
        let frames: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect.frame").collect();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].disposition, ConversionDisposition::SkippedNonessential);
        assert_eq!(frames[0].reason, ConversionReason::InvalidOptionalData);
        assert!(frames[0].targets.iter().any(|target| target.layer_id == Some(layers[0].id)));
    }
    for (width, x, right) in [(i32::MAX as u32, 0, i32::MAX), (i32::MAX as u32, i32::MIN, -1)] {
        let mut w = W { out: vec![0; 4], wide: true };
        let ptr = group(&mut w, width, 1, x, -1);
        let mut image = image(&w.out, MAX_METADATA_BYTES);
        let before = image.budget;
        let (layers, _) = image.import_layers(&[ptr], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
        assert!(layers[0].is_group());
        assert_eq!(image.pending_effects[0].bounds, Rect::new(x, -1, right, 0));
        assert_eq!(image.budget, before);
    }
}

#[test]
fn repeated_auxiliary_mask_names_are_bounded_before_allocation_and_charged_once() {
    let mut w = W { out: vec![0; 4], wide: true };
    let name = "界".repeat(30_000);
    let masks: Vec<_> = (0..24).map(|_| channel(&mut w, &name, &[])).collect();
    let header_bytes = std::mem::size_of::<CoverageMaskHeader<'_>>();
    let allowance = masks.len() * header_bytes;
    let mut bounded = image(&w.out, allowance);
    for (ordinal, offset) in masks.iter().enumerate() {
        let record = bounded.coverage_mask_header(*offset, &Interrupt::NONE).unwrap();
        assert!(record.name_limited);
        assert_eq!(record.name, "Mask");
        assert!(record.name.capacity() <= 16, "no source-sized retained name");
        assert!(record.children.is_empty());
        assert_eq!(bounded.meta_budget, allowance - (ordinal + 1) * header_bytes);
    }
    let mut admitted = image(&w.out, MAX_METADATA_BYTES);
    let before = admitted.meta_budget;
    let record = admitted.coverage_mask_header(masks[0], &Interrupt::NONE).unwrap();
    assert!(!record.name_limited);
    assert_eq!(record.name, name);
    assert_eq!(before - admitted.meta_budget, header_bytes + record.name.capacity(), "admitted names are not double charged");
    let cancelled = Interrupt::cancel_only(&|| true);
    let before = admitted.meta_budget;
    assert!(matches!(admitted.coverage_mask_header(masks[0], &cancelled), Err(IoError::Cancelled)));
    assert_eq!(admitted.meta_budget, before);
}

#[test]
fn omitted_auxiliary_mask_names_keep_inventory_and_later_pixels_with_persistent_loss_report() {
    let mut w = W { out: vec![0; 4], wide: true };
    let child = effect(&mut w, 0);
    let mask = channel(&mut w, &"界".repeat(30_000), &[child]);
    let parent = effect(&mut w, mask);
    let later = layer(&mut w, "Later", &[], 0);
    let mut image = image(&w.out, 16_384);
    let owner = coverage_owner(&mut image);
    image.inventory_coverage_effects(owner, &[parent], &Interrupt::NONE).unwrap();
    let (layers, _) = image.import_layers(&[later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert_eq!(layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert!(
        (0..8).filter_map(|index| image.outcomes.entry(index).ok()).any(|entry| entry.feature_code == "xcf.coverage_effect_mask"),
        "mask configuration remains inventoried"
    );
    assert_eq!(
        (0..8).filter_map(|index| image.outcomes.entry(index).ok()).filter(|entry| entry.feature_code == "xcf.coverage_effect").count(),
        2,
        "the child effect remains inventoried after its mask name is omitted"
    );
    let mut document = Document::new("Retained pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers.into();
    persist(document, image, &["xcf.coverage_effect_mask_name"]);
}

fn persist(mut document: Document, image: Image<'_>, features: &[&str]) {
    assert!(image.warnings.finish().iter().any(|warning| warning.contains("metadata retention limits")));
    let conversion = image.outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("conversion report") };
    report.validate().unwrap();
    for feature in features {
        assert!(
            report.entries.iter().any(|entry| entry.feature_code == *feature && entry.reason == ConversionReason::RetentionLimit),
            "{feature}: {:?}",
            report.entries
        );
    }
    document.metadata.import_conversion = Some(Arc::new(conversion));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), document);
}

#[test]
fn optional_layer_names_paths_and_vector_notes_keep_all_saved_rasters_after_metadata_exhaustion() {
    let style = vector_style();
    assert!(paths::vector_style(&style).is_ok(), "the loss is admission, not malformed source paint");
    let mut w = W { out: vec![0; 4], wide: true };
    let first = layer(&mut w, "First", &[(30, [3u32.to_be_bytes(), 2u32.to_be_bytes()].concat()), (47, style)], 0);
    let later = layer(&mut w, "Later", &[], 0);
    let mut image = image(&w.out, 5); // The first name alone fits; paths/style/later name do not.
    let (layers, _) = image.import_layers(&[first, later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert_eq!(layers.len(), 2);
    for layer in &layers {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    assert!(image.pending_vectors.is_empty(), "an inadmissible editable style cannot replace saved pixels");
    let mut document = Document::new("Usable raster", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers.into();
    persist(document, image, &["xcf.item_path", "xcf.vector_layer", "xcf.layer_name"]);
}

#[test]
fn channel_and_mask_names_and_effect_lists_are_optional_to_valid_coverage_pixels() {
    let mut w = W { out: vec![0; 4], wide: true };
    let mask = channel(&mut w, "Large mask name", &[usize::MAX]);
    let alpha = channel(&mut w, "Large alpha name", &[usize::MAX]);
    let first = layer(&mut w, "First", &[], mask);
    let later = layer(&mut w, "Later", &[], 0);
    let mut image = image(&w.out, 0);
    let (layers, _) = image.import_layers(&[first, later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    let (channels, selection) = image.import_channels(&[alpha], (1, 1), PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert!(selection.is_none());
    assert_eq!(layers.len(), 2);
    let masked = layers.iter().find(|layer| layer.mask.is_some()).unwrap();
    assert_eq!(masked.mask.as_ref().unwrap().surface.pixel(0, 0), vec![128. / 255.]);
    assert_eq!(channels[0].surface.pixel(0, 0), vec![128. / 255.]);
    for layer in &layers {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    let mut document = Document::new("Usable coverage", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers.into();
    document.channels = channels;
    persist(document, image, &["xcf.channel_name", "xcf.mask_name", "xcf.coverage_effect"]);
}

fn effect(w: &mut W, mask: usize) -> usize {
    let at = w.out.len();
    for value in ["Optional effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    w.end();
    w.ptr(mask);
    at
}

#[test]
fn coverage_inventory_exhaustion_at_each_work_stage_preserves_a_later_raster_and_distinguishes_malformed_data() {
    let mut w = W { out: vec![0; 4], wide: true };
    let child = effect(&mut w, 0);
    let mask = channel(&mut w, "Optional mask", &[child]);
    let parent = effect(&mut w, mask);
    let later = layer(&mut w, "Later", &[], 0);
    let mut available = usize::MAX;
    effects::decode(&image(&w.out, 0).c, parent, 26, &mut available, &Interrupt::NONE).unwrap();
    let effect_bytes = usize::MAX - available;
    let mut probe = image(&w.out, usize::MAX);
    probe.coverage_mask_header(mask, &Interrupt::NONE).unwrap();
    let mask_bytes = usize::MAX - probe.meta_budget;
    let work = 6 * std::mem::size_of::<usize>();
    let visit = 8 * std::mem::size_of::<usize>();
    for allowance in [0, work, work + visit, work + visit + effect_bytes + mask_bytes - 1, work + visit + effect_bytes + mask_bytes + work - 1] {
        let mut image = image(&w.out, allowance);
        let owner = coverage_owner(&mut image);
        image.inventory_coverage_effects(owner, &[parent], &Interrupt::NONE).unwrap();
        let (layers, _) = image.import_layers(&[later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
        assert_eq!(layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        let mut document = Document::new("Later raster", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        document.layers = layers.into();
        // The mask-header stage may use its narrower feature code.
        let features: &[&str] =
            if allowance == work + visit + effect_bytes + mask_bytes - 1 { &["xcf.coverage_effect_mask"] } else { &["xcf.coverage_effect"] };
        persist(document, image, features);
    }
    let mut malformed = image(&w.out, MAX_METADATA_BYTES);
    let owner = coverage_owner(&mut malformed);
    malformed.inventory_coverage_effects(owner, &[usize::MAX], &Interrupt::NONE).unwrap();
    let ImportConversion::Report(report) = malformed.outcomes.finish().unwrap() else { panic!("typed report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect" && entry.reason == ConversionReason::InvalidOptionalData));
    let mut cancelled = image(&w.out, 0);
    let owner = coverage_owner(&mut cancelled);
    assert!(matches!(cancelled.inventory_coverage_effects(owner, &[parent], &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
}

fn configured_effect(w: &mut W, raw: &str) -> usize {
    let offset = w.out.len();
    for value in ["Configured effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    let mut argument = W { out: Vec::new(), wide: false };
    argument.string("settings");
    argument.u32(6); // Bounded, inert configuration syntax.
    argument.string(raw);
    w.prop(45, &argument.out);
    w.end();
    w.ptr(0);
    offset
}

#[test]
fn valid_effect_config_quota_is_retention_loss_not_malformed_and_keeps_later_raster() {
    let raw = "(label \"owned text\")";
    let mut w = W { out: vec![0; 4], wide: true };
    let empty = configured_effect(&mut w, "");
    let valid = configured_effect(&mut w, raw);
    let malformed = configured_effect(&mut w, "(label \"unterminated)");
    let later = layer(&mut w, "Later", &[], 0);
    let cursor = Cursor { b: &w.out, pos: 0, wide: true };
    let mut remaining = usize::MAX;
    effects::decode(&cursor, empty, 26, &mut remaining, &Interrupt::NONE).unwrap();
    let envelope_bytes = usize::MAX - remaining;
    let nodes = 2 * std::mem::size_of::<effects::ConfigExpr<'_>>();
    let node_quota = nodes - 1;
    let string_quota = 3 * nodes + 2 * "owned text".len() - 1;
    let mut complete_budget = MAX_METADATA_BYTES;
    let complete = effects::decode(&cursor, valid, 26, &mut complete_budget, &Interrupt::NONE).unwrap();
    assert!(matches!(&complete.arguments[0].value, effects::ArgumentValue::Config(_)), "the source is valid when its metadata fits");
    for config_quota in [node_quota, string_quota] {
        let result = effects::decode(&cursor, valid, 26, &mut (envelope_bytes + config_quota), &Interrupt::NONE);
        assert!(matches!(result, Err(IoError::XcfMetadataLimit)), "quota must pass through the config parser and argument envelope: {result:?}");
        let allowance = 6 * std::mem::size_of::<usize>() + 8 * std::mem::size_of::<usize>() + envelope_bytes + config_quota;
        let mut image = image(&w.out, allowance);
        let owner = coverage_owner(&mut image);
        image.inventory_coverage_effects(owner, &[valid], &Interrupt::NONE).unwrap();
        let (layers, _) = image.import_layers(&[later], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
        assert_eq!(layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        let mut document = Document::new("Config quota preserves raster", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        document.layers = layers.into();
        persist(document, image, &["xcf.coverage_effect"]);
    }
    let mut malformed_budget = MAX_METADATA_BYTES;
    let decoded = effects::decode(&cursor, malformed, 26, &mut malformed_budget, &Interrupt::NONE).unwrap();
    assert!(matches!(&decoded.arguments[0].value, effects::ArgumentValue::Invalid { .. }), "real malformed syntax remains an optional invalid argument");
    let mut invalid = image(&w.out, MAX_METADATA_BYTES);
    let owner = coverage_owner(&mut invalid);
    invalid.inventory_coverage_effects(owner, &[malformed], &Interrupt::NONE).unwrap();
    let ImportConversion::Report(report) = invalid.outcomes.finish().unwrap() else { panic!("typed report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect" && entry.reason == ConversionReason::InvalidOptionalData));
    assert!(!report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit));
    let cancelled = Interrupt::cancel_only(&|| true);
    let mut cancelled_budget = MAX_METADATA_BYTES;
    assert!(matches!(effects::decode(&cursor, valid, 26, &mut cancelled_budget, &cancelled), Err(IoError::Cancelled)));
}
