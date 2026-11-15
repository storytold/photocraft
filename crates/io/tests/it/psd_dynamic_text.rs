//! Dynamic Text saves to PSD as the plain type it bakes to: the document's `Txt2` describes the
//! baked layer, as its `TySh` does, not the fitted layer's base sizes.

use photocraft_doc::text::{CharStyle, Kerning, TextFit, TextRun, TextShape};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};

fn txt2(doc: &Document) -> Vec<u8> {
    let bytes = photocraft_io::export(doc, "d.psd", &Default::default()).unwrap().bytes;
    let file = craft_psd::PsdFile::from_bytes(&bytes).unwrap();
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

/// The pixels of a layer whose lines are solved for the file show its solved look, as its baked
/// text does: the layer's channels and the merged image are those of the same layer solved
/// before the save. They were drawn from the layer's cache, laid out at its base sizes.
#[test]
fn unsolved_dynamic_text_is_saved_with_its_solved_pixels() {
    let text = "Hello dynamic world of text";
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, kerning: Kerning::Metrics, ..Default::default() };
    let mut t = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    t.shape = TextShape::Box { x: 0.0, y: 0.0, width: 300.0, height: 150.0 };
    t.sync_summary();
    t.fit = Some(TextFit::default());
    let mut solved = t.clone();
    photocraft_text::fit::refit(&mut photocraft_text::TextEngine::new(), &mut solved, 72.0);
    assert!(photocraft_text::fit::is_fitted_lines(&solved));
    let file = |t: &TextLayer| {
        let bytes = photocraft_io::export(&doc_with(t.clone()), "d.psd", &Default::default()).unwrap().bytes;
        craft_psd::PsdFile::from_bytes(&bytes).unwrap()
    };
    let pixels = |f: &craft_psd::PsdFile| {
        let layer = &f.layer_info.as_ref().expect("layers").layers[0];
        (layer.rect, layer.channels.iter().map(|c| c.data.clone()).collect::<Vec<_>>())
    };
    let (unsolved, solved) = (file(&t), file(&solved));
    assert_eq!(pixels(&unsolved), pixels(&solved), "the layer's channels");
    assert_eq!(unsolved.image_data.data, solved.image_data.data, "the merged image");
    let mut plain = t.clone();
    plain.fit = None;
    assert_ne!(pixels(&file(&plain)), pixels(&solved), "the solved look differs from the base sizes");
}

/// Photoshop's own paragraph indents and spacing (`corpus/photoshop/text`): the file's type layer
/// made Dynamic Text in an area, saved, holds in its `Txt2` paragraph runs the indents and spacing
/// its `TySh` holds, paragraph for paragraph, once the bake has split the paragraphs into lines.
/// The `Txt2` runs kept the file's, which no longer matched the baked paragraphs.
#[cfg(feature = "corpus")]
#[test]
fn corpus_area_bake_writes_its_paragraph_spacing_to_txt2() {
    use photocraft_doc::vector::{Path, Subpath};
    use photocraft_text::engine_data::Value as E;
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/text/paragraph-indents-space-before-after.psd");
    assert!(file.is_file(), "{} is missing: run `cargo xtask corpus --photoshop`", file.display());
    let mut doc = photocraft_io::import("p.psd", &std::fs::read(&file).unwrap()).unwrap().document;
    let (w, h) = (f64::from(doc.size.width), f64::from(doc.size.height));
    let dpi = doc.resolution_dpi;
    let LayerContent::Text(t) = &mut doc.layers.iter_mut().find(|l| matches!(l.content, LayerContent::Text(_))).expect("a type layer").content else {
        unreachable!()
    };
    let before = t.text.matches('\n').count() + 1;
    t.shape = TextShape::Area { path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])]) };
    t.fit = Some(TextFit::default());
    photocraft_text::fit::refit(&mut photocraft_text::TextEngine::new(), t, dpi);
    assert!(photocraft_text::fit::is_fitted_lines(t), "{:?}", t.fit);
    let out = photocraft_io::export(&doc, "p.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("p.psd", &out).unwrap().document;
    let LayerContent::Text(baked) = &back.layers.iter().find(|l| matches!(l.content, LayerContent::Text(_))).unwrap().content else { unreachable!() };
    let txt2 = back.metadata.psd_global_blocks.iter().find(|b| &b.1 == b"Txt2").map(|b| b.2.clone()).expect("the file's Txt2 is written");
    let txt2 = photocraft_text::psd::parse_txt2(&txt2).unwrap();
    let runs = txt2.path(&["1", "1"]).and_then(E::as_array).unwrap()[0].path(&["0", "5", "0"]).and_then(E::as_array).unwrap();
    // The TySh's paragraph at the start of each paragraph of the baked text.
    let mut starts = vec![0];
    starts.extend(baked.text.match_indices('\n').map(|(i, _)| i + 1));
    assert!(starts.len() > before, "the bake split the paragraphs into lines: {} -> {}", before, starts.len());
    assert_eq!(runs.len(), starts.len(), "one Txt2 paragraph run per paragraph");
    let pruns = baked.paragraph_runs();
    let style_at = |at: usize| {
        let mut end = 0;
        pruns.iter().find(|r| {
            end += r.len;
            end > at
        })
    };
    let k = f64::from(dpi) / 72.0;
    for (i, (run, at)) in runs.iter().zip(&starts).enumerate() {
        let p = &style_at(*at).expect("a paragraph run").style;
        let props = run.path(&["0", "0", "5"]).expect("the paragraph's style");
        let got: Vec<f64> = ["1", "2", "3", "4", "5"].iter().map(|key| props.get(key).and_then(E::as_f64).unwrap() / k).collect();
        let want: Vec<f64> = [p.first_line_indent_pt, p.start_indent_pt, p.end_indent_pt, p.space_before_pt, p.space_after_pt].map(f64::from).to_vec();
        assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-3), "paragraph {i}: Txt2 {got:?}, TySh {want:?}");
    }
}
