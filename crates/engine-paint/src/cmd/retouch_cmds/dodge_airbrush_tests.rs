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

/// The same params with 10 % Smoothing - what `paint_mouse::first_smoothing` puts on a tool's
/// starting brush, so this is what a real held Dodge pointer runs through: while the smoother
/// catches up, the positions of a held spot are not bit-identical.
fn smoothed(mut p: Value) -> Value {
    p["smoothing"] = json!(10.0);
    p
}

/// The centre pixel of a stroke with the tool's own Smoothing on.
fn grey_smoothed(on: bool, points: &[Value], exposure: f64) -> i32 {
    let mut s = grey();
    airbrush(&mut s, on);
    s.execute("paint.dodge", smoothed(dodge(points, exposure))).unwrap();
    rgb_at(&s, 30, 20)[0]
}

#[test]
fn a_held_airbrush_dodge_builds_up_under_the_tool_s_own_smoothing() {
    // The Dodge tool's starting brush has Smoothing at 10 %, so the held dabs land a fraction of a
    // pixel apart: the build-up has to follow the spot, not exact coordinates.
    let each: Vec<i32> = (0..=6).map(|holds| grey_smoothed(true, &held_points(holds), 50.0)).collect();
    for pair in each.windows(2) {
        assert!(pair[1] > pair[0], "held dabs keep lightening with the bar's own 10 % Smoothing: {each:?}");
    }
    assert!(each.last().unwrap() <= &255, "{each:?}");
}

#[test]
fn a_smoothed_held_stroke_stays_one_pass_with_airbrush_off() {
    let one = grey_smoothed(false, &held_points(0), 50.0);
    for holds in [4, 16] {
        assert_eq!(grey_smoothed(false, &held_points(holds), 50.0), one, "off, {holds} smoothed repeats add nothing: {one}");
    }
    // And a smoothed drag is the one pass it has always been.
    let mut s = grey();
    airbrush(&mut s, false);
    s.execute("paint.dodge", smoothed(dodge(&moving_points(8), 50.0))).unwrap();
    assert!(rgb_at(&s, 30, 20)[0] > 128, "the drag tones the spot it crosses");
}

#[test]
fn only_dodge_asks_the_canvas_to_repeat_a_held_point() {
    // `wants_time` is the canvas's whole signal for "keep this stroke's clock running", and the same
    // live-stroke type backs the Color Replacement, Background Eraser and Pattern Stamp previews.
    let mut s = grey();
    airbrush(&mut s, true);
    for cmd in ["paint.dodge", "paint.burn", "paint.sponge", "paint.blur", "paint.sharpen", "paint.smudge"] {
        let live = LiveDab::begin(&s, cmd, &dodge(&held_points(0), 50.0)).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        assert_eq!(live.wants_time(), cmd == "paint.dodge", "{cmd} with Build-up on");
    }
}

/// A drag of `steps` points 6 px apart, then `holds` repeats of the last point 100 ms apart: what
/// a pointer that moved and then stopped does, with the tool's own Smoothing still catching up.
fn drag_then_hold(steps: usize, holds: usize) -> Vec<Value> {
    let mut pts = moving_points(steps);
    let (x, y) = (12.0 + 6.0 * steps as f64, 20.0);
    for k in 1..=holds {
        pts.push(json!([x, y, 1.0, 0.0, 0.0, 0.0, (steps + k) as f64 * 100.0]));
    }
    pts
}

#[test]
fn a_hold_after_a_drag_builds_up_under_the_tool_s_own_smoothing() {
    // The pointer stops at one spot after a drag while its Smoothing is still catching up, so the
    // held dabs land a fraction of a pixel apart: each of them is still a dab on the spot the
    // pointer is holding. (Exposure 10 %, the issue's fixture: at 50 % the curve is already at its
    // ceiling after three passes and the steps round away.)
    let each: Vec<i32> = [0usize, 2, 4, 6, 8].map(|holds| grey_smoothed(true, &drag_then_hold(6, holds), 10.0)).to_vec();
    for pair in each.windows(2) {
        assert!(pair[1] > pair[0], "the hold after a drag keeps lightening: {each:?}");
    }
    // The same hold with the toggle off stays where the drag left it.
    let off: Vec<i32> = [0usize, 8].map(|holds| grey_smoothed(false, &drag_then_hold(6, holds), 10.0)).to_vec();
    assert_eq!(off[0], off[1], "off, the hold after the drag adds nothing: {off:?}");
}

