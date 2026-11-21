//! #14295: Define Custom Shape stores nothing for a path with no area. A single anchor or a straight
//! line is refused with the reference's alert text; a path that only subtracts is refused with no
//! alert (the menu does nothing). A 1×1 square still works.

use photocraft_doc::{Knot, Path, PathOp, Subpath};
use serde_json::json;

use super::{EMPTY_PATH, ShapeRefusal, define_custom_shape_refusal};
use crate::Session;

fn with_work_path(path: Path) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 160, "height": 120, "background": "white"})).unwrap();
    s.edit("path", |doc, _| {
        doc.work_path = Some(path);
        Ok(())
    })
    .unwrap();
    s
}

fn stored(s: &Session) -> usize {
    s.edit_state.custom_shapes.len()
}

fn refused_as_empty(path: Path) {
    let mut s = with_work_path(path);
    assert_eq!(define_custom_shape_refusal(&s), Some(ShapeRefusal::Empty), "the menu shows the alert");
    let err = s.execute("edit.defineCustomShape", json!({"name": "Nothing"})).unwrap_err();
    assert_eq!(err.to_string(), EMPTY_PATH);
    assert_eq!(stored(&s), 0);
}

#[test]
fn a_single_anchor_is_refused_with_the_alert() {
    refused_as_empty(Path::new(vec![Subpath { closed: false, knots: vec![Knot::corner(40.0, 30.0)], op: PathOp::Combine }]));
}

#[test]
fn a_closed_path_of_three_points_on_a_line_is_refused() {
    refused_as_empty(Path::new(vec![Subpath::polygon(&[(10.0, 10.0), (60.0, 35.0), (110.0, 60.0)])]));
}

#[test]
fn a_straight_open_line_is_refused() {
    refused_as_empty(Path::new(vec![Subpath::polyline(&[(15.0, 90.0), (140.0, 20.0)])]));
}

#[test]
fn a_path_that_only_subtracts_does_nothing() {
    let mut sub = Subpath::polygon(&[(20.0, 20.0), (80.0, 20.0), (80.0, 70.0), (20.0, 70.0)]);
    sub.op = PathOp::Subtract;
    let mut s = with_work_path(Path::new(vec![sub]));
    assert_eq!(define_custom_shape_refusal(&s), Some(ShapeRefusal::SubtractOnly), "the menu shows nothing");
    assert!(s.execute("edit.defineCustomShape", json!({"name": "Hole"})).is_err());
    assert_eq!(stored(&s), 0);
}

#[test]
fn a_one_pixel_square_is_still_stored() {
    let mut s = with_work_path(Path::new(vec![Subpath::polygon(&[(30.0, 30.0), (31.0, 30.0), (31.0, 31.0), (30.0, 31.0)])]));
    assert_eq!(define_custom_shape_refusal(&s), None);
    s.execute("edit.defineCustomShape", json!({"name": "Dot"})).unwrap();
    let shape = &s.edit_state.custom_shapes[0];
    assert_eq!(shape.name, "Dot");
    let (x0, y0, x1, y1) = shape.path.control_bounds().unwrap();
    assert_eq!((x1 - x0, y1 - y0), (1.0, 1.0));
}

#[test]
fn a_shape_with_area_opens_the_dialog() {
    let s = with_work_path(Path::new(vec![Subpath::polygon(&[(10.0, 10.0), (90.0, 10.0), (50.0, 80.0)])]));
    assert_eq!(define_custom_shape_refusal(&s), None);
}

/// A bow tie's signed areas cancel, but its two triangles are filled: it is a shape (CodeRabbit on #16238).
#[test]
fn a_self_crossing_bow_tie_is_stored() {
    let mut s = with_work_path(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0)])]));
    assert_eq!(define_custom_shape_refusal(&s), None);
    s.execute("edit.defineCustomShape", json!({"name": "Bow"})).unwrap();
    assert_eq!(stored(&s), 1);
}

/// A closed path of two anchors keeps its old behaviour: it is stored (not measured).
#[test]
fn a_closed_two_anchor_path_keeps_being_stored() {
    let mut s = with_work_path(Path::new(vec![Subpath { closed: true, knots: vec![Knot::corner(10.0, 10.0), Knot::corner(20.0, 20.0)], op: PathOp::Combine }]));
    assert_eq!(define_custom_shape_refusal(&s), None);
    s.execute("edit.defineCustomShape", json!({"name": "Two"})).unwrap();
    assert_eq!(stored(&s), 1);
}
