use crate::xcf::tests::{TestImage, TestLayer};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};

fn palette() -> Vec<u8> {
    [1u32.to_be_bytes().as_slice(), &[20, 40, 60]].concat()
}

fn fallback_entries(document: &photocraft_doc::Document) -> Vec<&photocraft_doc::import_conversion::ConversionEntry> {
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("bounded report") };
    report.validate().unwrap();
    report.entries.iter().filter(|entry| entry.feature_code == "xcf.indexed_palette_fallback").collect()
}

#[test]
fn missing_indexed_entries_expand_to_index_gray_without_changing_valid_colors_or_alpha() {
    for version in [3, 26] {
        let mut image = TestImage::rgb8(3, 1);
        image.version = version;
        image.base_type = 2;
        image.image_props.push((1, palette()));
        let mut layer = TestLayer::rgba("Partial palette", 3, 1, vec![0, 255, 7, 128, 255, 0]);
        layer.kind = 5;
        image.layers.push(layer);
        let imported = crate::import("partial-palette.xcf", &image.write()).unwrap();
        let pixels = imported.document.layers[0].surface().unwrap();
        assert_eq!(pixels.pixel(0, 0), vec![20. / 255., 40. / 255., 60. / 255., 1.]);
        assert_eq!(pixels.pixel(1, 0), vec![7. / 255., 7. / 255., 7. / 255., 128. / 255.]);
        assert_eq!(pixels.pixel(2, 0), vec![1., 1., 1., 0.]);
        let entries = fallback_entries(&imported.document);
        assert_eq!(entries.len(), 1, "one owner-level disclosure rather than one per missing pixel");
        assert_eq!(entries[0].reason, ConversionReason::InvalidOptionalData);
        assert_eq!(entries[0].disposition, ConversionDisposition::NativeApproximation);
        assert_eq!(imported.warnings.iter().filter(|warning| warning.contains("palette") && warning.contains("gray")).count(), 1);
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), pixels.read_region(imported.document.bounds()));
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}

#[test]
fn absent_and_empty_palette_preserve_gray_recovery_but_valid_palette_does_not_warn() {
    for palette in [None, Some(0u32.to_be_bytes().to_vec())] {
        let mut image = TestImage::rgb8(2, 1);
        image.base_type = 2;
        if let Some(palette) = palette {
            image.image_props.push((1, palette));
        }
        let mut layer = TestLayer::rgba("Missing palette", 2, 1, vec![0, 255]);
        layer.kind = 4;
        image.layers.push(layer);
        let imported = crate::import("missing-palette.xcf", &image.write()).unwrap();
        let pixels = imported.document.layers[0].surface().unwrap();
        assert_eq!(pixels.pixel(0, 0), vec![0., 0., 0., 1.]);
        assert_eq!(pixels.pixel(1, 0), vec![1., 1., 1., 1.]);
        assert_eq!(fallback_entries(&imported.document).len(), 1);
    }
    let mut image = TestImage::rgb8(1, 1);
    image.base_type = 2;
    image.image_props.push((1, palette()));
    let mut layer = TestLayer::rgba("Complete palette", 1, 1, vec![0, 128]);
    layer.kind = 5;
    image.layers.push(layer);
    let imported = crate::import("valid-palette.xcf", &image.write()).unwrap();
    assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), vec![20. / 255., 40. / 255., 60. / 255., 128. / 255.]);
    assert!(fallback_entries(&imported.document).is_empty());
    assert!(!imported.warnings.iter().any(|warning| warning.contains("palette") && warning.contains("gray")));
}

#[test]
fn many_missing_indices_keep_one_bounded_report_entry_and_warning_per_owner() {
    let mut image = TestImage::rgb8(128, 128);
    image.base_type = 2;
    image.image_props.push((1, palette()));
    for name in ["Owner one", "Owner two", "Owner three", "Owner four", "Owner five", "Owner six"] {
        let mut layer = TestLayer::rgba(name, 128, 128, [7, 128].repeat(128 * 128));
        layer.kind = 5;
        image.layers.push(layer);
    }
    let imported = crate::import("many-indices.xcf", &image.write()).unwrap();
    assert_eq!(imported.document.layers.len(), 6);
    assert_eq!(fallback_entries(&imported.document).len(), 6);
    for layer in &imported.document.layers {
        assert_eq!(layer.surface().unwrap().pixel(127, 127), vec![7. / 255., 7. / 255., 7. / 255., 128. / 255.]);
    }
    let warnings: Vec<_> = imported.warnings.iter().filter(|warning| warning.contains("palette") && warning.contains("gray")).collect();
    assert_eq!(warnings.len(), 1, "the existing warning grouper bounds repeated owner labels");
    assert!(warnings[0].len() < 512);
}
