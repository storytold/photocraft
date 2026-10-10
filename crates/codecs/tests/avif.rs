//! AVIF is encode-only: inspect its container and fidelity contract here;
//! `examples/avif_fixtures.rs` supplies pixels for an independent decoder oracle.

mod common;
use common::*;
use photocraft_codecs::*;

#[test]
fn capabilities_and_feature_gate_are_honest() {
    assert_eq!(caps(Format::Avif).write, cfg!(feature = "avif"));
    assert!(!caps(Format::Avif).read);
    #[cfg(not(feature = "avif"))]
    {
        let image = test_image(ChannelLayout::Rgb, SampleType::U8);
        assert!(matches!(encode(&image, Format::Avif, &EncodeOptions::default()), Err(CodecError::Unsupported { .. })));
        assert!(fidelity_warnings(&image, Format::Avif).contains(&FidelityWarning::WriteUnsupported { format: Format::Avif }));
    }
}

#[cfg(feature = "avif")]
mod enabled {
    use super::*;

    // Walk box boundaries rather than searching inside the compressed AV1 payload.
    fn properties<'a>(mut bytes: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
        let mut found = Vec::new();
        while bytes.len() >= 8 {
            let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert!((8..=bytes.len()).contains(&len), "invalid generated BMFF box length");
            let tag = &bytes[4..8];
            let body = &bytes[8..len];
            if tag == kind {
                found.push(body);
            }
            let children = match tag {
                b"meta" => Some(&body[4..]), // FullBox version and flags.
                b"iprp" | b"ipco" => Some(body),
                _ => None,
            };
            if let Some(children) = children {
                found.extend(properties(children, kind));
            }
            bytes = &bytes[len..];
        }
        assert!(bytes.is_empty(), "trailing bytes outside generated BMFF boxes");
        found
    }

    fn check_dimensions(bytes: &[u8], width: u32, height: u32) {
        assert_eq!(detect(bytes), Some(Format::Avif));
        let configurations = properties(bytes, b"av1C");
        assert!(!configurations.is_empty());
        for configuration in configurations {
            assert!(configuration.len() >= 4);
            // AV1CodecConfigurationRecord high_bitdepth and twelve_bit flags.
            assert_eq!(configuration[2] & 0x60, 0, "AVIF must store 8-bit colour and alpha");
        }
        let dimensions = properties(bytes, b"ispe");
        assert!(!dimensions.is_empty());
        for property in dimensions {
            assert_eq!(property.len(), 12);
            assert_eq!(u32::from_be_bytes(property[4..8].try_into().unwrap()), width);
            assert_eq!(u32::from_be_bytes(property[8..12].try_into().unwrap()), height);
        }
    }

    #[test]
    fn rgb_and_rgba_keep_odd_tiny_dimensions_and_alpha_items() {
        for (width, height) in [(1, 1), (1, 17), (17, 1), (37, 23)] {
            for layout in [ChannelLayout::Rgb, ChannelLayout::Rgba] {
                let image = synth(width, height, layout, SampleType::U8, 5, 0.03);
                let bytes = encode(&image, Format::Avif, &EncodeOptions::default()).unwrap();
                check_dimensions(&bytes, width, height);
                assert_eq!(!properties(&bytes, b"auxC").is_empty(), layout.has_alpha());
                assert!(matches!(decode(&bytes), Err(CodecError::Unsupported { .. })), "AVIF import remains explicitly unsupported");
            }
        }
    }

    #[test]
    fn every_input_depth_and_layout_uses_the_declared_encode_plan() {
        for layout in ChannelLayout::ALL {
            for sample in SampleType::ALL {
                let image = synth(5, 3, layout, sample, 9, 0.0);
                let bytes = encode(&image, Format::Avif, &EncodeOptions::default()).unwrap();
                check_dimensions(&bytes, 5, 3);
                let warnings = fidelity_warnings(&image, Format::Avif);
                assert!(warnings.contains(&FidelityWarning::LossyCompression));
                assert_eq!(warnings.contains(&FidelityWarning::DepthReduced { from: sample, to: SampleType::U8 }), sample != SampleType::U8);
                assert!(!warnings.contains(&FidelityWarning::AlphaDiscarded));
            }
        }
    }

    #[test]
    fn quality_controls_size_and_clamps_to_the_supported_range() {
        let image = synth(97, 61, ChannelLayout::Rgb, SampleType::U8, 3, 0.08);
        let at = |quality| encode(&image, Format::Avif, &EncodeOptions { jpeg_quality: quality, ..Default::default() }).unwrap();
        let low = at(20);
        let high = at(95);
        assert!(low.len() < high.len(), "q20 {} vs q95 {}", low.len(), high.len());
        assert_eq!(at(0), at(1));
        assert_eq!(at(255), at(100));
    }

    #[test]
    fn unsupported_metadata_and_hdr_precision_are_reported() {
        let mut image = synth(5, 3, ChannelLayout::Rgba, SampleType::F32, 7, 0.0);
        image.icc = Some(sample_icc(128));
        image.meta = Metadata {
            exif: Some(sample_exif()),
            xmp: Some(SAMPLE_XMP.into()),
            dpi: Some((300.0, 300.0)),
            text: vec![("title".into(), "synthetic".into())],
            ..Default::default()
        };
        image.data_mut()[..4].copy_from_slice(&2.0f32.to_ne_bytes());
        let warnings = fidelity_warnings(&image, Format::Avif);
        for warning in [
            FidelityWarning::DepthReduced { from: SampleType::F32, to: SampleType::U8 },
            FidelityWarning::RangeClipped,
            FidelityWarning::LossyCompression,
            FidelityWarning::IccDropped,
            FidelityWarning::ExifDropped,
            FidelityWarning::XmpDropped,
            FidelityWarning::DpiDropped,
            FidelityWarning::TextDropped,
        ] {
            assert!(warnings.contains(&warning), "{warning:?}");
        }
        assert!(!warnings.contains(&FidelityWarning::AlphaDiscarded));
        assert!(encode(&image, Format::Avif, &EncodeOptions::default()).is_ok());
    }

    #[test]
    fn empty_and_oversize_images_return_errors() {
        let empty = Image::new(0, 1, ChannelLayout::Rgb, SampleType::U8).unwrap();
        assert!(matches!(encode(&empty, Format::Avif, &EncodeOptions::default()), Err(CodecError::InvalidImage(_))));
        let wide = Image::new(65536, 1, ChannelLayout::Rgb, SampleType::U8).unwrap();
        assert!(fidelity_warnings(&wide, Format::Avif).contains(&FidelityWarning::DimensionsExceeded { max_width: 65535, max_height: 65535 }));
        assert!(matches!(encode(&wide, Format::Avif, &EncodeOptions::default()), Err(CodecError::Encode { .. })));
    }
}
