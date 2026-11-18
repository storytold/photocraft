//! The view of a `Txt2` text object agrees with the object's text.
//!
//! Beside its text and runs (the model, `/0`), a text object holds a view (`/1`): the frame it is
//! drawn in, view runs (`/1 /0`, one per paragraph, each counting the paragraph's UTF-16 units)
//! and the laid-out lines (`/2`), whose segments count the characters they hold. In every file
//! Photoshop saves, the view runs and the segments add up to the text's length. For type it has
//! not laid out, it writes the runs marked `/0 << /1 0 >>` and a strike root without lines
//! (`corpus/photoshop/text/path-wave-open.psd`).
//!
//! A save after a text edit used to keep the view of the old text, whose counts no longer added
//! up to the new text: a state none of Photoshop's own files is in (59 text objects in
//! `corpus/photoshop/text`). Whether that is what makes Photoshop refuse such files (#10990) has
//! not been confirmed in Photoshop.

use photocraft_doc::text::{CharStyle, Kerning, TextRun};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};
use photocraft_text::engine_data::Value as E;

fn utf16(s: &str) -> String {
    s.chars().flat_map(|c| ['\0', c]).collect()
}

/// A Photoshop-shaped `Txt2` block: one laid-out point-text object per `(text, view text)` pair.
/// The model holds `text`; the view counts `view text` (the same text in a file Photoshop wrote,
/// another one in a file an earlier release saved after an edit). Each view names its own frame,
/// has one run per paragraph and one line with a segment and a glyph strike per paragraph.
fn photoshop_txt2(objects: &[(&str, &str)]) -> Vec<u8> {
    let bom = "\u{fe}\u{ff}";
    let mut frames = String::new();
    let mut body = String::new();
    for (i, (text, view_text)) in objects.iter().enumerate() {
        let model_text = format!("{text}\r");
        let paragraphs = |t: &str| format!("{t}\r").split_inclusive('\r').map(|p| p.encode_utf16().count()).collect::<Vec<_>>();
        let para_runs: String = paragraphs(text).iter().map(|n| format!("<< /0 << /0 << /0 ({bom}) /5 << /7 1.2 >> >> >> /1 {n} >> ")).collect();
        let len = model_text.encode_utf16().count();
        let view_runs: String = paragraphs(view_text).iter().map(|n| format!("<< /0 << /1 1 >> /1 {n} >> ")).collect();
        let lines: String = paragraphs(view_text)
            .iter()
            .map(|n| {
                format!(
                    "<< /99 /L /1 [ 0.0 -9.0 40.0 3.0 ] /6 [ << /99 /S /15 << /0 {n} /2 0 /5 false /6 0 >> /6 [ << /99 /G /6 << /0 [ << /3 0.5 >> ] /1 [ {n} ] >> /21 << /0 [ 8 ] /1 [ 0.0 7.0 ] >> >> ] >> ] >> "
                )
            })
            .collect();
        frames.push_str("<< /0 << /2 << /2 [ 1.0 0.0 0.0 1.0 5.0 6.0 ] >> >> >> ");
        body.push_str(&format!(
            "<< /0 << /0 ({bom}{}) /5 << /0 [ {para_runs}] >> /6 << /0 [ << /0 << /0 << /0 ({bom}) /6 << /0 0 /1 12.0 /11 1 /53 << /99 /SimplePaint /0 << /0 1 /1 [ 1.0 0.0 0.0 0.0 ] >> >> >> >> >> /1 {len} >> ] >> /10 << /0 1 /2 true >> >> /1 << /4 << /0 7 /1 11 /2 0 >> /5 0 /0 [ << /0 {i} >> ] /1 << /0 [ {view_runs}] >> /2 [ << /99 /PC /5 0 /6 [ << /99 /F /10 {i} /5 2 /6 [ << /99 /R /6 [ << /99 /R /5 2 /6 [ {lines}] >> ] >> ] >> ] >> ] >> >> ",
            utf16(&model_text)
        ));
    }
    format!(
        "\n\n/98 << /0 14 >> /0 << /1 << /0 [ << /0 << /99 /CoolTypeFont /0 << /0 ({bom}{}) /2 1 >> >> >> ] >> /8 << /0 [ {frames}] >> >> /1 << /0 << /0 1 >> /1 [ {body}] /2 << /0 7 >> >>",
        utf16("MyriadPro-Regular")
    )
    .chars()
    .map(|c| c as u32 as u8)
    .collect()
}

/// A type layer holding `text`, styled as [`photoshop_txt2`] styles its objects, whose `TySh`
/// names `Txt2` object `index` (`None`: a layer the Type tool just made).
fn type_layer(doc: &Document, text: &str, index: Option<i32>, txt2: &[u8]) -> Layer {
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, postscript_name: Some("MyriadPro-Regular".into()), ..Default::default() };
    let mut t =
        TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style: CharStyle { kerning: Kerning::Metrics, ..style } }], ..Default::default() };
    t.sync_summary();
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    let mut raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None).unwrap();
    if let Some(index) = index {
        raw = photocraft_text::psd::set_text_index(&raw, index).unwrap();
        photocraft_text::psd::apply_txt2(&mut t, &raw, &photocraft_text::psd::parse_txt2(txt2).unwrap());
    }
    t.psd_raw = Some(std::sync::Arc::new(raw));
    Layer::new(text, LayerContent::Text(t))
}

