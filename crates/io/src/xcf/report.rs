//! Bounded historical evidence for conversions whose native policy belongs here.

use std::sync::Arc;

use craft_xcf::effects::{self, OwnedEffect};
use photocraft_doc::import_conversion::{
    ConversionDisposition, ConversionEntry, ConversionReason, ConversionReport, ImportConversion, NativeTargetRef, RetainedImportData, SourceItemRef,
};
use photocraft_doc::{Document, Layer};
use photocraft_raster::Interrupt;

use super::text_source::{Mapping, TextSource};
use super::{IoError, Result, Warnings};

const MAX_EFFECT_REPORT_ENTRIES: usize = 4_096;
const MAX_RETAINED_RECORD_BYTES: usize = 1 << 20;
const MAX_RETAINED_BYTES: usize = 8 << 20;

pub(super) struct Report<'a> {
    reader: effects::Reader<'a>,
    decode: effects::Budget,
    report: ConversionReport,
    raw_remaining: usize,
    limited: bool,
}

impl<'a> Report<'a> {
    pub(super) fn new(bytes: &'a [u8], version: u32) -> Self {
        Self {
            reader: effects::Reader::new(bytes, version),
            decode: effects::Budget::default(),
            report: ConversionReport { source_format: "XCF".into(), source_version: Some(version), policy_version: 1, ..Default::default() },
            raw_remaining: MAX_RETAINED_BYTES,
            limited: false,
        }
    }

    pub(super) fn layer_effects(&mut self, xcf: &craft_xcf::Xcf<'_>, index: usize, layer: &Layer, warnings: &mut Warnings, ctl: &Interrupt<'_>) -> Result<()> {
        let Some(source) = xcf.image().layers.get(index).filter(|source| source.effects != 0) else { return Ok(()) };
        warnings.layer(&layer.name, &format!("has {} layer effect(s) (non-destructive filters) that are not applied", source.effects));
        if self.limited {
            return Ok(());
        }
        let keep_going = || !ctl.cancelled();
        let inventory = match xcf.layer_effects(index, &mut self.decode, &keep_going) {
            Ok(inventory) => inventory,
            Err(effects::Error::Source(craft_xcf::Error::Cancelled)) => return Err(IoError::Cancelled),
            Err(effects::Error::Source(craft_xcf::Error::Invalid(_))) => {
                warnings.layer(&layer.name, "has an invalid XCF effect list; saved pixels were preserved");
                return Ok(());
            }
            Err(effects::Error::Limit(_)) => {
                self.limited = true;
                return Ok(());
            }
        };
        for (ordinal, offset) in inventory.offsets.into_iter().enumerate() {
            if self.report.entries.len() >= MAX_EFFECT_REPORT_ENTRIES || self.report.entries.try_reserve(1).is_err() {
                self.limited = true;
                break;
            }
            let effect = match self.reader.read(inventory.owner, offset, &mut self.decode, &keep_going) {
                Ok(effect) => Some(effect),
                Err(effects::Error::Source(craft_xcf::Error::Cancelled)) => return Err(IoError::Cancelled),
                Err(effects::Error::Source(craft_xcf::Error::Invalid(_))) => None,
                Err(effects::Error::Limit(_)) => {
                    self.limited = true;
                    break;
                }
            };
            self.record_effect(layer, index, ordinal, offset, effect.as_ref());
        }
        Ok(())
    }

