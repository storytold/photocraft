use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

const OWNERS: usize = 6;
const PROBE_ALLOWANCE: usize = 64 * 1024;
const LATER_NAME: &str = "Later valid raster metadata";

fn image(bytes: &[u8], metadata: usize) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::None,
        base_type: 0,
        colormap: Vec::new(),
        budget: 16 + usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &PixelFormat::RGBA8)).unwrap(),
        meta_budget: metadata,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: 72.,
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}

/// Independent framing: allocate a parsed config, then fail on a nonzero terminator length.
fn fixture(malformed: bool) -> (Vec<u8>, usize, usize) {
    let mut w = W { out: vec![0; 4], wide: true };
    let effect = w.out.len();
    for value in ["Inert effect", "", "gegl:nop", "1"] {
        w.string(value);
    }
    let mut argument = W { out: Vec::new(), wide: true };
    argument.string("config");
    argument.u32(6);
    argument.string(&format!("(own-config (value \"{}\"))", "x".repeat(256)));
    w.prop(45, &argument.out);
    w.u32(0);
    w.u32(u32::from(malformed));
    w.ptr(0);
    let layer = w.out.len();
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string(LATER_NAME);
    w.end();
    let raster = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(raster, at);
    (w.out, effect, layer)
}

fn owner(image: &mut Image<'_>, ordinal: usize, layer: Option<photocraft_doc::LayerId>) -> usize {
    image
        .outcomes
        .push(ConversionEntry {
            feature_code: if layer.is_some() { "xcf.layer" } else { "xcf.channel" }.into(),
            source: conversion::source(if layer.is_some() { "layer" } else { "channel" }, ordinal, 0, &format!("Owner {ordinal}"), &[ordinal as u32], None),
            targets: layer.into_iter().map(|id| NativeTargetRef { layer_id: Some(id), ..Default::default() }).collect(),
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: "Supported native owner".into(),
            native_description: "Pixels preserved".into(),
            attachments: Vec::new(),
        })
        .unwrap()
}

fn decoded_charge(bytes: &[u8], pointer: usize, malformed: bool) -> usize {
    let base = Cursor { b: bytes, pos: 0, wide: true };
    let mut budget = PROBE_ALLOWANCE;
    let decoded = effects::decode(&base, pointer, 26, &mut budget, &Interrupt::NONE);
    if malformed {
        assert!(decoded.unwrap_err().to_string().contains("terminator"), "the fixture fails after allocating its temporary parsed config");
    } else {
        assert!(matches!(&decoded.unwrap().arguments[0].value, effects::ArgumentValue::Config(_)));
    }
    assert!(budget < PROBE_ALLOWANCE);
    PROBE_ALLOWANCE - budget
}

fn admit_later_raster(image: &mut Image<'_>, pointer: usize) -> Layer {
    let (mut layers, _) = image.import_layers(&[pointer], PixelFormat::RGBA8, PixelFormat::GRAY8, &Interrupt::NONE).unwrap();
    assert_eq!(layers.len(), 1);
    let layer = layers.remove(0);
    assert_eq!(layer.name, LATER_NAME);
    assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(image.budget, 0);
    layer
}

fn persist(document: &mut Document, outcomes: conversion::Outcomes<'_>, feature: &str, count: usize) {
    document.metadata.import_conversion = Some(Arc::new(outcomes.finish().unwrap()));
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("durable report") };
    let failed: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == feature && entry.reason == ConversionReason::InvalidOptionalData).collect();
    assert_eq!(failed.len(), count);
    for ordinal in 0..count {
        assert!(failed.iter().any(|entry| entry.source.item_path == [ordinal as u32]));
    }
    let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    assert_eq!(reopened.layers, document.layers);
    assert_eq!(reopened.layers.last().unwrap().surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
}

