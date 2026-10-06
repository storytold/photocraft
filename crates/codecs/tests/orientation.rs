//! Asymmetric fixtures catch both incorrect rotations and mirrored EXIF orientations.

use std::io::Cursor;

use image::ImageDecoder;
use photocraft_codecs::*;

const ORDER: [[usize; 6]; 8] = [
    [0, 1, 2, 3, 4, 5],
    [2, 1, 0, 5, 4, 3],
    [5, 4, 3, 2, 1, 0],
    [3, 4, 5, 0, 1, 2],
    [0, 3, 1, 4, 2, 5],
    [3, 0, 4, 1, 5, 2],
    [5, 2, 4, 1, 3, 0],
    [2, 5, 1, 4, 0, 3],
];

fn exif(orientation: u16, big: bool) -> Vec<u8> {
    let short = |n: u16| if big { n.to_be_bytes() } else { n.to_le_bytes() };
    let long = |n: u32| if big { n.to_be_bytes() } else { n.to_le_bytes() };
    let mut b = if big { b"MM".to_vec() } else { b"II".to_vec() };
    b.extend(short(42));
    b.extend(long(8));
    b.extend(short(2));
    // Unrelated inline Software string must survive byte-for-byte.
    b.extend(short(0x0131));
    b.extend(short(2));
    b.extend(long(4));
    b.extend(b"PC\0\0");
    b.extend(short(0x0112));
    b.extend(short(3));
    b.extend(long(1));
    b.extend(short(orientation));
    b.extend([0, 0]);
    b.extend(long(0));
    b
}

fn fixture() -> Image {
    Image::from_u8(3, 2, ChannelLayout::Rgb, vec![20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190]).unwrap()
}

fn opts() -> EncodeOptions {
    EncodeOptions { jpeg_quality: 100, jpeg_chroma_subsampling: false, ..Default::default() }
}

