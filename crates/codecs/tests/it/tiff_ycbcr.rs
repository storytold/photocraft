//! YCbCr TIFFs (PhotometricInterpretation 6), as scanners write them: JPEG-compressed with 2×2
//! chroma subsampling, and uncompressed with ReferenceBlackWhite and YCbCrCoefficients (#10316).
//! Every file here is hand-built by `common::tiffgen`.

use crate::common::tiffgen::*;
use photocraft_codecs::*;

const COMPRESSION: u16 = 259;
const SUBSAMPLING: u16 = 530;
const YCBCR_COEFFICIENTS: u16 = 529;
const REFERENCE_BLACK_WHITE: u16 = 532;

/// A directory whose Compression is `c`.
fn compressed(mut d: Dir, c: u64) -> Dir {
    d.entries.retain(|e| e.0 != COMPRESSION);
    d.tag(COMPRESSION, SHORT, &[c])
}

/// A little-endian RATIONAL value for `tiffgen`.
fn rational(num: u32, den: u32) -> u64 {
    u64::from(num) | (u64::from(den) << 32)
}

/// The image's pixels as RGB bytes.
fn rgb(img: &Image) -> Vec<u8> {
    img.to_rgba8().as_chunks::<4>().0.iter().flat_map(|[r, g, b, _]| [*r, *g, *b]).collect()
}

/// A smooth colour test card, `w`×`h` RGB.
fn card(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            v.extend([(x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, (255 - x * 255 / (w - 1)) as u8]);
        }
    }
    v
}

/// A 4:2:0 baseline JPEG stream of `rgb`.
fn jpeg(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let img = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, rgb.to_vec()).unwrap();
    encode(&img, Format::Jpeg, &EncodeOptions { jpeg_chroma_subsampling: true, ..Default::default() }).unwrap()
}

#[test]
fn jpeg_compressed_ycbcr_with_subsampled_chroma_reads_as_rgb() {
    let (w, h) = (32, 24);
    let stream = jpeg(&card(w, h), w, h);
    // The stream really is subsampled: the luma component samples 2×2 in the frame header.
    let sof = stream.windows(2).position(|m| m == [0xFF, 0xC0]).expect("baseline frame header");
    assert_eq!(stream[sof + 11], 0x22, "luma sampling factors");
    let dir = compressed(strips(w, h, 8, 3, 6, h, &stream), 7).tag(SUBSAMPLING, SHORT, &[2, 2]);
    let file = build(true, false, &[dir], &[0]).bytes;

    let img = decode(&file).unwrap();
    assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (w, h, ChannelLayout::Rgb, SampleType::U8));
    // The oracle: the same JPEG stream decoded to RGB by an independent decoder.
    let want = image::load_from_memory_with_format(&stream, image::ImageFormat::Jpeg).unwrap().to_rgb8().into_raw();
    let worst = rgb(&img).iter().zip(&want).map(|(g, o)| g.abs_diff(*o)).max().unwrap();
    assert!(worst <= 2, "largest difference from the oracle: {worst}");
}

#[test]
fn uncompressed_ycbcr_uses_the_default_conversion() {
    // Black, white, mid gray and JFIF's pure red, without subsampling.
    let px = [0, 128, 128, 255, 128, 128, 128, 128, 128, 76, 85, 255];
    let dir = strips(4, 1, 8, 3, 6, 1, &px).tag(SUBSAMPLING, SHORT, &[1, 1]);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(img.layout(), ChannelLayout::Rgb);
    assert_eq!(rgb(&img), [0, 0, 0, 255, 255, 255, 128, 128, 128, 254, 0, 0]);
}

#[test]
fn reference_black_white_sets_the_code_range() {
    // Studio range: Y 16–235, chroma 128 ± 112.
    let rbw = [rational(16, 1), rational(235, 1), rational(128, 1), rational(240, 1), rational(128, 1), rational(240, 1)];
    let dir = strips(2, 1, 8, 3, 6, 1, &[16, 128, 128, 235, 128, 128]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(REFERENCE_BLACK_WHITE, RATIONAL, &rbw);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(rgb(&img), [0, 0, 0, 255, 255, 255]);
}

#[test]
fn ycbcr_coefficients_set_the_luma_weights() {
    // Weights 0.25, 0.5, 0.25 with Cb −64 and Cr +64: R = Y + 1.5·Cr, B = Y + 1.5·Cb, and G keeps Y
    // (BT.601 would give R 218 and B 15).
    let weights = [rational(1, 4), rational(1, 2), rational(1, 4)];
    let dir = strips(1, 1, 8, 3, 6, 1, &[128, 64, 192]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(YCBCR_COEFFICIENTS, RATIONAL, &weights);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(rgb(&img), [224, 128, 32]);
}

#[test]
fn sixteen_bit_ycbcr_reads_as_rgb() {
    let px: Vec<u8> = [65535u16, 32768, 32768, 0, 32768, 32768].iter().flat_map(|v| v.to_le_bytes()).collect();
    let dir = strips(2, 1, 16, 3, 6, 1, &px).tag(SUBSAMPLING, SHORT, &[1, 1]);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!((img.layout(), img.sample_type()), (ChannelLayout::Rgb, SampleType::U16));
    let n = img.to_normalized();
    assert!(n[..3].iter().all(|&v| v == 1.0) && n[3..].iter().all(|&v| v == 0.0), "{n:?}");
}

#[test]
fn unsupported_ycbcr_files_fail_cleanly() {
    // Four samples (an extra sample next to YCbCr).
    let four = strips(1, 1, 8, 4, 6, 1, &[128, 128, 128, 255]).tag(SUBSAMPLING, SHORT, &[1, 1]);
    assert!(decode(&build(true, false, &[four], &[0]).bytes).is_err());
    // Subsampled chroma without JPEG compression (the default subsampling is 2×2).
    let sub = strips(2, 2, 8, 3, 6, 2, &[128; 12]);
    assert!(decode(&build(true, false, &[sub], &[0]).bytes).is_err());
    // A JPEG strip that is not JPEG data.
    let junk = compressed(strips(16, 16, 8, 3, 6, 16, &[0x5A; 64]), 7).tag(SUBSAMPLING, SHORT, &[2, 2]);
    assert!(decode(&build(true, false, &[junk], &[0]).bytes).is_err());
    // Coefficients and ReferenceBlackWhite that would divide by zero fall back to the defaults.
    let bad = strips(1, 1, 8, 3, 6, 1, &[255, 128, 128])
        .tag(SUBSAMPLING, SHORT, &[1, 1])
        .tag(YCBCR_COEFFICIENTS, RATIONAL, &[rational(0, 1), rational(0, 0), rational(5, 1)])
        .tag(REFERENCE_BLACK_WHITE, RATIONAL, &[rational(7, 1); 6]);
    let img = decode(&build(true, false, &[bad], &[0]).bytes).unwrap();
    assert_eq!(&img.to_rgba8()[..3], &[255, 255, 255]);
}
