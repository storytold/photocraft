//! Quick Selection's Enhance Edge (#12923): one click with the option on must leave the soft rim
//! the reference application measures, and every path next to it (the option off, Select and Mask
//! with its values, feathering, a cut lying on the document border) keeps the result it has today.
//!
//! The documents and clicks are the ones the issue measured: 800x600 RGB/8, white Background, the
//! pixel layer Shapes active with a red (220,30,30) rectangle 100,100-300,250 ("red 1"), Quick
//! Selection at its default size 30, one click inside red 1. Results are written the way the issue
//! writes them: bounds (left, top, right, bottom), fully selected pixels, and partly selected
//! pixels by 8-bit value.

use super::*;
use photocraft_raster::Surface;
use std::collections::BTreeMap;

const W: i32 = 800;
const H: i32 = 600;
/// "red 1": the rectangle the issue measured, as [left, top, right, bottom].
const RECT: [i32; 4] = [100, 100, 300, 250];
/// The issue's click, inside red 1, at the default brush size.
const CLICK: (f32, f32) = (200.0, 175.0);

/// The issue's document: white Background, then the pixel layer Shapes (active) holding "red 1".
fn shapes_document(w: i32, h: i32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    fill_rect(&mut s, RECT, "#dc1e1e");
    s
}

/// Fills `[left, top, right, bottom)` on the active pixel layer and leaves nothing selected.
fn fill_rect(s: &mut Session, rect: [i32; 4], color: &str) {
    select_rect(s, rect);
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

/// Selects the rectangle `[left, top, right, bottom)` and leaves it selected.
fn select_rect(s: &mut Session, [l, t, r, b]: [i32; 4]) {
    s.execute("select.rect", json!({"x": l, "y": t, "width": r - l, "height": b - t})).unwrap();
}

/// One Quick Selection click at the default size 30, with Enhance Edge as given.
fn quick_click(s: &mut Session, enhance: bool) -> Value {
    s.execute("select.quick", json!({"points": [[CLICK.0, CLICK.1]], "size": 30, "mode": "new", "enhanceEdge": enhance})).unwrap()
}

/// The selection as `[left, top, right, bottom]`.
fn bounds(s: &Session) -> [i32; 4] {
    let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
    let b = sel.content_bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

/// The selection's 8-bit coverage at one document pixel.
fn value(s: &Session, x: i32, y: i32) -> u8 {
    let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
    (f64::from(sel.sample_channel(x, y, 0)) * 255.0).round() as u8
}

/// `(fully selected, partly selected, value: count of the partly selected pixels)`.
fn tally(s: &Session) -> (usize, usize, BTreeMap<u8, usize>) {
    let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
    let b = sel.content_bounds();
    let (mut full, mut hist) = (0usize, BTreeMap::new());
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let v = (f64::from(sel.sample_channel(x, y, 0)) * 255.0).round() as i32;
            if v >= 255 {
                full += 1;
            } else if v > 0 {
                *hist.entry(v as u8).or_default() += 1;
            }
        }
    }
    (full, hist.values().sum(), hist)
}

/// The selection's counts, after asserting every partly selected pixel sits in the band the issue
/// measured, 248 to 254.
fn band_tally(s: &Session) -> (usize, usize) {
    let (full, partial, hist) = tally(s);
    let out_of_band: Vec<(u8, usize)> = hist.iter().filter(|(v, _)| **v < 248 || **v > 254).map(|(v, c)| (*v, *c)).collect();
    assert!(out_of_band.is_empty(), "partly selected pixels outside the measured band 248-254: {out_of_band:?}");
    (full, partial)
}

// --- the measured rim ----------------------------------------------------------------------------

