//! #15650: a PDF-compatible AI that Photoshop placed with its artwork's bounds as the frame,
//! given a smart filter in PhotoCraft, is saved with unfiltered pixels at its placed bounds
//! rather than the whole page drawn into the frame.

use std::sync::Arc;

use craft_psd::PsdFile;
use craft_psd::descriptor::Value;
use lopdf::{Document as PdfDocument, Object, Stream, dictionary};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_io::{ExportOptions, export, linked, smart_map};
use photocraft_raster::Surface;

const RED: &str = "1 0 0 rg 100 200 100 50 re f";

/// 400 × 300 pt artboards, one per entry of `contents`.
fn pdf(contents: &[&str]) -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let mut kids = Vec::new();
    for content in contents {
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        kids.push(Object::Reference(pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 400.into(), 300.into()],
            "Contents" => content_id,
            "Resources" => dictionary! {},
        })));
    }
    let count = kids.len() as i64;
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

/// A 200 × 150 document holding the PDF `bytes` as a Photoshop import leaves it: the file
/// embedded in `lnk2`, and a placed layer per entry of `pages` whose frame is the 100 × 50
/// artwork's bounds on that page, drawn at (20, 30).
fn photoshop_placed_pages(bytes: Vec<u8>, pages: &[i32]) -> Document {
    let mut doc = Document::new("placed", Size::new(200, 150), ColorMode::Rgb, SampleType::U8);
    let file = linked::LinkedFile { uuid: "logo".into(), file_name: "Logo.ai".into(), bytes };
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(linked::encode_linked_file(&file))));
    for &page in pages {
        let mut layer = placed_layer(&doc, page);
        // Duplicates of one placed layer share their contents.
        if let (Some(LayerContent::Smart(first)), LayerContent::Smart(sm)) = (doc.layers.first().map(|l| &l.content), &mut layer.content) {
            sm.contents_id = first.contents_id;
        }
        doc.layers.push(layer);
    }
    doc
}

fn placed_layer(doc: &Document, page: i32) -> Layer {
    let transform = Affine::translate(20.0, 30.0);
    let spec =
        smart_map::PlacedSpec { idnt: "logo", placed: "logo", transform, perspective: None, size: (100.0, 50.0), dpi: 72.0, warp: None, filter_fx: None };
    let mut template = smart_map::sold_descriptor(&smart_map::sold_bytes(None, &spec, &mut vec![])).unwrap();
    template.items.iter_mut().filter(|(k, _)| k.is("PgNm")).for_each(|(_, v)| *v = Value::Integer(page));
    let sold = Arc::new(smart_map::sold_bytes(Some(&template), &spec, &mut vec![]));
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
    let mut layer = Layer::new(format!("Logo page {page}"), LayerContent::Smart(sm));
    layer.psd_blocks.push((*b"SoLd", sold));
    layer
}

fn photoshop_placed() -> Document {
    photoshop_placed_pages(pdf(&[RED]), &[1])
}

/// The unfiltered pixels the PSD written for `doc` keeps for each filtered layer, in order.
fn unfiltered(doc: &Document) -> Vec<Surface> {
    let out = export(doc, "psd", &ExportOptions::default()).unwrap();
    let file = PsdFile::from_bytes(&out.bytes).unwrap();
    let block = file.global_blocks.iter().find(|b| &b.key == b"FEid").expect("the filter cache is written");
    let fx = craft_psd::filter_effects::FilterEffects::parse(&block.data).unwrap();
    fx.items.iter().map(|item| smart_map::unfiltered_from_item(item, doc.pixel_format()).expect("unfiltered pixels")).collect()
}

#[test]
fn unfiltered_pixels_of_a_placed_ai_keep_its_placed_bounds() {
    let unfiltered = unfiltered(&photoshop_placed()).remove(0);
    // The planes cover the canvas; outside the artwork they are transparent.
    let opaque = |x, y| unfiltered.pixel(x, y)[3] > 0.0;
    assert!(opaque(20, 30) && opaque(119, 79) && unfiltered.pixel(60, 50)[..3] == [1.0, 0.0, 0.0]);
    for (x, y) in [(19, 30), (20, 29), (120, 79), (119, 80), (150, 100), (199, 149)] {
        assert!(!opaque(x, y), "({x}, {y}) is outside the placed artwork");
    }
}

#[test]
fn placements_of_different_pages_keep_their_own_unfiltered_pixels() {
    let doc = photoshop_placed_pages(pdf(&[RED, "0 0 1 rg 100 200 100 50 re f"]), &[1, 2]);
    let colours: Vec<Vec<f32>> = unfiltered(&doc).iter().map(|px| px.pixel(60, 50)[..3].to_vec()).collect();
    assert_eq!(colours, [vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0]], "page 1's red artwork, then page 2's blue");
}
