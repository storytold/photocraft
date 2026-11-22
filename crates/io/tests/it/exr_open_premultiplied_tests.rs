use craft_exr::decode::{Channel, Compression, Samples, write};
use photocraft_codecs::{self as codecs, ChannelLayout, EncodeOptions, ExrCompression, Format, Image};
use photocraft_doc::Document;
use photocraft_io::{ExportOptions, ImportOptions, export, import, import_file};
use photocraft_raster::Interrupt;

fn fixture(layout: ChannelLayout, left: &[f32], right: &[f32]) -> Vec<u8> {
    let samples: Vec<_> = (0..32 * 16).flat_map(|i| if i % 32 < 16 { left } else { right }).copied().collect();
    let image = Image::from_f32(32, 16, layout, &samples).unwrap();
    codecs::encode(&image, Format::OpenExr, &EncodeOptions { exr_compression: ExrCompression::Zip16, ..Default::default() }).unwrap()
}

fn assert_composite(doc: &Document, x: i32, black: [f32; 3], white: [f32; 3]) {
    let composite = photocraft_compose::flatten(doc);
    for (background, expected) in [([0.0; 3], black), ([1.0; 3], white)] {
        let actual = composite.over_background(background).get(x, 4);
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() <= 0.02, "pixel x={x} over {background:?}: expected {expected}, got {actual}");
        }
    }
}

#[test]
fn exr_scanline_alpha_opens_and_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("alpha.exr");
    let fixtures = [
        (fixture(ChannelLayout::Rgba, &[0.8, 0.4, 0.2, 1.0], &[0.8, 0.4, 0.2, 0.5]), [([0.8, 0.4, 0.2], [0.8, 0.4, 0.2]), ([0.8, 0.4, 0.2], [1.3, 0.9, 0.7])]),
        (fixture(ChannelLayout::Rgba, &[0.5, 0.5, 0.5, 0.0], &[0.0005, 0.0005, 0.0005, 0.001]), [([0.5; 3], [1.5; 3]), ([0.0005; 3], [0.9995; 3])]),
        (
            fixture(ChannelLayout::Rgba, &[8.0, 0.0, 0.0, 0.0], &[0.5, 0.0, 0.0, 1.0 / 1_048_576.0]),
            [([8.0, 0.0, 0.0], [9.0, 1.0, 1.0]), ([0.5, 0.0, 0.0], [1.5, 1.0, 1.0])],
        ),
        (fixture(ChannelLayout::Rgb, &[0.8, 0.4, 0.2], &[0.5, 0.25, 0.125]), [([0.8, 0.4, 0.2], [0.8, 0.4, 0.2]), ([0.5, 0.25, 0.125], [0.5, 0.25, 0.125])]),
    ];
    for (bytes, expected) in fixtures {
        std::fs::write(&path, &bytes).unwrap();
        let file = import_file("test.exr", &path, &ImportOptions::default(), &Interrupt::NONE).unwrap();
        let memory = import("test.exr", &bytes).unwrap();
        let saved = export(&file.document, "exr", &ExportOptions::default()).unwrap();
        let raw = codecs::decode(&saved.bytes).unwrap();
        let input = codecs::decode(&bytes).unwrap();
        for x in [4, 24] {
            for channel in 0..input.layout().channels() {
                assert!((raw.get(x, 4, channel) - input.get(x, 4, channel)).abs() <= 1e-6, "stored sample at x={x}, channel {channel}");
            }
        }
        let reopened = import("test.exr", &saved.bytes).unwrap();
        for doc in [&file.document, &memory.document, &reopened.document] {
            for (x, (black, white)) in [4, 24].into_iter().zip(expected) {
                assert_composite(doc, x, black, white);
            }
        }
    }
}

