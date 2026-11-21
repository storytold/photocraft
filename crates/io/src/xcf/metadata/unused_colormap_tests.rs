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

#[test]
fn legacy_short_palette_lengths_leave_rgb_and_gray_drawables_aligned() {
    for base in [0, 1] {
        let mut encoded = source(base, vec![0, 0, 0, 2, 17, 31, 57, 23, 41, 73]);
        let mut c = crate::xcf::Cursor { b: &encoded, pos: 30, wide: true };
        assert_eq!(c.u32().unwrap(), 17);
        let length = c.u32().unwrap() as usize;
        c.skip(length).unwrap();
        assert_eq!(c.u32().unwrap(), 1);
        let length_slot = c.pos;
        assert_eq!(c.u32().unwrap(), 10);
        encoded[length_slot..length_slot + 4].copy_from_slice(&6u32.to_be_bytes());
        let result = crate::import("legacy-palette.xcf", &encoded).unwrap();
        let expected = if base == 1 { vec![73. / 255., 128. / 255.] } else { vec![17. / 255., 31. / 255., 57. / 255., 128. / 255.] };
        assert_eq!(result.document.layers[0].surface().unwrap().pixel(0, 0), expected);
        assert_eq!(crate::xcf::source_geometry("legacy-palette.xcf", &encoded, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 72.)));
    }
}

#[test]
fn a_declared_unused_palette_boundary_can_precede_a_valid_dpi_record() {
    for base in [0, 1] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = 26;
        image.base_type = base;
        image.image_props.push((1, vec![0, 0, 0, 1, 57]));
        image.image_props.push((19, [240f32.to_be_bytes(); 2].concat()));
        let mut layer = TestLayer::rgba("Visible", 1, 1, if base == 0 { vec![17, 31, 57, 255] } else { vec![57, 255] });
        if base == 1 {
            layer.kind = 3;
        }
        image.layers.push(layer);
        let bytes = image.write();
        let imported = crate::import("short-palette.xcf", &bytes).unwrap();
        assert_eq!(imported.document.resolution_dpi, 240.);
        assert!(!imported.document.layers[0].surface().unwrap().content_bounds().is_empty());
        let wrapped = crabz2::compress(&bytes, crabz2::Level::default());
        for (name, data) in [("short-palette.xcf", bytes), ("short-palette.xcf.bz2", wrapped)] {
            assert_eq!(crate::xcf::source_geometry(name, &data, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 240.)));
        }
    }
}

#[test]
fn version_zero_uses_one_byte_per_index_and_the_historical_gray_ramp() {
    let mut image = TestImage::rgb8(2, 1);
    image.version = 0;
    image.base_type = 2;
    image.image_props.push((1, vec![0, 0, 0, 3, 87, 65, 43]));
    image.image_props.push((19, [96f32.to_be_bytes(); 2].concat()));
    let mut layer = TestLayer::rgba("Indexed", 2, 1, vec![1, 255, 2, 255]);
    layer.kind = 5;
    image.layers.push(layer);
    let bytes = image.write();
    let imported = crate::import("version-zero.xcf", &bytes).unwrap();
    assert_eq!(imported.document.resolution_dpi, 96.);
    let pixels = imported.document.layers[0].surface().unwrap();
    assert_eq!(pixels.pixel(0, 0), [1. / 255., 1. / 255., 1. / 255., 1.]);
    assert_eq!(pixels.pixel(1, 0), [2. / 255., 2. / 255., 2. / 255., 1.]);
    assert_eq!(crate::xcf::source_geometry("version-zero.xcf", &bytes, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((2., 1.), 96.)));
}

