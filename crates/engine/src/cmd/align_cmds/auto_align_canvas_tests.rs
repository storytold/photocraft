//! Where Auto-Align puts its result (#14195): the box of the aligned layers moves to 0,0 and the
//! canvas grows to it; a selected Background becomes "Layer 0"; unselected layers move with the
//! canvas origin and are clipped to it; an unselected Background grows with white; a fully
//! locked reference stays put and the canvas grows right and down only. One history step.

use super::*;

/// A deterministic blocky texture over the whole plane: 10 px cells of hashed colours.
fn tex(x: i32, y: i32) -> [f32; 4] {
    let (cx, cy) = (x.div_euclid(10) as u64, y.div_euclid(10) as u64);
    let mut h = cx.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ cy.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    let v = |s: u32| ((h >> s) & 0xff) as f32 / 255.0;
    [v(0), v(8), v(16), 1.0]
}

/// Writes `f(x, y)` over `r` into layer `id`.
fn paint(s: &mut Session, id: LayerId, r: Rect, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("tex", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut px = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                px.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(r, &px);
        Ok(())
    })
    .unwrap();
}

fn new_layer(s: &mut Session, name: &str) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.active().unwrap().active_layer.unwrap()
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

fn canvas(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn near(r: Rect, want: (i32, i32, i32, i32), tol: i32) -> bool {
    (r.x0 - want.0).abs() <= tol && (r.y0 - want.1).abs() <= tol && (r.x1 - want.2).abs() <= tol && (r.y1 - want.3).abs() <= tol
}

/// The triage document: a textured 400x300 Background and "Shift", a copy moved +24,+16 that is
/// wider than the canvas (-40,-32,440,336), plus an unselected "Top" at -22,-9,402,325.
fn wide_shift_doc(depth: u32) -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    paint(&mut s, bg, Rect::new(0, 0, 400, 300), tex);
    let shift = new_layer(&mut s, "Shift");
    paint(&mut s, shift, Rect::new(-40, -32, 440, 336), |x, y| tex(x - 24, y - 16));
    let top = new_layer(&mut s, "Top");
    paint(&mut s, top, Rect::new(-22, -9, 402, 325), |_, _| [0.2, 0.6, 0.9, 1.0]);
    select(&mut s, &[bg, shift]);
    (s, bg, shift, top)
}

#[test]
fn the_canvas_grows_to_the_aligned_box_which_starts_at_the_origin() {
    for projection in ["reposition", "auto"] {
        let (mut s, bg, shift, top) = wide_shift_doc(8);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": projection})).unwrap();
        assert_eq!(r["reference"], bg.0, "{projection}: {r}");
        // Reposition moves by whole pixels; the perspective warp is a hair off -24,-16.
        let tol = if projection == "reposition" { 0 } else { 2 };
        let (w, h) = canvas(&s);
        assert!((w as i32 - 480).abs() <= tol && (h as i32 - 368).abs() <= tol, "{projection}: the canvas grows to the box: {w}x{h} {r}");
        // The box starts at 0,0; a bicubic fringe of a pixel may hang past the canvas.
        assert!(near(bounds(&s, shift), (0, 0, w as i32, h as i32), tol.min(1)), "{projection}: {:?}", bounds(&s, shift));
        assert!(near(bounds(&s, bg), (64, 48, 464, 348), tol), "{projection}: {:?}", bounds(&s, bg));
        let doc = &s.active().unwrap().doc;
        assert!(!doc.is_background(bg), "{projection}: a selected Background becomes a normal layer");
        assert_eq!(doc.layer(bg).unwrap().name, "Layer 0", "{projection}");
        // The unselected layer moves with the canvas origin and is clipped to the canvas.
        assert!(near(bounds(&s, top), (42, 39, 466, 368), tol), "{projection}: {:?}", bounds(&s, top));
    }
}

