//! #12682: with the Dodge tool's Airbrush toggle on - the brush's Build-up, which its options bar
//! now offers - a pointer held still over one spot keeps laying dabs, and those dabs lighten past
//! the stroke's one-pass limit. With it off, a repeat of the same dab adds nothing.
//!
//! The reference application's measured pixels (132 held off, 179 held for 3 s, and the 163
//! per-stroke limit of #12681) are not pinned here: #12681 and the epic's tone-curve items are not
//! in `main` yet, so the numbers this document would produce are not the reference's. What is
//! pinned is the order the issue describes - more held time is strictly lighter, the build-up never
//! lightens past white, and off stays a one-pass stroke - and that the other retouching tools take
//! none of it.

use serde_json::{Value, json};

use super::*;

/// A 60 x 40 8-bit RGB document of grey 128, the reference's test step.
fn grey() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 60, "height": 40, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("grey", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _y in b.y0..b.y1 {
            for _x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// Turn the brush's Airbrush build-up on or off, at the Brush tool's rate: ten dabs a second (this
/// issue leaves the rate not measured and says to keep the Brush's own).
fn airbrush(s: &mut Session, on: bool) {
    s.execute("tools.setBrush", json!({"buildUp": on})).unwrap();
    assert_eq!(s.tools.brush.build_up, on, "the session brush took Build-up");
}

/// A press at (30, 20) plus `holds` repeats of that point, each 100 ms after the last: the points a
/// held pointer sends, with the times the canvas records with them. One 100 ms step is one airbrush
/// dab at the rate above, so `holds` is how many dabs the hold adds.
fn held_points(holds: usize) -> Vec<Value> {
    (0..=holds).map(|k| json!([30.0, 20.0, 1.0, 0.0, 0.0, 0.0, k as f64 * 100.0])).collect()
}

/// The same press and time as a drag: `steps` points 6 px apart, one 100 ms after the other, so the
/// pointer never stands still and the airbrush steps fall along the path.
fn moving_points(steps: usize) -> Vec<Value> {
    (0..=steps).map(|k| json!([12.0 + 6.0 * k as f64, 20.0, 1.0, 0.0, 0.0, 0.0, k as f64 * 100.0])).collect()
}

/// The Dodge params for `points`: the issue's fixture (40 px hard tip, Exposure `exposure`,
/// Midtones, Protect Tones on).
fn dodge(points: &[Value], exposure: f64) -> Value {
    json!({"points": points, "size": 40, "hardness": 100, "exposure": exposure, "range": "midtones", "protectTones": true})
}

/// The active layer's pixel at (x, y), in 8-bit RGB.
fn rgb_at(s: &Session, x: i32, y: i32) -> [i32; 3] {
    let d = s.active().unwrap();
    let c = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as i32)
}

/// The centre pixel of a Dodge stroke of `points` with Build-up `on`.
fn dodged(on: bool, points: &[Value], exposure: f64) -> [i32; 3] {
    let mut s = grey();
    airbrush(&mut s, on);
    s.execute("paint.dodge", dodge(points, exposure)).unwrap();
    rgb_at(&s, 30, 20)
}

/// The grey of that pixel.
fn grey_dodged(on: bool, points: &[Value], exposure: f64) -> i32 {
    dodged(on, points, exposure)[0]
}

#[test]
fn a_held_airbrush_dodge_keeps_lightening() {
    // Held longer is strictly lighter, one airbrush dab at a time.
    let steps: Vec<i32> = (0..=6).map(|holds| grey_dodged(true, &held_points(holds), 50.0)).collect();
    for pair in steps.windows(2) {
        assert!(pair[1] > pair[0], "held dabs keep lightening, step {pair:?}");
    }
    // The same at the issue's Exposure 10 %: more hold, more lightening, never less.
    let soft: Vec<i32> = (0..=6).map(|holds| grey_dodged(true, &held_points(holds), 10.0)).collect();
    for pair in soft.windows(2) {
        assert!(pair[1] >= pair[0], "never darker as the hold grows, step {pair:?}");
    }
    assert!(soft.last().unwrap() > soft.first().unwrap(), "{soft:?}");
}

#[test]
fn airbrush_repeats_lighten_past_the_one_pass_limit() {
    // One pass over the spot is what the stroke cap allows - here the single press with Build-up
    // off. The held repeats go past it, which is the whole point of the toggle.
    let one_pass = grey_dodged(false, &held_points(0), 50.0);
    let held_once = grey_dodged(true, &held_points(1), 50.0);
    let held_long = grey_dodged(true, &held_points(6), 50.0);
    assert!(held_once > one_pass, "the repeats pass the one-pass limit: {one_pass} -> {held_once}");
    assert!(held_long > held_once, "and keep going: {held_once} -> {held_long}");
    assert_eq!(grey_dodged(false, &held_points(6), 50.0), one_pass, "off, the same points stay at one pass");
}

#[test]
fn airbrush_off_a_held_pointer_adds_nothing_after_the_first_dab() {
    // Repeating the same point in one stroke tones it no further, however long the hold: that is
    // the stroke's own cap (#12681), and with the toggle off it stays.
    let first = grey_dodged(false, &held_points(0), 50.0);
    for holds in [1, 8, 40] {
        assert_eq!(grey_dodged(false, &held_points(holds), 50.0), first, "off, {holds} repeats add nothing");
    }
    // A second stroke does build on the first, on or off.
    let mut s = grey();
    airbrush(&mut s, false);
    s.execute("paint.dodge", dodge(&held_points(0), 50.0)).unwrap();
    let (after_one, after_two) = (rgb_at(&s, 30, 20)[0], {
        s.execute("paint.dodge", dodge(&held_points(0), 50.0)).unwrap();
        rgb_at(&s, 30, 20)[0]
    });
    assert_eq!(after_one, first, "the first stroke is one pass");
    assert!(after_two > after_one, "a second stroke builds on the first: {after_one} -> {after_two}");
}

#[test]
fn the_build_up_never_lightens_past_white_and_its_steps_shrink() {
    let long = dodged(true, &held_points(400), 50.0);
    assert!(long.iter().all(|&c| c <= 255), "a 40 s hold stays inside the tone curve: {long:?}");
    assert!(long.iter().all(|&c| c >= 0), "{long:?}");
    let mid = grey_dodged(true, &held_points(20), 50.0);
    assert!(mid <= long[0], "the hold climbs towards its ceiling, never over it: {mid} -> {long:?}");
    // Protect Tones guards the highlights, so each further dab moves the spot less: a hold twenty
    // times as long as one dab is nowhere near twenty times the lightening, and the last step is no
    // bigger than an early one.
    let (first, early) = (grey_dodged(true, &held_points(1), 50.0), grey_dodged(true, &held_points(2), 50.0));
    assert!(first > 128, "the hold does lighten: {first}");
    assert!(early - first >= 0 && long[0] - mid <= early - first, "the steps shrink: +{} early, +{} late", early - first, long[0] - mid);
    // And a grey stays neutral: the build-up lightens, it does not tint.
    assert!(long[0] == long[1] && long[1] == long[2], "grey 128 stays a grey: {long:?}");
}

#[test]
fn airbrush_off_leaves_a_moving_stroke_at_one_pass() {
    // With the toggle off a stroke is still the capped one pass of #12681, and the times its points
    // may carry change nothing: going back over the spot in the same stroke adds nothing, on the way
    // through or on the way back.
    let there = moving_points(8);
    let scrub: Vec<Value> = there.iter().chain(there.iter().rev()).cloned().collect();
    assert_eq!(dodged(false, &scrub, 50.0), dodged(false, &there, 50.0), "a scrub back over the spot stays one pass");
}

#[test]
fn airbrush_deposits_dab_by_dab_and_holding_still_deposits_most() {
    // Airbrush is a per-dab deposit, which is what the Brush tool's Build-up has always meant: along
    // a moving stroke its timed steps add dabs, and where two of them land on one spot that pixel
    // gets a fresh pass - a little more than the capped one pass. Holding still for the same time
    // lays every one of those dabs on the same spot, and lightens it far more.
    let (capped, moving, held) = (dodged(false, &moving_points(8), 50.0), dodged(true, &moving_points(8), 50.0), dodged(true, &held_points(8), 50.0));
    assert!(moving > capped, "the timed steps along the path deposit too: {capped:?} -> {moving:?}");
    assert!(held > moving, "and a held spot collects them all: {moving:?} vs {held:?}");
    assert!(moving.iter().all(|&c| c <= 255) && held.iter().all(|&c| c <= 255), "inside the tone curve: {moving:?} {held:?}");
}

#[test]
fn the_held_live_stroke_draws_what_its_command_replays() {
    // The canvas draws the hold dab by dab and its commit takes those pixels; replaying the
    // journaled command paints the same picture.
    let mut s = grey();
    airbrush(&mut s, true);
    let mut live = LiveDab::begin(&s, "paint.dodge", &dodge(&held_points(0), 50.0)).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = |live: &LiveDab| (live.doc.layer(id).unwrap().surface().unwrap().rgba(30, 20)[0] * 255.0).round() as i32;
    let mut each = vec![shown(&live)];
    for k in 1..=5u32 {
        let mut p = StrokePoint::new(30.0, 20.0, 1.0);
        p.time = f64::from(k) * 100.0;
        live.push(&[p]).unwrap();
        each.push(shown(&live));
    }
    for pair in each.windows(2) {
        assert!(pair[1] > pair[0], "the preview keeps lightening while held: {each:?}");
    }
    s.offer_live_dab(live).unwrap();
    let r = s.execute("paint.dodge", dodge(&held_points(5), 50.0)).unwrap();
    assert_eq!(r.get("fromLive").and_then(Value::as_bool), Some(true), "the commit took the preview's pixels");
    let committed = rgb_at(&s, 30, 20)[0];
    assert_eq!(committed, *each.last().unwrap(), "the commit is what the preview showed: {each:?} -> {committed}");
    let mut replay = grey();
    airbrush(&mut replay, true);
    replay.execute("paint.dodge", dodge(&held_points(5), 50.0)).unwrap();
    assert_eq!(rgb_at(&replay, 30, 20), [committed; 3], "and so is a replay of the same command");
}

#[test]
fn one_dodge_history_state_per_held_stroke() {
    for on in [true, false] {
        let mut s = grey();
        airbrush(&mut s, on);
        let before = s.active().unwrap().history.past_len();
        s.execute("paint.dodge", dodge(&held_points(6), 50.0)).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.history.past_len(), before + 1, "one state, Build-up {on}");
        assert_eq!(s.journal.iter().rev().next().unwrap().0, "paint.dodge", "Build-up {on}");
    }
}

