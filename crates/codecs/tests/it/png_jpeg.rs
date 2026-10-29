//! Format-specific behaviour: Adam7 PNG, JPEG colour models, oracle
//! cross-checks against the `image` crate, and the optional PngSuite corpus.

use crate::common;
use common::*;
use photocraft_codecs::*;

// ---------------------------------------------------------------------------
// PNG Adam7
// ---------------------------------------------------------------------------

fn interlaced(img: &Image) -> Vec<u8> {
    encode(img, Format::Png, &EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap()
}

fn check_adam7(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) {
    let img = synth(w, h, layout, sample, (w * 31 + h) as u64, 0.5);
    let a = interlaced(&img);
    let b = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    assert_eq!(a[28], 1, "IHDR interlace flag");
    assert_eq!(b[28], 0);
    let da = decode(&a).unwrap();
    let db = decode(&b).unwrap();
    assert_eq!(da, db, "{w}x{h} {layout:?} {sample:?}: interlaced != progressive");
    assert_eq!(da.data(), img.data());
    // Oracle: the image crate agrees.
    let o = image::load_from_memory_with_format(&a, image::ImageFormat::Png).unwrap();
    let ours = da.convert(ChannelLayout::Rgba, SampleType::U16).to_u16_samples().unwrap();
    assert_eq!(o.into_rgba16().into_raw(), ours);
}

#[test]
fn adam7_small_sizes_all_layouts() {
    for (w, h) in [(1, 1), (2, 2), (3, 3), (5, 1), (1, 9), (7, 5), (8, 8), (9, 17), (33, 31)] {
        for l in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            check_adam7(w, h, l, SampleType::U8);
        }
    }
}

#[test]
fn adam7_16bit() {
    for (w, h) in [(1, 1), (4, 3), (13, 11), (64, 40)] {
        for l in [ChannelLayout::Gray, ChannelLayout::Rgba] {
            check_adam7(w, h, l, SampleType::U16);
        }
    }
}

#[test]
fn adam7_with_metadata() {
    let mut img = test_image(ChannelLayout::Rgb, SampleType::U8).with_icc(Some(sample_icc(400)));
    img.meta.text.push(("Comment".into(), "interlaced".into()));
    let back = decode(&interlaced(&img)).unwrap();
    assert_eq!(back.icc, img.icc);
    assert_eq!(back.meta.text, img.meta.text);
}

#[test]
fn adam7_compressions() {
    let img = test_image(ChannelLayout::Rgba, SampleType::U8);
    for c in [PngCompression::None, PngCompression::Fast, PngCompression::Best] {
        let b = encode(&img, Format::Png, &EncodeOptions { png_interlaced: true, png_compression: c, ..Default::default() }).unwrap();
        assert_eq!(decode(&b).unwrap().data(), img.data());
    }
}

#[test]
fn png_palette_and_low_bit_depth_expand() {
    // Build a 2-bit gray PNG and a palette PNG with tRNS using the png crate.
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 4, 1);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Two);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0b00_01_10_11]).unwrap();
    }
    let img = decode(&out).unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::Gray, &[0u8, 85, 170, 255][..]));

    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 2, 1);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(vec![255, 0, 0, 0, 0, 255]);
        e.set_trns(vec![128]);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0, 1]).unwrap();
    }
    let img = decode(&out).unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgba, &[255u8, 0, 0, 128, 0, 0, 255, 255][..]));
}

// ---------------------------------------------------------------------------
// JPEG colour models
// ---------------------------------------------------------------------------

#[test]
fn jpeg_grayscale_decodes_as_gray() {
    let img = synth(40, 30, ChannelLayout::Gray, SampleType::U8, 2, 0.02);
    let back = decode(&encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap()).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Gray);
    assert!(psnr(&img, &back) > 35.0);
}

#[test]
fn jpeg_cmyk_roundtrip_with_adobe_marker() {
    let img = synth(40, 30, ChannelLayout::Cmyk, SampleType::U8, 3, 0.02);
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap();
    assert!(bytes.windows(5).any(|w| w == b"Adobe"), "APP14 Adobe marker");
    let back = decode(&bytes).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Cmyk);
    assert!(psnr(&img, &back) > 35.0, "{}", psnr(&img, &back));
}

#[test]
fn jpeg_ycck_decodes_to_cmyk() {
    let img = synth(40, 32, ChannelLayout::Cmyk, SampleType::U8, 4, 0.02);
    let mut bytes = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut bytes, 95);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    enc.encode(img.data(), 40, 32, jpeg_encoder::ColorType::CmykAsYcck).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Cmyk);
    assert!(psnr(&img, &back) > 32.0, "{}", psnr(&img, &back));
}

