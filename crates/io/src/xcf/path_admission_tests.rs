use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

fn image(bytes: &[u8]) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: MAX_PIXEL_BYTES,
        native_source_budget: MAX_PIXEL_BYTES,
        meta_budget: MAX_METADATA_BYTES,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: 72.,
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}

fn payload(knots: u32) -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    w.end();
    w.u32(1);
    w.u32(u32::from(knots != 0));
    if knots != 0 {
        for value in [1, 0, 2, knots * 3] {
            w.u32(value);
        }
        for _ in 0..knots {
            for tag in [1, 0, 1] {
                w.u32(tag);
                w.u32(1f32.to_bits());
                w.u32(2f32.to_bits());
            }
        }
    }
    w.out
}

fn standalone(name: &str, knots: u32) -> Vec<u8> {
    let mut w = W { out: vec![0; 4], wide: true };
    w.string(name);
    let data = payload(knots);
    w.u32(data.len() as u32);
    w.out.extend(data);
    w.out
}

fn read_with_allowance(bytes: &[u8], count: usize, allowance: usize) -> Result<Document> {
    let mut document = Document::new("Paths", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    import_paths_with_allowance(&mut document, &mut image(bytes), &[], &vec![4; count], &Interrupt::NONE, allowance)?;
    Ok(document)
}

#[test]
fn repeated_standalone_path_names_debit_the_shared_allowance() {
    // A 1 KiB source is sufficient: repeated pointers allocate distinct native names.
    let name = "n".repeat(1024);
    let bytes = standalone(&name, 0);
    assert!(read_with_allowance(&bytes, 8, 4096).is_err());
    let document = read_with_allowance(&bytes, 8, 16 * 1024).unwrap();
    assert_eq!(document.paths.len(), 8);
    assert!(document.paths.iter().all(|path| path.name == name));
}

#[test]
fn native_path_geometry_debits_the_shared_allowance() {
    let bytes = standalone("", 16);
    assert!(payload(16).len() < 1536);
    assert!(read_with_allowance(&bytes, 1, 1536).is_err());
    let document = read_with_allowance(&bytes, 1, 8192).unwrap();
    assert_eq!(document.paths[0].path.subpaths[0].knots.len(), 16);
}

#[test]
fn empty_path_records_debit_storage_and_keep_the_count_limit() {
    let bytes = standalone("", 0);
    assert_eq!(payload(0).len(), 16);
    assert!(read_with_allowance(&bytes, 8, 8 * 16).is_err());
    let error = read_with_allowance(&bytes, MAX_CHANNELS + 1, MAX_METADATA_BYTES).unwrap_err();
    assert!(error.to_string().contains("1,024"));
}

fn raster_with_path_tail(tail: &[u8]) -> Vec<u8> {
    let mut w = W::new(26);
    for value in [1, 1, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    let path = w.slot();
    w.ptr(0);
    let at = w.out.len();
    w.patch(layer, at);
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string("Usable pixels");
    w.end();
    let raster = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(raster, at);
    let at = w.out.len();
    w.patch(path, at);
    w.out.extend_from_slice(tail);
    w.out
}

#[test]
fn incomplete_optional_path_framing_preserves_saved_pixels_without_raw_attachment() {
    let mut tails = vec![64u32.to_be_bytes().to_vec()]; // Truncated name.
    let mut name = W { out: Vec::new(), wide: true };
    name.string("Broken path");
    tails.push(name.out.clone()); // Missing payload length.
    for length in [4096, u32::MAX] {
        let mut tail = name.out.clone();
        tail.extend_from_slice(&length.to_be_bytes());
        tail.extend_from_slice(&[0; 8]);
        tails.push(tail); // Declared payload extends past EOF, even beyond the budget.
    }
    for tail in tails {
        let result = import("optional-path.xcf", &raster_with_path_tail(&tail), &Interrupt::NONE).unwrap();
        let LayerContent::Raster(pixels) = &result.document.layers[0].content else { panic!("saved raster") };
        assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        assert!(result.document.paths.is_empty());
        assert!(result.warnings.iter().any(|warning| warning.contains("Optional saved paths")));
        let Some(ImportConversion::Report(report)) = result.document.metadata.import_conversion.as_deref() else { panic!("conversion report") };
        let paths: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.path").collect();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].reason, ConversionReason::InvalidOptionalData);
        assert_eq!(paths[0].disposition, ConversionDisposition::RetainedUnapplied);
        assert!(paths[0].targets.is_empty());
        assert!(paths[0].attachments.is_empty());
        let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.import_conversion, result.document.metadata.import_conversion);
        let LayerContent::Raster(pixels) = &reopened.layers[0].content else { panic!("reopened raster") };
        assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
}
