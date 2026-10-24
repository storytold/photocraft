//! Independent documented XCF26 framing, not privately published corpus data.

use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

#[derive(Clone, Copy)]
enum Owner {
    Channel,
    Selection,
    LayerMask,
}

fn channel(w: &mut W, name: &str, selection: bool) -> (usize, usize) {
    channel_with_placement(w, name, selection, false)
}
fn channel_with_placement(w: &mut W, name: &str, selection: bool, placed: bool) -> (usize, usize) {
    let start = w.out.len();
    w.u32(2);
    w.u32(1);
    w.string(name);
    if selection {
        w.prop(4, &[]);
    }
    if placed {
        w.prop(15, &[1i32.to_be_bytes(), 0i32.to_be_bytes()].concat());
    }
    w.end();
    let hierarchy_slot = w.slot();
    let effect_slot = w.slot();
    w.ptr(0);
    let pixels = hierarchy(w, 2, 1, 1, &[64, 192], Compression::None);
    w.patch(hierarchy_slot, pixels);
    (start, effect_slot)
}

fn effect(w: &mut W) -> (usize, usize) {
    let start = w.out.len();
    w.string("Coverage stage");
    w.string("inert-icon");
    w.string("gegl:exposure");
    w.string("1:0");
    w.prop_u32(8, 0);
    w.prop(33, &0.25f32.to_be_bytes());
    let mut argument = W { out: Vec::new(), wide: true };
    argument.string("exposure");
    argument.u32(3);
    argument.out.extend_from_slice(&1.25f32.to_be_bytes());
    w.prop(45, &argument.out);
    w.prop(9001, &[3, 5, 7]);
    w.end();
    let mask = w.slot();
    (start, mask)
}

fn source(owner: Owner, nested: bool, cyclic: bool) -> Vec<u8> {
    source_with_offset(owner, nested, cyclic, (0, 0))
}
fn source_with_offset(owner: Owner, nested: bool, cyclic: bool, offset: (i32, i32)) -> Vec<u8> {
    source_with_auxiliary_placement(owner, nested, cyclic, offset, false)
}
fn source_with_auxiliary_placement(owner: Owner, nested: bool, cyclic: bool, offset: (i32, i32), placed: bool) -> Vec<u8> {
    let mut w = W::new(26);
    w.u32(2);
    w.u32(1);
    w.u32(0);
    w.u32(150);
    w.prop(17, &[0]);
    w.end();
    let owner_slot = w.slot();
    let sibling_slot = w.slot();
    w.ptr(0);
    let channel_slot = if matches!(owner, Owner::LayerMask) { None } else { Some(w.slot()) };
    w.ptr(0);
    w.ptr(0);
    let mut layer_mask_slot = None;
    for (name, slot, color) in [("Owner", owner_slot, [20, 40, 60, 255]), ("Sibling", sibling_slot, [170, 190, 210, 255])] {
        let at = w.out.len();
        w.patch(slot, at);
        w.u32(2);
        w.u32(1);
        w.u32(1);
        w.string(name);
        if name == "Owner" && offset != (0, 0) {
            w.prop(15, &[offset.0.to_be_bytes(), offset.1.to_be_bytes()].concat());
        }
        w.end();
        let pixels = w.slot();
        let mask = w.slot();
        w.ptr(0);
        let hierarchy = hierarchy(&mut w, 2, 1, 4, &[color, color].concat(), Compression::None);
        w.patch(pixels, hierarchy);
        if name == "Owner" {
            layer_mask_slot = Some(mask);
        }
    }
    let (coverage, slot) = channel(&mut w, "Coverage", matches!(owner, Owner::Selection));
    if let Some(channel_slot) = channel_slot {
        w.patch(channel_slot, coverage);
    } else {
        w.patch(layer_mask_slot.unwrap(), coverage);
    }
    let (stored, mask_slot) = effect(&mut w);
    w.patch(slot, stored);
    if cyclic {
        w.patch(mask_slot, coverage);
    } else if nested {
        let (nested_mask, nested_slot) = channel_with_placement(&mut w, "Nested effect mask", false, placed);
        w.patch(mask_slot, nested_mask);
        let (nested_effect, _) = effect(&mut w);
        w.patch(nested_slot, nested_effect);
    }
    w.out
}

