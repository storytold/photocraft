use super::*;

#[test]
fn optional_attachment_respects_portable_entry_and_target_costs() {
    let mut outcomes = Outcomes::new(26).unwrap();
    let owner = outcomes.push(ConversionEntry {
        feature_code: "xcf.layer".into(),
        source: source("layer", 0, 20, "Owner", &[], None),
        targets: vec![NativeTargetRef::default(); 1000],
        disposition: ConversionDisposition::NativeEquivalent,
        reason: ConversionReason::NativePolicy,
        source_description: String::new(),
        native_description: String::new(),
        attachments: Vec::new(),
    }).unwrap();
    let bytes = vec![7; MAX_CONVERSION_BYTES - 250_000];
    outcomes.property(owner, 999, 30, &bytes).unwrap();
    let ImportConversion::Report(report) = outcomes.finish().expect("optional data must not make the import fail") else { panic!("typed report") };
    assert!(report.attachments.is_empty());
    assert_eq!(report.entries[1].disposition, ConversionDisposition::SkippedNonessential);
    assert_eq!(report.entries[1].reason, ConversionReason::RetentionLimit);
    report.validate().unwrap();
}
