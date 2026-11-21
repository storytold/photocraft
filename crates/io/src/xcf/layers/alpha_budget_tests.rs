use super::super::tests::{W, hierarchy};
use super::super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn pixels(channels: usize, data: Vec<f32>) -> Pixels {
    Pixels { width: 1, height: 1, channels, data, underflow: true, coverage_clamped: true, palette_fallback: true }
}

#[test]
fn hierarchy_emits_opaque_alpha_in_its_only_sample_allocation() {
    for (base_type, source_channels, colormap, data, expected) in [
        (0, 3, vec![], vec![17, 31, 57, 83, 127, 191], vec![17. / 255., 31. / 255., 57. / 255., 1., 83. / 255., 127. / 255., 191. / 255., 1.]),
        (1, 1, vec![], vec![17, 83], vec![17. / 255., 1., 83. / 255., 1.]),
        (2, 1, vec![[17, 31, 57]], vec![0, 9], vec![17. / 255., 31. / 255., 57. / 255., 1., 9. / 255., 9. / 255., 9. / 255., 1.]),
    ] {
        let mut writer = W { out: vec![0; 4], wide: true };
        let pointer = hierarchy(&mut writer, 2, 1, source_channels, &data, Compression::None);
        let bytes = writer.out;
        let required = expected.len() * std::mem::size_of::<f32>();
        let mut exact = image(&bytes, required, base_type, Precision::GAMMA8);
        exact.colormap = colormap.clone();
        let decoded = exact.hierarchy_with_alpha(pointer, source_channels, false, true, &Interrupt::NONE).unwrap();
        assert_eq!(decoded.data, expected);
        assert_eq!(decoded.channels, if base_type == 1 { 2 } else { 4 });
        assert_eq!(exact.budget, 0, "no transient second color buffer is admitted");
        assert_eq!(decoded.palette_fallback, base_type == 2);
        let mut short = image(&bytes, required - 1, base_type, Precision::GAMMA8);
        short.colormap = colormap;
        assert!(short.hierarchy_with_alpha(pointer, source_channels, false, true, &Interrupt::NONE).is_err());
        assert_eq!(short.budget, required - 1);
    }
}

