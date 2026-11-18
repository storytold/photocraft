use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ImportConversion};
use photocraft_geom::Size;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicUsize, Ordering};

fn outcomes<'a>() -> (conversion::Outcomes<'a>, usize) {
    let mut outcomes = conversion::Outcomes::new(26).unwrap();
    let owner = outcomes
        .push(ConversionEntry {
            feature_code: "xcf.image".into(),
            source: conversion::source("image", 0, 0, "Image", &[], None),
            targets: Vec::new(),
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: String::new(),
            native_description: String::new(),
            attachments: Vec::new(),
        })
        .unwrap();
    (outcomes, owner)
}
fn guides(count: usize) -> Vec<u8> {
    (0..count).flat_map(|_| [0, 0, 0, 1, 1]).collect()
}
fn samplers(kind: u32, count: usize) -> Vec<u8> {
    let record = if kind == 39 { vec![0; 20] } else { vec![0; 8] };
    record.repeat(count)
}

#[test]
fn pre_cancelled_metadata_changes_neither_document_view_budget_nor_report() {
    let resolution = [300f32.to_be_bytes(), 150f32.to_be_bytes()].concat();
    let guides = guides(16);
    let samplers = samplers(39, 16);
    let properties =
        [Prop { kind: 19, data: &resolution, offset: 0 }, Prop { kind: 18, data: &guides, offset: 16 }, Prop { kind: 39, data: &samplers, offset: 100 }];
    let mut doc = Document::new("Unchanged", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    doc.guides.horizontal.push(3.);
    let before = doc.clone();
    let mut view = NativeInitialView { samplers: vec![[2., 3.]], ..Default::default() };
    let before_view = view.clone();
    let (mut outcomes, owner) = outcomes();
    let mut budget = 4096;
    let mut warnings = crate::ora::Warnings::default();
    let cancel = || true;
    assert!(matches!(
        image(&mut doc, &properties, 26, &mut view, &mut outcomes, owner, &mut budget, &mut warnings, &Interrupt::cancel_only(&cancel)),
        Err(crate::IoError::Cancelled)
    ));
    assert_eq!(doc, before);
    assert_eq!(view, before_view);
    assert_eq!(budget, 4096);
    assert!(warnings.finish().is_empty());
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
    assert_eq!(report.entries.len(), 1);
}

#[test]
fn cancellation_interrupts_guide_records_and_uncancelled_guides_keep_their_order() {
    let payload = guides(4096);
    let properties = [Prop { kind: 18, data: &payload, offset: 0 }];
    for cancel_at in [None, Some(11), Some(4107)] {
        let mut doc = Document::new("Guides", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let (mut outcomes, owner) = outcomes();
        let mut budget = 1 << 20;
        let calls = AtomicUsize::new(0);
        // A one-shot request proves the error is propagated rather than rescued by a later outer check.
        let cancel = || Some(calls.fetch_add(1, Ordering::Relaxed)) == cancel_at;
        let result = image(
            &mut doc,
            &properties,
            26,
            &mut Default::default(),
            &mut outcomes,
            owner,
            &mut budget,
            &mut Default::default(),
            &Interrupt::cancel_only(&cancel),
        );
        if let Some(stop) = cancel_at {
            assert!(matches!(result, Err(crate::IoError::Cancelled)));
            assert_eq!(calls.load(Ordering::Relaxed), stop + 1);
            if stop == 11 {
                assert!(doc.guides.horizontal.is_empty());
                assert_eq!(budget, 1 << 20, "the counting pass has not reserved storage");
            } else {
                assert!(!doc.guides.horizontal.is_empty() && doc.guides.horizontal.len() < 16);
            }
        } else {
            result.unwrap();
            assert_eq!(doc.guides.horizontal, vec![1.; 4096]);
            assert!(doc.guides.vertical.is_empty());
        }
    }
}

#[test]
fn cancellation_interrupts_sampler_scans_even_after_native_capacity_is_full() {
    for kind in [27, 39] {
        let payload = samplers(kind, 4096);
        let properties = [Prop { kind, data: &payload, offset: 0 }];
        for cancel_during in [false, true] {
            let mut doc = Document::new("Samplers", Size::new(8, 8), ColorMode::Grayscale, SampleType::U16);
            let mut view = NativeInitialView::default();
            let (mut outcomes, owner) = outcomes();
            let calls = AtomicUsize::new(0);
            let cancel = || calls.fetch_add(1, Ordering::Relaxed) == 63 && cancel_during;
            let mut budget = usize::MAX;
            let result =
                image(&mut doc, &properties, 26, &mut view, &mut outcomes, owner, &mut budget, &mut Default::default(), &Interrupt::cancel_only(&cancel));
            assert_eq!(view.samplers, vec![[0., 0.]; MAX_INITIAL_SAMPLERS]);
            if cancel_during {
                assert!(matches!(result, Err(crate::IoError::Cancelled)));
                assert_eq!(calls.load(Ordering::Relaxed), 64);
            } else {
                result.unwrap();
                assert!(calls.load(Ordering::Relaxed) >= 4096);
            }
        }
    }
}
