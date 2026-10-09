//! Synthetic pixels; no third-party assets. ravif and rav1d are independent encoder/decoder
//! implementations. An additional FFmpeg/libdav1d oracle check is opt-in below.
#![cfg(all(feature = "avif", not(target_arch = "wasm32")))]
use photocraft_codecs::*;

fn image(sample: SampleType, alpha: bool) -> Image {
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let mut values = Vec::new();
    for y in 0..17 {
        for x in 0..31 {
            values.extend([x as f32 / 30.0, y as f32 / 16.0, 0.37]);
            if alpha {
                values.push(x as f32 / 30.0);
            }
        }
    }
    Image::from_normalized(31, 17, layout, sample, &values).unwrap()
}

#[test]
fn depths_alpha_profiles_and_lossy_error() {
    for (sample, depth) in [(SampleType::U8, 8), (SampleType::U16, 10), (SampleType::F32, 10)] {
        for alpha in [false, true] {
            let mut img = image(sample, alpha);
            // Codec preserves opaque profile bytes; CMS validation belongs to the I/O layer.
            img.icc = Some(b"synthetic codec profile".to_vec());
            let options = EncodeOptions { avif_quality: 100, avif_depth: depth, ..Default::default() };
            let bytes = encode(&img, Format::Avif, &options).unwrap();
            assert_eq!(detect(&bytes), Some(Format::Avif));
            let payload = avif_parse::read_avif(&mut bytes.as_slice()).unwrap();
            assert_eq!(payload.primary_item_metadata().unwrap().bit_depth, depth);
            assert_eq!(payload.alpha_item.is_some(), alpha);
            let decoded = decode(&bytes).unwrap();
            assert_eq!(decoded.icc, img.icc);
            assert_eq!(decoded.dimensions(), img.dimensions());
            assert_eq!(decoded.layout(), img.layout());
            assert_eq!(decoded.sample_type(), if depth == 8 { SampleType::U8 } else { SampleType::U16 });
            let expected = img.to_normalized();
            let actual = decoded.to_normalized();
            let max = expected.iter().zip(&actual).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            // RGB quantization + source/storage rounding: quality 100 is explicitly lossy.
            assert!(max < 0.012, "{sample:?}/{depth}/{alpha}: maximum normalized error {max}");
            assert!(fidelity_warnings_with(&img, Format::Avif, &options).contains(&FidelityWarning::LossyCompression));
        }
    }
}

#[test]
fn options_and_limits_fail_without_panicking() {
    let img = image(SampleType::U8, true);
    for options in [
        EncodeOptions { avif_quality: 0, ..Default::default() },
        EncodeOptions { avif_quality: 101, ..Default::default() },
        EncodeOptions { avif_speed: 0, ..Default::default() },
        EncodeOptions { avif_speed: 11, ..Default::default() },
        EncodeOptions { avif_depth: 12, ..Default::default() },
        EncodeOptions { avif_alpha_quality: 0, ..Default::default() },
    ] {
        assert!(encode(&img, Format::Avif, &options).is_err());
    }
    let bytes = encode(&img, Format::Avif, &Default::default()).unwrap();
    for limits in [
        Limits { max_width: 30, ..Default::default() },
        Limits { max_height: 16, ..Default::default() },
        Limits { max_pixels: 526, ..Default::default() },
        Limits { max_alloc: 31 * 17 * 8, ..Default::default() },
    ] {
        assert!(matches!(decode_with(&bytes, &DecodeOptions { limits, ..Default::default() }), Err(CodecError::LimitExceeded(_))));
    }
    for n in 0..bytes.len() {
        assert!(decode_as(Format::Avif, &bytes[..n]).is_err(), "truncated at {n}");
    }
    assert!(encode(&image(SampleType::U16, false).convert(ChannelLayout::Cmyk, SampleType::U16), Format::Avif, &Default::default()).is_err());
}

#[test]
fn untagged_cicp_is_explicit_and_sequence_is_rejected() {
    let img = image(SampleType::U8, false);
    let mut bytes = encode(&img, Format::Avif, &Default::default()).unwrap();
    assert_eq!(decode(&bytes).unwrap().meta.cicp, Some((1, 13)));
    bytes[8..12].copy_from_slice(b"avis");
    assert!(matches!(decode(&bytes), Err(CodecError::Unsupported { reason, .. }) if reason.contains("sequences")));
}

#[test]
fn random_container_bytes_never_panic() {
    // Bounded adversarial lengths/counts, including oversized and extended-size boxes.
    for value in [0, 1, 7, 8, 16, u32::MAX] {
        let mut b = value.to_be_bytes().to_vec();
        b.extend_from_slice(b"ftypavif\0\0\0\0");
        assert!(decode_as(Format::Avif, &b).is_err());
    }
}

