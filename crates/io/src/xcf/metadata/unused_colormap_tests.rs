use super::super::tests::{TestImage, TestLayer};
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};

fn source(base: u32, palette: Vec<u8>) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.base_type = base;
    image.image_props.push((1, palette));
    let mut layer = TestLayer::rgba(
        "Independent pixels",
        1,
        1,
        if base == 1 {
            vec![73, 128]
        } else if base == 2 {
            vec![0, 128]
        } else {
            vec![17, 31, 57, 128]
        },
    );
    layer.kind = match base {
        1 => 3,
        2 => 5,
        _ => 1,
    };
    image.layers.push(layer);
    image.write()
}

#[test]
fn malformed_unused_colormaps_keep_rgb_and_gray_pixels_and_inert_source() {
    for base in [0, 1] {
        for payload in [vec![], vec![0, 0, 1, 1], vec![0, 0, 0, 1, 10, 20], vec![0, 0, 0, 0, 99], u32::MAX.to_be_bytes().to_vec()] {
            let bytes = source(base, payload.clone());
            assert_eq!(crate::xcf::source_geometry("unused-palette.xcf", &bytes, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 72.)));
            let imported = crate::import("unused-palette.xcf", &bytes).unwrap();
            let document = &imported.document;
            let expected = if base == 1 { vec![73. / 255., 128. / 255.] } else { vec![17. / 255., 31. / 255., 57. / 255., 128. / 255.] };
            assert_eq!(document.layers[0].surface().unwrap().pixel(0, 0), expected);
            let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("report") };
            let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.source_property.1").unwrap();
            assert_eq!(entry.reason, ConversionReason::InvalidOptionalData);
            assert!(entry.attachments.iter().any(|index| report.attachments[*index as usize].bytes.as_slice() == payload));
            let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.layers, document.layers);
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        }
    }
}

#[test]
fn consumed_indexed_palettes_remain_strict_and_valid_palette_alpha_is_preserved() {
    for payload in [vec![], vec![0, 0, 1, 1], vec![0, 0, 0, 1, 10, 20], u32::MAX.to_be_bytes().to_vec()] {
        assert!(crate::import("indexed-palette.xcf", &source(2, payload)).is_err());
    }
    let imported = crate::import("indexed-palette.xcf", &source(2, vec![0, 0, 0, 1, 17, 31, 57])).unwrap();
    assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 128. / 255.]);
}

#[test]
fn unused_colormap_framing_does_not_hide_following_resolution_or_cancellation() {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.image_props.push((1, vec![255])); // No count at all: never borrow the next property as its count.
    image.image_props.push((19, [300f32.to_be_bytes(), 300f32.to_be_bytes()].concat()));
    image.layers.push(TestLayer::rgba("Pixels", 1, 1, vec![17, 31, 57, 255]));
    let bytes = image.write();
    let imported = crate::import("unused-short-palette.xcf", &bytes).unwrap();
    assert_eq!(imported.document.resolution_dpi, 300.);
    let cancel = || true;
    assert!(matches!(
        crate::xcf::import("unused-short-palette.xcf", &bytes, &photocraft_raster::Interrupt::cancel_only(&cancel)),
        Err(crate::IoError::Cancelled)
    ));
}

#[test]
fn palette_ignoring_never_ignores_truncated_declared_property_framing() {
    let mut bytes = source(0, vec![]);
    // A standalone image-property prefix claiming eight palette bytes but holding
    // only four is structurally truncated, independently of the unused contents.
    bytes.truncate(30);
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&8u32.to_be_bytes());
    bytes.extend_from_slice(&[0; 4]);
    assert!(crate::import("truncated-property.xcf", &bytes).is_err());
    assert!(crate::xcf::source_geometry("truncated-property.xcf", &bytes, &photocraft_raster::Interrupt::NONE).is_err());
    let indexed = source(2, u32::MAX.to_be_bytes().to_vec());
    assert!(crate::xcf::source_geometry("indexed.xcf", &indexed, &photocraft_raster::Interrupt::NONE).is_err());
}