#[test]
fn exr_straight_float_overflow_preserves_associated_colour() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("overflow.exr");
    let alpha = f32::MIN_POSITIVE;
    let bytes = fixture(ChannelLayout::Rgba, &[8.0, 0.0, 0.0, alpha], &[0.0, -16.0, 0.0, alpha]);
    std::fs::write(&path, &bytes).unwrap();
    let file = import_file("overflow.exr", &path, &ImportOptions::default(), &Interrupt::NONE).unwrap().document;
    let memory = import("overflow.exr", &bytes).unwrap().document;
    let saved = export(&file, "exr", &ExportOptions::default()).unwrap();
    let raw = codecs::decode(&saved.bytes).unwrap();
    let reopened = import("overflow.exr", &saved.bytes).unwrap().document;

    for (x, channel, value, black, white) in [(4, 0, 8.0f32, [8.0, 0.0, 0.0], [9.0, 1.0, 1.0]), (24, 1, -16.0, [0.0, -16.0, 0.0], [1.0, -15.0, 1.0])] {
        let coverage = raw.get(x, 4, 3);
        assert!(coverage > alpha);
        assert!((raw.get(x, 4, channel) - value).abs() <= value.abs() * f32::EPSILON);
        let max = f64::from(f32::MAX);
        assert!(max * f64::from(coverage) >= f64::from(value.abs()));
        assert!(max * f64::from(coverage.next_down()) < f64::from(value.abs()));
        for doc in [&file, &memory, &reopened] {
            let pixel = doc.layers[0].surface().unwrap().pixel(x as i32, 4);
            assert!(pixel[channel].is_finite());
            assert_eq!(pixel[3], coverage);
            assert_composite(doc, x as i32, black, white);
        }
    }
}

#[test]
fn exr_bright_emission_keeps_zero_coverage() {
    let red = f32::MAX / 2.0;
    let bytes = fixture(ChannelLayout::Rgba, &[red, 0.0, 0.0, 0.0], &[0.0, 0.0, 0.0, 0.0]);
    let mut doc = import("emission.exr", &bytes).unwrap().document;

    let buf = photocraft_compose::flatten(&doc);
    assert_eq!(buf.over_background([0.0; 3]).get(4, 4), [red, 0.0, 0.0, 1.0]);
    assert_eq!(buf.over_background([1.0; 3]).get(4, 4), [red, 1.0, 1.0, 1.0]);
    // Export a composite as well as the native single-layer path.
    doc.layers.insert(0, photocraft_doc::Layer::raster("empty", doc.pixel_format()));
    let saved = export(&doc, "exr", &ExportOptions::default()).unwrap();
    let raw = codecs::decode(&saved.bytes).unwrap();
    assert_eq!(raw.get(4, 4, 0), red);
    assert_eq!(raw.get(4, 4, 3).to_bits(), 0);
    assert_eq!(raw.get(24, 4, 0), 0.0);
    let reopened = import("emission.exr", &saved.bytes).unwrap().document;
    assert_eq!(photocraft_compose::flatten(&reopened).over_background([1.0; 3]).get(4, 4), [red, 1.0, 1.0, 1.0]);

    let bounds = doc.bounds();
    doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[1.0, 1.0, 1.0, 1.0]);
    assert_eq!(photocraft_compose::flatten(&doc).get(4, 4), [red, 1.0, 1.0, 1.0]);
}

#[test]
fn exr_emission_survives_half_float_texture_samples() {
    let bytes = fixture(ChannelLayout::Rgba, &[0.5, 0.25, 0.125, 0.0], &[0.5, 0.25, 0.125, 1.0 / 16_384.0]);
    let doc = import("emission.exr", &bytes).unwrap().document;
    let samples = doc.layers[0].surface().unwrap().read_region(doc.bounds());
    let half = Image::from_f32(32, 16, ChannelLayout::Rgba, &samples)
        .unwrap()
        .convert(ChannelLayout::Rgba, codecs::SampleType::F16)
        .convert(ChannelLayout::Rgba, codecs::SampleType::F32);
    let alpha = half.get(4, 4, 3);
    assert!(photocraft_color::is_emissive_alpha(alpha));
    for (channel, expected) in [0.5, 0.25, 0.125].into_iter().enumerate() {
        assert_eq!(half.get(4, 4, channel), expected);
    }

    let saved = export(&doc, "exr", &ExportOptions::default()).unwrap();
    let raw = codecs::decode(&saved.bytes).unwrap();
    assert_eq!(raw.get(4, 4, 3), 0.0);
    assert_eq!(raw.get(24, 4, 3), 1.0f32 / 16_384.0);
}

#[test]
fn exr_selected_channel_group_keeps_associated_colour() {
    let channels: Vec<_> = ["diffuse.R", "diffuse.G", "diffuse.B", "diffuse.A"]
        .into_iter()
        .zip([0.3, 0.2, 0.1, 0.5])
        .map(|(name, value)| Channel { name: name.into(), samples: Samples::F32(vec![value; 32 * 16]) })
        .collect();
    let bytes = write(32, 16, &channels, Compression::Zip16).unwrap();
    let selection = photocraft_io::exr_parts::Selection { parts: vec![], groups: vec!["diffuse".into()], view: "both".into(), precomp: false };
    let selected = photocraft_io::exr_parts::import("diffuse.exr", &bytes, &selection).unwrap();
    assert_composite(&selected.document, 4, [0.3, 0.2, 0.1], [0.8, 0.7, 0.6]);
}

