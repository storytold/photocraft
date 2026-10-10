//! JPEG XL decoding against libjxl's encoder (feature `jxl`). The fixtures in `fixtures/jxl/` are
//! written by `scripts/jxl_fixtures.py` with `cjxl` from procedural images that `pattern`,
//! `alpha`, `smooth` and `float_sample` regenerate here, so lossless files are checked sample for
//! sample against an independent encoder and lossy ones by PSNR. `exif-xmp-orient6.src.jpg` is
//! the JPEG two of them were made from.

#![cfg(feature = "jxl")]

mod common;
use common::*;
use photocraft_codecs::*;

const RGB8: &[u8] = include_bytes!("fixtures/jxl/rgb8-lossless.jxl");
const RGBA8: &[u8] = include_bytes!("fixtures/jxl/rgba8-lossless.jxl");
const GRAY8: &[u8] = include_bytes!("fixtures/jxl/gray8-lossless.jxl");
const GRAYA16: &[u8] = include_bytes!("fixtures/jxl/graya16-lossless.jxl");
const RGB16: &[u8] = include_bytes!("fixtures/jxl/rgb16-lossless.jxl");
const RGB10: &[u8] = include_bytes!("fixtures/jxl/rgb10-lossless.jxl");
const RGB_F32: &[u8] = include_bytes!("fixtures/jxl/rgb-f32-lossless.jxl");
const LOSSY: &[u8] = include_bytes!("fixtures/jxl/rgb8-lossy.jxl");
const P3: &[u8] = include_bytes!("fixtures/jxl/rgb8-p3.jxl");
const EXIF_XMP_ORIENT6: &[u8] = include_bytes!("fixtures/jxl/exif-xmp-orient6.jxl");
const JPEG_RECON: &[u8] = include_bytes!("fixtures/jxl/jpeg-recon.jxl");
const SOURCE_JPEG: &[u8] = include_bytes!("fixtures/jxl/exif-xmp-orient6.src.jpg");
const ANIMATED: &[u8] = include_bytes!("fixtures/jxl/animated.jxl");

const W: u32 = 12;
const H: u32 = 9;

/// Integer sample of the lossless fixtures (mirrors `pattern` in the script).
fn pattern(x: u32, y: u32, c: u32, maxval: u32) -> u32 {
    (x * 31 + y * 17 + c * 101 + 7) * 13 % (maxval + 1)
}

/// Alpha gradient from fully transparent (top-left) to fully opaque (bottom-right).
fn alpha(x: u32, y: u32, maxval: u32) -> u32 {
    ((x + y) * maxval / (W + H - 4)).min(maxval)
}

/// The smooth 8-bit image of the lossy fixture and the source JPEG.
fn smooth(x: u32, y: u32, c: usize) -> f32 {
    [x * 255 / (W - 1), y * 255 / (H - 1), 128][c] as f32 / 255.0
}

/// Exact binary fractions, some above 1.0.
fn float_sample(x: u32, y: u32, c: u32) -> f32 {
    (x * 3 + y * 5 + c * 7) as f32 / 64.0
}

fn keep() -> DecodeOptions {
    DecodeOptions { keep_orientation: true, ..Default::default() }
}

/// The image the smooth JPEG decodes to, as coded (12x9), by our own JPEG decoder.
fn source_jpeg() -> Image {
    decode_with(SOURCE_JPEG, &keep()).unwrap()
}

fn smooth_image() -> Image {
    let v: Vec<f32> = (0..H).flat_map(|y| (0..W).flat_map(move |x| (0..3).map(move |c| smooth(x, y, c)))).collect();
    Image::from_normalized(W, H, ChannelLayout::Rgb, SampleType::U8, &v).unwrap()
}

