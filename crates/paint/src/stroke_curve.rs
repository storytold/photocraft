//! Smooth Curves: the stroke follows a curve through its input points instead of straight segments.
//!
//! A fast stroke gets few input points (a tablet reports ~150–200 a second; an arc drawn in a
//! tenth of a second has a couple of dozen), and joined by straight lines a curve shows as a
//! polygon. [`CurveFit`] passes a centripetal Catmull–Rom spline through the points and samples it
//! densely. The spline goes through every input point exactly; the centripetal form doesn't loop
//! or overshoot at sharp turns. Pressure, tilt, wheel and time are interpolated linearly between
//! the two points of each segment; barrel rotation the shorter way round (350° to 10° passes 0°).
//!
//! A segment needs the point after it, so the live stroke runs one input point behind the pen
//! (a few milliseconds); [`CurveFit::finish`] draws the last segment at pen-up. A point at the
//! same position as the one before (a held pen: airbrush build-up, smoothing catch-up) ends the
//! curve there and passes through unchanged, so time still passes for the build-up.

use crate::StrokePoint;
use crate::dynamics::lerp_pt;

/// Spacing of the sampled curve, in document pixels (the dab walker spaces dabs along it).
const SAMPLE_PX: f64 = 1.5;
/// At most this many samples per segment.
const MAX_SAMPLES: usize = 256;

/// Incremental curve fit: feed points in order, then [`finish`](Self::finish).
#[derive(Clone, Debug, Default)]
pub struct CurveFit {
    /// The last (up to) three points: the segment `hist[1] → hist[2]` waits for the next point.
    hist: Vec<StrokePoint>,
}

fn finite(p: &StrokePoint) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn same_pos(a: &StrokePoint, b: &StrokePoint) -> bool {
    a.x == b.x && a.y == b.y
}

/// The point mirrored through `about`: a stand-in neighbour at a stroke end with one neighbour.
fn mirror(about: &StrokePoint, p: &StrokePoint) -> StrokePoint {
    StrokePoint { x: 2.0 * about.x - p.x, y: 2.0 * about.y - p.y, ..*about }
}

/// A stand-in neighbour before `a` on the parabola through `a`, `b`, `c` (`3a − 3b + c`): the
/// end segment keeps the curve's bend instead of straightening out.
fn extrapolate(a: &StrokePoint, b: &StrokePoint, c: &StrokePoint) -> StrokePoint {
    StrokePoint { x: 3.0 * a.x - 3.0 * b.x + c.x, y: 3.0 * a.y - 3.0 * b.y + c.y, ..*a }
}

/// `from` turned toward `to` by the fraction `f`, the shorter way round, in `0..360`.
fn turn_toward(from: f32, to: f32, f: f32) -> f32 {
    let d = (to - from + 540.0).rem_euclid(360.0) - 180.0;
    (from + d * f).rem_euclid(360.0)
}

/// Knot step of the centripetal parametrisation (√distance), never 0.
fn knot(a: &StrokePoint, b: &StrokePoint) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).sqrt().max(1e-6)
}

/// Samples of the segment `p1 → p2` (without `p1`, ending exactly at `p2`), with neighbours `p0`, `p3`.
fn segment(p0: &StrokePoint, p1: &StrokePoint, p2: &StrokePoint, p3: &StrokePoint, out: &mut Vec<StrokePoint>) {
    let len = (p2.x - p1.x).hypot(p2.y - p1.y);
    let n = ((len / SAMPLE_PX).ceil() as usize).clamp(1, MAX_SAMPLES);
    let t0 = 0.0;
    let t1 = t0 + knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let mix = |a: (f64, f64), b: (f64, f64), ta: f64, tb: f64, t: f64| {
        let (wa, wb) = ((tb - t) / (tb - ta), (t - ta) / (tb - ta));
        (a.0 * wa + b.0 * wb, a.1 * wa + b.1 * wb)
    };
    let (q0, q1, q2, q3) = ((p0.x, p0.y), (p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y));
    for i in 1..=n {
        let u = i as f64 / n as f64;
        let mut q = if i == n { *p2 } else { lerp_pt(p1, p2, u) };
        if i < n {
            q.rotation = turn_toward(p1.rotation, p2.rotation, u as f32);
            // Barry–Goldman pyramid.
            let t = t1 + (t2 - t1) * u;
            let a1 = mix(q0, q1, t0, t1, t);
            let a2 = mix(q1, q2, t1, t2, t);
            let a3 = mix(q2, q3, t2, t3, t);
            let b1 = mix(a1, a2, t0, t2, t);
            let b2 = mix(a2, a3, t1, t3, t);
            let (x, y) = mix(b1, b2, t1, t2, t);
            if x.is_finite() && y.is_finite() {
                (q.x, q.y) = (x, y);
            }
        }
        out.push(q);
    }
}

impl CurveFit {
    pub fn new() -> Self {
        Self::default()
    }

    /// The waiting segment `hist[n-2] → hist[n-1]`, drawn with `next` as the point after it.
    fn draw_pending(&self, next: StrokePoint, out: &mut Vec<StrokePoint>) {
        let n = self.hist.len();
        if n < 2 {
            return;
        }
        let (p1, p2) = (self.hist[n - 2], self.hist[n - 1]);
        let p0 = if n >= 3 { self.hist[n - 3] } else { extrapolate(&p1, &p2, &next) };
        segment(&p0, &p1, &p2, &next, out);
    }

