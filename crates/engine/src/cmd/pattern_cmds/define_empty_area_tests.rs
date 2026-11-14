//! #14299: Define Pattern refuses an area with no visible pixel, as the reference application
//! does, instead of adding a blank pattern.

use serde_json::json;

use super::*;

fn transparent_doc(width: u32, height: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    s
}

/// Runs `edit.definePattern` and checks it failed with the empty-area error, adding nothing.
fn assert_refused(s: &mut Session, params: Value, case: &str) {
    let before = s.patterns.items.len();
    let err = s.execute("edit.definePattern", params).unwrap_err();
    assert_eq!(err.to_string(), EMPTY_AREA, "{case}");
    assert_eq!(s.patterns.items.len(), before, "{case}: the library is unchanged");
}

#[test]
fn a_fully_transparent_area_is_refused() {
    let mut s = transparent_doc(40, 30);
    assert_refused(&mut s, json!({}), "whole transparent document, no selection");
    s.execute("select.rect", json!({"x": 2, "y": 3, "width": 5, "height": 5})).unwrap();
    assert_refused(&mut s, json!({"name": "Nothing"}), "selection over nothing");
}

#[test]
fn hidden_and_zero_opacity_pixels_count_as_they_look() {
    let mut s = transparent_doc(40, 30);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.0})).unwrap();
    assert_refused(&mut s, json!({}), "layer at 0% opacity");
    s.execute("layer.setProps", json!({"opacity": 1.0, "visible": false})).unwrap();
    assert_refused(&mut s, json!({}), "hidden layer");
}

#[test]
fn a_partly_transparent_area_defines_and_keeps_its_transparency() {
    let mut s = transparent_doc(40, 30);
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "Red square"})).unwrap();
    let pat = resolve(&s, r["pattern"].as_str().unwrap()).unwrap();
    assert_eq!((pat.width, pat.height), (40, 30));
    assert!(pat.surface.format().alpha, "keeps its alpha channel");
    let px = |x: i32, y: i32| pat.surface.read_region(Rect::new(x, y, x + 1, y + 1));
    assert_eq!(px(0, 0)[3], 0.0, "transparent at 0,0");
    assert_eq!(px(10, 10), vec![1.0, 0.0, 0.0, 1.0], "red at 10,10");
}

#[test]
fn white_is_not_empty() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1})).unwrap();
    s.execute("edit.definePattern", json!({"name": "White dot"})).unwrap();
    assert!(resolve(&s, "White dot").is_some());
}
