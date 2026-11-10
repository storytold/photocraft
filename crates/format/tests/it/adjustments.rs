//! Binary adjustment payloads must not consume the manifest's JSON value budget.
use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::adjust::{LutFormat, LutSource};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_format::{FormatError, LoadOptions, PcraftWriter, SaveOptions, load_from_bytes, load_from_bytes_with, load_path, save_to_bytes, zip};

/// Build a minimal document containing the requested adjustment and sample depth.
fn document(adjustment: Adjustment, depth: SampleType) -> Document {
    let mut doc = Document::new("adjustments", Size::new(2, 2), ColorMode::Rgb, depth);
    doc.layers.push(Layer::new("lookup", LayerContent::Adjustment(adjustment)));
    doc
}

/// Build a small LUT with its original source bytes and non-default options.
fn lookup() -> Adjustment {
    Adjustment::ColorLookup {
        name: "look.cube".into(),
        lut: Some(Arc::new(vec![0.5; 24])),
        size: 2,
        tetrahedral: true,
        dither: true,
        source: Some(LutSource { format: LutFormat::Cube, bytes: Arc::new(b"original LUT bytes".to_vec()) }),
    }
}

/// Read the uncompressed manifest for schema assertions.
fn manifest(bytes: &[u8]) -> serde_json::Value {
    let zip = zip::ZipReader::new(bytes).unwrap();
    serde_json::from_slice(&zip.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap()
}

/// Replace one adjustment record while preserving every other bundle entry.
fn with_adjustment(bytes: &[u8], adjustment: serde_json::Value) -> Vec<u8> {
    let input = zip::ZipReader::new(bytes).unwrap();
    let mut output = zip::ZipWriter::new();
    for entry in &input.entries {
        let mut data = input.read(entry, usize::MAX).unwrap();
        if entry.name == "manifest.json" {
            let mut m: serde_json::Value = serde_json::from_slice(&data).unwrap();
            m["document"]["layers"][0]["content"]["adjustment"] = adjustment.clone();
            data = serde_json::to_vec(&m).unwrap();
        }
        output.add(&entry.name, &data).unwrap();
    }
    output.finish().unwrap()
}

/// Verify large payloads round-trip at every depth through ZIP and directory bundles.
#[test]
fn large_lookup_and_source_use_blobs_and_roundtrip_at_every_depth() {
    let mut adjustment = lookup();
    let Adjustment::ColorLookup { lut, size, source, .. } = &mut adjustment else { unreachable!() };
    *size = 128;
    *lut = Some(Arc::new((0..128 * 128 * 128 * 3).map(|i| (i % 1009) as f32 / 1008.0).collect()));
    source.as_mut().unwrap().bytes = Arc::new(vec![42; 4_194_305]);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = document(adjustment.clone(), depth);
        let mut writer = PcraftWriter::new();
        let (bytes, stats) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
        assert!(stats.manifest_bytes < 10_000, "{}", stats.manifest_bytes);
        assert_eq!(stats.blobs_total, 2);
        let m = manifest(&bytes);
        let lookup = &m["document"]["layers"][0]["content"]["adjustment"]["ColorLookup"];
        assert!(lookup["lut"].is_string());
        assert!(lookup["source"]["bytes"].is_string());
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
        let dir = std::env::temp_dir().join(format!("pcraft-lookup-{}-{depth:?}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
        assert_eq!(load_path(&dir).unwrap(), doc);
        writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
        assert_eq!(load_path(&dir).unwrap(), doc, "GC keeps referenced LUT blobs");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Verify pre-blob inline adjustment records remain readable.
#[test]
fn legacy_inline_adjustments_still_open() {
    for adjustment in [lookup(), Adjustment::Unsupported { psd_key: "test".into(), raw: Arc::from([1, 2, 3]) }] {
        let doc = document(adjustment.clone(), SampleType::U8);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let legacy = with_adjustment(&bytes, serde_json::to_value(adjustment).unwrap());
        assert_eq!(load_from_bytes(&legacy).unwrap(), doc);
    }
}

/// Keep opaque adjustment data outside the manifest value budget.
#[test]
fn unsupported_adjustment_bytes_use_blobs() {
    let doc = document(Adjustment::Unsupported { psd_key: "test".into(), raw: vec![3; 4_194_305].into() }, SampleType::U8);
    let (bytes, stats) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
    assert!(stats.manifest_bytes < 10_000);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

/// Reject payloads beyond the load budgets and malformed or missing blob hashes.
#[test]
fn lookup_blobs_obey_load_budgets_and_require_valid_hashes() {
    let doc = document(blob_lookup(), SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    for limits in [LoadOptions { max_blob_bytes: 16, ..Default::default() }, LoadOptions { max_total_bytes: 16, ..Default::default() }] {
        assert!(matches!(load_from_bytes_with(&bytes, &limits), Err(FormatError::LimitExceeded(_))));
    }
    for hash in ["invalid".to_string(), "0".repeat(64)] {
        let mut adjustment = manifest(&bytes)["document"]["layers"][0]["content"]["adjustment"].clone();
        adjustment["ColorLookup"]["lut"] = hash.into();
        assert!(load_from_bytes(&with_adjustment(&bytes, adjustment)).is_err());
    }
}

/// Reject non-finite LUT samples before publishing a save.
#[test]
fn nonfinite_lookup_samples_cannot_be_saved() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut adjustment = lookup();
        let Adjustment::ColorLookup { lut, .. } = &mut adjustment else { unreachable!() };
        *lut = Some(Arc::new(vec![value; 24]));
        assert!(matches!(save_to_bytes(&document(adjustment, SampleType::U8), &SaveOptions::default()), Err(FormatError::NonFinite { .. })));
    }
}

/// Preserve profile lookups that do not contain a decoded LUT or source file.
#[test]
fn profile_lookup_without_a_table_or_source_roundtrips() {
    let doc = document(Adjustment::ColorLookup { name: "profile".into(), lut: None, size: 0, tetrahedral: false, dither: false, source: None }, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

/// Charge shared blobs once and reuse both decoded samples and original source bytes.
#[test]
fn shared_lookup_tables_obey_distinct_blob_budget() {
    let adjustment = blob_lookup();
    let Adjustment::ColorLookup { lut: Some(lut), source: Some(source), .. } = &adjustment else { unreachable!() };
    let budget = (lut.len() * 4 + source.bytes.len()) as u64;
    let mut doc = document(adjustment.clone(), SampleType::U8);
    doc.layers.push(Layer::new("second lookup", LayerContent::Adjustment(adjustment)));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes_with(&bytes, &LoadOptions { max_total_bytes: budget, ..Default::default() }).unwrap();
    assert_eq!(loaded, doc);
    let tables: Vec<_> = loaded
        .layers
        .iter()
        .map(|layer| {
            let LayerContent::Adjustment(Adjustment::ColorLookup { lut: Some(lut), source: Some(source), .. }) = &layer.content else { unreachable!() };
            (lut, &source.bytes)
        })
        .collect();
    assert!(Arc::ptr_eq(tables[0].0, tables[1].0));
    assert!(Arc::ptr_eq(tables[0].1, tables[1].1));
    assert!(matches!(load_from_bytes_with(&bytes, &LoadOptions { max_total_bytes: budget - 1, ..Default::default() }), Err(FormatError::LimitExceeded(_))));
}

/// Force blob storage while keeping the budget regressions inexpensive.
fn blob_lookup() -> Adjustment {
    let mut adjustment = lookup();
    let Adjustment::ColorLookup { source: Some(source), .. } = &mut adjustment else { unreachable!() };
    source.bytes = Arc::new(vec![42; 4097]);
    adjustment
}

#[path = "adjustment_compatibility.rs"]
mod compatibility;