#[test]
fn malformed_coverage_effects_refund_only_temporary_decode_storage_across_distinct_owners() {
    let (bytes, effect, later) = fixture(true);
    let temporary = decoded_charge(&bytes, effect, true);
    // One pointer's worklist plus seen-set traversal allowance: 6 + 8 machine words.
    let traversal = 14 * std::mem::size_of::<usize>();
    let initial = OWNERS * traversal + LATER_NAME.len() + temporary - 1;
    let mut image = image(&bytes, initial);
    for ordinal in 0..OWNERS {
        let owner = owner(&mut image, ordinal, None);
        image.inventory_coverage_effects(owner, &[effect], &Interrupt::NONE).unwrap();
        assert_eq!(image.meta_budget, initial - (ordinal + 1) * traversal, "failed temporary effect storage must not accumulate across owners");
    }
    assert_eq!(image.meta_budget, LATER_NAME.len() + temporary - 1);
    let layer = admit_later_raster(&mut image, later);
    assert_eq!(image.meta_budget, temporary - 1, "later valid raster metadata still fits the untouched shared allowance");
    let mut document = Document::new("Coverage inventory", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers.push(layer);
    persist(&mut document, image.outcomes, "xcf.coverage_effect", OWNERS);
}

#[test]
fn malformed_native_effects_refund_temporary_storage_without_losing_later_raster_metadata() {
    let (bytes, effect, later) = fixture(true);
    let temporary = decoded_charge(&bytes, effect, true);
    let initial = LATER_NAME.len() + temporary - 1;
    let mut image = image(&bytes, initial);
    let mut document = Document::new("Native effect owners", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    for ordinal in 0..OWNERS {
        let mut pixels = Surface::new(PixelFormat::RGBA8);
        pixels.fill_rect(Rect::new(0, 0, 1, 1), &[1., 0., 1., 1.]);
        let layer = Layer::new(format!("Owner {ordinal}"), LayerContent::Raster(pixels));
        let id = layer.id;
        document.layers.push(layer);
        let owner = owner(&mut image, ordinal, Some(id));
        image.pending_effects.push(PendingLayerEffects { layer: id, owner, bounds: Rect::new(0, 0, 1, 1), pointers: vec![effect] });
    }
    import_native_effects(&mut document, &mut image, &Interrupt::NONE).unwrap();
    assert_eq!(image.meta_budget, initial);
    assert!(image.pending_effects.is_empty());
    assert!(document.layers.iter().all(|layer| layer.surface().unwrap().pixel(0, 0) == vec![1., 0., 1., 1.]));
    document.layers.push(admit_later_raster(&mut image, later));
    assert_eq!(image.meta_budget, temporary - 1);
    persist(&mut document, image.outcomes, "xcf.layer_effect", OWNERS);
}

#[test]
fn successful_effect_decodes_keep_their_charges_in_both_import_paths() {
    let (bytes, effect, _) = fixture(false);
    let retained = decoded_charge(&bytes, effect, false);
    let mut coverage = image(&bytes, PROBE_ALLOWANCE);
    let source = owner(&mut coverage, 0, None);
    coverage.inventory_coverage_effects(source, &[effect], &Interrupt::NONE).unwrap();
    assert_eq!(coverage.meta_budget, PROBE_ALLOWANCE - retained - 14 * std::mem::size_of::<usize>());
    let mut native = image(&bytes, PROBE_ALLOWANCE);
    let mut document = Document::new("Native", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::raster("Owner", PixelFormat::RGBA8);
    let id = layer.id;
    document.layers.push(layer);
    let source = owner(&mut native, 0, Some(id));
    native.pending_effects.push(PendingLayerEffects { layer: id, owner: source, bounds: Rect::new(0, 0, 1, 1), pointers: vec![effect] });
    import_native_effects(&mut document, &mut native, &Interrupt::NONE).unwrap();
    assert_eq!(native.meta_budget, PROBE_ALLOWANCE - retained);
}

#[test]
fn cancelled_effect_decode_aborts_instead_of_becoming_refunded_optional_loss() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (bytes, effect, _) = fixture(false);
    for coverage in [false, true] {
        for mid_decode in [false, true] {
            let mut image = image(&bytes, PROBE_ALLOWANCE);
            let mut document = Document::new("Cancelled", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
            let layer = Layer::raster("Owner", PixelFormat::RGBA8);
            let id = layer.id;
            document.layers.push(layer);
            let original = document.layers.clone();
            let source = owner(&mut image, 0, Some(id));
            let checks = AtomicUsize::new(0);
            let cancelled = || {
                let count = checks.fetch_add(1, Ordering::Relaxed) + 1;
                !mid_decode || count >= 5
            };
            let ctl = Interrupt::cancel_only(&cancelled);
            let result = if coverage {
                image.inventory_coverage_effects(source, &[effect], &ctl)
            } else {
                image.pending_effects.push(PendingLayerEffects { layer: id, owner: source, bounds: Rect::new(0, 0, 1, 1), pointers: vec![effect] });
                import_native_effects(&mut document, &mut image, &ctl)
            };
            assert!(matches!(result, Err(IoError::Cancelled)));
            assert_eq!(document.layers, original);
            if mid_decode {
                assert!(image.meta_budget < PROBE_ALLOWANCE, "cancellation after allocation propagates without the handled-error refund");
            } else {
                assert_eq!(image.meta_budget, PROBE_ALLOWANCE);
            }
            let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("report") };
            assert!(!report.entries.iter().any(|entry| entry.reason == ConversionReason::InvalidOptionalData));
        }
    }
}