/// The `Txt2` of `doc` as PSD export writes it.
fn saved_txt2(doc: &Document) -> E {
    let bytes = photocraft_io::export(doc, "t.psd", &Default::default()).unwrap().bytes;
    let file = craft_psd::PsdFile::from_bytes(&bytes).unwrap();
    photocraft_text::psd::parse_txt2(&file.global_blocks.iter().find(|b| &b.key == b"Txt2").expect("Txt2 block").data).unwrap()
}

fn document(txt2: &[u8], layers: &[(&str, Option<i32>)]) -> Document {
    let mut doc = Document::new("t", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (text, index) in layers {
        let layer = type_layer(&doc, text, *index, txt2);
        doc.layers.push(layer);
    }
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(txt2.to_vec()))];
    doc
}

fn objects(txt2: &E) -> &[E] {
    txt2.path(&["1", "1"]).and_then(E::as_array).unwrap()
}

/// The lengths of a run array (`<< /0 [ << … /1 length >> … ] >>`).
fn run_lengths(runs: Option<&E>) -> Vec<i64> {
    runs.and_then(|r| r.get("0")).and_then(E::as_array).map(|a| a.iter().map(|r| r.get("1").and_then(E::as_i64).unwrap()).collect()).unwrap_or_default()
}

/// The characters the laid-out segments (`/99 /S`, count at `/15 /0`) under `node` hold, and how
/// many segments there are.
fn segment_characters(node: &E, total: &mut (i64, usize)) {
    match node {
        E::Dict(items) => {
            if node.get("99") == Some(&E::Name("S".into())) {
                total.0 += node.path(&["15", "0"]).and_then(E::as_i64).unwrap_or(0);
                total.1 += 1;
            }
            items.iter().for_each(|(_, v)| segment_characters(v, total));
        }
        E::Array(items) => items.iter().for_each(|v| segment_characters(v, total)),
        _ => {}
    }
}

/// What Photoshop's own files hold for every text object: the model's paragraph and style runs,
/// the view runs and the laid-out segments each add up to the text's UTF-16 length, with one
/// paragraph run and one view run per paragraph.
fn assert_views_cover_their_text(txt2: &E, what: &str) {
    for (i, object) in objects(txt2).iter().enumerate() {
        let Some(text) = object.path(&["0", "0"]).and_then(E::as_str) else { continue };
        let len = text.encode_utf16().count() as i64;
        let paragraphs = text.split_inclusive('\r').count();
        let paragraph_runs = run_lengths(object.path(&["0", "5"]));
        assert_eq!(
            (paragraph_runs.iter().sum::<i64>(), paragraph_runs.len()),
            (len, paragraphs),
            "{what}: object {i} {text:?}: paragraph runs {paragraph_runs:?}"
        );
        assert_eq!(run_lengths(object.path(&["0", "6"])).iter().sum::<i64>(), len, "{what}: object {i} {text:?}: style runs");
        let Some(view) = object.get("1") else { continue };
        if view.get("1").is_some() {
            let view_runs = run_lengths(view.get("1"));
            assert_eq!((view_runs.iter().sum::<i64>(), view_runs.len()), (len, paragraphs), "{what}: object {i} {text:?}: view runs {view_runs:?}");
        }
        let mut laid_out = (0, 0);
        if let Some(strikes) = view.get("2") {
            segment_characters(strikes, &mut laid_out);
        }
        assert!(laid_out.1 == 0 || laid_out.0 == len, "{what}: object {i} {text:?}: its laid-out lines hold {} characters", laid_out.0);
    }
}

/// The view of an object written as not laid out: its frame kept, one run per paragraph marked
/// `/0 << /1 0 >>`, and a strike root without lines.
fn assert_not_laid_out(object: &E, frame: i64, paragraphs: &[i64], what: &str) {
    let view = object.get("1").unwrap();
    assert_eq!(
        view.path(&["0"]).and_then(E::as_array).and_then(|f| f.first()).and_then(|f| f.get("0")),
        Some(&E::Int(frame)),
        "{what}: the frame its view names"
    );
    assert_eq!(run_lengths(view.get("1")), paragraphs, "{what}: view runs");
    for run in view.path(&["1", "0"]).and_then(E::as_array).unwrap() {
        assert_eq!(run.path(&["0", "1"]), Some(&E::Int(0)), "{what}: a view run still marked laid out");
    }
    let strikes = view.get("2").and_then(E::as_array).unwrap();
    assert_eq!(strikes.len(), 1, "{what}: strike root");
    assert_eq!(strikes[0].get("99"), Some(&E::Name("PC".into())), "{what}: strike root");
    assert!(strikes[0].get("6").is_none(), "{what}: lines of the old text were kept");
    assert_eq!(view.path(&["4", "1"]), Some(&E::Int(11)), "{what}: the view's other keys stay");
}

