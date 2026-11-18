//! Patterns paint in the document's colour mode, as in Photoshop: a pattern keeps its own colours
//! (an RGB pattern stays RGB, in the library and in the document), but a pattern fill in a
//! Grayscale or CMYK document shows what Edit › Fill with that pattern paints, and converting
//! back to RGB brings the colours back (the follow-up to #1907; #1905 is about solid and
//! gradient fills).

use crate::Session;
use photocraft_doc::ColorMode;
use serde_json::json;

/// Defines a blue-and-yellow RGB pattern from a scratch document and returns its id.
fn colour_pattern(s: &mut Session) -> String {
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "rgb", "depth": 8, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#2040e0"})).unwrap();
    s.execute("select.inverse", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#e0d020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "Blue Yellow"})).unwrap();
    r["pattern"].as_str().unwrap().to_string()
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
}

fn neutral(p: [f32; 4]) -> bool {
    p[0] == p[1] && p[1] == p[2]
}

const XS: [i32; 4] = [1, 5, 9, 14];

/// The composite where `pat` is painted with Edit › Fill on a pixel layer of a new document.
fn painted(s: &mut Session, mode: &str, depth: u32, pat: &str) -> Vec<[f32; 4]> {
    s.execute("file.new", json!({"width": 32, "height": 16, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": pat})).unwrap();
    XS.iter().map(|x| pixel(s, *x, 3)).collect()
}

fn pattern_mode(s: &Session) -> ColorMode {
    s.active().unwrap().doc.patterns[0].surface.format().mode
}

#[test]
fn pattern_fills_paint_like_edit_fill_in_every_mode() {
    for (mode, cm) in [("gray", ColorMode::Grayscale), ("cmyk", ColorMode::Cmyk), ("lab", ColorMode::Lab), ("rgb", ColorMode::Rgb)] {
        for depth in [8, 16, 32] {
            let mut s = crate::session();
            let pat = colour_pattern(&mut s);
            let want = painted(&mut s, mode, depth, &pat);
            s.execute("file.new", json!({"width": 32, "height": 16, "mode": mode, "depth": depth, "background": "white"})).unwrap();
            s.execute("layer.newFillLayer.pattern", json!({"pattern": pat})).unwrap();
            assert_eq!(pattern_mode(&s), ColorMode::Rgb, "{mode}/{depth}: the pattern keeps its own colours");
            for (x, w) in XS.iter().zip(&want) {
                let got = pixel(&mut s, *x, 3);
                // Lab: Edit › Fill quantizes Lab at 8/16 bits; the fill layer draws the RGB pattern.
                let tol = if cm == ColorMode::Lab { 0.02 } else { 0.01 };
                assert!(close(got, *w, tol), "{mode}/{depth} x {x}: pattern fill {got:?}, Edit › Fill {w:?}");
                if cm == ColorMode::Grayscale {
                    assert!(neutral(got), "{mode}/{depth} x {x}: {got:?}");
                }
            }
        }
    }
}

#[test]
fn a_grayscale_round_trip_keeps_the_pattern_colours() {
    for depth in [8, 16] {
        let mut s = crate::session();
        let pat = colour_pattern(&mut s);
        s.execute("file.new", json!({"width": 32, "height": 16, "mode": "rgb", "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": pat})).unwrap();
        let rgb: Vec<[f32; 4]> = XS.iter().map(|x| pixel(&mut s, *x, 3)).collect();
        assert!(rgb[0][2] > 0.8 && rgb[0][0] < 0.2, "blue in RGB: {:?}", rgb[0]);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        for x in XS {
            let p = pixel(&mut s, x, 3);
            assert!(neutral(p), "depth {depth} x {x}: grey in Grayscale, {p:?}");
        }
        assert_eq!(pattern_mode(&s), ColorMode::Rgb, "the document's pattern keeps its colours");
        s.execute("image.mode.rgb", json!({})).unwrap();
        for (x, want) in XS.iter().zip(&rgb) {
            let p = pixel(&mut s, *x, 3);
            assert!(close(p, *want, 1e-6), "depth {depth} x {x}: colour back in RGB, {p:?} vs {want:?}");
        }
    }
}

/// #14975: the issue's 20 x 20 test pattern (red, green, blue and black quadrants), defined from an
/// RGB document; returns its id.
fn quadrant_pattern(s: &mut Session) -> String {
    s.execute("file.new", json!({"width": 20, "height": 20, "mode": "rgb", "depth": 8, "background": "white"})).unwrap();
    for (x, y, c) in [(0, 0, "#ff0000"), (10, 0, "#00ff00"), (0, 10, "#0000ff"), (10, 10, "#000000")] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": c})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "Quadrants"})).unwrap();
    r["pattern"].as_str().unwrap().to_string()
}

/// The greys (of 255) the reference application paints for the red, green and blue cells in a
/// Grayscale document (#14975), where each cell sits on a 20 x 20 canvas-origin tiling.
const QUADRANT_GREYS: [(&str, (i32, i32), f32); 3] = [("red", (5, 5), 129.0), ("green", (15, 5), 221.0), ("blue", (5, 15), 69.0)];

/// The cells that are off by more than 2 levels, or not neutral.
fn wrong_greys(s: &mut Session, what: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    for (cell, (x, y), want) in QUADRANT_GREYS {
        let p = pixel(s, x, y);
        if !neutral(p) || (p[0] * 255.0 - want).abs() > 2.0 {
            wrong.push(format!("{what} {cell}: {p:?} (grey {:.1}), want {want}", p[0] * 255.0));
        }
    }
    wrong
}

#[test]
fn edit_fill_and_the_paint_bucket_paint_a_pattern_through_the_grey_profile() {
    let mut wrong = Vec::new();
    for depth in [8, 16] {
        let mut s = Session::new();
        let pat = quadrant_pattern(&mut s);
        // Edit › Fill with the pattern on a new layer.
        s.execute("file.new", json!({"width": 40, "height": 40, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "pattern", "pattern": pat})).unwrap();
        wrong.extend(wrong_greys(&mut s, &format!("Edit › Fill {depth}-bit")));
        // The Paint Bucket in pattern mode on the white Background.
        s.execute("file.new", json!({"width": 40, "height": 40, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
        s.execute("paint.bucket", json!({"x": 30, "y": 30, "contents": "pattern", "pattern": pat, "tolerance": 0, "antiAlias": false})).unwrap();
        wrong.extend(wrong_greys(&mut s, &format!("Paint Bucket {depth}-bit")));
        // A pattern fill layer paints the same greys.
        s.execute("file.new", json!({"width": 40, "height": 40, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": pat})).unwrap();
        wrong.extend(wrong_greys(&mut s, &format!("pattern fill layer {depth}-bit")));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn edit_fill_with_a_pattern_keeps_its_colours_outside_grayscale() {
    // RGB keeps the pattern's colours exactly; CMYK and Lab keep today's conversion (checked
    // against a pattern fill layer by `pattern_fills_paint_like_edit_fill_in_every_mode`).
    let mut s = Session::new();
    let pat = quadrant_pattern(&mut s);
    s.execute("file.new", json!({"width": 40, "height": 40, "mode": "rgb", "depth": 8, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": pat})).unwrap();
    for ((x, y), want) in [((5, 5), [1.0, 0.0, 0.0, 1.0]), ((15, 5), [0.0, 1.0, 0.0, 1.0]), ((5, 15), [0.0, 0.0, 1.0, 1.0])] {
        assert!(close(pixel(&mut s, x, y), want, 0.004), "RGB at {x},{y}: {:?}", pixel(&mut s, x, y));
    }
}