#[test]
fn undo_restores_the_canvas_the_layers_and_the_background_in_one_step() {
    let (mut s, bg, shift, top) = wide_shift_doc(8);
    let steps = s.active().unwrap().history.entries().len();
    s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one history step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Auto-Align Layers"));
    s.undo();
    assert_eq!(canvas(&s), (400, 300));
    let doc = &s.active().unwrap().doc;
    assert!(doc.is_background(bg));
    assert_eq!(doc.layer(bg).unwrap().name, "Background");
    assert_eq!(bounds(&s, shift), Rect::new(-40, -32, 440, 336));
    assert_eq!(bounds(&s, top), Rect::new(-22, -9, 402, 325));
    assert_eq!(bounds(&s, bg), Rect::new(0, 0, 400, 300));
}

#[test]
fn the_placement_holds_at_16_and_32_bit() {
    for depth in [16, 32] {
        let (mut s, bg, shift, _) = wide_shift_doc(depth);
        s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
        assert_eq!(canvas(&s), (480, 368), "depth {depth}");
        assert!(near(bounds(&s, shift), (0, 0, 480, 368), 1), "depth {depth}: {:?}", bounds(&s, shift));
        assert!(near(bounds(&s, bg), (64, 48, 464, 348), 1), "depth {depth}: {:?}", bounds(&s, bg));
        assert_eq!(s.active().unwrap().doc.layer(bg).unwrap().name, "Layer 0", "depth {depth}");
    }
}

/// The issue's document: Background, "Shift" (a copy moved +24,+16) and "Rot" (a copy rotated 5
/// degrees about its centre, then moved -10,+8), all selected. Auto only: Reposition here finds no
/// translation for the rotated copy and leaves it unaligned (alignment quality, not placement).
#[test]
fn shift_and_rot_end_in_a_box_at_the_origin_that_the_canvas_takes() {
    let a = 5f64.to_radians();
    let (sin, cos) = a.sin_cos();
    for projection in ["auto", "perspective", "collage"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        let bg = s.active().unwrap().doc.background().unwrap().id;
        paint(&mut s, bg, Rect::new(0, 0, 400, 300), tex);
        let shift = new_layer(&mut s, "Shift");
        paint(&mut s, shift, Rect::new(24, 16, 424, 316), |x, y| tex(x - 24, y - 16));
        let rot = new_layer(&mut s, "Rot");
        // Inverse map: undo the move, then rotate back about the centre (200, 150).
        paint(&mut s, rot, Rect::new(-23, -9, 403, 325), |x, y| {
            let (u, v) = (f64::from(x) + 10.0 - 200.0, f64::from(y) - 8.0 - 150.0);
            let (sx, sy) = (cos * u + sin * v + 200.0, -sin * u + cos * v + 150.0);
            if (0.0..400.0).contains(&sx) && (0.0..300.0).contains(&sy) { tex(sx as i32, sy as i32) } else { [0.0; 4] }
        });
        select(&mut s, &[bg, shift, rot]);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": projection})).unwrap();
        let union = [bg, shift, rot].iter().fold(Rect::EMPTY, |u, id| u.union(&bounds(&s, *id)));
        let (w, h) = canvas(&s);
        assert_eq!((union.x0, union.y0), (0, 0), "{projection}: the box starts at 0,0: {r}");
        assert_eq!((union.x1, union.y1), (w as i32, h as i32), "{projection}: the canvas is the box: {r}");
        assert!((400..=410).contains(&w) && (300..=310).contains(&h), "{projection}: about 402x302: {w}x{h} {r}");
        assert_eq!(s.active().unwrap().doc.layer(bg).unwrap().name, "Layer 0", "{projection}");
    }
}

/// Two 200x150 patches, the Background not selected: both end at 0,0,200,150, the canvas stays
/// 400x300 and the Background stays where and what it is.
#[test]
fn two_patches_move_to_the_origin_and_the_canvas_keeps_its_size() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    let a = new_layer(&mut s, "A");
    paint(&mut s, a, Rect::new(100, 75, 300, 225), |x, y| tex(x - 100, y - 75));
    let b = new_layer(&mut s, "B");
    paint(&mut s, b, Rect::new(130, 100, 330, 250), |x, y| tex(x - 130, y - 100));
    select(&mut s, &[a, b]);
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(canvas(&s), (400, 300), "{r}");
    assert!(near(bounds(&s, a), (0, 0, 200, 150), 0), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (0, 0, 200, 150), 1), "{:?}", bounds(&s, b));
    let doc = &s.active().unwrap().doc;
    assert!(doc.is_background(bg));
    assert_eq!(bounds(&s, bg), Rect::new(0, 0, 400, 300));
}