/// Done when, item 1: Enhance Edge on, one click in red 1 at size 30, gives bounds
/// 100,100,300,250, about 2100-2200 partly selected pixels, all of them at values 248-254.
#[test]
fn enhance_edge_on_a_click_in_red_1_gives_about_2100_soft_rim_pixels() {
    let mut s = shapes_document(W, H);
    let out = quick_click(&mut s, true);
    assert_eq!(out["selected"], json!(true));
    assert_eq!(bounds(&s), RECT, "Enhance Edge must not move the cut");
    let (full, partial) = band_tally(&s);
    assert!((2100..=2200).contains(&partial), "the reference application measures 2146 partly selected pixels, got {partial}");
    assert_eq!(full + partial, 30000, "the cut covers exactly red 1's 200 x 150 pixels");
}

/// The rim is about three pixels wide inside the edge on all four sides: the outer three pixels of
/// the cut read 248-254, the fourth is fully selected.
#[test]
fn enhance_edge_rim_is_about_three_pixels_wide_inside_the_edge() {
    let mut s = shapes_document(W, H);
    quick_click(&mut s, true);
    let (l, t, r, b) = (RECT[0], RECT[1], RECT[2], RECT[3]);
    let (mx, my) = ((l + r) / 2, (t + b) / 2);
    let sides: [(&str, [(i32, i32); 4]); 4] = [
        ("left", [(l, my), (l + 1, my), (l + 2, my), (l + 3, my)]),
        ("right", [(r - 1, my), (r - 2, my), (r - 3, my), (r - 4, my)]),
        ("top", [(mx, t), (mx, t + 1), (mx, t + 2), (mx, t + 3)]),
        ("bottom", [(mx, b - 1), (mx, b - 2), (mx, b - 3), (mx, b - 4)]),
    ];
    for (side, along) in sides {
        for (k, (x, y)) in along.into_iter().enumerate() {
            let v = value(&s, x, y);
            if k < 3 {
                assert!((248..=254).contains(&v), "{side} edge, {k} px in at ({x},{y}): value {v}, expected the soft rim 248-254");
            } else {
                assert_eq!(v, 255, "{side} edge, 3 px in at ({x},{y}) must be fully selected");
            }
        }
    }
}

/// The rim only ever takes coverage away: nothing appears outside red 1, so the bounds stay put.
#[test]
fn enhance_edge_rim_adds_nothing_outside_the_cut() {
    let mut s = shapes_document(W, H);
    quick_click(&mut s, true);
    assert_eq!(bounds(&s), RECT);
    let (l, t, r, b) = (RECT[0], RECT[1], RECT[2], RECT[3]);
    let (mx, my) = ((l + r) / 2, (t + b) / 2);
    for x in [l - 2, l - 1, r, r + 1] {
        assert_eq!(value(&s, x, my), 0, "({x},{my}) lies outside red 1 and must stay unselected");
    }
    for y in [t - 1, t - 2, b, b + 1] {
        assert_eq!(value(&s, mx, y), 0, "({mx},{y}) lies outside red 1 and must stay unselected");
    }
}

/// The reference application's rim dips further where two edges meet (its measured 248-253 sit at
/// the corners), so the corner of the cut is never more selected than the middle of a straight edge.
#[test]
fn enhance_edge_rim_dips_at_least_as_deep_at_the_corner() {
    let mut s = shapes_document(W, H);
    quick_click(&mut s, true);
    let corner = value(&s, RECT[0], RECT[1]);
    let straight = value(&s, (RECT[0] + RECT[2]) / 2, RECT[1]);
    assert!((248..=254).contains(&corner), "the corner of the rim reads {corner}, expected 248-254");
    assert!(corner <= straight, "the corner ({corner}) must not be more selected than the straight edge ({straight})");
    assert!(straight <= 254, "the outermost rim pixel must sit below fully selected, reads {straight}");
}

