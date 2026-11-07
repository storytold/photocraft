//! Reconstruct a brush path before spacing dabs along it. Filtering pointer positions alone
//! still leaves straight chords between sparse samples of a fast curve.

use crate::StrokePoint;
use crate::dynamics::lerp_pt;

const FLATNESS: f64 = 0.125;
const MAX_DEPTH: u32 = 10;
// Very large jumps are straight lines, not a request to allocate an enormous curve.
const MAX_SPAN: f64 = 16_384.0;

/// One sample of lookahead, independent of frame/batch boundaries. The renderer can finish a
/// clone to show a provisional tail without committing that tail to the stroke's coverage.
#[derive(Clone, Debug, Default)]
pub(crate) struct StrokeInterpolator {
    before: Option<StrokePoint>,
    start: Option<StrokePoint>,
    end: Option<StrokePoint>,
}

impl StrokeInterpolator {
    pub fn push(&mut self, p: StrokePoint, out: &mut Vec<StrokePoint>) {
        let Some(start) = self.start else {
            self.start = Some(p);
            out.push(p);
            return;
        };
        let last = self.end.unwrap_or(start);
        if p.x == last.x && p.y == last.y {
            // A pause/pose-only change must keep its time and sensors (airbrush and pressure).
            // It also ends a run, so resuming doesn't round the corner at a deliberate stop.
            self.finish(out);
            self.start = Some(p);
            out.push(p);
        } else if let Some(end) = self.end.replace(p) {
            segment(self.before, start, end, Some(p), out);
            self.before = Some(start);
            self.start = Some(end);
        }
    }

    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        if let (Some(start), Some(end)) = (self.start, self.end.take()) {
            segment(self.before, start, end, None, out);
        }
        self.before = None;
        self.start = None;
    }
}

fn direction(a: StrokePoint, b: StrokePoint) -> Option<([f64; 2], f64)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    (len.is_finite() && len > 1e-9 && len <= MAX_SPAN).then(|| ([dx / len, dy / len], len))
}

/// Angle bisector with shorter handles when sample distances differ substantially. A turn of
/// 90 degrees or more is a corner: use each segment's own direction instead of rounding it.
fn tangent(a: StrokePoint, b: StrokePoint, c: StrokePoint, span: f64) -> Option<[f64; 2]> {
    let (u, left) = direction(a, b)?;
    let (v, right) = direction(b, c)?;
    if u[0] * v[0] + u[1] * v[1] <= 1e-6 {
        return None;
    }
    let sum = [u[0] + v[0], u[1] + v[1]];
    let scale = span * (2.0 * left.min(right) / (left + right)) / sum[0].hypot(sum[1]);
    Some([sum[0] * scale, sum[1] * scale])
}

fn reflect(t: [f64; 2], axis: [f64; 2]) -> [f64; 2] {
    let dot = t[0] * axis[0] + t[1] * axis[1];
    [2.0 * dot * axis[0] - t[0], 2.0 * dot * axis[1] - t[1]]
}

fn segment(before: Option<StrokePoint>, a: StrokePoint, b: StrokePoint, after: Option<StrokePoint>, out: &mut Vec<StrokePoint>) {
    let Some((axis, len)) = direction(a, b) else {
        out.push(b);
        return;
    };
    let chord = [b.x - a.x, b.y - a.y];
    let ta = match before {
        Some(p) => tangent(p, a, b, len),
        None => after.and_then(|p| tangent(a, b, p, len)).map(|t| reflect(t, axis)),
    }
    .unwrap_or(chord);
    let tb = match after {
        Some(p) => tangent(a, b, p, len),
        None => before.and_then(|p| tangent(p, a, b, len)).map(|t| reflect(t, axis)),
    }
    .unwrap_or(chord);
    flatten(a, [a.x + ta[0] / 3.0, a.y + ta[1] / 3.0], [b.x - tb[0] / 3.0, b.y - tb[1] / 3.0], b, 0, out);
}

fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn flatten(a: StrokePoint, c1: [f64; 2], c2: [f64; 2], b: StrokePoint, depth: u32, out: &mut Vec<StrokePoint>) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    let distance = |c: [f64; 2]| ((c[0] - a.x) * dy - (c[1] - a.y) * dx).abs();
    if depth >= MAX_DEPTH || len <= 1e-9 || (distance(c1).max(distance(c2)) <= FLATNESS * len) {
        out.push(b);
        return;
    }
    let left = midpoint([a.x, a.y], c1);
    let center = midpoint(c1, c2);
    let right = midpoint(c2, [b.x, b.y]);
    let lc = midpoint(left, center);
    let rc = midpoint(center, right);
    let xy = midpoint(lc, rc);
    let mut mid = lerp_pt(&a, &b, 0.5);
    (mid.x, mid.y) = (xy[0], xy[1]);
    flatten(a, left, lc, mid, depth + 1, out);
    flatten(mid, rc, right, b, depth + 1, out);
}