#[test]
fn airbrush_on_deposits_at_every_dab_and_off_stops_at_the_cap() {
    // With Build-up on, every dab that finds the spot already covered still deposits - along a drag
    // as well as under a held pointer, which is what an airbrush does. With it off the stroke's cap
    // holds however many dabs cross the same pixel.
    let (off4, on4, off8, on8) = (
        grey_smoothed(false, &moving_points(4), 10.0),
        grey_smoothed(true, &moving_points(4), 10.0),
        grey_smoothed(false, &moving_points(8), 10.0),
        grey_smoothed(true, &moving_points(8), 10.0),
    );
    assert!(off4 > 128, "the drag tones the spot it crosses: {off4}");
    assert_eq!(off4, off8, "off, twice the dabs over the spot are still one pass: {off4} vs {off8}");
    assert!(on4 > off4 && on8 > on4, "on, each further dab adds its deposit: {off4} -> {on4} -> {on8}");
    assert!(on8 <= 255, "and the tone curve bounds it: {on8}");
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
fn more_held_dabs_deposit_more_at_the_same_exposure() {
    // The build-up follows the dabs on the spot: a longer hold lightens more, and any hold goes
    // past the one pass - still inside the tone curve. (At Exposure 10 %, the issue's fixture: at
    // 50 % the curve is at its ceiling after a few passes and the 8-bit steps round away.)
    let (off, short, long) = (grey_dodged(false, &held_points(16), 10.0), grey_dodged(true, &held_points(4), 10.0), grey_dodged(true, &held_points(16), 10.0));
    assert!(off > 128, "the press itself tones the spot: {off}");
    assert!(short > off, "a held spot goes past the stroke's one pass: {off} -> {short}");
    assert!(long > short, "and the longer it is held the more it lightens: {short} -> {long}");
    assert!(long <= 255, "inside the tone curve: {long}");
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
        assert_eq!(s.journal.iter().next_back().unwrap().0, "paint.dodge", "Build-up {on}");
    }
}

#[test]
fn a_bad_or_huge_time_element_is_the_default_or_one_bounded_step_and_never_panics() {
    // A time is read as a number or taken as 0, the default a point without a time has. A
    // non-number, a null, a NaN and a negative time therefore add no dab at all: the stroke is the
    // one whose last point carries no time. An absurd time runs into the Brush tool's own guard for
    // a stalled timestamp (dynamics.rs drops the backlog and lays one dab), so at most one normal
    // interval lands - never a backlog, never a panic.
    let with_last_time = |bad: Value| -> Vec<Value> {
        (0..=3).map(|k| json!([30.0, 20.0, 1.0, 0.0, 0.0, 0.0, if k == 3 { bad.clone() } else { json!(k as f64 * 100.0) }])).collect()
    };
    let three_points = dodged(true, &held_points(2), 50.0);
    for bad in [json!("soon"), json!(null), json!(-5000.0), json!(f64::NAN)] {
        assert_eq!(dodged(true, &with_last_time(bad.clone()), 50.0), three_points, "{bad}: no dab from that point, as a point without a time");
    }
    let one_interval = dodged(true, &held_points(3), 50.0);
    for bad in [json!(1e12), json!(1e30)] {
        let got = dodged(true, &with_last_time(bad.clone()), 50.0);
        assert_eq!(got, one_interval, "{bad}: the stalled-timestamp guard lays one dab, not a backlog");
        assert!(got.iter().all(|&c| c <= 255), "{bad}: inside the tone curve: {got:?}");
    }
    let no_times: Vec<Value> = (0..=3).map(|_| json!([30.0, 20.0, 1.0])).collect();
    assert_eq!(dodged(true, &no_times, 50.0), dodged(true, &held_points(0), 50.0), "points that carry no times at all are the stroke they always were");
    let mut s = grey();
    airbrush(&mut s, true);
    assert!(s.execute("paint.dodge", dodge(&[json!("x")], 50.0)).is_err(), "a point that is not an array of numbers is refused");
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