#[test]
fn jpeg_cmyk_flat_colour_exact_ish() {
    let img = Image::from_u8(16, 16, ChannelLayout::Cmyk, [10u8, 100, 200, 50].repeat(256)).unwrap();
    let back = decode(&encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap()).unwrap();
    for px in back.data().chunks(4) {
        for (a, b) in px.iter().zip([10u8, 100, 200, 50]) {
            assert!((*a as i32 - b as i32).abs() <= 2);
        }
    }
}

#[test]
fn jpeg_rgb_matches_oracle_decoder() {
    let img = test_image(ChannelLayout::Rgb, SampleType::U8);
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let ours = decode(&bytes).unwrap();
    let theirs = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap().into_rgb8();
    let max = ours.data().iter().zip(theirs.as_raw()).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(max <= 1, "max diff {max}");
}

#[test]
fn jpeg_progressive_from_other_encoder_decodes() {
    let img = test_image(ChannelLayout::Rgb, SampleType::U8);
    let mut bytes = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut bytes, 90);
    enc.set_progressive(true);
    enc.encode(img.data(), img.width() as u16, img.height() as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let back = decode(&bytes).unwrap();
    assert!(psnr(&img, &back) > 25.0);
}

// ---------------------------------------------------------------------------
// Oracle: files we write are readable by the image crate
// ---------------------------------------------------------------------------

#[test]
fn png_output_matches_oracle_all_layouts() {
    for l in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for s in [SampleType::U8, SampleType::U16] {
            let img = test_image(l, s);
            let b = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
            let o = image::load_from_memory(&b).unwrap().into_rgba16().into_raw();
            assert_eq!(o, img.to_rgba16(), "{l:?} {s:?}");
        }
    }
}

#[test]
fn tiff_output_matches_oracle() {
    for l in [ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for s in [SampleType::U8, SampleType::U16] {
            let img = test_image(l, s);
            let b = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
            let o = image::load_from_memory(&b).unwrap().into_rgba16().into_raw();
            assert_eq!(o, img.to_rgba16(), "{l:?} {s:?}");
        }
    }
}

#[test]
fn tiff_f32_output_matches_oracle() {
    let img = test_image(ChannelLayout::Rgba, SampleType::F32);
    let b = encode(&img, Format::Tiff, &EncodeOptions { tiff_compression: TiffCompression::None, ..Default::default() }).unwrap();
    let o = image::load_from_memory(&b).unwrap().into_rgba32f().into_raw();
    assert_eq!(o, img.to_f32_samples().unwrap());
}

// ---------------------------------------------------------------------------
// PNG decoded in steps (PngRows)
// ---------------------------------------------------------------------------

/// Decodes `bytes` with [`PngRows`], going on for `per_call` rows per call (0: unlimited);
/// returns the image and how many calls it took.
fn decode_in_steps(bytes: &[u8], per_call: usize) -> Result<(Image, usize), CodecError> {
    let mut rows = PngRows::new(bytes, &Limits::default())?;
    let mut calls = 0;
    loop {
        calls += 1;
        let mut left = per_call;
        let mut more = || {
            left = left.wrapping_sub(1);
            per_call == 0 || left > 0
        };
        if let Some(img) = rows.step(&mut more)? {
            return Ok((img, calls));
        }
        assert!(calls < 100_000, "the decoder never finished");
    }
}

fn assert_steps_equal_decode(bytes: &[u8], what: &str) {
    let whole = decode(bytes);
    for per_call in [1, 7, 0] {
        // `decode` also takes the resolution from EXIF or XMP and turns the image upright;
        // `finish_png_decode` does that for a stepped decode, as the stepped import does.
        let stepped = decode_in_steps(bytes, per_call).and_then(|(img, calls)| Ok((finish_png_decode(img, &DecodeOptions::default())?, calls)));
        match (&whole, stepped) {
            (Ok(a), Ok((b, _))) => assert_eq!(a, &b, "{what}, {per_call} rows per call"),
            (Err(_), Err(_)) => {}
            (a, b) => panic!("{what}, {per_call} rows per call: decode {:?}, steps {:?}", a.as_ref().map(|_| ()), b.map(|_| ())),
        }
    }
}

