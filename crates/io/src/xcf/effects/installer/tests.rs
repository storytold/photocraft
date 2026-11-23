use super::super::tests::{prepared, scalar, source};
use super::*;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::LayerMask;
use std::sync::atomic::{AtomicUsize, Ordering};

fn owner(mode: ColorMode, depth: SampleType) -> (Document, Layer, Rect) {
    let document = Document::new("Parent", Size::new(4, 4), mode, depth);
    let format = PixelFormat::new(mode, depth, true);
    let mut surface = Surface::new(format);
    let bounds = Rect::from_xywh(-3, 5, 2, 2);
    for (index, rgba) in [
        [0.1, 0.2, 0.3, 0.5],
        [0.4, 0.5, 0.6, 1.],
        [0.7, 0.8, 0.9, 0.75],
        if depth == SampleType::F32 { [-0.25, 1.25, 2., 0.25] } else { [0.2, 0.3, 0.4, 0.25] },
    ]
    .into_iter()
    .enumerate()
    {
        let pixel = photocraft_raster::from_rgba(&format, rgba);
        surface.write_pixel(bounds.x0 + (index % 2) as i32, bounds.y0 + (index / 2) as i32, &pixel);
    }
    let mut layer = Layer::new("Original owner", LayerContent::Raster(surface));
    layer.opacity = 0.375;
    layer.fill_opacity = 0.75;
    layer.visible = false;
    let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    mask.fill_rect(bounds, &[0.5]);
    layer.mask = Some(LayerMask { surface: mask, enabled: true, linked: false, density: 0.75, feather: 0. });
    (document, layer, bounds)
}
fn reference(source: &Surface, bounds: Rect, mode: ColorMode, depth: SampleType) -> Surface {
    let format = source.format();
    let mut output = Surface::new(format);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let mut buffer = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![source.rgba(x, y)] };
            for adjustment in
                [Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. }, Adjustment::BrightnessContrast { brightness: 0., contrast: -50., legacy: true }]
            {
                adjust::apply_depth(&adjustment, &mut buffer, Transfer::for_document(mode, depth), Some(depth));
                output.write_pixel(x, y, &photocraft_raster::from_rgba(&format, buffer.px[0]));
                buffer.px[0] = output.rgba(x, y);
            }
        }
    }
    output
}

#[test]
fn raster_tone_installation_keeps_offset_outer_settings_native_source_and_reopens() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in SampleType::ALL {
            let (mut document, original, bounds) = owner(mode, depth);
            let source = original.surface().unwrap();
            let expected = reference(source, bounds, mode, depth);
            let stages = [
                prepared(&source_effect("gegl:exposure", vec![scalar("exposure", 1.)])),
                prepared(&source_effect("gegl:brightness-contrast", vec![scalar("contrast", 0.5)])),
            ];
            let identity = stages[0].owner;
            let mut retained = 64 << 20;
            let mut work = 128 << 20;
            let result =
                prepare(identity, &original, bounds, &document, &stages, &mut Budgets { retained: &mut retained, work: &mut work }, &Interrupt::NONE).unwrap();
            assert_eq!(result.source_owner, identity);
            assert!(!result.notes.is_empty());
            assert_eq!(result.smart.transform, Affine::translate(-3., 5.));
            assert_eq!(result.smart.smart_filters.len(), 2);
            assert_eq!(result.smart.cache.as_ref().unwrap().read_region(bounds), expected.read_region(bounds));
            let SmartSource::Embedded { bytes, .. } = &result.smart.source else { panic!("native embedded source") };
            assert!(photocraft_format::is_pcraft(bytes));
            let contents = photocraft_format::load_from_bytes(bytes).unwrap();
            assert_eq!(contents.size, Size::new(2, 2));
            for y in 0..2 {
                for x in 0..2 {
                    assert_eq!(contents.layers[0].surface().unwrap().pixel(x, y), source.pixel(x - 3, y + 5));
                }
            }
            let mut installed = original.clone();
            installed.content = LayerContent::Smart(result.smart);
            assert_eq!(installed.id, original.id);
            assert_eq!(installed.opacity, original.opacity);
            assert_eq!(installed.fill_opacity, original.fill_opacity);
            assert_eq!(installed.visible, original.visible);
            assert_eq!(installed.mask, original.mask);
            document.layers.push(installed);
            let encoded = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&encoded).unwrap();
            let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("editable native stage lost") };
            assert_eq!(smart.smart_filters, stages.iter().flat_map(|stage| stage.filters.iter()).cloned().collect::<Vec<_>>());
            assert_eq!(smart.transform, Affine::translate(-3., 5.));
            assert_eq!(smart.cache.as_ref().unwrap().read_region(bounds), expected.read_region(bounds));
            assert_eq!(reopened.layers[0].mask, original.mask);
            assert!(retained < 64 << 20 && work < 128 << 20);
        }
    }
}

