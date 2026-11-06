//! PDF import end-to-end through PDFium.
//!
//! A synthetic two-page PDF with non-origin red text, a transformed blue rectangle and green text
//! is imported at 72 and 300 DPI, and the resulting editable layers are checked against the
//! fixture's geometry and pixels. PDFium is loaded at run time: when it is unavailable (no
//! `PDFIUM_DIR`, no DLL beside the test binary) the test prints an explicit SKIPPED line and
//! returns instead of pretending the end-to-end wiring passed.

use lopdf::{Document as PdfDocument, Object, Stream, dictionary};
use photocraft_doc::{Color, Document, Layer, LayerContent, ShapeLayer, TextLayer};
use photocraft_io::{ImportOptions, IoError, import_with_options_and_interrupt};

const PAGE_W: f64 = 200.0;
const PAGE_H: f64 = 100.0;
const GAP: f64 = 80.0;

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

fn page_children(document: &Document, page: usize) -> &[Layer] {
    let name = format!("Page {page}");
    let layer = document.layers.iter().find(|layer| layer.name == name).unwrap_or_else(|| panic!("no {name} layer"));
    match &layer.content {
        LayerContent::Group(group) => &group.children,
        _ => panic!("{name} is not a group"),
    }
}

fn text_layer(children: &[Layer]) -> &TextLayer {
    children
        .iter()
        .find_map(|layer| match &layer.content {
            LayerContent::Text(text) => Some(text),
            _ => None,
        })
        .expect("a text layer")
}

fn shape_layer(children: &[Layer]) -> &ShapeLayer {
    children
        .iter()
        .find_map(|layer| match &layer.content {
            LayerContent::Shape(shape) => Some(shape),
            _ => None,
        })
        .expect("a shape layer")
}

fn assert_red(color: &Color, what: &str) {
    assert!(color.c[0] > 0.8 && color.c[1] < 0.2 && color.c[2] < 0.2, "{what} should be red, got {:?}", color.c);
}

fn assert_green(color: &Color, what: &str) {
    assert!(color.c[1] > 0.8 && color.c[0] < 0.2 && color.c[2] < 0.2, "{what} should be green, got {:?}", color.c);
}