fn report(document: &Document) -> &photocraft_doc::import_conversion::ConversionReport {
    match document.metadata.import_conversion.as_ref().unwrap() {
        ImportConversion::Report(report) => report,
        _ => panic!("known report"),
    }
}

#[test]
fn v26_channels_selection_and_layer_masks_keep_native_coverage_and_inert_effect_data() {
    for owner in [Owner::Channel, Owner::Selection, Owner::LayerMask] {
        let result = import("coverage.xcf", &source(owner, false, false), &Interrupt::NONE).unwrap();
        let document = &result.document;
        let coverage = match owner {
            Owner::Channel => &document.channels[0].surface,
            Owner::Selection => document.selection.as_ref().unwrap(),
            Owner::LayerMask => &document.layers.iter().find(|layer| layer.name == "Owner").unwrap().mask.as_ref().unwrap().surface,
        };
        assert_eq!(coverage.read_region(Rect::new(0, 0, 2, 1)), vec![64.0 / 255.0, 192.0 / 255.0]);
        let sibling = document.layers.iter().find(|layer| layer.name == "Sibling").unwrap();
        assert_eq!(sibling.surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![170.0 / 255.0, 190.0 / 255.0, 210.0 / 255.0, 1.0]);
        let report = report(document);
        let stage = report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(stage.reason, ConversionReason::MissingMapping);
        assert_eq!(stage.disposition, ConversionDisposition::RetainedUnapplied);
        assert!(stage.source_description.contains("gegl:exposure version 1:0"));
        assert!(stage.source_description.contains("visible false, opacity 0.25"));
        assert!(stage.native_description.contains("unfinished"));
        assert!(report.attachments.iter().any(|attachment| attachment.bytes.windows(13).any(|bytes| bytes == b"gegl:exposure")));
        let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(&reopened.metadata.import_conversion, &document.metadata.import_conversion);
    }
}

#[test]
fn v26_nested_effect_mask_envelopes_have_their_own_source_identity() {
    let document = import("nested.xcf", &source(Owner::Channel, true, false), &Interrupt::NONE).unwrap().document;
    let report = report(&document);
    let stages: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").collect();
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0].source.kind, "channel_effect");
    assert_eq!(stages[1].source.kind, "coverage_effect_mask_effect");
    assert!(stages.iter().all(|entry| entry.reason == ConversionReason::MissingMapping));
}

#[test]
fn v26_reused_mask_references_are_bounded_and_do_not_change_coverage() {
    let document = import("cyclic.xcf", &source(Owner::Selection, false, true), &Interrupt::NONE).unwrap().document;
    assert_eq!(document.selection.as_ref().unwrap().read_region(Rect::new(0, 0, 2, 1)), vec![64.0 / 255.0, 192.0 / 255.0]);
    assert_eq!(report(&document).entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").count(), 1);
}

#[test]
fn malformed_optional_coverage_effect_mask_does_not_discard_the_native_owner() {
    let mut bytes = source(Owner::Channel, true, false);
    let needle = b"Nested effect mask";
    let name = bytes.windows(needle.len()).position(|value| value == needle).unwrap();
    // Independent writer above puts width/height and the string length before
    // this name. Corrupt only the optional nested mask's declared width.
    bytes[name - 12..name - 8].copy_from_slice(&0u32.to_be_bytes());
    let document = import("bad-optional-mask.xcf", &bytes, &Interrupt::NONE).unwrap().document;
    assert_eq!(document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![64.0 / 255.0, 192.0 / 255.0]);
    assert!(
        report(&document).entries.iter().any(|entry| entry.feature_code == "xcf.coverage_effect_mask" && entry.reason == ConversionReason::InvalidOptionalData)
    );
}

#[test]
fn enabled_coverage_effects_translate_to_native_pixels_for_each_owner() {
    for owner in [Owner::Channel, Owner::Selection, Owner::LayerMask] {
        let mut bytes = source(owner, false, false);
        let visibility = [0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0];
        let offset = bytes.windows(12).position(|value| value == visibility).unwrap();
        bytes[offset + 11] = 1;
        let result = import("enabled-coverage.xcf", &bytes, &Interrupt::NONE).unwrap();
        let document = &result.document;
        let coverage = match owner {
            Owner::Channel => &document.channels[0].surface,
            Owner::Selection => document.selection.as_ref().unwrap(),
            Owner::LayerMask => &document.layers.iter().find(|layer| layer.name == "Owner").unwrap().mask.as_ref().unwrap().surface,
        };
        let samples = coverage.read_region(Rect::new(0, 0, 2, 1));
        assert!(samples[0] > 64.0 / 255.0);
        assert!(samples[1] > 192.0 / 255.0);
        let stage = report(document).entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(stage.disposition, ConversionDisposition::NativeApproximation);
        assert!(stage.native_description.contains("parameter editability is lost"));
        assert!(report(document).attachments.iter().any(|attachment| attachment.owner.name == "Coverage stage" && !attachment.bytes.is_empty()));
        let sibling = document.layers.iter().find(|layer| layer.name == "Sibling").unwrap();
        if let LayerContent::Raster(surface) = &sibling.content {
            assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), vec![170.0 / 255.0, 190.0 / 255.0, 210.0 / 255.0, 1.]);
        } else {
            panic!("native sibling raster");
        }
    }
}