    fn record_effect(&mut self, layer: &Layer, index: usize, ordinal: usize, offset: u64, effect: Option<&OwnedEffect<'_>>) {
        let source = SourceItemRef {
            kind: "layer_effect".into(),
            ordinal: ordinal as u32,
            item_path: vec![index as u32],
            record_offset: Some(offset),
            name: effect.map_or_else(|| "Effect".into(), |effect| bounded_text(effect.record.name)),
            ..Default::default()
        };
        let retained = effect.and_then(|effect| self.retain_record(&source, "xcf.layer_effect", "xcf_effect_record", effect.record.raw));
        let reason = match effect {
            None => ConversionReason::InvalidOptionalData,
            Some(_) if retained.is_none() => ConversionReason::RetentionLimit,
            Some(effect) if !effect.record.errors.is_empty() => ConversionReason::InvalidOptionalData,
            Some(_) => ConversionReason::MissingMapping,
        };
        let source_description = effect.map_or_else(
            || "Invalid XCF effect record".into(),
            |effect| if effect.record.operation.is_empty() { "XCF layer effect".into() } else { bounded_text(effect.record.operation) },
        );
        let (disposition, native_description) = if retained.is_some() {
            (ConversionDisposition::RetainedUnapplied, "Source effect retained for inspection. Saved layer pixels were preserved.")
        } else {
            (ConversionDisposition::SkippedNonessential, "Source effect was omitted. Saved layer pixels were preserved.")
        };
        self.report.entries.push(ConversionEntry {
            feature_code: "xcf.layer_effect".into(),
            source,
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: bounded_text(&layer.name), ..Default::default() }],
            disposition,
            reason,
            source_description,
            native_description: native_description.into(),
            attachments: retained.into_iter().collect(),
        });
    }

    pub(super) fn text(&mut self, index: usize, layer: &Layer, text: &TextSource<'_>, mapping: &Mapping) -> Result<()> {
        use ConversionDisposition as Disposition;
        use ConversionReason as Reason;
        use craft_xcf::records::StopReason;

        let record = text.configuration_bytes();
        let source = SourceItemRef {
            kind: "layer".into(),
            ordinal: index as u32,
            record_offset: record.map(|(offset, _)| offset as u64),
            name: bounded_text(&layer.name),
            ..Default::default()
        };
        let retained = record.and_then(|(_, bytes)| self.retain_record(&source, "xcf.text", "xcf_text_configuration", bytes));
        let (disposition, reason, summary) = match mapping {
            Mapping::Converted { .. } => (
                Disposition::NativeApproximation,
                Reason::NativePolicy,
                "Editable native text keeps the saved raster appearance. Native text layout applies after editing.".into(),
            ),
            Mapping::Missing => (Disposition::SkippedNonessential, Reason::MissingMapping, "The layer was imported as its saved pixels.".into()),
            Mapping::Preserved { reason, source_reason } => {
                let disposition = if retained.is_some() { Disposition::RetainedUnapplied } else { Disposition::SkippedNonessential };
                let policy = match source_reason {
                    Some(StopReason::Malformed) => Reason::InvalidOptionalData,
                    Some(StopReason::Unsupported) | None => Reason::NativeLimitation,
                    Some(_) => Reason::RetentionLimit,
                };
                (disposition, policy, bounded_text(reason))
            }
        };
        self.append(ConversionEntry {
            feature_code: "xcf.text".into(),
            source,
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: bounded_text(&layer.name), ..Default::default() }],
            disposition,
            reason,
            source_description: "XCF text layer".into(),
            native_description: summary,
            attachments: retained.into_iter().collect(),
        })
    }

    fn retain_record(&mut self, owner: &SourceItemRef, kind: &str, encoding: &str, raw: &[u8]) -> Option<u32> {
        use photocraft_doc::import_conversion::MAX_CONVERSION_ATTACHMENTS;
        if raw.is_empty() || raw.len() > MAX_RETAINED_RECORD_BYTES || self.report.attachments.len() >= MAX_CONVERSION_ATTACHMENTS {
            return None;
        }
        let remaining = self.raw_remaining.checked_sub(raw.len())?;
        let index = u32::try_from(self.report.attachments.len()).ok()?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(raw.len()).ok()?;
        self.report.attachments.try_reserve(1).ok()?;
        bytes.extend_from_slice(raw);
        self.report.attachments.push(RetainedImportData { owner: owner.clone(), kind: kind.into(), encoding: encoding.into(), bytes: Arc::new(bytes) });
        self.raw_remaining = remaining;
        Some(index)
    }

    pub(super) fn append(&mut self, entry: ConversionEntry) -> Result<()> {
        self.report.entries.try_reserve(1).map_err(|_| IoError::OutOfMemory("unable to retain XCF conversion evidence".into()))?;
        self.report.entries.push(entry);
        Ok(())
    }

    pub(super) fn attach(self, document: &mut Document, warnings: &mut Warnings) {
        if self.limited {
            warnings.note("the XCF effect inventory exceeded the conversion report limit; saved pixels were preserved");
        }
        if !self.report.entries.is_empty() && document.metadata.import_conversion.is_none() {
            match self.report.validate() {
                Ok(()) => document.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(self.report))),
                Err(_) => warnings.note("the XCF conversion report could not retain all source evidence; saved pixels were preserved"),
            }
        }
    }
}

fn bounded_text(text: &str) -> String {
    text.get(..text.floor_char_boundary(256)).unwrap_or_default().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcf::tests::{W, hierarchy};

    fn with_effects() -> Vec<u8> {
        let mut w = W::new(26);
        for word in [1, 1, 0, 150] {
            w.u32(word);
        }
        w.prop(17, &[0]);
        w.end();
        let layer_slot = w.slot();
        for _ in 0..3 {
            w.ptr(0);
        }
        w.patch(layer_slot, w.out.len());
        for word in [1, 1, 1] {
            w.u32(word);
        }
        w.string("Saved RGB");
        w.end();
        let pixels_slot = w.slot();
        w.ptr(0); // No layer mask.
        let effect_slot = w.slot();
        w.ptr(usize::MAX);
        w.ptr(0);
        let pixels = hierarchy(&mut w, 1, 1, 4, &[18, 52, 86, 255], craft_xcf::Compression::None);
        w.patch(pixels_slot, pixels);
        w.patch(effect_slot, w.out.len());
        for value in ["Brightness", "", "gegl:brightness-contrast", "1:0"] {
            w.string(value);
        }
        w.end();
        w.ptr(0); // No effect mask.
        w.out
    }

    #[test]
    fn valid_and_invalid_effects_keep_pixels_and_persist_ordered_evidence() {
        let source = with_effects();
        let imported = crate::import("effects.xcf", &source).unwrap();
        let doc = imported.document;
        let layer = &doc.layers[0];
        let pixels = layer.surface().unwrap();
        for (component, expected) in [18, 52, 86, 255].into_iter().enumerate() {
            assert_eq!(pixels.sample_channel(0, 0, component), f32::from(expected as u8) / 255.0);
        }
        let Some(ImportConversion::Report(report)) = doc.metadata.import_conversion.as_deref() else { panic!("expected an XCF effect report") };
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].source.ordinal, 0);
        assert_eq!(report.entries[0].source_description, "gegl:brightness-contrast");
        assert_eq!(report.entries[0].targets[0].layer_id, Some(layer.id));
        assert_eq!(report.entries[0].disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(report.entries[1].source.ordinal, 1);
        assert_eq!(report.entries[1].reason, ConversionReason::InvalidOptionalData);
        assert_eq!(report.attachments.len(), 1);
        let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
        assert!(imported.warnings.iter().any(|text| text.contains("2 layer effect(s)")));
    }
}
