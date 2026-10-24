//! Source facts come from the published procedural generator; expected documents are built
//! with ordinary native constructors, never with the XCF converter or GIMP's merged PNG.
#![cfg(feature = "corpus")]

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

fn open(name: &str) -> Document {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --gimp", path.display()));
    let result = photocraft_io::import(name, &bytes).unwrap();
    assert!(result.source_read_only && !result.preview_only);
    result.document
}

fn pattern(x: u32, y: u32, c: u32, max: u32) -> f32 {
    ((x * 31 + y * 17 + c * 101 + 7) * 13 % (max + 1)) as f32 / max as f32
}

fn raster(name: &str, fmt: PixelFormat, width: u32, height: u32, offset: (i32, i32), seed: u32, opaque: bool) -> Layer {
    let mut layer = Layer::raster(name, fmt);
    let max = if fmt.sample == SampleType::U16 { 65535 } else { 255 };
    let colors = if fmt.mode == ColorMode::Grayscale { 1 } else { 3 };
    let surface = layer.surface_mut().unwrap();
    for y in 0..height {
        for x in 0..width {
            let mut px: Vec<_> = (0..colors).map(|c| pattern(x + seed, y, c, max)).collect();
            px.push(if opaque { 1.0 } else { ((x + y) * max / (width + height - 4).max(1)).min(max) as f32 / max as f32 });
            surface.write_pixel(x as i32 + offset.0, y as i32 + offset.1, &px);
        }
    }
    layer.locks.transparency = opaque;
    layer
}

fn assert_layer(actual: &Layer, expected: &Layer) {
    assert_eq!(
        (&actual.name, actual.blend, actual.opacity, actual.visible, actual.clipped, actual.locks, actual.label),
        (&expected.name, expected.blend, expected.opacity, expected.visible, expected.clipped, expected.locks, expected.label)
    );
    match (&actual.content, &expected.content) {
        (LayerContent::Raster(a), LayerContent::Raster(b)) => {
            assert_eq!(a.read_region(Rect::new(-10, -5, 45, 35)), b.read_region(Rect::new(-10, -5, 45, 35)), "{} pixels including off-canvas", actual.name)
        }
        (LayerContent::Group(a), LayerContent::Group(b)) => {
            assert_eq!(a.expanded, b.expanded);
            assert_eq!(a.children.len(), b.children.len());
            for (a, b) in a.children.iter().zip(&b.children) {
                assert_layer(a, b);
            }
        }
        _ => panic!("unexpected native layer kind: {}", actual.name),
    }
    match (&actual.mask, &expected.mask) {
        (Some(a), Some(b)) => {
            assert_eq!((a.enabled, a.linked, a.density, a.feather), (b.enabled, b.linked, b.density, b.feather));
            let a = a.surface.read_region(Rect::new(0, 0, 40, 30));
            let b = b.surface.read_region(Rect::new(0, 0, 40, 30));
            assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0), "native mask coverage");
        }
        (None, None) => {}
        _ => panic!("unexpected mask: {}", actual.name),
    }
}

fn compare(actual: Document, expected: Document) {
    assert_eq!((actual.size, actual.mode, actual.depth), (expected.size, expected.mode, expected.depth));
    assert_eq!(actual.layers.len(), expected.layers.len());
    for (a, b) in actual.layers.iter().zip(&expected.layers) {
        assert_layer(a, b);
    }
    let a = photocraft_compose::flatten(&actual);
    let b = photocraft_compose::flatten(&expected);
    assert!(a.px.iter().zip(b.px).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 2.0 / 255.0)), "ordinary native composition");
}

#[test]
fn rgb_layers_and_off_canvas_pixels_match_native_data_in_both_source_spaces() {
    for name in ["layers-8bit.xcf", "layers-8bit-perceptual.xcf"] {
        let mut expected = Document::new("Native", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
        let fmt = expected.pixel_format();
        let bg = raster("Background", fmt, 40, 30, (0, 0), 0, true);
        let mut multiply = raster("Multiply 60%", fmt, 20, 16, (5, 4), 3, false);
        multiply.blend = BlendMode::Multiply;
        multiply.opacity = 0.6;
        multiply.locks.transparency = true;
        multiply.label = photocraft_doc::LabelColor::Green;
        let mut screen = raster("Screen off-canvas", fmt, 24, 12, (-8, 20), 7, false);
        screen.blend = BlendMode::Screen;
        let mut hidden = raster("Hidden", fmt, 10, 10, (28, 2), 11, false);
        hidden.visible = false;
        expected.layers = vec![bg, multiply, screen, hidden];
        compare(open(name), expected);
    }
}

#[test]
fn isolated_and_pass_through_groups_match_native_groups() {
    let mut expected = Document::new("Native", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
    let fmt = expected.pixel_format();
    let bg = raster("Background", fmt, 40, 30, (0, 0), 0, false);
    let mut multiply = raster("Child Multiply", fmt, 20, 20, (3, 3), 2, false);
    multiply.blend = BlendMode::Multiply;
    let mut screen = raster("Nested child Screen", fmt, 16, 12, (18, 10), 5, false);
    screen.blend = BlendMode::Screen;
    let mut nested = Layer::group("Nested pass-through", vec![screen]);
    nested.blend = BlendMode::PassThrough;
    let mut group = Layer::group("Group", vec![multiply, nested]);
    group.blend = BlendMode::Normal;
    group.opacity = 0.7;
    expected.layers = vec![bg, group, raster("Top", fmt, 12, 8, (26, 20), 9, false)];
    compare(open("groups-perceptual.xcf"), expected);
}

#[test]
fn gray16_matches_native_gray16_without_reducing_to_rgb8() {
    let mut expected = Document::new("Native", Size::new(40, 30), ColorMode::Grayscale, SampleType::U16);
    let fmt = expected.pixel_format();
    expected.layers = vec![raster("Background", fmt, 40, 30, (0, 0), 0, true), raster("Gray alpha", fmt, 20, 15, (10, 8), 9, false)];
    compare(open("gray16.xcf"), expected);
}

#[test]
fn enabled_and_disabled_masks_remain_native_coverage_masks() {
    let mut expected = Document::new("Native", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
    let fmt = expected.pixel_format();
    let mut bg = Layer::raster("Background", fmt);
    bg.surface_mut().unwrap().write_region(Rect::new(0, 0, 40, 30), &[200.0 / 255.0, 200.0 / 255.0, 200.0 / 255.0, 1.0].repeat(1200));
    bg.locks.transparency = true;
    let mut masked = raster("Masked", fmt, 30, 20, (5, 5), 4, false);
    let mut coverage = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    for y in 0..20 {
        for x in 0..30 {
            let s = (x * 255 / 29) as f32 / 255.0;
            let linear = if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) };
            coverage.write_pixel(x + 5, y + 5, &[linear]);
        }
    }
    masked.mask = Some(LayerMask { surface: coverage, enabled: true, linked: true, density: 1.0, feather: 0.0 });
    let mut disabled = raster("Mask disabled", fmt, 20, 10, (15, 18), 6, false);
    disabled.mask = Some(LayerMask {
        surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)),
        enabled: false,
        linked: true,
        density: 1.0,
        feather: 0.0,
    });
    expected.layers = vec![bg, masked, disabled];
    compare(open("mask-perceptual.xcf"), expected);
}

