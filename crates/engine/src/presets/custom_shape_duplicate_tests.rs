//! #14291: Define Custom Shape keeps a second shape of a taken name, so the Shapes panel and the Shape picker address
//! a custom shape by its `position` in its group: renaming, deleting and placing one of two "TriS" changes or uses
//! only that one. A plain `preset` name still means the first shape of that name.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::{Value, json};

use super::CUSTOM_GROUP;
use crate::Session;
use crate::edit_menu_cmds::custom_shapes;

/// Two custom shapes named "TriS": a triangle first, then a square.
fn two_tris() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    for knots in [json!([[10, 10], [90, 10], [50, 80]]), json!([[100, 100], [200, 100], [200, 200], [100, 200]])] {
        s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": knots}]}})).unwrap();
        s.execute("edit.defineCustomShape", json!({"name": "TriS"})).unwrap();
    }
    s
}

fn knots(path: &photocraft_doc::Path) -> usize {
    path.subpaths.iter().map(|sp| sp.knots.len()).sum()
}

/// The custom shapes as (name, knots).
fn library(s: &Session) -> Vec<(String, usize)> {
    custom_shapes(s).iter().map(|c| (c.name.clone(), knots(&c.path))).collect()
}

/// The knots of the shape layer `shape.presets.place` made.
fn placed_knots(s: &mut Session, p: Value) -> usize {
    let r = s.execute("shape.presets.place", p).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => knots(&sh.path),
        other => panic!("not a shape layer: {other:?}"),
    }
}

fn edit(s: &mut Session, p: Value) -> crate::Result<Value> {
    s.execute("shape.presets.edit", p)
}

#[test]
fn placing_by_position_uses_the_chosen_shape() {
    let mut s = two_tris();
    assert_eq!(placed_knots(&mut s, json!({"preset": "TriS", "group": CUSTOM_GROUP, "position": 1})), 4, "the second TriS, the square");
    assert_eq!(placed_knots(&mut s, json!({"preset": "TriS", "group": CUSTOM_GROUP, "position": 0})), 3);
    assert_eq!(placed_knots(&mut s, json!({"group": CUSTOM_GROUP, "position": 1})), 4, "the position alone is enough");
    assert_eq!(placed_knots(&mut s, json!({"preset": "TriS"})), 3, "a name alone is the first of that name");
}

#[test]
fn renaming_by_position_renames_only_the_chosen_shape() {
    let mut s = two_tris();
    edit(&mut s, json!({"action": "rename", "preset": "TriS", "group": CUSTOM_GROUP, "position": 1, "name": "Square"})).unwrap();
    assert_eq!(library(&s), [("TriS".to_string(), 3), ("Square".to_string(), 4)]);
}

#[test]
fn deleting_by_position_deletes_only_the_chosen_shape() {
    let mut s = two_tris();
    edit(&mut s, json!({"action": "delete", "preset": "TriS", "group": CUSTOM_GROUP, "position": 1})).unwrap();
    assert_eq!(library(&s), [("TriS".to_string(), 3)]);
    let mut s = two_tris();
    edit(&mut s, json!({"action": "delete", "group": CUSTOM_GROUP, "position": 0})).unwrap();
    assert_eq!(library(&s), [("TriS".to_string(), 4)]);
}

#[test]
fn a_name_alone_renames_and_deletes_the_first_of_that_name() {
    let mut s = two_tris();
    edit(&mut s, json!({"action": "delete", "preset": "TriS"})).unwrap();
    assert_eq!(library(&s), [("TriS".to_string(), 4)], "one shape is deleted, not every TriS");
    let mut s = two_tris();
    edit(&mut s, json!({"action": "rename", "preset": "TriS", "name": "First"})).unwrap();
    assert_eq!(library(&s), [("First".to_string(), 3), ("TriS".to_string(), 4)]);
}

#[test]
fn a_bad_position_is_an_error_that_changes_nothing() {
    let mut s = two_tris();
    let bad = [
        json!({"position": 1}),
        json!({"group": CUSTOM_GROUP, "position": 2}),
        json!({"group": CUSTOM_GROUP, "position": -1}),
        json!({"group": CUSTOM_GROUP, "position": 0.5}),
        json!({"group": CUSTOM_GROUP, "position": "1"}),
        json!({"group": "No Such Group", "position": 0}),
        json!({"preset": "Other", "group": CUSTOM_GROUP, "position": 1}),
    ];
    for p in bad {
        let mut place = p.clone();
        place["rect"] = json!([0, 0, 50, 50]);
        assert!(s.execute("shape.presets.place", place).is_err(), "place {p}");
        let mut rename = p.clone();
        rename["action"] = json!("rename");
        rename["name"] = json!("X");
        assert!(edit(&mut s, rename).is_err(), "rename {p}");
        let mut delete = p.clone();
        delete["action"] = json!("delete");
        assert!(edit(&mut s, delete).is_err(), "delete {p}");
    }
    assert_eq!(library(&s), [("TriS".to_string(), 3), ("TriS".to_string(), 4)]);
    assert_eq!(s.active().unwrap().doc.walk().len(), 1, "no shape layer was placed");
}