#[test]
fn hierarchy_alpha_synthesis_checks_layout_and_cancellation_without_clamping_hdr() {
    let values = [-0.25f32, 3.5, f32::MIN_POSITIVE, 0.75, -2., 7.];
    let data: Vec<_> = values.into_iter().flat_map(f32::to_be_bytes).collect();
    let mut writer = W { out: vec![0; 4], wide: true };
    let pointer = hierarchy(&mut writer, 2, 1, 12, &data, Compression::None);
    let bytes = writer.out;
    let mut hdr = image(&bytes, 32, 0, Precision::from_header(11, 650).unwrap());
    assert_eq!(hdr.hierarchy_with_alpha(pointer, 3, false, true, &Interrupt::NONE).unwrap().data, [-0.25, 3.5, f32::MIN_POSITIVE, 1., 0.75, -2., 7., 1.]);
    assert_eq!(hdr.budget, 0);
    for (base_type, channels, coverage) in [(0, 0, false), (0, 4, false), (1, 2, false), (2, 2, false), (0, 3, true)] {
        let mut matching = W { out: vec![0; 4], wide: true };
        let pointer = hierarchy(&mut matching, 1, 1, channels, &vec![57; channels], Compression::None);
        let mut invalid = image(&matching.out, 64, base_type, Precision::GAMMA8);
        assert!(matches!(
            invalid.hierarchy_with_alpha(pointer, channels, coverage, true, &Interrupt::NONE),
            Err(IoError::Xcf(message)) if message == "opaque alpha synthesis does not match the drawable color layout"
        ));
        assert_eq!(invalid.budget, 64);
    }
    for (width, height, bpp) in [(0, 1, 12), (u32::MAX, u32::MAX, 12), (2, 1, 11)] {
        let mut malformed = W { out: vec![0; 4], wide: true };
        let at = malformed.out.len();
        for value in [width, height, bpp] {
            malformed.u32(value);
        }
        malformed.ptr(0);
        let mut decoder = image(&malformed.out, 64, 0, Precision::from_header(11, 650).unwrap());
        assert!(decoder.hierarchy_with_alpha(at, 3, false, true, &Interrupt::NONE).is_err());
        assert_eq!(decoder.budget, 64, "invalid source dimensions or byte layout allocate no samples");
    }
    let checks = AtomicUsize::new(0);
    let cancel = || {
        checks.fetch_add(1, Ordering::Relaxed);
        true
    };
    let mut cancelled = image(&bytes, 32, 0, Precision::from_header(11, 650).unwrap());
    assert!(matches!(cancelled.hierarchy_with_alpha(pointer, 3, false, true, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
    assert_eq!(cancelled.budget, 32, "cancellation precedes sample admission");
    assert_eq!(checks.load(Ordering::Relaxed), 1);
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
        name_work_left: crate::xcf::MAX_METADATA_BYTES,
        vector_style_work_left: crate::xcf::MAX_METADATA_BYTES,
        link_work_left: crate::xcf::MAX_METADATA_BYTES,
        effect_decode_work: Default::default(),
        text_decode_work: Default::default(),
        parasite_work: Default::default(),
        report_parasite_work: Default::default(),
        point_text_work_left: crate::xcf::text::POINT_LAYOUT_WORK_UNITS,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(11).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        source_resolution: (72., 72.),
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

/// Test-only copy of the removed two-buffer expansion for a measured comparison.
fn old_opaque_expansion(pixels: Pixels, budget: &mut usize) -> Result<Pixels> {
    let channels = pixels.channels + 1;
    let samples = pixels.data.len() / pixels.channels;
    let count = samples.checked_mul(channels).ok_or_else(|| invalid("test baseline size overflow"))?;
    let bytes = count.checked_mul(std::mem::size_of::<f32>()).ok_or_else(|| invalid("test baseline byte overflow"))?;
    let remaining = budget.checked_sub(bytes).ok_or_else(|| invalid("test baseline needs both full buffers"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| invalid("test baseline allocation unavailable"))?;
    for pixel in pixels.data.chunks_exact(pixels.channels) {
        data.extend_from_slice(pixel);
        data.push(1.);
    }
    *budget = remaining + pixels.data.len() * std::mem::size_of::<f32>();
    Ok(Pixels { channels, data, ..pixels })
}

#[test]
#[ignore = "24 MP F32 opaque RGB sample allocation comparison; run explicitly in release"]
fn opaque_f32_24mp_alpha_emission_timing() {
    let (width, height) = (6000u32, 4000u32);
    let pixels = width as usize * height as usize;
    let expected = [-0.25f32, 1.5, 0.375, 1.];
    let source: Vec<u8> = expected[..3].iter().copied().cycle().take(pixels * 3).flat_map(f32::to_be_bytes).collect();
    let mut writer = W { out: vec![0; 4], wide: true };
    let pointer = hierarchy(&mut writer, width, height, 12, &source, Compression::None);
    drop(source);
    let bytes = writer.out;
    let native_bytes = pixels * 4 * std::mem::size_of::<f32>();
    let old_peak_bytes = pixels * 7 * std::mem::size_of::<f32>();
    let precision = Precision::from_header(11, 650).unwrap();
    let validate = |decoded: &Pixels| {
        assert_eq!((decoded.width, decoded.height, decoded.channels), (width, height, 4));
        assert_eq!(decoded.data.len(), pixels * 4);
        assert!(decoded.data.as_chunks::<4>().0.iter().all(|pixel| *pixel == expected));
    };
    {
        let mut tight = image(&bytes, native_bytes, 0, precision);
        let decoded = tight.hierarchy_with_alpha(pointer, 3, false, true, &Interrupt::NONE).unwrap();
        assert_eq!(tight.budget, 0);
        validate(&decoded);
    }
    {
        let mut tight = image(&bytes, native_bytes, 0, precision);
        let rgb = tight.hierarchy(pointer, 3, false, &Interrupt::NONE).unwrap();
        assert!(old_opaque_expansion(rgb, &mut tight.budget).is_err(), "the old path requires two simultaneously live float buffers");
    }
    let mut new_times = Vec::new();
    let mut old_times = Vec::new();
    for round in 0..3 {
        // Alternate order to avoid assigning every cold run to the same path.
        for new in if round % 2 == 0 { [true, false] } else { [false, true] } {
            let mut decoder = image(&bytes, old_peak_bytes, 0, precision);
            let start = std::time::Instant::now();
            let decoded = if new {
                decoder.hierarchy_with_alpha(pointer, 3, false, true, &Interrupt::NONE).unwrap()
            } else {
                let rgb = decoder.hierarchy(pointer, 3, false, &Interrupt::NONE).unwrap();
                old_opaque_expansion(rgb, &mut decoder.budget).unwrap()
            };
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            validate(&decoded);
            assert_eq!(decoder.budget, old_peak_bytes - native_bytes);
            if new {
                new_times.push(elapsed);
            } else {
                old_times.push(elapsed);
            }
        }
    }
    eprintln!(
        "XCF opaque RGB32F hierarchy6000x4000 (24MP),3pairedruns; new_ms={new_times:?}; old_ms={old_times:?}; new_sample_peak_bytes={native_bytes}; old_sample_peak_bytes={old_peak_bytes}; exact_HDR_alpha=true; tight_native_budget_new=pass_old=refused; encoded_source_bytes={} (excludedfromsamplebudget); native_Surface_tiles_excluded=true",
        bytes.len()
    );
}
