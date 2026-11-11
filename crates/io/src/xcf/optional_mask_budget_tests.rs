use super::tests::{W, hierarchy};
use super::*;

fn image(bytes: &[u8], budget: usize) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget,
        meta_budget: MAX_METADATA_BYTES,
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

fn fixture(valid_mask: bool) -> (Vec<u8>, usize, usize, usize) {
    fixture_with_mask(valid_mask, 1, &[])
}

fn fixture_with_mask(valid_mask: bool, height: u32, properties: &[(u32, Vec<u8>)]) -> (Vec<u8>, usize, usize, usize) {
    let mut w = W { out: vec![0; 4], wide: true };
    let channel = w.out.len();
    w.u32(1);
    w.u32(height);
    w.string("Optional mask");
    for (kind, bytes) in properties {
        w.prop(*kind, bytes);
    }
    w.end();
    let channel_hierarchy = w.slot();
    w.ptr(0); // No child effects have been retained when channel decoding fails.
    let at = w.out.len();
    w.patch(channel_hierarchy, at);
    if valid_mask {
        hierarchy(&mut w, 1, height, 1, &vec![128; height as usize], Compression::None);
    } else {
        // Correct hierarchy and level dimensions, but no required tile pointer.
        // hierarchy() has already charged four decoded sample bytes at this failure.
        for value in [1, 1, 1] {
            w.u32(value);
        }
        let level = w.slot();
        w.ptr(0);
        let at = w.out.len();
        w.patch(level, at);
        w.u32(1);
        w.u32(1);
        w.ptr(0);
    }
    let layer = w.out.len();
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string("Later valid layer");
    w.end();
    let raster = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(raster, at);
    let effect = w.out.len();
    for value in ["Masked effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    w.end();
    w.ptr(channel);
    (w.out, channel, layer, effect)
}

#[test]
fn failed_optional_mask_decode_does_not_starve_a_later_valid_layer() {
    let (bytes, channel, layer, effect) = fixture(false);
    let rect = Rect::new(0, 0, 1, 1);
    let allowance = 16 + usize::try_from(photocraft_raster::tile_storage_bytes(rect, &PixelFormat::RGBA8)).unwrap();
    let mut image = image(&bytes, allowance);
    let mut state = CoverageTranslation::default();
    let mask_format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut original = Surface::new(mask_format);
    image.apply_coverage_stack(&[effect], &mut original, rect, 0, &mut state, &Interrupt::NONE).unwrap();
    assert!(matches!(state.applied.get(&effect), Some(Err(failure)) if failure.reason == ConversionReason::InvalidOptionalData));
    assert!(state.masks.is_empty());
    assert!(!state.active_masks.contains(&channel));
    assert_eq!(original.tile_count(), 0);
    let (layers, _) = image
        .import_layers(&[layer], PixelFormat::RGBA8, mask_format, &Interrupt::NONE)
        .expect("discarded optional mask must leave exact allowance for the valid layer");
    assert_eq!(layers[0].surface().unwrap().read_region(rect), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(image.budget, 0);
}

#[test]
fn successful_optional_mask_keeps_its_sample_and_tile_charges() {
    let (bytes, channel, _, _) = fixture(true);
    let rect = Rect::new(0, 0, 1, 1);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let allowance = 4 + usize::try_from(photocraft_raster::tile_storage_bytes(rect, &format)).unwrap();
    let mut image = image(&bytes, allowance);
    let mut state = CoverageTranslation::default();
    let mask = image.coverage_auxiliary(channel, 1, &mut state, &Interrupt::NONE).unwrap().unwrap();
    assert_eq!(mask.surface.read_region(rect), vec![128. / 255.]);
    assert!(state.masks.contains_key(&channel));
    assert!(!state.active_masks.contains(&channel));
    assert_eq!(image.budget, 0, "successful native mask storage must stay charged");
}

fn coverage_owner(image: &mut Image<'_>) -> usize {
    image
        .outcomes
        .push(ConversionEntry {
            feature_code: "xcf.channel".into(),
            source: conversion::source("channel", 0, 0, "Coverage owner", &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::RetainedUnapplied,
            reason: ConversionReason::MissingMapping,
            source_description: "Saved coverage owner".into(),
            native_description: "Preserved native coverage".into(),
            attachments: Vec::new(),
        })
        .unwrap()
}

fn later_layer_allowance() -> usize {
    16 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &PixelFormat::RGBA8)).unwrap()
}

fn admit_later_layer(image: &mut Image<'_>, layer: usize) {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let (layers, _) = image
        .import_layers(&[layer], PixelFormat::RGBA8, format, &Interrupt::NONE)
        .expect("conversion and reporting must preserve the exact allowance for the later valid layer");
    assert_eq!(layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(image.budget, 0);
}

#[test]
fn failed_optional_mask_translation_and_report_leave_budget_for_later_layer() {
    let (bytes, _, layer, effect) = fixture(false);
    let mut image = image(&bytes, later_layer_allowance());
    let owner = coverage_owner(&mut image);
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    image.translate_coverage_effects(owner, &[effect], &mut surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap();
    assert_eq!(surface.tile_count(), 0);
    admit_later_layer(&mut image, layer);
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("conversion report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect_mask"
        && entry.reason == ConversionReason::InvalidOptionalData
        && entry.disposition == ConversionDisposition::RetainedUnapplied));
}

#[test]
fn reporting_a_converted_mask_does_not_refund_retained_pixel_charges() {
    let (bytes, _, layer, effect) = fixture(true);
    let mask_format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mask_charge = 4 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &mask_format)).unwrap();
    let mut image = image(&bytes, later_layer_allowance() + mask_charge);
    let owner = coverage_owner(&mut image);
    let mut surface = Surface::new(mask_format);
    image.translate_coverage_effects(owner, &[effect], &mut surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap();
    assert_eq!(image.budget, later_layer_allowance(), "reporting must reuse the decoded mask without refunding its retained sample/tile charge");
    admit_later_layer(&mut image, layer);
}

#[test]
fn report_only_mask_decode_releases_pixels_but_keeps_configuration_charges() {
    let (bytes, _, layer, effect) = fixture(true);
    let mut image = image(&bytes, later_layer_allowance());
    let owner = coverage_owner(&mut image);
    let metadata_before = image.meta_budget;
    // A visible effect can have no decoded mask when conversion was not attempted.
    // This is the inventory fallback: its pixels are discarded, configuration is reported.
    image.report_converted_coverage(owner, &[effect], &CoverageTranslation::default(), true, &Interrupt::NONE).unwrap();
    assert!(image.meta_budget < metadata_before);
    admit_later_layer(&mut image, layer);
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("conversion report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect_mask" && entry.source.name == "Optional mask"));
}

