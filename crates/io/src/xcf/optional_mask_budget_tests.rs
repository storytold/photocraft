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
    let mut w = W { out: vec![0; 4], wide: true };
    let channel = w.out.len();
    w.u32(1);
    w.u32(1);
    w.string("Optional mask");
    w.end();
    let channel_hierarchy = w.slot();
    w.ptr(0); // No child effects have been retained when channel decoding fails.
    let at = w.out.len();
    w.patch(channel_hierarchy, at);
    if valid_mask {
        hierarchy(&mut w, 1, 1, 1, &[128], Compression::None);
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
