//! #12625: an Art History Brush press held still keeps starting new strokes inside the Area
//! circle from the press until release, all in one "Art History Brush" state; a drag keeps its
//! path too, and no hold makes the engine refuse the press.

use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::stroke_timing_tests::{button, harness, move_to, tap};

/// The canvas harness steps a quarter second per frame.
const STEP_MS: f64 = 250.0;

/// The canvas with the Art History Brush (Area 50, a `size` px tip) over a black fill; its
/// source, the open state, is white.
fn art_history(size: f64) -> Harness<'static, PhotocraftApp> {
    let mut h = harness();
    let app = h.state_mut();
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
    app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": size, "hardness": 1.0}})).unwrap();
    app.ui.tool = Tool::ArtHistoryBrush;
    app.ui.tool_options.art_history_area = 50.0;
    h.run_steps(1);
    h
}

/// Hold the pointer still for `ms`.
fn hold(h: &mut Harness<'static, PhotocraftApp>, ms: f64) {
    h.run_steps((ms / STEP_MS) as usize);
}

/// Press at (`x`, 50), hold `ms`, release.
fn held_press(h: &mut Harness<'static, PhotocraftApp>, x: f32, ms: f64) {
    move_to(h, x, 50.0);
    button(h, x, 50.0, true);
    hold(h, ms - STEP_MS);
    button(h, x, 50.0, false);
    h.run_steps(2);
}

/// Pixels the white source restored over the black fill.
fn restored(h: &Harness<'static, PhotocraftApp>) -> usize {
    let s = h.state().session.documents()[0].doc.layers[0].surface().unwrap();
    (0..100).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|&(x, y)| s.rgba(x, y)[0] > 0.5).count()
}

fn steps(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.documents()[0].history.past_len()
}

/// The `[x, y]` points of the last committed `paint.artHistoryBrush`.
fn committed(h: &Harness<'static, PhotocraftApp>) -> Vec<[f64; 2]> {
    let (_, p) = h.state().session.journal.iter().rev().find(|(id, _)| id == "paint.artHistoryBrush").cloned().unwrap();
    p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
}

fn within(p: [f64; 2], c: [f64; 2], r: f64) -> bool {
    (p[0] - c[0]).hypot(p[1] - c[1]) <= r + 1e-9
}

/// Pixels a click paints, with an 8 px tip.
fn click() -> usize {
    let mut h = art_history(8.0);
    tap(&mut h, 100.0, &[]);
    assert_eq!(committed(&h), vec![[100.0, 50.0]], "a click is one point, as before");
    restored(&h)
}

#[test]
fn a_press_starts_the_stroke_and_keeps_the_canvas_running() {
    let mut h = art_history(8.0);
    move_to(&mut h, 100.0, 50.0);
    button(&mut h, 100.0, 50.0, true);
    // An app that only repaints on input must still get frames while the pointer is still.
    assert!(h.state().drag.is_some(), "the press itself starts the stroke");
    assert!(h.ctx.has_requested_repaint(), "the held press asks for the next frame");
    assert!(h.state().live_stroke.is_none(), "no live preview: the tool paints on release");
    button(&mut h, 100.0, 50.0, false);
    h.run_steps(2);
    assert!(h.state().drag.is_none());
}

#[test]
fn a_held_press_keeps_painting_until_release_in_one_undoable_state() {
    let click = click();
    assert!(click > 0, "a click paints");

    // Half a second, before egui would take a still press for a drag: the hold counts from the press.
    let mut h = art_history(8.0);
    held_press(&mut h, 100.0, 500.0);
    assert!(restored(&h) > click, "a 0.5 s hold paints more than a click: {} vs {click} px", restored(&h));
    assert!(committed(&h).len() > 20, "one new stroke per 16 ms from the press: {} points", committed(&h).len());

    let mut h = art_history(8.0);
    let before = steps(&h);
    held_press(&mut h, 100.0, 1500.0);
    let held = restored(&h);
    assert!(held > click * 3, "a 1.5 s hold paints clearly more than a click: {held} vs {click} px");
    let pts = committed(&h);
    assert!(pts.len() > 80, "new strokes keep starting while held: {} points", pts.len());
    assert!(pts.iter().all(|&p| within(p, [100.0, 50.0], 25.0)), "every start inside the Area circle: {pts:?}");

    // Release ends it: more frames add nothing.
    h.run_steps(8);
    assert_eq!(restored(&h), held, "nothing paints after release");
    assert_eq!(steps(&h), before + 1, "the whole press is one state");
    assert_eq!(h.state().session.documents()[0].history.undo_label(), Some("Art History Brush"));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(restored(&h), 0, "one undo takes the whole press back");
}

