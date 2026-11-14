//! #11888: Smoothing trails the pointer as far as in the reference application, and Pulled String
//! Mode stops the string's length short of the release point, with or without Catch-up on Stroke
//! End. The drag is the issue's: (100, 199) to (300, 199) in 40 moves 10 ms apart, then released.

use super::*;
use crate::brush::Smoothing;

fn drag() -> Vec<StrokePoint> {
    (0..=40).map(|i| StrokePoint { time: f64::from(i) * 10.0, ..StrokePoint::new(100.0 + f64::from(i) * 5.0, 199.0, 1.0) }).collect()
}

/// How far short of the release point (300) the smoothed stroke ends.
fn short(cfg: &Smoothing) -> f64 {
    let out = smooth_points(cfg, 1.0, &drag());
    300.0 - out.last().map_or(f64::NAN, |p| p.x)
}

fn within(got: f64, want: f64, tolerance: f64) -> bool {
    (got - want).abs() <= want * tolerance
}

#[test]
fn ten_and_fifty_percent_lag_as_measured() {
    let off_end = Smoothing { catch_up_on_end: false, ..Default::default() };
    let ten = short(&Smoothing { amount: 0.1, ..off_end.clone() });
    let fifty = short(&Smoothing { amount: 0.5, ..off_end });
    assert!(within(ten, 19.0, 0.25), "10 %: {ten} px short (reference 19)");
    assert!(within(fifty, 78.0, 0.25), "50 %: {fifty} px short (reference 78)");
}

#[test]
fn catch_up_on_stroke_end_still_reaches_the_release_point() {
    let cfg = Smoothing { amount: 0.5, catch_up_on_end: true, ..Default::default() };
    assert_eq!(short(&cfg), 0.0);
}

#[test]
fn the_pulled_string_stops_its_length_short_either_way() {
    for catch_up_on_end in [false, true] {
        let cfg = Smoothing { amount: 0.5, pulled_string: true, catch_up_on_end, ..Default::default() };
        let s = short(&cfg);
        assert!((s - 150.0).abs() < 1e-9, "catch-up on end {catch_up_on_end}: {s} px short (reference about 150)");
    }
}

#[test]
fn zero_and_disabled_smoothing_paint_through_every_point() {
    let pts = drag();
    for cfg in [Smoothing { amount: 0.0, ..Default::default() }, Smoothing { enabled: false, amount: 0.5, ..Default::default() }] {
        let out = smooth_points(&cfg, 1.0, &pts);
        assert_eq!(out.len(), pts.len(), "{cfg:?}");
        assert!(out.iter().zip(&pts).all(|(a, b)| (a.x, a.y, a.pressure) == (b.x, b.y, b.pressure)), "{cfg:?}");
    }
}

#[test]
fn the_pull_grows_with_less_smoothing_and_stays_a_fraction() {
    let pulls: Vec<f64> = [0.01, 0.1, 0.25, 0.5, 0.75, 1.0].iter().map(|a| smoothing_pull(*a)).collect();
    assert!(pulls.windows(2).all(|w| w[0] > w[1]), "{pulls:?}");
    assert!(pulls.iter().all(|p| *p > 0.0 && *p < 1.0), "{pulls:?}");
    assert_eq!(smoothing_pull(0.0), 1.0, "no time constant: straight to the pointer");
    assert_eq!(smoothing_pull(f64::NAN), 1.0, "a bad amount never makes a NaN position");
}

/// Input without timestamps (a control-client stroke, a script, an old file) was never measured,
/// so each point keeps the pull it always had: `1 - amount` of the way to the pointer. At the
/// default 10 % a two-point stroke still ends 90 % of the way, not a third of it.
#[test]
fn strokes_without_timestamps_keep_their_per_point_pull() {
    let cfg = Smoothing { amount: 0.1, catch_up_on_end: false, ..Default::default() };
    let out = smooth_points(&cfg, 1.0, &[StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(100.0, 0.0, 1.0)]);
    let x = out.last().map_or(f64::NAN, |p| p.x);
    assert!((x - 90.0).abs() < 1e-4, "a 10 % untimed step reached {x}, want 90");
}

/// A timed stroke stays timed: a report with no time since the last one (two events in one
/// millisecond) leaves the brush where it is, instead of jumping it by the untimed per-point pull.
#[test]
fn a_report_with_no_time_passed_does_not_move_a_timed_brush() {
    let cfg = Smoothing { amount: 0.1, catch_up: true, catch_up_on_end: false, ..Default::default() };
    let at = |x: f64, time: f64| StrokePoint { time, ..StrokePoint::new(x, 0.0, 1.0) };
    let timed = smooth_points(&cfg, 1.0, &[at(0.0, 0.0), at(100.0, 10.0)]);
    let twice = smooth_points(&cfg, 1.0, &[at(0.0, 0.0), at(100.0, 10.0), at(100.0, 10.0)]);
    let (before, after) = (timed.last().map_or(f64::NAN, |p| p.x), twice.last().map_or(f64::NAN, |p| p.x));
    assert!(before > 0.0 && before < 90.0, "the timed move lags: {before}");
    assert_eq!(after, before, "no time passed, so the brush stays");
}