fn source_effect<'a>(operation: &'a str, args: Vec<craft_xcf::effects::Argument<'a>>) -> craft_xcf::effects::OwnedEffect<'a> {
    source(operation, args)
}

#[test]
fn disabled_source_stage_is_editable_but_does_not_change_the_saved_cache() {
    let (document, original, bounds) = owner(ColorMode::Rgb, SampleType::F32);
    let mut effect = source("gegl:exposure", vec![scalar("exposure", 4.), scalar("black-level", 0.1)]);
    effect.record.visible = false;
    let stages = [prepared(&effect)];
    let mut retained = 64 << 20;
    let mut work = 128 << 20;
    let result =
        prepare(stages[0].owner, &original, bounds, &document, &stages, &mut Budgets { retained: &mut retained, work: &mut work }, &Interrupt::NONE).unwrap();
    assert_eq!(result.smart.smart_filters.len(), 2);
    assert!(result.smart.smart_filters.iter().all(|filter| !filter.visible));
    assert_eq!(result.smart.cache.as_ref().unwrap().read_region(bounds), original.surface().unwrap().read_region(bounds));
}

#[test]
fn refusals_and_late_cancellation_keep_original_pixels_and_storage_atomic() {
    let (document, original, bounds) = owner(ColorMode::Rgb, SampleType::F32);
    let before = original.clone();
    let mut stage = prepared(&source("gegl:exposure", vec![scalar("exposure", 1.)]));
    for kind in 0..3 {
        stage.envelope.opacity = if kind == 0 { 0.5 } else { 1. };
        stage.envelope.mask = if kind == 1 { MaskRequirement::Required(123) } else { MaskRequirement::None };
        stage.owner.offset = if kind == 2 { 78 } else { 77 };
        let mut retained = 64 << 20;
        let mut work = 128 << 20;
        assert!(matches!(
            prepare(
                Owner { kind: OwnerKind::Layer, offset: 77 },
                &original,
                bounds,
                &document,
                std::slice::from_ref(&stage),
                &mut Budgets { retained: &mut retained, work: &mut work },
                &Interrupt::NONE
            ),
            Err(Refusal::Unsupported(_))
        ));
        assert_eq!(retained, 64 << 20);
        assert_eq!(original, before);
    }
    let stage = prepared(&source("gegl:exposure", vec![scalar("exposure", 1.)]));
    let mut retained = 64 << 20;
    let mut work = 1;
    assert!(matches!(
        prepare(
            stage.owner,
            &original,
            bounds,
            &document,
            std::slice::from_ref(&stage),
            &mut Budgets { retained: &mut retained, work: &mut work },
            &Interrupt::NONE
        ),
        Err(Refusal::Limit)
    ));
    assert_eq!(work, 0);
    assert_eq!(retained, 64 << 20);
    let mut tiny = 1;
    let mut enough_work = 128 << 20;
    assert!(matches!(
        prepare(
            stage.owner,
            &original,
            bounds,
            &document,
            std::slice::from_ref(&stage),
            &mut Budgets { retained: &mut tiny, work: &mut enough_work },
            &Interrupt::NONE
        ),
        Err(Refusal::Limit)
    ));
    assert_eq!(tiny, 1);
    assert_eq!(enough_work, 128 << 20);
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) + 1 >= 8; // after native serialization, before publication
    let mut work = 128 << 20;
    assert!(matches!(
        prepare(
            stage.owner,
            &original,
            bounds,
            &document,
            std::slice::from_ref(&stage),
            &mut Budgets { retained: &mut retained, work: &mut work },
            &Interrupt::cancel_only(&cancel)
        ),
        Err(Refusal::Cancelled)
    ));
    assert!(work < 128 << 20);
    assert_eq!(retained, 64 << 20);
    assert_eq!(original, before);
    let group = Layer::group("Not admitted", vec![]);
    assert!(matches!(
        prepare(
            stage.owner,
            &group,
            bounds,
            &document,
            std::slice::from_ref(&stage),
            &mut Budgets { retained: &mut retained, work: &mut work },
            &Interrupt::NONE
        ),
        Err(Refusal::Unsupported(_))
    ));
}