fn enable_all_effects(bytes: &mut [u8]) {
    let flag = [0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0];
    let offsets: Vec<_> = bytes.windows(flag.len()).enumerate().filter_map(|(i, value)| (value == flag).then_some(i)).collect();
    for offset in offsets {
        bytes[offset + 11] = 1;
    }
}

/// Independently construct a normal native Gray adjustment layer and native mask.
/// No importer/helper generates the expected coverage.
fn native_exposure_counterpart(surface: &Surface, bounds: Rect, mask: Option<&Surface>) -> Surface {
    let shift = (-bounds.x0, -bounds.y0);
    let mut document = Document::new("Independent native coverage", Size::new(bounds.width(), bounds.height()), ColorMode::Grayscale, surface.format().sample);
    document.layers.push(Layer::new("Original native coverage", LayerContent::Raster(photocraft_algo::resample::translate_surface(surface, shift.0, shift.1))));
    let mut adjustment =
        Layer::new("Native Exposure", LayerContent::Adjustment(photocraft_doc::Adjustment::Exposure { exposure: 1.25, offset: 0., gamma: 1. }));
    adjustment.opacity = 0.25;
    adjustment.mask = mask.map(|mask| LayerMask { surface: photocraft_algo::resample::translate_surface(mask, shift.0, shift.1), ..LayerMask::reveal_all() });
    document.layers.push(adjustment);
    let local = photocraft_compose::flatten_to_surface(&document, surface.format(), None);
    photocraft_algo::resample::translate_surface(&local, bounds.x0, bounds.y0)
}

#[test]
fn enabled_nested_auxiliary_masks_match_independent_native_gray_adjustments_and_mask_placement() {
    for (owner, offset) in [(Owner::Channel, (0, 0)), (Owner::Selection, (0, 0)), (Owner::LayerMask, (-1, 0))] {
        let mut bytes = source_with_offset(owner, true, false, offset);
        enable_all_effects(&mut bytes);
        let document = import("enabled-nested-mask.xcf", &bytes, &Interrupt::NONE).unwrap().document;
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mut original = Surface::new(fmt);
        let bounds = Rect::new(offset.0, offset.1, offset.0 + 2, offset.1 + 1);
        original.write_region(bounds, &[64. / 255., 192. / 255.]);
        // Auxiliary source pixels are image-global even when their parent mask is off-canvas.
        let mut auxiliary = Surface::new(fmt);
        auxiliary.write_region(Rect::new(0, 0, 2, 1), &[64. / 255., 192. / 255.]);
        let converted_auxiliary = native_exposure_counterpart(&auxiliary, Rect::new(0, 0, 2, 1), None);
        let expected = native_exposure_counterpart(&original, bounds, Some(&converted_auxiliary));
        let actual = match owner {
            Owner::Channel => &document.channels[0].surface,
            Owner::Selection => document.selection.as_ref().unwrap(),
            Owner::LayerMask => &document.layers.iter().find(|layer| layer.name == "Owner").unwrap().mask.as_ref().unwrap().surface,
        };
        assert!(
            actual.read_region(bounds).iter().zip(expected.read_region(bounds)).all(|(a, b)| (a - b).abs() <= 1. / 255.),
            "offset{offset:?}: actual{:?} expected{:?}",
            actual.read_region(bounds),
            expected.read_region(bounds)
        );
        if offset.0 < 0 {
            assert_eq!(actual.pixel(-1, 0)[0], 64. / 255., "image-global auxiliary must not move with owner");
        }
        let stages: Vec<_> = report(&document).entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").collect();
        assert_eq!(stages.len(), 2);
        assert!(stages.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
        assert!(
            report(&document)
                .entries
                .iter()
                .any(|entry| entry.feature_code == "xcf.coverage_effect_mask" && entry.disposition == ConversionDisposition::NativeApproximation)
        );
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        drop(bytes);
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, document);
    }
}