#[test]
fn exr_uint_channels_keep_their_raw_values() {
    let channels =
        [Channel { name: "Y".into(), samples: Samples::U32(vec![800; 32 * 16]) }, Channel { name: "A".into(), samples: Samples::U32(vec![0; 32 * 16]) }];
    let bytes = write(32, 16, &channels, Compression::Zip16).unwrap();
    let path = std::env::temp_dir().join(format!("photocraft-exr-uint-{}.exr", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let file = import_file("uint.exr", &path, &ImportOptions::default(), &Interrupt::NONE);
    std::fs::remove_file(&path).unwrap();
    let file = file.unwrap().document;
    let memory = import("uint.exr", &bytes).unwrap().document;
    let file_values = file.layers[0].surface().unwrap().read_region(file.bounds());
    let memory_values = memory.layers[0].surface().unwrap().read_region(memory.bounds());
    assert!(file_values.as_chunks::<4>().0.iter().all(|pixel| *pixel == [800.0, 800.0, 800.0, 0.0]));
    assert!(memory_values.as_chunks::<2>().0.iter().all(|pixel| *pixel == [800.0, 0.0]));
}

#[test]
fn exr_export_preserves_ordinary_low_coverage() {
    let low = 1.0f32 / 16_384.0;
    for layout in [ChannelLayout::Rgba, ChannelLayout::GrayA] {
        for alpha in [low, low.next_down(), low.next_up(), f32::MIN_POSITIVE, 0.0, 0.5, 1.0] {
            let samples = if layout == ChannelLayout::Rgba { vec![0.5, 0.25, 0.125, alpha] } else { vec![0.5, alpha] };
            let image = Image::from_f32(1, 1, layout, &samples).unwrap();
            let mut doc = photocraft_io::image_to_document("ordinary", &image).unwrap().document;
            doc.icc_profile = Some(if layout == ChannelLayout::Rgba {
                craft_cms::Builtin::LinearSrgb.profile().to_bytes()
            } else {
                craft_cms::builtin::linear_gray().to_bytes()
            });
            let saved = export(&doc, "exr", &ExportOptions::default()).unwrap();
            let raw = codecs::decode(&saved.bytes).unwrap();
            let alpha_channel = layout.channels() - 1;
            let stored_alpha = if raw.layout().has_alpha() { raw.get(0, 0, raw.layout().channels() - 1) } else { 1.0 };
            assert_eq!(stored_alpha, alpha, "{layout:?} at {alpha}");
            for (channel, colour) in samples[..alpha_channel].iter().enumerate() {
                assert_eq!(raw.get(0, 0, channel), colour * alpha, "{layout:?} at {alpha}");
            }
            let reopened = import("ordinary.exr", &saved.bytes).unwrap().document;
            assert_eq!(reopened.layers[0].surface().unwrap().pixel(0, 0)[alpha_channel], alpha, "{layout:?} at {alpha}");
        }
    }
}

#[test]
fn exr_emission_survives_export_to_an_opaque_hdr_format() {
    let bytes = fixture(ChannelLayout::Rgba, &[0.5, 0.0, 0.0, 0.0], &[0.0, -0.5, 0.0, 0.0]);
    let mut doc = import("emission.exr", &bytes).unwrap().document;
    for composite in [false, true] {
        if composite {
            doc.layers.insert(0, photocraft_doc::Layer::raster("empty", doc.pixel_format()));
        }
        let saved = export(&doc, "hdr", &ExportOptions::default()).unwrap();
        let raw = codecs::decode(&saved.bytes).unwrap();
        for (x, expected) in [(4, [1.5, 1.0, 1.0]), (24, [1.0, 0.5, 1.0])] {
            for (channel, value) in expected.into_iter().enumerate() {
                assert!(
                    (raw.get(x, 4, channel) - value).abs() <= 0.02,
                    "composite={composite}, x={x}, channel={channel}: expected {value}, got {}",
                    raw.get(x, 4, channel)
                );
            }
        }
    }
}
