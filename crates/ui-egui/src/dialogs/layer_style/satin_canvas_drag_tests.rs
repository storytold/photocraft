//! #14985: with a Satin page selected, a drag on the image changes the Satin's Angle and Distance,
//! and the options bar says so.
use craft::imaging::layer_style::{EffectKind, StyleValue};
use serde_json::json;

use super::open;
use super::tests::{app_with_layer, draft, edit, first_of};
use crate::PhotocraftApp;
use crate::dialogs::canvas_input::{self, DialogPick};

/// A Satin page at Angle 90 and Distance 20, open in the dialog.
fn satin() -> (Box<PhotocraftApp>, craft::ui::dialog::DialogId, craft::imaging::layer_style::EffectId) {
    let mut app = app_with_layer(64);
    app.run("layer.layerStyle.satin", json!({"angle": 90, "distance": 20})).unwrap();
    let id = open(&mut app, Some(EffectKind::Satin.key())).unwrap();
    let fx = first_of(&app, id, EffectKind::Satin);
    edit(&mut app, id, |d| {
        d.set_param(fx, "angle", StyleValue::Number(90.0));
        d.set_param(fx, "distance", StyleValue::Number(20.0));
    });
    (app, id, fx)
}

fn shown(app: &PhotocraftApp, id: craft::ui::dialog::DialogId, fx: craft::imaging::layer_style::EffectId) -> (f64, f64) {
    let p = &draft(app, id).style.effect(fx).unwrap().params;
    (p.number("angle").unwrap(), p.number("distance").unwrap())
}

/// Presses and drags by `total`, as the canvas tool does, from the Satin at 90 / 20.
fn drag(app: &mut PhotocraftApp, total: [f64; 2]) {
    let mut base = None;
    canvas_input::drag_satin(app, &mut base, [0.0, 0.0]);
    canvas_input::drag_satin(app, &mut base, total);
}

/// Each drag from the press gives the measured angle (truncated) and distance (truncated).
#[test]
fn a_satin_drag_sets_the_truncated_angle_and_distance() {
    let (mut app, id, fx) = satin();
    assert_eq!(canvas_input::armed(&app), Some(DialogPick::SatinDrag));
    drag(&mut app, [0.0, 0.0]);
    assert_eq!(shown(&app, id, fx), (90.0, 20.0), "a plain press keeps the satin where it is");
    for (total, want) in [
        ([30.0, 0.0], (146.0, 36.0)),
        ([0.0, -30.0], (-90.0, 10.0)),
        ([0.0, 30.0], (90.0, 50.0)),
        ([-30.0, 0.0], (33.0, 36.0)),
        ([20.0, -20.0], (180.0, 20.0)),
        ([5.0, 0.0], (104.0, 20.0)),
        // Negative angles truncate toward zero; Shift, which the drag ignores, gave (+30, +10) -> 135 / 42.
        ([5.0, -30.0], (-116.0, 11.0)),
        ([-5.0, -30.0], (-63.0, 11.0)),
        ([30.0, 10.0], (135.0, 42.0)),
    ] {
        edit(&mut app, id, |d| {
            d.set_param(fx, "angle", StyleValue::Number(90.0));
            d.set_param(fx, "distance", StyleValue::Number(20.0));
        });
        drag(&mut app, total);
        assert_eq!(shown(&app, id, fx), want, "drag {total:?}");
    }
}

/// A drag that brings the vector to (0, 0) gives Distance 1 and keeps the previous Angle.
#[test]
fn a_collapsed_vector_keeps_the_angle_and_a_long_drag_stops_at_the_limit() {
    let (mut app, id, fx) = satin();
    drag(&mut app, [0.0, -20.0]);
    assert_eq!(shown(&app, id, fx), (90.0, 1.0));
    edit(&mut app, id, |d| {
        d.set_param(fx, "angle", StyleValue::Number(90.0));
        d.set_param(fx, "distance", StyleValue::Number(20.0));
    });
    drag(&mut app, [0.0, 900.0]);
    assert_eq!(shown(&app, id, fx).1, 250.0, "Distance is at most 250");
}

/// The drag moves one gesture's baseline: a second press starts from the values shown then.
#[test]
fn each_press_starts_from_the_values_shown_then() {
    let (mut app, id, fx) = satin();
    drag(&mut app, [30.0, 0.0]);
    assert_eq!(shown(&app, id, fx), (146.0, 36.0));
    drag(&mut app, [0.0, 0.0]);
    assert_eq!(shown(&app, id, fx), (146.0, 36.0), "no travel keeps what the last drag left");
}

/// Other pages keep their picks, and the hint is the Satin's only.
#[test]
fn the_hint_and_the_pick_belong_to_the_satin_page() {
    let (mut app, id, _) = satin();
    assert_eq!(canvas_input::drag_hint(&app), Some("Click and drag to adjust the satin effect."));
    edit(&mut app, id, |d| d.open_on(EffectKind::DropShadow));
    assert_eq!(canvas_input::armed(&app), Some(DialogPick::ShadowDrag));
    assert_eq!(canvas_input::drag_hint(&app), None);
}
