//! Independently specified BE wire regressions, not historical GIMP-authored fixtures.
//!
//! Precision tables, pointer widths and tile layouts follow the public XCF document:
//! https://developer.gimp.org/core/standards/xcf/. In particular these tests do not
//! resolve the explicitly unspecified sample endianness of development XCF7–11.

use super::tests::{TestImage, TestLayer};
use super::*;
use photocraft_cms::{Builtin, Profile};

#[derive(Clone, Copy, Debug)]
enum Encoding {
    Byte,
    Word,
    Half,
    Float,
    Double,
}

impl Encoding {
    fn sample(self, index: usize, coverage: bool) -> (Vec<u8>, f32) {
        let index = index % 3;
        match self {
            Self::Byte => {
                let value = [0u8, 127, 255][index];
                (vec![value], f32::from(value) / 255.0)
            }
            Self::Word => {
                let value = [0u16, 0x1234, u16::MAX][index];
                (value.to_be_bytes().to_vec(), (f64::from(value) / 65535.0) as f32)
            }
            Self::Half => {
                // IEEE binary16 literals, with normalized alpha independently encoded.
                let (bits, value) =
                    if coverage { [(0u16, 0.0), (0x3800, 0.5), (0x3c00, 1.0)][index] } else { [(0xb400u16, -0.25), (0x3800, 0.5), (0x3e00, 1.5)][index] };
                (bits.to_be_bytes().to_vec(), value)
            }
            Self::Float | Self::Double => {
                let value = if coverage { [0.0f32, 0.5, 1.0][index] } else { [-0.25f32, 0.5, 1.5][index] };
                let bytes = if matches!(self, Self::Float) { value.to_be_bytes().to_vec() } else { f64::from(value).to_be_bytes().to_vec() };
                (bytes, value)
            }
        }
    }

    fn depth(self) -> SampleType {
        match self {
            Self::Byte => SampleType::U8,
            Self::Word => SampleType::U16,
            Self::Half | Self::Float | Self::Double => SampleType::F32,
        }
    }
}

#[test]
fn historical_be_tiles_keep_partial_edges_alpha_and_native_storage() {
    // The same precision number has different meanings in the development tables.
    // 65×65 crosses both tile edges; v10/v11 cross the pointer-width boundary.
    for (version, precision, encoding, gray) in [
        (4, 1, Encoding::Word, false),
        (4, 3, Encoding::Half, true),
        (5, 450, Encoding::Half, false),
        (6, 550, Encoding::Float, true),
        (8, 150, Encoding::Byte, false),
        (9, 150, Encoding::Byte, true),
        (10, 150, Encoding::Byte, false),
        (11, 150, Encoding::Byte, true),
        (12, 650, Encoding::Float, false),
        (13, 750, Encoding::Double, true),
    ] {
        for compression in [Compression::None, Compression::Rle, Compression::Zlib] {
            let (width, height) = (65, 65);
            let channels = if gray { 2 } else { 4 };
            let mut bytes = Vec::new();
            let mut expected = Vec::new();
            let mut coverage_bytes = Vec::new();
            let mut coverage = Vec::new();
            for pixel in 0..width as usize * height as usize {
                for channel in 0..channels {
                    let (encoded, value) = encoding.sample(pixel + channel, channel + 1 == channels);
                    bytes.extend(encoded);
                    expected.push(value);
                }
                let (encoded, value) = encoding.sample(pixel, true);
                coverage_bytes.extend(encoded);
                coverage.push(value);
            }
            let mut source = TestImage::rgb8(width, height);
            source.version = version;
            source.precision = precision;
            source.base_type = u32::from(gray);
            source.compression = compression;
            let mut layer = TestLayer::rgba("Historical partial tiles", width, height, bytes);
            layer.kind = if gray { 3 } else { 1 };
            layer.x = -2;
            layer.y = 1;
            layer.mask = Some(coverage_bytes.clone());
            source.layers.push(layer);
            source.channels.push(("Historical coverage", coverage_bytes.clone(), false));
            source.channels.push(("Historical selection", coverage_bytes, true));
            let imported = import("historical-be.xcf", &source.write(), &Interrupt::NONE).unwrap();
            let doc = imported.document;
            let rect = Rect::new(-2, 1, 63, 66);
            assert_eq!(doc.depth, encoding.depth(), "v{version}/{precision}");
            assert_eq!(doc.mode, if gray { ColorMode::Grayscale } else { ColorMode::Rgb });
            assert_eq!(doc.layers[0].surface().unwrap().read_region(rect), expected, "v{version}/{precision}/{compression:?}");
            assert_eq!(doc.layers[0].mask.as_ref().unwrap().surface.read_region(rect), coverage);
            assert_eq!(doc.channels[0].surface.read_region(doc.bounds()), coverage);
            assert_eq!(doc.selection.as_ref().unwrap().read_region(doc.bounds()), coverage);
            if compression == Compression::Zlib {
                let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&native).unwrap();
                assert_eq!(reopened.layers[0].surface(), doc.layers[0].surface());
                assert_eq!(reopened.layers[0].mask, doc.layers[0].mask);
                assert_eq!(reopened.channels, doc.channels);
                assert_eq!(reopened.selection, doc.selection);
                assert_eq!(reopened.icc_profile, doc.icc_profile);
                assert_eq!(reopened.depth, doc.depth);
            }
        }
    }
}

