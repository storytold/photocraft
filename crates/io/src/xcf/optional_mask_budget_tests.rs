use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

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

fn coverage_owner(image: &mut Image<'_>) -> usize {
    image
        .outcomes
        .push(ConversionEntry {
            feature_code: "xcf.channel".into(),
            source: conversion::source("channel", 0, 0, "Coverage owner", &[7, 3], None),
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
fn auxiliary_inventory_never_decodes_pixels_and_leaves_later_layer_budget() {
    for valid_hierarchy in [false, true] {
        let (bytes, _, layer, effect) = fixture(valid_hierarchy);
        let mut image = image(&bytes, later_layer_allowance());
        let owner = coverage_owner(&mut image);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        surface.fill_rect(Rect::new(0, 0, 1, 1), &[0.75]);
        let original = surface.read_region(Rect::new(0, 0, 1, 1));
        image.inventory_coverage_effects(owner, &[effect], &Interrupt::NONE).unwrap();
        assert_eq!(image.budget, later_layer_allowance());
        assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), original);
        admit_later_layer(&mut image, layer);
        let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
        let mask = report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect_mask").unwrap();
        assert_eq!(mask.reason, ConversionReason::NativeLimitation, "hierarchy is not inspected");
        assert_eq!(mask.disposition, ConversionDisposition::SkippedNonessential);
    }
}

#[test]
fn auxiliary_header_inventory_works_with_zero_pixel_budget_and_persists_loss_and_config() {
    let metadata = b"inert mask configuration";
    let placement = [1i32.to_be_bytes(), 0i32.to_be_bytes()].concat();
    let (bytes, channel, _, effect) = fixture_with_mask(false, 65_535, &[(777, metadata.to_vec()), (15, placement.clone())]);
    let mut image = image(&bytes, 0);
    let owner = coverage_owner(&mut image);
    image.inventory_coverage_effects(owner, &[effect], &Interrupt::NONE).unwrap();
    assert_eq!(image.budget, 0);
    let conversion = image.outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("report") };
    let mask = report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect_mask").unwrap();
    assert_eq!(mask.source.name, "Optional mask");
    assert_eq!(mask.source.record_offset, Some(channel as u64));
    assert_eq!(mask.reason, ConversionReason::NativeLimitation);
    assert_eq!(mask.disposition, ConversionDisposition::SkippedNonessential);
    assert!(mask.native_description.contains("pixels are not retained"));
    assert!(report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == metadata));
    assert!(report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == placement));
    let mut document = Document::new("Header-only inventory", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.metadata.import_conversion = Some(Arc::new(conversion));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().metadata.import_conversion, document.metadata.import_conversion);
}

#[test]
fn nested_and_unreadable_auxiliary_mask_identities_match_the_immediate_effect() {
    let mut w = W { out: vec![0; 4], wide: true };
    let nested_mask = append_mask(&mut w, &[]);
    let nested_effect = append_effect(&mut w, nested_mask);
    let parent_mask = append_mask(&mut w, &[nested_effect]);
    let parent_effect = append_effect(&mut w, parent_mask);
    // Keep the pointer inside the file so effect framing succeeds, but make
    // the optional channel header invalid before any hierarchy can be followed.
    let unreadable_mask = w.out.len();
    w.u32(0);
    w.u32(0);
    let unreadable_effect = append_effect(&mut w, unreadable_mask);
    let mut image = image(&w.out, 0);
    let owner = coverage_owner(&mut image);
    image.inventory_coverage_effects(owner, &[parent_effect, unreadable_effect], &Interrupt::NONE).unwrap();
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
    for (effect_offset, mask_offset) in [(parent_effect, parent_mask), (nested_effect, nested_mask), (unreadable_effect, unreadable_mask)] {
        let effect = report
            .entries
            .iter()
            .find(|entry| entry.feature_code == "xcf.coverage_effect" && entry.source.record_offset == Some(effect_offset as u64))
            .unwrap();
        let mask = report
            .entries
            .iter()
            .find(|entry| entry.feature_code == "xcf.coverage_effect_mask" && entry.source.record_offset == Some(mask_offset as u64))
            .unwrap();
        assert_eq!(mask.source.ordinal, effect.source.ordinal);
        assert_eq!(mask.source.item_path, effect.source.item_path);
        assert_eq!(mask.source.item_path, [7, 3]);
        assert_eq!(mask.disposition, ConversionDisposition::SkippedNonessential);
    }
    let mut document = Document::new("Durable effect-mask identities", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(report)));
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().metadata.import_conversion, document.metadata.import_conversion);
}

#[test]
fn inert_auxiliary_inventory_bounds_cycles_metadata_and_cancellation() {
    let mut w = W { out: vec![0; 4], wide: true };
    let mask = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Cyclic mask");
    w.end();
    w.ptr(usize::MAX); // Deliberately unreadable pixel hierarchy: never followed.
    let child = w.slot();
    w.ptr(0);
    let effect = append_effect(&mut w, mask);
    w.patch(child, effect);
    let mut image = image(&w.out, 0);
    let owner = coverage_owner(&mut image);
    image.inventory_coverage_effects(owner, &[effect], &Interrupt::NONE).unwrap();
    assert_eq!(image.budget, 0);
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
    assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::InvalidOptionalData && entry.source_description.contains("Repeated")));
    let mut cancelled = self::image(&w.out, 0);
    let owner = coverage_owner(&mut cancelled);
    assert!(matches!(cancelled.inventory_coverage_effects(owner, &[effect], &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
    let mut bounded = self::image(&w.out, 0);
    let owner = coverage_owner(&mut bounded);
    bounded.meta_budget = 0;
    assert!(bounded.inventory_coverage_effects(owner, &[effect], &Interrupt::NONE).is_err());
    assert_eq!(bounded.budget, 0);
}

#[test]
fn checked_float_and_indexed_tile_access_preserves_samples_and_rejects_truncation() {
    let mut float = W { out: vec![0; 4], wide: true };
    let values = [1.5f32, -0.25, 0.5, 1.0];
    let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_be_bytes).collect();
    let offset = hierarchy(&mut float, 1, 1, 16, &bytes, Compression::None);
    let mut image = image(&float.out, MAX_PIXEL_BYTES);
    image.precision = Precision::from_header(12, 600).unwrap();
    assert_eq!(image.hierarchy(offset, 4, false, &Interrupt::NONE).unwrap().data, values);
    float.out.pop();
    let mut truncated = self::image(&float.out, MAX_PIXEL_BYTES);
    truncated.precision = Precision::from_header(12, 600).unwrap();
    assert!(truncated.hierarchy(offset, 4, false, &Interrupt::NONE).is_err());

    let mut indexed = W { out: vec![0; 4], wide: true };
    let offset = hierarchy(&mut indexed, 1, 1, 2, &[0, 128], Compression::None);
    let mut image = self::image(&indexed.out, MAX_PIXEL_BYTES);
    image.base_type = 2;
    image.colormap.push([17, 31, 57]);
    assert_eq!(image.hierarchy(offset, 2, false, &Interrupt::NONE).unwrap().data, [17. / 255., 31. / 255., 57. / 255., 128. / 255.]);
    indexed.out.pop();
    let mut truncated = self::image(&indexed.out, MAX_PIXEL_BYTES);
    truncated.base_type = 2;
    truncated.colormap.push([17, 31, 57]);
    assert!(truncated.hierarchy(offset, 2, false, &Interrupt::NONE).is_err());
}