#[test]
fn repeated_late_refusals_spend_work_without_publishing_or_retaining_pixels() {
    let (document, mut original, bounds) = owner(ColorMode::Rgb, SampleType::F32);
    original.surface_mut().unwrap().write_pixel(bounds.x0, bounds.y0, &[f32::NAN, 0.25, 0.5, 1.]);
    let before = original.surface().unwrap().try_to_interleaved(bounds).unwrap();
    let stage = prepared(&source("gegl:exposure", vec![scalar("exposure", 1.)]));
    let mut retained = 64 << 20;
    let mut work = 128 << 20;
    for _ in 0..2 {
        let prior = work;
        assert!(matches!(
            prepare(
                stage.owner,
                &original,
                bounds,
                &document,
                std::slice::from_ref(&stage),
                &mut Budgets { retained: &mut retained, work: &mut work },
                &Interrupt::NONE
            ),
            Err(Refusal::Preparation(_))
        ));
        assert!(work < prior);
        assert_eq!(retained, 64 << 20);
        assert_eq!(original.surface().unwrap().try_to_interleaved(bounds).unwrap(), before);
    }
}

#[test]
fn caller_allowances_cannot_expand_the_absolute_private_preparation_cap() {
    let (document, original, _) = owner(ColorMode::Rgb, SampleType::F32);
    let stage = prepared(&source("gegl:exposure", vec![]));
    let mut retained = usize::MAX;
    let mut work = usize::MAX;
    let bounds = Rect::from_xywh(0, 0, craft_xcf::MAX_SIDE, craft_xcf::MAX_SIDE);
    assert!(matches!(
        prepare(stage.owner, &original, bounds, &document, &[stage], &mut Budgets { retained: &mut retained, work: &mut work }, &Interrupt::NONE),
        Err(Refusal::Limit)
    ));
    assert_eq!((retained, work), (usize::MAX, usize::MAX));
}

#[test]
fn native_tile_storage_is_admitted_before_preparation() {
    let (document, original, bounds) = owner(ColorMode::Rgb, SampleType::U8);
    let stage = prepared(&source("gegl:exposure", vec![scalar("exposure", 0.5)]));
    let mut retained = 2 << 20;
    let before = retained;
    let mut work = 128 << 20;
    let work_before = work;
    assert!(matches!(
        prepare(stage.owner, &original, bounds, &document, &[stage], &mut Budgets { retained: &mut retained, work: &mut work }, &Interrupt::NONE),
        Err(Refusal::Limit)
    ));
    assert_eq!(retained, before);
    assert_eq!(work, work_before, "storage refusal precedes work/materialization");
}

