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
    // Source data begins at (1, 1), 2×2, inside a 4×4 display stage at (0, 0).
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

/// A data window not fully inside the display window keeps the data window as its canvas:
/// a window entirely disjoint from the stage (or shifted past its edge) would lose texels
/// to clipping, so nothing is staged and every sample stays visible. Flat and deep EXRs
/// follow the same rule (`deep_exr` stages only position-contained windows).
#[test]
fn disjoint_data_window_keeps_its_data_window() {
    let values = vec![1.0f32, 2.0, 3.0, 4.0];
    // Right of the stage: data at (10, 10), 2×2, display 6×4 at the origin.
    let bytes = exr(IntegerBounds::new((0, 0), (6, 4)), (10, 10), (2, 2), values.clone());
    let image = decode(&bytes).expect("disjoint right");
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(image.get(0, 0, 0), 1.0);
    assert_eq!(image.get(1, 1, 0), 4.0);
    assert!(image.warnings.is_empty(), "nothing is claimed for a stage the data does not lie in");
    // Below-left of the stage: data at (-9, -9), 2×2.
    let bytes = exr(IntegerBounds::new((0, 0), (6, 4)), (-9, -9), (2, 2), values);
    let image = decode(&bytes).expect("disjoint below-left");
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(image.get(1, 1, 0), 4.0);
}

/// The fit rule counts positions, not just sizes: a same-sized data window shifted by one
/// is not contained, and staging it would drop the samples outside the stage (#6271's
/// review: 3 of 4 pixels). It keeps its data window with nothing lost.
#[test]
fn a_data_window_shifted_past_the_stage_edge_loses_no_pixels() {
    let values = vec![9.0f32, 8.0, 7.0, 6.0];
    let bytes = exr(IntegerBounds::new((0, 0), (2, 2)), (1, 1), (2, 2), values);
    let image = decode(&bytes).expect("shifted data window decodes");
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(image.get(0, 0, 0), 9.0);
    assert_eq!(image.get(1, 0, 0), 8.0);
    assert_eq!(image.get(0, 1, 0), 7.0);
    assert_eq!(image.get(1, 1, 0), 6.0);
    assert!(image.warnings.is_empty(), "no staging is claimed, so no stage warning");
}

/// The named-channel path (`decode_exr_channels`) picks the same canvas as `decode` and
/// `decode_exr_part`: a part the stage cannot hold stays on its data window there too
/// (#6271's review: the part layer and the group layer differed in size).
#[test]
fn named_channels_pick_the_same_canvas_as_the_whole_file_decode() {
    let values = (0..12).map(|v| v as f32 + 1.0).collect();
    let bytes = exr(IntegerBounds::new((0, 0), (2, 2)), (-2, -1), (4, 3), values);
    let image = decode(&bytes).expect("whole-file decode");
    assert_eq!(image.dimensions(), (4, 3));
    let channels = decode_exr_channels(&bytes, 0, &["R"], &DecodeOptions::default()).expect("named channel decode");
    assert_eq!(channels.dimensions(), (4, 3), "the named-channel path keeps the data window too");
    assert_eq!(channels.get(0, 0, 0), 1.0);
    assert_eq!(channels.get(3, 2, 0), 12.0);
}

#[test]
fn display_window_limits_are_checked_before_decode_allocation() {
    let bytes = exr(IntegerBounds::new((0, 0), (100, 100)), (0, 0), (1, 1), vec![1.0]);
    let limits = Limits { max_width: 16, max_height: 16, max_pixels: 256, max_alloc: 4096 };
    let error = decode_as_with(Format::OpenExr, &bytes, &DecodeOptions { limits, ..Default::default() }).expect_err("oversized display canvas rejected");
    assert!(matches!(error, photocraft_codecs::CodecError::LimitExceeded(_)), "{error:?}");
}

/// The admission in `read_meta` runs before any pixel data is read, not only before its
/// result is checked: this file's single chunk is cut off, so a decode that reached
/// `read_layer` would fail reading pixels (a malformed error). With limits the display
/// window cannot pass, the refusal is `LimitExceeded` from admission — the ordering the
/// allocation-DoS fix depends on (#6271's review: 1464 MiB peak RSS before the error).
#[test]
fn an_over_limit_display_window_is_refused_before_pixel_data_is_read() {
    let full = exr(IntegerBounds::new((0, 0), (100, 100)), (0, 0), (1, 1), vec![1.0]);
    // One scanline chunk: y coordinate (4 bytes) + 1 pixel of RGB f32 (12 bytes).
    let cut = &full[..full.len() - 16];
    // The truncation is real: with default limits the read of pixels itself fails.
    let error = decode(&cut).expect_err("truncated pixels fail");
    assert!(!matches!(error, photocraft_codecs::CodecError::LimitExceeded(_)), "truncation, not admission, fails here: {error:?}");
    let limits = Limits { max_width: 16, max_height: 16, max_pixels: 256, max_alloc: 4096 };
    let error = decode_as_with(Format::OpenExr, cut, &DecodeOptions { limits, ..Default::default() }).expect_err("oversized display canvas rejected");
    assert!(matches!(error, photocraft_codecs::CodecError::LimitExceeded(_)), "refused at admission, before reading pixels: {error:?}");
}

/// `probe_size` and `decode` agree because both run the same admission: a display window
/// over the default limits is refused by the probe too (it cannot report a size decode
/// rejects, #6271's review), and a stageable file probes at its staged size.
#[test]
fn probe_size_agrees_with_decode_on_staged_and_over_limit_canvases() {
    let bytes = exr(IntegerBounds::new((0, 0), (4, 4)), (1, 1), (2, 2), vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(photocraft_codecs::probe_size(&bytes).expect("probe staged EXR"), (4, 4));
    assert_eq!(decode(&bytes).unwrap().dimensions(), (4, 4));
    let hostile = exr(IntegerBounds::new((0, 0), (100000, 100000)), (0, 0), (1, 1), vec![1.0]);
    assert!(matches!(photocraft_codecs::probe_size(&hostile), Err(photocraft_codecs::CodecError::LimitExceeded(_))), "probe refuses what decode refuses");
    assert!(matches!(decode(&hostile), Err(photocraft_codecs::CodecError::LimitExceeded(_))));
}

/// Timings for the row-wise staging copy (AGENTS rule 8; run with
/// `cargo test --release -p photocraft-codecs --test it exr_staging_timings -- --ignored --nocapture`).
/// Prints only; asserts nothing (no timing asserts). 24 MP RGB f32 with no staging — the
/// common case the review's per-sample `/` and `%` walked, where the canvas is the data
/// window itself. The staged case runs the same row walk at an offset.
#[test]
#[ignore = "timing printout only"]
fn exr_staging_timings() {
    let (w, h) = (6000usize, 4000usize);
    let values: Vec<f32> = (0..w * h * 3).map(|i| ((i % 97) as f32) / 97.0).collect();
    let image = photocraft_codecs::Image::from_f32(w as u32, h as u32, ChannelLayout::Rgb, &values).expect("24 MP fixture");
    let plain = photocraft_codecs::encode(&image, Format::OpenExr, &Default::default()).expect("encode");
    let mut best = u128::MAX;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let img = decode(&plain).expect("decode 24 MP");
        let _ = img.data().len();
        best = best.min(t.elapsed().as_millis());
    }
    println!("24 MP RGB f32 decode (uncompressed, no staging), best of 3: {best} ms");
}
