//! #12884: with Impressionist on, each Pattern Stamp dab is one flat colour, the pattern pixel
//! (as the stroke places it) one pixel up and left of the dab's centre, with no random jitter.
//! The issue's test document: 800 x 600, a 20 x 20 pattern of 10 px red, green, blue and white
//! quadrants, a 40 px hard tip, spacing 25 %, Aligned on.

use super::*;
use photocraft_doc::Pattern;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];
/// The Background's grey, so a white dab shows.
const GREY: f32 = 128.0 / 255.0;

/// The document with a grey Background and the quadrant pattern; its id.
fn session(depth: u64) -> (Session, String) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth})).unwrap();
    let fmt = s.active().unwrap().doc.pixel_format();
    let mut tile = Surface::new(fmt);
    for (rect, color) in
        [(Rect::new(0, 0, 10, 10), RED), (Rect::new(10, 0, 20, 10), GREEN), (Rect::new(0, 10, 10, 20), BLUE), (Rect::new(10, 10, 20, 20), WHITE)]
    {
        tile.fill_rect(rect, &color);
    }
    let pat = Pattern::new("Quadrants", tile, 20, 20);
    let id = pat.id.clone();
    s.edit("setup", |doc, layer| {
        doc.patterns.push(pat);
        let bounds = doc.bounds();
        doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[GREY, GREY, GREY, 1.0]);
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn params(id: &str, points: &[[f64; 2]], extra: Value) -> Value {
    let mut p = json!({"pattern": id, "points": points, "size": 40, "hardness": 100, "spacing": 25, "aligned": true, "impressionist": true});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn stamp(s: &mut Session, id: &str, points: &[[f64; 2]], extra: Value) {
    s.execute("paint.patternStamp", params(id, points, extra)).unwrap();
}

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// Every pixel of row `y` over `x0..=x1` is `want`.
fn row(s: &Session, y: i32, x0: i32, x1: i32, want: [f32; 4], what: &str) {
    for x in x0..=x1 {
        assert_eq!(px(s, x, y), want, "{what}: ({x}, {y})");
    }
}

#[test]
fn a_click_at_600_450_is_a_solid_green_disc() {
    for depth in [8, 16, 32] {
        let (mut s, id) = session(depth);
        stamp(&mut s, &id, &[[600.0, 450.0]], json!({}));
        // The measured green spans less the antialiased rim pixel at each end.
        for (y, x0, x1) in [(440, 584, 614), (450, 581, 617), (460, 585, 613)] {
            row(&s, y, x0, x1, GREEN, &format!("{depth}-bit"));
        }
        assert_eq!(px(&s, 570, 450), [GREY, GREY, GREY, 1.0], "{depth}-bit: outside the disc");
    }
}

#[test]
fn a_centre_at_11_11_gives_white_and_a_centre_on_a_quadrant_edge_one_pure_colour() {
    // (cx, cy) → the pattern pixel ((cx - 1) mod 20, (cy - 1) mod 20).
    for (centre, want, name) in [
        ([511.0, 511.0], WHITE, "(11,11): white"),
        ([515.0, 511.0], WHITE, "(15,11): white"),
        ([511.0, 515.0], WHITE, "(11,15): white"),
        ([510.0, 505.0], RED, "x mod 20 = 10: the left column"),
        ([505.0, 500.0], BLUE, "y mod 20 = 0: the bottom row"),
    ] {
        let (mut s, id) = session(8);
        stamp(&mut s, &id, &[centre], json!({}));
        let (cx, cy) = (centre[0] as i32, centre[1] as i32);
        for (dx, dy) in [(0, 0), (-12, 0), (12, 0), (0, -12), (0, 12), (-8, -8), (8, 8)] {
            assert_eq!(px(&s, cx + dx, cy + dy), want, "{name}: ({}, {})", cx + dx, cy + dy);
        }
    }
}

#[test]
fn a_drag_lays_overlapping_solid_red_and_green_discs() {
    let (mut s, id) = session(8);
    stamp(&mut s, &id, &[[450.0, 150.0], [750.0, 150.0]], json!({}));
    let (mut red, mut green) = (0, 0);
    // Where the dabs overlap fully (the stroke's last dabs thin out towards its end).
    for x in 455..=705 {
        let c = px(&s, x, 150);
        // Only red and green, or a newer dab's antialiased rim over the previous one: never the
        // pattern's blue or white, never a piece of the pattern.
        assert!(c[2] == 0.0 && (c[0] + c[1] - 1.0).abs() < 0.01 && c[3] == 1.0, "({x}, 150) is {c:?}");
        red += usize::from(c == RED);
        green += usize::from(c == GREEN);
    }
    assert!(red > 60 && green > 60, "solid red and green discs: {red} red, {green} green of 251");
}

#[test]
fn the_same_stroke_twice_gives_the_same_pixels_whatever_the_seed() {
    let pts = [[450.0, 150.0], [600.0, 200.0], [750.0, 150.0]];
    let run = |seed: u64| {
        let (mut s, id) = session(8);
        stamp(&mut s, &id, &pts, json!({"seed": seed}));
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(400, 100, 800, 250))
    };
    assert_eq!(run(1), run(1));
    assert_eq!(run(1), run(99), "no random jitter");
}

#[test]
fn mode_opacity_and_the_selection_still_apply() {
    // Opacity 50 %: half green over the grey.
    let (mut s, id) = session(8);
    stamp(&mut s, &id, &[[600.0, 450.0]], json!({"opacity": 50}));
    let c = px(&s, 600, 450);
    assert!((c[0] - GREY * 0.5).abs() < 0.01 && (c[1] - (0.5 + GREY * 0.5)).abs() < 0.01, "opacity 50 %: {c:?}");
    // Multiply: green × grey.
    let (mut s, id) = session(8);
    stamp(&mut s, &id, &[[600.0, 450.0]], json!({"mode": "multiply"}));
    let c = px(&s, 600, 450);
    assert!(c[0] < 0.01 && (c[1] - GREY).abs() < 0.01 && c[2] < 0.01, "multiply: {c:?}");
    // A selection of the disc's left half: the right half stays grey.
    let (mut s, id) = session(8);
    s.execute("select.rect", json!({"x": 560, "y": 400, "width": 40, "height": 100})).unwrap();
    stamp(&mut s, &id, &[[600.0, 450.0]], json!({}));
    assert_eq!(px(&s, 590, 450), GREEN, "inside the selection");
    assert_eq!(px(&s, 610, 450), [GREY, GREY, GREY, 1.0], "outside the selection");
}

/// At 200 % the point one pixel up and left of the dab's centre maps to tile (9.75, 4.75): the single tile pixel there
/// is red, where a bilinear lookup would mix red and green (CodeRabbit on #16226).
#[test]
fn a_scaled_pattern_still_gives_one_pure_pattern_pixel() {
    let (mut s, id) = session(8);
    stamp(&mut s, &id, &[[20.0, 10.0]], json!({"scale": 200}));
    assert_eq!(px(&s, 20, 10), RED);
}