#[test]
fn moving_while_held_keeps_the_drag_and_the_held_strokes() {
    let mut h = art_history(8.0);
    let before = steps(&h);
    move_to(&mut h, 40.0, 50.0);
    button(&mut h, 40.0, 50.0, true);
    for x in [60.0, 80.0] {
        move_to(&mut h, x, 50.0);
    }
    hold(&mut h, 1500.0);
    for x in [120.0, 160.0] {
        move_to(&mut h, x, 50.0);
    }
    button(&mut h, 160.0, 50.0, false);
    h.run_steps(2);
    let pts = committed(&h);
    let at = |x: f64| pts.iter().position(|p| *p == [x, 50.0]).unwrap_or_else(|| panic!("the drag passes {x}: {pts:?}"));
    let (a, b) = (at(80.0), at(120.0));
    assert!(at(40.0) < at(60.0) && at(60.0) < a && a < b && b < at(160.0), "the drag keeps its order: {pts:?}");
    let held = &pts[a + 1..b];
    assert!(held.len() > 60, "the hold at 80 started new strokes: {} points", held.len());
    assert!(held.iter().all(|&p| within(p, [80.0, 50.0], 25.0)), "inside the Area circle around 80: {held:?}");
    assert_eq!(steps(&h), before + 1, "drag and hold are one state");
    let s = h.state().session.documents()[0].doc.layers[0].surface().unwrap();
    assert!((150..170).any(|x| s.rgba(x, 50)[0] > 0.5), "the drag still paints after the hold");
}

/// The press paints and adds its state, with no error.
fn painted(h: &Harness<'static, PhotocraftApp>, before: usize, case: &str) {
    let app = h.state();
    assert!(!app.ui.status_error, "{case}: {}", app.ui.status);
    assert_eq!(steps(h), before + 1, "{case}: one state");
    assert!(restored(h) > 0, "{case}: the press painted");
    assert!(committed(h).len() <= photocraft_algo::art_history::MAX_DABS, "{case}");
}

#[test]
fn long_holds_with_a_small_tip_stay_within_the_strokes_limits() {
    // A 1 px tip: every new start's way across the Area circle costs dozens of dabs.
    let mut h = art_history(1.0);
    let before = steps(&h);
    held_press(&mut h, 100.0, 6000.0);
    painted(&h, before, "one long hold");

    let mut h = art_history(1.0);
    let before = steps(&h);
    move_to(&mut h, 20.0, 50.0);
    button(&mut h, 20.0, 50.0, true);
    for x in [60.0, 100.0, 140.0] {
        hold(&mut h, 5000.0);
        move_to(&mut h, x, 50.0);
    }
    button(&mut h, 140.0, 50.0, false);
    h.run_steps(2);
    painted(&h, before, "several pauses in one drag");
}

#[test]
fn with_area_zero_a_hold_sends_no_repeated_points() {
    let mut h = art_history(8.0);
    h.state_mut().ui.tool_options.art_history_area = 0.0;
    held_press(&mut h, 100.0, 2000.0);
    assert_eq!(committed(&h), vec![[100.0, 50.0]], "no circle to start new strokes in: the press stays one point");
}

#[test]
fn a_hold_with_a_huge_tip_stays_within_the_coverage_limit() {
    // A click with this tip fits the engine's coverage limit; new starts across a 100 px Area
    // circle would push the dabs' bounds past it.
    let mut h = art_history(4540.0);
    h.state_mut().ui.tool_options.art_history_area = 100.0;
    let before = steps(&h);
    held_press(&mut h, 100.0, 4000.0);
    painted(&h, before, "a huge tip held");
}
