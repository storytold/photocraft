//! Recurring notices for persisted import history, independent of first-import disclosures.
use photocraft_doc::Document;
use photocraft_doc::import_conversion::{ConversionDisposition as D, ConversionEntry, ConversionReason as R, ConversionReport, ImportConversion};

pub(crate) fn needs_notice(doc: &Document) -> bool {
    match doc.metadata.import_conversion.as_deref() {
        Some(ImportConversion::Report(report)) => report.entries.iter().any(|entry| entry_needs_notice(report, entry)),
        Some(ImportConversion::Opaque { .. }) => true,
        None => false,
    }
}

fn entry_needs_notice(report: &ConversionReport, entry: &ConversionEntry) -> bool {
    if entry.disposition == D::NativeEquivalent {
        return false;
    }
    // These outcomes describe content already expressed by the native document.
    // The original import disclosure and inspectable report remain unchanged;
    // reopening or exporting the native result does not perform this conversion again.
    if entry.disposition == D::NativeApproximation
        && (entry.reason == R::NativePolicy || (entry.feature_code == "xcf.precision" && entry.reason == R::NumericalConversion))
    {
        return false;
    }
    // Only fully retained inert parser bookkeeping is quiet. Unapplied content,
    // invalid properties, missing bytes and retention failures remain actionable.
    !(entry.disposition == D::RetainedUnapplied
        && entry.feature_code.starts_with("xcf.source_property.")
        && matches!(entry.reason, R::MissingMapping | R::NativePolicy)
        && !entry.attachments.is_empty()
        && entry.attachments.iter().all(|index| report.attachments.get(*index as usize).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::import_conversion::SourceItemRef;

    fn entry(feature: &str, reason: R) -> ConversionEntry {
        ConversionEntry {
            feature_code: feature.into(),
            source: SourceItemRef::default(),
            targets: vec![],
            disposition: D::NativeApproximation,
            reason,
            source_description: "Source".into(),
            native_description: "Native outcome".into(),
            attachments: vec![],
        }
    }

    #[test]
    fn absent_history_is_quiet_and_future_opaque_history_still_warns() {
        let mut document = Document::new("Notices", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(!needs_notice(&document));
        document.metadata.import_conversion =
            Some(std::sync::Arc::new(ImportConversion::Opaque { schema_version: 2, record: std::sync::Arc::new(vec![1, 2, 3]), attachments: vec![] }));
        assert!(needs_notice(&document));
    }

    #[test]
    fn ordinary_native_mapping_is_quiet_but_numerical_damage_and_unapplied_reasons_are_not() {
        let report = ConversionReport::default();
        for feature in ["xcf.layer", "xcf.text", "xcf.text.formatting", "xcf.vector_layer", "xcf.path", "xcf.layer_mask", "xcf.channel"] {
            assert!(!entry_needs_notice(&report, &entry(feature, R::NativePolicy)), "{feature}");
        }
        assert!(!entry_needs_notice(&report, &entry("xcf.precision", R::NumericalConversion)));
        for feature in ["xcf.precision_underflow", "xcf.coverage_clamped", "xcf.precision.future", "other.precision"] {
            assert!(entry_needs_notice(&report, &entry(feature, R::NumericalConversion)), "{feature}");
        }
        for reason in [R::NativeLimitation, R::MissingMapping, R::ResourceUnavailable, R::InvalidOptionalData, R::RetentionLimit] {
            for feature in ["xcf.text", "xcf.vector_layer", "xcf.precision"] {
                assert!(entry_needs_notice(&report, &entry(feature, reason)), "{feature}: {reason:?}");
            }
        }
        let mut unapplied = entry("xcf.text", R::NativePolicy);
        unapplied.disposition = D::RetainedUnapplied;
        assert!(entry_needs_notice(&report, &unapplied));
    }
}
