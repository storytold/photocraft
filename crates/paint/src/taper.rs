//! Stroke taper: thin, pointed stroke ends instead of a blob where the pen lands or lifts.
//!
//! A tablet reports its pressure a little late: the first samples of a stroke often already carry
//! most of the pressure, and the last ones before the lift still do. With Size or Opacity on Pen
//! Pressure the stroke then starts and ends in a round cap of nearly full width. The taper scales
//! the pressure down near each end, by arc length along the painted (smoothed) path:
//!
//! - **Start** is applied as the points arrive ([`StartTaper`], used by the dab generator), so the
//!   live stroke shows it at once and the commit gives the same pixels.
//! - **End** needs to know where the stroke ends, so it can only be applied to the whole stroke.
//!   [`for_commit`] does it when the stroke is committed: the stroke is rendered again with the
//!   tapered points, and its end narrows to a point at pen-up.
//!
//! Lengths are document pixels. A stroke shorter than both tapers shrinks them in proportion, so a
//! short dash still reaches full pressure in the middle. Brushes without pressure dynamics are
//! unaffected (the taper only changes the pressure the dynamics read).

use serde::{Deserialize, Serialize};

use crate::StrokePoint;
use crate::brush::BrushSettings;

/// The longest taper, in document pixels.
pub const MAX_TAPER_PX: f32 = 2000.0;

/// Taper lengths in document pixels; 0 switches that end off (the default: no taper).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
#[serde(default, rename_all = "camelCase")]
pub struct Taper {
    /// Distance over which the stroke's start grows from nothing to the pen's pressure.
    pub start: f32,
    /// Distance over which the stroke's end shrinks from the pen's pressure to nothing.
    pub end: f32,
}

impl Taper {
    /// Lengths clamped to `0..=MAX_TAPER_PX`; NaN and negative lengths are 0 (off).
    pub fn sanitized(&self) -> Self {
        let clean = |v: f32| if v > 0.0 { v.min(MAX_TAPER_PX) } else { 0.0 };
        Self { start: clean(self.start), end: clean(self.end) }
    }

    pub fn is_off(&self) -> bool {
        let t = self.sanitized();
        t.start == 0.0 && t.end == 0.0
    }
}