#[test]
fn overlapping_palette_end_markers_use_drawable_table_framing() {
    for version in [3, 11, 26] {
        for extra_color_bytes in [false, true] {
            let mut image = TestImage::rgb8(1, 1);
            image.version = version;
            let mut palette = vec![0, 0, 0, 1, 255];
            if extra_color_bytes {
                palette.extend([0, 0]);
            }
            image.image_props.push((1, palette.clone()));
            image.layers.push(TestLayer::rgba("Visible", 1, 1, vec![17, 31, 57, 255]));
            image.channels.push(("Coverage", vec![64], false));
            let mut bytes = image.write();
            let payload = if version < 4 { 26 } else { 30 } + 9 + 8;
            bytes[payload - 4..payload].copy_from_slice(&5u32.to_be_bytes());
            let result = crate::import("overlapping-end.xcf", &bytes).unwrap();
            assert_eq!(result.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
            assert_eq!(result.document.channels[0].surface.pixel(0, 0), [64. / 255.]);
            let Some(ImportConversion::Report(report)) = result.document.metadata.import_conversion.as_deref() else { panic!("report") };
            let retained = report.entries.iter().find(|entry| entry.feature_code == "xcf.source_property.1").unwrap();
            assert!(retained.attachments.iter().any(|&index| report.attachments[index as usize].bytes.as_slice() == palette));
        }
    }
}

#[test]
fn overlapping_non_end_headers_keep_the_real_resolution_and_drawable_tables() {
    for version in [3, 11, 26] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = version;
        // A two-color legacy length stops immediately before the embedded 19.
        // That makes the color tail and the actual DPI tag look like (19, 19).
        image.image_props.push((1, vec![0, 0, 0, 2, 255, 64, 0, 0, 0, 19]));
        image.image_props.push((19, [240f32.to_be_bytes(); 2].concat()));
        image.layers.push(TestLayer::rgba("Visible", 1, 1, vec![17, 31, 57, 255]));
        image.channels.push(("Coverage", vec![64], false));
        let mut source = image.write();
        let palette_payload = if version < 4 { 26 } else { 30 } + 9 + 8;
        source[palette_payload - 4..palette_payload].copy_from_slice(&6u32.to_be_bytes());
        let result = crate::import("competing-property.xcf", &source).unwrap();
        assert_eq!(result.document.resolution_dpi, 240.);
        assert_eq!(result.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
        assert_eq!(result.document.channels[0].surface.pixel(0, 0), [64. / 255.]);
        let wrapped = crabz2::compress(&source, crabz2::Level::default());
        for (name, bytes) in [("competing-property.xcf", source), ("competing-property.xcf.bz2", wrapped)] {
            assert_eq!(crate::xcf::source_geometry(name, &bytes, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 240.)));
        }
    }
}

#[test]
fn legacy_palette_lookahead_understands_versioned_custom_units() {
    use crate::xcf::tests::W;
    for version in [11, 20, 21, 26] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = version;
        image.image_props.push((1, vec![0, 0, 0, 2, 255, 64, 0, 0, 0, 17]));
        let mut unit = W { out: Vec::new(), wide: false };
        unit.u32(1f32.to_bits());
        unit.u32(2);
        for value in ["unit", "u", "unit-symbol"].into_iter().chain(["units", "unit singular"].into_iter().take(if version < 21 { 2 } else { 0 })) {
            unit.string(value);
        }
        image.image_props.push((24, unit.out));
        image.image_props.push((19, [240f32.to_be_bytes(); 2].concat()));
        image.layers.push(TestLayer::rgba("Visible", 1, 1, vec![17, 31, 57, 255]));
        let mut source = image.write();
        let palette_length = 30 + 9 + 4;
        source[palette_length..palette_length + 4].copy_from_slice(&6u32.to_be_bytes());
        let unit_length = 30 + 9 + 8 + 10 + 4;
        source[unit_length..unit_length + 4].copy_from_slice(&4u32.to_be_bytes());
        let imported = crate::import("palette-unit.xcf", &source).unwrap();
        assert_eq!(imported.document.resolution_dpi, 240.);
        assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
        let wrapped = crabz2::compress(&source, crabz2::Level::default());
        assert_eq!(crate::xcf::source_geometry("palette-unit.xcf.bz2", &wrapped, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 240.)));
    }
}

