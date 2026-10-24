//! Arrowheads at the open ends of a stroke. A head is a filled outline in the stroke's paint;
//! the line is shortened so it ends inside the head (no butt or cap poking past the tip) and so
//! a dashed or dotted line still gets a solid head. Heads follow the chord over their own
//! length rather than the last flattened segment, so a curve's head points where it arrives.

use photocraft_doc::Arrowhead;

use crate::flatten::Polyline;

type P = (f64, f64);

/// Heads may use at most this share of a line's length together, so a short line keeps a
/// visible shaft and its heads shrink instead of overlapping.
const MAX_HEAD_SHARE: f64 = 0.9;

/// Head width and length limits in stroke widths, as `Arrowhead::validate` accepts them: a file can
/// hold any number, and the renderer bounds its own work.
const MIN_SIZE: f64 = 0.5;
const MAX_SIZE: f64 = 50.0;

/// A head size from a file, within the limits (a non-finite one is the smallest).
fn size(v: f32) -> f64 {
    let v = f64::from(v);
    if v.is_finite() { v.clamp(MIN_SIZE, MAX_SIZE) } else { MIN_SIZE }
}

/// Shortens the open polylines of `lines` for their heads and returns the head outlines
/// (positively oriented, for the non-zero rule). `width` is the stroke width in px.
pub fn arrowheads(lines: &mut [Polyline], start: Option<&Arrowhead>, end: Option<&Arrowhead>, width: f64) -> Vec<Vec<P>> {
    let mut out = Vec::new();
    if (start.is_none() && end.is_none()) || !(width.is_finite() && width > 0.0) {
        return out;
    }
    for pl in lines.iter_mut() {
        if pl.closed || pl.pts.len() < 2 || pl.pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            continue;
        }
        let total = length(&pl.pts);
        if total <= 1e-9 {
            continue;
        }
        let wanted = |h: Option<&Arrowhead>| h.map_or(0.0, |h| size(h.length) * width);
        let (ls, le) = (wanted(start), wanted(end));
        let k = if ls + le > total * MAX_HEAD_SHARE { total * MAX_HEAD_SHARE / (ls + le) } else { 1.0 };
        let mut cut_start = 0.0;
        let mut cut_end = 0.0;
        if let Some(h) = end {
            let (poly, cut) = head(&pl.pts, h, le * k, width * k);
            out.extend(poly);
            cut_end = cut;
        }
        if let Some(h) = start {
            let rev: Vec<P> = pl.pts.iter().rev().copied().collect();
            let (poly, cut) = head(&rev, h, ls * k, width * k);
            out.extend(poly);
            cut_start = cut;
        }
        trim(pl, cut_start, cut_end);
    }
    out
}

/// The head at the last point of `pts` (`len` px long, for a stroke `width` px wide) and how
/// much of the line to cut off under it.
fn head(pts: &[P], h: &Arrowhead, len: f64, width: f64) -> (Option<Vec<P>>, f64) {
    let Some(&tip) = pts.last() else { return (None, 0.0) };
    if len <= 1e-9 {
        return (None, 0.0);
    }
    let back = point_back(pts, len);
    let d = norm(sub(tip, back));
    if d == (0.0, 0.0) {
        return (None, 0.0);
    }
    let n = (-d.1, d.0);
    let half = size(h.width) * width * 0.5;
    let c = if h.concavity.is_finite() { f64::from(h.concavity).clamp(-0.5, 0.5) } else { 0.0 };
    let base = sub(tip, mul(d, len));
    let notch = sub(tip, mul(d, len * (1.0 - c)));
    let poly = vec![tip, add(base, mul(n, half)), notch, sub(base, mul(n, half))];
    // End the line where the head is at least as wide as the line, leaving room for a round or
    // square cap, and before the notch.
    let covered = if half > 1e-9 { len * (width * 0.5) / half + width * 0.5 } else { len };
    let cut = (len * (1.0 - c) * 0.5).max(covered).min(len * (1.0 - c).max(0.05));
    (Some(oriented(poly)), cut)
}

/// The point `dist` px back from the last point along `pts` (the first point if shorter).
fn point_back(pts: &[P], dist: f64) -> P {
    let mut left = dist;
    for w in pts.windows(2).rev() {
        let (a, b) = (w[0], w[1]);
        let l = dist2(a, b);
        if l >= left && l > 0.0 {
            return add(b, mul(sub(a, b), left / l));
        }
        left -= l;
    }
    pts.first().copied().unwrap_or((0.0, 0.0))
}

/// Removes `from_start` px from the start and `from_end` px from the end of `pl`.
fn trim(pl: &mut Polyline, from_start: f64, from_end: f64) {
    let total = length(&pl.pts);
    let (a, b) = (from_start.max(0.0), (total - from_end.max(0.0)).min(total));
    if b - a <= 1e-6 {
        // Heads cover the whole line: keep a single point so nothing but the heads draws.
        pl.pts.clear();
        pl.knot.clear();
        return;
    }
    let mut pts = Vec::with_capacity(pl.pts.len());
    let mut knots = Vec::with_capacity(pl.pts.len());
    let mut at = 0.0;
    for (i, w) in pl.pts.windows(2).enumerate() {
        let (p, q) = (w[0], w[1]);
        let l = dist2(p, q);
        let (s, e) = (at, at + l);
        if e >= a && s <= b && l > 0.0 {
            if pts.is_empty() {
                pts.push(add(p, mul(sub(q, p), ((a - s) / l).clamp(0.0, 1.0))));
                knots.push(s >= a && pl.knot.get(i).copied().unwrap_or(true));
            }
            if e <= b {
                pts.push(q);
                knots.push(pl.knot.get(i + 1).copied().unwrap_or(true));
            } else {
                pts.push(add(p, mul(sub(q, p), ((b - s) / l).clamp(0.0, 1.0))));
                knots.push(false);
                break;
            }
        }
        at = e;
    }
    pl.pts = pts;
    pl.knot = knots;
}

