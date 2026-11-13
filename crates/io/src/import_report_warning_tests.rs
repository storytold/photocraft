use super::*;
use photocraft_doc::import_conversion::{
    ConversionDisposition as D, ConversionEntry, ConversionReason as R, ConversionReport, ImportConversion, RetainedImportData, SourceItemRef,
};
use std::sync::Arc;

fn document(feature: &str, disposition: D, reason: R, retain: bool) -> Document {
    let source = SourceItemRef { kind: "layer".into(), name: "Source".into(), ..Default::default() };
    let mut report = ConversionReport {
        source_format: "xcf".into(),
        source_version: Some(26),
        policy_version: 1,
        entries: vec![ConversionEntry {
            feature_code: feature.into(),
            source: source.clone(),
            targets: Vec::new(),
            disposition,
            reason,
            source_description: "Source record".into(),
            native_description: "Historical conversion outcome".into(),
            attachments: if retain { vec![0] } else { Vec::new() },
        }],
        attachments: Vec::new(),
    };
    if retain {
        report.attachments.push(RetainedImportData {
            owner: source,
            kind: "xcf.property.6".into(),
            encoding: "xcf-property-payload".into(),
            bytes: Arc::new(vec![1, 2, 3]),
        });
    }
    report.validate().unwrap();
    let mut doc = Document::new("Import notices", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(report)));
    doc
}

#[test]
fn retained_parser_properties_are_quiet_but_content_losses_still_warn_after_native_reopen_and_export() {
    for (feature, disposition, reason, retain, warn) in [
        ("xcf.layer", D::NativeEquivalent, R::NativePolicy, false, false),
        ("xcf.source_property.6", D::RetainedUnapplied, R::MissingMapping, true, false),
        ("xcf.source_property.6", D::RetainedUnapplied, R::NativePolicy, true, false),
        ("xcf.source_property.6", D::RetainedUnapplied, R::InvalidOptionalData, true, true),
        ("xcf.source_property.6", D::RetainedUnapplied, R::NativeLimitation, true, true),
        ("xcf.source_property.6", D::RetainedUnapplied, R::MissingMapping, false, true),
        ("xcf.source_property.6", D::SkippedNonessential, R::RetentionLimit, false, true),
        ("xcf.coverage_effect", D::RetainedUnapplied, R::MissingMapping, true, true),
        ("xcf.precision", D::NativeApproximation, R::NumericalConversion, false, true),
        ("xcf.coverage_effect_mask", D::SkippedNonessential, R::NativeLimitation, false, true),
        ("xcf.report_truncated", D::SkippedNonessential, R::RetentionLimit, false, true),
    ] {
        let doc = document(feature, disposition, reason, retain);
        assert_eq!(open_warnings(&doc).len(), usize::from(warn), "{feature}: {disposition:?}");
        let native = export(&doc, "saved.pcraft", &Default::default()).unwrap();
        assert!(!native.warnings.iter().any(|warning| warning.contains("not exported")));
        let reopened = import("saved.pcraft", &native.bytes).unwrap();
        assert_eq!(reopened.document.metadata.import_conversion, doc.metadata.import_conversion);
        assert_eq!(reopened.warnings.len(), usize::from(warn), "reopened {feature}: {disposition:?}");
        let flat = export(&reopened.document, "png", &Default::default()).unwrap();
        assert_eq!(
            flat.warnings.iter().filter(|warning| warning.contains("import conversion report") && warning.contains("not exported")).count(),
            usize::from(warn),
            "exported {feature}: {disposition:?}"
        );
    }
}

#[test]
fn unknown_future_reports_remain_visible() {
    let mut doc = document("xcf.layer", D::NativeEquivalent, R::NativePolicy, false);
    doc.metadata.import_conversion = Some(Arc::new(ImportConversion::Opaque { schema_version: 2, record: Arc::new(b"{}".to_vec()), attachments: Vec::new() }));
    assert_eq!(open_warnings(&doc).len(), 1);
    let flat = export(&doc, "png", &Default::default()).unwrap();
    assert_eq!(flat.warnings.iter().filter(|warning| warning.contains("import conversion report") && warning.contains("not exported")).count(), 1);
}

