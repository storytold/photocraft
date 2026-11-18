//! #15650: a PDF-compatible AI that another application placed with its artwork's bounds as the
//! frame keeps that frame when the layer is re-rendered, instead of filling it with the page.

use super::*;

/// A 400 × 300 pt artboard with a red 100 × 50 pt rectangle whose top-left is at (100, 50).
fn artwork() -> Vec<u8> {
    let content = "1 0 0 rg 100 200 100 50 re f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 4 0 R /Resources << >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// A 200 × 150 document with the artwork placed as Photoshop places it: its 100 × 50 bounds
/// are the frame, drawn at (20, 30), with the pixels Photoshop saved as the cache.
fn placed(frame: (f64, f64)) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150, "background": "transparent"})).unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let transform = Affine::translate(20.0, 30.0);
    let mut cache = Surface::new(doc.pixel_format());
    cache.fill_rect(Rect::new(20, 30, 120, 80), &RED);
    let source = SmartSource::Embedded { file_name: "Logo.ai".into(), bytes: Arc::new(artwork()) };
    let mut sm = SmartObject::new(source, transform, Some(cache));
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "logo",
        placed: "logo",
        transform,
        perspective: None,
        size: frame,
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    sm.psd_raw = Some(Arc::new(photocraft_io::smart_map::sold_bytes(None, &spec, &mut vec![])));
    let layer = Layer::new("Logo", LayerContent::Smart(sm));
    let id = layer.id;
    doc.layers.push(layer);
    (s, id)
}

fn cache(s: &Session, id: LayerId) -> Surface {
    smart(&s.active().unwrap().doc, id).unwrap().cache.clone().unwrap()
}

#[test]
fn image_size_keeps_a_placed_ai_at_its_placed_bounds() {
    let (mut s, id) = placed((100.0, 50.0));
    s.execute("image.imageSize", json!({"width": 180, "height": 135})).unwrap();
    let px = cache(&s, id);
    let b = px.content_bounds();
    assert!((17..=19).contains(&b.x0) && (26..=28).contains(&b.y0) && (107..=109).contains(&b.x1) && (71..=73).contains(&b.y1), "{b:?}");
    let centre = px.pixel(63, 49);
    assert!(centre.iter().zip(RED).all(|(a, b)| (a - b).abs() < 0.02), "{centre:?}");
}

#[test]
fn a_placed_ai_re_renders_in_its_frame() {
    let (s, id) = placed((100.0, 50.0));
    let doc = &s.active().unwrap().doc;
    let rendered = render(doc, smart(doc, id).unwrap()).unwrap().unwrap();
    assert_eq!(rendered.content_bounds(), Rect::new(20, 30, 120, 80));
}

#[test]
fn a_frame_matching_neither_the_artwork_nor_the_page_keeps_the_saved_pixels() {
    let (mut s, id) = placed((100.0, 100.0));
    let doc = &s.active().unwrap().doc;
    assert!(render(doc, smart(doc, id).unwrap()).is_err());
    s.execute("image.imageSize", json!({"width": 100, "height": 75})).unwrap();
    assert_eq!(cache(&s, id).content_bounds(), Rect::new(10, 15, 60, 40), "the saved pixels are scaled instead");
}

#[test]
fn a_frame_that_is_the_pdfs_own_canvas_opens_the_pdf_as_usual() {
    let canvas = photocraft_io::pdf_default_canvas(&artwork()).unwrap();
    let (s, id) = placed((canvas.0 as f64, canvas.1 as f64));
    let doc = &s.active().unwrap().doc;
    let rendered = render(doc, smart(doc, id).unwrap()).unwrap().unwrap();
    // PhotoCraft's own placement: the whole page, on white, from the placement's origin.
    assert_eq!(rendered.pixel(25, 35), vec![1.0, 1.0, 1.0, 1.0]);
    assert_eq!(rendered.content_bounds(), Rect::new(20, 30, 20 + canvas.0 as i32, 30 + canvas.1 as i32));
}

#[test]
fn convert_to_layers_keeps_a_placed_ai_at_its_placed_bounds() {
    let (mut s, id) = placed((100.0, 50.0));
    s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap();
    let doc = &s.active().unwrap().doc;
    let LayerContent::Raster(px) = &doc.layers.last().unwrap().content else { panic!("a pixel layer: {:?}", doc.layers.last().unwrap().content.kind_name()) };
    assert_eq!(px.content_bounds(), Rect::new(20, 30, 120, 80));
}
