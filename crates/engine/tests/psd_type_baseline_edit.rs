//! #10999: entering a Photoshop paragraph type layer redraws for caret placement without
//! translating its text. The synthetic `Txt2` line stands in for Photoshop's rendered baseline;
//! Inter is bundled, so neither the public PSD nor its proprietary font is a test dependency.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::text::TextShape;
use photocraft_doc::{Document, Layer, LayerContent, Size, TextLayer};
use photocraft_engine::Session;
use photocraft_geom::Affine;
use photocraft_text::engine_data::{self, Value as E};
use serde_json::json;

fn dict(items: Vec<(&str, E)>) -> E {
    E::Dict(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

fn synthetic_psd() -> Vec<u8> {
    let mut doc = Document::new("baseline", Size::new(500, 300), ColorMode::Rgb, SampleType::U8);
    let mut t = TextLayer {
        text: "Hello".into(),
        font_family: "Inter".into(),
        size_pt: 36.0,
        shape: TextShape::Box { x: 0.0, y: 0.0, width: 200.0, height: 100.0 },
        transform: Affine::translate(70.0, 80.0),
        ..Default::default()
    };
    let mut engine = photocraft_text::TextEngine::new();
    let first = engine.layout(&t, 72.0).lines[0].baseline;
    let recorded = first + 15.0;
    // Photoshop's cached pixels are drawn at its recorded baseline. The layer's editable model
    // has the same box and transform but needs the `Txt2` line to reproduce that position.
    let mut cached = t.clone();
    cached.shape = TextShape::Box { x: 0.0, y: 15.0, width: 200.0, height: 100.0 };
    t.cache = Some(engine.render(&cached, 72.0, doc.pixel_format()).1.surface);
    let tysh = photocraft_text::psd::build_tysh(&t, 72.0, None);
    t.psd_raw = Some(Arc::new(photocraft_text::psd::set_text_index(&tysh, 0).unwrap()));

    let raw = photocraft_text::psd::build_txt2(&[(0, &t)], None);
    let mut txt2 = photocraft_text::psd::parse_txt2(&raw).unwrap();
    let E::Array(objects) = txt2.get_mut("1").and_then(|v| v.get_mut("1")).unwrap() else { panic!("text objects") };
    let object = objects.get_mut(0).unwrap();
    let line = dict(vec![
        ("99", E::Name("L".into())),
        ("10", E::Real(f64::from(recorded))),
        ("0", dict(vec![("0", E::Array(vec![E::Real(0.0), E::Real(f64::from(recorded))]))])),
    ]);
    let region = dict(vec![("99", E::Name("R".into())), ("6", E::Array(vec![line]))]);
    let region = dict(vec![("99", E::Name("R".into())), ("6", E::Array(vec![region]))]);
    let frame = dict(vec![("99", E::Name("F".into())), ("10", E::Int(0)), ("6", E::Array(vec![region]))]);
    let pc = dict(vec![("99", E::Name("PC".into())), ("6", E::Array(vec![frame]))]);
    match object.get_mut("1") {
        Some(view) => view.set("2", E::Array(vec![pc])),
        None => object.set("1", dict(vec![("2", E::Array(vec![pc]))])),
    }
    txt2.get_mut("0").unwrap().set("8", dict(vec![("0", E::Array(vec![dict(vec![("0", E::dict())])]))]));
    let E::Dict(items) = txt2 else { panic!("Txt2") };
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"Txt2", Arc::new(engine_data::write_bare(&items))));
    doc.layers.push(Layer::new("Hello", LayerContent::Text(t)));
    photocraft_io::export(&doc, "baseline.psd", &Default::default()).unwrap().bytes
}

fn text(session: &Session, id: photocraft_doc::LayerId) -> &TextLayer {
    let LayerContent::Text(t) = &session.active().unwrap().doc.layer(id).unwrap().content else { panic!("type layer") };
    t
}

