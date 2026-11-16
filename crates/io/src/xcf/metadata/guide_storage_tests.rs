use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ImportConversion, NativeTargetRef};
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

fn fixture<'a>() -> (Document, conversion::Outcomes<'a>, usize) {
    let mut document = Document::new("Bounded guides", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(PixelFormat::RGBA8);
    pixels.fill_rect(Rect::new(0, 0, 1, 1), &[0.1, 0.2, 0.3, 1.]);
    document.layers.push(Layer::new("Usable pixels", LayerContent::Raster(pixels)));
    let mut outcomes = conversion::Outcomes::new(26).unwrap();
    let owner = outcomes
        .push(ConversionEntry {
            feature_code: "xcf.canvas".into(),
            source: conversion::source("image", 0, 0, "Image", &[], None),
            targets: vec![NativeTargetRef { label: "Canvas".into(), ..Default::default() }],
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: "Source canvas".into(),
            native_description: "Native canvas".into(),
            attachments: Vec::new(),
        })
        .unwrap();
    (document, outcomes, owner)
}

fn property(guides: &[(i32, u8)]) -> Vec<u8> {
    guides.iter().flat_map(|&(position, orientation)| position.to_be_bytes().into_iter().chain([orientation])).collect()
}

fn retained_bytes(document: &Document) -> usize {
    (document.guides.horizontal.capacity() + document.guides.vertical.capacity()) * std::mem::size_of::<f64>()
}

#[test]
fn guide_spare_capacity_is_charged_before_admission_and_survives_native_reopen() {
    let data = property(&[(1, 1), (2, 1), (3, 1), (4, 1), (5, 1)]);
    let (mut document, mut outcomes, owner) = fixture();
    let pixels = document.layers.clone();
    let initial = 5 * std::mem::size_of::<f64>();
    let mut budget = initial;
    image(
        &mut document,
        &[Prop { offset: 0, kind: 18, data: &data }],
        26,
        &mut Default::default(),
        &mut outcomes,
        owner,
        &mut budget,
        &mut Default::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(!document.guides.horizontal.is_empty());
    assert!(document.guides.horizontal.len() < 5, "five pushed values used to retain eight slots while paying for only five");
    assert!(retained_bytes(&document) <= initial);
    assert_eq!(initial - budget, retained_bytes(&document));
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.guides" && entry.reason == ConversionReason::RetentionLimit));
    document.metadata.import_conversion = Some(std::sync::Arc::new(ImportConversion::Report(report)));
    assert_eq!(document.layers, pixels);
    let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&document, &Default::default()).unwrap()).unwrap();
    assert_eq!(reopened.guides, document.guides);
    assert_eq!(reopened.metadata, document.metadata);
    assert_eq!(reopened.layers, pixels);
}

#[test]
fn guide_capacity_is_charged_once_per_orientation_across_properties() {
    let first = property(&[(0, 1), (1, 2), (2, 1)]);
    let second = property(&[(0, 1), (4, 2), (5, 1), (6, 1)]);
    let props = [Prop { offset: 0, kind: 18, data: &first }, Prop { offset: first.len(), kind: 18, data: &second }];
    let (mut document, mut outcomes, owner) = fixture();
    let mut budget = 1024;
    image(&mut document, &props, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(document.guides.horizontal, [0., 2., 0., 5., 6.]);
    assert_eq!(document.guides.vertical, [1., 4.]);
    assert_eq!(1024 - budget, retained_bytes(&document));
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.guides"));
}

#[test]
fn guide_preallocated_slots_cannot_bypass_admission_and_refusal_is_atomic() {
    let mut guides = Vec::with_capacity(8);
    guides.push(1.);
    let capacity = guides.capacity();
    let mut charged = 0;
    let mut budget = std::mem::size_of::<f64>();
    assert!(!push_guide(&mut guides, 2., &mut charged, &mut budget));
    assert_eq!(guides, [1.]);
    assert_eq!(guides.capacity(), capacity);
    assert_eq!(charged, 0);
    assert_eq!(budget, std::mem::size_of::<f64>());
    budget = capacity * std::mem::size_of::<f64>();
    assert!(push_guide(&mut guides, 2., &mut charged, &mut budget));
    assert_eq!(budget, 0);
    assert!(push_guide(&mut guides, 3., &mut charged, &mut budget));
    while guides.len() < capacity {
        assert!(push_guide(&mut guides, 4., &mut charged, &mut budget));
    }
    let before = guides.clone();
    assert!(!push_guide(&mut guides, 5., &mut charged, &mut budget));
    assert_eq!(guides, before);
    assert_eq!(guides.capacity(), capacity);
    assert_eq!(charged, capacity);
    assert_eq!(budget, 0);
}

#[test]
fn admitted_guide_growth_is_geometric_and_charges_only_retained_capacity() {
    let mut guides = Vec::new();
    let mut charged = 0;
    let initial = 1024 * std::mem::size_of::<f64>();
    let mut budget = initial;
    let mut changes = 0;
    for i in 0..1024 {
        let old = guides.capacity();
        assert!(push_guide(&mut guides, f64::from(i), &mut charged, &mut budget));
        changes += usize::from(guides.capacity() != old);
        assert_eq!(initial - budget, guides.capacity() * std::mem::size_of::<f64>());
    }
    assert!(changes <= 11, "growth must not copy the full prefix for every guide");
    assert_eq!(guides, (0..1024).map(f64::from).collect::<Vec<_>>());
}
