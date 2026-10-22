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
fn a_part_larger_than_the_display_window_keeps_its_data_window() {
    // Source data begins at (-2, -1), 4×3, while the display stage is (0, 0)..(2, 2): the
    // data window exceeds the stage, so it stays its own canvas (multi-part files size the
    // shared display window from their first layer, #5866) — no staging, no clipping.
    let values = (0..12).map(|v| v as f32 + 1.0).collect();
    let bytes = exr(IntegerBounds::new((0, 0), (2, 2)), (-2, -1), (4, 3), values);
    let image = decode(&bytes).expect("decode oversized-data EXR");
    assert_eq!(image.dimensions(), (4, 3));
    assert_eq!(image.get(0, 0, 0), 1.0);
    assert_eq!(image.get(3, 2, 0), 12.0);
    assert!(image.warnings.is_empty(), "a part the stage cannot hold does not claim one");
}

/// A data window that fits the stage is placed on it: cropped renders open on their full
/// canvas, the overlap keeping its texels and the stage's rest the default pixel.
#[test]
fn overlapping_data_within_the_display_window_is_staged() {
    // Source data begins at (-2, -1), 2×2, inside a 4×4 display stage at (0, 0).
    let values = vec![9.0f32, 8.0, 7.0, 6.0];
    let bytes = exr(IntegerBounds::new((0, 0), (4, 4)), (1, 1), (2, 2), values);
    let image = decode(&bytes).expect("decode staged EXR");
    assert_eq!(image.dimensions(), (4, 4));
    assert_eq!(image.get(1, 1, 0), 9.0);
    assert_eq!(image.get(2, 1, 0), 8.0);
    assert_eq!(image.get(1, 2, 0), 7.0);
    assert_eq!(image.get(2, 2, 0), 6.0);
    assert_eq!(image.get(0, 0, 0), 0.0, "outside the data window the stage is default");
    assert_eq!(image.get(3, 3, 0), 0.0);
    assert!(image.warnings.iter().any(|w| matches!(w, photocraft_codecs::DecodeWarning::DisplayWindowStage { .. })));
}

/// A data window entirely disjoint from the display window intersects it in nothing: the
/// decode succeeds with an empty (default-pixel) stage instead of the "EXR clipped width"
/// error a negative width difference produced (#6271's review). RGB and RGBA, f32 and f16.
#[test]
fn disjoint_data_window_decodes_to_an_empty_stage() {
    let values = vec![1.0f32, 2.0, 3.0, 4.0];
    // Right of the stage: data at (10, 10), 2×2, display 6×4 at the origin.
    let bytes = exr(IntegerBounds::new((0, 0), (6, 4)), (10, 10), (2, 2), values.clone());
    let image = decode(&bytes).expect("disjoint right");
    assert_eq!(image.dimensions(), (6, 4));
    assert!((0..3).all(|c| image.get(5, 3, c) == 0.0));
    // Below-left of the stage: data at (-9, -9), 2×2.
    let bytes = exr(IntegerBounds::new((0, 0), (6, 4)), (-9, -9), (2, 2), values);
    let image = decode(&bytes).expect("disjoint below-left");
    assert_eq!(image.dimensions(), (6, 4));
    assert!((0..3).all(|c| image.get(0, 0, c) == 0.0));
}

#[test]
fn display_window_limits_are_checked_before_decode_allocation() {
    let bytes = exr(IntegerBounds::new((0, 0), (100, 100)), (0, 0), (1, 1), vec![1.0]);
    let limits = Limits { max_width: 16, max_height: 16, max_pixels: 256, max_alloc: 4096 };
    let error = decode_as_with(Format::OpenExr, &bytes, &DecodeOptions { limits, ..Default::default() }).expect_err("oversized display canvas rejected");
    assert!(matches!(error, photocraft_codecs::CodecError::LimitExceeded(_)), "{error:?}");
}
