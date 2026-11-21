use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};
use crate::xcf::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Size;
use photocraft_raster::Interrupt;

fn reset() {
    MARKUP_ATTRIBUTE_VISITS_DONE.with(|n| n.set(0));
    MARKUP_ATTRIBUTE_BYTES_DONE.with(|n| n.set(0));
    TEXT_CONFIG_PARSES.with(|n| n.set(0));
}
fn counts() -> (usize, usize, usize) {
    (
        TEXT_CONFIG_PARSES.with(std::cell::Cell::get),
        MARKUP_ATTRIBUTE_VISITS_DONE.with(std::cell::Cell::get),
        MARKUP_ATTRIBUTE_BYTES_DONE.with(std::cell::Cell::get),
    )
}
fn quota(source: usize, visits: usize, bytes: usize) -> DecodeWork {
    DecodeWork {
        config_bytes_left: source,
        attributes: AttributeWork { visits_left: visits, bytes_left: bytes, copies_left: DecodeWork::default().attributes.copies_left },
    }
}
fn config(letter: char, leaves: usize) -> Vec<u8> {
    // A fixed box avoids making font availability or origin layout the oracle.
    format!(
        "(markup \"<markup><span weight='bold'>{}</span></markup>\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16)",
        format!("<b>{letter}</b>").repeat(leaves)
    )
    .into_bytes()
}
fn source(configs: &[Vec<u8>]) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    for config in configs {
        let mut parasite = W { out: Vec::new(), wide: false };
        parasite.string("gimp-text-layer");
        parasite.u32(0);
        parasite.u32(config.len() as u32);
        parasite.out.extend_from_slice(config);
        let mut layer = TestLayer::rgba("File text work", 1, 1, vec![17, 31, 57, 255]);
        layer.props.push((26, 0u32.to_be_bytes().to_vec()));
        layer.props.push((21, parasite.out));
        image.layers.push(layer);
    }
    image.write()
}
fn image(bytes: &[u8], work: DecodeWork) -> Image<'_> {
    Image {
        c: Cursor { b: bytes, pos: 0, wide: true },
        version: 26,
        precision: Precision::GAMMA8,
        sample_order: crate::HistoricalXcfSampleOrder::BigEndian,
        compression: Compression::Rle,
        base_type: 0,
        colormap: Vec::new(),
        budget: MAX_PIXEL_BYTES,
        meta_budget: MAX_METADATA_BYTES,
        name_work_left: crate::xcf::MAX_METADATA_BYTES,
        vector_style_work_left: crate::xcf::MAX_METADATA_BYTES,
        link_work_left: crate::xcf::MAX_METADATA_BYTES,
        effect_decode_work: Default::default(),
        parasite_work: Default::default(),
        report_parasite_work: Default::default(),
        text_decode_work: work,
        point_text_work_left: POINT_LAYOUT_WORK_UNITS,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        source_resolution: (72., 72.),
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    }
}
fn pointers(bytes: &[u8], count: usize) -> Vec<usize> {
    let mut cursor = Cursor { b: bytes, pos: 30, wide: true };
    props_version(&mut cursor, Some(26)).unwrap();
    (0..count).map(|_| cursor.ptr().unwrap()).collect()
}

#[test]
fn file_attribute_work_survives_same_pointer_and_distinct_owners_with_raster_persistence() {
    for distinct in [false, true] {
        for limitation in 0..3 {
            let configs = if distinct { vec![config('x', 2), config('y', 2), config('z', 2)] } else { vec![config('x', 2)] };
            let bytes = source(&configs);
            let pointers = if distinct { pointers(&bytes, 3) } else { vec![pointers(&bytes, 1)[0]; 3] };
            let allowance = match limitation {
                0 => quota(4096, 8, 1000),
                1 => quota(4096, 100, 80),
                _ => quota(configs[0].len() * 2, 100, 1000),
            };
            let mut image = image(&bytes, allowance);
            reset();
            let (layers, _) =
                image.import_layers(&pointers, PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
            assert_eq!(counts(), (if limitation == 2 { 2 } else { 3 }, 8, 80), "the third owner starts no attribute work");
            assert_eq!(layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text(_))).count(), 2);
            let raster = layers.iter().find(|layer| matches!(layer.content, LayerContent::Raster(_))).unwrap();
            assert_eq!(raster.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
            assert!(image.meta_budget > MAX_METADATA_BYTES / 2, "retained storage is not the work oracle");
            assert_eq!(image.point_text_work_left, POINT_LAYOUT_WORK_UNITS);
            assert!(image.warnings.finish().iter().any(|warning| warning.contains("opens as pixels")));
            let report = image.outcomes.finish().unwrap();
            let ImportConversion::Report(details) = &report else { panic!("typed report") };
            let reason = match limitation {
                0 => "document text attribute-visit",
                1 => "document text attribute-byte",
                _ => "document text configuration source-work",
            };
            assert_eq!(
                details
                    .entries
                    .iter()
                    .filter(|entry| entry.feature_code == "xcf.text"
                        && entry.reason == ConversionReason::MissingMapping
                        && entry.source_description.contains(reason))
                    .count(),
                1
            );
            let failed_config = if distinct { &configs[2] } else { &configs[0] };
            assert!(details.attachments.iter().any(|attachment| attachment.bytes.windows(failed_config.len()).any(|part| part == failed_config.as_slice())));
            let mut doc = Document::new("File work", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
            doc.layers = layers.into();
            doc.metadata.import_conversion = Some(std::sync::Arc::new(report));
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.layers, doc.layers);
            assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
        }
    }
}

