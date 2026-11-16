use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ImportConversion, NativeTargetRef};
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture<'a>() -> (Document, conversion::Outcomes<'a>, usize) {
    let mut document = Document::new("Bounded text metadata", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(PixelFormat::RGBA8);
    pixels.fill_rect(Rect::new(0, 0, 1, 1), &[17. / 255., 31. / 255., 57. / 255., 1.]);
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

fn xml_property(xml: &str) -> Vec<u8> {
    let mut property = Vec::new();
    let name = b"gimp-image-metadata\0";
    property.extend_from_slice(&(name.len() as u32).to_be_bytes());
    property.extend_from_slice(name);
    property.extend_from_slice(&0u32.to_be_bytes());
    property.extend_from_slice(&(xml.len() as u32).to_be_bytes());
    property.extend_from_slice(xml.as_bytes());
    property
}

fn retained_text_bytes(document: &Document) -> usize {
    document.metadata.text.capacity() * std::mem::size_of::<(String, String)>()
        + document.metadata.text.iter().map(|(key, value)| key.capacity() + value.capacity()).sum::<usize>()
}

#[test]
fn short_duplicate_metadata_tags_share_a_storage_budget_including_all_pair_slots() {
    let xml = format!("<metadata>{}</metadata>", "<tag name=\"Xmp\">x</tag>".repeat(64));
    let property = xml_property(&xml);
    let props = [Prop { offset: 0, kind: 21, data: &property }];
    let (mut document, mut outcomes, owner) = fixture();
    let pixels = document.layers.clone();
    let initial = xml.len();
    let mut budget = initial;
    let mut warnings = crate::ora::Warnings::default();
    image(&mut document, &props, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut warnings, &Interrupt::NONE).unwrap();
    assert!(!document.metadata.text.is_empty());
    assert!(document.metadata.text.len() < 64, "the original string-byte-only accounting retained all 64 pairs");
    assert!(document.metadata.text.iter().all(|entry| entry == &("Xmp".into(), "x".into())), "meaningful duplicates are retained in order");
    let retained = retained_text_bytes(&document);
    assert!(retained <= initial, "actual retained buffers {retained} exceed allowance {initial}");
    assert_eq!(initial - budget, retained, "charge actual pair-vector and String capacities once");
    let ImportConversion::Report(report) = outcomes.finish().unwrap() else { panic!("report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.metadata_partial" && entry.reason == ConversionReason::RetentionLimit));
    assert!(warnings.finish().iter().any(|warning| warning.contains("native metadata limits")));
    document.metadata.import_conversion = Some(std::sync::Arc::new(ImportConversion::Report(report)));
    assert_eq!(document.layers, pixels);
    let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.metadata, document.metadata);
    assert_eq!(reopened.layers, pixels);
}

#[test]
fn sufficient_budget_preserves_duplicate_tag_values_and_their_source_order() {
    let first = xml_property("<metadata><tag name=\"Xmp\">red</tag><tag name=\"Xmp\">red</tag></metadata>");
    let second = xml_property("<metadata><tag name=\"Xmp\">blue</tag></metadata>");
    let props = [Prop { offset: 0, kind: 21, data: &first }, Prop { offset: first.len(), kind: 21, data: &second }];
    let (mut document, mut outcomes, owner) = fixture();
    let mut budget = 4096;
    let mut warnings = crate::ora::Warnings::default();
    image(&mut document, &props, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut warnings, &Interrupt::NONE).unwrap();
    assert_eq!(document.metadata.text, vec![("Xmp".into(), "red".into()), ("Xmp".into(), "red".into()), ("Xmp".into(), "blue".into())]);
    assert_eq!(4096 - budget, retained_text_bytes(&document));
    assert!(warnings.finish().is_empty());
}

#[test]
fn existing_spare_capacity_cannot_bypass_admission_and_refused_growth_is_atomic() {
    let (mut document, _, _) = fixture();
    document.metadata.text = Vec::with_capacity(8);
    let capacity = document.metadata.text.capacity();
    let mut storage = TextStorage::default();
    let mut budget = 4;
    assert!(!push_text(&mut document, "Xmp", "x", &mut storage, &mut budget));
    assert_eq!((document.metadata.text.len(), document.metadata.text.capacity(), storage.charged_capacity, budget), (0, capacity, 0, 4));
    let mut budget = capacity * std::mem::size_of::<(String, String)>() + 8;
    assert!(push_text(&mut document, "Xmp", "x", &mut storage, &mut budget));
    assert!(push_text(&mut document, "Xmp", "y", &mut storage, &mut budget));
    assert_eq!(budget, 0, "already charged spare slots may be reused; each String remains charged");
    let (mut growing, _, _) = fixture();
    let mut storage = TextStorage::default();
    let mut budget = std::mem::size_of::<(String, String)>() + 8;
    assert!(push_text(&mut growing, "Xmp", "x", &mut storage, &mut budget));
    let before = (growing.metadata.text.clone(), growing.metadata.text.capacity(), storage.charged_capacity, budget);
    assert!(!push_text(&mut growing, "Xmp", "y", &mut storage, &mut budget));
    assert_eq!((growing.metadata.text.clone(), growing.metadata.text.capacity(), storage.charged_capacity, budget), before);
}

#[test]
fn metadata_tag_admission_observes_cancellation_during_a_single_xml_property() {
    let xml = format!("<metadata>{}</metadata>", "<tag name=\"Xmp\">x</tag>".repeat(64));
    let property = xml_property(&xml);
    let props = [Prop { offset: 0, kind: 21, data: &property }];
    let (mut document, mut outcomes, owner) = fixture();
    let pixels = document.layers.clone();
    let mut budget = 1 << 20;
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 16;
    let result =
        image(&mut document, &props, 26, &mut Default::default(), &mut outcomes, owner, &mut budget, &mut Default::default(), &Interrupt::cancel_only(&cancel));
    assert!(matches!(result, Err(crate::IoError::Cancelled)));
    assert!(
        !document.metadata.text.is_empty() && document.metadata.text.len() < 64,
        "cancellation occurs during tag admission, not only before the source scan"
    );
    assert_eq!(document.layers, pixels);
    assert!(retained_text_bytes(&document) <= (1 << 20) - budget);
}

#[test]
fn many_short_duplicate_entries_grow_geometrically_without_losing_order() {
    let (mut document, _, _) = fixture();
    let mut storage = TextStorage::default();
    let initial = 1024 * (std::mem::size_of::<(String, String)>() + 4);
    let mut budget = initial;
    let mut growths = 0;
    for index in 0..512 {
        let capacity = document.metadata.text.capacity();
        assert!(push_text(&mut document, "Xmp", if index % 2 == 0 { "x" } else { "y" }, &mut storage, &mut budget));
        if document.metadata.text.capacity() != capacity {
            growths += 1;
        }
    }
    assert!(growths <= 10, "512 entries need only the initial allocation and nine doublings, saw {growths}");
    assert_eq!(document.metadata.text.len(), 512);
    for (index, (key, value)) in document.metadata.text.iter().enumerate() {
        assert_eq!(key, "Xmp");
        assert_eq!(value, if index % 2 == 0 { "x" } else { "y" });
    }
    assert_eq!(initial - budget, retained_text_bytes(&document));
}