/// Original v26/U8 framing with one saved channel, one property and one
/// uncompressed pixel. No shared fixture visibility or corpus data is needed.
fn channel_property_xcf(kind: u32, payload: &[u8]) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn pointer(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend(value.to_be_bytes());
    }
    fn slot(bytes: &mut Vec<u8>) -> usize {
        let at = bytes.len();
        pointer(bytes, 0);
        at
    }
    fn patch(bytes: &mut [u8], at: usize) {
        let end = bytes.len() as u64;
        bytes.get_mut(at..at + 8).unwrap().copy_from_slice(&end.to_be_bytes());
    }
    let mut bytes = b"gimp xcf v026\0".to_vec();
    for value in [1, 1, 0, 150] {
        word(&mut bytes, value);
    }
    word(&mut bytes, 17); // Compression: no compression.
    word(&mut bytes, 1);
    bytes.push(0);
    word(&mut bytes, 0);
    word(&mut bytes, 0); // End image properties.
    pointer(&mut bytes, 0); // No layers.
    let channel = slot(&mut bytes);
    pointer(&mut bytes, 0); // End channels.
    pointer(&mut bytes, 0); // No paths.
    patch(&mut bytes, channel);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let name = b"Warning channel\0";
    word(&mut bytes, name.len() as u32);
    bytes.extend(name);
    word(&mut bytes, kind);
    word(&mut bytes, payload.len() as u32);
    bytes.extend(payload);
    word(&mut bytes, 0);
    word(&mut bytes, 0); // End channel properties.
    let hierarchy = slot(&mut bytes);
    pointer(&mut bytes, 0); // No channel effects in v26.
    patch(&mut bytes, hierarchy);
    for value in [1, 1, 1] {
        word(&mut bytes, value);
    } // One one-byte sample.
    let level = slot(&mut bytes);
    pointer(&mut bytes, 0); // End hierarchy levels.
    patch(&mut bytes, level);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let tile = slot(&mut bytes);
    pointer(&mut bytes, 0); // End tiles.
    patch(&mut bytes, tile);
    bytes.push(128);
    bytes
}

#[test]
fn retained_channel_errors_and_limitations_warn_after_public_import_native_reopen_and_export() {
    for (kind, payload, reason) in [(33, vec![0], R::InvalidOptionalData), (28, 1u32.to_be_bytes().to_vec(), R::NativeLimitation)] {
        // Truncated FLOAT_OPACITY is rejected by channel_overlay; LOCK_CONTENT
        // has no native alpha-channel counterpart. Both have only the retained
        // property outcome, so no separate outcome can rescue a hidden notice.
        let source = channel_property_xcf(kind, &payload);
        let imported = import("channel-property.xcf", &source).unwrap();
        assert!(!imported.preview_only);
        assert!(imported.source_read_only);
        assert!(imported.document.layers.is_empty());
        assert_eq!(imported.document.channels.len(), 1);
        let pixels = imported.document.channels[0].surface.read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
        assert_eq!(pixels, vec![128. / 255.]);
        assert_eq!(imported.document.channels[0].opacity, 0.5, "rejected or unsupported controls keep native defaults");
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
        report.validate().unwrap();
        let property = report.entries.iter().find(|entry| entry.feature_code == format!("xcf.source_property.{kind}")).unwrap();
        assert_eq!(property.disposition, D::RetainedUnapplied);
        assert_eq!(property.reason, reason);
        assert_eq!(property.attachments.len(), 1);
        assert_eq!(report.attachments[property.attachments[0] as usize].bytes.as_slice(), payload);
        assert_eq!(
            report.entries.iter().filter(|entry| entry.disposition != D::NativeEquivalent).count(),
            1,
            "the retained property is the sole exceptional outcome"
        );
        assert_eq!(open_warnings(&imported.document).iter().filter(|warning| warning.contains("historical import conversion report")).count(), 1);
        let saved = export(&imported.document, "channel-property.pcraft", &Default::default()).unwrap();
        let reopened = import("channel-property.pcraft", &saved.bytes).unwrap();
        assert_eq!(reopened.document.metadata.import_conversion, imported.document.metadata.import_conversion);
        assert_eq!(reopened.document.channels[0].surface.read_region(photocraft_geom::Rect::new(0, 0, 1, 1)), pixels);
        assert_eq!(reopened.warnings.iter().filter(|warning| warning.contains("historical import conversion report")).count(), 1);
        let flat = export(&reopened.document, "png", &Default::default()).unwrap();
        assert_eq!(flat.warnings.iter().filter(|warning| warning.contains("import conversion report") && warning.contains("not exported")).count(), 1);
    }
}