#[test]
fn unsupported_optional_mask_placement_and_report_leave_budget_for_later_layer() {
    let placement = [1i32.to_be_bytes(), 0i32.to_be_bytes()].concat();
    let (bytes, _, layer, effect) = fixture_with_mask(true, 1, &[(15, placement)]);
    let mut image = image(&bytes, later_layer_allowance());
    let owner = coverage_owner(&mut image);
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    image.translate_coverage_effects(owner, &[effect], &mut surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap();
    assert_eq!(surface.tile_count(), 0, "unestablished placement must not change owner coverage");
    admit_later_layer(&mut image, layer);
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("conversion report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect"
        && entry.reason == ConversionReason::MissingMapping
        && entry.disposition == ConversionDisposition::RetainedUnapplied
        && entry.native_description.contains("unestablished placement")));
}

#[test]
fn optional_mask_tile_admission_and_report_leave_budget_for_later_layer() {
    let height = 65_535;
    let (bytes, _, layer, effect) = fixture_with_mask(true, height, &[]);
    assert!(bytes.len() < 128 * 1024, "small source fixture, not a large-allocation stress test");
    let allowance = later_layer_allowance();
    let sample_bytes = height as usize * std::mem::size_of::<f32>();
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mask_tiles = photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, height as i32), &format);
    assert_eq!(allowance, 262_160);
    assert_eq!(sample_bytes, 262_140);
    assert!(mask_tiles > (allowance - sample_bytes) as u64, "reject tiles before any large surface allocation");
    let mut image = image(&bytes, allowance);
    let owner = coverage_owner(&mut image);
    let mut surface = Surface::new(format);
    image.translate_coverage_effects(owner, &[effect], &mut surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap();
    assert_eq!(surface.tile_count(), 0);
    admit_later_layer(&mut image, layer);
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("conversion report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect"
        && entry.reason == ConversionReason::InvalidOptionalData
        && entry.disposition == ConversionDisposition::RetainedUnapplied
        && entry.native_description.contains("pixel limit")));
}

