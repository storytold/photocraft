//! #14465: Image › Image Rotation › Arbitrary resamples with Preferences › General › Image
//! Interpolation when the `interpolation` param is absent; an explicit param still wins
//! (scripts and actions, #7630). Measured with the issue's document: a 400×300 RGB 8-bit
//! white Background with a green square (x 174..274, y 100..260) rotated 30° cw. The
//! square's left edge crosses row y=230 at x≈218.5, inside the scanned window 205..235,
//! and nothing else does: with `nearest` that row is pure white and pure green (the hard
//! edge the reference application shows for Nearest Neighbor), bilinear and bicubic put
//! soft pixels there. The exact soft values are not asserted (not part of the item).

use super::*;
use crate::prefs::Interpolation;

fn green_edge_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "mode": "rgb", "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 174, "y": 100, "width": 100, "height": 160, "antiAlias": false})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// Row pixels as 8-bit triples, x0..x1 at y (the rotated Background is layers[0]).
fn row(s: &Session, y: i32, x0: i32, x1: i32) -> Vec<[u8; 3]> {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    (x0..x1)
        .map(|x| {
            let c = surf.rgba(x, y);
            [c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8)
        })
        .collect()
}

/// Every Background pixel, row-major, as 8-bit triples.
fn snapshot(s: &Session) -> Vec<[u8; 3]> {
    let d = &s.active().unwrap().doc;
    let surf = d.layers[0].surface().unwrap();
    (0..d.size.height as i32)
        .flat_map(|y| {
            (0..d.size.width as i32).map(move |x| {
                let c = surf.rgba(x, y);
                [c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8)
            })
        })
        .collect()
}

fn is_white(p: &[u8; 3]) -> bool {
    p[0] >= 250 && p[1] >= 250 && p[2] >= 250
}

fn is_green(p: &[u8; 3]) -> bool {
    p[0] <= 5 && p[1] >= 250 && p[2] <= 5
}

/// Pixels neither white nor green: soft (resampled) edge pixels.
fn soft(r: &[[u8; 3]]) -> usize {
    r.iter().filter(|p| !is_white(p) && !is_green(p)).count()
}

/// Rotate 30° with the preference set to `pref` and no `interpolation` param; the edge row.
fn rotate_with_pref(pref: Interpolation) -> Vec<[u8; 3]> {
    let mut s = green_edge_doc();
    s.edit_prefs(|p| p.general.image_interpolation = pref);
    s.execute("image.rotation.arbitrary", json!({"angle": 30})).unwrap();
    row(&s, 230, 205, 236)
}

/// Rotate 30° with an explicit param on default preferences; the edge row.
fn rotate_with_param(tag: &str) -> Vec<[u8; 3]> {
    let mut s = green_edge_doc();
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "interpolation": tag})).unwrap();
    row(&s, 230, 205, 236)
}

#[test]
fn absent_param_uses_the_nearest_neighbor_preference() {
    let r = rotate_with_pref(Interpolation::Nearest);
    assert!(r.iter().any(is_white) && r.iter().any(is_green), "the edge must cross the scanned row: {r:?}");
    assert_eq!(soft(&r), 0, "Nearest Neighbor gives hard edges");
    assert_eq!(r, rotate_with_param("nearest"), "the preference acts as the `nearest` param does");
}

#[test]
fn absent_param_uses_the_bilinear_preference() {
    let r = rotate_with_pref(Interpolation::Bilinear);
    assert!(soft(&r) > 0, "Bilinear gives soft edges");
    assert_eq!(r, rotate_with_param("bilinear"), "the preference acts as the `bilinear` param does");
    assert_ne!(r, rotate_with_param("bicubic"), "the preference is not ignored");
}

#[test]
fn absent_param_with_a_bicubic_preference_stays_bicubic() {
    for pref in [Interpolation::Bicubic, Interpolation::BicubicAutomatic] {
        let r = rotate_with_pref(pref);
        assert!(soft(&r) > 0, "{pref:?} gives soft edges");
        assert_eq!(r, rotate_with_param("bicubic"), "{pref:?}");
    }
}

#[test]
fn explicit_interpolation_param_beats_the_preference() {
    let mut s = green_edge_doc();
    s.edit_prefs(|p| p.general.image_interpolation = Interpolation::Nearest);
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "interpolation": "bicubic"})).unwrap();
    assert!(soft(&row(&s, 230, 205, 236)) > 0, "scripts and actions keep their param");

    let mut s = green_edge_doc();
    s.edit_prefs(|p| p.general.image_interpolation = Interpolation::Bicubic);
    s.execute("image.rotation.arbitrary", json!({"angle": 30, "interpolation": "nearest"})).unwrap();
    assert_eq!(soft(&row(&s, 230, 205, 236)), 0, "a `nearest` param still gives hard edges");
}

#[test]
fn not_measured_preferences_keep_todays_bicubic_result() {
    for pref in [Interpolation::BicubicSmoother, Interpolation::BicubicSharper, Interpolation::PreserveDetails] {
        assert_eq!(rotate_with_pref(pref), rotate_with_param("bicubic"), "{pref:?}");
    }
}

#[test]
fn rotated_crop_and_straighten_ignore_the_interpolation_preference() {
    // #14465 measured Image Rotation › Arbitrary only: the Crop tool's rotated crop and
    // Straighten call the rotation with today's Bicubic and must not pick up the
    // preference (their cases are the epic's *not measured*, today's behaviour).
    for setup in ["crop", "straighten"] {
        let mut a = green_edge_doc();
        let mut b = green_edge_doc();
        b.edit_prefs(|p| p.general.image_interpolation = Interpolation::Nearest);
        for s in [&mut a, &mut b] {
            if setup == "crop" {
                s.execute("image.crop", json!({"x": 50, "y": 40, "width": 300, "height": 200, "angle": 30})).unwrap();
            } else {
                s.execute("image.analysis.rulerTool", json!({"start": [100.0, 250.0], "end": [300.0, 150.0]})).unwrap();
                // Straighten Layer is greyed out for the Background (#12033); `crop: true` is
                // the branch that turns the canvas through `rotate_arbitrary_with`.
                s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
                s.execute("image.analysis.straightenLayer", json!({"crop": true})).unwrap();
            }
        }
        assert_eq!(snapshot(&a), snapshot(&b), "{setup} keeps today's behaviour with the preference set");
        assert!(soft(&snapshot(&a)) > 0, "{setup} keeps the soft (bicubic) edge it has today");
    }
}
