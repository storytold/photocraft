//! PDF import end-to-end through PDFium.
//!
//! A synthetic two-page PDF with non-origin red text, a transformed blue rectangle and green text
//! is imported at 72 and 300 DPI, and the resulting editable layers are checked against the
//! fixture's geometry and pixels. PDFium is loaded at run time: when it is unavailable (no
//! `PDFIUM_DIR`, no DLL beside the test binary) the test prints an explicit SKIPPED line and
//! returns instead of pretending the end-to-end wiring passed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use lopdf::{Document as PdfDocument, Object, Stream, dictionary};
use pdfium_render::prelude::{PdfRenderConfig, Pdfium};
use photocraft_doc::{Color, Document, Layer, LayerContent, ShapeLayer, TextLayer};
use photocraft_geom::Rect;
use photocraft_io::{ImportOptions, ImportResult, IoError, import_with_options_and_interrupt};
use photocraft_raster::Interrupt;

const PAGE_W: f64 = 200.0;
const PAGE_H: f64 = 100.0;
const GAP: f64 = 80.0;

/// Imports the four-page fixture at 72 and 300 DPI and checks every layer it produces.
#[test]
fn imports_text_and_a_transformed_path_on_two_pages() {
    let pdf = three_page_pdf();
    for dpi in [72.0f32, 300.0] {
        let options = ImportOptions { pdf_dpi: dpi, ..Default::default() };
        let result = match import_with_options_and_interrupt("fixture.pdf", &pdf, &options, &photocraft_raster::Interrupt::NONE) {
            Ok(result) => result,
            Err(IoError::Unsupported(message)) if message.contains("cannot load PDFium") => {
                eprintln!("SKIPPED the PDF import end-to-end test: PDFium is unavailable ({message})");
                // The Windows integration leg provisions PDFium and sets this: there a missing
                // library must fail rather than let the runner report an unreached pass.
                assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_PDFIUM").is_none(), "PDFium is required for this test but could not load: {message}");
                return;
            }
            Err(error) => panic!("PDF import failed at {dpi} dpi: {error}"),
        };
        check(&result.document, dpi);
        eprintln!("PDF import executed at {dpi} dpi: {} page groups, layer checks passed", result.document.layers.len());
    }
}

/// Page 1: red 48 pt text at (20, 50). Page 2: a 20 x 10 pt rectangle scaled 2x and moved by
/// (10, 5) - corners (30, 25)..(70, 45) - plus green 48 pt text at (20, 20). Page 3: a crop box
/// offset to (10, 20, 210, 120) and rotated 90 degrees clockwise, with a 40 x 40 pt rectangle at
/// (20, 30)..(60, 70) that renders at (10, 10)..(50, 50) on a 100 x 200 page.
fn three_page_pdf() -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let font_id = pdf.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources = pdf.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let pages = [
        ("1 0 0 rg BT /F1 48 Tf 20 50 Td (Red) Tj ET", [0.0f32, 0.0, PAGE_W as f32, PAGE_H as f32], None, None),
        ("0 0 1 rg q 2 0 0 2 10 5 cm 10 10 20 10 re f Q 0 1 0 rg BT /F1 48 Tf 20 20 Td (Green) Tj ET", [0.0, 0.0, PAGE_W as f32, PAGE_H as f32], None, None),
        ("0 0 1 rg 20 30 40 40 re f 0 0.5 0 rg BT /F1 24 Tf 40 60 Td (Rot) Tj ET", [10.0, 20.0, 210.0, 120.0], Some(90i64), None),
        // A CropBox larger than the MediaBox: the rendered page and the mapping use the
        // intersection, so the rectangle must not be offset by the crop box's negative origin.
        ("0 0.5 0 rg 20 30 40 40 re f", [0.0, 0.0, PAGE_W as f32, PAGE_H as f32], None, Some([-50.0f32, -50.0, 250.0, 150.0])),
    ];
    let mut kids = Vec::new();
    for (content, [left, bottom, right, top], rotate, crop) in pages {
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(left), Object::Real(bottom), Object::Real(right), Object::Real(top)]),
            "Contents" => content_id,
            "Resources" => resources,
        };
        if let Some(rotate) = rotate {
            page.set("Rotate", rotate);
        }
        if let Some([left, bottom, right, top]) = crop {
            page.set("CropBox", Object::Array(vec![Object::Real(left), Object::Real(bottom), Object::Real(right), Object::Real(top)]));
        }
        let page_id = pdf.add_object(page);
        kids.push(Object::Reference(page_id));
    }
    let count = kids.len() as i64;
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => Object::Array(kids),
            "Count" => count,
        }),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the PDF fixture");
    bytes
}