#[test]
fn jxl_is_read_only_and_listed() {
    let c = caps(Format::Jxl);
    assert!(c.read && !c.write);
    assert!(c.icc && c.exif && c.xmp && c.animation);
    assert_eq!(c.depths, &[SampleType::U8, SampleType::U16, SampleType::F16, SampleType::F32]);
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Jxl));
    for ext in ["jxl", "JXL", "photo.jxl", "dir/IMG_0001.JXL"] {
        assert_eq!(from_extension(ext), Some(Format::Jxl), "{ext}");
    }
    assert_eq!(Format::Jxl.mime_type(), "image/jxl");
    let img = decode(RGB8).unwrap();
    assert!(matches!(encode(&img, Format::Jxl, &EncodeOptions::default()), Err(CodecError::Unsupported { .. })));
    assert_eq!(fidelity_warnings(&img, Format::Jxl), vec![FidelityWarning::WriteUnsupported { format: Format::Jxl }]);
}

#[test]
fn detect_bare_codestreams_and_containers() {
    assert!(RGB8.starts_with(&[0xFF, 0x0A]), "cjxl writes a bare codestream for plain PNG input");
    assert!(RGB16.starts_with(b"\0\0\0\x0cJXL "), "and a container when it needs a level box");
    for f in [RGB8, RGBA8, GRAY8, GRAYA16, RGB16, RGB10, RGB_F32, LOSSY, P3, EXIF_XMP_ORIENT6, JPEG_RECON, ANIMATED] {
        assert_eq!(detect(f), Some(Format::Jxl));
    }
    assert_eq!(detect(&[0xFF, 0x0A]), Some(Format::Jxl), "two bytes are enough to know");
    assert_eq!(detect(&[0xFF, 0xD8, 0xFF]), Some(Format::Jpeg), "a JPEG is not mistaken for it");
    assert_eq!(detect(b"\0\0\0\x0cJXL \r\n\x87\x0b"), None, "the signature box is checked to its last byte");
}

#[test]
fn lossless_8_bit_rgb_rgba_and_gray_decode_exactly() {
    for (bytes, layout) in [(RGB8, ChannelLayout::Rgb), (RGBA8, ChannelLayout::Rgba), (GRAY8, ChannelLayout::Gray)] {
        let img = decode(bytes).unwrap();
        assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), layout, SampleType::U8));
        assert!(img.icc.is_none(), "sRGB needs no profile");
        assert!(img.meta.is_empty() && img.warnings.is_empty());
        let nc = layout.channels() as u32;
        let data = img.data();
        for y in 0..H {
            for x in 0..W {
                for c in 0..nc {
                    let want = if layout.has_alpha() && c == nc - 1 { alpha(x, y, 255) } else { pattern(x, y, c, 255) };
                    let got = data[((y * W + x) * nc + c) as usize] as u32;
                    assert_eq!(got, want, "{layout:?} at {x},{y} channel {c}");
                }
            }
        }
    }
}

#[test]
fn lossless_16_bit_decodes_exactly_and_keeps_its_depth() {
    let img = decode(RGB16).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgb, SampleType::U16));
    let s = img.to_u16_samples().unwrap();
    for y in 0..H {
        for x in 0..W {
            for c in 0..3 {
                assert_eq!(s[((y * W + x) * 3 + c) as usize] as u32, pattern(x, y, c, 65535), "at {x},{y} channel {c}");
            }
        }
    }
    // Gray + alpha: 16-bit gray, and an alpha channel cjxl 0.7 stores at 8 bits (so the fixture's
    // alpha values are multiples of 257): a per-channel depth the decoder scales to 16 bits.
    let img = decode(GRAYA16).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::GrayA, SampleType::U16));
    let s = img.to_u16_samples().unwrap();
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 2) as usize;
            assert_eq!(s[i] as u32, pattern(x, y, 0, 65535), "gray at {x},{y}");
            assert_eq!(s[i + 1] as u32, alpha(x, y, 255) * 257, "alpha at {x},{y}");
        }
    }
}

