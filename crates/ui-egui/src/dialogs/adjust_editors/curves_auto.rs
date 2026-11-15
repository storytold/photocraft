//! Curves' Auto (#9237): the points Photoshop's Curves › Auto (Enhance Brightness and Contrast,
//! clipping 0.10 % at each end) puts on the curve, fitted to what it was measured to do.
//!
//! Measured on Photoshop 2025 (notes: `photocraft/notes/curves-measurements-2026-11-14.md`, the
//! output of Auto on synthetic images read back pixel by pixel):
//! - the black point stays at (0, 0), whatever the image's darkest level;
//! - the white end moves to input 228 (Levels' Auto white, 229) unless the image reaches white
//!   itself, in which case it sits at the image's own white;
//! - one to three points in between lift the midtones, by an amount that follows the image.
//!
//! The inner points are NOT Photoshop's algorithm, which is not known: they are a fit. Three points
//! sit at 28 %, 64 % and 82 % of the white input, and their outputs are linear in the histogram's
//! mean and in whether the image reaches white, fitted to nine measured images (ramps, uniform
//! ranges, a bimodal image, a colour ramp, and a strip image with 17 levels). Against the measured
//! curves the fit is within about 2 levels on the dark and mid-range images and within 6 to 12 on
//! the most spread-out ones (the bimodal 60/200 image is the worst); the exact spread of an image
//! changes the real curve more than the mean does, and is not modelled.

use super::auto_levels;

/// The white input of a Curves Auto curve that does not reach white (Photoshop's Auto: about 228).
const WHITE_FLOOR: f32 = 228.0;

/// Where the three inner points sit, as a fraction of the white input.
const INNER_AT: [f32; 3] = [0.28, 0.64, 0.82];

/// The inner points' outputs at mean 0: the intercept, the change per level of the histogram's
/// mean, and the change when the image reaches white (its 0.1 %-clipped white is 253 or more).
const INNER_BASE: [f32; 3] = [76.632, 198.447, 245.139];
const INNER_PER_MEAN: [f32; 3] = [-0.1549, -0.2456, -0.2039];
const INNER_AT_WHITE: [f32; 3] = [30.84, 7.7175, -4.862];

/// An image whose clipped white is at least this reaches white (the dialog's white end stays at its
/// own white input, 255 in the two measured images that reach it).
const REACHES_WHITE: f32 = 253.0;

/// The mean level of a histogram (0 for an empty one).
fn mean(h: &[u32; 256]) -> f32 {
    let total: u64 = h.iter().map(|n| u64::from(*n)).sum();
    if total == 0 {
        return 0.0;
    }
    let sum: u64 = h.iter().enumerate().map(|(level, n)| level as u64 * u64::from(*n)).sum();
    sum as f32 / total as f32
}