#[test]
fn exact_file_work_boundary_keeps_multiple_native_owners() {
    let config = config('é', 2);
    let mut work = quota(config.len() * 2, 8, 80);
    let mut layout = POINT_LAYOUT_WORK_UNITS;
    reset();
    for _ in 0..2 {
        let decoded = decode_with_work(&config, 72., (0, 0), &Interrupt::NONE, &mut layout, &mut work).unwrap().unwrap();
        assert_eq!(decoded.layer.text, "éé");
        assert!(decoded.layer.runs.iter().all(|run| run.style.weight == 700));
    }
    assert_eq!(counts(), (2, 8, 80));
    assert_eq!((work.config_bytes_left, work.attributes.visits_left, work.attributes.bytes_left), (0, 0, 0));
}

#[test]
fn failed_markup_does_not_refund_file_attribute_work() {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.set(self.0));
        }
    }
    let restore = Restore(MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.get()));
    let config = config('x', 3);
    let mut work = quota(config.len() * 3, 4, 40);
    let mut layout = POINT_LAYOUT_WORK_UNITS;
    reset();
    for owner in 0..3 {
        // Each of the first two attempts fails its local cap after two visits.
        // Failure must not restore those visits for the next owner.
        MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.set(Some(2)));
        let error = decode_with_work(&config, 72., (0, 0), &Interrupt::NONE, &mut layout, &mut work).err().unwrap();
        let expected = if owner < 2 { "bounded attribute-visit" } else { "document text attribute-visit" };
        assert!(matches!(error, crate::IoError::Unsupported(message) if message.contains(expected)));
        assert_eq!(counts(), (owner + 1, (owner + 1).min(2) * 2, (owner + 1).min(2) * 20));
    }
    assert_eq!((work.attributes.visits_left, work.attributes.bytes_left), (0, 0));
    assert_eq!(layout, POINT_LAYOUT_WORK_UNITS);
    drop(restore);
}

#[test]
fn optional_layer_failures_keep_file_work_spent_before_later_aliases() {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.set(self.0));
        }
    }
    let restore = Restore(MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.get()));
    let config = config('x', 3);
    let bytes = source(std::slice::from_ref(&config));
    let pointer = pointers(&bytes, 1)[0];
    let mut image = image(&bytes, quota(config.len() * 3, 4, 40));
    reset();
    for owner in 0..3 {
        MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.set(Some(2)));
        let (layers, _) =
            image.import_layers(&[pointer], PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
        assert!(matches!(layers[0].content, LayerContent::Raster(_)));
        assert_eq!(layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        assert_eq!(counts(), (owner + 1, (owner + 1).min(2) * 2, (owner + 1).min(2) * 20));
    }
    assert_eq!(image.point_text_work_left, POINT_LAYOUT_WORK_UNITS);
    assert!(image.meta_budget > MAX_METADATA_BYTES / 2);
    assert_eq!((image.text_decode_work.attributes.visits_left, image.text_decode_work.attributes.bytes_left), (0, 0));
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("typed report") };
    assert_eq!(report.entries.iter().filter(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::MissingMapping).count(), 3);
    assert!(report.entries.iter().any(|entry| entry.source_description.contains("document text attribute-visit")));
    drop(restore);
}

#[test]
fn invalid_modern_and_legacy_configs_spend_shared_source_work_before_parse() {
    for repeat in [false, true] {
        let mut work = quota(2, 100, 1000);
        let mut layout = POINT_LAYOUT_WORK_UNITS;
        reset();
        for owner in 0..3 {
            let error = if !repeat && owner == 1 {
                decode_gdyntext_with_work(&[255], 72., (0, 0), &Interrupt::NONE, &mut layout, &mut work).err().unwrap()
            } else {
                decode_with_work(b"(", 72., (0, 0), &Interrupt::NONE, &mut layout, &mut work).err().unwrap()
            };
            if owner == 2 {
                assert!(matches!(error, crate::IoError::Unsupported(message) if message.contains("configuration source-work")));
            } else {
                assert!(!matches!(error, crate::IoError::Unsupported(_)), "the first two attempts actually parse invalid input");
            }
        }
        assert_eq!(counts(), (2, 0, 0));
        assert_eq!((work.config_bytes_left, work.attributes.visits_left, work.attributes.bytes_left), (0, 100, 1000));
        assert_eq!(layout, POINT_LAYOUT_WORK_UNITS);
    }
}

#[test]
fn cancellation_precedes_file_work_debits_for_modern_and_legacy() {
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    let mut work = quota(0, 0, 0);
    let mut layout = POINT_LAYOUT_WORK_UNITS;
    reset();
    assert!(matches!(decode_with_work(b"(", 72., (0, 0), &ctl, &mut layout, &mut work), Err(crate::IoError::Cancelled)));
    assert!(matches!(decode_gdyntext_with_work(&[255], 72., (0, 0), &ctl, &mut layout, &mut work), Err(crate::IoError::Cancelled)));
    assert_eq!(counts(), (0, 0, 0));
    assert_eq!((work.config_bytes_left, work.attributes.visits_left, work.attributes.bytes_left), (0, 0, 0));
    assert_eq!(layout, POINT_LAYOUT_WORK_UNITS);
}
