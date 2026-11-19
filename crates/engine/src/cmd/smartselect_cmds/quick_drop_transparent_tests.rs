//! #12919: with Sample All Layers off and a pixel layer active, a Quick Selection stroke also
//! drops the layer's fully transparent pixels from the selection it leaves, in every mode. With
//! Sample All Layers on, or with the opaque Background active, the old selection keeps its
//! transparent parts. One "Quick Selection" history step per stroke.
//!
//! The document is the one the issue measured: white Background; pixel layer Shapes with red
//! (220,30,30) rectangles 100,100-300,250 ("red 1") and 500,100-600,200 ("red 2") on
//! transparency, active.

use super::*;

const W: i32 = 800;
const H: i32 = 600;
const RED1: [i32; 4] = [100, 100, 300, 250];
const RED2: [i32; 4] = [500, 100, 600, 200];

fn issue_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    for [x0, y0, x1, y1] in [RED1, RED2] {
        s.execute("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0})).unwrap();
        s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn marquee(s: &mut Session, [x0, y0, x1, y1]: [i32; 4]) {
    s.execute("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0})).unwrap();
}

/// One Quick Selection click with the default 30 px brush.
fn click(s: &mut Session, at: [f32; 2], mode: &str, sample_all: bool) -> Value {
    s.execute("select.quick", json!({"points": [at], "size": 30, "mode": mode, "sampleAllLayers": sample_all})).unwrap()
}

fn at(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn inside([x0, y0, x1, y1]: [i32; 4], x: i32, y: i32) -> bool {
    (x0..x1).contains(&x) && (y0..y1).contains(&y)
}

/// (fully selected, partly selected) pixels outside both red rectangles, where Shapes is fully
/// transparent.
fn on_transparency(s: &Session) -> (usize, usize) {
    let mut n = (0, 0);
    for y in 0..H {
        for x in 0..W {
            if inside(RED1, x, y) || inside(RED2, x, y) {
                continue;
            }
            let v = at(s, x, y);
            if v >= 0.999 {
                n.0 += 1;
            } else if v > 0.0 {
                n.1 += 1;
            }
        }
    }
    n
}

/// Fully selected pixels of `rect`.
fn full_in(s: &Session, [x0, y0, x1, y1]: [i32; 4]) -> usize {
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| at(s, *x, *y) >= 0.999).count()
}

#[test]
fn adding_red_2_drops_the_marquees_transparent_frame() {
    let mut s = issue_document();
    marquee(&mut s, [50, 50, 350, 300]);
    let before = s.active().unwrap().history.past_len();
    click(&mut s, [550.0, 150.0], "add", false);
    assert_eq!(on_transparency(&s), (0, 0), "no pixel that is transparent on Shapes stays selected");
    assert_eq!(full_in(&s, RED1), 200 * 150, "red 1 with the marquee's hard edge");
    assert!(full_in(&s, RED2) >= 9000, "red 2 from the stroke");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Quick Selection"));
}

#[test]
fn subtracting_red_1_leaves_nothing_of_the_marquee() {
    let mut s = issue_document();
    marquee(&mut s, [50, 50, 350, 300]);
    click(&mut s, [200.0, 175.0], "subtract", false);
    assert_eq!(on_transparency(&s), (0, 0), "the transparent frame is gone");
    assert_eq!(full_in(&s, RED1), 0, "and red 1 was subtracted");
}

#[test]
fn a_transparent_only_selection_minus_red_1_is_no_selection() {
    let mut s = issue_document();
    marquee(&mut s, [400, 50, 480, 300]);
    click(&mut s, [200.0, 175.0], "subtract", false);
    assert!(s.active().unwrap().doc.selection.is_none(), "no selection");
}

#[test]
fn a_new_stroke_drops_transparent_pixels_too() {
    let mut s = issue_document();
    click(&mut s, [550.0, 150.0], "new", false);
    assert_eq!(on_transparency(&s), (0, 0));
    assert!(full_in(&s, RED2) >= 9000);
}

#[test]
fn sample_all_layers_keeps_the_transparent_frame() {
    let mut s = issue_document();
    marquee(&mut s, [50, 50, 350, 300]);
    click(&mut s, [200.0, 175.0], "subtract", true);
    assert!(at(&s, 60, 60) >= 0.999 && at(&s, 340, 290) >= 0.999, "the frame stays");
}

#[test]
fn the_background_loses_only_the_brush_disc() {
    let mut s = issue_document();
    let d = s.active_mut().unwrap();
    d.active_layer = Some(d.doc.layers[0].id);
    marquee(&mut s, [50, 50, 350, 300]);
    click(&mut s, [70.0, 280.0], "subtract", false);
    assert!(at(&s, 70, 280) < 0.5, "the disc is subtracted");
    assert!(at(&s, 340, 60) >= 0.999 && at(&s, 200, 175) >= 0.999, "the rest of the marquee stays");
}

#[test]
fn a_stroke_that_finds_nothing_changes_nothing() {
    let mut s = issue_document();
    marquee(&mut s, [50, 50, 350, 300]);
    let before = s.active().unwrap().history.past_len();
    // Off the canvas: no region, so the transparent frame stays and no step is added.
    let out = s.execute("select.quick", json!({"points": [[-500.0, -500.0]], "size": 30, "mode": "add"})).unwrap();
    assert_eq!(out["changed"], json!(false));
    assert!(at(&s, 60, 60) >= 0.999);
    assert_eq!(s.active().unwrap().history.past_len(), before);
}

/// CodeRabbit on #14672: only fully transparent pixels drop out. A faint patch (alpha 0.2) beside
/// the red rectangles stays selected, and a similar-looking patch of fully transparent pixels next
/// to it does not.
#[test]
fn partly_transparent_pixels_stay_selected() {
    let mut s = issue_document();
    // A faint patch on the Shapes layer: 20 % red over 350,100-450,200.
    marquee(&mut s, [350, 100, 450, 200]);
    s.execute("edit.fill", json!({"color": "#dc1e1e", "opacity": 20})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    marquee(&mut s, [50, 50, 480, 300]);
    click(&mut s, [550.0, 150.0], "add", false);
    assert_eq!(full_in(&s, [350, 100, 450, 200]), 100 * 100, "the faint patch stays selected");
    assert_eq!(full_in(&s, [450, 100, 480, 200]), 0, "the transparent pixels beside it drop out");
}

/// CodeRabbit on #14672: the filter works on the selection's own tiles, so two pieces far apart
/// (in different tiles, with nothing selected between them) each lose only their transparent
/// pixels.
#[test]
fn two_selected_pieces_far_apart_each_drop_their_transparent_pixels() {
    let mut s = issue_document();
    marquee(&mut s, [10, 10, 130, 130]);
    s.execute("select.rect", json!({"x": 560, "y": 440, "width": 120, "height": 120, "mode": "add"})).unwrap();
    // A stroke that adds nothing new on transparency.
    click(&mut s, [550.0, 150.0], "add", false);
    assert_eq!(full_in(&s, [100, 100, 130, 130]), 30 * 30, "red 1's corner under the first piece");
    assert_eq!(full_in(&s, [10, 10, 100, 100]), 0, "the first piece's transparent part drops out");
    assert_eq!(full_in(&s, [560, 440, 680, 560]), 0, "the second piece lies on transparency and drops out");
}