/// Asserts the imported text and path layers at one DPI against the fixture geometry.
fn check(document: &Document, dpi: f32) {
    let scale = f64::from(dpi) / 72.0;
    let page_two_x = (PAGE_W * scale).round() + GAP;

    // Page 1: the editable text keeps its position and takes the red ink from the page.
    let text = text_layer(page_children(document, 1));
    assert_red(&text.color, "page 1 text");
    let x = text.transform.m[4];
    let y = text.transform.m[5];
    assert!((x - 20.0 * scale).abs() < 8.0 * scale, "page 1 text x is {x} at {dpi} dpi");
    // 48 pt Helvetica's bounds top is about 35 pt above the baseline at y = 50.
    assert!((y - 15.0 * scale).abs() < 20.0 * scale, "page 1 text y is {y} at {dpi} dpi");

    // Page 2: the transformed rectangle lands at (30, 25)..(70, 45) plus the page's document X.
    let shape = shape_layer(page_children(document, 2));
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for knot in shape.path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
        min_x = min_x.min(knot.anchor.x);
        max_x = max_x.max(knot.anchor.x);
        min_y = min_y.min(knot.anchor.y);
        max_y = max_y.max(knot.anchor.y);
    }
    assert!((min_x - (page_two_x + 30.0 * scale)).abs() < 4.0, "path min x is {min_x}, page 2 starts at {page_two_x} ({dpi} dpi)");
    assert!((max_x - (page_two_x + 70.0 * scale)).abs() < 4.0, "path max x is {max_x} ({dpi} dpi)");
    assert!((min_y - (PAGE_H - 45.0) * scale).abs() < 4.0, "path min y is {min_y} ({dpi} dpi)");
    assert!((max_y - (PAGE_H - 25.0) * scale).abs() < 4.0, "path max y is {max_y} ({dpi} dpi)");

    let text = text_layer(page_children(document, 2));
    assert_green(&text.color, "page 2 text");
    assert!(text.transform.m[4] > page_two_x + 10.0 * scale, "page 2 text x misses the page offset: {} <= {page_two_x}", text.transform.m[4]);

    // Page 3: the crop box (10, 20, 210, 120) and /Rotate 90 put the rectangle at (10..50, 10..50).
    let shape = shape_layer(page_children(document, 3));
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for knot in shape.path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
        min_x = min_x.min(knot.anchor.x);
        max_x = max_x.max(knot.anchor.x);
        min_y = min_y.min(knot.anchor.y);
        max_y = max_y.max(knot.anchor.y);
    }
    let page_three_x = 2.0 * (PAGE_W * scale).round() + 2.0 * GAP;
    assert!((min_x - (page_three_x + 10.0 * scale)).abs() < 3.0, "rotated path min x is {min_x}, page 3 starts at {page_three_x} ({dpi} dpi)");
    assert!((max_x - (page_three_x + 50.0 * scale)).abs() < 3.0, "rotated path max x is {max_x} ({dpi} dpi)");
    assert!((min_y - 10.0 * scale).abs() < 3.0, "rotated path min y is {min_y} ({dpi} dpi)");
    assert!((max_y - 50.0 * scale).abs() < 3.0, "rotated path max y is {max_y} ({dpi} dpi)");

    // Rotated text: the layer's transform carries the 90 degree turn, not just its position.
    let text = text_layer(page_children(document, 3));
    let m = text.transform.m;
    assert!(m[0].abs() < 0.01 && (m[1] - 1.0).abs() < 0.01 && (m[2] + 1.0).abs() < 0.01 && m[3].abs() < 0.01, "page 3 text rotation is {m:?}");
    assert!(m[4] > page_three_x, "page 3 text x misses the page offset: {} <= {page_three_x}", m[4]);
    assert!(text.color.c[1] > 0.3 && text.color.c[0] < 0.2 && text.color.c[2] < 0.2, "page 3 text should be green, got {:?}", text.color.c);

    // Page 4: a CropBox (-50, -50, 250, 150) around a 200 x 100 MediaBox must not offset the rect.
    let shape = shape_layer(page_children(document, 4));
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for knot in shape.path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
        min_x = min_x.min(knot.anchor.x);
        max_x = max_x.max(knot.anchor.x);
        min_y = min_y.min(knot.anchor.y);
        max_y = max_y.max(knot.anchor.y);
    }
    // Page 3 is rotated 90 degrees, so it is 100 pt wide on the strip, not 200.
    let page_four_x = 2.0 * (PAGE_W * scale).round() + (100.0 * scale).round() + 3.0 * GAP;
    assert!((min_x - (page_four_x + 20.0 * scale)).abs() < 3.0, "oversized-crop path min x is {min_x}, page 4 starts at {page_four_x} ({dpi} dpi)");
    assert!((max_x - (page_four_x + 60.0 * scale)).abs() < 3.0, "oversized-crop path max x is {max_x} ({dpi} dpi)");
    assert!((min_y - 30.0 * scale).abs() < 3.0, "oversized-crop path min y is {min_y} ({dpi} dpi)");
    assert!((max_y - 70.0 * scale).abs() < 3.0, "oversized-crop path max y is {max_y} ({dpi} dpi)");
}

