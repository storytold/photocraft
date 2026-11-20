//! An Elliptical Marquee dragged past the canvas keeps its whole shape, so moving the selection back onto the canvas
//! shows the full oval; a Rectangular Marquee still stops at the canvas edge.

use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s
}

fn selection(s: &Session) -> Option<Surface> {
    s.active().unwrap().doc.selection.clone()
}

fn bounds(s: &Session) -> Rect {
    selection(s).map_or(Rect::EMPTY, |m| m.content_bounds())
}

fn select(s: &mut Session, params: Value) {
    s.execute("select.rect", params).unwrap();
}

fn move_by(s: &mut Session, dx: i32, dy: i32) {
    s.execute("select.transformSelection", json!({"dx": dx, "dy": dy, "stepName": "Move Selection"})).unwrap();
}

/// The selection of the same ellipse drawn at `x, y`, wholly on the canvas.
fn drawn_at(x: i32, y: i32, params: Value) -> Surface {
    let mut s = session();
    let mut params = params;
    params["x"] = json!(x);
    params["y"] = json!(y);
    select(&mut s, params);
    selection(&s).unwrap()
}

fn assert_same(moved: &Surface, there: &Surface, what: &str) {
    let b = there.content_bounds();
    assert_eq!(moved.content_bounds(), b, "{what}: bounds");
    assert_eq!(moved.read_region(b), there.read_region(b), "{what}: pixel for pixel");
}

#[test]
fn an_ellipse_from_the_pasteboard_keeps_the_part_past_the_canvas() {
    let mut s = session();
    select(&mut s, json!({"x": -100, "y": 100, "width": 200, "height": 120, "ellipse": true}));
    let b = bounds(&s);
    assert!(b.x0 < 0, "the left half is kept: {b:?}");
    assert!(b.x1 > 0, "the right half is on the canvas: {b:?}");
}

#[test]
fn moving_it_onto_the_canvas_shows_the_whole_oval() {
    for params in [
        json!({"width": 200, "height": 120, "ellipse": true}),
        json!({"width": 200, "height": 120, "ellipse": true, "antiAlias": false}),
        json!({"width": 200, "height": 120, "ellipse": true, "feather": 4}),
    ] {
        let mut p = params.clone();
        (p["x"], p["y"]) = (json!(-100), json!(100));
        let mut s = session();
        select(&mut s, p);
        move_by(&mut s, 150, 0);
        assert_same(&selection(&s).unwrap(), &drawn_at(50, 100, params.clone()), &params.to_string());
    }
}

#[test]
fn every_edge_keeps_its_part() {
    // Past the right, top and bottom edges.
    for (x, y, dx, dy) in [(300, 100, -150, 0), (100, -60, 0, 100), (100, 240, 0, -100)] {
        let params = json!({"width": 200, "height": 120, "ellipse": true});
        let mut p = params.clone();
        (p["x"], p["y"]) = (json!(x), json!(y));
        let mut s = session();
        select(&mut s, p);
        move_by(&mut s, dx, dy);
        assert_same(&selection(&s).unwrap(), &drawn_at(x + dx, y + dy, params), &format!("from {x},{y}"));
    }
}

#[test]
fn adding_an_ellipse_past_the_canvas_keeps_its_part_too() {
    let mut s = session();
    select(&mut s, json!({"x": 300, "y": 10, "width": 50, "height": 50}));
    select(&mut s, json!({"x": -100, "y": 100, "width": 200, "height": 120, "ellipse": true, "mode": "add"}));
    assert!(bounds(&s).x0 < 0, "{:?}", bounds(&s));
}

#[test]
fn a_rectangle_still_stops_at_the_canvas_edge() {
    let mut s = session();
    select(&mut s, json!({"x": -100, "y": 100, "width": 200, "height": 120}));
    assert_eq!(bounds(&s), Rect::new(0, 100, 100, 220));
}

#[test]
fn an_ellipse_wholly_on_the_pasteboard_selects_nothing() {
    let mut s = session();
    select(&mut s, json!({"x": -300, "y": 100, "width": 200, "height": 120, "ellipse": true}));
    assert!(selection(&s).is_none());
}

/// The part kept past the canvas stops one canvas size past each edge, so an enormous ellipse allocates a bounded
/// number of tiles.
#[test]
fn a_huge_ellipse_is_kept_only_one_canvas_past_each_edge() {
    let mut s = session();
    select(&mut s, json!({"x": -1_000_000_000, "y": -1_000_000_000, "width": 2_000_000_000, "height": 2_000_000_000, "ellipse": true}));
    let b = bounds(&s);
    assert!(Rect::new(-400, -300, 800, 600).contains_rect(&b), "{b:?}");
    assert!(b.contains_rect(&Rect::new(0, 0, 400, 300)), "the canvas is selected: {b:?}");
}
