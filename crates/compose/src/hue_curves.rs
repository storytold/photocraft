//! Hue Curves (`Adjustment::HueCurves`): hue vs hue, hue vs saturation and hue vs luminance.
//!
//! Each curve is periodic over the hue circle and interpolated with a monotone cubic (Fritsch and
//! Carlson's tangents), so transitions are soft and a run of equal points stays flat: anchoring the
//! neighbours of an edited hue keeps the edit from spreading. The three curves are sampled into the
//! hue-indexed tables Hue/Saturation's colour ranges use ([`crate::adjust::hue_range_tables`]), so
//! the CPU and the GPU kernel evaluate both adjustments the same way.

use photocraft_doc::adjust::{CurvePoint, HUE_CURVE_NEUTRAL, is_neutral_hue_curve};

/// Samples per table (as the other adjustment lookups).
pub const TABLE_LEN: usize = 4096;

/// Whether all three curves are neutral (the adjustment changes nothing).
pub fn is_identity(hue: &[CurvePoint], saturation: &[CurvePoint], luminance: &[CurvePoint]) -> bool {
    [hue, saturation, luminance].iter().zip(HUE_CURVE_NEUTRAL).all(|(c, n)| is_neutral_hue_curve(c, n))
}

/// The curve's points as `(hue degrees in 0..360, value)`, sorted, one per hue (the first wins).
fn points(points: &[CurvePoint]) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> =
        points.iter().filter(|p| p.input.is_finite() && p.output.is_finite()).map(|p| (f64::from(p.input).rem_euclid(360.0), f64::from(p.output))).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|b, a| (b.0 - a.0).abs() < 1e-3);
    // 360 wraps to 0 only through rem_euclid rounding; a point that close to 360 is the one at 0.
    if pts.len() > 1
        && let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied())
        && 360.0 - last.0 + first.0 < 1e-3
    {
        pts.pop();
    }
    pts
}

/// The curve sampled at [`TABLE_LEN`] hues from 0° to 360° (both ends the same hue), `neutral`
/// where it has no points.
pub fn curve_table(curve: &[CurvePoint], neutral: f32) -> Vec<f32> {
    let pts = points(curve);
    let n = pts.len();
    let at = |k: usize| k as f64 / (TABLE_LEN - 1) as f64 * 360.0;
    let Some(&first) = pts.first() else { return vec![neutral; TABLE_LEN] };
    if n == 1 {
        return vec![first.1 as f32; TABLE_LEN];
    }
    // Point `i` (any integer) on the unrolled circle: wraps by whole turns.
    let get = |i: isize| -> (f64, f64) {
        let len = n as isize;
        let turns = i.div_euclid(len);
        let p = pts.get(i.rem_euclid(len) as usize).copied().unwrap_or(first);
        (p.0 + 360.0 * turns as f64, p.1)
    };
    let secant = |i: isize| {
        let (a, b) = (get(i), get(i + 1));
        if b.0 > a.0 { (b.1 - a.1) / (b.0 - a.0) } else { 0.0 }
    };
    // Fritsch–Carlson tangents: zero at a local extremum or a flat neighbour, else the mean
    // secant limited to three times the smaller one (no overshoot).
    let tangent = |i: isize| {
        let (d0, d1) = (secant(i - 1), secant(i));
        if d0 * d1 <= 0.0 {
            0.0
        } else {
            let m = (d0 + d1) / 2.0;
            m.signum() * m.abs().min(3.0 * d0.abs().min(d1.abs()))
        }
    };
    let tangents: Vec<f64> = (0..n as isize).map(tangent).collect();
    let m = |i: isize| tangents.get(i.rem_euclid(n as isize) as usize).copied().unwrap_or(0.0);
    let mut seg: isize = -1;
    (0..TABLE_LEN)
        .map(|k| {
            let x = at(k);
            // The segment from point `seg` to `seg + 1` holding x; -1 wraps from the last point.
            while seg + 1 < n as isize && get(seg + 1).0 <= x {
                seg += 1;
            }
            let ((x0, y0), (x1, y1)) = (get(seg), get(seg + 1));
            let h = x1 - x0;
            if h <= 0.0 {
                return y0 as f32;
            }
            let t = (x - x0) / h;
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * m(seg) + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * m(seg + 1);
            y as f32
        })
        .collect()
}