#[test]
fn ten_bit_samples_decode_to_16_bit_scaled_to_full_range() {
    let img = decode(RGB10).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgb, SampleType::U16));
    let s = img.to_u16_samples().unwrap();
    for y in 0..H {
        for x in 0..W {
            for c in 0..3 {
                let want = (pattern(x, y, c, 1023) as f64 * 65535.0 / 1023.0).round() as i64;
                let got = s[((y * W + x) * 3 + c) as usize] as i64;
                assert!((got - want).abs() <= 1, "at {x},{y} channel {c}: {got} vs {want}");
            }
        }
    }
    // Scaled, not left in the low bits.
    assert!(s.iter().any(|&v| v > 1023));
}

#[test]
fn lossless_float_decodes_exactly_including_values_above_one() {
    let img = decode(RGB_F32).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgb, SampleType::F32));
    let s = img.to_f32_samples().unwrap();
    for y in 0..H {
        for x in 0..W {
            for c in 0..3 {
                assert_eq!(s[((y * W + x) * 3 + c) as usize], float_sample(x, y, c), "at {x},{y} channel {c}");
            }
        }
    }
    assert!(img.has_out_of_range(), "HDR values are kept, not clamped");
}

#[test]
fn lossy_vardct_is_close_to_the_source() {
    let img = decode(LOSSY).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgb, SampleType::U8));
    let want = smooth_image();
    assert!(psnr(&img, &want) > 30.0, "PSNR {}", psnr(&img, &want));
    assert!(max_abs_diff(&img, &want) < 0.1);
}

#[test]
fn a_non_srgb_colour_encoding_comes_with_a_profile() {
    let img = decode(P3).unwrap();
    let icc = img.icc.as_deref().expect("Display P3 needs a profile");
    assert!(icc.len() > 128 && &icc[36..40] == b"acsp", "a valid ICC header");
    // The samples themselves are untouched: same pattern as the sRGB-tagged file.
    assert_eq!(img.data(), decode(RGB8).unwrap().data());
}

#[test]
fn orientation_is_applied_once_and_can_be_kept() {
    // The header says 6 (turn 90° clockwise to display): the 12x9 coded image is shown 9x12.
    let img = decode(EXIF_XMP_ORIENT6).unwrap();
    assert_eq!(img.dimensions(), (H, W));
    let coded = decode_with(EXIF_XMP_ORIENT6, &keep()).unwrap();
    assert_eq!(coded.dimensions(), (W, H), "keep_orientation hands back the pixels as coded");
    // The decoder's turn matches ours: turning the coded pixels with EXIF orientation 6 gives
    // the upright image.
    assert_eq!(coded.clone().oriented(6).unwrap().data(), img.data());
    // The pixels are the source JPEG's (losslessly re-encoded).
    assert!(psnr(&coded, &source_jpeg()) > 45.0, "{}", psnr(&coded, &source_jpeg()));
}

#[test]
fn exif_and_xmp_survive_and_their_orientation_is_not_applied_twice() {
    let img = decode(EXIF_XMP_ORIENT6).unwrap();
    let exif = img.meta.exif.as_deref().unwrap();
    assert!(exif.starts_with(b"MM\0*"), "TIFF-structured EXIF, without the box's offset header");
    assert!(exif.windows(26).any(|w| w == b"PhotoCraft JPEG XL fixture"), "the ImageDescription is there");
    assert_eq!(exif_orientation(exif), 1, "rewritten so exports don't turn it again");
    let xmp = img.meta.xmp.as_deref().unwrap();
    assert!(xmp.contains("x:xmpmeta") && xmp.contains("PhotoCraft JPEG XL fixture"));
    assert!(xmp.contains("tiff:Orientation=\"1\""), "{xmp}");

    let raw = decode_with(EXIF_XMP_ORIENT6, &keep()).unwrap();
    assert_eq!(exif_orientation(raw.meta.exif.as_deref().unwrap()), 6, "kept verbatim when asked");
    assert!(raw.meta.xmp.as_deref().unwrap().contains("tiff:Orientation=\"6\""));
}

