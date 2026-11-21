//! #14296: Define Custom Shape results are listed at the top level, after the last group, in the
//! order defined, not inside a "Custom Shapes" group.

use photocraft_doc::{Path, Subpath};
use serde_json::{Value, json};

use super::CUSTOM_GROUP;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 160, "background": "white"})).unwrap();
    s
}

/// Defines a custom shape `name` from a polygon set as the work path.
fn define(s: &mut Session, name: &str, pts: &[(f64, f64)]) {
    let path = Path::new(vec![Subpath::polygon(pts)]);
    s.edit("path", |doc, _| {
        doc.work_path = Some(path);
        Ok(())
    })
    .unwrap();
    s.execute("edit.defineCustomShape", json!({"name": name})).unwrap();
}

fn define_two(s: &mut Session) {
    define(s, "RectA", &[(20.0, 20.0), (90.0, 20.0), (90.0, 70.0), (20.0, 70.0)]);
    define(s, "TriS", &[(110.0, 120.0), (150.0, 40.0), (190.0, 120.0)]);
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
}

fn group_names(list: &Value) -> Vec<String> {
    names(&list["groups"])
}

#[test]
fn defined_shapes_follow_the_last_group_at_the_top_level_in_order() {
    let mut s = session();
    let defaults = group_names(&s.execute("shape.presets.list", json!({})).unwrap());
    define_two(&mut s);
    let list = s.execute("shape.presets.list", json!({})).unwrap();
    assert_eq!(group_names(&list), defaults, "the groups are the default ones, no {CUSTOM_GROUP:?} group");
    assert_eq!(names(&list["shapes"]), ["RectA", "TriS"]);
    assert_eq!(list["shapes"][1]["subpaths"], 1);
}

#[test]
fn top_level_shapes_still_place_rename_and_delete() {
    let mut s = session();
    define_two(&mut s);
    let placed = s.execute("shape.presets.place", json!({"preset": "TriS", "rect": [10, 10, 60, 50]})).unwrap();
    assert!(placed["layer"].as_u64().is_some(), "{placed}");
    s.execute("shape.presets.edit", json!({"action": "rename", "preset": "RectA", "name": "Box"})).unwrap();
    assert_eq!(names(&s.execute("shape.presets.list", json!({})).unwrap()["shapes"]), ["Box", "TriS"]);
    s.execute("shape.presets.edit", json!({"action": "delete", "preset": "TriS"})).unwrap();
    assert_eq!(names(&s.execute("shape.presets.list", json!({})).unwrap()["shapes"]), ["Box"]);
}

#[test]
fn custom_shapes_saved_before_still_load_at_the_top_level() {
    let mut s = session();
    define_two(&mut s);
    let saved = s.presets.to_json(&s);
    let mut later = Session::new();
    later.load_presets_json(saved);
    let list = later.execute("shape.presets.list", json!({})).unwrap();
    assert_eq!(names(&list["shapes"]), ["RectA", "TriS"]);
    assert!(!group_names(&list).iter().any(|g| g == CUSTOM_GROUP));
}