/// A fully locked reference stays put; the canvas grows right and down only, and the unselected
/// Background grows with white whatever its own colour.
#[test]
fn a_locked_reference_stays_and_the_background_grows_with_white() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    paint(&mut s, bg, Rect::new(0, 0, 400, 300), |_, _| [0.5, 0.25, 0.1, 1.0]);
    let shift = new_layer(&mut s, "Shift");
    paint(&mut s, shift, Rect::new(24, 16, 424, 316), |x, y| tex(x - 24, y - 16));
    s.edit("lock", |doc, _| {
        doc.layer_mut(shift).unwrap().locks.set("all", true);
        Ok(())
    })
    .unwrap();
    let other = new_layer(&mut s, "Other");
    paint(&mut s, other, Rect::new(30, 20, 430, 320), |x, y| tex(x - 30, y - 20));
    select(&mut s, &[shift, other]);
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(r["reference"], shift.0, "{r}");
    assert_eq!(bounds(&s, shift), Rect::new(24, 16, 424, 316), "the locked reference stays put");
    assert!(near(bounds(&s, other), (24, 16, 424, 316), 1), "{:?}", bounds(&s, other));
    let (w, h) = canvas(&s);
    assert!((424..=425).contains(&w) && (316..=317).contains(&h), "grows right and down: {w}x{h}");
    let doc = &s.active().unwrap().doc;
    assert!(doc.is_background(bg));
    let surf = doc.layer(bg).unwrap().surface().unwrap();
    assert_eq!(surf.content_bounds(), Rect::new(0, 0, w as i32, h as i32), "the Background fills the canvas");
    let px = |x, y| photocraft_raster::to_rgba(&surf.format(), &surf.read_region(Rect::new(x, y, x + 1, y + 1)));
    assert_eq!(px(w as i32 - 1, 5), [1.0; 4], "new border is white");
    assert_eq!(px(5, h as i32 - 1), [1.0; 4], "new border is white");
    let old = px(5, 5);
    assert!((old[0] - 0.5).abs() < 0.01 && (old[1] - 0.25).abs() < 0.01, "the old pixels stay: {old:?}");
}

/// Layers with nothing to match: an error, and the document, canvas and history are untouched.
#[test]
fn degenerate_layers_fail_gracefully_and_change_nothing() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    let a = new_layer(&mut s, "A");
    let b = new_layer(&mut s, "B");
    select(&mut s, &[a, b]);
    let steps = s.active().unwrap().history.entries().len();
    for projection in ["auto", "reposition", "spherical"] {
        assert!(s.execute("edit.autoAlignLayers", json!({"projection": projection})).is_err(), "{projection}");
        assert_eq!(canvas(&s), (400, 300));
        assert_eq!(s.active().unwrap().history.entries().len(), steps);
    }
}

/// The box placement itself: empty bounds keep everything, and a box past the largest canvas is
/// refused rather than allocated.
#[test]
fn placement_guards_empty_and_huge_boxes() {
    let canvas = Rect::new(0, 0, 400, 300);
    assert!(matches!(placement(canvas, Rect::EMPTY, false), Ok(None)));
    assert!(placement(canvas, Rect::new(-10, 0, 300_000, 10), false).is_err());
    assert!(placement(canvas, Rect::new(i32::MIN, 0, i32::MAX, 10), false).is_err());
    assert!(placement(canvas, Rect::new(0, 0, 400, 300), false).unwrap().is_none(), "nothing to change");
    let p = placement(canvas, Rect::new(-1, -1, 401, 301), false).unwrap().unwrap();
    assert_eq!((p.shift, p.origin, p.size), ((1, 1), (1, 1), (402, 302)));
    let p = placement(canvas, Rect::new(100, 75, 300, 225), false).unwrap().unwrap();
    assert_eq!((p.shift, p.origin, p.size), ((-100, -75), (0, 0), (400, 300)));
    let p = placement(canvas, Rect::new(23, 15, 425, 317), true).unwrap().unwrap();
    assert_eq!((p.shift, p.origin, p.size), ((0, 0), (0, 0), (425, 317)));
}

