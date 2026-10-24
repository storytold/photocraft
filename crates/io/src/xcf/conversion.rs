//! Bounded historical outcomes, never a source-policy evaluator.

use std::io::Write;

use photocraft_doc::import_conversion::{
    ConversionDisposition, ConversionEntry, ConversionReason, ConversionReport, ImportConversion, MAX_CONVERSION_ATTACHMENTS, MAX_CONVERSION_BYTES,
    MAX_CONVERSION_ENTRIES, NativeTargetRef, RetainedImportData, SourceItemRef,
};

use super::{Result, invalid};

pub(super) struct Outcomes<'a> {
    report: ConversionReport,
    /// Count JSON incrementally without allocating an escaped copy of each source string.
    encoded: usize,
    retained: usize,
    pending: Vec<Pending<'a>>,
}

struct Pending<'a> {
    entry: usize,
    property: u32,
    bytes: &'a [u8],
    reason: ConversionReason,
}

struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_CONVERSION_BYTES)
            .ok_or_else(|| std::io::Error::other("XCF conversion report exceeds its separate 32 MiB allowance"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> Outcomes<'a> {
    pub(super) fn entry(&self, index: usize) -> Result<&ConversionEntry> {
        self.report.entries.get(index).ok_or_else(|| invalid("missing XCF conversion owner"))
    }
    pub(super) fn new(version: u32) -> Result<Self> {
        let report = ConversionReport { source_format: "xcf".into(), source_version: Some(version), policy_version: 1, ..Default::default() };
        let mut count = Count(0);
        serde_json::to_writer(&mut count, &report).map_err(|e| invalid(e.to_string()))?;
        Ok(Self { retained: report.source_format.len(), report, encoded: count.0, pending: Vec::new() })
    }

    pub(super) fn push(&mut self, entry: ConversionEntry) -> Result<usize> {
        if self.report.entries.len() >= MAX_CONVERSION_ENTRIES {
            return Err(invalid("XCF conversion report exceeds 65,536 entries"));
        }
        // Includes conservative native struct overhead as well as owned strings/paths.
        let bytes = std::mem::size_of::<ConversionEntry>()
            .saturating_add(entry.feature_code.len())
            .saturating_add(entry.source.kind.len())
            .saturating_add(entry.source.name.len())
            .saturating_add(std::mem::size_of_val(entry.source.item_path.as_slice()))
            .saturating_add(entry.source_description.len())
            .saturating_add(entry.native_description.len())
            .saturating_add(
                entry
                    .targets
                    .iter()
                    .map(|t| std::mem::size_of::<NativeTargetRef>() + t.label.len() + std::mem::size_of_val(t.item_path.as_slice()))
                    .sum::<usize>(),
            );
        let retained = self
            .retained
            .checked_add(bytes)
            .filter(|n| *n <= MAX_CONVERSION_BYTES)
            .ok_or_else(|| invalid("XCF conversion report exceeds its separate 32 MiB allowance"))?;
        let mut count = Count(self.encoded.saturating_add(1)); // One comma, conservative for first entry.
        serde_json::to_writer(&mut count, &entry).map_err(|e| invalid(format!("XCF conversion report metadata limit: {e}")))?;
        let index = self.report.entries.len();
        self.report.entries.push(entry);
        self.encoded = count.0;
        self.retained = retained;
        Ok(index)
    }

    pub(super) fn detail(&mut self, owner: usize, feature: &str, reason: ConversionReason, source: String, native: String) -> Result<()> {
        let entry = self.report.entries.get(owner).ok_or_else(|| invalid("missing XCF conversion owner"))?;
        let entry = ConversionEntry {
            feature_code: feature.into(),
            source: entry.source.clone(),
            targets: entry.targets.clone(),
            disposition: ConversionDisposition::NativeApproximation,
            reason,
            source_description: source,
            native_description: native,
            attachments: Vec::new(),
        };
        self.push(entry)?;
        Ok(())
    }

    /// The compatible short warning and its durable detailed outcome are produced together.
    pub(super) fn notice(
        &mut self,
        owner: usize,
        feature: &str,
        reason: ConversionReason,
        descriptions: (String, String),
        human: Option<&str>,
        warnings: &mut crate::ora::Warnings,
    ) -> Result<()> {
        self.detail(owner, feature, reason, descriptions.0, descriptions.1)?;
        if let Some(human) = human {
            let owner = self.report.entries.get(owner).ok_or_else(|| invalid("missing XCF conversion owner"))?;
            warnings.layer(&owner.source.name, human);
        }
        Ok(())
    }

    /// Borrow optional property payloads until all mandatory outcomes fit. Optional bytes must
    /// never make a later mandatory outcome fail simply because they appeared earlier in XCF.
    pub(super) fn property(&mut self, owner: usize, property: u32, offset: usize, bytes: &'a [u8]) -> Result<()> {
        self.property_with_reason(owner, property, offset, bytes, ConversionReason::MissingMapping)
    }

    pub(super) fn property_with_reason(&mut self, owner: usize, property: u32, offset: usize, bytes: &'a [u8], reason: ConversionReason) -> Result<()> {
        let owner = self.report.entries.get(owner).ok_or_else(|| invalid("missing XCF configuration owner"))?;
        let mut source = owner.source.clone();
        source.record_offset = Some(offset as u64);
        let index = self.push(ConversionEntry {
            feature_code: format!("xcf.source_property.{property}"),
            source,
            targets: owner.targets.clone(),
            disposition: ConversionDisposition::SkippedNonessential,
            reason: ConversionReason::RetentionLimit,
            source_description: format!("Optional source property {property}; configuration bytes confer no resource permissions"),
            native_description:
                "The raw configuration is inert. Supported native translations are recorded separately. Missing attachment bytes indicate retention limits."
                    .into(),
            attachments: Vec::new(),
        })?;
        if self.pending.len() < MAX_CONVERSION_ATTACHMENTS {
            self.pending.push(Pending { entry: index, property, bytes, reason });
        }
        Ok(())
    }

    pub(super) fn parasites(&mut self, owner: usize, offset: usize, bytes: &'a [u8], handled: &[&str]) -> Result<()> {
        let mut cursor = super::Cursor { b: bytes, pos: 0, wide: false };
        while cursor.pos < bytes.len() {
            let start = cursor.pos;
            let record = (|| {
                let name = cursor.string()?;
                cursor.u32()?; // Inert source flags, never permissions.
                let length = cursor.u32()? as usize;
                cursor.take(length)?;
                Ok::<_, super::IoError>(name)
            })();
            let offset = offset.saturating_add(8).saturating_add(start);
            match record {
                Ok(name) if handled.contains(&name.as_str()) => {}
                Ok(_) => self.property(owner, 21, offset, bytes.get(start..cursor.pos).ok_or_else(|| invalid("invalid source configuration bounds"))?)?,
                Err(_) => {
                    self.property_with_reason(
                        owner,
                        21,
                        offset,
                        bytes.get(start..).ok_or_else(|| invalid("invalid source configuration bounds"))?,
                        ConversionReason::InvalidOptionalData,
                    )?;
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<ImportConversion> {
        self.report.validate().map_err(|e| invalid(e.to_string()))?;
        let mut payload_bytes = 0usize;
        for pending in self.pending {
            let owner = self.report.entries.get(pending.entry).ok_or_else(|| invalid("missing XCF configuration outcome"))?;
            let mut attachment = RetainedImportData {
                owner: owner.source.clone(),
                kind: format!("xcf.property.{}", pending.property),
                encoding: "xcf-property-payload".into(),
                bytes: std::sync::Arc::new(Vec::new()),
            };
            let memory = std::mem::size_of::<RetainedImportData>()
                .saturating_add(attachment.owner.kind.len())
                .saturating_add(attachment.owner.name.len())
                .saturating_add(std::mem::size_of_val(attachment.owner.item_path.as_slice()))
                .saturating_add(attachment.kind.len())
                .saturating_add(attachment.encoding.len())
                .saturating_add(pending.bytes.len())
                .saturating_add(4);
            let Some(retained) = self.retained.checked_add(memory).filter(|n| *n <= MAX_CONVERSION_BYTES) else { continue };
            // Reserve 16 bytes for commas, the new attachment index, and enum spelling changes.
            let Some(base) = self
                .encoded
                .checked_add(payload_bytes)
                .and_then(|n| n.checked_add(pending.bytes.len()))
                .and_then(|n| n.checked_add(16))
                .filter(|n| *n <= MAX_CONVERSION_BYTES)
            else {
                continue;
            };
            let mut count = Count(base);
            if serde_json::to_writer(&mut count, &attachment).is_err() {
                continue;
            }
            let entry = self.report.entries.get_mut(pending.entry).ok_or_else(|| invalid("missing XCF configuration outcome"))?;
            entry.disposition = ConversionDisposition::RetainedUnapplied;
            entry.reason = pending.reason;
            entry.attachments.push(self.report.attachments.len() as u32);
            attachment.bytes = std::sync::Arc::new(pending.bytes.to_vec());
            self.report.attachments.push(attachment);
            self.retained = retained;
            self.encoded = self.encoded.saturating_add(count.0.saturating_sub(base)).saturating_add(16);
            payload_bytes = payload_bytes.saturating_add(pending.bytes.len());
        }
        self.report.validate().map_err(|e| invalid(e.to_string()))?;
        Ok(ImportConversion::Report(self.report))
    }
}

pub(super) fn source(kind: &str, ordinal: usize, offset: usize, name: &str, path: &[u32], tattoo: Option<u32>) -> SourceItemRef {
    SourceItemRef {
        kind: kind.into(),
        ordinal: ordinal as u32,
        record_offset: Some(offset as u64),
        name: name.into(),
        item_path: path.iter().copied().take(64).collect(),
        tattoo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_source_names_cannot_create_an_unpersistable_report() {
        let mut outcomes = Outcomes::new(11).unwrap();
        let entry = ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: source("layer", 0, 20, &"\u{1}".repeat(1 << 20), &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::NativeApproximation,
            reason: ConversionReason::NativePolicy,
            source_description: String::new(),
            native_description: String::new(),
            attachments: Vec::new(),
        };
        for _ in 0..5 {
            outcomes.push(entry.clone()).unwrap();
        }
        assert!(outcomes.push(entry).unwrap_err().to_string().contains("metadata limit"));
        outcomes.finish().unwrap();
    }

    #[test]
    fn optional_bytes_wait_until_all_mandatory_outcomes_fit() {
        let bytes = vec![7; 16 << 20];
        let mut outcomes = Outcomes::new(11).unwrap();
        let mut entry = ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: source("layer", 0, 20, "Owner", &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: String::new(),
            native_description: String::new(),
            attachments: Vec::new(),
        };
        let owner = outcomes.push(entry.clone()).unwrap();
        outcomes.property(owner, 999, 30, &bytes).unwrap();
        entry.source.name = "\u{1}".repeat(1 << 20);
        for _ in 0..3 {
            outcomes.push(entry.clone()).unwrap();
        }
        let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("typed report expected") };
        assert!(report.attachments.is_empty());
        let omitted = report.entries.iter().find(|e| e.feature_code == "xcf.source_property.999").unwrap();
        assert_eq!((omitted.disposition, omitted.reason), (ConversionDisposition::SkippedNonessential, ConversionReason::RetentionLimit));
    }

    #[test]
    fn attachment_count_is_capped_with_explicit_omission() {
        let mut outcomes = Outcomes::new(11).unwrap();
        let owner = outcomes
            .push(ConversionEntry {
                feature_code: "xcf.layer".into(),
                source: source("layer", 0, 20, "Owner", &[], None),
                targets: Vec::new(),
                disposition: ConversionDisposition::NativeEquivalent,
                reason: ConversionReason::NativePolicy,
                source_description: String::new(),
                native_description: String::new(),
                attachments: Vec::new(),
            })
            .unwrap();
        for offset in 0..=MAX_CONVERSION_ATTACHMENTS {
            outcomes.property(owner, 999, offset, b"x").unwrap();
        }
        let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("typed report expected") };
        assert_eq!(report.attachments.len(), MAX_CONVERSION_ATTACHMENTS);
        assert_eq!(report.entries.last().unwrap().disposition, ConversionDisposition::SkippedNonessential);
        report.validate().unwrap();
    }
}