#[test]
fn entering_psd_type_edit_without_typing_keeps_the_recorded_baseline() {
    let bytes = synthetic_psd();
    let imported = photocraft_io::import("baseline.psd", &bytes).unwrap().document;
    let id = imported.layers[0].id;
    let mut session = Session::new();
    session.add_document(imported, None);
    let original = text(&session, id).cache.as_ref().unwrap().content_bounds();
    let original_text = text(&session, id).text.clone();
    let native = photocraft_io::export(&session.active().unwrap().doc, "baseline.pcraft", &Default::default()).unwrap().bytes;
    let native = photocraft_io::import("baseline.pcraft", &native).unwrap().document;
    let LayerContent::Text(native_type) = &native.layers[0].content else { panic!("type layer") };
    assert_eq!(native_type.psd_first_baseline, text(&session, id).psd_first_baseline);
    session.execute("type.edit", json!({"layer": id.0})).unwrap();
    let edited = text(&session, id).cache.as_ref().unwrap().content_bounds();
    assert!((edited.y0 - original.y0).abs() <= 1, "redraw shifted vertically: {original:?} → {edited:?}");
    assert!((edited.x0 - original.x0).abs() <= 1);
    assert_eq!(text(&session, id).text, original_text);
    assert!(session.undo());
    assert_eq!(text(&session, id).cache.as_ref().unwrap().content_bounds(), original);
    assert!(session.redo());
    assert_eq!(text(&session, id).cache.as_ref().unwrap().content_bounds(), edited);
    session.execute("type.edit", json!({"layer": id.0, "replace": {"start": 5, "end": 5, "text": "!"}})).unwrap();
    assert_eq!(text(&session, id).text, "Hello!");
    let layout = photocraft_text::TextEngine::new().layout(text(&session, id), 72.0);
    assert_eq!(layout.lines.len(), 1);
    assert!(layout.lines[0].baseline > 30.0, "caret layout uses the PSD baseline");
    let saved = photocraft_io::export(&session.active().unwrap().doc, "baseline.psd", &Default::default()).unwrap().bytes;
    let reopened = photocraft_io::import("baseline.psd", &saved).unwrap().document;
    let LayerContent::Text(reopened) = &reopened.layers[0].content else { panic!("type layer") };
    assert_eq!(reopened.text, "Hello!");
    assert_eq!(reopened.cache.as_ref().unwrap().content_bounds(), text(&session, id).cache.as_ref().unwrap().content_bounds());
    let recorded = text(&session, id).psd_first_baseline.as_ref().unwrap().offset_px;
    session.execute("type.setStyle", json!({"layer": id.0, "size": 48.0})).unwrap();
    let resized = photocraft_text::TextEngine::new().layout(text(&session, id), 72.0);
    assert!((resized.lines[0].baseline - recorded).abs() > 1.0, "a changed font size invalidates the PSD baseline");
}

#[test]
fn invalid_native_psd_baseline_does_not_hide_the_first_line() {
    let mut imported = photocraft_io::import("baseline.psd", &synthetic_psd()).unwrap().document;
    let LayerContent::Text(layer) = &mut imported.layers[0].content else { panic!("type layer") };
    let expected = {
        let mut without_hint = layer.clone();
        without_hint.psd_first_baseline = None;
        photocraft_text::TextEngine::new().layout(&without_hint, 72.0).lines[0].baseline
    };
    layer.psd_first_baseline.as_mut().unwrap().offset_px = 1000.0;
    let native = photocraft_io::export(&imported, "baseline.pcraft", &Default::default()).unwrap().bytes;
    let reopened = photocraft_io::import("baseline.pcraft", &native).unwrap().document;
    let LayerContent::Text(layer) = &reopened.layers[0].content else { panic!("type layer") };
    let layout = photocraft_text::TextEngine::new().layout(layer, 72.0);
    assert_eq!(layout.lines.len(), 1);
    assert_eq!(layout.lines[0].baseline, expected);
}