/// The fixture is shaped as the check expects of a Photoshop file.
#[test]
fn fixture_views_cover_their_text() {
    let source = photocraft_text::psd::parse_txt2(&photoshop_txt2(&[("Old", "Old"), ("One\rTwo", "One\rTwo")])).unwrap();
    assert_views_cover_their_text(&source, "fixture");
    assert_eq!(run_lengths(objects(&source)[1].path(&["1", "1"])), [4, 4]);
}

/// Editing a type layer's text to a longer one writes its object with a view of the new text, not
/// laid out; the layer beside it keeps its object, layout included.
#[test]
fn edited_text_gets_a_view_of_the_new_text() {
    let txt2 = photoshop_txt2(&[("Old", "Old"), ("Kept", "Kept")]);
    let source = photocraft_text::psd::parse_txt2(&txt2).unwrap();
    let saved = saved_txt2(&document(&txt2, &[("Newer!!", Some(0)), ("Kept", Some(1))]));
    assert_eq!(objects(&saved)[0].path(&["0", "0"]).and_then(E::as_str), Some("Newer!!\r"));
    assert_views_cover_their_text(&saved, "edited");
    assert_not_laid_out(&objects(&saved)[0], 0, &[8], "edited");
    assert_eq!(objects(&saved)[1], objects(&source)[1], "the unedited layer's object is saved unchanged");
}

/// An edit that changes the number of paragraphs: one view run per paragraph of the new text.
#[test]
fn edited_paragraphs_get_a_view_run_each() {
    let txt2 = photoshop_txt2(&[("One paragraph", "One paragraph")]);
    let saved = saved_txt2(&document(&txt2, &[("First paragraph here\nto\nThird paragraph, last", Some(0))]));
    assert_views_cover_their_text(&saved, "three paragraphs");
    assert_not_laid_out(&objects(&saved)[0], 0, &[21, 3, 22], "three paragraphs");
    let txt2 = photoshop_txt2(&[("One\rTwo\rThree", "One\rTwo\rThree")]);
    let saved = saved_txt2(&document(&txt2, &[("One", Some(0))]));
    assert_views_cover_their_text(&saved, "one paragraph");
    assert_not_laid_out(&objects(&saved)[0], 0, &[4], "one paragraph");
}

/// An edit that keeps the text's length still leaves the old text's lines out: their glyphs are
/// the old text's.
#[test]
fn an_edit_of_the_same_length_drops_the_old_lines() {
    let txt2 = photoshop_txt2(&[("Old", "Old")]);
    let saved = saved_txt2(&document(&txt2, &[("New", Some(0))]));
    assert_views_cover_their_text(&saved, "same length");
    assert_not_laid_out(&objects(&saved)[0], 0, &[4], "same length");
}

/// A file an earlier release saved after an edit holds the new text beside the old text's view.
/// Saving it again, with nothing changed, repairs the object.
#[test]
fn a_view_of_another_text_is_repaired_on_resave() {
    let txt2 = photoshop_txt2(&[("Newer!!", "Old"), ("Kept", "Kept")]);
    let source = photocraft_text::psd::parse_txt2(&txt2).unwrap();
    let saved = saved_txt2(&document(&txt2, &[("Newer!!", Some(0)), ("Kept", Some(1))]));
    assert_views_cover_their_text(&saved, "resave");
    assert_not_laid_out(&objects(&saved)[0], 0, &[8], "resave");
    assert_eq!(objects(&saved)[1], objects(&source)[1], "the sound object is saved unchanged");
}

/// A type layer added to a Photoshop file takes the shape of one of the file's objects, but not
/// that object's laid-out lines.
#[test]
fn a_new_type_layer_takes_no_lines_from_its_template() {
    let txt2 = photoshop_txt2(&[("Template text", "Template text")]);
    let saved = saved_txt2(&document(&txt2, &[("Template text", Some(0)), ("Hi\nthere", None)]));
    assert_eq!(objects(&saved).len(), 2);
    assert_views_cover_their_text(&saved, "new layer");
    assert_eq!(objects(&saved)[1].path(&["0", "0"]).and_then(E::as_str), Some("Hi\rthere\r"));
    assert_not_laid_out(&objects(&saved)[1], 1, &[3, 6], "new layer");
}

/// A save with no type change returns the file's block byte for byte (#2374), laid-out lines and
/// all.
#[test]
fn an_unedited_save_keeps_the_block() {
    let txt2 = photoshop_txt2(&[("Old", "Old"), ("One\rTwo", "One\rTwo")]);
    let doc = document(&txt2, &[("Old", Some(0)), ("One\nTwo", Some(1))]);
    let bytes = photocraft_io::export(&doc, "t.psd", &Default::default()).unwrap().bytes;
    let file = craft_psd::PsdFile::from_bytes(&bytes).unwrap();
    assert!(file.global_blocks.iter().find(|b| &b.key == b"Txt2").unwrap().data == txt2, "Txt2 changed on an unedited save");
}
