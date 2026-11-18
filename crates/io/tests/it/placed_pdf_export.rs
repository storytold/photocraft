//! #15650: a PDF-compatible AI that Photoshop placed with its artwork's bounds as the frame,
//! given a smart filter in PhotoCraft, is saved with unfiltered pixels at its placed bounds
//! rather than the whole page drawn into the frame.

use std::sync::Arc;

use craft_psd::PsdFile;
use lopdf::{Document as PdfDocument, Object, Stream, dictionary};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_io::{ExportOptions, export, linked, smart_map};
use photocraft_raster::Surface;

/// A 400 × 300 pt artboard with a red 100 × 50 pt rectangle whose top-left is at (100, 50).
fn artwork() -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let content_id = pdf.add_object(Stream::new(dictionary! {}, b"1 0 0 rg 100 200 100 50 re f".to_vec()));
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 400.into(), 300.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {},
    });
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }));
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

/// A 200 × 150 document holding the artwork as a Photoshop import leaves it: the file embedded
/// in `lnk2`, and a placed block whose frame is the artwork's 100 × 50 bounds, drawn at (20, 30).
fn photoshop_placed() -> Document {
    let mut doc = Document::new("placed", Size::new(200, 150), ColorMode::Rgb, SampleType::U8);
    let file = linked::LinkedFile { uuid: "logo".into(), file_name: "Logo.ai".into(), bytes: artwork() };
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(linked::encode_linked_file(&file))));
    let transform = Affine::translate(20.0, 30.0);
    let spec =
        smart_map::PlacedSpec { idnt: "logo", placed: "logo", transform, perspective: None, size: (100.0, 50.0), dpi: 72.0, warp: None, filter_fx: None };
    let sold = Arc::new(smart_map::sold_bytes(None, &spec, &mut vec![]));
    let mut cache = Surface::new(doc.pixel_format());
    cache.fill_rect(Rect::new(20, 30, 120, 80), &[1.0, 0.0, 0.0, 1.0]);
    let mut sm = SmartObject::new(SmartSource::Linked { path: "logo".into() }, transform, Some(cache));
    sm.psd_raw = Some(sold.clone());
    sm.smart_filters.push(SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({ "radius": 1.0 }),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    let mut layer = Layer::new("Logo", LayerContent::Smart(sm));
    layer.psd_blocks.push((*b"SoLd", sold));
    doc.layers.push(layer);
    doc
}

#[test]
fn unfiltered_pixels_of_a_placed_ai_keep_its_placed_bounds() {
    let doc = photoshop_placed();
    let out = export(&doc, "psd", &ExportOptions::default()).unwrap();
    let file = PsdFile::from_bytes(&out.bytes).unwrap();
    let block = file.global_blocks.iter().find(|b| &b.key == b"FEid").expect("the filter cache is written");
    let fx = craft_psd::filter_effects::FilterEffects::parse(&block.data).unwrap();
    let unfiltered = smart_map::unfiltered_from_item(&fx.items[0], doc.pixel_format()).expect("unfiltered pixels");
    // The planes cover the canvas; outside the artwork they are transparent.
    let opaque = |x, y| unfiltered.pixel(x, y)[3] > 0.0;
    assert!(opaque(20, 30) && opaque(119, 79) && unfiltered.pixel(60, 50)[..3] == [1.0, 0.0, 0.0]);
    for (x, y) in [(19, 30), (20, 29), (120, 79), (119, 80), (150, 100), (199, 149)] {
        assert!(!opaque(x, y), "({x}, {y}) is outside the placed artwork");
    }
}
