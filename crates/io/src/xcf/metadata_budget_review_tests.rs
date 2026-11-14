//! Optional metadata admission must leave independently framed raster/coverage samples usable.
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
    document.layers = layers;
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
    let (channels, selection) = image.import_channels(&[alpha], PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert!(selection.is_none());
    assert_eq!(layers.len(), 2);
    let masked = layers.iter().find(|layer| layer.mask.is_some()).unwrap();
    assert_eq!(masked.mask.as_ref().unwrap().surface.pixel(0, 0), vec![128. / 255.]);
    assert_eq!(channels[0].surface.pixel(0, 0), vec![128. / 255.]);
    for layer in &layers {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    let mut document = Document::new("Usable coverage", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers;
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
        document.layers = layers;
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
        document.layers = layers;
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
