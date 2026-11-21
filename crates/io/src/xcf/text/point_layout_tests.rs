use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};
use photocraft_doc::LayerContent;
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
use photocraft_raster::Interrupt;

fn reset() {
    ORIGIN_SHARED_ACCESSES.with(|n| n.set(0));
    ORIGIN_LAYOUTS.with(|n| n.set(0));
}
fn accesses() -> usize {
    ORIGIN_SHARED_ACCESSES.with(std::cell::Cell::get)
}
fn layouts() -> usize {
    ORIGIN_LAYOUTS.with(std::cell::Cell::get)
}
fn config(text: &str) -> Vec<u8> {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");
    format!("(text \"{escaped}\") (font \"Sans\") (font-size 12)").into_bytes()
}
fn legacy(text: &str) -> Vec<u8> {
    format!("GDT10{{{text}}}{{1}}{{0}}{{0}}{{0}}{{ffffff}}{{0}}{{-misc-fixed-medium-r-normal--12-120-75-75-p-100-iso8859-1}}").into_bytes()
}
fn source(name: &str, config: &[u8]) -> Vec<u8> {
    let mut parasite = W { out: Vec::new(), wide: false };
    parasite.string(name);
    parasite.u32(0);
    parasite.u32(config.len() as u32);
    parasite.out.extend_from_slice(config);
    let mut layer = TestLayer::rgba("Point source", 1, 1, vec![17, 31, 57, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite.out));
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.layers.push(layer);
    image.write()
}
fn controlled(config: &[u8], work: &mut usize) -> Result<Option<DecodedText>> {
    decode_with_interrupt(config, 72., (13, 17), &Interrupt::NONE, work)
}
fn refused_before_shared(config: &[u8], dpi: f32) {
    reset();
    let mut work = POINT_LAYOUT_WORK_UNITS;
    assert!(decode_with_interrupt(config, dpi, (0, 0), &Interrupt::NONE, &mut work).unwrap().is_none());
    assert_eq!((accesses(), layouts()), (0, 0), "refusal precedes shared font-engine initialization/lock and layout");
    assert_eq!(work, POINT_LAYOUT_WORK_UNITS, "unattempted layout spends no work units");
}

#[test]
fn point_origin_admission_precedes_shared_access_for_scalar_line_run_and_geometry_limits() {
    // Including combining marks prevents a single-grapheme shaping workload
    // from evading the scalar allowance. Long lines have a separate bound.
    refused_before_shared(&config(&format!("a{}", "\u{301}".repeat(layout_admission::MAX_SCALARS))), 72.);
    refused_before_shared(&config(&"a".repeat(layout_admission::MAX_LINE_SCALARS + 1)), 72.);
    refused_before_shared(&config(&"x\n".repeat(layout_admission::MAX_BREAKS + 1)), 72.);
    refused_before_shared(&config(&"x".repeat(512 * 1024)), 72.);
    // This markup reaches the older materialized-formatting allowance before
    // origin admission. Keep its distinct refusal and pre-lock guarantees.
    let markup = format!("(markup \"<markup>{}</markup>\")", "<b>x</b><i>x</i>".repeat(layout_admission::MAX_RUNS / 2 + 1));
    reset();
    let mut work = POINT_LAYOUT_WORK_UNITS;
    assert!(matches!(controlled(markup.as_bytes(), &mut work), Err(crate::IoError::Unsupported(message)) if message.contains("bounded formatting allowance")));
    assert_eq!((work, accesses(), layouts()), (POINT_LAYOUT_WORK_UNITS, 0, 0));
    // Construct the final native data directly to isolate the new run limit
    // from that independent parser allocation allowance.
    let mut layer = TextLayer { text: "x".repeat(layout_admission::MAX_RUNS), ..Default::default() };
    layer.runs = vec![TextRun { len: 1, style: CharStyle::default() }; layout_admission::MAX_RUNS];
    layer.paragraphs.push(ParagraphRun { len: layer.text.len(), style: ParagraphStyle::default() });
    assert!(layout_admission::cost(&layer, 72.).is_some(), "the run boundary itself is admitted");
    layer.text.push('x');
    layer.runs.push(TextRun { len: 1, style: CharStyle::default() });
    layer.paragraphs[0].len = layer.text.len();
    assert!(layout_admission::cost(&layer, 72.).is_none());
    assert!(!anchor_point_origin(&mut layer, 72., &Interrupt::NONE, &mut work).unwrap());
    assert_eq!((work, accesses(), layouts()), (POINT_LAYOUT_WORK_UNITS, 0, 0));
    refused_before_shared(b"(text \"x\") (font-size 8192) (font-size-unit points)", 999_999.);
}