#[test]
fn all_exif_orientations_and_endiannesses_decode_upright() {
    for format in [Format::Jpeg, Format::Png, Format::WebP] {
        let base = fixture();
        // The same compressed pixels without EXIF provide the JPEG's quantized reference.
        let raw = decode(&encode(&base, format, &opts()).unwrap()).unwrap();
        for big in [false, true] {
            for orientation in 1..=8 {
                let mut src = base.clone();
                src.meta.exif = Some(exif(orientation, big));
                src.meta.xmp = Some("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">keep me</x:xmpmeta>".into());
                let got = decode(&encode(&src, format, &opts()).unwrap()).unwrap();
                let dims = if orientation >= 5 { (2, 3) } else { (3, 2) };
                assert_eq!(got.dimensions(), dims, "{format:?}, {orientation}, big={big}");
                let expected: Vec<u8> = ORDER[orientation as usize - 1].iter().flat_map(|&i| raw.data()[i * 3..i * 3 + 3].iter().copied()).collect();
                assert_eq!(got.data(), expected, "{format:?}, {orientation}, big={big}");
                assert_eq!(got.meta.exif, Some(exif(1, big)));
                assert_eq!(got.meta.xmp, src.meta.xmp);
            }
        }
    }
}

#[test]
fn normalized_exports_have_raw_orientation_one_and_do_not_rotate_again() {
    let mut src = fixture();
    src.meta.exif = Some(exif(6, false));
    let upright = decode(&encode(&src, Format::Jpeg, &opts()).unwrap()).unwrap();
    assert_eq!(upright.dimensions(), (2, 3));
    for format in [Format::Jpeg, Format::Png, Format::WebP] {
        let bytes = encode(&upright, format, &opts()).unwrap();
        // Inspect the encoded metadata independently: our own decode normalizes it.
        let (dimensions, metadata) = if format == Format::WebP {
            let mut oracle = image_webp::WebPDecoder::new(Cursor::new(&bytes)).unwrap();
            (oracle.dimensions(), oracle.exif_metadata().unwrap().unwrap())
        } else {
            let mut oracle = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().unwrap().into_decoder().unwrap();
            (oracle.dimensions(), oracle.exif_metadata().unwrap().unwrap())
        };
        assert_eq!(dimensions, (2, 3));
        assert!(metadata.ends_with(&exif(1, false)), "{format:?}");
        let again = decode(&bytes).unwrap();
        assert_eq!(again.dimensions(), upright.dimensions());
        if format != Format::Jpeg {
            assert_eq!(again.data(), upright.data());
        }
    }
}

#[test]
fn png_sixteen_bit_alpha_and_resolution_are_oriented_without_conversion() {
    let values: Vec<u16> = (0..24).map(|n| n * 2003).collect();
    let mut src = Image::from_u16(3, 2, ChannelLayout::Rgba, &values).unwrap();
    src.meta.exif = Some(exif(8, true));
    src.meta.dpi = Some((300.0, 150.0));
    let got = decode(&encode(&src, Format::Png, &opts()).unwrap()).unwrap();
    assert_eq!(got.dimensions(), (2, 3));
    assert_eq!(got.sample_type(), SampleType::U16);
    assert_eq!(got.layout(), ChannelLayout::Rgba);
    let expected: Vec<u16> = ORDER[7].iter().flat_map(|&i| values[i * 4..i * 4 + 4].iter().copied()).collect();
    assert_eq!(got.to_u16_samples().unwrap(), expected);
    let (x, y) = got.meta.dpi.unwrap();
    assert!((x - 150.0).abs() < 0.05 && (y - 300.0).abs() < 0.05);
}

#[test]
fn malformed_or_unsupported_orientation_is_preserved_without_transform_or_panic() {
    let valid = exif(6, false);
    let mut cases: Vec<Vec<u8>> = (0..valid.len()).map(|n| valid[..n].to_vec()).collect();
    cases.extend([exif(0, false), exif(9, true), exif(u16::MAX, false)]);
    for (range, bytes) in [
        (0..2, b"xx".to_vec()),
        (2..4, 43u16.to_le_bytes().to_vec()),
        (4..8, u32::MAX.to_le_bytes().to_vec()),
        (4..8, 0u32.to_le_bytes().to_vec()),
        (8..10, u16::MAX.to_le_bytes().to_vec()),
        (24..26, 4u16.to_le_bytes().to_vec()), // Orientation must be SHORT, not LONG.
        (26..30, u32::MAX.to_le_bytes().to_vec()),
    ] {
        let mut bad = valid.clone();
        bad[range].copy_from_slice(&bytes);
        cases.push(bad);
    }
    for b in cases {
        let mut src = fixture();
        src.meta.exif = Some(b.clone());
        let got = decode(&encode(&src, Format::Jpeg, &opts()).unwrap()).unwrap();
        let raw = decode(&encode(&fixture(), Format::Jpeg, &opts()).unwrap()).unwrap();
        assert_eq!(got.data(), raw.data());
        assert_eq!(got.dimensions(), (3, 2));
        assert_eq!(got.meta.exif, Some(b));
    }
}

#[test]
fn limits_also_apply_to_oriented_dimensions() {
    let mut src = fixture();
    src.meta.exif = Some(exif(6, false));
    let bytes = encode(&src, Format::Jpeg, &opts()).unwrap();
    let options = DecodeOptions { limits: Limits { max_width: 3, max_height: 2, ..Default::default() } };
    assert!(matches!(decode_with(&bytes, &options), Err(CodecError::LimitExceeded(_))));
}

/// Run explicitly in release to measure the added work on a camera-sized image.
#[test]
#[ignore = "24 MP release benchmark"]
fn orientation_decode_24mp_timing() {
    use std::time::{Duration, Instant};

    let mut src = Image::from_u8(6000, 4000, ChannelLayout::Rgb, vec![100; 6000 * 4000 * 3]).unwrap();
    src.meta.exif = Some(exif(1, false));
    let normal = encode(&src, Format::Jpeg, &opts()).unwrap();
    let mut rotated = normal.clone();
    let tag = exif(1, false);
    let start = rotated.windows(tag.len()).position(|b| b == tag).unwrap();
    rotated[start..start + tag.len()].copy_from_slice(&exif(6, false));
    let mut times = [Duration::MAX; 2];
    for _ in 0..3 {
        for (i, bytes) in [&normal, &rotated].into_iter().enumerate() {
            let start = Instant::now();
            let img = decode(std::hint::black_box(bytes)).unwrap();
            times[i] = times[i].min(start.elapsed());
            assert_eq!(img.dimensions(), if i == 0 { (6000, 4000) } else { (4000, 6000) });
            assert_eq!(img.meta.exif, Some(exif(1, false)));
            std::hint::black_box(img);
        }
    }
    println!("24 MP JPEG decode, best of 3: identity={:.1} ms; orientation6={:.1} ms", times[0].as_secs_f64() * 1000.0, times[1].as_secs_f64() * 1000.0);
}
