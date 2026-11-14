//! #14508: Image › Image Rotation › Arbitrary keeps the document's guides and turns them with the
//! image instead of dropping them. The reference application measures a 400×300 canvas turned 30°
//! clockwise: the vertical guide at x=100 lands at 236.49 and the horizontal guide at y=50 at
//! 43.19, each within 0.15 px. A quarter turn swaps the guides' orientation, agreeing with Image
//! Rotation › 90° Clockwise (#14469). The Crop tool's straighten turn shares the rotation and keeps
//! dropping guides: not measured, so today's behaviour.

use serde_json::json;

use crate::Session;

/// The tolerance the measured guide positions came with.
const TOL: f64 = 0.15;

/// A 400×300 document with a vertical guide at x=100 and a horizontal guide at y=50.
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.edit("guides", |d, _| {
        d.guides.vertical = vec![100.0];
        d.guides.horizontal = vec![50.0];
        Ok(())
    })
    .unwrap();
    s
}

/// The document's guides, as (vertical positions, horizontal positions).
fn guides(s: &Session) -> (Vec<f64>, Vec<f64>) {
    let g = &s.active().unwrap().doc.guides;
    (g.vertical.clone(), g.horizontal.clone())
}

/// A guide at `want` within the measuring tolerance.
fn near(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() <= TOL, "{what}: got {got}, want {want} ± {TOL}");
}

/// The same guide position (an exact map, up to float noise).
fn same(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what}: got {got}, want {want}");
}

#[test]
fn arbitrary_turn_keeps_both_measured_guide_positions() {
    let mut s = doc();
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "cw"})).unwrap();
    let (v, h) = guides(&s);
    assert_eq!((v.len(), h.len()), (1, 1), "a 30° turn keeps both guides, each in its own direction");
    near(v[0], 236.49, "the vertical guide at x=100 after 30° clockwise");
    near(h[0], 43.19, "the horizontal guide at y=50 after 30° clockwise");
}

#[test]
fn arbitrary_turn_puts_a_guide_at_its_turned_canvas_edge_point() {
    // The guide follows the point it crossed the canvas edge, so the guide along the old left edge
    // ends up at the turned image's top corner: (0, 0) → (150, 0) for a 30° clockwise turn.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.edit("guides", |d, _| {
        d.guides.vertical = vec![0.0];
        d.guides.horizontal = vec![0.0];
        Ok(())
    })
    .unwrap();
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "cw"})).unwrap();
    let (v, h) = guides(&s);
    same(v[0], 150.0, "the guide along the left edge");
    same(h[0], 0.0, "the guide along the top edge");
}

#[test]
fn quarter_turn_swaps_the_guides_like_the_right_angle_command_does() {
    let mut s = doc();
    s.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw"})).unwrap();
    let (v, h) = guides(&s);
    assert_eq!((v.len(), h.len()), (1, 1), "a quarter turn swaps their orientation");
    // x=100 vertical becomes y=100 horizontal, and y=50 horizontal becomes x = 300 − 50 vertical.
    near(h[0], 100.0, "the vertical guide at x=100 after 90° clockwise");
    near(v[0], 250.0, "the horizontal guide at y=50 after 90° clockwise");
    // Image Rotation › 90° Clockwise on the same document lands on the same guides (#14469).
    let mut s2 = doc();
    s2.execute("image.imageRotation.90cw", json!({})).unwrap();
    let (v2, h2) = guides(&s2);
    assert_eq!((v2.len(), h2.len()), (v.len(), h.len()), "90° clockwise");
    for (a, b) in v.iter().zip(&v2) {
        same(*a, *b, "arbitrary 90° vs imageRotation.90cw, vertical");
    }
    for (a, b) in h.iter().zip(&h2) {
        same(*a, *b, "arbitrary 90° vs imageRotation.90cw, horizontal");
    }
}

#[test]
fn right_angle_turns_keep_the_number_of_guides() {
    for angle in [90.0, 180.0, 270.0] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        s.edit("guides", |d, _| {
            d.guides.vertical = vec![100.0, 250.0];
            d.guides.horizontal = vec![50.0];
            Ok(())
        })
        .unwrap();
        s.execute("image.rotation.arbitrary", json!({"angle": angle, "direction": "cw"})).unwrap();
        let (v, h) = guides(&s);
        // A half turn keeps both guides in their own direction; a quarter turn swaps them.
        let want = if angle == 180.0 { (2, 1) } else { (1, 2) };
        assert_eq!((v.len(), h.len()), want, "{angle}° clockwise keeps every guide");
    }
}

#[test]
fn undo_restores_the_guides_before_the_turn() {
    let mut s = doc();
    let before = guides(&s);
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "cw"})).unwrap();
    assert_ne!(guides(&s), before, "the turn moved them");
    s.undo();
    assert_eq!(guides(&s), before, "undo puts the guides back exactly");
}

#[test]
fn a_guide_outside_the_canvas_is_turned_rather_than_dropped() {
    // Nothing is clamped to the canvas: every point of the image's own edges lands inside the
    // turned canvas, and a guide that was already outside it (a mark left outside by Canvas Size,
    // cf. #11762) turns with the image and stays outside instead of going away.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.edit("guides", |d, _| {
        d.guides.vertical = vec![500.0];
        d.guides.horizontal = vec![-20.0];
        Ok(())
    })
    .unwrap();
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "cw"})).unwrap();
    let (v, h) = guides(&s);
    assert_eq!((v.len(), h.len()), (1, 1), "no guide is dropped for sitting outside the canvas");
    let size = s.active().unwrap().doc.size;
    assert!(v[0] > f64::from(size.width), "the vertical guide should stay outside the {} px width: {v:?}", size.width);
    assert!(h[0] < 0.0, "the horizontal guide should stay outside the {} px height: {h:?}", size.height);
}

#[test]
fn the_turn_drops_guide_colours_as_it_does_today() {
    // Guide colours for a turn are not measured, so they keep today's behaviour: the guide's
    // position stays, its own colour goes with the cleared colours.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 100, "color": "#ff00ff"})).unwrap();
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "cw"})).unwrap();
    let d = s.active().unwrap().doc.clone();
    assert_eq!(d.guides.vertical.len(), 1, "the guide itself stays");
    assert!(d.guides.colors.is_empty(), "its colour goes, as it does today");
}

#[test]
fn crop_with_an_angle_still_drops_the_guides() {
    // The Crop tool's straighten turn runs the same rotation with `keep_counts` off; its guides are
    // not measured, so they keep going away.
    let mut s = doc();
    s.execute("image.crop", json!({"x": 50, "y": 40, "width": 300, "height": 200, "angle": 30})).unwrap();
    let (v, h) = guides(&s);
    assert!(v.is_empty() && h.is_empty(), "the turned crop still drops the guides: vertical {v:?}, horizontal {h:?}");
}

#[test]
fn ruler_straighten_still_drops_the_guides() {
    // Straighten Layer is off for the Background (#12033), so a layer is added first.
    let mut s = doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [100, 10]})).unwrap();
    s.execute("image.analysis.straightenLayer", json!({"crop": true})).unwrap();
    let (v, h) = guides(&s);
    assert!(v.is_empty() && h.is_empty(), "straighten still drops the guides: vertical {v:?}, horizontal {h:?}");
}