    /// Feed one point; appends the curve samples that are ready.
    pub fn push(&mut self, p: StrokePoint, out: &mut Vec<StrokePoint>) {
        if !finite(&p) {
            return;
        }
        match self.hist.last() {
            None => out.push(p),
            Some(last) if same_pos(last, &p) => {
                // The pen rests: finish the curve here, let the point through (its time matters
                // to build-up) and start a new curve from it.
                self.finish(out);
                out.push(p);
            }
            Some(_) => self.draw_pending(p, out),
        }
        self.hist.push(p);
        if self.hist.len() > 3 {
            self.hist.remove(0);
        }
    }

    /// End of stroke (or a rest): draw the last waiting segment.
    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        let n = self.hist.len();
        if n >= 3 {
            let end = extrapolate(&self.hist[n - 1], &self.hist[n - 2], &self.hist[n - 3]);
            self.draw_pending(end, out);
        } else if n == 2 {
            let end = mirror(&self.hist[1], &self.hist[0]);
            self.draw_pending(end, out);
        }
        self.hist.clear();
    }
}

/// Fit a whole point list (what the live stroke draws, pushed at once).
pub fn fit_points(pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut c = CurveFit::new();
    let mut out = Vec::with_capacity(pts.len() * 8);
    for p in pts {
        c.push(*p, &mut out);
    }
    c.finish(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> StrokePoint {
        StrokePoint::new(x, y, 1.0)
    }

    /// Points on a circle of radius `r`, `n` of them over a quarter turn.
    fn arc(r: f64, n: usize) -> Vec<StrokePoint> {
        (0..=n).map(|i| {
            let a = std::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            pt(r * a.cos(), r * a.sin())
        }).collect()
    }

    #[test]
    fn passes_through_every_input_point() {
        let input = arc(300.0, 6);
        let out = fit_points(&input);
        for p in &input {
            assert!(out.iter().any(|q| q.x == p.x && q.y == p.y), "{p:?} missing");
        }
        assert_eq!(out.first().map(|q| (q.x, q.y)), Some((input[0].x, input[0].y)));
        assert_eq!(out.last().map(|q| (q.x, q.y)), Some((input[6].x, input[6].y)));
    }

    #[test]
    fn a_sparse_arc_comes_out_round() {
        // Six segments of ~78 px: joined straight, their midpoints sit ~2.6 px inside the circle.
        let r = 300.0;
        let out = fit_points(&arc(r, 6));
        let worst = out.iter().map(|q| (q.x.hypot(q.y) - r).abs()).fold(0.0, f64::max);
        assert!(worst < 0.5, "max deviation from the circle {worst}");
        // Densely sampled.
        assert!(out.windows(2).all(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y) <= SAMPLE_PX + 1e-9));
    }

    #[test]
    fn whole_and_incremental_fits_agree() {
        let input = arc(200.0, 9);
        let whole = fit_points(&input);
        let mut c = CurveFit::new();
        let mut inc = Vec::new();
        for chunk in input.chunks(2) {
            for p in chunk {
                c.push(*p, &mut inc);
            }
        }
        c.finish(&mut inc);
        assert_eq!(whole, inc);
    }

    #[test]
    fn a_straight_line_stays_straight_and_pressure_is_interpolated() {
        let input = vec![StrokePoint::new(0.0, 0.0, 0.0), StrokePoint::new(30.0, 0.0, 1.0), StrokePoint::new(60.0, 0.0, 1.0)];
        let out = fit_points(&input);
        assert!(out.iter().all(|q| q.y.abs() < 1e-9 && q.x >= 0.0 && q.x <= 60.0));
        let mid = out.iter().find(|q| (q.x - 15.0).abs() < 0.8).map(|q| q.pressure).unwrap_or(-1.0);
        assert!(mid > 0.0 && mid < 1.0, "between the pressures of its two points: {mid}");
    }

    #[test]
    fn barrel_rotation_turns_the_shorter_way_round() {
        let mut a = pt(0.0, 0.0);
        let mut b = pt(40.0, 0.0);
        (a.rotation, b.rotation) = (350.0, 10.0);
        let out = fit_points(&[a, b]);
        // Every sample stays within the 20° between them, across 0°, never near 180°.
        assert!(out.iter().all(|q| q.rotation >= 350.0 || q.rotation <= 10.0), "{:?}", out.iter().map(|q| q.rotation).collect::<Vec<_>>());
    }

    #[test]
    fn a_resting_pen_passes_through_and_single_points_stay_single() {
        let mut held = vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 0.0), pt(10.0, 0.0)];
        held[2].time = 5.0;
        held[3].time = 9.0;
        let out = fit_points(&held);
        let rests: Vec<f64> = out.iter().filter(|q| q.x == 10.0).map(|q| q.time).collect();
        assert!(rests.contains(&5.0) && rests.contains(&9.0), "{rests:?}");
        assert_eq!(fit_points(&[pt(3.0, 4.0)]).len(), 1);
        assert!(fit_points(&[]).is_empty());
        assert_eq!(fit_points(&[pt(f64::NAN, 0.0), pt(1.0, 1.0)]).len(), 1);
    }
}
