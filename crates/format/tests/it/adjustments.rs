//! Binary adjustment payloads must not consume the manifest's JSON value budget.
use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::adjust::{LutFormat, LutSource};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_format::{FormatError, LoadOptions, PcraftWriter, SaveOptions, load_from_bytes, load_from_bytes_with, load_path, save_to_bytes, zip};

fn document(adjustment: Adjustment, depth: SampleType) -> Document {
    let mut doc = Document::new("adjustments", Size::new(2, 2), ColorMode::Rgb, depth);
    doc.layers.push(Layer::new("lookup", LayerContent::Adjustment(adjustment)));
    doc
}

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

fn manifest(bytes: &[u8]) -> serde_json::Value {
    let zip = zip::ZipReader::new(bytes).unwrap();
    serde_json::from_slice(&zip.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap()
}

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

#[test]
fn legacy_inline_adjustments_still_open() {
    for adjustment in [lookup(), Adjustment::Unsupported { psd_key: "test".into(), raw: Arc::from([1, 2, 3]) }] {
        let doc = document(adjustment.clone(), SampleType::U8);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let legacy = with_adjustment(&bytes, serde_json::to_value(adjustment).unwrap());
        assert_eq!(load_from_bytes(&legacy).unwrap(), doc);
    }
}

#[test]
fn unsupported_adjustment_bytes_use_blobs() {
    let doc = document(Adjustment::Unsupported { psd_key: "test".into(), raw: vec![3; 4_194_305].into() }, SampleType::U8);
    let (bytes, stats) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
    assert!(stats.manifest_bytes < 10_000);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn lookup_blobs_obey_load_budgets_and_require_valid_hashes() {
    let doc = document(lookup(), SampleType::U8);
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

#[test]
fn nonfinite_lookup_samples_cannot_be_saved() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut adjustment = lookup();
        let Adjustment::ColorLookup { lut, .. } = &mut adjustment else { unreachable!() };
        *lut = Some(Arc::new(vec![value; 24]));
        assert!(matches!(save_to_bytes(&document(adjustment, SampleType::U8), &SaveOptions::default()), Err(FormatError::NonFinite { .. })));
    }
}

#[test]
fn other_adjustments_keep_their_json_schema_and_roundtrip() {
    use photocraft_doc::adjust::{CurvePoint, HueRange, LevelsChannel, ToneSpace};
    let points = vec![CurvePoint { input: 0.0, output: 0.1 }, CurvePoint { input: 1.0, output: 0.9 }];
    let adjustments = vec![
        Adjustment::BrightnessContrast { brightness: 0.2, contrast: 0.3, legacy: false },
        Adjustment::Levels {
            master: LevelsChannel::default(),
            per_channel: std::array::from_fn(|_| LevelsChannel::default()),
            space: ToneSpace::Cmyk,
            black: LevelsChannel::default(),
        },
        Adjustment::Curves {
            master: points.clone(),
            per_channel: std::array::from_fn(|_| points.clone()),
            space: ToneSpace::Cmyk,
            black: points,
            pencil: true,
        },
        Adjustment::Exposure { exposure: 0.5, offset: -0.1, gamma: 1.2 },
        Adjustment::Vibrance { vibrance: 0.3, saturation: 0.1 },
        Adjustment::HueSaturation { hue: 0.2, saturation: 0.3, lightness: -0.1, colorize: true, ranges: HueRange::defaults() },
        Adjustment::ColorBalance { shadows: [0.1; 3], midtones: [0.2; 3], highlights: [0.3; 3], preserve_luminosity: true },
        Adjustment::BlackWhite { weights: [0.3; 6], tint: Some([0.1, 0.2, 0.3]) },
        Adjustment::PhotoFilter { color: [0.1, 0.2, 0.3], density: 0.5, preserve_luminosity: true },
        Adjustment::ChannelMixer { matrix: [[0.1; 4]; 3], monochrome: true },
        Adjustment::Invert,
        Adjustment::Posterize { levels: 8 },
        Adjustment::Threshold { level: 0.4 },
        Adjustment::GradientMap { stops: vec![(0.0, [0.1; 3]), (1.0, [0.9; 3])], reverse: true, dither: true },
        Adjustment::SelectiveColor { relative: true, adjustments: [[0.1; 4]; 9] },
    ];
    for adjustment in adjustments {
        let doc = document(adjustment.clone(), SampleType::U8);
        let (bytes, stats) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(stats.blobs_total, 0, "{}", adjustment.label());
        assert_eq!(
            manifest(&bytes)["document"]["layers"][0]["content"]["adjustment"],
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&adjustment).unwrap()).unwrap()
        );
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc, "{}", adjustment.label());
    }
}

#[test]
fn profile_lookup_without_a_table_or_source_roundtrips() {
    let doc = document(Adjustment::ColorLookup { name: "profile".into(), lut: None, size: 0, tetrahedral: false, dither: false, source: None }, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
