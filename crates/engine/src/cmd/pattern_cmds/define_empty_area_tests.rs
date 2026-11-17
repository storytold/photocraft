use serde_json::json;

use super::*;

#[test]
fn transparent_document_without_selection_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8, "background": "transparent"})).unwrap();
    let before = s.patterns.items.len();
    let err = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
    assert!(err.contains(EMPTY_AREA_ERROR), "{err}");
    assert_eq!(s.patterns.items.len(), before);
}

#[test]
fn transparent_selection_over_nothing_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 5, "height": 5})).unwrap();
    let before = s.patterns.items.len();
    let err = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
    assert!(err.contains(EMPTY_AREA_ERROR), "{err}");
    assert_eq!(s.patterns.items.len(), before);
}

#[test]
fn hidden_or_zero_opacity_layer_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert!(s.execute("edit.definePattern", json!({})).unwrap_err().to_string().contains(EMPTY_AREA_ERROR));

    s.execute("layer.setProps", json!({"visible": true, "opacity": 0.0})).unwrap();
    assert!(s.execute("edit.definePattern", json!({})).unwrap_err().to_string().contains(EMPTY_AREA_ERROR));
}

#[test]
fn one_by_one_white_document_still_defines() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1, "depth": 8, "background": "white"})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "WhitePx"})).unwrap();
    assert_eq!(r["info"]["width"], 1);
    assert_eq!(r["info"]["height"], 1);
}

#[test]
fn partly_transparent_area_defines_and_keeps_transparency() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "RedSquare"})).unwrap();
    let id = r["pattern"].as_str().unwrap();
    let pat = s.patterns.items.iter().find(|p| p.id == id).unwrap();
    assert_eq!((pat.width, pat.height), (40, 30));
    let corner = pat.surface.read_region(Rect::new(0, 0, 1, 1));
    assert_eq!(corner[3], 0.0, "outside red rect stays transparent");
    let inside = pat.surface.read_region(Rect::new(10, 10, 11, 11));
    assert!(inside[0] > 0.9 && inside[3] > 0.9, "inside red rect is red and opaque: {inside:?}");
}