/// The rule is not keyed to the measured rectangle: the same click in a bigger document gives the
/// same rim pixel for pixel (the rim is computed in bands, and the bands must not show).
#[test]
fn enhance_edge_rim_does_not_depend_on_the_canvas_size() {
    let mut small = shapes_document(W, H);
    quick_click(&mut small, true);
    let mut big = shapes_document(1600, 1200);
    quick_click(&mut big, true);
    assert_eq!(bounds(&big), RECT, "the same click in a bigger document gives the same bounds");
    assert_eq!(tally(&big), tally(&small), "the same click in a bigger document gives the same rim");
}

/// The rim is one rule, not a number tuned to the measured rectangle: on a smaller shape (60 x 40 px)
/// it gives the same three-pixel band at the same values, and the cut still covers exactly the shape.
/// Measured here: 1780 fully selected and 620 partly selected of its 2400 pixels.
#[test]
fn enhance_edge_gives_the_same_rim_on_a_smaller_shape() {
    let mut s = shapes_document(W, H);
    fill_rect(&mut s, [400, 400, 460, 440], "#1edc3c");
    s.execute("select.quick", json!({"points": [[430.0, 420.0]], "size": 30, "mode": "new", "enhanceEdge": true})).unwrap();
    assert_eq!(bounds(&s), [400, 400, 460, 440], "the cut covers exactly the smaller shape");
    let (full, partial) = band_tally(&s);
    assert_eq!((full, partial), (1780, 620));
    assert_eq!(full + partial, 2400, "all 2400 pixels of the shape are selected");
    for k in 0..3 {
        let v = value(&s, 400 + k, 420);
        assert!((248..=254).contains(&v), "the rim reaches in from the left edge: {k} px in reads {v}");
    }
    assert_eq!(value(&s, 403, 420), 255, "the interior of the smaller shape stays fully selected");
}

/// The rim works on selection coverage, which is 8-bit whatever the document's depth, so a 16-bit
/// document gets the same rim as the measured 8-bit one: the same bounds and the same pixels.
#[test]
fn enhance_edge_gives_the_same_rim_in_a_16_bit_document() {
    let mut eight = shapes_document(W, H);
    quick_click(&mut eight, true);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    fill_rect(&mut s, RECT, "#dc1e1e");
    quick_click(&mut s, true);
    assert_eq!(bounds(&s), RECT, "the cut covers exactly red 1 in a 16-bit document");
    band_tally(&s);
    assert_eq!(tally(&s), tally(&eight), "the rim works on selection coverage, which is 8-bit at any document depth");
}

// --- neighbours that must not move --------------------------------------------------------------
/// Done when, item 2: Enhance Edge off is unchanged, pinned exactly as main measures it: bounds
/// 100,100,300,250 with 29304 fully selected and 696 partly (a 191 rim and the four corners at 143).
/// The issue wrote 99,99,301,251 with 1400 partly selected; #12922 landed on main while this was in
/// progress and moved the raw footprint, so the pin holds main's current output, not that older one.
#[test]
fn enhance_edge_off_is_unchanged() {
    let mut s = shapes_document(W, H);
    quick_click(&mut s, false);
    assert_eq!(bounds(&s), [100, 100, 300, 250]);
    let (full, partial, hist) = tally(&s);
    assert_eq!(full, 29304);
    assert_eq!(partial, 696);
    assert_eq!(hist.get(&191), Some(&692), "the inner rim keeps its value 191");
    assert_eq!(hist.get(&143), Some(&4), "the four corners keep their value 143");
    assert_eq!(hist.len(), 2, "nothing else is partly selected: {hist:?}");
}