#[test]
fn independent_fixtures_reject_truncation_and_corrupt_av1_payloads() {
    let options = DecodeOptions { limits: Limits { max_width: 128, max_height: 128, max_pixels: 128 * 128, max_alloc: 1024 * 1024 }, ..Default::default() };
    for bytes in [include_bytes!("fixtures/avif/gradient12.avif").as_slice(), include_bytes!("fixtures/avif/gradient-general-header.avif").as_slice()] {
        // First establish that the same budgets allow the intact independent fixture.
        assert_eq!(decode_with(bytes, &options).unwrap().dimensions(), (32, 18));
        for end in 0..bytes.len() {
            assert!(decode_as_with(Format::Avif, &bytes[..end], &options).is_err(), "truncated at {end}");
        }
        let mut reader = bytes;
        let payload = avif_parse::read_avif(&mut reader).unwrap();
        let start = bytes.windows(payload.primary_item.len()).position(|window| window == payload.primary_item.as_slice()).unwrap();
        // Preserve the container and its offsets, corrupt only the AV1 bytes.
        for value in [0, 255] {
            let mut corrupt = bytes.to_vec();
            corrupt[start..start + payload.primary_item.len()].fill(value);
            assert!(decode_with(&corrupt, &options).is_err(), "invalid AV1 payload filled with {value}");
        }
        // Keep the sequence header valid so metadata checks pass and rav1d itself
        // receives the malformed trailing OBUs (forbidden header bits set).
        let header_end =
            (1..payload.primary_item.len()).find(|end| avif_parse::AV1Metadata::parse_av1_bitstream(&payload.primary_item[..*end]).is_ok()).unwrap();
        let mut corrupt = bytes.to_vec();
        corrupt[start + header_end..start + payload.primary_item.len()].fill(255);
        assert!(avif_parse::AV1Metadata::parse_av1_bitstream(&corrupt[start..start + payload.primary_item.len()]).is_ok());
        assert!(matches!(decode_with(&corrupt, &options), Err(CodecError::Malformed { .. })));
        println!("{} truncations, two corrupt payloads and malformed trailing AV1 OBUs rejected", bytes.len());
    }
}