#[test]
fn enabled_cyclic_auxiliary_reference_is_retained_and_preserves_owner_coverage() {
    let mut bytes = source(Owner::Selection, false, true);
    enable_all_effects(&mut bytes);
    let document = import("enabled-cycle.xcf", &bytes, &Interrupt::NONE).unwrap().document;
    assert_eq!(document.selection.as_ref().unwrap().read_region(Rect::new(0, 0, 2, 1)), vec![64. / 255., 192. / 255.]);
    let stage = report(&document).entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
    assert_eq!(stage.disposition, ConversionDisposition::RetainedUnapplied);
    assert_eq!(stage.reason, ConversionReason::InvalidOptionalData);
    assert!(stage.native_description.contains("cyclic"));
    assert!(matches!(import("cancelled-cycle.xcf", &bytes, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
}

#[test]
fn active_malformed_or_unestablished_auxiliary_placement_preserves_native_coverage() {
    for malformed in [false, true] {
        let mut bytes = source_with_auxiliary_placement(Owner::Channel, true, false, (0, 0), !malformed);
        enable_all_effects(&mut bytes);
        if malformed {
            let needle = b"Nested effect mask";
            let name = bytes.windows(needle.len()).position(|value| value == needle).unwrap();
            bytes[name - 12..name - 8].copy_from_slice(&0u32.to_be_bytes());
        }
        let document = import("active-invalid-auxiliary.xcf", &bytes, &Interrupt::NONE).unwrap().document;
        assert_eq!(document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![64. / 255., 192. / 255.]);
        let stage = report(&document).entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(stage.disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(stage.reason, if malformed { ConversionReason::InvalidOptionalData } else { ConversionReason::MissingMapping });
        assert!(report(&document).attachments.iter().any(|attachment| !attachment.bytes.is_empty()));
    }
}

#[test]
fn omitted_coverage_stage_preserves_later_native_stage_order_and_reports_downstream_effect() {
    let mut w = W::new(26);
    for value in [2, 1, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    w.ptr(0); // no pixel layers; the independent channel remains editable
    let channel = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = w.out.len();
    w.patch(channel, at);
    w.u32(2);
    w.u32(1);
    w.string("Ordered coverage");
    w.end();
    let pixels = w.slot();
    let slots: Vec<_> = (0..3).map(|_| w.slot()).collect();
    w.ptr(0);
    let data = hierarchy(&mut w, 2, 1, 1, &[64, 192], Compression::None);
    w.patch(pixels, data);
    // Stored top-first: the execution order is Invert, unknown, Invert.
    for (slot, operation) in slots.into_iter().zip(["gegl:invert-gamma", "source:unmapped-coverage", "gegl:invert-gamma"]) {
        let at = w.out.len();
        w.patch(slot, at);
        w.string(operation);
        w.string("");
        w.string(operation);
        w.string("0:0");
        w.end();
        w.ptr(0);
    }
    let document = import("ordered-coverage.xcf", &w.out, &Interrupt::NONE).unwrap().document;
    assert_eq!(document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![64. / 255., 192. / 255.]);
    let stages: Vec<_> = report(&document).entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").collect();
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[1].disposition, ConversionDisposition::RetainedUnapplied);
    assert!(stages.iter().any(|entry| entry.disposition == ConversionDisposition::NativeApproximation
        && entry.native_description.contains("earlier source coverage stage was omitted")));
}

fn layer_effect_with_nested_mask(operation: &str) -> Vec<u8> {
    let mut w = W::new(26);
    for value in [2, 1, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    w.ptr(0);
    let at = w.out.len();
    w.patch(layer, at);
    for value in [2, 1, 1] {
        w.u32(value);
    }
    w.string("Original layer");
    w.end();
    let pixels = w.slot();
    w.ptr(0);
    let parent = w.slot();
    w.ptr(0);
    let hierarchy = hierarchy(&mut w, 2, 1, 4, &[20, 40, 60, 255, 170, 190, 210, 255], Compression::None);
    w.patch(pixels, hierarchy);
    let at = w.out.len();
    w.patch(parent, at);
    w.string("Original parent operation");
    w.string("");
    w.string(operation);
    w.string("0:0");
    w.end();
    let mask = w.slot();
    let at = w.out.len();
    w.patch(mask, at);
    w.u32(2);
    w.u32(1);
    w.string("Original effect mask");
    w.prop(9002, &[2, 4, 6]);
    w.end();
    let pixels = w.slot();
    let child = w.slot();
    w.ptr(0);
    let hierarchy = super::tests::hierarchy(&mut w, 2, 1, 1, &[64, 192], Compression::None);
    w.patch(pixels, hierarchy);
    let (at, _) = effect(&mut w);
    w.patch(child, at);
    enable_all_effects(&mut w.out);
    w.out
}

#[test]
fn temporary_layer_effect_mask_results_are_only_reported_applied_after_parent_commits() {
    for (operation, committed) in [("gegl:invert-gamma", true), ("gegl:bevel", false)] {
        // Bevel becomes a native style; its separate source effect mask is
        // currently unsupported. The mask's Exposure must not claim output use.
        let bytes = layer_effect_with_nested_mask(operation);
        let document = import("deferred-effect-mask.xcf", &bytes, &Interrupt::NONE).unwrap().document;
        let rows = report(&document);
        let top = rows.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect_mask").unwrap();
        assert_eq!(top.source.name, "Original effect mask");
        assert_eq!(top.disposition, if committed { ConversionDisposition::NativeEquivalent } else { ConversionDisposition::RetainedUnapplied });
        let child = rows.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(child.source.kind, "layer_effect_mask_effect");
        assert_eq!(child.disposition, if committed { ConversionDisposition::NativeApproximation } else { ConversionDisposition::RetainedUnapplied });
        if !committed {
            assert_eq!(child.reason, ConversionReason::NativePolicy);
            assert!(child.native_description.contains("discarded"));
            assert!(matches!(document.layers[0].content, LayerContent::Raster(_)));
        }
        assert!(rows.entries.iter().any(|entry| entry.feature_code == "xcf.source_property.9002" && entry.source.kind == "layer_effect_mask"));
        assert!(rows.attachments.iter().any(|attachment| attachment.owner.kind == "layer_effect_mask" && attachment.bytes.as_slice() == [2, 4, 6]));
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        drop(bytes);
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), document);
    }
}

#[test]
fn nested_coverage_child_is_not_claimed_applied_when_parent_mapping_is_missing() {
    let mut bytes = source(Owner::Channel, true, false);
    enable_all_effects(&mut bytes);
    let needle = b"gegl:exposure";
    let operation = bytes.windows(needle.len()).position(|value| value == needle).unwrap();
    bytes[operation..operation + needle.len()].copy_from_slice(b"test:unmapped");
    let document = import("unmapped-coverage-parent.xcf", &bytes, &Interrupt::NONE).unwrap().document;
    assert_eq!(document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![64. / 255., 192. / 255.]);
    let stages: Vec<_> = report(&document).entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_effect").collect();
    assert_eq!(stages.len(), 2);
    assert!(stages.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    assert_eq!(stages[1].reason, ConversionReason::NativePolicy);
    assert!(stages[1].native_description.contains("discarded"));
}

// Original5x3 scalar pattern: enough variation to expose neighborhood and
// deformation behavior without a private corpus or converter-defined oracle.
const ADAPTER_VALUES: [u8; 15] = [16, 64, 220, 80, 32, 96, 180, 240, 128, 48, 24, 80, 200, 160, 64];
fn adapter_sample_bytes(values: &[f32], depth: SampleType) -> Vec<u8> {
    match depth {
        SampleType::U8 => values.iter().map(|v| (v * 255.).round() as u8).collect(),
        SampleType::U16 => values.iter().flat_map(|v| ((v * 65535.).round() as u16).to_be_bytes()).collect(),
        SampleType::F32 => values.iter().flat_map(|v| v.to_be_bytes()).collect(),
    }
}
fn adapter_source(owner: Owner, depth: SampleType, operation: &str, args: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
    let mut w = W::new(26);
    for value in [
        5,
        3,
        0,
        match depth {
            SampleType::U8 => 150,
            SampleType::U16 => 250,
            SampleType::F32 => 650,
        },
    ] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let owner_slot = w.slot();
    let sibling_slot = w.slot();
    w.ptr(0);
    let coverage_slot = if matches!(owner, Owner::LayerMask) { None } else { Some(w.slot()) };
    w.ptr(0);
    w.ptr(0);
    let sample_bytes = match depth {
        SampleType::U8 => 1,
        SampleType::U16 => 2,
        SampleType::F32 => 4,
    };
    let mut owner_mask = None;
    for (name, slot, color) in [("Owner", owner_slot, [20u8, 40, 60, 255]), ("Sibling", sibling_slot, [170, 190, 210, 255])] {
        let at = w.out.len();
        w.patch(slot, at);
        w.u32(5);
        w.u32(3);
        w.u32(1);
        w.string(name);
        w.end();
        let pixels = w.slot();
        let mask = w.slot();
        w.ptr(0);
        let values = (0..15).flat_map(|_| color.map(|v| f32::from(v) / 255.)).collect::<Vec<_>>();
        let data = adapter_sample_bytes(&values, depth);
        let at = hierarchy(&mut w, 5, 3, 4 * sample_bytes, &data, Compression::None);
        w.patch(pixels, at);
        if name == "Owner" {
            owner_mask = Some(mask);
        }
    }
    let at = w.out.len();
    if let Some(slot) = coverage_slot {
        w.patch(slot, at);
    } else {
        w.patch(owner_mask.unwrap(), at);
    }
    w.u32(5);
    w.u32(3);
    w.string("Original scalar coverage");
    if matches!(owner, Owner::Selection) {
        w.prop(4, &[]);
    }
    w.end();
    let pixels = w.slot();
    let fx = w.slot();
    w.ptr(0);
    let values = ADAPTER_VALUES.map(|v| f32::from(v) / 255.);
    let at = hierarchy(&mut w, 5, 3, sample_bytes, &adapter_sample_bytes(&values, depth), Compression::None);
    w.patch(pixels, at);
    let at = w.out.len();
    w.patch(fx, at);
    w.string("New adapter stage");
    w.string("");
    w.string(operation);
    w.string("0:0");
    w.prop_u32(8, 1);
    for (name, kind, bytes) in args {
        let mut argument = W { out: Vec::new(), wide: true };
        argument.string(name);
        argument.u32(*kind);
        argument.out.extend(bytes);
        w.prop(45, &argument.out);
    }
    w.end();
    w.ptr(0);
    w.out
}
fn adapter_coverage(document: &Document, owner: Owner) -> &Surface {
    match owner {
        Owner::Channel => &document.channels[0].surface,
        Owner::Selection => document.selection.as_ref().unwrap(),
        Owner::LayerMask => &document.layers.iter().find(|layer| layer.name == "Owner").unwrap().mask.as_ref().unwrap().surface,
    }
}
fn adapter_original(depth: SampleType) -> Surface {
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    surface.write_region(Rect::new(0, 0, 5, 3), &ADAPTER_VALUES.map(|v| f32::from(v) / 255.));
    surface
}
fn assert_adapter_native_result(document: &Document, owner: Owner, expected: &Surface) {
    let actual = adapter_coverage(document, owner);
    let bounds = Rect::new(0, 0, 5, 3);
    let stage = report(document).entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
    assert_eq!(stage.disposition, ConversionDisposition::NativeApproximation, "{}", stage.native_description);

    let tolerance = match actual.format().sample {
        SampleType::U8 => 1. / 255.,
        SampleType::U16 => 1. / 65535.,
        SampleType::F32 => 1e-5,
    };
    assert_eq!(actual.format(), expected.format());
    for (index, (a, b)) in actual.read_region(bounds).iter().zip(expected.read_region(bounds)).enumerate() {
        assert!((a - b).abs() <= tolerance, "sample{index}: actual{a} expected{b}");
    }
    assert!(stage.native_description.contains("parameter editability is lost"));
    assert!(report(document).attachments.iter().any(|a| a.owner.name == "New adapter stage" && !a.bytes.is_empty()));
    let sibling = document.layers.iter().find(|layer| layer.name == "Sibling").unwrap();
    assert_eq!(sibling.surface().unwrap().pixel(0, 0), vec![170. / 255., 190. / 255., 210. / 255., 1.]);
    let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(adapter_coverage(&reopened, owner), actual);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
}
#[test]
fn stored_coverage_ca01_colorize_matches_independent_native_adjustment_at_three_depths() {
    for (owner, depth) in [(Owner::Channel, SampleType::U8), (Owner::Selection, SampleType::U16), (Owner::LayerMask, SampleType::F32)] {
        let args = [("hue", 3, 0f32.to_be_bytes().to_vec()), ("saturation", 3, 0f32.to_be_bytes().to_vec()), ("lightness", 3, 0.2f32.to_be_bytes().to_vec())];
        let document = import("ca01-color.xcf", &adapter_source(owner, depth, "gimp:colorize", &args), &Interrupt::NONE).unwrap().document;
        let mut counterpart = Document::new("Independent native color", Size::new(5, 3), ColorMode::Grayscale, depth);
        counterpart.layers.push(Layer::new("Native scalar input", LayerContent::Raster(adapter_original(depth))));
        counterpart.layers.push(Layer::new(
            "Native Colorize",
            LayerContent::Adjustment(photocraft_doc::Adjustment::HueSaturation {
                hue: 0.,
                saturation: 0.,
                lightness: 20.,
                colorize: true,
                ranges: photocraft_doc::adjust::HueRange::defaults(),
            }),
        ));
        let expected = photocraft_compose::flatten_to_surface(&counterpart, PixelFormat::new(ColorMode::Grayscale, depth, false), None);
        assert_ne!(expected, adapter_original(depth));
        assert_adapter_native_result(&document, owner, &expected);
    }
}
#[test]
fn stored_coverage_ca02_maximum_matches_independent_native_neighborhood() {
    for (owner, depth) in [(Owner::LayerMask, SampleType::U8), (Owner::Channel, SampleType::U16), (Owner::Selection, SampleType::F32)] {
        let args = [("radius-x", 1, 2i32.to_be_bytes().to_vec()), ("radius-y", 1, 2i32.to_be_bytes().to_vec())];
        let document = import("ca02-spatial.xcf", &adapter_source(owner, depth, "gimp:grow", &args), &Interrupt::NONE).unwrap().document;
        let bounds = Rect::new(0, 0, 5, 3);
        let original = adapter_original(depth);
        let expected = photocraft_algo::apply_in_with(
            &original,
            &photocraft_algo::FilterParams::Maximum { radius: 2., preserve: photocraft_algo::Preserve::Roundness },
            bounds,
            bounds,
            None,
            bounds,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_ne!(expected, original);
        assert_adapter_native_result(&document, owner, &expected);
    }
}
#[test]
fn stored_coverage_ca03_pinch_matches_independent_native_geometry() {
    for (owner, depth) in [(Owner::Selection, SampleType::U8), (Owner::LayerMask, SampleType::U16), (Owner::Channel, SampleType::F32)] {
        let args = [("whirl", 3, 0f32.to_be_bytes().to_vec()), ("pinch", 3, 0.4f32.to_be_bytes().to_vec()), ("radius", 3, 1f32.to_be_bytes().to_vec())];
        let document = import("ca03-geometry.xcf", &adapter_source(owner, depth, "gegl:whirl-pinch", &args), &Interrupt::NONE).unwrap().document;
        let bounds = Rect::new(0, 0, 5, 3);
        let original = adapter_original(depth);
        let expected =
            photocraft_algo::apply_in_with(&original, &photocraft_algo::FilterParams::Pinch { amount: 40. }, bounds, bounds, None, bounds, &Interrupt::NONE)
                .unwrap();
        assert_ne!(expected, original);
        assert_adapter_native_result(&document, owner, &expected);
    }
}
#[test]
fn stored_coverage_ca04_alpha_extraction_does_not_invent_scalar_alpha() {
    for owner in [Owner::Channel, Owner::Selection, Owner::LayerMask] {
        let args = [("component", 5, 20i32.to_be_bytes().to_vec()), ("invert", 2, 1u32.to_be_bytes().to_vec())];
        let document =
            import("ca04-scalar-alpha.xcf", &adapter_source(owner, SampleType::F32, "gegl:component-extract", &args), &Interrupt::NONE).unwrap().document;
        assert_eq!(adapter_coverage(&document, owner), &adapter_original(SampleType::F32));
        let stage = report(&document).entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
        assert_eq!(stage.disposition, ConversionDisposition::RetainedUnapplied);
        assert!(stage.native_description.contains("alpha"), "{}", stage.native_description);
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(adapter_coverage(&reopened, owner), &adapter_original(SampleType::F32));
    }
}