/// `refine_surface` also serves Select and Mask, so the refinement itself must keep its results.
/// Pinned with Enhance Edge's values as user parameters on the same rectangle: 29996 fully
/// selected and four pixels at 231 with Smooth 15, and 30000 fully selected with Smooth 10.
/// Guard test: passes before and after this change.
#[test]
fn select_and_mask_with_enhance_edge_values_is_unchanged() {
    let cases: [(_, usize, usize, (_, usize)); 2] = [
        (json!({"radius": 4, "smartRadius": true, "smooth": 15, "contrast": 25}), 29996, 4, (231u8, 4usize)),
        (json!({"radius": 4, "smartRadius": true, "smooth": 10, "contrast": 25}), 30000, 0, (0u8, 0usize)),
    ];
    for (params, full, partial, last) in cases {
        let mut s = shapes_document(W, H);
        select_rect(&mut s, RECT);
        s.execute("select.refineEdge", params.clone()).unwrap();
        assert_eq!(bounds(&s), RECT, "{params}");
        let (f, p, hist) = tally(&s);
        assert_eq!((f, p), (full, partial), "{params}");
        let expect: BTreeMap<u8, usize> = if last.1 == 0 { BTreeMap::new() } else { [(last.0, last.1)].into_iter().collect() };
        assert_eq!(hist, expect, "{params}");
    }
}

/// Feathering and Shift Edge, the other options that run the same refinement, keep their measured
/// soft results. Guard test: passes before and after this change.
#[test]
fn select_and_mask_feathering_and_shift_edge_are_unchanged() {
    let cases: [(_, [i32; 4], usize, usize); 3] = [
        (json!({"radius": 4, "smartRadius": true, "smooth": 15, "contrast": 25, "feather": 2}), [99, 99, 301, 251], 29304, 1396),
        (json!({"feather": 3, "smooth": 20, "contrast": 40}), [99, 99, 301, 251], 29292, 1408),
        (json!({"shiftEdge": -30, "smooth": 20}), [99, 99, 301, 251], 29304, 1388),
    ];
    for (params, want_bounds, full, partial) in cases {
        let mut s = shapes_document(W, H);
        select_rect(&mut s, RECT);
        s.execute("select.refineEdge", params.clone()).unwrap();
        assert_eq!(bounds(&s), want_bounds, "{params}");
        let (f, p, _) = tally(&s);
        assert_eq!((f, p), (full, partial), "{params}");
    }
}

/// A cut lying on the document border is not measured by the issue, so it keeps today's behaviour
/// there: pixels past the canvas repeat the canvas edge and the border stays fully selected, while
/// the edges away from the border get the same soft rim. Guard on the border, red on the rim.
#[test]
fn enhance_edge_keeps_a_cut_at_the_document_border_fully_selected_there() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Blue"})).unwrap();
    fill_rect(&mut s, [0, 0, 300, 200], "#1e3cdc");
    s.execute("select.quick", json!({"points": [[100.0, 100.0]], "size": 30, "mode": "new", "enhanceEdge": true})).unwrap();
    assert_eq!(bounds(&s), [0, 0, 300, 200]);
    // Where the cut lies on the document border it keeps its full coverage: pixels past the canvas
    // repeat the canvas edge, so there is nothing outside the cut there to dip towards.
    for y in [0, 1, 100] {
        assert_eq!(value(&s, 0, y), 255, "the left document border at y={y} stays fully selected");
    }
    for x in [0, 100, 250] {
        assert_eq!(value(&s, x, 0), 255, "the top document border at x={x} stays fully selected");
    }
    // The edges of the cut that lie away from the border get the same soft rim as red 1, and an edge
    // that is inside the document is rimmed even where it crosses the document border.
    for k in 0..3 {
        let v = value(&s, 299 - k, 100);
        assert!((248..=254).contains(&v), "the rim reaches in from the right edge: {k} px in reads {v}");
        let w = value(&s, 100, 199 - k);
        assert!((248..=254).contains(&w), "the rim reaches in from the bottom edge: {k} px in reads {w}");
    }
    assert_eq!(value(&s, 296, 100), 255, "the rim stops about three pixels in");
    assert_eq!(value(&s, 100, 196), 255, "the rim stops about three pixels in");
    assert!((248..=254).contains(&value(&s, 299, 0)), "the cut's right edge is rimmed on the top row too: {}", value(&s, 299, 0));
}