#[test]
fn masked_coverage_late_cancellation_keeps_owner_and_retention_atomic() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (document, mut original, bounds) = owner(ColorMode::Rgb, SampleType::U16);
    original.surface_mut().unwrap().fill_rect(bounds, &[0.125, 0.375, 0.625, 1.]);
    let before = original.clone();
    let mut effect = source("gegl:exposure", vec![scalar("exposure", 0.75)]);
    effect.record.mask_offset = 123;
    effect.record.opacity = 0.65;
    let stage = prepared(&effect);
    let make_coverage = || {
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        mask.surface.fill_rect(bounds, &[0.375]);
        ResolvedMask { source_offset: 123, mask }
    };
    let mut retained = 64 << 20;
    let mut work = 128 << 20;
    let calls = AtomicUsize::new(0);
    let observe = || {
        calls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let result = prepare_masked_exposure(
        stage.owner,
        &original,
        bounds,
        &document,
        MaskedExposure { stage: &stage, coverage: make_coverage() },
        &mut Budgets { retained: &mut retained, work: &mut work },
        &Interrupt::cancel_only(&observe),
    )
    .unwrap();
    assert!(result.smart.filter_mask.is_some());
    // The final two checks are after coverage and before publication. Refuse
    // inside mix_in's final checks after its staged tile write, not at entry.
    let cancel_at = calls.load(Ordering::Relaxed).checked_sub(4).unwrap();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= cancel_at;
    let mut retained = 64 << 20;
    let retained_before = retained;
    let mut work = 128 << 20;
    assert!(matches!(
        prepare_masked_exposure(
            stage.owner,
            &original,
            bounds,
            &document,
            MaskedExposure { stage: &stage, coverage: make_coverage() },
            &mut Budgets { retained: &mut retained, work: &mut work },
            &Interrupt::cancel_only(&cancel)
        ),
        Err(Refusal::Cancelled)
    ));
    assert_eq!(original, before);
    assert_eq!(retained, retained_before);
    assert!(work < 128 << 20, "attempted native generation is never refunded");
}

#[test]
fn levels_unproven_contexts_refuse_before_spending_generation_budgets() {
    for (mode, depth) in [(ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Grayscale, SampleType::F32)] {
        let (document, original, bounds) = owner(mode, depth);
        let before = original.clone();
        let stage = prepared(&source("gegl:levels", vec![scalar("in-low", 0.125), scalar("in-high", 0.875)]));
        let mut retained = 64 << 20;
        let mut work = 128 << 20;
        assert!(matches!(
            prepare(stage.owner, &original, bounds, &document, &[stage], &mut Budgets { retained: &mut retained, work: &mut work }, &Interrupt::NONE),
            Err(Refusal::Unsupported(_))
        ));
        assert_eq!(retained, 64 << 20);
        assert_eq!(work, 128 << 20);
        assert_eq!(original, before);
    }
}

#[test]
fn spatial_workspace_work_and_late_cancel_do_not_publish_partial_owners() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (document, original, bounds) = owner(ColorMode::Rgb, SampleType::U8);
    let before = original.clone();
    let stage = prepared(&source("gegl:gaussian-blur", vec![scalar("std-dev-x", 2.), scalar("std-dev-y", 1.)]));
    for (retained_start, work_start) in [(4 << 20, 128 << 20), (64 << 20, 1)] {
        let mut retained = retained_start;
        let mut work = work_start;
        assert!(matches!(
            prepare(
                stage.owner,
                &original,
                bounds,
                &document,
                std::slice::from_ref(&stage),
                &mut Budgets { retained: &mut retained, work: &mut work },
                &Interrupt::NONE
            ),
            Err(Refusal::Limit)
        ));
        assert_eq!(retained, retained_start);
        assert_eq!(original, before);
        if work_start == 1 {
            assert_eq!(work, 0);
        }
    }
    let flag = AtomicBool::new(false);
    let cancel = || flag.load(Ordering::Relaxed);
    let progress = |p: f32| {
        if p > 0. {
            flag.store(true, Ordering::Relaxed);
        }
    };
    let mut retained = 64 << 20;
    let mut work = 128 << 20;
    assert!(matches!(
        prepare(
            stage.owner,
            &original,
            bounds,
            &document,
            std::slice::from_ref(&stage),
            &mut Budgets { retained: &mut retained, work: &mut work },
            &Interrupt::new(&cancel, &progress)
        ),
        Err(Refusal::Cancelled)
    ));
    assert!(flag.load(Ordering::Relaxed));
    assert_eq!(retained, 64 << 20);
    assert!(work < 128 << 20);
    assert_eq!(original, before);
}
