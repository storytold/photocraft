//! #11777: the Magic Wand's Sample Size. With an N by N Average the click compares against the
//! average of the square around the clicked pixel instead of that one pixel; transparent pixels
//! in the square don't pull the average; a size that isn't one of the seven is refused.

use serde_json::json;

use crate::Session;

/// 800 × 600, white, with a grey ramp of ten 40 px blocks from x 100 (100, 105, …, 145) at
/// y 350–450.
fn ramp() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    for k in 0..10 {
        let v = 100 + 5 * k;
        s.execute("select.rect", json!({"x": 100 + 40 * k, "y": 350, "width": 40, "height": 100})).unwrap();
        s.execute("edit.fill", json!({"color": format!("#{v:02x}{v:02x}{v:02x}")})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The columns of row `y` that are at least half selected: (first, end), or `None`. Every
/// column in between must be selected too.
fn selected_row(s: &Session, y: i32) -> Option<(i32, i32)> {
    let doc = &s.active().unwrap().doc;
    let row = photocraft_geom::Rect::new(0, y, 800, y + 1);
    let m = photocraft_algo::selection::mask_from_surface(doc.selection.as_ref(), row);
    let first = m.iter().position(|v| *v >= 0.5)?;
    let last = m.iter().rposition(|v| *v >= 0.5)?;
    assert!(m[first..=last].iter().all(|v| *v >= 0.5), "a hole in the selected run {first}..={last}");
    Some((first as i32, last as i32 + 1))
}

/// The rows of column `x` that are at least half selected: (first, end), or `None`.
fn selected_column(s: &Session, x: i32) -> Option<(i32, i32)> {
    let doc = &s.active().unwrap().doc;
    let col = photocraft_geom::Rect::new(x, 0, x + 1, 600);
    let m = photocraft_algo::selection::mask_from_surface(doc.selection.as_ref(), col);
    let first = m.iter().position(|v| *v >= 0.5)?;
    let last = m.iter().rposition(|v| *v >= 0.5)?;
    Some((first as i32, last as i32 + 1))
}

#[test]
fn the_average_of_the_square_is_the_reference_colour() {
    // Pixel 139 is the last column of the 100 block, next to the 105 block. Tolerance 3, without
    // the Anti-alias fringe, so the run is exactly the blocks taken.
    for (size, want) in [(1, (100, 140)), (3, (100, 180)), (11, (100, 180))] {
        let mut s = ramp();
        s.execute("select.magicWand", json!({"x": 139, "y": 400, "tolerance": 3, "sampleSize": size, "antiAlias": false})).unwrap();
        assert_eq!(selected_row(&s, 400), Some(want), "sample size {size}");
    }
    // Point Sample is the default.
    let mut s = ramp();
    s.execute("select.magicWand", json!({"x": 139, "y": 400, "tolerance": 3, "antiAlias": false})).unwrap();
    assert_eq!(selected_row(&s, 400), Some((100, 140)));
}

#[test]
fn transparent_pixels_do_not_pull_the_average() {
    // A red rectangle on a layer that is transparent around it; Sample All Layers off.
    for size in [1, 3, 5, 11, 31, 51, 101] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
        s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.magicWand", json!({"x": 102, "y": 170, "tolerance": 32, "sampleSize": size, "sampleAllLayers": false})).unwrap();
        assert_eq!(selected_row(&s, 170), Some((100, 300)), "sample size {size}");
        assert_eq!(selected_column(&s, 102), Some((100, 250)), "sample size {size}");
    }
}

#[test]
fn a_size_that_is_not_one_of_the_seven_is_refused() {
    let mut s = ramp();
    for bad in [json!(0), json!(2), json!(4), json!(102), json!(-3), json!(3.5), json!("big"), json!(u64::MAX)] {
        let r = s.execute("select.magicWand", json!({"x": 139, "y": 400, "tolerance": 3, "sampleSize": bad}));
        assert!(r.is_err(), "{bad} is refused");
    }
    assert!(s.active().unwrap().doc.selection.is_none(), "a refused click selects nothing");
}

#[test]
fn a_square_at_the_canvas_edge_is_clipped() {
    // The 101 by 101 square around (0, 0) is clipped to the canvas, which is white there: the
    // white background is selected and the ramp is not.
    let mut s = ramp();
    s.execute("select.magicWand", json!({"x": 0, "y": 0, "tolerance": 3, "sampleSize": 101})).unwrap();
    let doc = &s.active().unwrap().doc;
    let at = |x: i32, y: i32| doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0));
    assert!(at(0, 0) >= 0.5 && at(799, 599) >= 0.5 && at(50, 400) >= 0.5, "the white background");
    assert!(at(139, 400) < 0.5 && at(300, 400) < 0.5, "not the ramp");
}
