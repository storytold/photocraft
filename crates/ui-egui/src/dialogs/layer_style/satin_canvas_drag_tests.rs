//! #14985: with a Satin page selected, a drag on the image changes the Satin's Angle and Distance,
//! and the options bar says so. The draft-level tests call the drag directly; the canvas tests drive the dialog tool
//! through `canvas::testing`, as a press, moves and a release on the image.
use craft::imaging::layer_style::{EffectKind, StyleValue};
use egui::Modifiers;
use serde_json::json;

use super::open;
use super::tests::{app_with_layer, draft, edit, first_of};
use crate::PhotocraftApp;
use crate::canvas::testing::{clear_of_dialogs, ctx, frame, move_to, press, release, to_screen};
use crate::dialogs::canvas_input::{self, DialogPick};

/// A Satin page at Angle 90 and Distance 20, open in the dialog.
fn satin() -> (Box<PhotocraftApp>, craft::ui::dialog::DialogId, craft::imaging::layer_style::EffectId) {
    let mut app = app_with_layer(400);
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
    assert!(canvas_input::drag_satin(app, true, [0.0, 0.0]), "the Satin page takes the press");
    assert!(canvas_input::drag_satin(app, false, total), "and the drag");
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

/// One drag through the zero vector keeps the Angle shown, not the one captured at the press.
#[test]
fn a_continuing_drag_through_the_zero_vector_keeps_the_displayed_angle() {
    let (mut app, id, fx) = satin();
    canvas_input::drag_satin(&mut app, true, [0.0, 0.0]);
    canvas_input::drag_satin(&mut app, false, [30.0, 0.0]);
    assert_eq!(shown(&app, id, fx), (146.0, 36.0));
    canvas_input::drag_satin(&mut app, false, [0.0, -20.0]);
    assert_eq!(shown(&app, id, fx), (146.0, 1.0));
}

/// A plain press, or a drag back to where it began, keeps fractional values as they were.
#[test]
fn no_travel_keeps_fractional_values() {
    let (mut app, id, fx) = satin();
    edit(&mut app, id, |d| {
        d.set_param(fx, "angle", StyleValue::Number(90.5));
        d.set_param(fx, "distance", StyleValue::Number(20.75));
    });
    drag(&mut app, [0.0, 0.0]);
    assert_eq!(shown(&app, id, fx), (90.5, 20.75));
}

/// A quarter-pixel offset (one screen pixel at 400 %) still has its direction; Distance stops at 250 and a non-finite
/// drag changes nothing.
#[test]
fn a_quarter_pixel_keeps_its_direction_and_the_distance_is_bounded() {
    let (mut app, id, fx) = satin();
    drag(&mut app, [0.25, -20.0]);
    assert_eq!(shown(&app, id, fx), (180.0, 1.0));
    edit(&mut app, id, |d| {
        d.set_param(fx, "angle", StyleValue::Number(90.0));
        d.set_param(fx, "distance", StyleValue::Number(20.0));
    });
    drag(&mut app, [0.0, 380.0]);
    assert_eq!(shown(&app, id, fx).1, 250.0);
    edit(&mut app, id, |d| {
        d.set_param(fx, "angle", StyleValue::Number(90.0));
        d.set_param(fx, "distance", StyleValue::Number(20.0));
    });
    drag(&mut app, [f64::NAN, 0.0]);
    assert_eq!(shown(&app, id, fx), (90.0, 20.0));
}

/// The Satin on the active layer of `doc`, as `(angle, distance)`.
fn satin_of(doc: &photocraft_doc::Document) -> Option<(f32, f32)> {
    doc.layers.iter().rev().find_map(|l| {
        l.effects.items.iter().find_map(|e| match e {
            photocraft_doc::Effect::Satin(s) => Some((s.angle, s.distance)),
            _ => None,
        })
    })
}

/// The Satin dialog's app after the host lists the dialog it opened.
fn canvas_app() -> (Box<PhotocraftApp>, craft::ui::dialog::DialogId, craft::imaging::layer_style::EffectId) {
    let (mut app, id, fx) = satin();
    frame(&mut app);
    (app, id, fx)
}

/// A press, two moves and (unless `hold`) a release on the image, from `start` by `delta` document pixels.
fn canvas_drag(app: &mut PhotocraftApp, start: [f64; 2], delta: [f64; 2], mods: Modifiers, hold: bool) {
    let end = [start[0] + delta[0], start[1] + delta[1]];
    move_to(app, start, mods);
    press(app, start, mods);
    move_to(app, [start[0] + delta[0] / 2.0, start[1] + delta[1] / 2.0], mods);
    move_to(app, end, mods);
    if !hold {
        release(app, end, mods);
    }
}

/// Through the canvas: the drag counts in document pixels at every zoom (60 screen px at 200 % is 30 px).
#[test]
fn on_the_canvas_a_drag_counts_in_document_pixels_at_every_zoom() {
    for (zoom, delta, want) in [(2.0f32, [30.0, 0.0], (146.0, 36.0)), (2.0, [5.0, -30.0], (-116.0, 11.0)), (1.0, [30.0, 0.0], (146.0, 36.0))] {
        let (mut app, id, fx) = canvas_app();
        app.ui.views[0].zoom = zoom;
        let start = [200.0, 150.0];
        clear_of_dialogs(&mut app, start);
        if zoom == 2.0 && delta == [30.0, 0.0] {
            let screen = to_screen(&app, [start[0] + delta[0], start[1]]) - to_screen(&app, start);
            assert!((screen.x - 60.0).abs() < 0.5, "the drag travels 60 screen px: {screen:?}");
        }
        canvas_drag(&mut app, start, delta, Modifiers::NONE, false);
        assert_eq!(shown(&app, id, fx), want, "{zoom}x, {delta:?}");
    }
}

/// Through the canvas: Shift changes nothing and doesn't snap the angle.
#[test]
fn on_the_canvas_shift_changes_nothing() {
    for (delta, want) in [([30.0, 0.0], (146.0, 36.0)), ([5.0, -30.0], (-116.0, 11.0)), ([30.0, 10.0], (135.0, 42.0))] {
        let (mut app, id, fx) = canvas_app();
        let start = [200.0, 150.0];
        clear_of_dialogs(&mut app, start);
        canvas_drag(&mut app, start, delta, Modifiers::SHIFT, false);
        assert_eq!(shown(&app, id, fx), want, "Shift, {delta:?}");
    }
}

/// Through the canvas: the preview shows the drag while the document keeps its style, and Cancel restores it.
#[test]
fn on_the_canvas_the_preview_follows_and_cancel_restores() {
    let (mut app, id, fx) = canvas_app();
    let start = [200.0, 150.0];
    clear_of_dialogs(&mut app, start);
    let before = satin_of(&app.session.active().unwrap().doc);
    canvas_drag(&mut app, start, [30.0, 0.0], Modifiers::NONE, true);
    assert_eq!(shown(&app, id, fx), (146.0, 36.0), "live while the button is held");
    let (preview, _) = super::preview_document(&app).expect("the dialog previews its draft");
    assert_eq!(satin_of(&preview), Some((146.0, 36.0)), "the preview shows the dragged Satin");
    assert_eq!(satin_of(&app.session.active().unwrap().doc), before, "the document is untouched while the dialog is open");
    release(&mut app, [230.0, 150.0], Modifiers::NONE);
    super::tests::cancel(&mut app, id);
    assert!(app.framework.dialogs.stack().top().is_none());
    assert_eq!(satin_of(&app.session.active().unwrap().doc), before, "Cancel leaves the document as it was");
}

/// A release past the last move, with no move event there, still lands the Satin where the button came up.
#[test]
fn on_the_canvas_the_release_position_counts() {
    use craft::ui::runtime::windows::{Released, WindowHost};
    let (mut app, id, fx) = canvas_app();
    let start = [200.0, 150.0];
    clear_of_dialogs(&mut app, start);
    move_to(&mut app, start, Modifiers::NONE);
    press(&mut app, start, Modifiers::NONE);
    move_to(&mut app, [210.0, 150.0], Modifiers::NONE);
    move_to(&mut app, [215.0, 150.0], Modifiers::NONE);
    assert_eq!(shown(&app, id, fx), (126.0, 25.0), "the last move: 15 px right");
    let end = to_screen(&app, [230.0, 150.0]);
    WindowHost::of(&ctx(&app)).inject_release(egui::ViewportId::ROOT, Some(end), Released::Button(craft::behavior::PointerButton::Primary));
    frame(&mut app);
    assert_eq!(shown(&app, id, fx), (146.0, 36.0), "the release, 30 px right");
}
