//! Kerning of PSD type layers (#206): manual kerning in EngineData (`AutoKerning`/`Kerning`
//! style-run fields), the automatic mode in `Txt2`, and the layout against Photoshop's own
//! glyph positions.
//!
//! The Photoshop-authored oracles (`corpus/photoshop/text/kerning-*.psd`) run under the
//! `corpus` feature (`cargo xtask test-corpus`).

#[cfg(feature = "corpus")]
use std::path::Path;

use photocraft_doc::text::{CharStyle, Kerning, TextRun};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};
use photocraft_text::engine_data::Value as E;

/// `(AutoKerning, Kerning)` of every UTF-16 unit of the engine text of a `TySh` block.
fn kerning_fields(tysh: &[u8]) -> Vec<(bool, i64)> {
    let t = photocraft_text::psd::parse_tysh(tysh).unwrap();
    let e = photocraft_text::psd::engine_data(&t.text).unwrap();
    let runs = e.path(&["EngineDict", "StyleRun", "RunArray"]).and_then(E::as_array).unwrap();
    let lens = e.path(&["EngineDict", "StyleRun", "RunLengthArray"]).and_then(E::as_array).unwrap();
    let mut out = Vec::new();
    for (r, n) in runs.iter().zip(lens) {
        let d = r.path(&["StyleSheet", "StyleSheetData"]).unwrap();
        let f = (d.get("AutoKerning").and_then(E::as_bool).unwrap_or(true), d.get("Kerning").and_then(E::as_i64).unwrap_or(0));
        out.extend(std::iter::repeat_n(f, n.as_i64().unwrap() as usize));
    }
    out
}

/// Per character: (mode, manual kern).
fn chars(t: &TextLayer) -> Vec<(Kerning, f32)> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in t.char_runs() {
        let n = t.text.get(at..at + r.len).map_or(0, |s| s.chars().count());
        out.extend(std::iter::repeat_n((r.style.kerning, r.style.kern), n));
        at += r.len;
    }
    out
}

fn layer(text: &str, runs: &[(usize, Kerning, f32)]) -> TextLayer {
    let base = CharStyle { font_family: "Inter".into(), size_pt: 24.0, ..Default::default() };
    let mut t = TextLayer {
        text: text.into(),
        runs: runs.iter().map(|&(len, kerning, kern)| TextRun { len, style: CharStyle { kerning, kern, ..base.clone() } }).collect(),
        ..Default::default()
    };
    t.sync_summary();
    t
}

