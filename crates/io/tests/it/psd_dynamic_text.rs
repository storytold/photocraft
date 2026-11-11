//! Dynamic Text saves to PSD as the plain type it bakes to: the document's `Txt2` describes the
//! baked layer, as its `TySh` does, not the fitted layer's base sizes.

use photocraft_doc::text::{CharStyle, Kerning, TextFit, TextRun, TextShape};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};

fn txt2(doc: &Document) -> Vec<u8> {
    let bytes = photocraft_io::export(doc, "d.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    file.global_blocks.iter().find(|b| &b.key == b"Txt2").map(|b| b.data.clone()).expect("Txt2 written for Optical kerning")
}

fn doc_with(t: TextLayer) -> Document {
    let mut doc = Document::new("d", photocraft_geom::Size::new(320, 160), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t = t;
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    doc.layers.push(Layer::new("fitted", LayerContent::Text(t)));
    doc
}

#[test]
fn txt2_describes_the_baked_text() {
    let text = "Hello dynamic world of text";
    // Optical kerning lives only in `Txt2`, so the block is written.
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, kerning: Kerning::Optical, ..Default::default() };
    let mut t = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    t.shape = TextShape::Box { x: 0.0, y: 0.0, width: 300.0, height: 150.0 };
    t.sync_summary();
    let mut eng = photocraft_text::TextEngine::new();
    t.fit = Some(TextFit::default());
    let lines = photocraft_text::fit::solve_box(&mut eng, &t, 72.0);
    assert!(lines.len() >= 2, "{lines:?}");
    t.fit = Some(TextFit { lines, ..Default::default() });
    let baked = photocraft_text::fit::bake(&mut eng, &t, 72.0).unwrap();
    let mut unfitted = t.clone();
    unfitted.fit = None;
    assert_ne!(txt2(&doc_with(unfitted)), txt2(&doc_with(baked.clone())), "baking changes what Txt2 holds");
    assert_eq!(txt2(&doc_with(t)), txt2(&doc_with(baked)));
}