#[test]
fn png_rows_equal_decode_in_every_layout_and_depth() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            let mut img = synth(41, 29, layout, sample, 77, 0.3).with_icc(Some(sample_icc(400)));
            img.meta.text.push(("Comment".into(), "stepped".into()));
            img.meta.dpi = Some((300.0, 300.0));
            for interlaced in [false, true] {
                let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
                assert_steps_equal_decode(&bytes, &format!("{layout:?} {sample:?} interlaced={interlaced}"));
            }
        }
    }
}

#[test]
fn png_rows_equal_decode_for_palette_and_transparency() {
    let pal = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [9, 9, 9]];
    let idx: Vec<u8> = (0..50 * 20).map(|i| (i % 4) as u8).collect();
    let bytes = encode_png_indexed(50, 20, &idx, &pal, Some(3), None, None).unwrap();
    assert_steps_equal_decode(&bytes, "palette with a transparent entry");
}

#[test]
fn png_rows_take_one_call_per_row_and_report_progress() {
    let bytes = encode(&synth(16, 40, ChannelLayout::Rgb, SampleType::U8, 5, 0.2), Format::Png, &EncodeOptions::default()).unwrap();
    let mut rows = PngRows::new(bytes.as_slice(), &Limits::default()).unwrap();
    assert_eq!(rows.size(), (16, 40));
    assert_eq!(rows.progress(), 0.0);
    let mut seen = Vec::new();
    let img = loop {
        if let Some(img) = rows.step(&mut || false).unwrap() {
            break img;
        }
        seen.push(rows.progress());
    };
    assert_eq!(seen.len(), 39, "one call per row but the last, which also finishes the file");
    assert!(seen.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(rows.progress(), 1.0);
    assert_eq!(img, decode(&bytes).unwrap());
    // A decoder that has finished says so instead of decoding again.
    assert!(rows.step(&mut || true).is_err());
}

#[test]
fn png_rows_share_their_bytes_without_copying_them() {
    let bytes = std::sync::Arc::new(encode(&synth(20, 20, ChannelLayout::Rgba, SampleType::U8, 3, 0.2), Format::Png, &EncodeOptions::default()).unwrap());
    let mut rows = PngRows::new(SharedBytes(bytes.clone()), &Limits::default()).unwrap();
    let img = rows.step(&mut || true).unwrap().unwrap();
    assert_eq!(img, decode(&bytes).unwrap());
    drop(rows);
    assert_eq!(std::sync::Arc::strong_count(&bytes), 1, "the decoder let go of the bytes");
}

#[test]
fn png_rows_fail_like_decode_on_damaged_files() {
    let img = synth(64, 64, ChannelLayout::Rgb, SampleType::U8, 11, 1.0);
    for interlaced in [false, true] {
        let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
        for cut in [8, 20, 40, bytes.len() / 3, bytes.len() / 2, bytes.len() - 20] {
            assert_steps_equal_decode(bytes.get(..cut).unwrap(), &format!("cut at {cut} interlaced={interlaced}"));
        }
        // A flipped byte in the middle of the compressed data.
        let mut bad = bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xFF;
        assert_steps_equal_decode(&bad, "a damaged byte");
    }
    assert!(decode_in_steps(&[], 1).is_err());
    assert!(decode_in_steps(b"not a png at all, just some text bytes", 1).is_err());
}

// ---------------------------------------------------------------------------
// PngSuite corpus (repo-root corpus/pngsuite/*.png; feature `corpus`, fetched by
// `cargo xtask corpus --all`; missing = failure)
// ---------------------------------------------------------------------------

#[cfg(feature = "corpus")]
#[test]
fn pngsuite_corpus_matches_oracle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("{} is missing: run `cargo xtask corpus --all`", dir.display());
    };
    let mut checked = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        let oracle = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png);
        let ours = decode(&bytes);
        match (ours, oracle) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.to_rgba16(), b.into_rgba16().into_raw(), "{}", p.display());
                checked += 1;
            }
            (Err(e), Ok(_)) => panic!("{}: we failed but oracle decoded: {e}", p.display()),
            _ => {}
        }
    }
    eprintln!("pngsuite: {checked} files compared");
    assert!(checked > 100, "{}: only {checked} PNGs compared: run `cargo xtask corpus --all`", dir.display());
}

#[cfg(feature = "corpus")]
#[test]
fn png_rows_equal_decode_on_the_pngsuite_corpus() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("{} is missing: run `cargo xtask corpus --all`", dir.display());
    };
    let mut checked = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        assert_steps_equal_decode(&bytes, &p.display().to_string());
        checked += 1;
    }
    assert!(checked > 100, "{}: only {checked} PNGs compared: run `cargo xtask corpus --all`", dir.display());
}