/// The corner box that trims a planar warp's resampling fringe: exact for whole-pixel moves,
/// within 0.05 px of them, and nothing past a horizon or for an empty layer.
#[test]
fn mapped_bounds_trims_the_fringe_and_refuses_bad_corners() {
    let r = Rect::new(10, 20, 110, 70);
    assert_eq!(mapped_bounds(r, |x, y| (x - 24.0, y - 16.0)), Some(Rect::new(-14, 4, 86, 54)));
    assert_eq!(mapped_bounds(r, |x, y| (x + 0.01, y - 0.02)), Some(r));
    assert_eq!(mapped_bounds(r, |x, y| (x + 0.5, y)), Some(Rect::new(10, 20, 111, 70)));
    assert_eq!(mapped_bounds(r, |_, _| (f64::NAN, 0.0)), None);
    assert_eq!(mapped_bounds(r, |x, y| (x * 1e12, y)), None);
    assert_eq!(mapped_bounds(Rect::EMPTY, |x, y| (x, y)), None);
}

/// A selected layer that Auto-Align couldn't place isn't moved with the aligned box: it only
/// follows the canvas origin, like an unselected layer. Locked, it isn't checked for the lock, so
/// moving it would move a locked layer.
#[test]
fn a_selected_layer_that_was_not_placed_stays_with_the_canvas() {
    for lock in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "background": "white"})).unwrap();
        // The aligned box is the reference at 50,40,350,260: no larger than the canvas, so the
        // canvas keeps its size and only the aligned layers move by -50,-40.
        let reference = new_layer(&mut s, "Ref");
        paint(&mut s, reference, Rect::new(50, 40, 350, 260), tex);
        let shift = new_layer(&mut s, "Shift");
        paint(&mut s, shift, Rect::new(74, 56, 374, 276), |x, y| tex(x - 24, y - 16));
        // Flat colour: nothing to match, so it gets no placement.
        let odd = new_layer(&mut s, "Odd");
        paint(&mut s, odd, Rect::new(120, 100, 180, 160), |_, _| [0.3, 0.3, 0.3, 1.0]);
        if lock {
            s.edit("lock", |doc, _| {
                doc.layer_mut(odd).unwrap().locks.set("all", true);
                Ok(())
            })
            .unwrap();
        }
        select(&mut s, &[reference, shift, odd]);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": reference.0})).unwrap();
        assert_eq!(r["failed"], json!([odd.0]), "lock {lock}: {r}");
        assert_eq!(canvas(&s), (400, 300), "lock {lock}");
        assert_eq!(bounds(&s, reference), Rect::new(0, 0, 300, 220), "lock {lock}: the aligned box starts at 0,0");
        assert!(near(bounds(&s, shift), (0, 0, 300, 220), 0), "lock {lock}: {:?}", bounds(&s, shift));
        assert_eq!(bounds(&s, odd), Rect::new(120, 100, 180, 160), "lock {lock}: the layer that wasn't placed stays put");
    }
}

#[test]
fn a_selected_background_that_was_not_placed_stays_the_background() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
        let bg = s.active().unwrap().doc.background().unwrap().id;
        // The aligned box is the reference at 50,40,350,260: the canvas keeps its size and only the
        // aligned layers move by -50,-40. The flat white Background has nothing to match.
        let reference = new_layer(&mut s, "Ref");
        paint(&mut s, reference, Rect::new(50, 40, 350, 260), tex);
        let shift = new_layer(&mut s, "Shift");
        paint(&mut s, shift, Rect::new(74, 56, 374, 276), |x, y| tex(x - 24, y - 16));
        select(&mut s, &[bg, reference, shift]);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": reference.0})).unwrap();
        assert_eq!(r["failed"], json!([bg.0]), "depth {depth}: {r}");
        assert_eq!(canvas(&s), (400, 300), "depth {depth}");
        assert_eq!(bounds(&s, reference), Rect::new(0, 0, 300, 220), "depth {depth}");
        let doc = &s.active().unwrap().doc;
        assert!(doc.is_background(bg), "depth {depth}: the Background didn't move, so it stays the Background");
        assert_eq!(doc.layer(bg).unwrap().name, "Background", "depth {depth}");
    }
}