/// The layer children of the imported `Page N` group.
fn page_children(document: &Document, page: usize) -> &[Layer] {
    let name = format!("Page {page}");
    let layer = document.layers.iter().find(|layer| layer.name == name).unwrap_or_else(|| panic!("no {name} layer"));
    match &layer.content {
        LayerContent::Group(group) => &group.children,
        _ => panic!("{name} is not a group"),
    }
}

/// The first text layer among `children`.
fn text_layer(children: &[Layer]) -> &TextLayer {
    children
        .iter()
        .find_map(|layer| match &layer.content {
            LayerContent::Text(text) => Some(text),
            _ => None,
        })
        .expect("a text layer")
}

/// The first shape layer among `children`.
fn shape_layer(children: &[Layer]) -> &ShapeLayer {
    children
        .iter()
        .find_map(|layer| match &layer.content {
            LayerContent::Shape(shape) => Some(shape),
            _ => None,
        })
        .expect("a shape layer")
}

/// Asserts `color` is the fixture's red ink.
fn assert_red(color: &Color, what: &str) {
    assert!(color.c[0] > 0.8 && color.c[1] < 0.2 && color.c[2] < 0.2, "{what} should be red, got {:?}", color.c);
}

/// Asserts `color` is the fixture's green ink.
fn assert_green(color: &Color, what: &str) {
    assert!(color.c[1] > 0.8 && color.c[0] < 0.2 && color.c[2] < 0.2, "{what} should be green, got {:?}", color.c);
}

/// Imports `bytes`, or prints SKIPPED and returns `None` when PDFium cannot load. The Windows
/// integration leg sets `PHOTOCRAFT_REQUIRE_PDFIUM`, where a missing library must fail.
fn imported_or_skipped(bytes: &[u8], options: &ImportOptions, ctl: &Interrupt<'_>, what: &str) -> Option<ImportResult> {
    match import_with_options_and_interrupt("probe.pdf", bytes, options, ctl) {
        Ok(result) => Some(result),
        Err(IoError::Unsupported(message)) if message.contains("cannot load PDFium") => {
            eprintln!("SKIPPED {what}: PDFium is unavailable ({message})");
            assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_PDFIUM").is_none(), "PDFium is required for {what} but could not load: {message}");
            None
        }
        Err(error) => panic!("{what} failed: {error}"),
    }
}

/// Six one-page variants for the cancellation and fidelity probes: plain, a CropBox offset to
/// (10, 20, 210, 120), rotations of 90, 180 and 270 degrees, and an oversized CropBox
/// (-50, -50, 250, 150) around the 200 x 100 MediaBox. Each page draws a 40 x 40 blue rectangle
/// and a 10 x 10 red one at (20, 30).
fn variant_pdf() -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let pages: [(Option<i64>, Option<[f32; 4]>); 6] = [
        (None, None),
        (None, Some([10.0, 20.0, 210.0, 120.0])),
        (Some(90), None),
        (Some(180), None),
        (Some(270), None),
        (None, Some([-50.0, -50.0, 250.0, 150.0])),
    ];
    let mut kids = Vec::new();
    for (rotate, crop) in pages {
        let content = b"0 0 1 rg 20 30 40 40 re f 1 0 0 rg 20 30 10 10 re f";
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(PAGE_W as f32), Object::Real(PAGE_H as f32)]),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        };
        if let Some(rotate) = rotate {
            page.set("Rotate", rotate);
        }
        if let Some([left, bottom, right, top]) = crop {
            page.set("CropBox", Object::Array(vec![Object::Real(left), Object::Real(bottom), Object::Real(right), Object::Real(top)]));
        }
        kids.push(Object::Reference(pdf.add_object(page)));
    }
    let count = kids.len() as i64;
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => Object::Array(kids),
            "Count" => count,
        }),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the variant PDF");
    bytes
}