#[test]
fn historical_assigned_colorants_and_sample_encoding_survive_native_persistence() {
    // No sample conversion: only the transfer curve indicated by each historical
    // table changes. Native-authored profiles keep the expectation independent.
    for (version, precision, encoding, gray, linear) in [
        (4, 3, Encoding::Half, false, true),
        (5, 450, Encoding::Half, false, false),
        (6, 500, Encoding::Float, true, true),
        (12, 675, Encoding::Float, false, false),
    ] {
        let builtin = if gray { Builtin::GrayGamma22 } else { Builtin::DisplayP3 };
        let assigned = builtin.profile().to_bytes();
        let mut source = TestImage::rgb8(1, 1);
        source.version = version;
        source.precision = precision;
        source.base_type = u32::from(gray);
        let mut parasite = Vec::new();
        parasite.extend_from_slice(&12u32.to_be_bytes());
        parasite.extend_from_slice(b"icc-profile\0");
        parasite.extend_from_slice(&0u32.to_be_bytes());
        parasite.extend_from_slice(&(assigned.len() as u32).to_be_bytes());
        parasite.extend_from_slice(&assigned);
        source.image_props.push((21, parasite));
        let channels = if gray { 2 } else { 4 };
        let mut bytes = Vec::new();
        let mut expected = Vec::new();
        for channel in 0..channels {
            let (encoded, value) = encoding.sample(channel, channel + 1 == channels);
            bytes.extend(encoded);
            expected.push(value);
        }
        let mut layer = TestLayer::rgba("Historical assigned profile", 1, 1, bytes);
        layer.kind = if gray { 3 } else { 1 };
        source.layers.push(layer);
        let imported = import("historical-profile.xcf", &source.write(), &Interrupt::NONE).unwrap().document;
        assert_eq!(imported.layers[0].surface().unwrap().read_region(imported.bounds()), expected);
        let profile = Profile::parse(imported.icc_profile.as_ref().unwrap()).unwrap();
        assert_eq!(profile.matrix, builtin.profile().matrix);
        assert_eq!(profile.white_point, builtin.profile().white_point);
        let curve = if gray { profile.gray_trc.as_ref().unwrap() } else { &profile.trc.as_ref().unwrap()[0] };
        let expected_curve = if linear {
            0.5
        } else if precision == 675 {
            0.21404114
        } else {
            builtin.profile().trc.as_ref().unwrap()[0].eval64(0.5)
        };
        assert!((curve.eval64(0.5) - expected_curve).abs() < 0.0001);
        if !linear && precision != 675 {
            assert_eq!(imported.icc_profile.as_ref().unwrap(), &assigned);
        }
        let native = photocraft_format::save_to_bytes(&imported, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.icc_profile, imported.icc_profile);
        assert_eq!(reopened.layers[0].surface(), imported.layers[0].surface());
    }
}

#[test]
fn ambiguous_development_high_depth_requires_choice_then_keeps_durable_explicit_policy() {
    use photocraft_doc::import_conversion::ImportConversion;

    for (version, precision, encoding, ambiguous) in [
        (7, 600, Encoding::Float, true),
        (11, 600, Encoding::Float, true),
        (6, 500, Encoding::Float, false),
        (12, 600, Encoding::Float, false),
        (7, 150, Encoding::Byte, false),
        (11, 150, Encoding::Byte, false),
    ] {
        let mut source = TestImage::rgb8(1, 1);
        source.version = version;
        source.precision = precision;
        source.base_type = 1;
        let (mut bytes, gray) = encoding.sample(1, false);
        let (alpha_bytes, alpha) = encoding.sample(2, true);
        bytes.extend(alpha_bytes);
        let mut layer = TestLayer::rgba("Documented BE samples", 1, 1, bytes);
        layer.kind = 3;
        source.layers.push(layer);
        let bytes = source.write();
        if ambiguous {
            assert!(matches!(import("historical-order.xcf", &bytes, &Interrupt::NONE), Err(IoError::HistoricalXcfSampleOrderRequired(_))));
        }
        let imported = import_with_sample_order("historical-order.xcf", &bytes, Some(crate::HistoricalXcfSampleOrder::BigEndian), &Interrupt::NONE).unwrap();
        assert_eq!(imported.document.layers[0].surface().unwrap().read_region(imported.document.bounds()), [gray, alpha]);
        let notices: Vec<_> = imported.warnings.iter().filter(|warning| warning.contains("explicit big-endian choice")).collect();
        assert_eq!(notices.len(), usize::from(ambiguous), "version{version}/precision{precision}");
        if ambiguous {
            assert!(notices[0].contains("big-endian"));
        }
        let Some(ImportConversion::Report(report)) = &imported.document.metadata.import_conversion else { panic!("conversion report") };
        let entries: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.historical_sample_order").collect();
        assert_eq!(entries.len(), usize::from(ambiguous));
        if ambiguous {
            let entry = entries[0];
            assert_eq!(entry.reason, ConversionReason::NativePolicy);
            assert_eq!(entry.disposition, ConversionDisposition::NativeApproximation);
            assert!(entry.source_description.contains("no reliable sample byte-order marker"));
            assert!(entry.native_description.contains("big-endian"));
            assert!(entry.native_description.contains("selected before decoding"));
            assert!(entry.attachments.is_empty(), "byte-order uncertainty must not become executable source data");
        }
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        assert_eq!(reopened.layers[0].surface(), imported.document.layers[0].surface());
    }
}
