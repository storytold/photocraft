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
    let path = std::env::temp_dir().join(format!("photocraft-exr-alpha-{}.exr", std::process::id()));
    let fixtures = [
        (fixture(ChannelLayout::Rgba, &[0.8, 0.4, 0.2, 1.0], &[0.8, 0.4, 0.2, 0.5]), [([0.8, 0.4, 0.2], [0.8, 0.4, 0.2]), ([0.8, 0.4, 0.2], [1.3, 0.9, 0.7])]),
        (fixture(ChannelLayout::Rgba, &[0.5, 0.5, 0.5, 0.0], &[0.0005, 0.0005, 0.0005, 0.001]), [([0.5; 3], [1.5; 3]), ([0.0005; 3], [0.9995; 3])]),
        (fixture(ChannelLayout::Rgb, &[0.8, 0.4, 0.2], &[0.5, 0.25, 0.125]), [([0.8, 0.4, 0.2], [0.8, 0.4, 0.2]), ([0.5, 0.25, 0.125], [0.5, 0.25, 0.125])]),
    ];
    for (bytes, expected) in fixtures {
        std::fs::write(&path, &bytes).unwrap();
        let file = import_file("test.exr", &path, &ImportOptions::default(), &Interrupt::NONE).unwrap();
        let memory = import("test.exr", &bytes).unwrap();
        std::fs::remove_file(&path).unwrap();
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
    let file = import_file("uint.exr", &path, &ImportOptions::default(), &Interrupt::NONE).unwrap();
    std::fs::remove_file(path).unwrap();
    let memory = import("uint.exr", &bytes).unwrap();
    for doc in [&file.document, &memory.document] {
        let values = doc.layers[0].surface().unwrap().read_region(doc.bounds());
        assert!(values.chunks_exact(2).all(|pixel| pixel == [800.0, 0.0]));
    }
}
