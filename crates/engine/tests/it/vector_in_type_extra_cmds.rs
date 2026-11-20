//! Tests that run vector commands, moved from the engine-core `type_extra_cmds` tests: 2 test(s).

use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Path, Subpath};
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use photocraft_engine::Session;
use serde_json::json;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 120, "depth": depth, "background": "white"})).unwrap();
    s
}

fn text_layer(s: &mut Session, text: &str) -> LayerId {
    let r = s.execute("type.create", json!({"x": 20, "y": 70, "text": text, "size": 36, "color": "#000000"})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

fn text(s: &Session, id: LayerId) -> &TextLayer {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        c => panic!("not text: {}", c.kind_name()),
    }
}

fn ink(s: &Session, id: LayerId) -> photocraft_geom::Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

/// #9114: type on a path follows its own path when the path is edited (Direct Selection's
/// `path.*` commands with `"name": "layer"`), as Photoshop's Type Path does; the Paths list names
/// it, an edit is one undo step, and an edit that leaves no path is refused.
#[test]
fn type_follows_its_path_when_the_path_is_edited() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hello path");
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"start": 4})).unwrap();
    // In document space, the type path is where the work path was.
    let path = photocraft_engine::type_extra_cmds::type_path(text(&s, id)).unwrap();
    let ends: Vec<(f64, f64)> = path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect();
    assert!(ends.iter().zip([(150.0, 10.0), (150.0, 110.0)]).all(|(a, b)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6), "{ends:?}");
    let listed = s.execute("path.list", json!({})).unwrap();
    assert_eq!(listed["typePath"]["kind"], "typePath", "{listed}");
    // Swing the path's end to the right: the type now runs down and to the right.
    let before = ink(&s, id);
    let r = s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "anchors": [[0, 1]], "move": [120, 0]})).unwrap();
    assert_eq!(r["name"], "layer");
    let after = ink(&s, id);
    assert!(after.width() > before.width() + 20, "{before:?} → {after:?}");
    assert!(matches!(text(&s, id).shape, TextShape::Path { start, .. } if start == 4.0), "where the type starts is kept");
    // The work path it came from is untouched; Undo puts the type path back.
    assert_eq!(s.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[1].anchor.x, 150.0);
    assert!(s.undo());
    assert_eq!(ink(&s, id), before);
    // An edit that would leave fewer than two points is refused and changes nothing.
    assert!(s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id.0, "subpath": 0, "knot": 0})).is_err());
    assert_eq!(ink(&s, id), before);
}

/// #9114: a type-on-a-path layer's vector mask is still what `"name": "layer"` edits, as on main
/// and as `path.info` reads it; `"typePath": true` edits the Type Path; `path.list` reports both.
#[test]
fn a_vector_mask_on_type_on_a_path_is_edited_not_the_type_path() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hello path");
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({})).unwrap();
    let rect = json!({"subpaths": [{"closed": true, "knots": [[10, 10], [200, 10], [200, 100], [10, 100]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": id.0, "path": rect})).unwrap();
    let listed = s.execute("path.list", json!({})).unwrap();
    assert_eq!((listed["layerPath"]["kind"].as_str(), listed["typePath"]["kind"].as_str()), (Some("vectorMask"), Some("typePath")), "{listed}");
    let type_path = |s: &Session| photocraft_engine::type_extra_cmds::type_path(text(s, id)).unwrap();
    let before = type_path(&s);
    let mask = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().vector_mask.as_ref().unwrap().path.subpaths[0].knots[0].anchor;
    s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "anchors": [[0, 0]], "move": [10, 0]})).unwrap();
    assert_eq!(mask(&s), photocraft_geom::Point::new(20.0, 10.0), "the mask's corner moved");
    assert_eq!(type_path(&s), before, "the Type Path stays");
    s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "typePath": true, "anchors": [[0, 1]], "move": [20, 0]})).unwrap();
    assert!((type_path(&s).subpaths[0].knots[1].anchor.x - before.subpaths[0].knots[1].anchor.x - 20.0).abs() < 1e-6, "the Type Path moved");
    assert_eq!(mask(&s), photocraft_geom::Point::new(20.0, 10.0), "the mask stays");
}