#[test]
fn point_origin_style_geometry_and_font_requests_are_bounded_without_layout() {
    let mut layer = TextLayer { text: "x".into(), ..Default::default() };
    layer.runs.push(TextRun { len: 1, style: CharStyle::default() });
    layer.paragraphs.push(ParagraphRun { len: 1, style: ParagraphStyle::default() });
    assert!(layout_admission::cost(&layer, 72.).is_some());
    for value in [f32::NAN, f32::INFINITY] {
        let mut bad = layer.clone();
        bad.runs[0].style.horizontal_scale = value;
        assert!(layout_admission::cost(&bad, 72.).is_none());
    }
    let mut bad = layer.clone();
    bad.runs[0].style.font_family = "f".repeat(1025);
    assert!(layout_admission::cost(&bad, 72.).is_none());
    let mut bad = layer;
    bad.runs[0].style.tracking = f32::MAX;
    assert!(layout_admission::cost(&bad, 72.).is_none());
}

#[test]
fn point_origin_boundary_multilingual_and_box_text_preserve_complete_native_content() {
    // 4096 scalars across two legal lines exercises the exact scalar boundary.
    let boundary = format!("{}\n{}", "a".repeat(2047), "b".repeat(2048));
    let mut work = POINT_LAYOUT_WORK_UNITS;
    let parsed = controlled(&config(&boundary), &mut work).unwrap().unwrap();
    assert_eq!(parsed.layer.text, boundary);
    let text = "Ag é λ العربية 界";
    let parsed = controlled(&config(text), &mut work).unwrap().unwrap();
    assert_eq!(parsed.layer.text, text);
    assert_eq!(parsed.layer.runs[0].style.size_pt, 12.);
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let [left, top, _, _] = engine.layout(&parsed.layer, 72.).bounds().unwrap();
    let origin = parsed.layer.transform.apply(photocraft_geom::Point::new(f64::from(left), f64::from(top)));
    assert!((origin.x - 13.).abs() < 0.0001 && (origin.y - 17.).abs() < 0.0001);
    drop(engine);
    let big = "x".repeat(layout_admission::MAX_SCALARS + 1);
    let mut boxed = config(&big);
    boxed.extend_from_slice(b" (box-mode fixed) (box-width 100) (box-height 100)");
    reset();
    let mut empty = 0;
    let parsed = controlled(&boxed, &mut empty).unwrap().unwrap();
    assert_eq!(parsed.layer.text, big);
    assert!(matches!(parsed.layer.shape, TextShape::Box { .. }));
    assert_eq!((empty, accesses(), layouts()), (0, 0, 0), "Box text never performs the optional point-origin layout");
}

#[test]
fn point_origin_document_work_is_shared_and_spent_layouts_are_not_refunded() {
    let bytes = config("Ag");
    let mut work = POINT_LAYOUT_WORK_UNITS;
    let layer = controlled(&bytes, &mut work).unwrap().unwrap().layer;
    let cost = layout_admission::cost(&layer, 72.).unwrap();
    assert_eq!(POINT_LAYOUT_WORK_UNITS - work, cost);
    reset();
    let mut work = cost * 2;
    for _ in 0..2 {
        assert!(controlled(&bytes, &mut work).unwrap().is_some());
    }
    assert_eq!((work, layouts()), (0, 2));
    assert!(controlled(&bytes, &mut work).unwrap().is_none());
    assert_eq!((accesses(), layouts()), (2, 2), "exhaustion cannot enter the shared engine again");
    reset();
    let mut work = cost;
    let after_layout = || layouts() > 0;
    assert!(matches!(decode_with_interrupt(&bytes, 72., (0, 0), &Interrupt::cancel_only(&after_layout), &mut work), Err(crate::IoError::Cancelled)));
    assert_eq!((work, layouts()), (0, 1), "a completed/cancelled measurement is not refunded");
}

#[test]
fn legacy_point_origin_has_the_same_pre_lock_limits_and_cancellation() {
    reset();
    let mut work = POINT_LAYOUT_WORK_UNITS;
    assert!(decode_gdyntext_with_interrupt(&legacy(&"x".repeat(4097)), 72., (0, 0), &Interrupt::NONE, &mut work).unwrap().is_none());
    assert_eq!((work, accesses(), layouts()), (POINT_LAYOUT_WORK_UNITS, 0, 0));
    let cancel = || true;
    assert!(matches!(decode_gdyntext_with_interrupt(&legacy("Ag"), 72., (0, 0), &Interrupt::cancel_only(&cancel), &mut work), Err(crate::IoError::Cancelled)));
    assert_eq!(accesses(), 0);
    assert!(decode_gdyntext_with_interrupt(&legacy("Ag"), 72., (0, 0), &Interrupt::NONE, &mut work).unwrap().is_some());
}