#[test]
fn independent_twelve_bit_fixture_and_hostile_dimensions() {
    let bytes = include_bytes!("fixtures/avif/gradient12.avif");
    let oracle = include_bytes!("fixtures/avif/gradient12.rgb48le");
    let decoded = decode(bytes).unwrap();
    assert_eq!(decoded.dimensions(), (32, 18));
    assert_eq!(decoded.sample_type(), SampleType::U16);
    let error = decoded
        .data()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(oracle.as_chunks::<2>().0.iter())
        .map(|(a, b)| (i32::from(u16::from_ne_bytes(*a)) - i32::from(u16::from_le_bytes(*b))).abs())
        .max()
        .unwrap();
    // FFmpeg's zscale YUV conversion and our float matrix rounding differ slightly.
    assert!(error <= 128, "maximum 16-bit sample difference from libdav1d/zscale: {error}");
    let mut hostile = bytes.to_vec();
    let p = hostile.windows(4).position(|w| w == b"ispe").unwrap();
    hostile[p + 8..p + 16].copy_from_slice(&[255; 8]);
    assert!(matches!(decode(&hostile), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn still_item_with_a_general_av1_header_is_supported() {
    // A single-image FFmpeg/libaom AVIF with -still-picture 0. AVIF 1.2 §2.1
    // does not require the optional AV1 still_picture header optimization.
    let bytes = include_bytes!("fixtures/avif/gradient-general-header.avif");
    let payload = avif_parse::read_avif(&mut bytes.as_slice()).unwrap();
    assert!(!payload.primary_item_metadata().unwrap().still_picture);
    let decoded = decode(bytes).unwrap();
    assert_eq!(decoded.dimensions(), (32, 18));
    assert_eq!(decoded.sample_type(), SampleType::U16);
    assert_eq!(decoded.meta.cicp, Some((2, 2))); // FFmpeg leaves primaries/transfer unspecified.
    let values = decoded.to_normalized();
    for y in 0..18 {
        for x in 0..32 {
            let expected = [x as f32 * 1901.0 + 73.0, y as f32 * 3511.0 + 107.0, 23117.0];
            for (channel, value) in expected.into_iter().enumerate() {
                assert!((values[(y * 32 + x) * 3 + channel] - value / 65535.0).abs() < 0.01);
            }
        }
    }
}

#[test]
fn unsupported_colour_and_transforms_are_explicit_errors() {
    let bytes = encode(&image(SampleType::U8, false), Format::Avif, &Default::default()).unwrap();
    let at = bytes.windows(4).position(|w| w == b"nclx").unwrap();
    for transfer in [16u16, 18] {
        let mut hdr = bytes.clone();
        hdr[at + 6..at + 8].copy_from_slice(&transfer.to_be_bytes());
        assert!(matches!(decode(&hdr), Err(CodecError::Unsupported { reason, .. }) if reason.contains("PQ/HLG")));
    }
    for (primaries, transfer) in [(1u16, 8u16), (12, 13), (9, 14)] {
        let mut known = bytes.clone();
        known[at + 4..at + 6].copy_from_slice(&primaries.to_be_bytes());
        known[at + 6..at + 8].copy_from_slice(&transfer.to_be_bytes());
        let decoded = decode(&known).unwrap();
        assert_eq!(decoded.meta.cicp, Some((primaries, transfer)));
        // The raw encoder cannot silently relabel these values as sRGB. Document I/O
        // supplies the corresponding RGB profile and preserves their colour meaning.
        assert!(matches!(encode(&decoded, Format::Avif, &Default::default()), Err(CodecError::Unsupported { .. })));
    }
    let mut unknown = bytes.clone();
    unknown[at..at + 4].copy_from_slice(b"junk");
    assert!(matches!(decode(&unknown), Err(CodecError::Unsupported { .. })));
    let mut transform = bytes;
    let at = transform.windows(4).position(|w| w == b"pixi").unwrap();
    transform[at..at + 4].copy_from_slice(b"irot");
    assert!(matches!(decode(&transform), Err(CodecError::Unsupported { reason, .. }) if reason.contains("rotation")));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires FFmpeg with AVIF demuxing and libdav1d; no production runtime dependency"]
fn independent_export_oracle() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("photocraft-avif-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for depth in [8, 10] {
        let mut img = image(SampleType::U16, true);
        img.icc = Some(b"opaque profile container test".to_vec());
        let bytes = encode(&img, Format::Avif, &EncodeOptions { avif_quality: 100, avif_depth: depth, ..Default::default() }).unwrap();
        let avif = dir.join(format!("rgba{depth}.avif"));
        std::fs::write(&avif, bytes).unwrap();
        for (stream, pixel_format, planes) in [("0:v:0", "gbrp16le", 3usize), ("0:v:1", "gray16le", 1)] {
            let raw = dir.join(format!("{depth}-{planes}.raw"));
            let result = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-threads", "1", "-i"])
                .arg(&avif)
                .args(["-map", stream, "-frames:v", "1", "-pix_fmt", pixel_format, "-f", "rawvideo"])
                .arg(&raw)
                .output()
                .unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            let data = std::fs::read(&raw).unwrap();
            let pixels = 31 * 17;
            assert_eq!(data.len(), pixels * planes * 2);
            let values = img.to_normalized();
            let mut max_error = 0i32;
            for (i, value) in data.as_chunks::<2>().0.iter().enumerate() {
                let channel = if planes == 1 { 3 } else { [1, 2, 0][i / pixels] };
                let expected = (values[(i % pixels) * 4 + channel] * 65535.0).round() as i32;
                let actual = i32::from(u16::from_le_bytes(*value));
                max_error = max_error.max((actual - expected).abs());
            }
            assert!(max_error <= 512, "{depth}-bit/{stream}: independent maximum error {max_error}");
            println!("{depth}-bit/{stream}: maximum U16 error {max_error}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "manual 24 MP release benchmark; allocates large image and decoder buffers"]
fn avif_24mp_release_timing() {
    let (width, height) = (6000u32, 4000u32);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.extend([(x * 255 / (width - 1)) as u8, (y * 255 / (height - 1)) as u8, 97]);
        }
    }
    let source = Image::from_u8(width, height, ChannelLayout::Rgb, pixels).unwrap();
    let start = std::time::Instant::now();
    let bytes = encode(&source, Format::Avif, &Default::default()).unwrap();
    let encode_ms = start.elapsed().as_millis();
    let start = std::time::Instant::now();
    let decoded = decode(&bytes).unwrap();
    let decode_ms = start.elapsed().as_millis();
    assert_eq!(decoded.dimensions(), (width, height));
    assert_eq!(decoded.layout(), ChannelLayout::Rgb);
    assert_eq!(decoded.sample_type(), SampleType::U8);
    println!("24 MP synthetic RGB8 gradient, quality 90/speed 8, scalar single-thread: encode {encode_ms} ms, decode {decode_ms} ms, {} bytes", bytes.len());
}
