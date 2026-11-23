use super::*;
use crate::xcf::tests::{TestImage, TestLayer};
use crate::{HistoricalXcfOrder, HistoricalXcfSampleOrder, ImportOptions, IoError};
use photocraft_doc::{Document, import_conversion::ImportConversion};
use photocraft_geom::Rect;
use std::io::Write;
use std::sync::Arc;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn words(values: &[u16], order: SampleOrder) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| match order {
            SampleOrder::BigEndian => value.to_be_bytes(),
            SampleOrder::LittleEndian => value.to_le_bytes(),
        })
        .collect()
}

fn fixture(version: u32, order: SampleOrder, gray: bool) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = version;
    image.precision = 250;
    image.base_type = u32::from(gray);
    let values = if gray { vec![0xbeef, 0xffff] } else { vec![0x1234, 0xabcd, 42, 0xffff] };
    let mut layer = TestLayer::rgba("Original sample bytes", 1, 1, words(&values, order));
    layer.kind = if gray { 3 } else { 1 };
    layer.mask = Some(words(&[0x7fff], order));
    layer.apply_mask = false;
    image.layers.push(layer);
    image.channels.push(("Alpha", words(&[0x1234], order), false));
    image.channels.push(("Selection", words(&[0xabcd], order), true));
    image.write()
}

fn options(order: SampleOrder) -> ImportOptions {
    ImportOptions { historical_xcf_order: HistoricalXcfOrder::Explicit(order), ..Default::default() }
}

fn first_pixel(document: &Document) -> Vec<f32> {
    document.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1))
}

#[test]
fn explicit_historical_order_preserves_exact_color_mask_channel_and_selection_samples() {
    for version in [7, 10, 11] {
        for gray in [false, true] {
            for order in [SampleOrder::BigEndian, SampleOrder::LittleEndian] {
                let plain = fixture(version, order, gray);
                let wrapped = gzip(&plain);
                for (name, bytes) in [("source.xcf", plain.as_slice()), ("source.bin", wrapped.as_slice())] {
                    let imported = crate::import_with_options(name, bytes, &options(order)).unwrap();
                    assert_eq!(imported.historical_xcf_sample_order, Some(order));
                    let expected = if gray { vec![0xbeef as f32 / 65535., 1.] } else { vec![0x1234 as f32 / 65535., 0xabcd as f32 / 65535., 42. / 65535., 1.] };
                    assert_eq!(first_pixel(&imported.document), expected, "v{version} gray={gray} {order:?}");
                    assert_eq!(imported.document.layers[0].mask.as_ref().unwrap().surface.read_region(Rect::new(0, 0, 1, 1)), vec![0x7fff as f32 / 65535.]);
                    assert_eq!(imported.document.channels[0].surface.read_region(Rect::new(0, 0, 1, 1)), vec![0x1234 as f32 / 65535.]);
                    assert_eq!(imported.document.selection.as_ref().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![0xabcd as f32 / 65535.]);
                    assert!(imported.source_read_only && !imported.preview_only);
                }
            }
        }
    }
}

#[test]
fn required_choice_signal_allows_retry_of_the_exact_retained_source_without_changing_default() {
    let source = Arc::new(fixture(11, SampleOrder::LittleEndian, false));
    let original = source.clone();
    let require = ImportOptions { historical_xcf_order: HistoricalXcfOrder::RequireExplicit, ..Default::default() };
    for bytes in [source.as_slice().to_vec(), gzip(&source)] {
        let result = crate::import_with_options("source.xcf", &bytes, &require);
        assert!(matches!(result, Err(IoError::HistoricalXcfSampleOrderRequired(crate::HistoricalXcfSampleOrderRequest { version: 11, bits_per_sample: 16 }))));
    }
    assert!(Arc::ptr_eq(&source, &original));
    let retried = crate::import_with_options("source.xcf", &source, &options(SampleOrder::LittleEndian)).unwrap();
    assert_eq!(first_pixel(&retried.document)[0], 0x1234 as f32 / 65535.);
    assert_eq!(retried.historical_xcf_sample_order, Some(SampleOrder::LittleEndian));
    let compatible = crate::import("source.xcf", &source).unwrap();
    assert_eq!(compatible.historical_xcf_sample_order, None);
    assert_eq!(first_pixel(&compatible.document)[0], 0x3412 as f32 / 65535., "the existing default remains big-endian, never a heuristic");
    assert_ne!(first_pixel(&compatible.document), first_pixel(&retried.document));
}