#[test]
fn point_layout_refusal_keeps_pixels_source_disclosure_and_native_roundtrip() {
    for (name, bytes) in [("gimp-text-layer", config(&"x".repeat(4097))), ("plug_in_gdyntext/data", legacy(&"x".repeat(4097)))] {
        reset();
        let imported = crate::import("bounded.xcf", &source(name, &bytes)).unwrap();
        assert_eq!((accesses(), layouts()), (0, 0));
        let document = imported.document;
        assert_eq!(document.layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        let ImportConversion::Report(report) = document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::RetentionLimit));
        assert!(report.attachments.iter().any(|attachment| attachment.bytes.windows(bytes.len()).any(|window| window == bytes.as_slice())));
        assert!(imported.warnings.iter().any(|warning| warning.contains("origin-layout allowance")));
        assert!(imported.warnings.iter().all(|warning| warning.len() < 1024));
        let native = crate::export(&document, "pcraft", &Default::default()).unwrap();
        let reopened = crate::import("bounded.pcraft", &native.bytes).unwrap();
        assert_eq!(reopened.document, document);
        assert!(!reopened.warnings.is_empty(), "unapplied editable text remains disclosed after native reopen");
        let flat = crate::export(&reopened.document, "png", &Default::default()).unwrap();
        assert!(flat.warnings.iter().any(|warning| warning.contains("not exported")));
    }
    let ordinary = crate::import("ordinary.xcf", &source("gimp-text-layer", &config("Ag"))).unwrap();
    assert!(matches!(ordinary.document.layers[0].content, LayerContent::Text(_)));
}

#[test]
fn point_layout_cancellation_propagates_through_optional_layer_translation() {
    reset();
    for (name, config) in [("gimp-text-layer", config("Ag")), ("plug_in_gdyntext/data", legacy("Ag"))] {
        reset();
        let bytes = source(name, &config);
        let observed = std::sync::atomic::AtomicBool::new(false);
        // A one-shot cancellation proves optional translation propagates the
        // error itself; a later outer check cannot rescue a swallowed error.
        let cancel = || layouts() > 0 && !observed.swap(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(crate::xcf::import("cancel.xcf", &bytes, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        assert_eq!(layouts(), 1);
        assert!(observed.load(std::sync::atomic::Ordering::Relaxed));
    }
    reset();
    let cancel = || true;
    let mut work = POINT_LAYOUT_WORK_UNITS;
    assert!(matches!(decode_with_interrupt(&config("Ag"), 72., (0, 0), &Interrupt::cancel_only(&cancel), &mut work), Err(crate::IoError::Cancelled)));
    assert_eq!((work, accesses(), layouts()), (POINT_LAYOUT_WORK_UNITS, 0, 0));
}

#[test]
fn repeated_point_layers_exhaust_only_layout_work_and_report_raster_fallback() {
    use crate::xcf::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let bytes = source("gimp-text-layer", &config("Ag"));
    // Use the public synthetic framing but repeat the same layer pointer, so
    // source size and retained allocation cannot stand in for the work budget.
    let mut cursor = Cursor { b: &bytes, pos: 30, wide: true };
    props_version(&mut cursor, Some(26)).unwrap();
    let pointer = cursor.ptr().unwrap();
    // Source helper uses RLE. The direct Image matches that wire encoding.
    let mut image = Image {
        c: Cursor { b: &bytes, pos: 0, wide: true },
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
        text_decode_work: Default::default(),
        point_text_work_left: 0,
        warnings: Warnings::default(),
        outcomes: conversion::Outcomes::new(26).unwrap(),
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        source_resolution: (72., 72.),
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error: None,
    };
    let mut probe = POINT_LAYOUT_WORK_UNITS;
    let layer = controlled(&config("Ag"), &mut probe).unwrap().unwrap().layer;
    image.point_text_work_left = layout_admission::cost(&layer, 72.).unwrap();
    reset();
    let (layers, _) =
        image.import_layers(&[pointer; 3], PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
    assert_eq!((image.point_text_work_left, accesses(), layouts()), (0, 1, 1));
    assert_eq!(layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text(_))).count(), 1);
    for layer in layers.iter().filter(|layer| matches!(layer.content, LayerContent::Raster(_))) {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    assert!(image.meta_budget > MAX_METADATA_BYTES / 2, "layout refusal is independent of retained allocation exhaustion");
    let ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("typed report") };
    assert_eq!(report.entries.iter().filter(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::RetentionLimit).count(), 2);
}
