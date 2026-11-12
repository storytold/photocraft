//! Dynamic Text saves to PSD as the plain type it bakes to: the document's `Txt2` describes the
//! baked layer, as its `TySh` does, not the fitted layer's base sizes.

use photocraft_doc::text::{CharStyle, Kerning, TextFit, TextRun, TextShape};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};

fn txt2(doc: &Document) -> Vec<u8> {
    let bytes = photocraft_io::export(doc, "d.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    file.global_blocks.iter().find(|b| &b.key == b"Txt2").map(|b| b.data.clone()).expect("Txt2 written for the Optical layer")
}

fn doc_with(t: TextLayer) -> Document {
    let mut doc = Document::new("d", photocraft_geom::Size::new(320, 160), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    // A second layer with Optical kerning, which lives only in `Txt2`, so the block is written.
    let optical = CharStyle { font_family: "Inter".into(), size_pt: 12.0, kerning: Kerning::Optical, ..Default::default() };
    let mut o = TextLayer { text: "Wave".into(), runs: vec![TextRun { len: 4, style: optical }], ..Default::default() };
    o.sync_summary();
    for (name, mut l) in [("fitted", t), ("optical", o)] {
        photocraft_text::TextEngine::new().render_layer(&mut l, doc.resolution_dpi, doc.pixel_format());
        doc.layers.push(Layer::new(name, LayerContent::Text(l)));
    }
    doc
}

#[test]
fn txt2_describes_the_baked_text() {
    let text = "Hello dynamic world of text";
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, kerning: Kerning::Metrics, ..Default::default() };
    let mut t = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    t.shape = TextShape::Box { x: 0.0, y: 0.0, width: 300.0, height: 150.0 };
    t.sync_summary();
    let mut eng = photocraft_text::TextEngine::new();
    t.fit = Some(TextFit::default());
    photocraft_text::fit::refit(&mut eng, &mut t, 72.0);
    assert!(photocraft_text::fit::is_fitted_lines(&t), "{:?}", t.fit);
    let baked = photocraft_text::fit::bake(&mut eng, &t, 72.0).unwrap_or_else(|e| panic!("{e:?}"));
    let mut unfitted = t.clone();
    unfitted.fit = None;
    assert_ne!(txt2(&doc_with(unfitted)), txt2(&doc_with(baked.clone())), "baking changes what Txt2 holds");
    assert_eq!(txt2(&doc_with(t)), txt2(&doc_with(baked)));
}

#[test]
fn unsolved_dynamic_text_is_solved_for_the_file() {
    let text = "Hello dynamic world of text";
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, kerning: Kerning::Metrics, ..Default::default() };
    let mut t = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    t.shape = TextShape::Box { x: 0.0, y: 0.0, width: 300.0, height: 150.0 };
    t.sync_summary();
    // Dynamic Text on, its lines never solved (as a file can hold it).
    t.fit = Some(TextFit::default());
    assert!(!photocraft_text::fit::is_fitted_lines(&t));
    let mut eng = photocraft_text::TextEngine::new();
    let mut solved = t.clone();
    photocraft_text::fit::refit(&mut eng, &mut solved, 72.0);
    let baked = photocraft_text::fit::bake(&mut eng, &solved, 72.0).unwrap_or_else(|e| panic!("{e:?}"));
    let doc = doc_with(t);
    assert_eq!(txt2(&doc), txt2(&doc_with(baked)), "saved as the solved, baked look");
    let LayerContent::Text(kept) = &doc.layers[0].content else { panic!("text layer") };
    assert!(kept.fit.as_ref().is_some_and(|f| f.lines.is_empty()), "the document is left alone");
}
