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
    let mut image = image(bytes);
    image.meta_budget = allowance;
    import_paths(&mut document, &mut image, &[], &vec![4; count], None, &Interrupt::NONE)?;
    document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish()?));
    Ok(document)
}

fn has_retention_limit(document: &Document) -> bool {
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { return false };
    report.entries.iter().any(|entry| entry.feature_code == "xcf.path" && entry.reason == ConversionReason::RetentionLimit)
}

#[test]
fn repeated_standalone_path_names_debit_the_shared_allowance() {
    // A 1 KiB source is sufficient: repeated pointers allocate distinct native names.
    let name = "n".repeat(1024);
    let bytes = standalone(&name, 0);
    let limited = read_with_allowance(&bytes, 8, 4096).unwrap();
    assert!(!limited.paths.is_empty() && limited.paths.len() < 8);
    assert!(has_retention_limit(&limited));
    let document = read_with_allowance(&bytes, 8, 16 * 1024).unwrap();
    assert_eq!(document.paths.len(), 8);
    assert!(document.paths.iter().all(|path| path.name == name));
}

#[test]
fn native_path_geometry_debits_the_shared_allowance() {
    let bytes = standalone("", 16);
    assert!(payload(16).len() < 900);
    let limited = read_with_allowance(&bytes, 1, 900).unwrap();
    assert!(limited.paths.is_empty());
    assert!(has_retention_limit(&limited));
    let document = read_with_allowance(&bytes, 1, 8192).unwrap();
    assert_eq!(document.paths[0].path.subpaths[0].knots.len(), 16);
}

#[test]
fn empty_path_records_debit_storage_and_keep_the_count_limit() {
    let bytes = standalone("", 0);
    assert_eq!(payload(0).len(), 16);
    let limited = read_with_allowance(&bytes, 8, 8 * 16).unwrap();
    assert!(limited.paths.len() < 8);
    assert!(has_retention_limit(&limited));
    let limited = read_with_allowance(&bytes, MAX_CHANNELS + 1, MAX_METADATA_BYTES).unwrap();
    assert_eq!(limited.paths.len(), MAX_CHANNELS);
    assert!(has_retention_limit(&limited));
}

#[test]
fn path_storage_uses_remaining_document_metadata_without_charging_borrowed_payload_twice() {
    let bytes = standalone("first", 0);
    let path = paths::standalone(&image(&bytes).c, 4).unwrap();
    let mut remaining = usize::MAX;
    admit_path(&mut remaining, &path).unwrap();
    let retained = usize::MAX - remaining;
    for count in [1, 2] {
        let mut image = image(&bytes);
        image.meta_budget = retained * count;
        let mut document = Document::new("Shared allowance", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        import_paths(&mut document, &mut image, &[], &[4, 4], None, &Interrupt::NONE).unwrap();
        assert_eq!(document.paths.len(), count);
        assert_eq!(image.meta_budget, 0, "native allocations debit the same allowance later metadata consumers see");
        document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish().unwrap()));
        assert_eq!(has_retention_limit(&document), count == 1);
    }
}

#[test]
fn path_limit_keeps_saved_pixels_and_earlier_paths_with_a_durable_disclosure() {
    let bytes = standalone("retained", 0);
    let mut image = image(&bytes);
    let decoded = paths::standalone(&image.c, 4).unwrap();
    let mut remaining = usize::MAX;
    admit_path(&mut remaining, &decoded).unwrap();
    image.meta_budget = usize::MAX - remaining;
    let mut document = Document::new("Saved pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Usable pixels", PixelFormat::RGBA8);
    let LayerContent::Raster(pixels) = &mut layer.content else { panic!("raster") };
    let expected = vec![17. / 255., 31. / 255., 57. / 255., 1.];
    pixels.write_region(document.bounds(), &expected);
    document.layers.push(layer);
    import_paths(&mut document, &mut image, &[], &[4, 4], None, &Interrupt::NONE).unwrap();
    assert_eq!(document.paths.len(), 1);
    assert!(image.warnings.finish().iter().any(|warning| warning.contains("admission limit")));
    document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish().unwrap()));
    assert!(has_retention_limit(&document));
    let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.paths, document.paths);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
}