/// The pressure factor `d` pixels from a stroke end, for a taper of length `len`: 0 at the end,
/// 1 from `len` on. `t·(2 − t)` starts with a constant slope, so the end is a point rather than a
/// rounded bulb, and meets full pressure with zero slope, so there is no visible joint.
pub fn ramp(d: f64, len: f32) -> f32 {
    if !(len > 0.0) {
        return 1.0;
    }
    let t = d / f64::from(len);
    if !(t > 0.0) {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    (t * (2.0 - t)) as f32
}

/// Incremental start taper: feed the stroke's points in order.
#[derive(Clone, Debug, Default)]
pub struct StartTaper {
    len: f32,
    travelled: f64,
    last: Option<(f64, f64)>,
}

impl StartTaper {
    pub fn new(len: f32) -> Self {
        Self { len: Taper { start: len, end: 0.0 }.sanitized().start, travelled: 0.0, last: None }
    }

    /// Is there anything to do? Off, points pass through unchanged.
    pub fn active(&self) -> bool {
        self.len > 0.0
    }

    /// `p` with its pressure scaled by its distance from the stroke's first point.
    pub fn apply(&mut self, mut p: StrokePoint) -> StrokePoint {
        if !self.active() || !(p.x.is_finite() && p.y.is_finite()) {
            return p;
        }
        if let Some((x, y)) = self.last {
            self.travelled += (p.x - x).hypot(p.y - y);
        }
        self.last = Some((p.x, p.y));
        p.pressure *= ramp(self.travelled, self.len);
        p
    }
}

/// Distance along `pts` from the first point to each point (non-finite points add nothing).
fn arc_lengths(pts: &[StrokePoint]) -> Vec<f64> {
    let mut out = Vec::with_capacity(pts.len());
    let (mut acc, mut last): (f64, Option<(f64, f64)>) = (0.0, None);
    for p in pts {
        if p.x.is_finite() && p.y.is_finite() {
            if let Some((x, y)) = last {
                acc += (p.x - x).hypot(p.y - y);
            }
            last = Some((p.x, p.y));
        }
        out.push(acc);
    }
    out
}

/// The taper lengths `t` shrunk to fit a stroke `total` pixels long: both in proportion when
/// they would overlap.
pub fn fit(t: &Taper, total: f64) -> Taper {
    let t = t.sanitized();
    let sum = f64::from(t.start) + f64::from(t.end);
    if sum <= total || sum == 0.0 {
        return t;
    }
    let k = (total.max(0.0) / sum) as f32;
    Taper { start: t.start * k, end: t.end * k }
}

/// Scale the pressure of a whole stroke by both tapers (lengths used as given).
pub fn apply(pts: &mut [StrokePoint], t: &Taper) {
    let t = t.sanitized();
    if t.start == 0.0 && t.end == 0.0 {
        return;
    }
    let lens = arc_lengths(pts);
    let total = lens.last().copied().unwrap_or(0.0);
    for (p, s) in pts.iter_mut().zip(lens) {
        p.pressure *= ramp(s, t.start) * ramp(total - s, t.end);
    }
}

/// Prepare a stroke for its commit. With an end taper the points are smoothed here (with the
/// brush's own smoothing, `zoom` as for the stroke, after Smooth Curves), both tapers fitted to
/// the smoothed path and applied, and the returned brush has smoothing, Smooth Curves and the
/// start taper switched off so the renderer doesn't apply them twice. The bool says whether anything changed: then the live stroke's pixels
/// (rendered without the end taper) can't stand in for the commit.
///
/// Without an end taper nothing changes: the renderer applies the start taper as it goes, exactly
/// as the live stroke did.
pub fn for_commit(mut brush: BrushSettings, pts: Vec<StrokePoint>, zoom: f32) -> (BrushSettings, Vec<StrokePoint>, bool) {
    let t = brush.taper.sanitized();
    if t.end == 0.0 || pts.len() < 2 {
        return (brush, pts, false);
    }
    let input = if brush.smooth_curves { crate::stroke_curve::fit_points(&pts) } else { pts };
    let mut smoothed = crate::dynamics::smooth_points(&brush.smoothing, zoom, &input);
    let total = arc_lengths(&smoothed).last().copied().unwrap_or(0.0);
    apply(&mut smoothed, &fit(&t, total));
    brush.smoothing.enabled = false;
    brush.smooth_curves = false;
    brush.taper = Taper::default();
    (brush, smoothed, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize, step: f64, pressure: f32) -> Vec<StrokePoint> {
        (0..n).map(|i| StrokePoint::new(i as f64 * step, 0.0, pressure)).collect()
    }

    #[test]
    fn ramp_is_nothing_at_the_end_and_full_past_the_length() {
        assert_eq!(ramp(0.0, 10.0), 0.0);
        assert_eq!(ramp(10.0, 10.0), 1.0);
        assert_eq!(ramp(50.0, 10.0), 1.0);
        // Halfway in, the pressure is partway up, and past a straight ramp: the join to full pressure is smooth.
        let half = ramp(5.0, 10.0);
        assert!(half > 0.5 && half < 1.0, "{half}");
        // Off, or a broken length: full pressure everywhere.
        assert_eq!(ramp(0.0, 0.0), 1.0);
        assert_eq!(ramp(0.0, f32::NAN), 1.0);
        assert_eq!(ramp(f64::NAN, 10.0), 0.0);
    }

    #[test]
    fn ramp_rises_monotonically() {
        let mut prev = 0.0;
        for i in 0..=100 {
            let v = ramp(f64::from(i) * 0.1, 10.0);
            assert!(v >= prev, "{v} < {prev} at {i}");
            prev = v;
        }
    }

    #[test]
    fn start_taper_matches_the_whole_stroke_taper() {
        let pts = line(40, 1.0, 0.8);
        let mut whole = pts.clone();
        apply(&mut whole, &Taper { start: 15.0, end: 0.0 });
        let mut inc = StartTaper::new(15.0);
        let live: Vec<_> = pts.iter().map(|p| inc.apply(*p)).collect();
        assert_eq!(whole, live);
        assert_eq!(live[0].pressure, 0.0);
        assert_eq!(live[39].pressure, 0.8);
    }

    #[test]
    fn end_taper_thins_the_last_points_to_nothing() {
        let mut pts = line(40, 1.0, 1.0);
        apply(&mut pts, &Taper { start: 0.0, end: 10.0 });
        assert_eq!(pts[39].pressure, 0.0);
        assert!(pts[35].pressure < pts[30].pressure);
        assert_eq!(pts[20].pressure, 1.0);
        assert_eq!(pts[0].pressure, 1.0);
    }

    #[test]
    fn short_strokes_shrink_both_tapers_in_proportion() {
        let asked = Taper { start: 30.0, end: 10.0 };
        let t = fit(&asked, 20.0);
        assert!(t.start + t.end <= 20.0 + 1e-4, "the tapers fit the stroke: {t:?}");
        assert!((t.start / t.end - asked.start / asked.end).abs() < 1e-4, "in proportion: {t:?}");
        assert_eq!(fit(&Taper { start: 5.0, end: 5.0 }, 100.0), Taper { start: 5.0, end: 5.0 }, "a long stroke keeps them");
    }

    #[test]
    fn bad_lengths_are_off_and_huge_ones_are_bounded() {
        assert!(Taper { start: f32::NAN, end: -3.0 }.is_off());
        assert!(Taper { start: 1e9, end: 0.0 }.sanitized().start <= MAX_TAPER_PX);
    }

    #[test]
    fn commit_without_end_taper_changes_nothing() {
        let brush = BrushSettings { taper: Taper { start: 10.0, end: 0.0 }, ..Default::default() };
        let pts = line(10, 2.0, 1.0);
        let (b, p, changed) = for_commit(brush.clone(), pts.clone(), 1.0);
        assert!(!changed);
        assert_eq!((b, p), (brush, pts));
    }

    #[test]
    fn commit_with_end_taper_bakes_both_ends_into_the_points() {
        let brush = BrushSettings { taper: Taper { start: 10.0, end: 10.0 }, ..Default::default() };
        let (b, p, changed) = for_commit(brush, line(50, 1.0, 1.0), 1.0);
        assert!(changed);
        assert!(b.taper.is_off() && !b.smoothing.enabled);
        assert_eq!(p.first().map(|q| q.pressure), Some(0.0));
        assert_eq!(p.last().map(|q| q.pressure), Some(0.0));
        assert_eq!(p[25].pressure, 1.0);
    }

    #[test]
    fn a_brush_saved_without_a_taper_loads_with_none() {
        let b: BrushSettings = serde_json::from_str("{}").unwrap_or_default();
        assert!(b.taper.is_off());
    }
}
