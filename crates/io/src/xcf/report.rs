//! Bounded historical evidence for conversions whose native policy belongs here.

use std::sync::Arc;

use craft_xcf::effects::{self, OwnedEffect};
use photocraft_doc::import_conversion::{
    ConversionDisposition, ConversionEntry, ConversionReason, ConversionReport, ImportConversion, NativeTargetRef, RetainedImportData, SourceItemRef,
};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Rect;
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
    native_context: Option<Document>,
    native_work: usize,
}

struct DecodedEffect<'a> {
    offset: u64,
    effect: Option<OwnedEffect<'a>>,
    reason: ConversionReason,
    note: Option<&'static str>,
}

impl<'a> Report<'a> {
    pub(super) fn new(bytes: &'a [u8], version: u32) -> Self {
        Self {
            reader: effects::Reader::new(bytes, version),
            decode: effects::Budget::default(),
            report: ConversionReport { source_format: "XCF".into(), source_version: Some(version), policy_version: 1, ..Default::default() },
            raw_remaining: MAX_RETAINED_BYTES,
            limited: false,
            native_context: None,
            native_work: super::MAX_PIXEL_BYTES.saturating_mul(8),
        }
    }

    pub(super) fn with_native_context(mut self, document: &Document) -> Self {
        // Only the interpretation required by ordinary native raster contents is
        // inherited. Arbitrary parent metadata/resources are not cloned.
        let mut context = Document::new("Imported tone context", document.size, document.mode, document.depth);
        context.resolution_dpi = document.resolution_dpi;
        context.icc_profile = document.icc_profile.clone();
        self.native_context = Some(context);
        self
    }