/// The model survives a PSD round trip, and the EngineData fields follow Photoshop's form: a
/// manual kern after character c sits on c + 1; "no automatic kerning" on c is
/// `AutoKerning false` on c + 2 (how Photoshop reads it).
#[test]
fn manual_and_off_kerning_round_trip_through_psd() {
    use Kerning::{Metrics as M, Off as O};
    // "AVATAR Wave": +100 after A, -50 after T, characters 6..8 without automatic kerning.
    let t = layer("AVATAR Wave", &[(1, O, 100.0), (2, M, 0.0), (1, O, -50.0), (2, M, 0.0), (3, O, 0.0), (2, M, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t2 = t.clone();
    photocraft_text::TextEngine::new().render_layer(&mut t2, doc.resolution_dpi, doc.pixel_format());
    doc.layers.push(Layer::new("kerned", LayerContent::Text(t2)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(b) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(chars(b), chars(&t));

    let tysh = photocraft_text::psd::build_tysh(&t, 72.0, None);
    let f = kerning_fields(&tysh);
    // 11 characters + the trailing paragraph break.
    assert_eq!(f.len(), 12);
    assert_eq!(f[1], (false, 100), "kern after A on V, Photoshop's form");
    assert_eq!(f[4], (true, -50), "kern after T on A; character 2 isn't manual");
    for j in [8, 9, 10] {
        assert!(!f[j].0, "character {} without automatic kerning", j - 2);
    }
    assert!(f.iter().enumerate().filter(|(j, _)| ![1, 4, 8, 9, 10].contains(j)).all(|(_, x)| *x == (true, 0)), "{f:?}");
}

/// #3514: a fresh document of ordinary Metrics type writes no document `Txt2` block — Photoshop
/// never writes its empty `<< /0 << >> >>` stub, and readers such as Photopea reject it — while an
/// Optical layer still does, because its mode lives only in `Txt2`.
#[test]
fn fresh_metrics_text_writes_no_txt2_block() {
    use photocraft_doc::text::Kerning;
    let saves_txt2 = |kerning: Kerning| {
        let t = layer("AV", &[(2, kerning, 0.0)]);
        let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut r = t.clone();
        photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
        doc.layers.push(Layer::new("t", LayerContent::Text(r)));
        let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
        photocraft_psd::PsdFile::from_bytes(&bytes).unwrap().global_blocks.iter().any(|b| &b.key == b"Txt2")
    };
    assert!(!saves_txt2(Kerning::Metrics), "a plain Metrics layer must not write the empty Txt2 stub");
    assert!(saves_txt2(Kerning::Optical), "Optical lives only in Txt2");
}

/// #1348: the automatic mode (Metrics/Optical) lives in the document `Txt2` block, one model
/// object per `TextIndex`. A PhotoCraft-authored PSD must carry it or a reopened layer falls
/// back to Metrics — Optical used to be silently lost.
#[test]
fn optical_kerning_round_trips_through_psd() {
    use Kerning::{Metrics as M, Optical as P};
    let t1 = layer("AVATAR", &[(4, P, 0.0), (2, M, 0.0)]);
    let t2 = layer("Wave", &[(2, M, 0.0), (2, P, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (name, want) in [("one", &t1), ("two", &t2)] {
        let mut t = want.clone();
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        doc.layers.push(Layer::new(name, LayerContent::Text(t)));
    }
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    for (want, l) in [(&t1, &back.layers[0]), (&t2, &back.layers[1])] {
        let LayerContent::Text(b) = &l.content else { panic!("not text") };
        assert_eq!(chars(b), chars(want), "{}", l.name);
    }
}

/// #10308: stylistic sets (and other Txt2-only OpenType flags) on a PhotoCraft-native document
/// write a `Txt2` block and round-trip through save and reopen.
#[test]
fn fresh_stylistic_sets_round_trip_through_psd() {
    use photocraft_doc::text::FontFeature;
    let base = CharStyle { font_family: "Inter".into(), size_pt: 24.0, ..Default::default() };
    let mut t = TextLayer {
        text: "Hi".into(),
        runs: vec![TextRun {
            len: 2,
            style: CharStyle { features: vec![FontFeature { tag: "ss01".into(), value: 1 }, FontFeature { tag: "ss05".into(), value: 1 }], ..base },
        }],
        ..Default::default()
    };
    t.sync_summary();
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut r = t.clone();
    photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
    doc.layers.push(Layer::new("t", LayerContent::Text(r)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 =
        file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).expect("Txt2 written for stylistic sets");
    let style = txt2
        .path(&["1", "1"])
        .and_then(E::as_array)
        .and_then(|a| a.first())
        .and_then(|o| o.path(&["0", "6", "0"]))
        .and_then(E::as_array)
        .and_then(|a| a.first())
        .and_then(|r| r.path(&["0", "0", "6"]))
        .expect("style run");
    // Bits 0 and 4 → sets 1 and 5.
    assert_eq!(style.get("85").and_then(E::as_i64), Some((1 << 0) | (1 << 4)));
    assert_eq!(style.path(&["53", "99"]).and_then(E::as_str), Some("SimplePaint"), "paint stream tag");
    assert_eq!(style.path(&["53", "0", "0"]).and_then(E::as_i64), Some(1), "fill colour type RGB");
    assert!(style.path(&["53", "0", "1"]).and_then(E::as_array).is_some_and(|v| v.len() == 4), "fill colour values");
    assert_eq!(style.get("0").and_then(E::as_i64), Some(0), "font index 0");
    assert_eq!(
        txt2.path(&["0", "1", "0"]).and_then(E::as_array).and_then(|a| a.first()).and_then(|f| f.path(&["0", "0", "0"])).and_then(E::as_str),
        Some("Inter-Regular"),
        "fresh Txt2 seeds a font entry"
    );
    assert!(txt2.path(&["0", "8", "0"]).and_then(E::as_array).is_some_and(|f| !f.is_empty()), "fresh Txt2 names a frame");
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(b) = &back.layers[0].content else { panic!("not text") };
    let runs = b.char_runs();
    assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), b.text.len(), "the runs cover the text");
    for r in &runs {
        let on = |tag: &str| r.style.features.iter().any(|f| f.tag == tag && f.value == 1);
        assert!(on("ss01") && on("ss05"), "both sets on every character: {:?}", r.style.features);
    }
}

/// Two imported layers can name the same Txt2 object; export must update the second
/// preserved TySh pointer so each layer restores its own automatic kerning mode.
#[test]
fn duplicate_preserved_text_indices_get_distinct_txt2_objects() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let cases = [("optical", "AV", Kerning::Optical), ("metrics", "BC", Kerning::Metrics)];
    for (name, text, mode) in cases {
        let mut t = layer(text, &[(text.len(), mode, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
        doc.layers.push(Layer::new(name, LayerContent::Text(t)));
    }
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
    assert_eq!(txt2.path(&["1", "1"]).and_then(E::as_array).unwrap().len(), 2);
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    for (i, expected) in [Kerning::Optical, Kerning::Metrics].into_iter().enumerate() {
        let LayerContent::Text(t) = &back.layers[i].content else { panic!("expected text layer") };
        assert!(chars(t).iter().all(|(mode, _)| *mode == expected), "layer {i} used another layer's Txt2 object");
    }
}

/// A type layer added below preserved ones takes a number past theirs: claiming the next free
/// number on the way up renumbered every layer above it onto its neighbour's `Txt2` object
/// (#7773, 70 of 74 layers on the #7698 file).
#[test]
fn new_text_layer_below_keeps_the_preserved_text_indices() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (i, text) in ["AV", "BC"].into_iter().enumerate() {
        let mut t = layer(text, &[(2, Kerning::Optical, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, i as i32).unwrap()));
        doc.layers.push(Layer::new(text, LayerContent::Text(t)));
    }
    let mut t = layer("new", &[(3, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    doc.layers.insert(0, Layer::new("new", LayerContent::Text(t)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let indices: Vec<_> = back
        .layers
        .iter()
        .map(|l| {
            let LayerContent::Text(t) = &l.content else { panic!("not text") };
            (l.name.as_str(), t.psd_raw.as_deref().and_then(|d| photocraft_text::psd::text_index(d)))
        })
        .collect();
    assert_eq!(indices, [("new", Some(2)), ("AV", Some(0)), ("BC", Some(1))]);
}

/// A Photoshop-shaped `Txt2` block: one object per `(text, style keys)` pair, each with its view
/// naming its own frame.
fn photoshop_txt2(objects: &[(&str, &str)]) -> Vec<u8> {
    let bom = "\u{fe}\u{ff}";
    let frame = "<< /0 << /2 << /2 [ 1.0 0.0 0.0 1.0 5.0 6.0 ] >> >> >> ";
    let mut frames = String::new();
    let mut body = String::new();
    for (i, (text, style)) in objects.iter().enumerate() {
        let utf16: String = format!("{text}\r").chars().flat_map(|c| ['\0', c]).collect();
        let len = text.len() + 1;
        frames.push_str(frame);
        body.push_str(&format!(
            "<< /0 << /0 ({bom}{utf16}) /5 << /0 [ << /0 << /0 << /0 ({bom}) /5 << /7 1.2 >> >> >> /1 {len} >> ] >> /6 << /0 [ << /0 << /0 << /0 ({bom}) /6 << /0 0 /1 12.0 /11 1 /53 << /99 /SimplePaint /0 << /0 1 /1 [ 1.0 0.0 0.0 0.0 ] >> >> {style} >> >> >> /1 {len} >> ] >> >> /1 << /0 [ << /0 {i} >> ] /1 << /0 [ << /0 << /1 1 >> /1 {len} >> ] >> /2 [ << /99 /PC /6 [ << /99 /F /10 {i} >> ] >> ] >> >> "
        ));
    }
    let font: String = "MyriadPro-Regular".chars().flat_map(|c| ['\0', c]).collect();
    format!("\n\n/98 << /0 7 >> /0 << /1 << /0 [ << /0 << /99 /CoolTypeFont /0 << /0 ({bom}{font}) /2 1 >> >> >> ] >> /8 << /0 [ {frames}] >> >> /1 << /0 << /0 1 >> /1 [ {body}] /2 << /0 7 >> >>").chars().map(|c| c as u32 as u8).collect()
}

/// An edited object takes the layer's style, not the old text's: "Hello world" with Stylistic
/// Alternates on "Hello " edited to "world" without them comes back without `salt`.
#[test]
fn edited_text_layer_takes_its_own_style_runs() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t = layer("world", &[(5, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
    t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
    doc.layers.push(Layer::new("edited", LayerContent::Text(t)));
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(photoshop_txt2(&[("Hello world", "/28 true")])))];
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(t) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(t.text, "world");
    assert!(t.char_runs().iter().all(|r| r.style.features.iter().all(|f| f.tag == "calt")), "the old text's salt came back");
}

/// Stylistic Sets (#10285) on a type layer of an opened file save into its `Txt2` object as the
/// set mask and come back on reopen: Set 1 and Set 5 on "Il", none on "36".
#[test]
fn edited_text_layer_keeps_its_stylistic_sets() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t = layer("Il36", &[(2, Kerning::Metrics, 0.0), (2, Kerning::Metrics, 0.0)]);
    if let Some(run) = t.runs.first_mut() {
        run.style.features = ["ss01", "ss05"].iter().map(|tag| photocraft_doc::text::FontFeature { tag: (*tag).into(), value: 1 }).collect();
    }
    t.sync_summary();
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
    t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
    doc.layers.push(Layer::new("sets", LayerContent::Text(t)));
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(photoshop_txt2(&[("Il36", "")])))];
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(t) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(t.text, "Il36");
    let on = |r: &TextRun, tag: &str| r.style.features.iter().any(|f| f.tag == tag && f.value > 0);
    let runs = t.char_runs();
    let first = runs.first().unwrap();
    assert!(on(first, "ss01") && on(first, "ss05") && !on(first, "ss02"), "{runs:?}");
    assert_eq!(first.len, 2, "{runs:?}");
    assert!(runs.iter().skip(1).all(|r| !on(r, "ss01") && !on(r, "ss05")), "{runs:?}");
}

/// A file-controlled `TextIndex` must not size the save: `i32::MAX` is ignored (the regenerated
/// `Txt2` is sized by real text objects, never by a file's numbers) and the document still saves
/// and reopens with its text.
#[test]
fn hostile_text_index_never_sizes_the_save() {
    let t = layer("AV", &[(1, Kerning::Metrics, 0.0), (1, Kerning::Optical, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut r = t.clone();
    photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
    // An invalid index is remapped into the saved Txt2 rather than preserved.
    r.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&photocraft_text::psd::build_tysh(&r, 72.0, None), i32::MAX).unwrap()));
    doc.layers.push(Layer::new("hostile", LayerContent::Text(r)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
    let slots = txt2.path(&["1", "1"]).and_then(E::as_array).unwrap();
    assert!(slots.len() <= 8, "a file-controlled TextIndex sized the save: {} slots", slots.len());
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(b) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(b.text, "AV");
    assert_eq!(chars(b), chars(&t), "a remapped TextIndex must still restore Optical kerning");
}

/// A save must not drop what the file held beyond the style runs: an unedited resave writes the
/// file's `Txt2` back as it was, with Photoshop's glyph pen positions (`/21 /1`) in it.
#[test]
fn txt2_extras_survive_an_unedited_resave() {
    let obj = E::Dict(vec![
        ("0".into(), E::Dict(vec![("0".into(), E::String("AV\r".into())), ("6".into(), E::Dict(vec![("0".into(), E::Array(vec![]))]))])),
        ("21".into(), E::Dict(vec![("1".into(), E::Array(vec![E::Real(10.0), E::Real(20.0)]))])),
    ]);
    let photoshop = photocraft_text::engine_data::write_bare(&[
        ("98".into(), E::Dict(vec![("0".into(), E::Int(14))])),
        ("0".into(), E::dict()),
        ("1".into(), E::Dict(vec![("1".into(), E::Array(vec![obj]))])),
    ]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut r = layer("AV", &[(2, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
    // The file's layer: its `TySh` names object 0.
    let raw = photocraft_text::psd::build_tysh(&r, doc.resolution_dpi, None);
    r.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
    // The file's own block for this text: its style runs, and the pen positions only it holds.
    let txt2 = photocraft_text::psd::build_txt2(&[(0, &r)], Some(&photoshop));
    doc.layers.push(Layer::new("kept", LayerContent::Text(r)));
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"Txt2", std::sync::Arc::new(txt2.clone())));
    let saved = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap();
    let file = photocraft_psd::PsdFile::from_bytes(&saved.bytes).unwrap();
    let back = &file.global_blocks.iter().find(|b| &b.key == b"Txt2").expect("an unedited resave keeps the file's Txt2").data;
    assert_eq!(back[..], txt2[..], "written back as it was");
    let object = photocraft_text::psd::parse_txt2(back).unwrap().path(&["1", "1"]).and_then(E::as_array).unwrap()[0].clone();
    assert_eq!(object.path(&["21", "1"]).and_then(E::as_array).map(|items| items.len()), Some(2), "pen positions survive");
    assert!(!saved.warnings.iter().any(|w| w.contains("Txt2")), "nothing is lost, so nothing is said: {:?}", saved.warnings);
}

/// `tysh` with the superscript, subscript and small-caps sizes taken out of its EngineData (as an
/// older PhotoCraft wrote a type layer it made), and then `also` applied to the EngineData.
fn without_document_sizes(tysh: &[u8], also: impl FnOnce(&mut E)) -> Vec<u8> {
    use photocraft_psd::descriptor::Value as D;
    let mut t = photocraft_text::psd::parse_tysh(tysh).unwrap();
    let mut e = photocraft_text::psd::engine_data(&t.text).unwrap();
    for key in ["ResourceDict", "DocumentResources"] {
        if let Some(E::Dict(items)) = e.get_mut(key) {
            items.retain(|(k, _)| !["SuperscriptSize", "SuperscriptPosition", "SubscriptSize", "SubscriptPosition", "SmallCapSize"].contains(&k.as_str()));
        }
    }
    also(&mut e);
    let item = t.text.items.iter_mut().find(|(k, _)| k.is("EngineData")).unwrap();
    item.1 = D::RawData(photocraft_text::engine_data::write(&e));
    let out = photocraft_text::psd::write_tysh(&t);
    assert!(!out.windows(15).any(|w| w == b"SuperscriptSize"));
    out
}

/// An unedited save writes the file's `Txt2` back byte for byte; once a type layer is edited or
/// added the block is dropped and every type layer gets generated `TySh` data, as in a new
/// document. Photoshop refused a file that mixed its own `Txt2` with `TySh` written here
/// ("program error", REQUIRE 903 once per type layer) and opened it without the `Txt2`.
#[test]
fn edited_type_drops_the_files_txt2() {
    let make = |text: &str| {
        let mut t = layer(text, &[(text.len(), Kerning::Metrics, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, 72.0, photocraft_color::PixelFormat::RGBA8);
        let raw = photocraft_text::psd::build_tysh(&t, 72.0, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
        t
    };
    let kept = make("AV");
    // The file's own block, as an unedited save writes it.
    let prev = photocraft_text::psd::build_txt2(&[(0, &kept)], Some(&photoshop_txt2(&[("AV", "")])));
    let save = |layers: Vec<photocraft_doc::TextLayer>| {
        let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        for (i, t) in layers.into_iter().enumerate() {
            doc.layers.push(Layer::new(format!("t{i}"), LayerContent::Text(t)));
        }
        doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(prev.clone()))];
        let saved = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap();
        let file = photocraft_psd::PsdFile::from_bytes(&saved.bytes).unwrap();
        let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").map(|b| b.data.clone());
        // Dropping the file's Txt2 is said, untouched layers included; keeping it says nothing.
        let warned = saved.warnings.iter().any(|w| w.contains("Txt2") && w.contains("unedited ones included"));
        assert_eq!(warned, txt2.is_none(), "{:?}", saved.warnings);
        let back = photocraft_io::import("k.psd", &saved.bytes).unwrap().document;
        let texts: Vec<String> = back.layers.iter().filter_map(|l| if let LayerContent::Text(t) = &l.content { Some(t.text.clone()) } else { None }).collect();
        (txt2, texts)
    };
    let (txt2, texts) = save(vec![kept.clone()]);
    assert_eq!(txt2.as_deref(), Some(&prev[..]), "an unedited save keeps the file's Txt2 byte for byte");
    assert_eq!(texts, ["AV"]);

    let mut edited = kept.clone();
    edited.text = "AX".into();
    let (txt2, texts) = save(vec![edited]);
    assert!(txt2.is_none(), "an edited type layer drops the file's Txt2");
    assert_eq!(texts, ["AX"]);

    // A layer an older PhotoCraft made from scratch beside the file's Txt2: what the builder
    // writes, less the superscript sizes Photoshop needs. The file is mixed already, so even an
    // unedited save fixes it.
    let mut old = kept.clone();
    old.psd_raw = Some(std::sync::Arc::new(without_document_sizes(old.psd_raw.as_deref().unwrap(), |_| {})));
    assert!(photocraft_text::psd::made_here_without_document_sizes(old.psd_raw.as_deref().unwrap(), 72.0));
    let (txt2, texts) = save(vec![old]);
    assert!(txt2.is_none(), "a file mixed by an older save drops its Txt2");
    assert_eq!(texts, ["AV"]);

    // A Photoshop type layer that lacks the same keys (as an older Photoshop might write it) is
    // not taken for one: Photoshop writes more than the builder does, here a key of its own.
    let mut photoshop = kept.clone();
    photoshop.psd_raw = Some(std::sync::Arc::new(without_document_sizes(photoshop.psd_raw.as_deref().unwrap(), |e| {
        if let Some(E::Dict(items)) = e.get_mut("DocumentResources") {
            items.push(("KinsokuSet".into(), E::Array(vec![])));
        }
    })));
    assert!(!photocraft_text::psd::made_here_without_document_sizes(photoshop.psd_raw.as_deref().unwrap(), 72.0));
    let (txt2, texts) = save(vec![photoshop]);
    assert_eq!(txt2.as_deref(), Some(&prev[..]), "an unedited file lacking the sizes keeps its Txt2 byte for byte");
    assert_eq!(texts, ["AV"]);

    let mut fresh = layer("Hi", &[(2, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut fresh, 72.0, photocraft_color::PixelFormat::RGBA8);
    let (txt2, texts) = save(vec![kept, fresh]);
    assert!(txt2.is_none(), "a new type layer drops the file's Txt2");
    assert_eq!(texts, ["AV", "Hi"], "every type layer is still live text");
}

/// A PhotoCraft-native `Txt2` carries what Photoshop 27 needs, found by building the block up in
/// Photoshop: to open the file, paragraph runs whose style is `<< /0 << /0 () /5 << >> /6 1 >> >>`
/// (an empty dictionary there makes it refuse the file), a paragraph style sheet (`/0 /6`) and the
/// document settings' `/1 /0 /2`; to draw the text (export, flatten), a character style sheet
/// (`/0 /5`). Type on a path asks for the block too: its path is in the frame.
#[test]
fn native_txt2_carries_what_photoshop_needs() {
    let mut t = layer("AV\rWA", &[(5, Kerning::Optical, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut t, 72.0, photocraft_color::PixelFormat::RGBA8);
    assert!(photocraft_text::psd::needs_txt2(&[(0, &t)]));
    let txt2 = photocraft_text::psd::parse_txt2(&photocraft_text::psd::build_txt2(&[(0, &t)], None)).unwrap();
    assert_eq!(txt2.path(&["1", "0", "2"]), Some(&E::Int(0)), "document settings");
    assert_eq!(txt2.path(&["0", "6", "0"]).and_then(E::as_array).map(<[E]>::len), Some(1), "a paragraph style sheet");
    assert_eq!(txt2.path(&["0", "5", "0"]).and_then(E::as_array).map(<[E]>::len), Some(1), "a character style sheet");
    let object = &txt2.path(&["1", "1"]).and_then(E::as_array).unwrap()[0];
    let runs = object.path(&["0", "5", "0"]).and_then(E::as_array).unwrap();
    assert_eq!(runs.len(), 2, "one run per paragraph");
    for run in runs {
        assert_eq!(run.path(&["0", "0", "6"]), Some(&E::Int(1)), "{run:?}");
        assert!(matches!(run.path(&["0", "0", "5"]), Some(E::Dict(_))), "{run:?}");
    }
    let mut on_path = layer("AV", &[(2, Kerning::Metrics, 0.0)]);
    on_path.shape = photocraft_doc::text::TextShape::Path {
        path: photocraft_doc::vector::Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)]),
        start: 0.0,
        flip: false,
    };
    assert!(photocraft_text::psd::needs_txt2(&[(0, &on_path)]), "type on a path is kept in Txt2");
}

/// Kerning that can't be represented (non-finite) and bad PSD fields never panic and stay sane.
#[test]
fn hostile_kerning_values() {
    let t = layer("AV", &[(1, Kerning::Off, f32::NAN), (1, Kerning::Metrics, f32::INFINITY)]);
    let tysh = photocraft_text::psd::build_tysh(&t, 72.0, None);
    assert!(kerning_fields(&tysh).iter().all(|f| f.1 == 0));
    let back = photocraft_text::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
    assert!(chars(&back).iter().all(|c| c.1 == 0.0));
    // Huge values clamp.
    let t = layer("AV", &[(1, Kerning::Off, 1e30), (1, Kerning::Metrics, 0.0)]);
    let f = kerning_fields(&photocraft_text::psd::build_tysh(&t, 72.0, None));
    assert_eq!(f[1].1, 10_000);
}

/// Glyph pen positions Photoshop stored for text object `index` in `Txt2` (`/21 /1`).
#[cfg(feature = "corpus")]
fn photoshop_positions(txt2: &E, index: usize) -> Option<Vec<f64>> {
    fn find(v: &E, depth: usize) -> Option<Vec<f64>> {
        if depth > 100 {
            return None;
        }
        match v {
            E::Dict(items) => {
                if let Some(p) = v.get("21").and_then(|d| d.get("1")).and_then(E::as_array) {
                    return Some(p.iter().filter_map(E::as_f64).collect());
                }
                items.iter().find_map(|(_, x)| find(x, depth + 1))
            }
            E::Array(a) => a.iter().find_map(|x| find(x, depth + 1)),
            _ => None,
        }
    }
    find(txt2.path(&["1", "1"])?.as_array()?.get(index)?, 0)
}

/// Photoshop-authored kerning oracles (`corpus/photoshop/text/kerning-*.psd`, made from scratch
/// by photocraft-corpus's generator): the automatic mode read from `Txt2` per character, the
/// EngineData kerning fields after export, and our layout against Photoshop's glyph positions.
#[cfg(feature = "corpus")]
#[test]
fn photoshop_kerning_oracles() {
    use Kerning::{Metrics as M, Off as O, Optical as P};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/text");
    assert!(dir.is_dir(), "{} is missing: run `cargo xtask corpus --all`", dir.display());
    let mut engine = photocraft_text::TextEngine::with_system_fonts();
    let arial = engine.fonts.has_family("Arial");
    // "AVATAR Wavy To. LT" (18 characters).
    let all = |m: Kerning| vec![(m, 0.0f32); 18];
    let partial: Vec<(Kerning, f32)> = (0..18).map(|i| (if (1..=3).contains(&i) { O } else { M }, 0.0)).collect();
    // (file, expected modes, EngineData fields as Photoshop wrote them survive our export,
    // worst / mean advance error allowed in px).
    type Case = (&'static str, Vec<(Kerning, f32)>, bool, f64, f64);
    let cases: [Case; 5] = [
        ("kerning-metrics.psd", all(M), true, 0.02, 0.02),
        ("kerning-off.psd", all(O), false, 0.02, 0.02),
        ("kerning-off-partial.psd", partial, false, 0.02, 0.02),
        // The fitted optical model: within ~30/1000 em per pair, ~12 on average.
        ("kerning-optical.psd", all(P), true, 1.0, 0.4),
        ("kerning-optical-24pt.psd", all(P), true, 0.75, 0.3),
    ];
    for (name, want, same_fields, worst_max, mean_max) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
        let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
        let doc = photocraft_io::import(name, &bytes).unwrap().document;
        let t = doc
            .layers
            .iter()
            .find_map(|l| match &l.content {
                LayerContent::Text(t) => Some(t),
                _ => None,
            })
            .unwrap();
        assert_eq!(chars(t), want, "{name}");

        let original = t.psd_raw.as_ref().map(|r| r.to_vec()).unwrap();
        let tysh = photocraft_text::psd::parse_tysh(&original).unwrap();
        let rebuilt = photocraft_text::psd::build_tysh(t, doc.resolution_dpi, None);
        if same_fields {
            assert_eq!(kerning_fields(&rebuilt), kerning_fields(&original), "{name}");
        }
        // Re-import (with the document's Txt2, as an exported file keeps it): same model.
        let mut back = photocraft_text::psd::text_layer_from_tysh(&rebuilt, doc.resolution_dpi).unwrap();
        photocraft_text::psd::apply_txt2(&mut back, &rebuilt, &txt2);
        assert_eq!(chars(&back), want, "{name} after export");

        // Layout against Photoshop's glyph positions (needs Arial, as Photoshop used).
        if !arial {
            eprintln!("{name}: Arial not installed, layout comparison skipped");
            continue;
        }
        let index = match &tysh.text.get("TextIndex") {
            Some(photocraft_psd::descriptor::Value::Integer(i)) => *i as usize,
            _ => panic!("{name}: no TextIndex"),
        };
        let ps = photoshop_positions(&txt2, index).unwrap();
        let x: Vec<f64> = engine.layout(t, doc.resolution_dpi).glyphs.iter().map(|g| f64::from(g.x)).collect();
        let (mut worst, mut sum) = (0.0f64, 0.0f64);
        for i in 0..x.len() - 1 {
            let d = (x[i + 1] - x[i]) - (ps[i + 1] - ps[i]);
            worst = worst.max(d.abs());
            sum += d.abs();
        }
        let mean = sum / (x.len() - 1) as f64;
        eprintln!("{name}: advance error vs Photoshop: mean {mean:.3} px, worst {worst:.3} px");
        assert!(worst <= worst_max && mean <= mean_max, "{name}: mean {mean}, worst {worst}");
    }
}
