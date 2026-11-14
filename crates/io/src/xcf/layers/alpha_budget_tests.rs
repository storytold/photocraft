use super::super::tests::{W, hierarchy};
use super::super::*;
use super::opaque_alpha;
use std::sync::atomic::{AtomicUsize, Ordering};

fn pixels(channels: usize, data: Vec<f32>) -> Pixels {
    Pixels { width: 1, height: 1, channels, data, underflow: true, coverage_clamped: true, palette_fallback: true }
}

#[test]
fn opaque_alpha_reserves_the_whole_destination_before_allocating_and_refunds_only_the_source() {
    for channels in [1, 3] {
        let output = (channels + 1) * 4;
        let mut shortage = output - 1;
        assert!(opaque_alpha(pixels(channels, vec![0.5; channels]), &mut shortage, &Interrupt::NONE).is_err());
        assert_eq!(shortage, output - 1);
        let mut exact = output;
        let result = opaque_alpha(pixels(channels, vec![0.5; channels]), &mut exact, &Interrupt::NONE).unwrap();
        assert_eq!(exact, channels * 4, "only the original, already-charged sample buffer is released");
        assert_eq!(result.channels, channels + 1);
        assert_eq!(result.data.last(), Some(&1.));
        assert!(result.underflow && result.coverage_clamped && result.palette_fallback);
    }
}

#[test]
fn opaque_alpha_checks_shapes_and_cancels_transactionally_without_clamping_hdr() {
    for (channels, width, height, data) in
        [(0, 1, 1, vec![]), (2, 1, 1, vec![0.; 2]), (3, 1, 1, vec![0.; 2]), (3, 0, 1, vec![]), (3, u32::MAX, u32::MAX, vec![])]
    {
        let mut budget = usize::MAX;
        let source = Pixels { width, height, ..pixels(channels, data) };
        assert!(opaque_alpha(source, &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget, usize::MAX);
    }
    let mut budget = 16;
    let result = opaque_alpha(pixels(3, vec![-0.25, 3.5, f32::MIN_POSITIVE]), &mut budget, &Interrupt::NONE).unwrap();
    assert_eq!(result.data, [-0.25, 3.5, f32::MIN_POSITIVE, 1.]);
    for threshold in [0, 2, 4] {
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= threshold;
        let source = Pixels { width: 4097, height: 1, ..pixels(3, vec![0.5; 4097 * 3]) };
        let mut budget = 100_000;
        assert!(matches!(opaque_alpha(source, &mut budget, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(budget, 100_000);
        assert_eq!(checks.load(Ordering::Relaxed), threshold + 1);
    }
}

fn image(bytes: &[u8], budget: usize, base_type: u32, precision: Precision) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 11,
        precision,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type,
        colormap: Vec::new(),
        budget,
        meta_budget: MAX_METADATA_BYTES,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(11).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: 72.,
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}

fn layer_fixture(kind: u32, bytes_per_sample: usize, data: &[u8]) -> (Vec<u8>, usize) {
    let mut writer = W { out: vec![0; 4], wide: true };
    let layer = writer.out.len();
    for value in [1, 1, kind] {
        writer.u32(value);
    }
    writer.string("Budget-controlled layer");
    writer.end();
    let slot = writer.slot();
    writer.ptr(0);
    let channels = match kind {
        0 => 3,
        1 => 4,
        2 => 1,
        _ => 2,
    };
    let pointer = hierarchy(&mut writer, 1, 1, channels * bytes_per_sample, data, Compression::None);
    writer.patch(slot, pointer);
    (writer.out, layer)
}

#[test]
fn opaque_and_alpha_layers_charge_expanded_samples_and_native_tiles_at_every_depth_and_model() {
    for (code, depth, encoded, expected, bytes_per_sample) in [
        (150, SampleType::U8, vec![17, 31, 57], vec![17. / 255., 31. / 255., 57. / 255.], 1),
        (
            250,
            SampleType::U16,
            [0x1234u16, 0x4567, 0x89ab].into_iter().flat_map(u16::to_be_bytes).collect(),
            vec![0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535.],
            2,
        ),
        (650, SampleType::F32, [-0.25f32, 1.5, 0.375].into_iter().flat_map(f32::to_be_bytes).collect(), vec![-0.25, 1.5, 0.375], 4),
    ] {
        for (base_type, mode, color_channels) in [(0, ColorMode::Rgb, 3), (1, ColorMode::Grayscale, 1)] {
            for alpha in [false, true] {
                let mut data = encoded[..color_channels * bytes_per_sample].to_vec();
                if alpha {
                    data.extend_from_slice(&match depth {
                        SampleType::U8 => vec![255],
                        SampleType::U16 => u16::MAX.to_be_bytes().to_vec(),
                        _ => 1f32.to_be_bytes().to_vec(),
                    });
                }
                let kind = if base_type == 0 { u32::from(alpha) } else { 2 + u32::from(alpha) };
                let (bytes, pointer) = layer_fixture(kind, bytes_per_sample, &data);
                let format = PixelFormat::new(mode, depth, true);
                let mask = PixelFormat::new(ColorMode::Grayscale, depth, false);
                let expanded_samples = (color_channels + 1) * 4;
                let tiles = usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &format)).unwrap();
                let precision = Precision::from_header(11, code).unwrap();
                let mut exact = image(&bytes, expanded_samples + tiles, base_type, precision);
                let (node, _) = exact.layer(0, pointer, format, mask, &Interrupt::NONE).unwrap().unwrap();
                assert_eq!(exact.budget, 0, "{mode:?}/{depth:?}, alpha={alpha}");
                let actual = node.layer.surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
                assert_eq!(&actual[..color_channels], &expected[..color_channels]);
                assert_eq!(actual[color_channels], 1.);
                let mut shortage = image(&bytes, expanded_samples + tiles - 1, base_type, precision);
                assert!(shortage.layer(0, pointer, format, mask, &Interrupt::NONE).is_err(), "{mode:?}/{depth:?}, alpha={alpha}");
                assert_eq!(shortage.budget, tiles - 1, "no native tile charge after admission fails");
            }
        }
    }
}

#[test]
fn gray_rgb_expansion_also_reserves_the_full_live_copy_before_releasing_source_samples() {
    for channels in [1, 2] {
        let output = (channels + 2) * 4;
        let mut shortage = output - 1;
        assert!(resources::gray_to_rgb(pixels(channels, vec![0.5; channels]), &mut shortage, &Interrupt::NONE).is_err());
        assert_eq!(shortage, output - 1);
        let mut exact = output;
        let result = resources::gray_to_rgb(pixels(channels, vec![0.5; channels]), &mut exact, &Interrupt::NONE).unwrap();
        assert_eq!(result.channels, channels + 2);
        assert_eq!(exact, channels * 4);
    }
}
