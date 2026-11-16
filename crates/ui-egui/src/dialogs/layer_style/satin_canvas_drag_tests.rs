//! #14985: with a Satin page selected, a drag on the image changes the Satin's Angle and
//! Distance, and the options bar says so.
use super::*;

/// The press start time the canvas passes for one gesture.
const PRESS: f64 = 1.0;

fn fields(kind: &str) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("selected".into(), json!("fx1"));
    f.insert("globalLight".into(), json!(120.0));
    f.insert(
        "effects".into(),
        json!([
            {"id": "fx1", "kind": kind, "on": true, "params": {"angle": 90.0, "distance": 20.0, "useGlobalLight": false}},
            {"id": "fx2", "kind": "dropShadow", "on": true, "params": {"angle": 120.0, "distance": 10.0, "useGlobalLight": true}},
        ]),
    );
    f
}

fn params(f: &Map<String, Value>, id: &str) -> (f64, f64) {
    let p = &entry(f, id).unwrap()["params"];
    (p["angle"].as_f64().unwrap(), p["distance"].as_f64().unwrap())
}

/// Resets the Satin to 90 / 20, presses, and drags by `total`.
fn drag_from_90_20(f: &mut Map<String, Value>, total: [f64; 2]) -> (f64, f64) {
    set_param(f, "fx1", "angle", json!(90.0));
    set_param(f, "fx1", "distance", json!(20.0));
    drag_satin(f, true, [0.0, 0.0], PRESS);
    drag_satin(f, false, total, PRESS);
    params(f, "fx1")
}

/// #14985: each drag from the press gives the measured angle (truncated) and distance (truncated).
#[test]
fn a_satin_drag_sets_the_truncated_angle_and_distance() {
    let mut f = fields("satin");
    assert!(drags_satin(&f));
    drag_satin(&mut f, true, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0), "a plain press keeps the satin where it is");
    for (total, want) in [
        ([30.0, 0.0], (146.0, 36.0)),
        ([0.0, -30.0], (-90.0, 10.0)),
        ([0.0, 30.0], (90.0, 50.0)),
        ([-30.0, 0.0], (33.0, 36.0)),
        ([20.0, -20.0], (180.0, 20.0)),
        ([5.0, 0.0], (104.0, 20.0)),
        // Measured later on the issue: negative angles truncate toward zero, and Shift (which the
        // drag ignores) gave (+30, +10) -> 135 / 42.
        ([5.0, -30.0], (-116.0, 11.0)),
        ([-5.0, -30.0], (-63.0, 11.0)),
        ([30.0, 10.0], (135.0, 42.0)),
    ] {
        assert_eq!(drag_from_90_20(&mut f, total), want, "drag {total:?}");
    }
}

/// #14985: a drag that brings the vector to (0, 0) gives Distance 1 and keeps the previous Angle.
#[test]
fn a_satin_drag_to_the_zero_vector_keeps_the_angle_at_distance_one() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [0.0, -20.0]), (90.0, 1.0));
}

/// #14985: Distance stays within the slider's 1 to 250 px, and a non-finite drag changes nothing.
#[test]
fn a_satin_drag_clamps_the_distance_and_ignores_non_finite_input() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [0.0, 380.0]).1, 250.0);
    assert_eq!(drag_from_90_20(&mut f, [f64::NAN, 0.0]), (90.0, 20.0));
}

/// #14985: only a Satin page takes the drag, and only it has the options bar hint.
#[test]
fn only_the_satin_page_takes_the_drag_and_the_hint() {
    let hint = "Click and drag to adjust the satin effect.";
    assert_eq!(canvas_drag_hint(&fields("satin")), Some(hint));
    for kind in ["dropShadow", "innerShadow", "gradientOverlay"] {
        let mut f = fields(kind);
        assert!(!drags_satin(&f), "{kind}");
        assert_eq!(canvas_drag_hint(&f), None, "{kind}");
        drag_satin(&mut f, true, [0.0, 0.0], PRESS);
        drag_satin(&mut f, false, [30.0, 0.0], PRESS);
        assert_eq!(params(&f, "fx1"), (90.0, 20.0), "{kind}: nothing moves");
    }
    let mut f = fields("satin");
    f.insert("selected".into(), json!(BLENDING));
    assert!(!drags_satin(&f));
    assert_eq!(canvas_drag_hint(&f), None);
    drag_satin(&mut f, true, [0.0, 0.0], PRESS);
    drag_satin(&mut f, false, [30.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0));
}