fn length(pts: &[P]) -> f64 {
    pts.windows(2).map(|w| dist2(w[0], w[1])).sum()
}

fn oriented(mut poly: Vec<P>) -> Vec<P> {
    let n = poly.len();
    let area: f64 = (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum();
    if area < 0.0 {
        poly.reverse();
    }
    poly
}

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P, k: f64) -> P {
    (a.0 * k, a.1 * k)
}
fn dist2(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}
fn norm(a: P) -> P {
    let l = a.0.hypot(a.1);
    if l > 0.0 { (a.0 / l, a.1 / l) } else { (0.0, 0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(pts: &[P]) -> Polyline {
        Polyline { pts: pts.to_vec(), knot: vec![true; pts.len()], closed: false }
    }

    #[test]
    fn end_head_points_along_the_line_and_trims_it() {
        let mut lines = vec![line(&[(0.0, 0.0), (100.0, 0.0)])];
        let heads = arrowheads(&mut lines, None, Some(&Arrowhead::default()), 2.0);
        assert_eq!(heads.len(), 1);
        let tip = heads[0].iter().fold(f64::MIN, |m, p| m.max(p.0));
        assert!((tip - 100.0).abs() < 1e-9, "tip at the end: {tip}");
        let ys: Vec<f64> = heads[0].iter().map(|p| p.1).collect();
        assert!(ys.iter().any(|y| (*y - 4.0).abs() < 1e-9) && ys.iter().any(|y| (*y + 4.0).abs() < 1e-9), "4 widths wide: {ys:?}");
        let end = lines[0].pts.last().map(|p| p.0).unwrap_or_default();
        assert!(end < 100.0 && end > 92.0, "line ends inside the 8 px head: {end}");
    }

    #[test]
    fn both_heads_and_curves_follow_the_chord() {
        // A quarter turn: the end head points down, along the final stretch.
        let pts: Vec<P> = (0..=32).map(|i| (i as f64 / 32.0 * std::f64::consts::FRAC_PI_2).sin_cos()).map(|(s, c)| (100.0 * s, 100.0 - 100.0 * c)).collect();
        let mut lines = vec![line(&pts)];
        let heads = arrowheads(&mut lines, Some(&Arrowhead::default()), Some(&Arrowhead::default()), 3.0);
        assert_eq!(heads.len(), 2);
        assert!(lines[0].pts.len() >= 2);
        assert!(lines[0].pts[0].0 > 1.0, "start trimmed");
    }

    #[test]
    fn short_lines_shrink_their_heads() {
        let mut lines = vec![line(&[(0.0, 0.0), (10.0, 0.0)])];
        let heads = arrowheads(&mut lines, Some(&Arrowhead::default()), Some(&Arrowhead::default()), 5.0);
        for h in &heads {
            let xs: Vec<f64> = h.iter().map(|p| p.0).collect();
            let span = xs.iter().fold(f64::MIN, |m, x| m.max(*x)) - xs.iter().fold(f64::MAX, |m, x| m.min(*x));
            assert!(span <= 4.5 + 1e-9, "each head at most 45% of the line: {span}");
        }
    }

    #[test]
    fn closed_degenerate_and_non_finite_lines_get_no_heads() {
        let mut lines = vec![
            Polyline { pts: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], knot: vec![true; 3], closed: true },
            line(&[(5.0, 5.0), (5.0, 5.0)]),
            line(&[(0.0, 0.0), (f64::NAN, 1.0)]),
            line(&[(0.0, 0.0)]),
        ];
        assert!(arrowheads(&mut lines, Some(&Arrowhead::default()), Some(&Arrowhead::default()), 2.0).is_empty());
        assert!(arrowheads(&mut lines, None, Some(&Arrowhead::default()), f64::NAN).is_empty());
    }

    #[test]
    fn heads_from_files_are_bounded() {
        // A file can hold any number: heads stay within the sizes Arrowhead::validate accepts.
        for h in [Arrowhead { width: 1e30, length: 1e30, concavity: f32::NAN }, Arrowhead { width: f32::NAN, length: f32::INFINITY, concavity: 9.0 }] {
            let mut lines = vec![line(&[(0.0, 0.0), (1000.0, 0.0)])];
            let heads = arrowheads(&mut lines, None, Some(&h), 2.0);
            assert_eq!(heads.len(), 1);
            assert!(heads[0].iter().all(|p| p.0.is_finite() && p.1.is_finite() && p.1.abs() <= 50.0 + 1e-9), "{:?}", heads[0]);
        }
    }

    #[test]
    fn concave_heads_notch_toward_the_tip() {
        let mut lines = vec![line(&[(0.0, 0.0), (100.0, 0.0)])];
        let h = Arrowhead { concavity: 0.5, ..Arrowhead::default() };
        let heads = arrowheads(&mut lines, None, Some(&h), 2.0);
        // Tip at 100, base at 92, notch halfway: 96.
        assert!(heads[0].iter().any(|p| (p.0 - 96.0).abs() < 1e-9 && p.1.abs() < 1e-9), "{:?}", heads[0]);
        let end = lines[0].pts.last().map(|p| p.0).unwrap_or_default();
        assert!((96.0..=98.0).contains(&end), "line ends between the notch and the tip: {end}");
    }
}