/// A cancel that lands on the last object's check, or on the public boundary after
/// `import_stages`, must be reported as `Cancelled` rather than returning a document.
#[test]
fn cancelling_on_the_last_checks_is_reported() {
    let pdf = variant_pdf();
    let options = ImportOptions { pdf_dpi: 72.0, ..Default::default() };
    let count = AtomicUsize::new(0);
    let cancel = || {
        count.fetch_add(1, Ordering::SeqCst);
        false
    };
    let ctl = Interrupt::cancel_only(&cancel);
    if imported_or_skipped(&pdf, &options, &ctl, "the last-check cancellation test").is_none() {
        return;
    }
    let total = count.load(Ordering::SeqCst);
    assert!(total >= 3, "a complete import should check cancellation at least three times, got {total}");
    for (label, threshold) in [("the last per-object check", total - 1), ("the public boundary check", total)] {
        let seen = AtomicUsize::new(0);
        let cancel = || seen.fetch_add(1, Ordering::SeqCst) + 1 >= threshold;
        let ctl = Interrupt::cancel_only(&cancel);
        let started = Instant::now();
        let result = import_with_options_and_interrupt("probe.pdf", &pdf, &options, &ctl);
        let elapsed = started.elapsed();
        assert_eq!(seen.load(Ordering::SeqCst), threshold, "cancelling on {label} should stop at check {threshold} of {total}");
        match result {
            Err(IoError::Cancelled) => {}
            Ok(_) => panic!("cancelling on {label} returned a document"),
            Err(error) => panic!("cancelling on {label} returned {error:?}"),
        }
        assert!(elapsed < Duration::from_secs(10), "cancelling on {label} took {elapsed:?}");
    }
}

/// Every imported page's stored raster must equal a direct PDFium render of the same page at the
/// same target width: crop intersection, rotation and the strip placement may not move or resample
/// a pixel. At 72, 150 and 300 DPI over the six variants that is 18 page renders.
#[test]
fn raster_matches_a_direct_pdfium_render() {
    let pdf = variant_pdf();
    let mut compared = 0;
    for dpi in [72.0f32, 150.0, 300.0] {
        let options = ImportOptions { pdf_dpi: dpi, ..Default::default() };
        let Some(result) = imported_or_skipped(&pdf, &options, &Interrupt::NONE, "the raster fidelity matrix") else {
            return;
        };
        let pdfium = Pdfium::default();
        let reference = pdfium.load_pdf_from_byte_slice(&pdf, None).expect("reference PDF");
        assert_eq!(reference.pages().len() as usize, result.document.layers.len(), "page count at {dpi} dpi");
        for (index, layer) in result.document.layers.iter().enumerate() {
            let group = match &layer.content {
                LayerContent::Group(group) => group,
                _ => panic!("page layer {} is not a group", layer.name),
            };
            let artboard = group.artboard.as_ref().expect("page artboard").rect;
            let raster = group.children.first().and_then(|layer| layer.surface()).expect("page raster layer");
            let (width, height) = (artboard.width(), artboard.height());
            let mut got = vec![[0u8; 4]; (width as usize) * (height as usize)];
            raster.read_rgba8_into(Rect::from_xywh(artboard.x0, artboard.y0, width, height), &mut got);

            let page = reference.pages().get(index as i32).expect("reference page");
            let bitmap = page.render_with_config(&PdfRenderConfig::new().set_target_width(width as i32)).expect("reference render");
            assert_eq!((bitmap.width() as u32, bitmap.height() as u32), (width, height), "page {} size at {dpi} dpi", index + 1);
            let want = bitmap.as_rgba_bytes();

            let mut max_diff = 0u32;
            let mut sum = 0u64;
            for (pixel, reference) in got.iter().zip(want.as_chunks::<4>().0.iter()) {
                for channel in 0..4 {
                    let diff = (pixel[channel] as i32 - reference[channel] as i32).unsigned_abs();
                    sum += u64::from(diff);
                    max_diff = max_diff.max(diff);
                }
            }
            let channels = u64::from(width) * u64::from(height) * 4;
            let mean = sum as f64 / channels as f64;
            assert!(max_diff <= 1, "page {} at {dpi} dpi differs from the direct render: max channel diff {max_diff}, mean {mean:.4}", index + 1);
            compared += 1;
        }
    }
    assert_eq!(compared, 18, "six pages at three DPIs should all be compared");
}