/// #14985: a Satin ignores Global Light, so its drag leaves the shared angle and other effects
/// alone, and the shadow drag does not take Satin pages.
#[test]
fn a_satin_drag_leaves_global_light_and_the_shadow_drag_alone() {
    let mut f = fields("satin");
    assert!(!drags_shadow(&f));
    drag_shadow(&mut f, true, [0.0, 0.0]);
    drag_shadow(&mut f, false, [30.0, 0.0]);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0), "the shadow drag ignores a Satin page");
    drag_from_90_20(&mut f, [30.0, 0.0]);
    assert_eq!(f["globalLight"], json!(120.0));
    assert!(f.get("pendingGlobalLight").is_none());
    assert_eq!(params(&f, "fx2"), (120.0, 10.0));
}

/// #14985: the canvas's one offset drag takes Satin and shadow pages alike and hands each to its
/// own drag.
#[test]
fn the_canvas_offset_drag_routes_satin_and_shadow_pages() {
    let mut f = fields("satin");
    assert!(drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    drag_offset(&mut f, false, [5.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (104.0, 20.0), "truncated, as the satin drag does");
    f.insert("selected".into(), json!("fx2"));
    assert!(drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx2"), (120.0, 10.0));
    let mut f = fields("gradientOverlay");
    assert!(!drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    drag_offset(&mut f, false, [30.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0));
}

/// #14985: one drag continuing past the zero vector keeps the Angle shown (146), not the press's.
#[test]
fn a_continuing_drag_through_the_zero_vector_keeps_the_displayed_angle() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    drag_satin(&mut f, false, [0.0, -20.0], PRESS);
    assert_eq!(params(&f, "fx1"), (146.0, 1.0));
}

/// #14985: a press the drag did not see begin (another press time) moves the satin only by the travel
/// since the drag took over, from the values shown, never from an earlier gesture's.
#[test]
fn a_press_the_drag_did_not_see_begin_rebaselines_from_the_shown_values() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    set_param(&mut f, "fx1", "angle", json!(0.0));
    set_param(&mut f, "fx1", "distance", json!(40.0));
    let b = 2.0;
    drag_satin(&mut f, false, [10.0, 0.0], b);
    assert_eq!(params(&f, "fx1"), (0.0, 40.0), "nothing jumps to the old gesture's values");
    // Offset (-40, 0) + travel (10, 30) = (-30, 30): 45 degrees, 42 px.
    drag_satin(&mut f, false, [20.0, 30.0], b);
    assert_eq!(params(&f, "fx1"), (45.0, 42.0));
    // The same press goes on from that baseline: (-40, 0) + (0, 40) = (-40, 40).
    drag_satin(&mut f, false, [10.0, 40.0], b);
    assert_eq!(params(&f, "fx1"), (45.0, 56.0));
}

/// #14985: a new gesture that begins at the same spot as the last one (a middle-button pan pressed
/// there during a held press, then released) is still another press: the drag re-baselines from
/// the values shown instead of restoring the earlier gesture's.
#[test]
fn a_new_press_at_the_same_spot_rebaselines_from_the_shown_values() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    set_param(&mut f, "fx1", "angle", json!(0.0));
    set_param(&mut f, "fx1", "distance", json!(40.0));
    // Same position, so the same travel as the old gesture's last frame, but a later press.
    drag_satin(&mut f, false, [30.0, 0.0], PRESS + 1.0);
    assert_eq!(params(&f, "fx1"), (0.0, 40.0), "nothing jumps back to 146 / 36");
    drag_satin(&mut f, false, [40.0, 0.0], PRESS + 1.0);
    assert_eq!(params(&f, "fx1"), (0.0, 30.0), "(-40, 0) + (10, 0) = (-30, 0)");
}