#[test]
fn legacy_palette_boundary_accepts_an_unknown_following_property() {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.image_props.push((1, vec![0, 0, 0, 2, 255, 64, 0, 0, 0, 17]));
    image.image_props.push((50_000, vec![1, 2, 3, 4]));
    image.image_props.push((19, [240f32.to_be_bytes(); 2].concat()));
    image.layers.push(TestLayer::rgba("Visible", 1, 1, vec![17, 31, 57, 255]));
    let mut source = image.write();
    let palette_length = 30 + 9 + 4;
    source[palette_length..palette_length + 4].copy_from_slice(&6u32.to_be_bytes());
    let document = crate::import("palette-unknown.xcf", &source).unwrap().document;
    assert_eq!(document.resolution_dpi, 240.);
    assert_eq!(document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
    let wrapped = crabz2::compress(&source, crabz2::Level::default());
    assert_eq!(crate::xcf::source_geometry("palette-unknown.xcf.bz2", &wrapped, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), 240.)));
}

#[test]
fn palette_lookahead_skips_large_properties_without_accepting_empty_tables_inside_them() {
    use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
    use photocraft_geom::Affine;
    use std::sync::Arc;

    for version in [3, 11, 26] {
        for legacy_length in [true, false] {
            let mut image = TestImage::rgb8(1, 1);
            image.version = version;
            let mut palette = vec![0, 0, 0, 2, 255, 64];
            if legacy_length {
                // The short boundary sees (19, 19) from the RGB tail and actual DPI tag.
                palette.extend([0, 0, 0, 19]);
            }
            image.image_props.push((1, palette));
            image.image_props.push((19, [240f32.to_be_bytes(); 2].concat()));
            image.image_props.push((50, vec![0; 64 * 1024]));
            image.layers.push(TestLayer::rgba("Visible", 1, 1, vec![17, 31, 57, 255]));
            image.channels.push(("Coverage", vec![64], false));
            let mut source = image.write();
            let palette_payload = if version < 4 { 26 } else { 30 } + 9 + 8;
            source[palette_payload - 4..palette_payload].copy_from_slice(&6u32.to_be_bytes());
            let result = crate::import("long-continuation.xcf", &source).unwrap();
            assert_eq!(result.document.resolution_dpi, 240.);
            assert_eq!(result.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
            assert_eq!(result.document.channels[0].surface.pixel(0, 0), [64. / 255.]);

            let name = "long-continuation.xcf.bz2";
            let wrapped = crabz2::compress(&source, crabz2::Level::default());
            let expected_dpi = f64::from(result.document.resolution_dpi);
            for (name, bytes) in [("long-continuation.xcf", source), (name, wrapped)] {
                assert_eq!(crate::xcf::source_geometry(name, &bytes, &photocraft_raster::Interrupt::NONE).unwrap(), Some(((1., 1.), expected_dpi)));
                let mut doc = Document::new("Placed XCF", result.document.size, result.document.mode, result.document.depth);
                let cache = result.document.layers[0].surface().unwrap().clone();
                let source = SmartSource::Embedded { file_name: name.into(), bytes: Arc::new(bytes) };
                doc.layers.push(Layer::new("Historical palette", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, Some(cache)))));
                let exported = crate::export(&doc, "placed.psd", &Default::default()).unwrap();
                let psd = craft_psd::PsdFile::from_bytes(&exported.bytes).unwrap();
                let layer = &psd.layer_info.unwrap().layers[0];
                let placed = crate::smart_map::parse_sold(&layer.block(b"SoLd").unwrap().data).unwrap();
                assert_eq!(crate::blocks::num(placed.descriptor.get("Rslt")), Some(expected_dpi), "{name}, XCF v{version}, legacy length {legacy_length}");
            }
        }
    }
}
