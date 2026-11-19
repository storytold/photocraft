use lopdf::{Document as PdfDocument, Object, Stream, dictionary};

use super::*;
use crate::smart_map::{PlacedSpec, placed_pdf, sold_bytes, sold_descriptor};

/// A PDF with one `width` × `height` pt page per entry of `contents` (PDF space, y up).
fn pdf_pages(width: f64, height: f64, contents: &[&str]) -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let mut kids = Vec::new();
    for content in contents {
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        kids.push(Object::Reference(pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![0.0, 0.0, width, height].into_iter().map(|v| Object::Real(v as f32)).collect()),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        })));
    }
    let count = kids.len() as i64;
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the PDF");
    bytes
}

/// A 400 × 300 pt artboard with a red 100 × 50 pt rectangle whose top-left is at (100, 50).
fn artwork() -> Vec<u8> {
    pdf_pages(400.0, 300.0, &["1 0 0 rg 100 200 100 50 re f"])
}

fn placed(frame: (f64, f64)) -> PlacedPdf {
    PlacedPdf { frame, page: 1, dpi: None }
}

fn draw(bytes: &[u8], placed: &PlacedPdf) -> Result<Document, IoError> {
    import_placed("Logo.ai", bytes, placed, &Interrupt::NONE)
}

fn rgba(doc: &Document, x: i32, y: i32) -> Vec<f32> {
    let LayerContent::Raster(surface) = &doc.layers[0].content else { panic!("a raster layer") };
    surface.pixel(x, y)
}

fn alpha(doc: &Document, x: i32, y: i32) -> f32 {
    rgba(doc, x, y)[3]
}

#[test]
fn a_frame_with_the_artworks_proportions_is_filled_by_the_artwork_alone() {
    let doc = draw(&artwork(), &placed((200.0, 100.0))).unwrap();
    assert_eq!(doc.size, Size::new(200, 100));
    for (x, y) in [(0, 0), (199, 0), (0, 99), (199, 99), (100, 50)] {
        let px = rgba(&doc, x, y);
        assert!(px[3] > 0.99 && px[0] > 0.99 && px[1] < 0.01, "({x}, {y}) is the red artwork, not the page: {px:?}");
    }
}

#[test]
fn a_fractional_frame_rounds_up_and_keeps_the_artwork_at_its_scale() {
    let doc = draw(&artwork(), &placed((60.5, 30.25))).unwrap();
    assert_eq!(doc.size, Size::new(61, 31));
    assert!(alpha(&doc, 0, 0) > 0.99 && alpha(&doc, 59, 29) > 0.99);
}

#[test]
fn a_frame_with_the_pages_proportions_shows_the_page_on_transparency() {
    let doc = draw(&artwork(), &placed((800.0, 600.0))).unwrap();
    assert_eq!(doc.size, Size::new(800, 600));
    assert_eq!(alpha(&doc, 10, 10), 0.0, "the page around the artwork stays transparent, not white");
    assert!(alpha(&doc, 300, 150) > 0.99, "the artwork sits where the page has it, at twice its size");
    assert_eq!(alpha(&doc, 300, 205), 0.0);
}

#[test]
fn the_stored_resolution_decides_when_artwork_and_page_share_proportions() {
    // A centred 200 × 150 pt artwork on a 400 × 300 pt page: both are 4:3.
    let bytes = pdf_pages(400.0, 300.0, &["1 0 0 rg 100 75 200 150 re f"]);
    // 800 × 600 px at 144 ppi is 400 × 300 pt: the page.
    let page = draw(&bytes, &PlacedPdf { frame: (800.0, 600.0), page: 1, dpi: Some(144.0) }).unwrap();
    assert_eq!(alpha(&page, 10, 10), 0.0, "cropped to the page, whose corner is empty");
    assert!(alpha(&page, 400, 300) > 0.99);
    // At 288 ppi the same frame is 200 × 150 pt: the artwork.
    let art = draw(&bytes, &PlacedPdf { frame: (800.0, 600.0), page: 1, dpi: Some(288.0) }).unwrap();
    assert!(alpha(&art, 2, 2) > 0.99, "cropped to the artwork, which fills the frame");
    // Without a resolution the artwork wins, as Photoshop's default crop for artwork is its bounds.
    assert!(alpha(&draw(&bytes, &placed((800.0, 600.0))).unwrap(), 2, 2) > 0.99);
}

#[test]
fn tiny_artwork_does_not_match_a_frame_of_other_proportions() {
    // A 1 × 0.6 pt rectangle on a 400 × 300 pt page measures about 6 × 4 px on the probe, within
    // the probe's tolerance for a 2:1 frame; measured finely it is 5:3.
    let bytes = pdf_pages(400.0, 300.0, &["0 0 1 rg 100 100 1 0.6 re f"]);
    assert!(draw(&bytes, &placed((100.0, 50.0))).is_err());
    assert!(draw(&bytes, &placed((100.0, 60.0))).is_ok());
}

#[test]
fn the_stored_page_is_drawn() {
    let bytes = pdf_pages(400.0, 300.0, &["1 0 0 rg 0 0 400 300 re f", "0 0 1 rg 100 200 100 50 re f"]);
    let doc = draw(&bytes, &PlacedPdf { frame: (100.0, 50.0), page: 2, dpi: None }).unwrap();
    let px = rgba(&doc, 50, 25);
    assert!(px[2] > 0.99 && px[0] < 0.01, "page 2's blue artwork: {px:?}");
    assert!(draw(&bytes, &PlacedPdf { frame: (100.0, 50.0), page: 3, dpi: None }).is_err(), "there is no page 3");
}

