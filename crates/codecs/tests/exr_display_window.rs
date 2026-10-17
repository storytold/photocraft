//! Flat EXR data windows are positioned and clipped on their declared display canvas.

use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Image, ImageAttributes, IntegerBounds, Layer, LayerAttributes, LineOrder,
    WritableImage,
};
use photocraft_codecs::{ChannelLayout, DecodeOptions, Format, Limits, decode, decode_as_with, decode_exr_channels};

fn exr(display: IntegerBounds, data_position: (i32, i32), size: (usize, usize), pixels: Vec<f32>) -> Vec<u8> {
    let expected = size.0 * size.1;
    assert_eq!(pixels.len(), expected);
    let layer = Layer::new(
        size,
        LayerAttributes::named("beauty").with_position(data_position.into()),
        Encoding { compression: Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
        AnyChannels::sort(
            vec![
                AnyChannel::new("R", FlatSamples::F32(pixels.clone())),
                AnyChannel::new("G", FlatSamples::F32(pixels.clone())),
                AnyChannel::new("B", FlatSamples::F32(pixels)),
            ]
            .into_iter()
            .collect(),
        ),
    );
    let image = Image::from_layers(ImageAttributes::new(display), vec![layer]);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut bytes).expect("synthetic EXR writes");
    bytes.into_inner()
}

#[test]
fn data_window_is_placed_on_negative_origin_display_stage() {
    // The two-by-two data window starts at (-1, 1) inside a four-by-four display
    // window starting at (-2, 0): its canvas offset is (1, 1).
    let bytes = exr(IntegerBounds::new((-2, 0), (4, 4)), (-1, 1), (2, 2), vec![1.0, 2.0, 3.0, 4.0]);
    let image = decode(&bytes).expect("decode positioned EXR");
    assert_eq!(image.dimensions(), (4, 4));
    assert_eq!(image.layout(), ChannelLayout::Rgb);
    assert_eq!(image.get(0, 0, 0), 0.0);
    assert_eq!(image.get(1, 1, 0), 1.0);
    assert_eq!(image.get(2, 1, 0), 2.0);
    assert_eq!(image.get(1, 2, 0), 3.0);
    assert_eq!(image.get(2, 2, 0), 4.0);
    assert!(image.warnings.iter().any(|w| matches!(w, photocraft_codecs::DecodeWarning::DisplayWindowStage { stage: (4, 4), origin: (1, 1) })));
    let channels = decode_exr_channels(&bytes, 0, &["R"], &DecodeOptions::default()).expect("decode named channel on stage");
    assert_eq!(channels.dimensions(), (4, 4));
    assert_eq!(channels.get(1, 1, 0), 1.0);
    assert_eq!(channels.get(0, 0, 0), 0.0);
}

#[test]
fn data_window_outside_display_is_clipped_to_the_intersection() {
    // Source data begins at (-2, -1), while the display stage is (0, 0)..(2, 2).
    // Only source columns 2..4 and rows 1..3 should be visible.
    let values = (0..12).map(|v| v as f32 + 1.0).collect();
    let bytes = exr(IntegerBounds::new((0, 0), (2, 2)), (-2, -1), (4, 3), values);
    let image = decode(&bytes).expect("decode clipped EXR");
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!([image.get(0, 0, 0), image.get(1, 0, 0), image.get(0, 1, 0), image.get(1, 1, 0)], [7.0, 8.0, 11.0, 12.0]);
}

#[test]
fn display_window_limits_are_checked_before_decode_allocation() {
    let bytes = exr(IntegerBounds::new((0, 0), (100, 100)), (0, 0), (1, 1), vec![1.0]);
    let limits = Limits { max_width: 16, max_height: 16, max_pixels: 256, max_alloc: 4096 };
    let error = decode_as_with(Format::OpenExr, &bytes, &DecodeOptions { limits, ..Default::default() }).expect_err("oversized display canvas rejected");
    assert!(matches!(error, photocraft_codecs::CodecError::LimitExceeded(_)), "{error:?}");
}