#[test]
fn cancellation_during_a_real_layered_import_does_not_publish_a_partial_result() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/layers-8bit.xcf");
    let bytes = std::fs::read(path).unwrap();
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let progress = |p| {
        if p >= 0.05 {
            cancelled.store(true, Ordering::Relaxed);
        }
    };
    let ctl = photocraft_raster::Interrupt::new(&cancel, &progress);
    assert!(matches!(photocraft_io::import_with("layers-8bit.xcf", &bytes, &ctl), Err(photocraft_io::IoError::Cancelled)));
}

#[test]
fn published_text_becomes_native_editable_data_with_inert_configuration() {
    use photocraft_doc::import_conversion::{ConversionDisposition as D, ImportConversion};
    let doc = open("text-perceptual.xcf");
    let LayerContent::Text(text) = &doc.layers[1].content else { panic!("native editable text") };
    assert_eq!(text.text, "XCF");
    assert!(text.cache.is_none(), "source pixels are never a valid native editable-text cache");
    let Some(ImportConversion::Report(report)) = &doc.metadata.import_conversion else { panic!("missing report") };
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.feature_code == "xcf.text" && e.disposition == D::NativeApproximation && e.native_description.contains("editable text"))
    );
    assert!(
        report.attachments.iter().any(|a| a.bytes.windows(b"gimp-text-layer".len()).any(|w| w == b"gimp-text-layer")),
        "the published source text configuration is retained without execution"
    );
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(doc.metadata.import_conversion, reopened.metadata.import_conversion);
    let LayerContent::Text(reopened_text) = &reopened.layers[1].content else { panic!("reopened editable text") };
    assert_eq!(text.text, reopened_text.text);
    assert_eq!(text.runs, reopened_text.runs);
    assert_eq!(text.transform, reopened_text.transform);
}

#[test]
fn native_reopen_warns_once_about_historical_outcomes_and_remains_writable() {
    let doc = open("text-perceptual.xcf");
    let native = photocraft_io::export(&doc, "converted.pcraft", &Default::default()).unwrap();
    assert!(!native.warnings.iter().any(|w| w.contains("not exported")));
    let reopened = photocraft_io::import("converted.pcraft", &native.bytes).unwrap();
    assert!(!reopened.source_read_only && !reopened.preview_only);
    assert_eq!(reopened.document.metadata.import_conversion, doc.metadata.import_conversion);
    assert_eq!(reopened.warnings.iter().filter(|w| w.contains("historical import conversion report")).count(), 1);
}

#[test]
fn ordinary_psd_and_flat_exports_keep_native_output_and_disclose_report_loss() {
    for name in ["layers-8bit-perceptual.xcf", "groups-perceptual.xcf", "mask-perceptual.xcf"] {
        let doc = open(name);
        let native = photocraft_compose::flatten(&doc);
        for extension in ["psd", "png"] {
            let exported = photocraft_io::export(&doc, extension, &Default::default()).unwrap();
            assert_eq!(exported.warnings.iter().filter(|w| w.contains("import conversion report") && w.contains("not exported")).count(), 1);
            let reopened = photocraft_io::import(&format!("exported.{extension}"), &exported.bytes).unwrap().document;
            assert!(reopened.metadata.import_conversion.is_none());
            let rendered = photocraft_compose::flatten(&reopened);
            assert_eq!(native.px.len(), rendered.px.len());
            assert!(
                native.px.iter().zip(rendered.px).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 2.5 / 255.0)),
                "native {name} -> {extension} output"
            );
            assert!(!reopened.metadata.text.iter().any(|(name, _)| name.contains("source_property") || name.contains("importConversion")));
        }
    }
}