/// Hue/Saturation's saturation change (-1..=1: lowering multiplies by `1 + s`, raising divides by
/// `1 - s`) that scales saturation by `factor` (0..=2).
pub fn saturation_change(factor: f32) -> f32 {
    let f = if factor.is_finite() { factor.clamp(0.0, 2.0) } else { 1.0 };
    if f <= 1.0 { f - 1.0 } else { 1.0 - 1.0 / f }
}

/// The three curves as Hue/Saturation's hue-indexed range tables: hue shift in degrees, saturation
/// change and lightness change as fractions (see [`crate::adjust::hue_range_tables`]).
pub fn tables(hue: &[CurvePoint], saturation: &[CurvePoint], luminance: &[CurvePoint]) -> [Vec<f32>; 3] {
    let [nh, ns, nl] = HUE_CURVE_NEUTRAL;
    let shift = curve_table(hue, nh).into_iter().map(|v| v.clamp(-180.0, 180.0)).collect();
    let sat = curve_table(saturation, ns).into_iter().map(saturation_change).collect();
    let lum = curve_table(luminance, nl).into_iter().map(|v| (v / 100.0).clamp(-1.0, 1.0)).collect();
    [shift, sat, lum]
}

/// Grid points per axis of the Color Lookup Hue Curves are saved to PSD as ([`as_lookup`]).
pub const LOOKUP_SIZE: u32 = 33;

