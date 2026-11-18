//! Coverage execution is independent of optional report retention. Orphan masks
//! still use header-only inventory; only admitted native owners reach this module.
use super::{
    ConversionReason, CoverageMaskHeader, HashMap, Image, Interrupt, IoError, LayerMask, MAX_DEPTH, MAX_LAYERS, Pixels, Rect, Result, Surface, coverage,
    effects, invalid, native_work,
};
#[derive(Default)]
pub(super) struct CoverageExecution<'a> {
    pub(super) applied: HashMap<usize, std::result::Result<String, effects::MappingFailure>>,
    pub(super) masks: HashMap<usize, CoverageMaskHeader<'a>>,
    pub(super) sample_flags: HashMap<usize, Pixels>,
    active_effects: std::collections::HashSet<usize>,
    active_masks: std::collections::HashSet<usize>,
    visited: usize,
    pub(super) limited: bool,
    pub(super) discarded: bool,
    finished: Vec<usize>,
}
impl CoverageExecution<'_> {
    pub(super) fn discard_since(&mut self, start: usize, work: &native_work::Work, ctl: &Interrupt<'_>) -> Result<()> {
        for pointer in self.finished.get(start..).unwrap_or_default() {
            if let Some(result) = self.applied.get_mut(pointer)
                && let Ok(notes) = result
            {
                let (reason, message) = match work.charge_bytes(notes.len().saturating_add(96), ctl) {
                    Ok(()) => {
                        (ConversionReason::NativePolicy, format!("Temporary native coverage result discarded because its owning stage did not commit. {notes}"))
                    }
                    Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                    Err(_) => {
                        (ConversionReason::RetentionLimit, "Temporary native coverage discarded; further details omitted after generation work limits".into())
                    }
                };
                *result = Err(effects::MappingFailure { reason, message });
            }
        }
        Ok(())
    }
    pub(super) fn discard(&mut self, work: &native_work::Work, ctl: &Interrupt<'_>) -> Result<()> {
        self.discard_since(0, work, ctl)?;
        self.discarded = true;
        Ok(())
    }
}
impl<'a> Image<'a> {
    pub(super) fn execute_coverage(
        &mut self,
        owner: usize,
        pointers: &[usize],
        surface: &mut Surface,
        bounds: Rect,
        source_offset: usize,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        let state = self.run_coverage(pointers, surface, bounds, source_offset, ctl)?;
        self.report_coverage_effects(owner, pointers, Some(&state), ctl)
    }
    pub(super) fn run_coverage(
        &mut self,
        pointers: &[usize],
        surface: &mut Surface,
        bounds: Rect,
        source_offset: usize,
        ctl: &Interrupt<'_>,
    ) -> Result<CoverageExecution<'a>> {
        let mut state = CoverageExecution::default();
        state.active_masks.insert(source_offset);
        self.apply_coverage_stack(pointers, surface, bounds, 0, &mut state, ctl)?;
        Ok(state)
    }
    fn apply_coverage_stack(
        &mut self,
        pointers: &[usize],
        surface: &mut Surface,
        bounds: Rect,
        depth: usize,
        state: &mut CoverageExecution<'a>,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        let mut omitted = false;
        for &pointer in pointers.iter().rev() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if state.visited >= MAX_LAYERS {
                state.limited = true;
                break;
            }
            if depth > MAX_DEPTH || !state.active_effects.insert(pointer) {
                state.applied.insert(
                    pointer,
                    Err(effects::MappingFailure {
                        reason: ConversionReason::InvalidOptionalData,
                        message: "Cyclic or excessively nested source coverage operation was not followed".into(),
                    }),
                );
                omitted = true;
                continue;
            }
            let Some(left) = self.meta_budget.checked_sub(512) else {
                state.limited = true;
                break;
            };
            self.meta_budget = left;
            state.visited += 1;
            let metadata_before = self.meta_budget;
            let decoded = effects::decode_with_work(&self.c, pointer, self.version, &mut self.meta_budget, &mut self.effect_decode_work, ctl);
            let effect = match decoded {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
                    self.meta_budget = metadata_before;
                    let reason =
                        if matches!(error, IoError::XcfMetadataLimit) { ConversionReason::RetentionLimit } else { ConversionReason::InvalidOptionalData };
                    state
                        .applied
                        .insert(pointer, Err(effects::MappingFailure { reason, message: format!("Optional coverage framing was not applied: {error}") }));
                    state.active_effects.remove(&pointer);
                    omitted = true;
                    continue;
                }
            };
            let own_configuration = metadata_before.saturating_sub(self.meta_budget);
            if !effect.visible || effect.opacity == 0. {
                self.meta_budget = self.meta_budget.saturating_add(own_configuration);
                state.active_effects.remove(&pointer);
                continue;
            }
            let mapped = coverage::prepare(
                surface,
                bounds,
                &effect,
                self.precision.linear,
                &mut coverage::Budgets { workspace: &mut self.native_source_budget, metadata: &mut self.meta_budget, work: &self.native_work },
                ctl,
            )?;
            let mapping = match mapped {
                Ok(mapping) => mapping,
                Err(failure) => {
                    self.meta_budget = self.meta_budget.saturating_add(own_configuration);
                    state.applied.insert(pointer, Err(failure));
                    state.finished.push(pointer);
                    state.active_effects.remove(&pointer);
                    omitted = true;
                    continue;
                }
            };
            let child_start = state.finished.len();
            let auxiliary = if effect.mask_offset == 0 { Ok(None) } else { self.execution_mask(effect.mask_offset, depth + 1, state, ctl).map(Some) };
            let result = match auxiliary {
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => Err(effects::MappingFailure {
                    reason: if matches!(error, IoError::XcfMetadataLimit | IoError::XcfNativeWorkLimit | IoError::XcfNativeWorkUnpriced(_)) {
                        ConversionReason::RetentionLimit
                    } else {
                        ConversionReason::InvalidOptionalData
                    },
                    message: format!("Source auxiliary coverage could not be prepared; stage inactive: {error}"),
                }),
                Ok(mask) => {
                    let result = coverage::apply_prepared(
                        surface,
                        bounds,
                        &effect,
                        mask.as_ref().map(|(mask, _)| mask),
                        mapping,
                        &mut coverage::Budgets { workspace: &mut self.native_source_budget, metadata: &mut self.meta_budget, work: &self.native_work },
                        ctl,
                    );
                    // Auxiliary coverage has been consumed, not retained as a native owner.
                    if let Some((_, own_charge)) = mask {
                        self.budget = self.budget.saturating_add(own_charge);
                    }
                    match result {
                        Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                        Err(error) => Err(effects::MappingFailure {
                            reason: if matches!(error, IoError::XcfNativeWorkLimit | IoError::XcfNativeWorkUnpriced(_) | IoError::XcfMetadataLimit) {
                                ConversionReason::RetentionLimit
                            } else {
                                ConversionReason::InvalidOptionalData
                            },
                            message: format!("Optional native coverage stage was not applied: {error}"),
                        }),
                        Ok(result) => result,
                    }
                }
            };
            match result {
                Ok((converted, mut notes)) => {
                    if omitted {
                        notes.push_str("; An earlier source coverage stage was omitted; this stage receives preserved native coverage instead of the complete source pipeline");
                    }
                    *surface = converted;
                    if let Some(left) = self.meta_budget.checked_sub(notes.len()) {
                        self.meta_budget = left;
                    } else {
                        notes = "Native coverage applied; further native details omitted after metadata retention limits".into();
                        state.limited = true;
                    }
                    state.applied.insert(pointer, Ok(notes));
                }
                Err(failure) => {
                    // Child pixels are temporary until the complete parent stage commits.
                    // Both preparation errors and native mapping refusals discard them;
                    // generation work remains spent even though storage is released.
                    state.discard_since(child_start, &self.native_work, ctl)?;
                    state.applied.insert(pointer, Err(failure));
                    omitted = true;
                }
            }
            state.finished.push(pointer);
            // Decoded operation configuration is temporary; reporting independently
            // admits retained configuration. Its file-wide parse work is never refunded.
            self.meta_budget = self.meta_budget.saturating_add(own_configuration);
            state.active_effects.remove(&pointer);
        }
        Ok(())
    }
    fn execution_mask(&mut self, offset: usize, depth: usize, state: &mut CoverageExecution<'a>, ctl: &Interrupt<'_>) -> Result<(LayerMask, usize)> {
        if depth > MAX_DEPTH || !state.active_masks.insert(offset) {
            return Err(invalid("cyclic or excessively nested auxiliary coverage"));
        }
        let mut charge = 0;
        let result = (|| {
            let mut prepared = self.native_effect_mask(offset, Rect::new(0, 0, 0, 0), self.precision.depth(), ctl)?;
            charge = prepared.pixel_charge;
            let bounds = Rect::new(0, 0, prepared.record.width as i32, prepared.record.height as i32);
            self.apply_coverage_stack(&prepared.record.children, &mut prepared.mask.surface, bounds, depth, state, ctl)?;
            state.masks.insert(offset, prepared.record);
            state.sample_flags.insert(offset, prepared.sample_flags);
            Ok((prepared.mask, charge))
        })();
        if result.is_err() {
            self.budget = self.budget.saturating_add(charge);
        }
        state.active_masks.remove(&offset);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::W;
    use super::super::{
        Arc, ColorMode, Compression, ConversionDisposition, ConversionEntry, Document, Layer, LayerContent, MAX_PIXEL_BYTES, PixelFormat, SampleType, Size,
        conversion,
    };
    use super::*;
    fn source() -> (Vec<u8>, usize) {
        let mut w = W { out: vec![0; 4], wide: true };
        let pointer = w.out.len();
        for value in ["Exposure", "", "gegl:exposure", "1:0"] {
            w.string(value);
        }
        let mut argument = W { out: Vec::new(), wide: true };
        argument.string("exposure");
        argument.u32(3);
        argument.out.extend_from_slice(&1f32.to_be_bytes());
        w.prop(45, &argument.out);
        w.end();
        w.ptr(0);
        (w.out, pointer)
    }
    fn owner(image: &mut Image<'_>) -> usize {
        image
            .outcomes
            .push(ConversionEntry {
                feature_code: "xcf.channel".into(),
                source: conversion::source("channel", 0, 1, "Coverage", &[], None),
                targets: Vec::new(),
                disposition: ConversionDisposition::NativeEquivalent,
                reason: ConversionReason::NativePolicy,
                source_description: "Coverage".into(),
                native_description: "Native coverage".into(),
                attachments: Vec::new(),
            })
            .unwrap()
    }
    #[test]
    fn admitted_coverage_executes_after_report_truncation_and_survives_native_roundtrip() {
        let (bytes, pointer) = source();
        let mut image = super::super::optional_mask_budget_tests::image(&bytes, MAX_PIXEL_BYTES);
        let owner = owner(&mut image);
        image
            .outcomes
            .detail(
                owner,
                "xcf.optional_large_detail",
                ConversionReason::MissingMapping,
                "x".repeat(photocraft_doc::import_conversion::MAX_CONVERSION_STRING + 1),
                "Inert optional detail".into(),
            )
            .unwrap();
        assert!(!image.outcomes.recording());
        let bounds = Rect::new(0, 0, 1, 1);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut surface = Surface::new(format);
        surface.fill_rect(bounds, &[0.25]);
        let mut native = Document::new("Independent native control", Size::new(1, 1), ColorMode::Grayscale, SampleType::F32);
        native.layers.push(Layer::new("Original", LayerContent::Raster(surface.clone())));
        native
            .layers
            .push(Layer::new("Native exposure", LayerContent::Adjustment(photocraft_doc::Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. })));
        let expected = photocraft_compose::flatten_to_surface(&native, format, None);
        image.execute_coverage(owner, &[pointer], &mut surface, bounds, 1, &Interrupt::NONE).unwrap();
        assert_eq!(surface.read_region(bounds), expected.read_region(bounds));
        assert_ne!(surface.read_region(bounds), vec![0.25]);
        native.layers = vec![Layer::new("Retained coverage", LayerContent::Raster(surface))];
        let encoded = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&encoded).unwrap();
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(bounds), expected.read_region(bounds));
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.report_truncated"));
    }
    #[test]
    fn unsupported_parent_keeps_auxiliary_hierarchy_inert_before_decode() {
        let (mut bytes, pointer) = source();
        let operation = bytes.windows(b"gegl:exposure".len()).position(|window| window == b"gegl:exposure").unwrap();
        bytes[operation..operation + b"test:unmapped".len()].copy_from_slice(b"test:unmapped");
        let mask_pointer = bytes.len() - 8;
        let mut w = W { out: bytes, wide: true };
        let mask = w.out.len();
        w.patch(mask_pointer, mask);
        w.u32(1);
        w.u32(1);
        w.string("Inert auxiliary");
        w.end();
        w.ptr(usize::MAX); // Must never be followed by an unsupported parent.
        w.ptr(0);
        let mut image = super::super::optional_mask_budget_tests::image(&w.out, MAX_PIXEL_BYTES);
        let owner = owner(&mut image);
        let before = image.native_work.remaining_bytes();
        let bounds = Rect::new(0, 0, 1, 1);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        surface.fill_rect(bounds, &[0.25]);
        image.execute_coverage(owner, &[pointer], &mut surface, bounds, 1, &Interrupt::NONE).unwrap();
        assert_eq!(surface.read_region(bounds), vec![0.25]);
        assert_eq!(image.budget, MAX_PIXEL_BYTES);
        assert_eq!(before - image.native_work.remaining_bytes(), 260, "only one source-component admission plus its fixed header, no auxiliary decode");
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
        let stage = report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(stage.reason, ConversionReason::MissingMapping);
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect_mask" && entry.reason == ConversionReason::NativePolicy));
    }
    #[test]
    fn rejected_parent_blend_discards_temporary_child_coverage_and_preserves_native_pixels() {
        let (bytes, parent) = source();
        let mut w = W { out: bytes, wide: true };
        // Insert the parent's unsupported Behind blend before its end marker.
        let mut property = W { out: Vec::new(), wide: true };
        property.prop_u32(7, 2);
        let end = w.out.len() - 16;
        w.out.splice(end..end, property.out);
        let parent_mask = w.out.len() - 8;
        let mask = w.out.len();
        w.patch(parent_mask, mask);
        w.u32(1);
        w.u32(1);
        w.string("Temporary auxiliary");
        w.end();
        let hierarchy_slot = w.slot();
        let child_slot = w.slot();
        w.ptr(0);
        let hierarchy = super::super::tests::hierarchy(&mut w, 1, 1, 1, &[64], Compression::None);
        w.patch(hierarchy_slot, hierarchy);
        let child = w.out.len();
        w.patch(child_slot, child);
        let (child_bytes, child_start) = source();
        w.out.extend_from_slice(&child_bytes[child_start..]);

        let mut image = super::super::optional_mask_budget_tests::image(&w.out, MAX_PIXEL_BYTES);
        let owner = owner(&mut image);
        let bounds = Rect::new(0, 0, 1, 1);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mut surface = Surface::new(format);
        surface.fill_rect(bounds, &[0.25]);
        let original = surface.clone();
        let work_before = image.native_work.remaining_bytes();
        let state = image.run_coverage(&[parent], &mut surface, bounds, 1, &Interrupt::NONE).unwrap();
        assert_eq!(surface.read_region(bounds), original.read_region(bounds));
        assert!(state.applied[&parent].as_ref().unwrap_err().message.contains("blend mode"));
        assert!(state.applied[&child].as_ref().unwrap_err().message.contains("discarded"));
        assert!(image.native_work.remaining_bytes() < work_before, "discarded child generation is not refunded");
        assert_eq!(image.budget, MAX_PIXEL_BYTES, "temporary auxiliary pixels are released");
        image.report_coverage_effects(owner, &[parent], Some(&state), &Interrupt::NONE).unwrap();
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
        for pointer in [parent, child] {
            let entry =
                report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect" && entry.source.record_offset == Some(pointer as u64)).unwrap();
            assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
        }
        let mut document = Document::new("Rejected coverage", Size::new(1, 1), ColorMode::Grayscale, SampleType::U8);
        document.layers.push(Layer::new("Preserved pixels", LayerContent::Raster(surface)));
        document.metadata.import_conversion = Some(Arc::new(photocraft_doc::import_conversion::ImportConversion::Report(report)));
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, document);
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(bounds), original.read_region(bounds));
    }
    #[test]
    fn coverage_work_refusal_preserves_pixels_and_cancellation_precedes_all_work() {
        let (bytes, pointer) = source();
        let mut image = super::super::optional_mask_budget_tests::image(&bytes, MAX_PIXEL_BYTES);
        let owner = owner(&mut image);
        image.native_work = native_work::Work::with_units(0);
        let bounds = Rect::new(0, 0, 1, 1);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        surface.fill_rect(bounds, &[0.25]);
        image.execute_coverage(owner, &[pointer], &mut surface, bounds, 1, &Interrupt::NONE).unwrap();
        assert_eq!(surface.read_region(bounds), vec![0.25]);
        assert!(image.native_work.rejections() > 0);
        let before = (image.budget, image.native_source_budget, image.meta_budget, image.native_work.remaining_bytes());
        assert!(matches!(image.execute_coverage(owner, &[pointer], &mut surface, bounds, 1, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
        assert_eq!((image.budget, image.native_source_budget, image.meta_budget, image.native_work.remaining_bytes()), before);
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect" && entry.reason == ConversionReason::RetentionLimit));
    }
}
