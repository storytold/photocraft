//! Tests that run vector commands, moved from the engine-core `layer_copy_cmds` tests: 5 test(s).

use photocraft_color::ColorMode;

use photocraft_doc::{Document, LayerContent, LayerId};

use serde_json::{Value, json};

use photocraft_engine::Session;

fn doc(s: &Session, i: usize) -> &Document {
    &s.documents()[i].doc
}

fn copied(r: &Value) -> LayerId {
    LayerId(r["layers"][0].as_u64().unwrap())
}

/// The type layer `id` of document `i`.
fn text(s: &Session, i: usize, id: LayerId) -> &photocraft_doc::TextLayer {
    match &doc(s, i).layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        other => panic!("not type: {other:?}"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * b.abs().max(1.0)
}

/// Document 0 at 72 ppi with "Hello" in Paragraph Style 1 (24 pt) and "He" also in Character
/// Style 1 (24 pt, underlined); document 1 at 300 ppi, its own styles made by `dest`.
fn styled_type_docs(dest: impl FnOnce(&mut Session)) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 100, "y": 100})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.paragraphStyle.new", json!({"attrs": {"size": 24}, "fromSelection": false, "apply": true, "layer": t})).unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"size": 24, "underline": true}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]}))
        .unwrap();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 300})).unwrap();
    dest(&mut s);
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The styles panel state of `range` in layer `id` of document 1.
fn styles_of(s: &mut Session, id: LayerId, range: [usize; 2]) -> (Value, Value) {
    s.set_active(1);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    (c, p)
}

/// #10500: type copied to a document of another resolution brings the styles it uses, as in
/// the reference application (measured with Duplicate Layer from 72 to 300 ppi): a style the
/// destination lacks arrives under its name, scaled like the text (24 pt → 5.76 pt), so the
/// copy has no override; it never takes over an unrelated destination style with the same id.
#[test]
fn copied_type_brings_its_styles_scaled() {
    for paste in [false, true] {
        let (mut s, _) = styled_type_docs(|s| {
            s.execute("type.characterStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
            s.execute("type.paragraphStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
        });
        let id = if paste {
            s.execute("edit.copy", json!({})).unwrap();
            s.set_active(1);
            LayerId(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap()["layer"].as_u64().unwrap())
        } else {
            copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap())
        };
        assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76), "paste {paste}");
        let (c, p) = styles_of(&mut s, id, [0, 2]);
        let names = |v: &Value| v["styles"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&c), ["None", "Unrelated", "Character Style 1"], "paste {paste}");
        assert_eq!(names(&p), ["Basic Paragraph", "Unrelated", "Paragraph Style 1"], "paste {paste}");
        let (cs, ps) = (&c["styles"][2], &p["styles"][2]);
        assert!(close(cs["attrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{cs}");
        assert!(close(ps["characterAttrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{ps}");
        assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&cs["id"], &json!(false)), "paste {paste}");
        assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&ps["id"], &json!(false)), "paste {paste}");
        // The source keeps its 24 pt styles.
        assert_eq!(doc(&s, 0).text_styles.character[0].attrs["size_pt"], json!(24.0));
    }
}

/// A destination that already has a style of the same name keeps its definition (24 pt), and
/// the copy, at 5.76 pt, shows a local override ("+"), as in the reference application.
#[test]
fn copied_type_uses_the_destinations_same_named_style() {
    let (mut s, _) = styled_type_docs(|s| {
        s.execute("type.characterStyle.new", json!({"name": "Filler", "fromSelection": false})).unwrap();
        s.execute("type.characterStyle.new", json!({"name": "Character Style 1", "attrs": {"size": 24, "underline": true}, "fromSelection": false})).unwrap();
    });
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let (c, _) = styles_of(&mut s, id, [0, 2]);
    assert_eq!(c["styles"].as_array().unwrap().len(), 3, "no second Character Style 1: {c}");
    let existing = &c["styles"][2];
    assert_eq!(existing["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&existing["id"], &json!(true)));
    assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76));
}

