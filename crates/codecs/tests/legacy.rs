//! Baseline interchange, conversion and hostile-header coverage for the new exporters.
use photocraft_codecs::*;
const FORMATS: [Format; 11] = [
    Format::Pcx,
    Format::Sgi,
    Format::SunRaster,
    Format::Farbfeld,
    Format::Wbmp,
    Format::Xbm,
    Format::Xpm,
    Format::Cur,
    Format::Icns,
    Format::Gbr,
    Format::GimpPat,
];
fn sample(layout: ChannelLayout, sample: SampleType) -> Image {
    let values = (0..16 * 16 * layout.channels()).map(|i| ((i * 71 + 19) % 256) as f32 / 255.0).collect::<Vec<_>>();
    Image::from_normalized(16, 16, layout, sample, &values).unwrap()
}
#[test]
fn every_layout_and_depth_has_an_explicit_verified_conversion() {
    for f in FORMATS {
        for layout in ChannelLayout::ALL {
            for sample_type in SampleType::ALL {
                let src = sample(layout, sample_type);
                let bytes = encode(&src, f, &Default::default()).unwrap();
                assert_eq!(detect(&bytes), Some(f), "{f:?}");
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.dimensions(), src.dimensions());
                let expected = src.convert(decoded.layout(), decoded.sample_type());
                match f {
                    Format::Wbmp | Format::Xbm => assert!(expected.data().iter().zip(decoded.data()).all(|(a, b)| *b == if *a >= 128 { 255 } else { 0 })),
                    Format::Xpm => {
                        for (a, b) in expected.data().as_chunks::<4>().0.iter().zip(decoded.data().as_chunks::<4>().0) {
                            if a[3] < 128 {
                                assert_eq!(*b, [0, 0, 0, 0]);
                            } else {
                                assert_eq!(*b, [a[0], a[1], a[2], 255]);
                            }
                        }
                    }
                    _ => assert_eq!(expected.data(), decoded.data(), "{f:?}, {layout:?}, {sample_type:?}"),
                }
            }
        }
    }
}
#[test]
fn exported_samples_for_independent_readers() {
    let src = sample(ChannelLayout::Rgba, SampleType::U8);
    for f in FORMATS {
        let bytes = encode(&src, f, &Default::default()).unwrap();
        let decoded = decode(&bytes).unwrap();
        if let Ok(dir) = std::env::var("PHOTOCRAFT_EXPORT_ORACLE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            let path = std::path::Path::new(&dir).join(format!("sample.{}", f.extensions()[0]));
            std::fs::write(path, bytes).unwrap();
            let expected = encode(&decoded, Format::Png, &Default::default()).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(format!("expected-{}.png", f.extensions()[0])), expected).unwrap();
        }
    }
}
#[test]
fn truncations_and_small_allocation_budgets_never_panic() {
    let opts = DecodeOptions { limits: Limits { max_alloc: 32, ..Default::default() }, ..Default::default() };
    for f in FORMATS {
        let bytes = encode(&sample(ChannelLayout::Rgba, SampleType::U8), f, &Default::default()).unwrap();
        assert!(decode_as_with(f, &bytes, &opts).is_err(), "{f:?} respects memory budget");
        for n in 0..bytes.len() {
            let result = std::panic::catch_unwind(|| decode_as(f, &bytes[..n]));
            assert!(result.is_ok(), "{f:?}, prefix {n}");
        }
        for len in 0..160 {
            let bad = (0..len).map(|i| ((i * 113 + len * 7) % 256) as u8).collect::<Vec<_>>();
            assert!(std::panic::catch_unwind(|| decode_as(f, &bad)).is_ok(), "{f:?} hostile length {len}");
        }
    }
}
#[test]
fn odd_widths_preserve_row_padding_and_orientation() {
    for f in [Format::Pcx, Format::Sgi, Format::SunRaster, Format::Wbmp, Format::Xbm] {
        let data = (0..9 * 3 * 3).map(|i| if i < 27 { 0 } else { 255 }).collect();
        let src = Image::from_u8(9, 3, ChannelLayout::Rgb, data).unwrap();
        let back = decode(&encode(&src, f, &Default::default()).unwrap()).unwrap();
        assert_eq!(back.data(), src.convert(back.layout(), back.sample_type()).data(), "{f:?}");
    }
}
#[test]
fn format_constraints_and_transparency_losses_are_actionable() {
    let bad = Image::new(17, 16, ChannelLayout::Rgba, SampleType::U8).unwrap();
    assert!(encode(&bad, Format::Icns, &Default::default()).unwrap_err().to_string().contains("square"));
    let src = sample(ChannelLayout::Rgba, SampleType::U16);
    assert!(fidelity_warnings(&src, Format::Xpm).contains(&FidelityWarning::AlphaBinarized));
    assert!(fidelity_warnings(&src, Format::Pcx).contains(&FidelityWarning::AlphaDiscarded));
    assert!(fidelity_warnings(&src, Format::Wbmp).contains(&FidelityWarning::ColorToGray));
}
#[test]
fn independent_handwritten_text_and_sgi_rle_fixtures() {
    let xbm = b"#define tiny_width 3\n#define tiny_height 1\nstatic unsigned char tiny_bits[] = { 0x05 };";
    assert_eq!(decode(xbm).unwrap().data(), [0, 255, 0]);
    let xpm = b"/* XPM */\nstatic char *tiny[]={\"3 1 3 1\",\"a c #f00\",\"b c None\",\"c c #0000ffff0000\",\"abc\"};";
    assert_eq!(decode(xpm).unwrap().data(), [255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 0, 255]);
    let src = Image::from_u8(3, 1, ChannelLayout::Gray, vec![7, 7, 7]).unwrap();
    let mut bytes = encode(&src, Format::Sgi, &Default::default()).unwrap();
    bytes[2] = 1;
    bytes.truncate(512);
    bytes.extend_from_slice(&520u32.to_be_bytes());
    bytes.extend_from_slice(&3u32.to_be_bytes());
    bytes.extend_from_slice(&[3, 7, 0]);
    assert_eq!(decode(&bytes).unwrap().data(), [7, 7, 7]);
    bytes[520] = 4;
    assert!(decode(&bytes).is_err());
}

#[test]
fn metadata_presence_agrees_with_every_new_capability_and_embed_option() {
    let mut image = sample(ChannelLayout::Rgba, SampleType::U8);
    image.icc = Some(photocraft_profile_fixture());
    image.meta.xmp = Some("<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".into());
    image.meta.text.push(("Description".into(), "synthetic fixture".into()));
    image.meta.dpi = Some((144.0, 144.0));
    for format in FORMATS {
        for embed in [true, false] {
            let options = EncodeOptions { embed_icc: embed, embed_metadata: embed, ..Default::default() };
            let back = decode(&encode(&image, format, &options).unwrap()).unwrap();
            let caps = format.caps();
            assert_eq!(back.icc.is_some(), caps.icc && embed, "{format:?}");
            assert_eq!(back.meta.xmp.is_some(), caps.xmp && embed, "{format:?}");
            assert_eq!(!back.meta.text.is_empty(), caps.text && embed, "{format:?}");
            assert_eq!(back.meta.dpi.is_some(), caps.dpi && embed, "{format:?}");
        }
    }
}
fn photocraft_profile_fixture() -> Vec<u8> {
    // PNG transports profile bytes without imposing a CMS dependency on the standalone crate.
    vec![1, 2, 3, 4]
}
