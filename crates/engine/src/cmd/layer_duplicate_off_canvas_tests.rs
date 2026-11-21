//! #14606: Duplicate Layer into another document (`layer.copyToDocument` with `centerOutside`) keeps the place of a
//! copy whose bounds meet the destination's canvas and centres one that would land wholly off it; several layers move
//! by one offset. Without `centerOutside` (dragging layers between documents) the copies keep their place as before.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;

/// Document 0: 400×300 with pixel layers `(name, x, y, w, h)` above the Background. Document 1: 200×100. Document 0
/// is active.
fn docs(layers: &[(&str, i64, i64, i64, i64)]) -> (Session, Vec<LayerId>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut ids = Vec::new();
    for &(name, x, y, w, h) in layers {
        ids.push(LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()));
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.set_active(0);
    (s, ids)
}

fn copy(s: &mut Session, ids: &[LayerId], centre_outside: bool) -> Value {
    let layers: Vec<u64> = ids.iter().map(|id| id.0).collect();
    s.execute("layer.copyToDocument", json!({"source": 0, "document": 1, "layers": layers, "centerOutside": centre_outside})).unwrap()
}

/// Where copy `k` of reply `r` landed in document 1.
fn landed(s: &Session, r: &Value, k: usize) -> (i32, i32) {
    let id = LayerId(r["layers"][k].as_u64().unwrap());
    let doc = &s.documents()[1].doc;
    let b: Rect = crate::layer_multi_cmds::layer_bounds(doc.layer(id).unwrap()).unwrap();
    (b.x0, b.y0)
}

#[test]
fn a_copy_that_meets_the_canvas_keeps_its_place() {
    for (x, y, w, h) in [(30, 30, 20, 20), (190, 40, 20, 20), (0, 10, 250, 20), (50, 50, 300, 200)] {
        let (mut s, ids) = docs(&[("L", x, y, w, h)]);
        let r = copy(&mut s, &ids, true);
        assert_eq!(landed(&s, &r, 0), (x as i32, y as i32), "{w}×{h} at {x},{y}");
    }
}

#[test]
fn a_copy_wholly_off_the_canvas_is_centred_on_it() {
    let (mut s, ids) = docs(&[("Top", 175, 125, 50, 50)]);
    let r = copy(&mut s, &ids, true);
    assert_eq!(landed(&s, &r, 0), (75, 25));
    // An odd size: the half pixel goes to the top left.
    let (mut s, ids) = docs(&[("Small", 11, 271, 9, 9)]);
    let r = copy(&mut s, &ids, true);
    assert_eq!(landed(&s, &r, 0), (95, 45));
}

#[test]
fn several_layers_move_by_one_offset() {
    let (mut s, ids) = docs(&[("A", 220, 150, 20, 20), ("B", 260, 190, 30, 10)]);
    let r = copy(&mut s, &ids, true);
    let (a, b) = (landed(&s, &r, 0), landed(&s, &r, 1));
    assert_eq!((b.0 - a.0, b.1 - a.1), (40, 40), "the copies keep their layout");
    // Their joint bounds, 70×50, are centred on the 200×100 canvas.
    assert_eq!(a, (65, 25));
}

#[test]
fn without_the_rule_copies_keep_their_place_as_before() {
    let (mut s, ids) = docs(&[("Top", 175, 125, 50, 50)]);
    let r = copy(&mut s, &ids, false);
    assert_eq!(landed(&s, &r, 0), (175, 125), "a drag between documents keeps today's placement");
}