/// Hue Curves approximated as a Color Lookup: a [`LOOKUP_SIZE`]³ cube (red fastest) of the
/// adjustment applied to each grid colour. Photoshop has no Hue Curves, so PSD files carry this
/// lookup instead. It is an approximation: the reader interpolates trilinearly between grid
/// colours, so an edit narrower than the grid's hue spacing (a few degrees at full saturation)
/// is softened or lost, and the curves are no longer editable. `None` for any other adjustment.
pub fn as_lookup(adj: &photocraft_doc::Adjustment) -> Option<photocraft_doc::Adjustment> {
    let photocraft_doc::Adjustment::HueCurves { .. } = adj else { return None };
    let n = LOOKUP_SIZE as usize;
    let step = |i: usize| i as f32 / (n - 1) as f32;
    let px: Vec<[f32; 4]> = (0..n * n * n).map(|i| [step(i % n), step(i / n % n), step(i / (n * n)), 1.0]).collect();
    let width = i32::try_from(px.len()).unwrap_or(i32::MAX);
    let mut buf = crate::Buffer { rect: crate::Rect::new(0, 0, width, 1), px };
    crate::adjust::apply(adj, &mut buf);
    let table: Vec<f32> = buf.px.iter().flat_map(|p| [p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0))).collect();
    Some(photocraft_doc::Adjustment::ColorLookup {
        name: adj.label().to_string(),
        lut: Some(std::sync::Arc::new(table)),
        size: LOOKUP_SIZE,
        tetrahedral: false,
        dither: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Buffer;
    use crate::adjust::{apply, rgb_to_hsl};
    use photocraft_doc::Adjustment;

    fn pts(p: &[(f32, f32)]) -> Vec<CurvePoint> {
        p.iter().map(|&(input, output)| CurvePoint { input, output }).collect()
    }

    /// DaVinci's starting curve: neutral anchors at the six primaries and secondaries, one moved.
    fn anchored(hue: f32, value: f32, neutral: f32) -> Vec<CurvePoint> {
        pts(&(0..6).map(|i| i as f32 * 60.0).map(|h| (h, if h == hue { value } else { neutral })).collect::<Vec<_>>())
    }

    fn run(adj: &Adjustment, colours: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let mut b = Buffer { rect: crate::Rect::new(0, 0, colours.len() as i32, 1), px: colours.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect() };
        apply(adj, &mut b);
        b.px.iter().map(|p| [p[0], p[1], p[2]]).collect()
    }

    fn hue_deg(c: [f32; 3]) -> f32 {
        rgb_to_hsl(c).0 * 360.0
    }

    fn curves(hue: Vec<CurvePoint>, saturation: Vec<CurvePoint>, luminance: Vec<CurvePoint>) -> Adjustment {
        Adjustment::HueCurves { hue, saturation, luminance }
    }

    const RED: [f32; 3] = [0.9, 0.1, 0.1];
    const GREEN: [f32; 3] = [0.15, 0.8, 0.2];
    const BLUE: [f32; 3] = [0.1, 0.2, 0.9];

    #[test]
    fn red_to_orange_leaves_blue_alone() {
        let a = curves(anchored(0.0, 30.0, 0.0), Vec::new(), Vec::new());
        let out = run(&a, &[RED, BLUE, [0.5, 0.5, 0.5]]);
        let h = hue_deg(out[0]);
        assert!((h - 30.0).abs() < 0.5, "red turns orange: hue {h}");
        assert!((rgb_to_hsl(out[0]).1 - rgb_to_hsl(RED).1).abs() < 1e-4, "saturation kept");
        for c in 0..3 {
            assert!((out[1][c] - BLUE[c]).abs() < 1e-5, "blue untouched: {:?}", out[1]);
            assert!((out[2][c] - 0.5).abs() < 1e-5, "grey stays grey: {:?}", out[2]);
        }
    }

    #[test]
    fn saturation_down_on_green_only() {
        let a = curves(Vec::new(), anchored(120.0, 0.5, 1.0), Vec::new());
        let out = run(&a, &[GREEN, RED, BLUE]);
        let (s0, s1) = (rgb_to_hsl(GREEN).1, rgb_to_hsl(out[0]).1);
        assert!((s1 - s0 * 0.5).abs() < 0.02, "green at half saturation: {s0} -> {s1}");
        assert!((hue_deg(out[0]) - hue_deg(GREEN)).abs() < 0.5, "hue kept");
        for (c, o) in [RED, BLUE].iter().zip(&out[1..]) {
            for k in 0..3 {
                assert!((c[k] - o[k]).abs() < 1e-5, "{c:?} -> {o:?}");
            }
        }
        // Factor 2 doubles a dull colour's saturation; 0 makes it grey.
        let dull = [0.5, 0.6, 0.5];
        let up = run(&curves(Vec::new(), pts(&[(120.0, 2.0)]), Vec::new()), &[dull]);
        assert!((rgb_to_hsl(up[0]).1 - 2.0 * rgb_to_hsl(dull).1).abs() < 0.01);
        let grey = run(&curves(Vec::new(), pts(&[(0.0, 0.0)]), Vec::new()), &[GREEN]);
        assert!(rgb_to_hsl(grey[0]).1 < 1e-4);
    }

    #[test]
    fn luminance_moves_the_chosen_hue() {
        let a = curves(Vec::new(), Vec::new(), anchored(240.0, -50.0, 0.0));
        let out = run(&a, &[BLUE, RED]);
        assert!(rgb_to_hsl(out[0]).2 < rgb_to_hsl(BLUE).2 - 0.05, "blue darker: {:?}", out[0]);
        assert_eq!(out[1], RED);
    }

    #[test]
    fn thirty_two_bit_curves_keep_bright_colours() {
        // Red at twice white in a 32-bit document, hue +30: Hue/Saturation's 32-bit evaluation
        // (the GPU's too) keeps it a bright orange; the 8/16-bit one makes it white.
        let adj = curves(anchored(0.0, 30.0, 0.0), Vec::new(), Vec::new());
        let mut b = Buffer { rect: crate::Rect::new(0, 0, 1, 1), px: vec![[2.0, 0.0, 0.0, 1.0]] };
        crate::adjust::apply_depth(&adj, &mut b, crate::adjust::Transfer::Gamma(1.0), Some(photocraft_color::SampleType::F32));
        let got = [b.px[0][0], b.px[0][1], b.px[0][2]];
        let want = crate::adjust::hue_saturation_in([2.0, 0.0, 0.0], 30.0, 0.0, 0.0, false, Some(photocraft_color::SampleType::F32));
        assert_eq!(got, want);
        assert!(got.iter().all(|v| v.is_finite()) && got[0] > 1.0 && got[2] < 0.5, "{got:?}");
    }

    #[test]
    fn neutral_curves_are_a_no_op() {
        let many: Vec<[f32; 3]> = (0..64).map(|i| [(i % 4) as f32 / 3.0, (i / 4 % 4) as f32 / 3.0, (i / 16) as f32 / 3.0]).collect();
        for a in [curves(Vec::new(), Vec::new(), Vec::new()), curves(anchored(0.0, 0.0, 0.0), anchored(0.0, 1.0, 1.0), anchored(0.0, 0.0, 0.0))] {
            assert_eq!(run(&a, &many), many, "{a:?}");
        }
        assert!(is_identity(&[], &anchored(60.0, 1.0, 1.0), &[]));
        assert!(!is_identity(&[], &anchored(60.0, 0.9, 1.0), &[]));
    }

    #[test]
    fn curves_wrap_around_red() {
        // A point just below 360 and one just above 0 make one smooth bump through red.
        let t = curve_table(&pts(&[(350.0, 20.0), (10.0, 20.0), (180.0, 0.0)]), 0.0);
        let first = *t.first().unwrap();
        let last = *t.last().unwrap();
        assert!((first - last).abs() < 1e-4, "0° and 360° are the same hue: {first} vs {last}");
        assert!((first - 20.0).abs() < 1e-3, "red sits between two 20° points: {first}");
        // No step anywhere: neighbouring samples stay close, also across the seam.
        let max_step = t.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.1, "smooth: {max_step}");
        // Points given past 360 or below 0 are the same hues.
        assert_eq!(t, curve_table(&pts(&[(-10.0, 20.0), (370.0, 20.0), (540.0, 0.0)]), 0.0));
        // Monotone between anchors: no overshoot past the points' range.
        assert!(t.iter().all(|v| (-1e-4..=20.0001).contains(v)), "no overshoot");
        // Colours either side of the seam move alike.
        let a = curves(pts(&[(350.0, 20.0), (10.0, 20.0), (180.0, 0.0)]), Vec::new(), Vec::new());
        let out = run(&a, &[[0.9, 0.1, 0.15], [0.9, 0.15, 0.1]]);
        let shift = |c: [f32; 3], o: [f32; 3]| (hue_deg(o) - hue_deg(c) + 540.0).rem_euclid(360.0) - 180.0;
        assert!((shift([0.9, 0.1, 0.15], out[0]) - shift([0.9, 0.15, 0.1], out[1])).abs() < 0.5);
    }

    #[test]
    fn the_psd_lookup_renders_like_the_curves() {
        let a = curves(anchored(0.0, 30.0, 0.0), anchored(120.0, 0.5, 1.0), anchored(240.0, -30.0, 0.0));
        let lookup = as_lookup(&a).unwrap();
        let Adjustment::ColorLookup { name, size, .. } = &lookup else { panic!() };
        assert_eq!((name.as_str(), *size), ("Hue Curves", LOOKUP_SIZE));
        let colours = [RED, GREEN, BLUE, [0.5, 0.5, 0.5], [0.8, 0.6, 0.2]];
        let (want, got) = (run(&a, &colours), run(&lookup, &colours));
        for (w, g) in want.iter().zip(&got) {
            for c in 0..3 {
                assert!((w[c] - g[c]).abs() < 0.03, "{w:?} vs {g:?}");
            }
        }
        assert!(as_lookup(&Adjustment::Invert).is_none());
    }

    #[test]
    fn odd_curves_do_not_panic() {
        for c in [pts(&[(f32::NAN, 1.0)]), pts(&[(10.0, f32::INFINITY)]), pts(&[(5.0, 3.0), (5.0, 4.0)]), pts(&[(0.0, 1.0), (360.0, 2.0)])] {
            let t = curve_table(&c, 0.0);
            assert_eq!(t.len(), TABLE_LEN);
            assert!(t.iter().all(|v| v.is_finite()));
            let a = curves(c.clone(), c.clone(), c);
            let _ = run(&a, &[RED, GREEN, BLUE]);
        }
    }
}