#[test]
fn modern_and_byte_wide_sources_ignore_historical_choices_and_do_not_emit_historical_evidence() {
    let modern = fixture(12, SampleOrder::BigEndian, false);
    let expected = crate::import("source.xcf", &modern).unwrap().document;
    let mut byte_image = TestImage::rgb8(1, 1);
    byte_image.version = 11;
    byte_image.layers.push(TestLayer::rgba("Byte samples", 1, 1, vec![17, 31, 57, 255]));
    let bytes = byte_image.write();
    for source in [&modern, &bytes] {
        let compatible = crate::import("source.xcf", source).unwrap();
        for policy in [HistoricalOrder::RequireExplicit, HistoricalOrder::Explicit(SampleOrder::LittleEndian)] {
            let imported = crate::import_with_options("source.xcf", source, &ImportOptions { historical_xcf_order: policy, ..Default::default() }).unwrap();
            assert_eq!(imported.historical_xcf_sample_order, None);
            assert_eq!(first_pixel(&imported.document), first_pixel(&compatible.document));
            assert!(imported.document.metadata.import_conversion.is_none());
        }
    }
    assert_eq!(first_pixel(&expected)[0], 0x1234 as f32 / 65535.);
}

#[test]
fn required_choice_precedes_drawable_decode_but_not_full_wrapper_integrity() {
    let plain = fixture(11, SampleOrder::BigEndian, false);
    let truncated = &plain[..plain.len() - 1];
    let require = ImportOptions { historical_xcf_order: HistoricalXcfOrder::RequireExplicit, ..Default::default() };
    assert!(matches!(crate::import_with_options("source.xcf", truncated, &require), Err(IoError::HistoricalXcfSampleOrderRequired(_))));
    assert!(matches!(crate::import_with_options("source.xcf", truncated, &options(SampleOrder::BigEndian)), Err(IoError::Xcf(_))));
    let mut wrapped = gzip(&plain);
    let crc = wrapped.len() - 8;
    wrapped[crc] ^= 1;
    assert!(matches!(crate::import_with_options("source.xcf.gz", &wrapped, &require), Err(IoError::Xcf(_))));
}

#[test]
fn inert_order_evidence_survives_native_save_and_never_reapplies_on_reopen() {
    let source = fixture(11, SampleOrder::LittleEndian, false);
    let imported = crate::import_with_options("source.xcf", &source, &options(HistoricalXcfSampleOrder::LittleEndian)).unwrap();
    let record = imported.document.metadata.import_conversion.as_ref().unwrap();
    record.validate().unwrap();
    let ImportConversion::Report(report) = record.as_ref() else { panic!("known report") };
    assert_eq!(report.source_version, Some(11));
    let entry = &report.entries[0];
    assert_eq!(entry.feature_code, "xcf.historical_sample_order");
    assert!(entry.native_description.contains("little-endian") && entry.native_description.contains("explicit"));
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = crate::import_with_options(
        "native.pcraft",
        &native,
        &ImportOptions { historical_xcf_order: HistoricalXcfOrder::RequireExplicit, ..Default::default() },
    )
    .unwrap();
    assert_eq!(first_pixel(&reopened.document), first_pixel(&imported.document));
    assert_eq!(reopened.document.metadata.import_conversion, imported.document.metadata.import_conversion);
}

#[test]
fn evidence_attachment_preserves_existing_future_reports_and_other_metadata() {
    let source = fixture(11, SampleOrder::BigEndian, false);
    let xcf = craft_xcf::Xcf::parse(&source).unwrap();
    let mut document = crate::import("source.xcf", &source).unwrap().document;
    document.metadata.text.push(("Original field".into(), "Original value".into()));
    let existing = Arc::new(ImportConversion::Opaque { schema_version: 99, record: Arc::new(b"opaque".to_vec()), attachments: Vec::new() });
    document.metadata.import_conversion = Some(existing.clone());
    let entry = evidence(xcf.historical_sample_order_request(), HistoricalOrder::Explicit(SampleOrder::LittleEndian)).unwrap();
    let mut report = crate::xcf::report::Report::new(&source, 11);
    report.append(entry).unwrap();
    report.attach(&mut document, &mut crate::ora::Warnings::default());
    assert!(Arc::ptr_eq(&existing, document.metadata.import_conversion.as_ref().unwrap()));
    assert_eq!(document.metadata.text.last().unwrap(), &("Original field".into(), "Original value".into()));
}

#[cfg(any(unix, windows))]
mod source_tests;
