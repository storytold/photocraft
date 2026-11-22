//! #14291: the name Define Custom Shape stores. An empty or spaces-only name is refused with the reference's alert
//! text and stores nothing; spaces are kept; a name is cut at 240 characters, never inside one; a name that is taken
//! adds a second shape instead of replacing the first.

use photocraft_doc::vector::Subpath;
use serde_json::{Value, json};

use super::{MAX_SHAPE_NAME, Path, SHAPE_NAME_REQUIRED, custom_shape_name};
use crate::Session;
use crate::edit_menu_cmds::custom_shapes;

fn square(x: f64, w: f64) -> Path {
    Path::new(vec![Subpath::polygon(&[(x, x), (x + w, x), (x + w, x + w), (x, x + w)])])
}

/// A document whose work path is a `w` square at (`x`, `x`).
fn with_square(x: f64, w: f64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 160, "height": 120, "background": "white"})).unwrap();
    set_work_path(&mut s, square(x, w));
    s
}

fn set_work_path(s: &mut Session, path: Path) {
    s.edit("path", |doc, _| {
        doc.work_path = Some(path);
        Ok(())
    })
    .unwrap();
}

fn names(s: &Session) -> Vec<String> {
    custom_shapes(s).iter().map(|c| c.name.clone()).collect()
}

fn define(s: &mut Session, name: &str) -> crate::Result<Value> {
    s.execute("edit.defineCustomShape", json!({"name": name}))
}

#[test]
fn an_empty_or_spaces_only_name_is_refused_and_stores_nothing() {
    for typed in ["", " ", "     "] {
        let mut s = with_square(10.0, 20.0);
        let err = define(&mut s, typed).expect_err("an empty name is refused");
        assert!(err.to_string().contains(SHAPE_NAME_REQUIRED), "{typed:?}: {err}");
        assert!(names(&s).is_empty(), "{typed:?}: nothing is stored");
        assert_eq!(custom_shape_name(typed), None);
    }
    assert_eq!(SHAPE_NAME_REQUIRED, "You must specify a name for the shape.");
}

#[test]
fn leading_and_trailing_spaces_are_kept() {
    let mut s = with_square(10.0, 20.0);
    let r = define(&mut s, "  RectA  ").unwrap();
    assert_eq!(r["name"], "  RectA  ");
    assert_eq!(names(&s), ["  RectA  "]);
}

#[test]
fn a_long_name_is_cut_at_240_characters() {
    let mut s = with_square(10.0, 20.0);
    let typed: String = ('a'..='z').cycle().take(300).collect();
    define(&mut s, &typed).unwrap();
    assert_eq!(MAX_SHAPE_NAME, 240);
    assert_eq!(names(&s), [typed[..240].to_string()]);
    // Exactly 240 is kept whole.
    let exact = "b".repeat(240);
    define(&mut s, &exact).unwrap();
    assert_eq!(names(&s)[1], exact);
}

/// Not measured: characters outside basic Latin count as one each, and the cut never splits one.
#[test]
fn the_cut_counts_characters_not_bytes() {
    let typed = "ä".repeat(250);
    let mut s = with_square(10.0, 20.0);
    define(&mut s, &typed).unwrap();
    let stored = &names(&s)[0];
    assert_eq!(stored.chars().count(), 240);
    assert_eq!(stored, &"ä".repeat(240));
    assert_eq!(custom_shape_name(&"🙂".repeat(241)).map(|n| n.chars().count()), Some(240));
}

#[test]
fn a_taken_name_adds_a_second_shape() {
    let mut s = with_square(10.0, 20.0);
    define(&mut s, "TriS").unwrap();
    let first = custom_shapes(&s)[0].path.clone();
    set_work_path(&mut s, square(40.0, 50.0));
    let r = define(&mut s, "TriS").unwrap();
    assert_eq!(r["count"], 2);
    assert_eq!(names(&s), ["TriS", "TriS"]);
    assert_eq!(custom_shapes(&s)[0].path, first, "the first keeps its own path");
    assert_eq!(custom_shapes(&s)[1].path, square(40.0, 50.0));
}

#[test]
fn the_preset_manager_renames_and_deletes_duplicates_by_index() {
    let mut s = with_square(10.0, 20.0);
    define(&mut s, "TriS").unwrap();
    set_work_path(&mut s, square(40.0, 50.0));
    define(&mut s, "TriS").unwrap();
    let list = s.execute("edit.presets.presetManager", json!({"kind": "customShapes"})).unwrap();
    assert_eq!(list["customShapes"], json!(["TriS", "TriS"]));
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "customShapes", "index": 1, "newName": "Second"})).unwrap();
    assert_eq!(names(&s), ["TriS", "Second"]);
    assert_eq!(custom_shapes(&s)[1].path, square(40.0, 50.0), "the renamed one is the second");
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "customShapes", "index": 1, "newName": "TriS"})).unwrap();
    s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "customShapes", "index": 0})).unwrap();
    assert_eq!(names(&s), ["TriS"]);
    assert_eq!(custom_shapes(&s)[0].path, square(40.0, 50.0), "only the first was deleted");
}

/// A script that gives no `name` keeps today's default; a `name` that isn't text is an error, not a panic.
#[test]
fn script_params_fail_cleanly() {
    let mut s = with_square(10.0, 20.0);
    let r = s.execute("edit.defineCustomShape", json!({})).unwrap();
    assert_eq!(r["name"], "Shape 1");
    let r = s.execute("edit.defineCustomShape", json!({"name": null})).unwrap();
    assert_eq!(r["name"], "Shape 2");
    for bad in [json!(7), json!(true), json!(["a"]), json!({"a": 1})] {
        assert!(s.execute("edit.defineCustomShape", json!({"name": bad})).is_err(), "{bad}");
    }
    assert_eq!(names(&s).len(), 2);
}