#[test]
fn a_frame_matching_neither_box_is_refused() {
    let error = draw(&artwork(), &placed((100.0, 100.0))).unwrap_err();
    assert!(error.to_string().contains("cropped"), "{error}");
}

#[test]
fn an_empty_or_unreadable_frame_or_pdf_is_refused() {
    for frame in [(0.0, 100.0), (f64::NAN, 100.0), (100_000.0, 50_000.0)] {
        assert!(draw(&artwork(), &placed(frame)).is_err(), "{frame:?}");
    }
    assert!(draw(b"%PDF-1.6\nnot a pdf", &placed((200.0, 100.0))).is_err());
}

fn sold(frame: (f64, f64)) -> Vec<u8> {
    let spec = PlacedSpec {
        idnt: "logo",
        placed: "logo",
        transform: photocraft_geom::Affine::IDENTITY,
        perspective: None,
        size: frame,
        dpi: 300.0,
        warp: None,
        filter_fx: None,
    };
    sold_bytes(None, &spec, &mut vec![])
}

#[test]
fn the_placed_block_gives_frame_page_and_resolution() {
    let d = sold_descriptor(&sold((100.0, 50.0))).unwrap();
    assert_eq!(placed_pdf(&d), Some(PlacedPdf { frame: (100.0, 50.0), page: 1, dpi: Some(300.0) }));
    let mut d2 = d.clone();
    d2.items.retain(|(k, _)| !k.is("PgNm"));
    d2 = d2.with("PgNm", craft_psd::descriptor::Value::Integer(3));
    assert_eq!(placed_pdf(&d2).map(|p| p.page), Some(3));
    for bad in [0, -1] {
        let mut d3 = d.clone();
        d3.items.retain(|(k, _)| !k.is("PgNm"));
        assert_eq!(placed_pdf(&d3.with("PgNm", craft_psd::descriptor::Value::Integer(bad))), None, "page {bad}");
    }
}

#[test]
fn only_a_pdf_placed_in_a_frame_other_than_its_own_canvas_is_a_placement() {
    let bytes = artwork();
    assert_eq!(super::super::placement(&bytes, &sold((100.0, 50.0))).map(|p| p.frame), Some((100.0, 50.0)));
    let canvas = super::super::default_canvas(&bytes).unwrap();
    assert_eq!(super::super::placement(&bytes, &sold((canvas.0 as f64, canvas.1 as f64))), None, "PhotoCraft's own placement");
    assert_eq!(super::super::placement(b"\x89PNG not a pdf", &sold((100.0, 50.0))), None);
}

#[test]
fn the_default_canvas_is_the_imported_pages_size() {
    for bytes in [artwork(), pdf_pages(400.0, 300.0, &["", ""])] {
        let imported = super::super::import("Logo.ai", &bytes, &crate::ImportOptions::default(), &Interrupt::NONE).unwrap().document;
        assert_eq!(super::super::default_canvas(&bytes), Some((u64::from(imported.size.width), u64::from(imported.size.height))));
    }
    assert_eq!(super::super::default_canvas(b"%PDF-1.6\nnot a pdf"), None);
}

#[test]
fn with_a_stored_resolution_the_box_must_also_have_the_frames_size() {
    // The 100 × 50 pt artwork has a 200 × 100 px frame's proportions, but at 72 ppi that frame is
    // 200 × 100 pt: a larger box of the same shape, around the artwork, that isn't tried.
    assert!(draw(&artwork(), &PlacedPdf { frame: (200.0, 100.0), page: 1, dpi: Some(72.0) }).is_err());
    assert!(draw(&artwork(), &PlacedPdf { frame: (200.0, 100.0), page: 1, dpi: Some(144.0) }).is_ok());
    // Likewise the 400 × 300 pt page and an 800 × 600 px frame.
    assert!(draw(&artwork(), &PlacedPdf { frame: (800.0, 600.0), page: 1, dpi: Some(72.0) }).is_err());
    assert!(draw(&artwork(), &PlacedPdf { frame: (800.0, 600.0), page: 1, dpi: Some(144.0) }).is_ok());
}

/// [`artwork`] with a 10 × 10 pt image over it whose JPEG data can't be decoded.
fn artwork_with_a_broken_image() -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let image = pdf.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 8, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" },
        b"not a jpeg".to_vec(),
    ));
    let content = pdf.add_object(Stream::new(dictionary! {}, b"1 0 0 rg 100 200 100 50 re f q 10 0 0 10 120 210 cm /Im0 Do Q".to_vec()));
    let page = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 400.into(), 300.into()],
        "Contents" => content,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image } },
    });
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the PDF");
    bytes
}

#[test]
fn a_page_that_cant_be_drawn_completely_is_refused() {
    let error = draw(&artwork_with_a_broken_image(), &placed((200.0, 100.0))).unwrap_err();
    assert!(error.to_string().contains("can't be drawn completely"), "{error}");
}

#[test]
fn only_running_out_of_memory_is_out_of_memory() {
    assert!(matches!(render_failure(1, 1.0, raster::Error::Allocation), IoError::OutOfMemory(_)));
    for e in [raster::Error::OutputLimit, raster::Error::InvalidPage, raster::Error::BackendPanic] {
        assert!(matches!(render_failure(1, 1.0, e), IoError::Unsupported(_)));
    }
}