#[test]
fn a_bad_or_huge_time_element_is_the_default_and_never_panics() {
    // A point whose time is not a number, negative or absurd paints like a point without a time
    // (the default), as a stroke with no times at all does, and nothing panics.
    let no_times: Vec<Value> = (0..=3).map(|_| json!([30.0, 20.0, 1.0])).collect();
    let one_press = dodged(true, &held_points(0), 50.0);
    assert_eq!(dodged(true, &no_times, 50.0), one_press, "a stroke whose points carry no times is the press it always was");
    for bad in [json!("soon"), json!(null), json!(-5000.0), json!(1e12)] {
        let pts: Vec<_> = (0..=3).map(|k| json!([30.0, 20.0, 1.0, 0.0, 0.0, 0.0, if k == 3 { bad.clone() } else { json!(k as f64 * 100.0) }])).collect();
        let mut s = grey();
        airbrush(&mut s, true);
        let r = s.execute("paint.dodge", dodge(&pts, 50.0));
        assert!(r.is_ok(), "{bad}: {}", r.unwrap_err());
        let got = rgb_at(&s, 30, 20);
        assert!(got[0] >= one_press[0] && got.iter().all(|&c| c <= 255), "{bad}: toned inside the curve: {got:?}");
    }
    // A whole point that is not an array of numbers at all is refused, as today.
    let mut s = grey();
    airbrush(&mut s, true);
    assert!(s.execute("paint.dodge", dodge(&[json!("x")], 50.0)).is_err(), "no point, no stroke");
}

#[test]
fn burn_sponge_blur_and_sharpen_take_no_time_element() {
    // The other retouching tools keep today's points: a time element changes nothing for them, and
    // their held build-up (Burn's is #12665) stays inside the stroke's own limit.
    for (cmd, extra) in
        [("paint.burn", json!({})), ("paint.sponge", json!({"flow": 50})), ("paint.blur", json!({"strength": 50})), ("paint.sharpen", json!({"strength": 50}))]
    {
        let at = |pts: Vec<Value>| {
            let mut s = grey();
            airbrush(&mut s, true);
            let mut p = json!({"points": pts, "size": 40, "hardness": 100});
            if let Some(map) = extra.as_object() {
                for (k, v) in map {
                    p[k] = v.clone();
                }
            }
            s.execute(cmd, p).unwrap();
            rgb_at(&s, 30, 20)
        };
        let held = at(held_points(6));
        let untime = at((0..=6).map(|_| json!([30.0, 20.0, 1.0])).collect());
        assert_eq!(held, untime, "{cmd}: a time element changes nothing");
    }
}
