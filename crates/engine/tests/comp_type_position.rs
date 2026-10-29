//! A comp restores editable Type placement independently of the current glyph's ink bounds.

use photocraft_doc::{LayerContent, LayerId, TextLayer};
use photocraft_engine::Session;
use serde_json::json;

fn text(s: &Session, id: LayerId) -> &TextLayer {
    let LayerContent::Text(t) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("type") };
    t
}

#[test]
fn comp_restores_edited_type_origin_without_reverting_content_or_style() {
    for depth in [8, 16, 32] {
        for (value, size) in [("H", 72), ("M", 48), ("M", 72)] {
            for (position, appearance) in [(true, false), (true, true), (false, true)] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 512, "height": 256, "depth": depth, "background": "white"})).unwrap();
                let id = LayerId(
                    s.execute("type.create", json!({"x": 100.25, "y": 150.75, "text": "H", "size": 48, "color": "#ff0000"})).unwrap()["layer"]
                        .as_u64()
                        .unwrap(),
                );
                let original = text(&s, id).transform;
                s.execute("layerComp.new", json!({"name": "Original", "visibility": false, "position": position, "appearance": appearance})).unwrap();
                s.execute("type.edit", json!({"layer": id.0, "text": value})).unwrap();
                s.execute("type.setStyle", json!({"layer": id.0, "size": size, "color": "#0000ff"})).unwrap();
                assert_eq!(text(&s, id).transform, original, "editing alone preserves placement");
                s.execute("layer.translate", json!({"layer": id.0, "dx": 40, "dy": 20})).unwrap();
                let edited = text(&s, id).clone();
                let mut expected = edited.clone();
                if position {
                    expected.transform = original;
                }
                s.execute("layerComp.apply", json!({"comp": "Original"})).unwrap();
                let applied = text(&s, id).clone();
                assert_eq!(applied.transform, expected.transform, "{depth}-bit {value}{size}, position={position}, appearance={appearance}");
                assert_eq!(applied.text, edited.text);
                assert_eq!(applied.runs, edited.runs);
                assert_eq!(applied.paragraphs, edited.paragraphs);
                assert!(s.undo());
                assert_eq!(text(&s, id), &edited);
                assert!(s.redo());
                assert_eq!(text(&s, id), &applied);
                s.execute("layerComp.apply", json!({"comp": "Original"})).unwrap();
                assert_eq!(text(&s, id), &applied, "reapplying is idempotent");
                if position {
                    s.execute("layerComp.restoreLastDocumentState", json!({})).unwrap();
                    assert_eq!(text(&s, id), &edited, "reapplying must retain the edited Last Document State");
                }
            }
        }
    }
}

#[test]
fn edited_type_comp_placement_survives_native_and_psd_reopening() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 512, "height": 256, "depth": depth, "background": "white"})).unwrap();
        let id = LayerId(s.execute("type.create", json!({"x": 100, "y": 150, "text": "H", "size": 48})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layerComp.new", json!({"name": "Original", "visibility": false, "appearance": false})).unwrap();
        s.execute("type.edit", json!({"layer": id.0, "text": "M"})).unwrap();
        s.execute("type.setStyle", json!({"layer": id.0, "size": 72})).unwrap();
        s.execute("layer.translate", json!({"layer": id.0, "dx": 40, "dy": 20})).unwrap();
        let doc = &s.active().unwrap().doc;
        let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        let psd = photocraft_io::export(doc, "comp.psd", &Default::default()).unwrap().bytes;
        for reopened in [photocraft_format::load_from_bytes(&native).unwrap(), photocraft_io::import("comp.psd", &psd).unwrap().document] {
            let id = reopened.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).unwrap().2.id;
            let mut s = Session::new();
            s.add_document(reopened, None);
            let edited = text(&s, id).clone();
            assert_eq!(&edited.transform.m[4..], &[140.0, 170.0]);
            s.execute("layerComp.apply", json!({"comp": "Original"})).unwrap();
            assert_eq!(&text(&s, id).transform.m[4..], &[100.0, 150.0], "{depth}-bit reopen");
            assert_eq!(text(&s, id).text, edited.text);
            assert_eq!(text(&s, id).runs, edited.runs);
            let applied = s.active().unwrap().doc.as_ref().clone();
            let bytes = photocraft_format::save_to_bytes(&applied, &Default::default()).unwrap();
            let back = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(back, applied, "applied comp and Last Document State persist");
            s.execute("layerComp.restoreLastDocumentState", json!({})).unwrap();
            assert_eq!(text(&s, id), &edited);
        }
    }
}

#[test]
fn updating_position_recaptures_the_type_anchor_independently_of_appearance() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 512, "height": 256})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"x": 100, "y": 150, "text": "H", "size": 48})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layerComp.new", json!({"name": "Original"})).unwrap();
    s.execute("layer.translate", json!({"layer": id.0, "dx": 40, "dy": 20})).unwrap();
    s.execute("layerComp.update", json!({"comp": "Original", "what": "position"})).unwrap();
    let captured = text(&s, id).transform;
    s.execute("type.edit", json!({"layer": id.0, "text": "M"})).unwrap();
    s.execute("type.setStyle", json!({"layer": id.0, "size": 72})).unwrap();
    s.execute("layer.translate", json!({"layer": id.0, "dx": -160, "dy": -180})).unwrap();
    s.execute("layerComp.update", json!({"comp": "Original", "what": "appearance"})).unwrap();
    s.execute("layerComp.apply", json!({"comp": "Original"})).unwrap();
    assert_eq!(text(&s, id).transform, captured);
    assert_eq!(text(&s, id).text, "M");
    assert_eq!(text(&s, id).size_pt, 72.0);
}
