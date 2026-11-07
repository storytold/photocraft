//! One projective rectification of the WHOLE image from independent rectangular guides.
//!
//! No region ownership, blending of local maps, or mesh deformation is involved. In centered,
//! isotropically normalized coordinates write u=(x+b*y)/w, v=(d*x+y)/w, w=g*x+h*y+1.
//! A horizontal edge p--q supplies d*dx + (d*h-g)*(px*qy-qx*py) = -dy; a vertical
//! edge supplies b*dy + (h-b*g)*(px*qy-qx*py) = -dx. Two robust 2-parameter line fits
//! therefore estimate ALL guides' common directions, including vanishing points at infinity.
//! Recover h=(kv-b*kh)/(1-b*d), g=d*h-kh. Axis scales preserve apparent unit lengths
//! along the two architectural directions at the image center: true aspect is not observable
//! from unknown rectangles alone. Optional aspect is an explicit user choice, not metrology.
//! The denominator is affine, so a positive lower bound at all four domain corners proves
//! no horizon crosses the rectangle. A positive determinant then proves positive Jacobian
//! everywhere, and a nonsingular homography is injective on this domain. No sampled-only
//! no-fold claim is used. Raster callers MUST use this matrix, never per-guide transforms.

use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};

use crate::transform::{Homography, convex_quad};

pub const MAX_GUIDES: usize = 128;
pub const MAX_OUTPUT_PIXELS: f64 = 67_108_864.0;
const FIT_THRESHOLD: f64 = 0.035;
const GUIDE_THRESHOLD_DEG: f64 = 3.0;

fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Guide {
    pub id: u64,
    /// Clockwise in image coordinates, TL, TR, BR, BL. Not an image region or target quad.
    pub corners: [[f64; 2]; 4],
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    /// Uniformly fit the complete transformed image into its original bounds, without cropping.
    #[serde(default = "yes")]
    pub fit_canvas: bool,
    /// Explicit global width/height adjustment. Does not imply recovered physical dimensions.
    #[serde(default = "one")]
    pub aspect: f64,
}
impl Default for Options {
    fn default() -> Self {
        Self { fit_canvas: true, aspect: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideResult {
    pub id: u64,
    pub enabled: bool,
    pub used: bool,
    /// Only a guide used in the final fit receives residual diagnostics.
    pub max_error_deg: Option<f64>,
    /// An unused or disabled guide must not project into the unsafe off-canvas domain.
    pub transformed_corners: Option<[[f64; 2]; 4]>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Solution {
    pub model: &'static str,
    pub matrix: [f64; 9],
    pub source_bounds: [f64; 4],
    pub output_bounds: [f64; 4],
    pub used_guide_ids: Vec<u64>,
    pub rejected_guide_ids: Vec<u64>,
    pub guides: Vec<GuideResult>,
    pub max_used_error_deg: f64,
    pub min_denominator: f64,
    pub min_jacobian: f64,
    pub aspect_ambiguous: bool,
    pub warnings: Vec<String>,
}
impl Solution {
    pub fn homography(&self) -> Homography {
        Homography(self.matrix)
    }
}

pub fn validate_guides(guides: &[Guide]) -> Result<(), String> {
    if guides.len() > MAX_GUIDES {
        return Err(format!("too many guides (maximum {MAX_GUIDES})"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for guide in guides {
        if !ids.insert(guide.id) {
            return Err(format!("duplicate guide id {}", guide.id));
        }
        if !guide.weight.is_finite() || !(0.05..=20.0).contains(&guide.weight) {
            return Err(format!("guide {} weight must be finite, 0.05..20", guide.id));
        }
        if guide.corners.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1_000_000.0) || !convex_quad(&guide.corners) {
            return Err(format!("guide {} must be a finite, clockwise, nondegenerate convex quad (TL TR BR BL)", guide.id));
        }
    }
    Ok(())
}

pub fn validate_options(options: &Options) -> Result<(), String> {
    if !options.aspect.is_finite() || !(0.25..=4.0).contains(&options.aspect) {
        return Err("aspect must be finite and within 0.25..4".into());
    }
    Ok(())
}

fn corners(r: Rect) -> [[f64; 2]; 4] {
    [[r.x0 as f64, r.y0 as f64], [r.x1 as f64, r.y0 as f64], [r.x1 as f64, r.y1 as f64], [r.x0 as f64, r.y1 as f64]]
}
fn determinant(m: &[f64; 9]) -> f64 {
    m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6])
}
fn mapped(h: &Homography, q: &[[f64; 2]; 4]) -> [[f64; 2]; 4] {
    q.map(|p| {
        let (x, y) = h.apply(p[0], p[1]);
        [x, y]
    })
}
fn extent(q: &[[f64; 2]; 4]) -> [f64; 4] {
    [
        q.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        q.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        q.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
        q.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
    ]
}

/// Analytic full-domain safety check; also used for linked masks before raster allocation.
/// Returns (output extent, minimum denominator, minimum Jacobian determinant).
pub fn validate_map(h: &Homography, bounds: Rect) -> Result<([f64; 4], f64, f64), String> {
    if bounds.is_empty() || bounds.width() > 32768 || bounds.height() > 32768 {
        return Err("image domain is empty or exceeds 32768 pixels per side".into());
    }
    if h.0.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite global homography".into());
    }
    let c = corners(bounds);
    let ws = c.map(|p| h.0[6] * p[0] + h.0[7] * p[1] + h.0[8]);
    let wmin = ws.iter().copied().fold(f64::INFINITY, f64::min);
    let wmax = ws.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if wmin <= 0.0 || wmin < 0.04 * wmax {
        return Err("unsafe global perspective: horizon crosses or approaches the image; correct the guides".into());
    }
    let det = determinant(&h.0);
    if det <= 1e-12 || !det.is_finite() || h.inverse().is_none() {
        return Err("global homography is singular, mirrored or ill-conditioned".into());
    }
    let q = mapped(h, &c);
    if q.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1_000_000.0) {
        return Err("global perspective exceeds the safe coordinate range".into());
    }
    let e = extent(&q);
    let w = e[2] - e[0];
    let height = e[3] - e[1];
    if w < 1.0 || height < 1.0 || w > 16.0 * bounds.width() as f64 || height > 16.0 * bounds.height() as f64 {
        return Err("global perspective collapses or stretches the image excessively".into());
    }
    Ok((e, wmin, det / (wmax * wmax * wmax)))
}

#[derive(Clone, Copy)]
struct Row {
    a: f64,
    b: f64,
    target: f64,
    weight: f64,
}
impl Row {
    fn error(self, x: [f64; 2]) -> f64 {
        self.a * x[0] + self.b * x[1] - self.target
    }
}
fn fit_rows(rows: &[Row], seed: Option<[f64; 2]>) -> Option<[f64; 2]> {
    let (mut aa, mut ab, mut bb, mut at, mut bt) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for r in rows {
        if seed.is_some_and(|x| r.error(x).abs() > FIT_THRESHOLD) {
            continue;
        }
        aa += r.weight * r.a * r.a;
        ab += r.weight * r.a * r.b;
        bb += r.weight * r.b * r.b;
        at += r.weight * r.a * r.target;
        bt += r.weight * r.b * r.target;
    }
    let det = aa * bb - ab * ab;
    if det <= 1e-12 * (aa + bb) * (aa + bb) || !det.is_finite() {
        return None;
    }
    let x = [(at * bb - bt * ab) / det, (bt * aa - at * ab) / det];
    x.iter().all(|v| v.is_finite()).then_some(x)
}
fn robust_fit(rows: &[Row]) -> Result<[f64; 2], String> {
    let mut best = fit_rows(rows, None).ok_or("insufficient spread of guide edges for a global perspective")?;
    let score = |x: [f64; 2]| rows.iter().map(|r| r.weight * r.error(x).powi(2).min(FIT_THRESHOLD * FIT_THRESHOLD)).sum::<f64>();
    let mut best_score = score(best);
    // Bounded deterministic hypotheses: at most 64 representative edges, no random state.
    let step = rows.len().div_ceil(64).max(1);
    for (i, r) in rows.iter().enumerate().step_by(step) {
        for t in rows.iter().take(i).step_by(step) {
            let det = r.a * t.b - t.a * r.b;
            if det.abs() < 1e-10 {
                continue;
            }
            let x = [(r.target * t.b - t.target * r.b) / det, (t.target * r.a - r.target * t.a) / det];
            let s = score(x);
            if s.is_finite() && s < best_score {
                best = x;
                best_score = s;
            }
        }
    }
    for _ in 0..4 {
        if let Some(x) = fit_rows(rows, Some(best)) {
            best = x;
        }
    }
    Ok(best)
}
fn rows(guides: &[&Guide], bounds: Rect, horizontal: bool) -> Vec<Row> {
    let cx = (bounds.x0 as f64 + bounds.x1 as f64) / 2.0;
    let cy = (bounds.y0 as f64 + bounds.y1 as f64) / 2.0;
    let scale = bounds.width().max(bounds.height()) as f64;
    let pairs = if horizontal { [(0, 1), (3, 2)] } else { [(0, 3), (1, 2)] };
    let mut result = Vec::with_capacity(2 * guides.len());
    for guide in guides {
        let q = guide.corners.map(|p| [(p[0] - cx) / scale, (p[1] - cy) / scale]);
        for (i, j) in pairs {
            let (p, t) = (q[i], q[j]);
            let dx = p[0] - t[0];
            let dy = p[1] - t[1];
            let length = dx.hypot(dy);
            let cross = p[0] * t[1] - t[0] * p[1];
            result.push(Row {
                a: if horizontal { dx / length } else { dy / length },
                b: cross / length,
                target: if horizontal { -dy / length } else { -dx / length },
                weight: guide.weight * 0.5,
            });
        }
    }
    result
}
fn matrix_from_fits(horizontal: [f64; 2], vertical: [f64; 2], bounds: Rect, aspect: f64) -> Result<Homography, String> {
    let [d, kh] = horizontal;
    let [b, kv] = vertical;
    let q = 1.0 - b * d;
    if !q.is_finite() || q < 0.05 {
        return Err("architectural direction families are incompatible or nearly parallel".into());
    }
    let h = (kv - b * kh) / q;
    let g = d * h - kh;
    let sx = 1.0_f64.hypot(d) / q * aspect.sqrt();
    let sy = 1.0_f64.hypot(b) / q / aspect.sqrt();
    let local = Homography([sx, sx * b, 0.0, sy * d, sy, 0.0, g, h, 1.0]);
    let cx = (bounds.x0 as f64 + bounds.x1 as f64) / 2.0;
    let cy = (bounds.y0 as f64 + bounds.y1 as f64) / 2.0;
    let scale = bounds.width().max(bounds.height()) as f64;
    let n = Homography([1.0 / scale, 0.0, -cx / scale, 0.0, 1.0 / scale, -cy / scale, 0.0, 0.0, 1.0]);
    let restore = Homography([scale, 0.0, cx, 0.0, scale, cy, 0.0, 0.0, 1.0]);
    let m = restore.mul(&local).mul(&n);
    validate_map(&m, bounds)?;
    Ok(m)
}
fn residual(h: &Homography, guide: &Guide) -> f64 {
    let q = mapped(h, &guide.corners);
    let mut worst = 0.0_f64;
    for (i, j, horizontal) in [(0, 1, true), (3, 2, true), (0, 3, false), (1, 2, false)] {
        let dx = (q[j][0] - q[i][0]).abs();
        let dy = (q[j][1] - q[i][1]).abs();
        let angle = if horizontal { dy.atan2(dx) } else { dx.atan2(dy) }.to_degrees();
        worst = worst.max(angle);
    }
    worst
}

/// Solve globally; inconsistent guides are reported, not independently straightened.
pub fn solve(guides: &[Guide], bounds: Rect, options: &Options) -> Result<Solution, String> {
    validate_guides(guides)?;
    validate_options(options)?;
    if bounds.is_empty() {
        return Err("cannot rectify an empty image".into());
    }
    let mut active: Vec<&Guide> = guides.iter().filter(|g| g.enabled).collect();
    active.sort_by_key(|g| g.id);
    if active.is_empty() {
        return Err("add at least one enabled rectangular guide".into());
    }
    // User-drawn guides are allowed to extend into the pasteboard beyond the
    // raster bounds. Finite/coherent quad checks precede this; validate_map
    // still protects the actual image domain against horizons or folds.
    // Fit all guides robustly, then use a common inlier set for BOTH directions.
    let mut h = matrix_from_fits(robust_fit(&rows(&active, bounds, true))?, robust_fit(&rows(&active, bounds, false))?, bounds, options.aspect)?;
    let mut used = Vec::new();
    for _ in 0..4 {
        let good: Vec<&Guide> = active.iter().copied().filter(|g| residual(&h, g) <= GUIDE_THRESHOLD_DEG).collect();
        if good.len() <= active.len() / 2 {
            return Err("incompatible guides: no majority supports a single global perspective; disable conflicting guides".into());
        }
        let ids: Vec<u64> = good.iter().map(|g| g.id).collect();
        if ids == used {
            break;
        }
        used = ids;
        h = matrix_from_fits(
            fit_rows(&rows(&good, bounds, true), None).ok_or("horizontal guides are underconstrained")?,
            fit_rows(&rows(&good, bounds, false), None).ok_or("vertical guides are underconstrained")?,
            bounds,
            options.aspect,
        )?;
    }
    // Classify final fit afresh. Never label an above-threshold guide as accepted.
    used = active.iter().filter(|g| residual(&h, g) <= GUIDE_THRESHOLD_DEG).map(|g| g.id).collect();
    if used.len() <= active.len() / 2 {
        return Err("global rectification did not converge to a compatible majority".into());
    }
    let (raw_extent, _, _) = validate_map(&h, bounds)?;
    if options.fit_canvas {
        let scale = (bounds.width() as f64 / (raw_extent[2] - raw_extent[0])).min(bounds.height() as f64 / (raw_extent[3] - raw_extent[1]));
        let tx = (bounds.x0 as f64 + bounds.x1 as f64 - scale * (raw_extent[0] + raw_extent[2])) / 2.0;
        let ty = (bounds.y0 as f64 + bounds.y1 as f64 - scale * (raw_extent[1] + raw_extent[3])) / 2.0;
        h = Homography([scale, 0.0, tx, 0.0, scale, ty, 0.0, 0.0, 1.0]).mul(&h);
    }
    let (output, wmin, jmin) = validate_map(&h, bounds)?;
    let output_pixels = (output[2].ceil() - output[0].floor() + 2.0) * (output[3].ceil() - output[1].floor() + 2.0);
    if output_pixels > MAX_OUTPUT_PIXELS {
        return Err("global rectification output exceeds the 64-megapixel safety budget".into());
    }
    // An off-canvas guide could cross a projective horizon outside the image.
    // Keep user controls permissive, but never expose non-finite previews or
    // diagnostics from an unsafe extrapolation of H beyond its safe domain.
    for g in guides.iter().filter(|g| used.contains(&g.id)) {
        let transformed = mapped(&h, &g.corners);
        if transformed.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1_000_000.0) {
            return Err(format!("guide {} projects outside the safe preview domain", g.id));
        }
    }
    let rejected: Vec<u64> = active.iter().filter(|g| !used.contains(&g.id)).map(|g| g.id).collect();
    let mut results: Vec<GuideResult> = guides
        .iter()
        .map(|g| {
            let used_guide = g.enabled && used.contains(&g.id);
            GuideResult {
                id: g.id,
                enabled: g.enabled,
                used: used_guide,
                max_error_deg: used_guide.then(|| residual(&h, g)),
                transformed_corners: used_guide.then(|| mapped(&h, &g.corners)),
            }
        })
        .collect();
    results.sort_by_key(|g| g.id);
    let max_error = results.iter().filter_map(|g| g.max_error_deg).fold(0.0_f64, f64::max);
    let mut warnings = vec!["Aspect ratio is not physically calibrated; apparent central proportions are preserved.".into()];
    if active.len() == 1 {
        warnings.push("Only one guide: no independent consistency check is possible.".into());
    }
    if !rejected.is_empty() {
        warnings.push(format!(
            "{} inconsistent guide(s) excluded. They remain visible and may reflect inaccurate corners or different facade directions.",
            rejected.len()
        ));
    }
    if !options.fit_canvas {
        warnings.push("Fit canvas is off: transformed pixels may extend beyond the document canvas.".into());
    }
    Ok(Solution {
        model: "singleGlobalHomography",
        matrix: h.0,
        source_bounds: [bounds.x0 as f64, bounds.y0 as f64, bounds.x1 as f64, bounds.y1 as f64],
        output_bounds: output,
        used_guide_ids: used,
        rejected_guide_ids: rejected,
        guides: results,
        max_used_error_deg: max_error,
        min_denominator: wmin,
        min_jacobian: jmin,
        aspect_ambiguous: true,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guide(id: u64, q: [[f64; 2]; 4]) -> Guide {
        Guide { id, corners: q, enabled: true, weight: 1.0 }
    }
    fn fixture() -> (Rect, Vec<Guide>) {
        let h = Homography([0.9, 0.04, 30.0, -0.04, 0.85, 45.0, 0.0001, -0.00013, 1.0]);
        let guides = (0..6)
            .map(|i| {
                let x = 100.0 + (i % 2) as f64 * 370.0;
                let y = 120.0 + (i / 2) as f64 * 240.0;
                guide(i, mapped(&h, &[[x, y], [x + 80.0 + i as f64 * 6.0, y], [x + 80.0 + i as f64 * 6.0, y + 120.0], [x, y + 120.0]]))
            })
            .collect();
        (Rect::new(0, 0, 1000, 1000), guides)
    }
    #[test]
    fn six_disjoint_guides_share_one_global_map() {
        let (bounds, g) = fixture();
        let s = solve(&g, bounds, &Options::default()).unwrap();
        assert_eq!(s.used_guide_ids.len(), 6);
        assert!(s.max_used_error_deg < 1e-7);
        assert!(s.min_jacobian > 0.0);
        let h = s.homography();
        // Collinearity in the unmarked wall, nowhere near guide corners.
        for t in [0.13, 0.37, 0.65, 0.87] {
            let a = h.apply(80.0, 50.0);
            let b = h.apply(900.0, 850.0);
            let p = h.apply(80.0 + t * 820.0, 50.0 + t * 800.0);
            assert!(((p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)).abs() < 1e-6);
        }
    }
    #[test]
    fn straight_identity_and_different_rectangular_proportions() {
        let g =
            vec![guide(1, [[10.0, 10.0], [70.0, 10.0], [70.0, 180.0], [10.0, 180.0]]), guide(2, [[100.0, 30.0], [250.0, 30.0], [250.0, 80.0], [100.0, 80.0]])];
        let s = solve(&g, Rect::new(0, 0, 300, 200), &Options::default()).unwrap();
        assert!(s.homography().is_identity());
        assert!(s.aspect_ambiguous);
    }
    #[test]
    fn deterministic_and_input_order_independent() {
        let (r, mut g) = fixture();
        let a = solve(&g, r, &Options::default()).unwrap();
        g.reverse();
        let b = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(a.matrix, b.matrix);
    }
    #[test]
    fn outlier_is_reported_without_warping_its_own_region() {
        let (r, mut g) = fixture();
        g.push(guide(77, [[650.0, 600.0], [800.0, 690.0], [740.0, 830.0], [600.0, 750.0]]));
        let s = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(s.used_guide_ids.len(), 6);
        assert_eq!(s.rejected_guide_ids, vec![77]);
        assert!(s.max_used_error_deg < 1e-7);
    }
    #[test]
    fn disabled_guide_does_not_change_fit() {
        let (r, mut g) = fixture();
        let a = solve(&g, r, &Options::default()).unwrap();
        let mut disabled = guide(88, [[5.0, 10.0], [80.0, 15.0], [70.0, 90.0], [10.0, 80.0]]);
        disabled.enabled = false;
        g.push(disabled);
        let b = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(a.matrix, b.matrix);
        assert_eq!(b.guides.len(), 7);
    }
    /// Reviewer #8497: a disabled guide far away at the projective horizon
    /// is valid input but must never veto a matrix fitted exclusively to active guides.
    #[test]
    fn disabled_near_horizon_guide_does_not_reject_valid_global_solution() {
        let (bounds, mut guides) = fixture();
        let original = solve(&guides, bounds, &Options::default()).unwrap();
        let h = original.homography();
        let (a, b, c) = (h.0[6], h.0[7], h.0[8]);
        assert!(a.abs().max(b.abs()) > 1e-9, "fixture must produce a projective horizon");
        let near_horizon = if a.abs() >= b.abs() {
            let x = (-c - b * 25.0) / a;
            [[x, 25.0], [x + 20.0, 25.0], [x + 20.0, 45.0], [x, 45.0]]
        } else {
            let y = (-c - a * 25.0) / b;
            [[25.0, y], [45.0, y], [45.0, y + 20.0], [25.0, y + 20.0]]
        };
        let disabled = Guide { enabled: false, ..guide(999, near_horizon) };
        assert!(validate_guides(std::slice::from_ref(&disabled)).is_ok());
        let projected = mapped(&h, &disabled.corners);
        assert!(
            projected.iter().flatten().any(|v| !v.is_finite() || v.abs() > 1_000_000.0),
            "the disabled guide must reproduce the previous unsafe extrapolation gate: {projected:?}"
        );
        guides.push(disabled);
        let actual = solve(&guides, bounds, &Options::default()).unwrap();
        assert_eq!(actual.matrix, original.matrix, "disabled guide cannot change accepted homography");
        assert_eq!(actual.used_guide_ids, original.used_guide_ids);
        assert_eq!(actual.guides.len(), 7);
        let unused = actual.guides.iter().find(|g| g.id == 999).unwrap();
        assert!(!unused.used && !unused.enabled);
        assert_eq!(unused.max_error_deg, None);
        assert_eq!(unused.transformed_corners, None);
    }

    #[test]
    fn a_single_quad_rectifies_but_is_not_metric_calibration() {
        let (r, g) = fixture();
        let s = solve(&g[..1], r, &Options::default()).unwrap();
        assert!(s.max_used_error_deg < 1e-7);
        assert_eq!(s.warnings.len(), 2);
    }
    #[test]
    fn malformed_duplicate_and_missing_guides_fail() {
        let (r, mut g) = fixture();
        assert!(solve(&[], r, &Options::default()).is_err());
        g[1].id = g[0].id;
        assert!(validate_guides(&g).is_err());
        g[1].id = 19;
        g[0].corners[2] = g[0].corners[0];
        assert!(validate_guides(&g).is_err());
        g[0].corners[0][0] = f64::NAN;
        assert!(validate_guides(&g).is_err());
        assert!(validate_options(&Options { fit_canvas: true, aspect: 0.0 }).is_err());
    }
    #[test]
    fn no_horizon_mirror_or_unbounded_output() {
        let r = Rect::new(0, 0, 1000, 1000);
        assert!(validate_map(&Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -0.002, 0.0, 1.0]), r).is_err());
        assert!(validate_map(&Homography([-1.0, 0.0, 1000.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]), r).is_err());
        assert!(validate_map(&Homography([100.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 1.0]), r).is_err());
    }
    #[test]
    fn coordinate_scale_and_nonzero_origin_are_supported() {
        let (r, g) = fixture();
        let a = solve(&g, r, &Options::default()).unwrap();
        let scaled: Vec<_> = g.iter().map(|g| guide(g.id, g.corners.map(|p| [p[0] * 8.0 - 500.0, p[1] * 8.0 + 700.0]))).collect();
        let b = solve(&scaled, Rect::new(-500, 700, 7500, 8700), &Options::default()).unwrap();
        let p = a.homography().apply(330.0, 570.0);
        let q = b.homography().apply(2140.0, 5260.0);
        assert!((q.0 - (p.0 * 8.0 - 500.0)).abs() < 1e-6);
        assert!((q.1 - (p.1 * 8.0 + 700.0)).abs() < 1e-6);
    }
    #[test]
    fn fit_canvas_contains_entire_transformed_footprint() {
        let (r, g) = fixture();
        let s = solve(&g, r, &Options::default()).unwrap();
        for p in mapped(&s.homography(), &corners(r)) {
            assert!(p[0] >= -1e-7 && p[0] <= 1000.0 + 1e-7 && p[1] >= -1e-7 && p[1] <= 1000.0 + 1e-7);
        }
    }
    #[test]
    fn strict_serde_rejects_unknown_keys() {
        assert!(serde_json::from_str::<Guide>(r#"{"id":1,"corners":[[1,1],[20,1],[20,20],[1,20]],"mystery":true}"#).is_err());
        assert!(serde_json::from_str::<Options>(r#"{"strength":0.5}"#).is_err());
    }

    #[test]
    fn noisy_guides_remain_global() {
        let (r, mut g) = fixture();
        for (i, guide) in g.iter_mut().enumerate() {
            for (j, p) in guide.corners.iter_mut().enumerate() {
                p[0] += ((i * 4 + j) as f64).sin() * 0.3;
                p[1] += ((i * 4 + j + 7) as f64).cos() * 0.3;
            }
        }
        let s = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(s.used_guide_ids.len(), 6);
        assert!(s.max_used_error_deg < 2.0);
    }
    #[test]
    fn off_image_disabled_guides_are_safe_but_invalid_and_excessive_count_are_rejected() {
        let (r, mut g) = fixture();
        let mut outside = guide(77, [[-50.0, 10.0], [50.0, 10.0], [50.0, 100.0], [-50.0, 100.0]]);
        outside.enabled = false;
        g.push(outside);
        let solution = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(solution.used_guide_ids.len(), 6);
        assert_eq!(solution.guides.len(), 7);
        g.last_mut().unwrap().corners[0][0] = f64::NAN;
        assert!(solve(&g, r, &Options::default()).is_err());
        let many: Vec<_> = (0..129).map(|id| guide(id, [[10.0, 10.0], [40.0, 10.0], [40.0, 80.0], [10.0, 80.0]])).collect();
        assert!(solve(&many, r, &Options::default()).is_err());
        for guide in &mut g {
            guide.enabled = false;
        }
        assert!(solve(&g, r, &Options::default()).is_err());
    }
    #[test]
    fn enabled_guide_may_cross_image_border_without_changing_the_photo_domain() {
        let r = Rect::new(0, 0, 300, 200);
        let g = [guide(1, [[-35.0, -12.0], [75.0, -12.0], [75.0, 140.0], [-35.0, 140.0]])];
        let solution = solve(&g, r, &Options::default()).unwrap();
        assert!(solution.min_jacobian > 0.0);
        assert!(solution.guides[0].transformed_corners.as_ref().unwrap().iter().flatten().all(|v| v.is_finite()));
        assert_eq!(solution.source_bounds, [0.0, 0.0, 300.0, 200.0]);
    }
    #[test]
    fn incompatible_equal_groups_are_not_stitched() {
        let (r, mut g) = fixture();
        g.truncate(3);
        let angle = 0.5_f64;
        let (sn, cs) = angle.sin_cos();
        for i in 0..3 {
            let cx = 670.0;
            let cy = 180.0 + i as f64 * 240.0;
            let q = [[-45.0, -55.0], [45.0, -55.0], [45.0, 55.0], [-45.0, 55.0]].map(|p| [cx + cs * p[0] - sn * p[1], cy + sn * p[0] + cs * p[1]]);
            g.push(guide(100 + i, q));
        }
        assert!(solve(&g, r, &Options::default()).is_err());
    }
    #[test]
    fn bounded_output_budget_precedes_raster_allocation() {
        let g = [guide(1, [[10.0, 10.0], [70.0, 10.0], [70.0, 100.0], [10.0, 100.0]])];
        let e = solve(&g, Rect::new(0, 0, 10000, 10000), &Options::default()).unwrap_err();
        assert!(e.contains("budget"));
    }
    #[test]
    fn weights_influence_the_joint_fit_without_local_maps() {
        let (r, mut g) = fixture();
        g[0].corners[0][0] += 0.5;
        g[0].weight = 0.05;
        let a = solve(&g, r, &Options::default()).unwrap();
        g[0].weight = 10.0;
        let b = solve(&g, r, &Options::default()).unwrap();
        assert_ne!(a.matrix, b.matrix);
        assert_eq!(a.used_guide_ids, b.used_guide_ids);
        assert!(b.guides[0].max_error_deg.unwrap() < a.guides[0].max_error_deg.unwrap());
    }
    #[test]
    fn nearly_affine_vanishing_points_need_no_division_by_infinity() {
        let (r, mut g) = fixture();
        let near = Homography([1.0, 0.001, 0.0, -0.001, 1.0, 0.0, 1e-10, -1e-10, 1.0]);
        for (i, guide) in g.iter_mut().enumerate() {
            let x = 80.0 + (i % 2) as f64 * 400.0;
            let y = 70.0 + (i / 2) as f64 * 240.0;
            guide.corners = mapped(&near, &[[x, y], [x + 90.0, y], [x + 90.0, y + 150.0], [x, y + 150.0]]);
        }
        let s = solve(&g, r, &Options::default()).unwrap();
        assert_eq!(s.used_guide_ids.len(), 6);
        assert!(s.max_used_error_deg < 1e-6);
    }
}