    pub(super) fn layer_effects(
        &mut self,
        xcf: &craft_xcf::Xcf<'_>,
        index: usize,
        layer: &mut Layer,
        admission: &mut super::effects::coverage::Admission<'_>,
        warnings: &mut Warnings,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        let Some(source) = xcf.image().layers.get(index).filter(|source| source.effects != 0) else { return Ok(()) };
        let unapplied = |warnings: &mut Warnings, layer: &Layer| {
            warnings.layer(&layer.name, &format!("has {} layer effect(s) (non-destructive filters) that are not applied", source.effects));
        };
        if self.limited {
            unapplied(warnings, layer);
            return Ok(());
        }
        let keep_going = || !ctl.cancelled();
        let inventory = match xcf.layer_effects(index, &mut self.decode, &keep_going) {
            Ok(inventory) => inventory,
            Err(effects::Error::Source(craft_xcf::Error::Cancelled)) => return Err(IoError::Cancelled),
            Err(effects::Error::Source(craft_xcf::Error::Invalid(_))) => {
                unapplied(warnings, layer);
                warnings.layer(&layer.name, "has an invalid XCF effect list; saved pixels were preserved");
                self.record_list_failure(layer, index);
                return Ok(());
            }
            Err(effects::Error::Limit(_)) => {
                self.limited = true;
                unapplied(warnings, layer);
                return Ok(());
            }
        };
        let count = inventory.offsets.len().min(MAX_EFFECT_REPORT_ENTRIES.saturating_sub(self.report.entries.len()));
        let mut records = Vec::new();
        let mut mappings = Vec::new();
        if records.try_reserve_exact(count).is_err() || mappings.try_reserve_exact(count.min(32)).is_err() || self.report.entries.try_reserve(count).is_err() {
            self.limited = true;
            unapplied(warnings, layer);
            return Ok(());
        }
        let mut eligible = count == inventory.offsets.len() && count <= 32;
        if count < inventory.offsets.len() {
            self.limited = true;
        }
        for &offset in inventory.offsets.iter().take(count) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let effect = match self.reader.read(inventory.owner, offset, &mut self.decode, &keep_going) {
                Ok(effect) => Some(effect),
                Err(effects::Error::Source(craft_xcf::Error::Cancelled)) => return Err(IoError::Cancelled),
                Err(effects::Error::Source(craft_xcf::Error::Invalid(_))) => None,
                Err(effects::Error::Limit(_)) => {
                    self.limited = true;
                    eligible = false;
                    break;
                }
            };
            let mut reason = ConversionReason::InvalidOptionalData;
            let mut note = None;
            if let Some(effect) = &effect {
                match super::effects::map(effect, ctl) {
                    Ok(Some(mapping)) => {
                        reason = ConversionReason::NativePolicy;
                        note = mapping.notes.first().copied();
                        if mappings.len() < 32 {
                            mappings.push(mapping);
                        } else {
                            eligible = false;
                        }
                    }
                    Ok(None) => {
                        eligible = false;
                        reason = ConversionReason::MissingMapping;
                    }
                    Err(super::effects::Failure::Cancelled) => return Err(IoError::Cancelled),
                    Err(super::effects::Failure::Invalid(_)) => eligible = false,
                    Err(super::effects::Failure::Unsupported(_) | super::effects::Failure::Range(_)) => {
                        eligible = false;
                        reason = ConversionReason::NativeLimitation;
                    }
                }
            } else {
                eligible = false;
            }
            records.push(DecodedEffect { offset, effect, reason, note });
        }
        eligible &= records.len() == inventory.offsets.len() && mappings.len() == records.len();
        let mut installed = false;
        let mut installed_note = None;
        let mut mask_header = None;
        let mut mask_attempted = false;
        let mut refusal = if self.limited || records.len() > 32 { Some(ConversionReason::RetentionLimit) } else { None };
        if eligible
            && let Some(context) = &self.native_context
            && let Some(authorize) = crate::native_xcf_tones::current()
        {
            let bounds = i32::try_from(source.width).ok().zip(i32::try_from(source.height).ok()).and_then(|(width, height)| {
                source.x.checked_add(width).zip(source.y.checked_add(height)).map(|(x1, y1)| Rect::new(source.x, source.y, x1, y1))
            });
            if let Some(bounds) = bounds {
                // Source lists are top-first: the last stored filter executes
                // first. Evidence retains original wire ordinals below.
                mappings.reverse();
                // Authorize the complete native sequence before any evaluation
                // or cache generation. A later denied command cannot leave an
                // earlier stage evaluated or an owner partly installed.
                for mapping in &mappings {
                    for filter in &mapping.filters {
                        ctl.check().map_err(|_| IoError::Cancelled)?;
                        match authorize(&filter.command, &filter.params) {
                            Ok(()) => {}
                            Err(crate::native_xcf_tones::AuthorizationError::Cancelled) => return Err(IoError::Cancelled),
                            Err(crate::native_xcf_tones::AuthorizationError::Denied) => {
                                return Err(IoError::NativeXcfToneDenied { command: filter.command.clone() });
                            }
                        }
                    }
                }
                let mut remaining = *admission.retained;
                let masked = records.len() == 1
                    && records.first().and_then(|record| record.effect.as_ref()).is_some_and(|effect| effect.record.operation == "gegl:exposure")
                    && mappings.len() == 1
                    && mappings.first().is_some_and(|mapping| mapping.filters.len() == 1)
                    && mappings.first().is_some_and(|mapping| mapping.envelope.mask != super::effects::MaskRequirement::None)
                    && context.mode == photocraft_color::ColorMode::Rgb
                    && matches!(context.depth, photocraft_color::SampleType::U8 | photocraft_color::SampleType::U16)
                    && xcf.image().version == 22;
                let prepared = if masked {
                    mask_attempted = true;
                    let effect = records.first().and_then(|record| record.effect.as_ref()).ok_or_else(|| super::invalid("missing admitted source effect"))?;
                    match super::effects::coverage::decode(xcf, &effect.record, admission, &mut remaining, &mut self.native_work, ctl) {
                        Ok(decoded) => {
                            mask_header = Some(decoded.header);
                            let mut budgets = super::effects::installer::Budgets { retained: &mut remaining, work: &mut self.native_work };
                            let stage = mappings.first().ok_or_else(|| super::invalid("missing admitted native Exposure"))?;
                            super::effects::installer::prepare_masked_exposure(
                                inventory.owner,
                                layer,
                                bounds,
                                context,
                                super::effects::installer::MaskedExposure { stage, coverage: decoded.native },
                                &mut budgets,
                                ctl,
                            )
                        }
                        Err(super::effects::coverage::Failure::Cancelled) => return Err(IoError::Cancelled),
                        Err(error) => {
                            refusal = Some(match error {
                                super::effects::coverage::Failure::Invalid => ConversionReason::InvalidOptionalData,
                                super::effects::coverage::Failure::Unsupported => ConversionReason::NativeLimitation,
                                super::effects::coverage::Failure::Limit => ConversionReason::RetentionLimit,
                                super::effects::coverage::Failure::Resource => ConversionReason::ResourceUnavailable,
                                super::effects::coverage::Failure::Cancelled => return Err(IoError::Cancelled),
                            });
                            Err(super::effects::installer::Refusal::Unsupported("auxiliary coverage was not admitted"))
                        }
                    }
                } else {
                    let mut budgets = super::effects::installer::Budgets { retained: &mut remaining, work: &mut self.native_work };
                    super::effects::installer::prepare(inventory.owner, layer, bounds, context, &mappings, &mut budgets, ctl)
                };
                match prepared {
                    Ok(prepared) if prepared.source_owner == inventory.owner => {
                        ctl.check().map_err(|_| IoError::Cancelled)?;
                        // This is the only publication point; every source stage
                        // and the complete private native bundle/cache succeeded.
                        installed_note = prepared.notes.first().copied();
                        layer.content = LayerContent::Smart(prepared.smart);
                        *admission.retained = remaining;
                        installed = true;
                    }
                    Ok(_) => {
                        refusal = Some(ConversionReason::ResourceUnavailable);
                    }
                    Err(super::effects::installer::Refusal::Cancelled) => return Err(IoError::Cancelled),
                    Err(super::effects::installer::Refusal::Limit) => refusal = Some(ConversionReason::RetentionLimit),
                    Err(super::effects::installer::Refusal::Unsupported(_)) => {
                        if refusal.is_none() {
                            refusal = Some(ConversionReason::NativeLimitation);
                        }
                    }
                    Err(super::effects::installer::Refusal::Preparation(_)) => refusal = Some(ConversionReason::ResourceUnavailable),
                }
            } else {
                refusal = Some(ConversionReason::NativeLimitation);
            }
        }
        if installed {
            warnings.layer(&layer.name, "opens its layer effects as editable native tone filters; native pivots, transfer and integer quantization may differ");
            if let Some(note) = installed_note {
                warnings.layer(&layer.name, note);
            }
        } else {
            unapplied(warnings, layer);
        }
        for (ordinal, record) in records.iter().enumerate() {
            self.record_effect(layer, index, ordinal, record, installed, refusal);
        }
        if mask_attempted {
            let offset = records.first().and_then(|record| record.effect.as_ref()).map_or(0, |effect| effect.record.mask_offset);
            self.record_mask(layer, index, offset, mask_header.as_ref(), installed, refusal);
        }
        Ok(())
    }

    fn record_mask(
        &mut self,
        layer: &Layer,
        index: usize,
        offset: u64,
        header: Option<&craft_xcf::samples::EffectMaskHeader<'_>>,
        installed: bool,
        refusal: Option<ConversionReason>,
    ) {
        if self.report.entries.len() >= MAX_EFFECT_REPORT_ENTRIES || self.report.entries.try_reserve(1).is_err() {
            self.limited = true;
            return;
        }
        let source = SourceItemRef {
            kind: "effect_mask".into(),
            ordinal: 0,
            item_path: vec![index as u32],
            record_offset: Some(offset),
            name: header.map_or_else(|| "Effect mask".into(), |header| bounded_text(&header.channel().name)),
            ..Default::default()
        };
        let mut attachments = Vec::new();
        let mut lost = false;
        if let Some(header) = header {
            if attachments.try_reserve(header.properties().len()).is_ok() {
                for property in header.properties() {
                    if let Some(index) = self.retain_record(&source, "xcf.effect_mask_property", "xcf_property", property.raw) {
                        attachments.push(index);
                    } else {
                        lost = true;
                    }
                }
            } else {
                lost = true;
            }
        }
        self.report.entries.push(ConversionEntry {
            feature_code: "xcf.effect_mask".into(), source,
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: "Native Smart Filter mask".into(), ..Default::default() }],
            disposition: if installed { ConversionDisposition::NativeApproximation } else { ConversionDisposition::SkippedNonessential },
            reason: if lost { ConversionReason::RetentionLimit } else if installed { ConversionReason::NativePolicy } else { refusal.unwrap_or(ConversionReason::NativeLimitation) },
            source_description: header.map_or_else(|| "Unusable source effect coverage".into(), |header| format!("{}x{} image-global source coverage",header.width(),header.height())),
            native_description: if installed { "Independent native F32 Smart Filter coverage; source opacity stays on the native Exposure filter. Channel display properties remain source evidence.".into() }
                else { "Auxiliary coverage was not installed; original owner pixels were preserved.".into() }, attachments,
        });
    }

    fn record_list_failure(&mut self, layer: &Layer, index: usize) {
        if self.report.entries.len() >= MAX_EFFECT_REPORT_ENTRIES || self.report.entries.try_reserve(1).is_err() {
            self.limited = true;
            return;
        }
        self.report.entries.push(ConversionEntry {
            feature_code: "xcf.layer_effects".into(),
            source: SourceItemRef {
                kind: "layer".into(),
                ordinal: index as u32,
                item_path: vec![index as u32],
                name: bounded_text(&layer.name),
                ..Default::default()
            },
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: bounded_text(&layer.name), ..Default::default() }],
            disposition: ConversionDisposition::SkippedNonessential,
            reason: ConversionReason::InvalidOptionalData,
            source_description: "XCF layer effect list".into(),
            native_description: "The effect list could not be inspected. Saved layer pixels were preserved and no native stage was installed.".into(),
            attachments: Vec::new(),
        });
    }

    fn record_effect(&mut self, layer: &Layer, index: usize, ordinal: usize, record: &DecodedEffect<'_>, installed: bool, refusal: Option<ConversionReason>) {
        let effect = record.effect.as_ref();
        let source = SourceItemRef {
            kind: "layer_effect".into(),
            ordinal: ordinal as u32,
            item_path: vec![index as u32],
            record_offset: Some(record.offset),
            name: effect.map_or_else(|| "Effect".into(), |effect| bounded_text(effect.record.name)),
            ..Default::default()
        };
        let retained = effect.and_then(|effect| self.retain_record(&source, "xcf.layer_effect", "xcf_effect_record", effect.record.raw));
        let reason = if effect.is_none() || record.reason == ConversionReason::InvalidOptionalData {
            ConversionReason::InvalidOptionalData
        } else if retained.is_none() {
            ConversionReason::RetentionLimit
        } else if installed {
            ConversionReason::NativePolicy
        } else {
            refusal.unwrap_or(if record.reason == ConversionReason::NativePolicy { ConversionReason::NativeLimitation } else { record.reason })
        };
        let source_description = effect.map_or_else(
            || "Invalid XCF effect record".into(),
            |effect| {
                if effect.record.operation.is_empty() { "XCF layer effect".into() } else { bounded_text(effect.record.operation) }
            },
        );
        let (disposition, native_description) = if installed {
            (
                ConversionDisposition::NativeApproximation,
                format!(
                    "Installed as editable native tone controls. {}{}",
                    record.note.unwrap_or("Native tone behavior applies."),
                    if retained.is_none() {
                        " Original source record exceeded report retention limits."
                    } else {
                        " Original source record retained for inspection."
                    }
                ),
            )
        } else if retained.is_some() {
            (
                ConversionDisposition::RetainedUnapplied,
                "The complete source effect sequence was not installed. Source effect retained for inspection; saved layer pixels were preserved.".into(),
            )
        } else {
            (ConversionDisposition::SkippedNonessential, "Source effect was omitted. Saved layer pixels were preserved.".into())
        };
        self.report.entries.push(ConversionEntry {
            feature_code: "xcf.layer_effect".into(),
            source,
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: bounded_text(&layer.name), ..Default::default() }],
            disposition,
            reason,
            source_description,
            native_description,
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

    pub(super) fn attach(mut self, document: &mut Document, warnings: &mut Warnings) {
        if self.limited {
            // A single durable summary is separate from the optional 4096-entry
            // detail allowance, including refusal before the first record.
            if self.report.entries.len() < photocraft_doc::import_conversion::MAX_CONVERSION_ENTRIES && self.report.entries.try_reserve(1).is_ok() {
                self.report.entries.push(ConversionEntry {
                    feature_code: "xcf.layer_effects.limit".into(),
                    source: SourceItemRef { kind: "image".into(), name: "XCF".into(), ..Default::default() },
                    targets: Vec::new(), disposition: ConversionDisposition::SkippedNonessential, reason: ConversionReason::RetentionLimit,
                    source_description: "Bounded XCF effect inventory".into(),
                    native_description: "Remaining source effects were not applied because their inventory or retention allowance was exhausted; saved owner pixels were preserved.".into(),
                    attachments: Vec::new(),
                });
            }
            warnings.note(
                "the XCF effect inventory exceeded the conversion report limit; remaining effects were not applied and their saved pixels were preserved",
            );
        }
        if !self.report.entries.is_empty() && document.metadata.import_conversion.is_none() {
            match self.report.validate() {
                Ok(()) => document.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(self.report))),
                Err(_) => warnings.note("the XCF conversion report could not retain all source evidence; imported content was preserved"),
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
    #[test]
    fn exhausted_effect_inventory_persists_a_loss_marker_without_claiming_installation() {
        let bytes = with_effects();
        let xcf = craft_xcf::Xcf::parse(&bytes).unwrap();
        let mut document = Document::new("Preserved", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut owner = Layer::raster("Original", document.pixel_format());
        owner.surface_mut().unwrap().write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.]);
        let original = owner.clone();
        let mut report = Report::new(&bytes, 26).with_native_context(&document);
        report.decode = effects::Budget::new(0, 0);
        let mut retained = super::super::MAX_PIXEL_BYTES;
        let mut source_budget = super::super::MAX_PIXEL_BYTES;
        let mut metadata_budget = craft_xcf::records::Budget::default();
        let mut warnings = Warnings::default();
        report
            .layer_effects(
                &xcf,
                0,
                &mut owner,
                &mut super::super::effects::coverage::Admission {
                    retained: &mut retained,
                    source: &mut source_budget,
                    metadata: &mut metadata_budget,
                    options: Default::default(),
                },
                &mut warnings,
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(owner, original);
        document.layers.push(owner);
        report.attach(&mut document, &mut warnings);
        let Some(ImportConversion::Report(evidence)) = document.metadata.import_conversion.as_deref() else {
            panic!("durable effect-limit disclosure missing")
        };
        assert_eq!(evidence.entries.len(), 1);
        assert_eq!(evidence.entries[0].reason, ConversionReason::RetentionLimit);
        assert_eq!(evidence.entries[0].disposition, ConversionDisposition::SkippedNonessential);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        assert_eq!(reopened.layers[0].surface(), original.surface());
    }
}