/// The curve Curves' Auto sets for the histogram `h`: black at (0, 0), three fitted inner points and
/// the white end (white input, 255), inputs strictly increasing and outputs rising.
pub(super) fn auto_curve(h: &[u32; 256]) -> Vec<[f32; 2]> {
    let (_, hi) = auto_levels(h);
    let white = hi.max(WHITE_FLOOR).round();
    let reaches = if hi >= REACHES_WHITE { 1.0 } else { 0.0 };
    let m = mean(h);
    let mut points = vec![[0.0, 0.0]];
    let mut below = 0.0f32;
    for i in 0..INNER_AT.len() {
        let x = (INNER_AT[i] * white).round();
        // Rising, and at least a level under the next point's ceiling, so the curve never folds.
        let y = (INNER_BASE[i] + INNER_PER_MEAN[i] * m + INNER_AT_WHITE[i] * reaches).round().clamp(below + 1.0, 254.0 - (INNER_AT.len() - 1 - i) as f32);
        points.push([x, y]);
        below = y;
    }
    points.push([white, 255.0]);
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_compose::adjust::curve_samples;
    use photocraft_doc::adjust::CurvePoint;

    /// The curve's output (0..=255) at input `x`, through the same natural spline the dialog draws.
    fn at(points: &[[f32; 2]], x: usize) -> f32 {
        let cp: Vec<CurvePoint> = points.iter().map(|q| CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
        curve_samples::<256>(&cp)[x] * 255.0
    }

    fn histogram(levels: impl IntoIterator<Item = usize>) -> [u32; 256] {
        let mut h = [0u32; 256];
        for level in levels {
            h[level] += 100;
        }
        h
    }

    /// The maintainer's 340 x 60 image of 17 strips (strip i = RGB(40+4i, 50+4i, 30+3i), 20 px
    /// wide): the composite histogram is the channel mean of each strip.
    fn strips() -> [u32; 256] {
        histogram((0..17).map(|i| ((40 + 4 * i) + (50 + 4 * i) + (30 + 3 * i) + 1) / 3))
    }

    /// #9237 (davidmann4): Photoshop's Auto on the 17-strip image puts the composite curve through
    /// (0, 0), (66, 68), (147, 182), (188, 231) and (229, 255); the fit holds each within 2 levels.
    #[test]
    fn the_17_strip_image_matches_photoshops_composite_curve_within_2_levels() {
        let curve = auto_curve(&strips());
        assert_eq!(curve.first(), Some(&[0.0, 0.0]), "black stays at (0, 0): {curve:?}");
        for (x, want) in [(0, 0.0), (66, 68.0), (147, 182.0), (188, 231.0), (229, 255.0)] {
            let got = at(&curve, x);
            assert!((got - want).abs() <= 2.0, "input {x}: {got} vs {want}: {curve:?}");
        }
    }

    /// The black point stays at (0, 0) and the white end is 228 unless the image reaches white, then
    /// the image's own white: measured on ramps and uniform ranges from 30..220 to 0..255.
    #[test]
    fn black_stays_and_white_is_228_or_the_images_own() {
        for (lo, hi, white) in [(40, 120, 228.0), (64, 192, 228.0), (30, 220, 228.0), (100, 250, 250.0), (0, 255, 255.0)] {
            let curve = auto_curve(&histogram(lo..=hi));
            assert_eq!(curve.first(), Some(&[0.0, 0.0]), "{lo}..{hi}");
            assert_eq!(curve.last(), Some(&[white, 255.0]), "{lo}..{hi}: {curve:?}");
            assert_eq!(curve.len(), 5, "{lo}..{hi}: black, three inner points, white");
        }
    }

    /// The fit against what Photoshop 2025 gave on the measured ramps: output at a few inputs, within
    /// the fit's tolerance (2 on the dark and mid-range images, 6 on a full ramp).
    #[test]
    fn the_fit_stays_near_the_measured_ramps() {
        // (lo, hi, [(input, measured output)], tolerance)
        type Case<'a> = (usize, usize, &'a [(usize, f32)], f32);
        let cases: [Case; 4] = [
            (64, 192, &[(80, 77.0), (128, 138.0), (176, 209.0), (192, 226.0)], 5.0),
            (40, 120, &[(64, 64.0), (96, 106.0), (112, 130.0)], 4.0),
            (100, 250, &[(128, 112.0), (176, 175.0), (220, 222.0)], 7.0),
            (0, 255, &[(32, 37.0), (128, 141.0), (220, 225.0)], 8.0),
        ];
        for (lo, hi, measured, tolerance) in cases {
            let curve = auto_curve(&histogram(lo..=hi));
            for (x, want) in measured {
                let got = at(&curve, *x);
                assert!((got - want).abs() <= tolerance, "{lo}..{hi} at {x}: {got} vs {want}: {curve:?}");
            }
        }
    }

    /// Whatever the histogram, the points are finite, the inputs strictly increase (no vertical
    /// segment), the outputs rise, and everything stays inside 0..=255.
    #[test]
    fn every_histogram_gives_a_rising_curve_with_distinct_inputs() {
        let mut cases = vec![[0u32; 256], histogram([0]), histogram([128]), histogram([255]), histogram([0, 255]), histogram(0..=255)];
        cases.extend((0..256).step_by(17).map(|level| histogram([level])));
        for h in cases {
            let curve = auto_curve(&h);
            assert_eq!(curve.len(), 5);
            assert!(curve.iter().flatten().all(|v| v.is_finite() && (0.0..=255.0).contains(v)), "{curve:?}");
            assert!(curve.windows(2).all(|w| w[1][0] > w[0][0]), "inputs increase: {curve:?}");
            assert!(curve.windows(2).all(|w| w[1][1] > w[0][1]), "outputs rise: {curve:?}");
        }
    }
}