#[test]
fn omitted_paths_do_not_guess_an_initial_selection_or_vector_tattoo_binding() {
    let mut payload = W { out: Vec::new(), wide: false };
    for value in [1, 0, 2] {
        payload.u32(value);
    }
    for name in ["first", "omitted"] {
        payload.string(name);
        for value in [7, 0, 0, 0, 0] {
            payload.u32(value);
        }
    }
    let properties = [Prop { kind: 25, offset: 0, data: &payload.out }];
    let decoded = paths::vectors(&payload.out).unwrap();
    let mut remaining = usize::MAX;
    admit_path(&mut remaining, &decoded[0]).unwrap();
    let mut image = image(&[]);
    image.meta_budget = usize::MAX - remaining;
    assert!(payload.out.len() <= image.meta_budget);
    let mut document = Document::new("Paths", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    import_paths(&mut document, &mut image, &properties, &[], None, &Interrupt::NONE).unwrap();
    assert_eq!(document.paths.len(), 1);
    assert_eq!(image.initial_view.selected_path, None);
    assert_eq!(image.path_tattoos.get(&7), Some(&None), "omitted geometry cannot make a duplicate tattoo look unique");
}

#[test]
fn cancelled_path_admission_still_cancels_instead_of_reporting_a_limit() {
    let bytes = standalone("", 0);
    let mut image = image(&bytes);
    image.meta_budget = 0;
    let mut document = Document::new("Paths", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let cancel = || true;
    assert!(matches!(import_paths(&mut document, &mut image, &[], &[4], None, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
}

#[test]
fn public_import_truncates_standalone_pointer_table_without_losing_paths_or_pixels() {
    let mut w = W::new(26);
    for value in [1, 1, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    let paths: Vec<_> = (0..MAX_CHANNELS + 1).map(|_| w.slot()).collect();
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
    for (name, slots) in [("Retained path", &paths[..MAX_CHANNELS]), ("Omitted path", &paths[MAX_CHANNELS..])] {
        let at = w.out.len();
        for &slot in slots {
            w.patch(slot, at);
        }
        w.string(name);
        let mut payload = W { out: Vec::new(), wide: false };
        payload.prop(43, &[]); // Selection must not survive the incomplete path inventory.
        payload.end();
        payload.u32(1);
        payload.u32(0);
        w.u32(payload.out.len() as u32);
        w.out.extend(payload.out);
    }
    let imported = import("many-paths.xcf", &w.out, &Interrupt::NONE).unwrap();
    let document = &imported.document;
    assert_eq!(document.paths.len(), MAX_CHANNELS);
    assert!(document.paths.iter().all(|path| path.name == "Retained path"));
    assert!(imported.initial_view.as_ref().is_none_or(|view| view.selected_path.is_none()));
    assert!(has_retention_limit(document));
    assert!(imported.warnings.iter().any(|warning| warning.contains("admission limit")));
    let expected = vec![17. / 255., 31. / 255., 57. / 255., 1.];
    assert_eq!(document.layers[0].surface().unwrap().read_region(document.bounds()), expected);
    let native = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.paths, document.paths);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    assert!(has_retention_limit(&reopened));
    assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
}

#[test]
fn vector_geometry_copy_shortage_keeps_raster_and_reports_retention_limit() {
    let bytes = standalone("Vector source", 1);
    for shortage in [true, false] {
        let mut image = image(&bytes);
        let path = paths::standalone(&image.c, 4).unwrap().named;
        let copy_bytes = std::mem::size_of::<photocraft_doc::vector::Knot>() + std::mem::size_of::<photocraft_doc::vector::Subpath>();
        image.meta_budget = copy_bytes - usize::from(shortage);
        let mut document = Document::new("Vector pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        document.paths.push(path);
        image.path_tattoos.insert(7, Some(0));
        let mut layer = Layer::raster("Vector owner", PixelFormat::RGBA8);
        let LayerContent::Raster(pixels) = &mut layer.content else { panic!("saved raster") };
        pixels.write_region(document.bounds(), &[1., 0., 0., 1.]);
        let id = layer.id;
        document.layers.push(layer);
        let owner = image
            .outcomes
            .push(ConversionEntry {
                feature_code: "xcf.layer".into(),
                source: conversion::source("layer", 0, 0, "Vector owner", &[], None),
                targets: vec![NativeTargetRef { layer_id: Some(id), ..Default::default() }],
                disposition: ConversionDisposition::NativeEquivalent,
                reason: ConversionReason::NativePolicy,
                source_description: "Saved vector layer pixels".into(),
                native_description: "Native raster owner".into(),
                attachments: Vec::new(),
            })
            .unwrap();
        image.pending_vectors.push((id, owner, paths::VectorStyle { path_tattoo: 7, modified: false, shape: Default::default(), source_only: Vec::new() }));
        import_vectors(&mut document, &mut image, &Interrupt::NONE).unwrap();
        if shortage {
            assert_eq!(image.meta_budget, copy_bytes - 1, "a discarded copy must not spend metadata allowance");
            assert_eq!(document.layers[0].surface().unwrap().read_region(document.bounds()), vec![1., 0., 0., 1.]);
            assert!(image.warnings.finish().iter().any(|warning| warning.contains("geometry allowance") && warning.contains("opens as pixels")));
            document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish().unwrap()));
            let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("conversion report") };
            assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.vector_layer" && entry.reason == ConversionReason::RetentionLimit));
            let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
            assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), vec![1., 0., 0., 1.]);
        } else {
            let LayerContent::Shape(shape) = &document.layers[0].content else { panic!("the exact allowance admits editable geometry") };
            assert_eq!(shape.path, document.paths[0].path);
            assert_eq!(image.meta_budget, 0);
        }
    }
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

fn tattoo_path_record(name: &str, version: u32) -> Vec<u8> {
    let mut body = W { out: Vec::new(), wide: false };
    body.prop_u32(20, 7);
    body.prop(43, &[]);
    body.end();
    body.u32(version);
    body.u32(0);
    let mut record = W { out: Vec::new(), wide: true };
    record.string(name);
    record.u32(body.out.len() as u32);
    record.out.extend(body.out);
    record.out
}

#[test]
fn malformed_optional_paths_do_not_make_retained_tattoos_or_selection_look_unique() {
    let valid = tattoo_path_record("retained", 1);
    let invalid = tattoo_path_record("malformed duplicate tattoo", 2);
    for malformed in [None, Some(invalid), Some(64u32.to_be_bytes().to_vec())] {
        for malformed_first in [false, true] {
            let mut bytes = vec![0; 4];
            let mut pointers = Vec::new();
            let records = if malformed_first { [malformed.as_deref(), Some(valid.as_slice())] } else { [Some(valid.as_slice()), malformed.as_deref()] };
            for record in records.into_iter().flatten() {
                pointers.push(bytes.len());
                bytes.extend_from_slice(record);
            }
            let mut image = image(&bytes);
            let mut document = Document::new("Usable pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
            let mut layer = Layer::raster("Saved raster", PixelFormat::RGBA8);
            let expected = vec![17. / 255., 31. / 255., 57. / 255., 1.];
            let LayerContent::Raster(pixels) = &mut layer.content else { panic!("raster") };
            pixels.write_region(document.bounds(), &expected);
            document.layers.push(layer);
            import_paths(&mut document, &mut image, &[], &pointers, None, &Interrupt::NONE).unwrap();
            assert_eq!(document.paths.len(), 1);
            let complete = malformed.is_none();
            assert_eq!(image.initial_view.selected_path, complete.then_some(0));
            assert_eq!(image.path_tattoos.get(&7), Some(&complete.then_some(0)));
            let conversion = image.outcomes.finish().unwrap();
            let ImportConversion::Report(report) = &conversion else { panic!("conversion report") };
            assert_eq!(
                report.entries.iter().filter(|entry| entry.feature_code == "xcf.path" && entry.reason == ConversionReason::InvalidOptionalData).count(),
                usize::from(!complete)
            );
            let raw_entries: Vec<_> = report
                .entries
                .iter()
                .filter(|entry| entry.feature_code == "xcf.source_property.25" && entry.reason == ConversionReason::InvalidOptionalData)
                .collect();
            let framed_malformed = malformed.as_deref().filter(|record| record.len() > 4);
            assert_eq!(raw_entries.len(), usize::from(framed_malformed.is_some()));
            if let Some(raw) = framed_malformed {
                let entry = raw_entries[0];
                assert_eq!(entry.attachments.len(), 1);
                assert_eq!(report.attachments[entry.attachments[0] as usize].bytes.as_slice(), raw);
            }
            assert!(!report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit));
            document.metadata.import_conversion = Some(Arc::new(conversion));
            let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened.paths, document.paths);
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
            assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
        }
    }
}

#[test]
fn malformed_vector_property_disables_identity_guesses_for_other_admitted_paths() {
    // This property declares tattoo 7 but has an incomplete record; its identity
    // cannot safely be excluded when resolving a separately framed valid path.
    let mut malformed = W { out: Vec::new(), wide: false };
    for value in [1, 0, 1] {
        malformed.u32(value);
    }
    malformed.string("incomplete duplicate");
    malformed.u32(7);
    let mut bytes = vec![0; 4];
    bytes.extend(tattoo_path_record("retained", 1));
    let mut image = image(&bytes);
    let properties = [Prop { kind: 25, offset: 0, data: &malformed.out }];
    let mut document = Document::new("Paths", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    import_paths(&mut document, &mut image, &properties, &[4], None, &Interrupt::NONE).unwrap();
    assert_eq!(document.paths.len(), 1);
    assert_eq!(image.initial_view.selected_path, None);
    assert_eq!(image.path_tattoos.get(&7), Some(&None));
    let conversion = image.outcomes.finish().unwrap();
    let ImportConversion::Report(report) = &conversion else { panic!("conversion report") };
    assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::InvalidOptionalData));
    assert!(!report.entries.iter().any(|entry| entry.reason == ConversionReason::RetentionLimit));
}