/// Duplicate Layer › New Document brings the styles the type uses (at the same size).
#[test]
fn type_duplicated_to_a_new_document_keeps_its_styles() {
    let (mut s, t) = styled_type_docs(|_| {});
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    assert_eq!(c["styles"][1]["name"], "Character Style 1");
    assert_eq!(c["styles"][1]["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&c["styles"][1]["id"], &json!(false)));
    assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&p["styles"][1]["id"], &json!(false)));
    assert_eq!(text(&s, i, id).runs, text(&s, 0, t).runs);
}

/// Document 0 (RGB, 72 ppi) with "Hello" whose "He" is in Character Style 1 (red); document 1
/// (CMYK, 72 ppi). The type layer is selected in document 0.
fn red_styled_type_docs() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"color": "#ff0000"}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]})).unwrap();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72, "mode": "cmyk"})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The colour Character Style 1 of document `i` sets, and whether "He" of `id` overrides it.
fn style_color(s: &mut Session, i: usize, id: LayerId) -> (photocraft_color::Color, bool) {
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let style = c["styles"].as_array().unwrap().iter().find(|x| x["name"] == "Character Style 1").unwrap();
    (serde_json::from_value(style["attrs"]["color"].clone()).unwrap(), c["current"]["characterOverride"].as_bool().unwrap())
}

/// Type copied to a document of another colour mode converts its run colours, and the style it
/// brings converts with the same transform: the run matches its style (no override), so a later
/// change to the style's colour still reaches it. Converting the document itself (Image › Mode,
/// Convert to Profile with and without dither) keeps its styles in step the same way.
#[test]
fn copied_type_styles_convert_their_colours_with_the_type() {
    let (mut s, _) = red_styled_type_docs();
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let run = text(&s, 1, id).runs[0].style.color;
    assert_eq!(run.mode, ColorMode::Cmyk, "the run converted");
    let (style, over) = style_color(&mut s, 1, id);
    assert_eq!(style, run, "the copied style converted with the run");
    assert!(!over, "no false override");
    for (cmd, p) in [
        ("image.mode.cmyk", json!({})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": false})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": true})),
        ("edit.convertToProfile", json!({"profile": "lab-d50"})),
    ] {
        let (mut s, t) = red_styled_type_docs();
        s.execute(cmd, p.clone()).unwrap();
        let run = text(&s, 0, t).runs[0].style.color;
        assert_ne!(run.mode, ColorMode::Rgb, "{cmd} {p}: the run converted");
        let (style, over) = style_color(&mut s, 0, t);
        assert_eq!(style, run, "{cmd} {p}: the style converted with the run");
        assert!(!over, "{cmd} {p}: no false override");
    }
}

/// Duplicate Layer › New Document keeps the source's Basic Paragraph: type that inherits its
/// size from it has no override there, and Clear Override leaves it as it was.
#[test]
fn type_duplicated_to_a_new_document_keeps_the_sources_basic_paragraph() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let t = LayerId(s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap());
    s.execute("type.paragraphStyle.set", json!({"id": 0, "attrs": {"size": 30}})).unwrap();
    assert!(close(text(&s, 0, t).runs[0].style.size_pt, 30.0), "the type inherits Basic Paragraph");
    let basic = doc(&s, 0).text_styles.basic.clone();
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    assert_eq!(doc(&s, i).text_styles.basic, basic);
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0})).unwrap();
    assert_eq!((&c["current"]["characterOverride"], &p["current"]["paragraphOverride"]), (&json!(false), &json!(false)), "{c} {p}");
    s.execute("type.paragraphStyle.clearOverride", json!({"layer": id.0})).unwrap();
    s.execute("type.characterStyle.clearOverride", json!({"layer": id.0})).unwrap();
    assert!(close(text(&s, i, id).runs[0].style.size_pt, 30.0), "{}", text(&s, i, id).runs[0].style.size_pt);
}