fn append_mask(w: &mut W, children: &[usize]) -> usize {
    let offset = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Optional mask");
    w.end();
    let pixels = w.slot();
    for child in children {
        w.ptr(*child);
    }
    w.ptr(0);
    let at = hierarchy(w, 1, 1, 1, &[128], Compression::None);
    w.patch(pixels, at);
    offset
}

fn append_effect(w: &mut W, mask: usize) -> usize {
    let offset = w.out.len();
    for value in ["Masked effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    w.end();
    w.ptr(mask);
    offset
}

#[test]
fn failed_optional_parent_mask_refunds_only_its_pixels_and_keeps_cached_child_charged() {
    let mut w = W { out: vec![0; 4], wide: true };
    let child = append_mask(&mut w, &[]);
    let first = append_effect(&mut w, child);
    let second = append_effect(&mut w, 0);
    // Coverage stages run in reverse source-pointer order. Cache the child first,
    // then exceed the traversal allowance on the second stage without big fixtures.
    let parent = append_mask(&mut w, &[second, first]);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mask_charge = 4 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &format)).unwrap();
    let mut image = image(&w.out, 2 * mask_charge);
    let mut state = CoverageTranslation { visited: MAX_LAYERS - 1, ..Default::default() };
    let result = image.coverage_auxiliary(parent, 1, &mut state, &Interrupt::NONE);
    assert!(matches!(result, Err(ref error) if error.to_string().contains("coverage traversal exceeds")));
    assert_eq!(state.visited, MAX_LAYERS + 1);
    assert_eq!(state.masks.len(), 1);
    assert!(state.masks.contains_key(&child));
    assert!(!state.masks.contains_key(&parent));
    assert!(state.active_masks.is_empty());
    assert_eq!(image.budget, mask_charge, "refund parent sample/tile storage, never the retained child's charge");
}

#[test]
fn optional_mask_metadata_admission_refunds_pixels_without_refunding_metadata_work() {
    let (bytes, channel, _, _) = fixture(true);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mask_charge = 4 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &format)).unwrap();
    let cache_charge = std::mem::size_of::<CoverageMaskRecord<'_>>() + 4 * std::mem::size_of::<usize>();
    let mut image = image(&bytes, mask_charge);
    image.meta_budget = "Optional mask".len() + cache_charge - 1;
    let mut state = CoverageTranslation::default();
    let result = image.coverage_auxiliary(channel, 1, &mut state, &Interrupt::NONE);
    assert!(matches!(result, Err(ref error) if error.to_string().contains("auxiliary coverage metadata")));
    assert!(state.masks.is_empty());
    assert!(state.active_masks.is_empty());
    assert_eq!(image.budget, mask_charge, "unretained pixels are released even when cache metadata cannot fit");
    assert_eq!(image.meta_budget, cache_charge - 1, "channel-name parsing stays charged");
    // This checks ownership accounting, not whole-import success after exhausting metadata.
}