#[test]
fn a_recompressed_jpeg_decodes_to_the_jpegs_pixels() {
    // cjxl -j 1: the JPEG's DCT coefficients, stored losslessly; the header took the EXIF
    // orientation (6) from the JPEG, so compare as coded.
    let img = decode_with(JPEG_RECON, &keep()).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgb, SampleType::U8));
    let want = source_jpeg();
    assert!(psnr(&img, &want) > 40.0, "PSNR {}", psnr(&img, &want));
    assert_eq!(decode(JPEG_RECON).unwrap().dimensions(), (H, W));
}

#[test]
fn an_animation_opens_at_its_first_frame_with_a_warning() {
    let img = decode(ANIMATED).unwrap();
    assert_eq!((img.dimensions(), img.layout(), img.sample_type()), ((W, H), ChannelLayout::Rgba, SampleType::U8));
    assert_eq!(img.warnings, vec![DecodeWarning::MoreFrames { total: Some(3) }]);
    // Frame 0 of the GIF: red rises with x, green with y, no blue.
    assert!(img.get(11, 0, 0) > img.get(0, 0, 0) && img.get(0, 8, 1) > img.get(0, 0, 1));
    assert!((0..H).all(|y| (0..W).all(|x| img.get(x, y, 2) == 0.0 && img.get(x, y, 3) == 1.0)));
}

#[test]
fn a_truncated_animation_still_says_there_were_more_frames() {
    // Cut the 3-frame file shorter and shorter: the longest prefix that still decodes holds
    // fewer than three frames, and must say so even when it cannot say how many.
    let decoded = (1..ANIMATED.len()).rev().find_map(|cut| decode_as(Format::Jxl, &ANIMATED[..cut]).ok().map(|img| (cut, img)));
    let (cut, img) = decoded.expect("some prefix decodes its first frame");
    assert!(cut < ANIMATED.len());
    assert_eq!(img.dimensions(), (W, H));
    assert!(matches!(img.warnings[..], [DecodeWarning::MoreFrames { .. }]), "{:?} at cut {cut}", img.warnings);
}

#[test]
fn limits_are_checked_before_decoding() {
    let tight = |limits| DecodeOptions { limits, ..Default::default() };
    let narrow = Limits { max_width: 11, ..Default::default() };
    assert!(matches!(decode_with(RGB8, &tight(narrow)), Err(CodecError::LimitExceeded(_))));
    // Orientation 6 swaps the sides: the coded 12x9 and the upright 9x12 must both fit.
    let short = Limits { max_height: 10, ..Default::default() };
    assert!(matches!(decode_with(EXIF_XMP_ORIENT6, &tight(short)), Err(CodecError::LimitExceeded(_))));
    assert!(decode_with(RGB8, &tight(short)).is_ok());
    let few = Limits { max_pixels: 100, ..Default::default() };
    assert!(matches!(decode_with(RGB8, &tight(few)), Err(CodecError::LimitExceeded(_))));
    let bytes = Limits { max_alloc: (W * H * 3 - 1) as u64, ..Default::default() };
    assert!(matches!(decode_with(RGB8, &tight(bytes)), Err(CodecError::LimitExceeded(_))));
    let floats = Limits { max_alloc: (W * H * 3 * 4 - 1) as u64, ..Default::default() };
    assert!(matches!(decode_with(RGB_F32, &tight(floats)), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn broken_files_are_clear_errors() {
    assert!(matches!(decode_as(Format::Jxl, b""), Err(CodecError::Malformed { format: Format::Jxl, .. })));
    assert!(matches!(decode_as(Format::Jxl, &RGB8[..RGB8.len() / 2]), Err(CodecError::Malformed { .. })), "half a file");
    let mut flipped = RGB16.to_vec();
    flipped[100] ^= 0xFF;
    let _ = decode_as(Format::Jxl, &flipped);
}

#[test]
fn every_truncation_errors_or_decodes_never_panics() {
    for bytes in [RGB8, RGBA8, RGB16, RGB_F32, LOSSY, EXIF_XMP_ORIENT6, JPEG_RECON, ANIMATED] {
        for cut in 0..bytes.len() {
            for opts in [DecodeOptions::default(), keep()] {
                let _ = decode_as_with(Format::Jxl, &bytes[..cut], &opts);
            }
        }
    }
}
