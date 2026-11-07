//! Compatibility of adjustment records and format envelopes.
use super::*;

/// Preserve the existing schema and values of every other adjustment type.
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

/// Small adjustments retain the exact legacy representation and envelope version.
#[test]
fn small_adjustments_remain_compatible_with_version_one() {
    for adjustment in [lookup(), Adjustment::Unsupported { psd_key: "test".into(), raw: vec![3; 4096].into() }] {
        let doc = document(adjustment.clone(), SampleType::U8);
        let (bytes, stats) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
        let m = manifest(&bytes);
        assert_eq!(m["format_version"], 1);
        assert_eq!(stats.blobs_total, 0);
        assert_eq!(m["document"]["layers"][0]["content"]["adjustment"], serde_json::to_value(adjustment).unwrap());
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}

/// The original reader's envelope check rejects blob records before document decoding.
#[test]
fn blob_adjustments_report_too_new_to_version_one_readers() {
    for adjustment in [blob_lookup(), Adjustment::Unsupported { psd_key: "test".into(), raw: vec![3; 4097].into() }] {
        let doc = document(adjustment, SampleType::U8);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["format_version"], 2);
        let zip = zip::ZipReader::new(&bytes).unwrap();
        let raw = zip.read_by_name("manifest.json", usize::MAX).unwrap();
        let result = craft_pcraft_format::read_manifest_with::<serde_json::Value, FormatError>(&raw, &Default::default(), &[], 1, |_| {
            panic!("old reader must reject before decoding adjustments")
        });
        assert!(matches!(result, Err(FormatError::TooNew { found: 2, supported: 1 })));
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}

/// Nested blob adjustments also require a newer reader; empty groups do not.
#[test]
fn nested_adjustments_select_the_required_envelope_version() {
    for (adjustment, version) in [(lookup(), 1), (blob_lookup(), 2)] {
        let mut doc = document(adjustment, SampleType::U8);
        doc.layers = vec![Layer::group("outer", vec![Layer::group("inner", std::mem::take(&mut doc.layers))])];
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["format_version"], version);
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}

/// The inline threshold counts LUT samples and source bytes together.
#[test]
fn lookup_inline_threshold_counts_all_payload_values() {
    for (source_len, version) in [(4072, 1), (4073, 2)] {
        let mut adjustment = lookup();
        let Adjustment::ColorLookup { source: Some(source), .. } = &mut adjustment else { unreachable!() };
        source.bytes = Arc::new(vec![42; source_len]);
        let doc = document(adjustment, SampleType::U8);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["format_version"], version);
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}
