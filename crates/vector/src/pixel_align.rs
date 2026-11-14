//! Render-only alignment of shape edges and straight one-pixel stroke runs. Stored
//! geometry and dash settings remain unchanged; curved interiors retain area antialiasing.
use photocraft_doc::{LiveShape, Path, ShapeLayer, StrokeAlign};
use photocraft_geom::Rect;

// Trigonometric live shapes and quarter-turn transforms carry tiny rounding errors.
const ALIGN_EPSILON: f64 = 1e-7;
/// Round an edge to the nearest pixel, choosing increasing coordinates at half ties.
/// Comparing the fractional part avoids addition rounding just below a half pixel.
pub(crate) fn snap_edge(coordinate: f64) -> f64 {
    let lower = coordinate.floor();
    if coordinate - lower >= 0.5 { lower + 1.0 } else { lower }
}

type CleanLine = (Vec<(f64, f64)>, Vec<bool>);
/// Compare source coordinates without treating flattened curve chords as straight segments.
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= ALIGN_EPSILON
}

/// A PSD path is rounded to signed 8.24 fractions of the document dimensions. Only
/// imported PSD geometry gets its half-quantum allowance; native paths keep the strict epsilon.
pub(crate) fn coordinate_tolerance(shape: &ShapeLayer, _canvas: Rect) -> (f64, f64) {
    let [width, height] = match shape.psd_coordinate_frame {
        Some(frame) => frame,
        // Import and native-load migration supply provenance. A render ROI cannot.
        None => return (ALIGN_EPSILON, ALIGN_EPSILON),
    };
    // Valid PSB dimensions need less than 0.009 px; cap hostile oversized frames too.
    let epsilon = |side: u32| (f64::from(side) / 33_554_432.0).clamp(ALIGN_EPSILON, 1.0 / 64.0);
    (epsilon(width), epsilon(height))
}

#[derive(Default)]
pub(crate) struct PixelRuns(Vec<Run>);
struct Run {
    horizontal: bool,
    coordinate: i32,
    negative_side: bool,
    sample_offset: f64,
    start: f64,
    end: f64,
}
impl PixelRuns {
    /// Quantize only dash cap coverage on isolated axis runs. Butt caps sample the
    /// compiled outline at pixel centers; extended caps retain area selection. Sampling neighbors
    /// resolves an exact half-pixel tie consistently: round both boundaries toward
    /// increasing document coordinates. A 9px dash therefore stays 9 opaque pixels,
    /// including when its phase alternates between integer and half-integer starts.
    /// Ambiguous equal-neighbor ties probe a positively biased copy of the compiled edges;
    /// this distinguishes alternating one-pixel dashes whose area samples are all one half.
    pub(crate) fn sharpen_dash_caps(
        &self,
        rect: Rect,
        coverage: &mut [f32],
        threshold: f32,
        rasterizer: &crate::raster::Rasterizer,
        biased_rasterizer: Option<&crate::raster::Rasterizer>,
        sample_centers: bool,
    ) {
        let width = rect.width() as usize;
        for run in &self.0 {
            let lo = run.start.ceil() as i32;
            let hi = run.end.floor() as i32;
            let c = run.coordinate;
            let strip = if run.horizontal { Rect::new(lo, c - 1, hi, c + 1) } else { Rect::new(c - 1, lo, c + 1, hi) };
            let visible = strip.intersect(&rect);
            if visible.is_empty() {
                continue;
            }
            // Probe the original outline at the centerline and along-axis pixel centers.
            // The small negative bias preserves the existing positive-coordinate tie policy.
            let centers_rect = if run.horizontal { Rect::new(visible.x0, c, visible.x1, c + 1) } else { Rect::new(c, visible.y0, c + 1, visible.y1) };
            let offset = if run.horizontal { (0.5 - 1e-6, run.sample_offset) } else { (run.sample_offset, 0.5 - 1e-6) };
            if sample_centers {
                let centers = rasterizer.point_samples(centers_rect, offset);
                for y in visible.y0..visible.y1 {
                    for x in visible.x0..visible.x1 {
                        let index = (y - rect.y0) as usize * width + (x - rect.x0) as usize;
                        let sample = if run.horizontal { x - visible.x0 } else { y - visible.y0 } as usize;
                        if let Some(out) = coverage.get_mut(index).filter(|out| **out > 0.0) {
                            *out = centers.get(sample).copied().unwrap_or(0.0);
                        }
                    }
                }
                continue;
            }
            let source = if run.horizontal {
                Rect::new(visible.x0.saturating_sub(1), c - 1, visible.x1.saturating_add(1), c + 1)
            } else {
                Rect::new(c - 1, visible.y0.saturating_sub(1), c + 1, visible.y1.saturating_add(1))
            };
            let values = rasterizer.render(source);
            let mut biased = None;
            let sw = source.width() as usize;
            let sum = |p: i32| {
                let k = if run.horizontal { (p - source.x0) as usize } else { (p - source.y0) as usize * sw };
                let adjacent = if run.horizontal { k + sw } else { k + 1 };
                values.get(k).copied().unwrap_or(0.0) + values.get(adjacent).copied().unwrap_or(0.0)
            };
            for y in visible.y0..visible.y1 {
                for x in visible.x0..visible.x1 {
                    let index = (y - rect.y0) as usize * width + (x - rect.x0) as usize;
                    let Some(out) = coverage.get_mut(index) else {
                        continue;
                    };
                    // The normal alignment / rectangle clip already selects the row.
                    if *out <= 0.0 {
                        continue;
                    }
                    let p = if run.horizontal { x } else { y };
                    let value = sum(p);
                    *out = if (value - threshold).abs() <= 1e-5 {
                        let before = sum(p.saturating_sub(1));
                        let after = sum(p.saturating_add(1));
                        if (before - after).abs() > 1e-5 {
                            f32::from(before > after)
                        } else {
                            let values = biased.get_or_insert_with(|| biased_rasterizer.map_or_else(Vec::new, |r| r.render(source)));
                            let k = if run.horizontal { (p - source.x0) as usize } else { (p - source.y0) as usize * sw };
                            let adjacent = if run.horizontal { k + sw } else { k + 1 };
                            f32::from(values.get(k).copied().unwrap_or(0.0) + values.get(adjacent).copied().unwrap_or(0.0) > threshold)
                        }
                    } else {
                        f32::from(value > threshold)
                    };
                }
            }
        }
    }

    /// Whether any isolated source run can use coverage alignment.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Keep centered one-pixel source runs isolated from neighboring geometry and joins.
    pub(crate) fn stroke(shape: &ShapeLayer, tol: f64, coordinate_tolerance: (f64, f64)) -> Self {
        if !shape.use_aligned_rendering || !shape.stroke.as_ref().is_some_and(|s| s.width == 1.0 && s.align == StrokeAlign::Center) {
            return Self::default();
        }
        let mut runs = Self::path(&shape.path, coordinate_tolerance);
        if runs.0.is_empty() {
            return runs;
        }
        let Some(joins) = join_bounds(shape, tol) else {
            return Self::default();
        };
        runs.0.retain(|run| {
            joins.iter().all(|(p, radius)| {
                let (along, normal) = if run.horizontal { (p.0, p.1) } else { (p.1, p.0) };
                along + radius <= run.start
                    || along - radius >= run.end
                    || normal + radius <= f64::from(run.coordinate) - 1.0
                    || normal - radius >= f64::from(run.coordinate) + 1.0
            })
        });
        runs
    }
    /// Leave endpoint guards on an integer-axis segment long enough to have a safe interior.
    fn segment(a: (f64, f64), b: (f64, f64), tolerance: (f64, f64)) -> Self {
        let horizontal = same(a.1, b.1);
        if !horizontal && !same(a.0, b.0) {
            return Self::default();
        }
        let (coordinate, start, end) = if horizontal { (a.1, a.0.min(b.0), a.0.max(b.0)) } else { (a.0, a.1.min(b.1), a.1.max(b.1)) };
        if !coordinate.is_finite()
            || (coordinate - coordinate.round()).abs() > if horizontal { tolerance.1 } else { tolerance.0 }
            // Normal-axis sampling needs both adjacent integer rows/columns.
            || coordinate.round() <= f64::from(i32::MIN)
            || coordinate.round() >= f64::from(i32::MAX)
            || !start.is_finite()
            || !end.is_finite()
            || end - start < 5.0
        {
            return Self::default();
        }
        Self(vec![Run { horizontal, coordinate: coordinate.round() as i32, negative_side: false, sample_offset: 0.0, start: start + 2.0, end: end - 2.0 }])
    }
    /// Find straight source segments with bounded overlap checks; curves retain normal coverage.
    fn path(path: &Path, coordinate_tolerance: (f64, f64)) -> Self {
        // Alignment is optional. Complex artwork retains its normal bounded renderer;
        // do not add quadratic compilation work to arbitrary imported paths.
        if path.subpaths.iter().map(|s| s.knots.len()).sum::<usize>() > 256 {
            return Self::default();
        }
        let segments: Vec<_> = path.subpaths.iter().flat_map(|s| s.segments()).collect();
        let mut runs = Self::default();
        for (i, [a, c1, c2, b]) in segments.iter().enumerate() {
            if !((same(a.x, b.x) && same(c1.x, a.x) && same(c2.x, a.x)) || (same(a.y, b.y) && same(c1.y, a.y) && same(c2.y, a.y))) {
                continue;
            }
            let candidate = Self::segment((a.x, a.y), (b.x, b.y), coordinate_tolerance);
            for run in candidate.0 {
                // Coverage belongs to the entire stroke. Leave crossing/overlapping
                // geometry alone rather than moving another segment's antialiasing.
                let isolated = segments.iter().enumerate().all(|(j, points)| i == j || isolated(&run, points, 0.75));
                if isolated {
                    runs.0.push(run);
                }
            }
        }
        runs
    }
    /// Probe dash centers on the stored segment even when its normal coordinate snapped.
    /// Run discovery already caps the source to 256 knots and excludes overlaps.
    pub(crate) fn use_source_coordinates(&mut self, path: &Path) {
        if self.is_empty() {
            return;
        }
        let segments: Vec<_> = path.subpaths.iter().flat_map(|s| s.segments()).collect();
        for run in &mut self.0 {
            for points in &segments {
                let coordinate = if run.horizontal { points[0].y } else { points[0].x };
                let on_axis = points.iter().all(|p| same(if run.horizontal { p.y } else { p.x }, coordinate));
                let (a, b) = if run.horizontal { (points[0].x, points[3].x) } else { (points[0].y, points[3].y) };
                if on_axis && snap_edge(coordinate) == f64::from(run.coordinate) && a.min(b) <= run.start && a.max(b) >= run.end {
                    run.sample_offset = coordinate - f64::from(run.coordinate);
                    break;
                }
            }
        }
    }

    /// Rectangle strokes select the interior side on the right and bottom edges.
    pub(crate) fn select_rectangle_interior(&mut self, (x0, y0, x1, y1): (f64, f64, f64, f64)) {
        for run in &mut self.0 {
            let (low, high) = if run.horizontal { (y0, y1) } else { (x0, x1) };
            run.negative_side = f64::from(run.coordinate) == snap_edge(high);
            run.sample_offset = if run.negative_side { high } else { low } - f64::from(run.coordinate);
        }
    }

    /// Combine normal-axis coverage in the selected pixel, sampling beyond tile boundaries.
    pub(crate) fn apply(&self, rect: Rect, coverage: &mut [f32], sample: impl Fn(Rect) -> Vec<f32>) {
        let width = rect.width() as usize;
        for run in &self.0 {
            let lo = run.start.ceil() as i32;
            let hi = run.end.floor() as i32;
            let c = run.coordinate;
            let strip = if run.horizontal { Rect::new(lo, c - 1, hi, c + 1) } else { Rect::new(c - 1, lo, c + 1, hi) };
            // Sample both rows even when the caller renders tiles/bands split at the run.
            let visible = strip.intersect(&rect);
            if visible.is_empty() {
                continue;
            }
            let source = if run.horizontal { Rect::new(visible.x0, c - 1, visible.x1, c + 1) } else { Rect::new(c - 1, visible.y0, c + 1, visible.y1) };
            let values = sample(source);
            let sw = source.width() as usize;
            for y in visible.y0..visible.y1 {
                for x in visible.x0..visible.x1 {
                    let index = (y - rect.y0) as usize * width + (x - rect.x0) as usize;
                    let k = if run.horizontal { (x - source.x0) as usize } else { (y - source.y0) as usize * sw };
                    let adjacent = if run.horizontal { k + sw } else { k + 1 };
                    if let (Some(out), Some(a), Some(b)) = (coverage.get_mut(index), values.get(k), values.get(adjacent)) {
                        *out = if (if run.horizontal { y } else { x }) == c - i32::from(run.negative_side) { (*a + *b).min(1.0) } else { 0.0 };
                    }
                }
            }
        }
    }
}

/// A Line Tool bar is a fill, so translate its render-only copy by half a pixel.
/// Moving the whole arrow also retains antialiasing and the proportions of its heads.
pub(crate) fn line_fill_path(shape: &ShapeLayer) -> Option<Path> {
    // A completely transparent solid outline paints nothing and must not prevent
    // alignment of the live bar. Other paints retain conservative legacy placement.
    let visible_outline = shape
        .stroke
        .as_ref()
        .is_some_and(|s| s.width > 0.0 && s.opacity > 0.0 && !matches!(&s.paint, photocraft_doc::Fill::Solid(color) if color.alpha <= 0.0));
    if !shape.use_aligned_rendering || shape.has_soft_mask() || visible_outline {
        return None;
    }
    let live = shape.live.as_ref()?;
    let (from, to, weight) = match live {
        LiveShape::Line { from, to, weight } | LiveShape::ArrowLine { from, to, weight, .. } => (from, to, weight),
        _ => return None,
    };
    if *weight != 1.0 || !equivalent_live_path(&crate::shapes::live_path(live), &shape.path) {
        return None;
    }
    let horizontal = same(from[1], to[1]) && !same(from[0], to[0]);
    let vertical = same(from[0], to[0]) && !same(from[1], to[1]);
    let coordinate = if horizontal { from[1] } else { from[0] };
    if (!horizontal && !vertical) || !coordinate.is_finite() || !same(coordinate, coordinate.round()) {
        return None;
    }
    Some(shape.path.transform(&photocraft_geom::Affine::translate(if vertical { 0.5 } else { 0.0 }, if horizontal { 0.5 } else { 0.0 })))
}

/// Reflections reverse closed outlines and can change their first knot. Compare all
/// controls too, so stale live metadata cannot qualify a different filled shape.
fn equivalent_live_path(expected: &Path, stored: &Path) -> bool {
    if expected == stored {
        return true;
    }
    if expected.fill_rule != stored.fill_rule || expected.inverted != stored.inverted || expected.subpaths.len() != stored.subpaths.len() {
        return false;
    }
    expected.subpaths.iter().zip(&stored.subpaths).all(|(a, b)| {
        if !a.closed || !b.closed || a.op != b.op || a.knots.len() != b.knots.len() {
            return false;
        }
        let matching = |left: &[photocraft_doc::Knot], right: &[photocraft_doc::Knot]| {
            left.iter().zip(right).all(|(l, r)| {
                l.smooth == r.smooth
                    && [l.anchor, l.in_ctrl, l.out_ctrl].into_iter().zip([r.anchor, r.in_ctrl, r.out_ctrl]).all(|(p, q)| same(p.x, q.x) && same(p.y, q.y))
            })
        };
        if matching(&a.knots, &b.knots) {
            return true;
        }
        let mut reversed = a.knots.clone();
        reversed.reverse();
        for knot in &mut reversed {
            std::mem::swap(&mut knot.in_ctrl, &mut knot.out_ctrl);
        }
        for _ in 0..reversed.len() {
            if matching(&reversed, &b.knots) {
                return true;
            }
            reversed.rotate_left(1);
        }
        false
    })
}

/// Reject a run when another source segment or its control bounds enter its coverage strip.
fn isolated(run: &Run, points: &[photocraft_geom::Point; 4], padding: f64) -> bool {
    let c = f64::from(run.coordinate);
    let [a, c1, c2, b] = points;
    if a == c1 && b == c2 {
        let (n0, n1, t0, t1) = if run.horizontal { (a.y, b.y, a.x, b.x) } else { (a.x, b.x, a.y, b.y) };
        let delta = n1 - n0;
        if delta != 0.0 {
            let u = (c - 1.0 - padding - n0) / delta;
            let v = (c + 1.0 + padding - n0) / delta;
            let lo = u.min(v).max(0.0);
            let hi = u.max(v).min(1.0);
            if lo > hi {
                return true;
            }
            let p = t0 + (t1 - t0) * lo;
            let q = t0 + (t1 - t0) * hi;
            return p.max(q) + padding <= run.start || p.min(q) - padding >= run.end;
        }
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if run.horizontal {
        x1 + padding <= run.start || x0 - padding >= run.end || y1 + padding <= c - 1.0 || y0 - padding >= c + 1.0
    } else {
        y1 + padding <= run.start || y0 - padding >= run.end || x1 + padding <= c - 1.0 || x0 - padding >= c + 1.0
    }
}

/// Recognize the same rectangle after a reversed traversal or quarter-turn transform.
/// The stored path still determines stroke geometry and dash phase.
pub(crate) fn integer_rectangle(path: &Path, tolerance: (f64, f64)) -> bool {
    if path.inverted || path.subpaths.len() != 1 {
        return false;
    }
    let Some(s) = path.subpaths.first() else {
        return false;
    };
    if !s.closed || s.knots.len() != 4 {
        return false;
    }
    let Some((x0, y0, x1, y1)) = path.control_bounds() else {
        return false;
    };
    let near_integer = |coordinate: f64, epsilon: f64| coordinate.is_finite() && (coordinate - coordinate.round()).abs() <= epsilon;
    if x1.round() - x0.round() < 2.0
        || y1.round() - y0.round() < 2.0
        || ![(x0, tolerance.0), (x1, tolerance.0), (y0, tolerance.1), (y1, tolerance.1)].iter().all(|(value, epsilon)| near_integer(*value, *epsilon))
    {
        return false;
    }
    let mut corners = 0u8;
    for k in &s.knots {
        corners |= 1 << (u8::from(same(k.anchor.x, x1)) + 2 * u8::from(same(k.anchor.y, y1)));
        if !(same(k.anchor.x, x0) || same(k.anchor.x, x1)) || !(same(k.anchor.y, y0) || same(k.anchor.y, y1)) || k.in_ctrl != k.anchor || k.out_ctrl != k.anchor
        {
            return false;
        }
    }
    corners == 15 && s.segments().iter().all(|[a, _, _, b]| (same(a.x, b.x) && !same(a.y, b.y)) || (same(a.y, b.y) && !same(a.x, b.x)))
}

/// The original bounds of an eligible rectangle; only its render-time clipping mask is rounded.
pub(crate) fn rectangle_bounds(shape: &ShapeLayer, tolerance: (f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let stroke = shape.stroke.as_ref()?;
    if !shape.use_aligned_rendering || stroke.width != 1.0 || stroke.align != StrokeAlign::Center || !integer_rectangle(&shape.path, tolerance) {
        return None;
    }
    shape.path.control_bounds()
}

/// Normalize the selected interior row using its actual normal-axis coverage. This keeps
/// quantized PSD edges opaque without rounding the path used for dash lengths and phase.
pub(crate) fn rectangle_coverage_scale((x0, y0, x1, y1): (f64, f64, f64, f64), x: i32, y: i32) -> f32 {
    let mut normal: f64 = 1.0;
    for (pixel, edge, inside_positive) in [(x, x0, true), (x, x1, false), (y, y0, true), (y, y1, false)] {
        let rounded = snap_edge(edge);
        let row = (rounded as i32).saturating_sub(i32::from(!inside_positive));
        if pixel == row {
            normal = normal.min(0.5 + (edge - rounded) * if inside_positive { 1.0 } else { -1.0 });
        }
    }
    (1.0 / normal) as f32
}

/// Bound actual tessellated joins, including sharp interior cusps. Source control
/// bounds alone do not contain a long miter. Use the stroker's flattening tolerance
/// and duplicate-point policy, without modifying its path or dash calculations.
fn join_bounds(shape: &ShapeLayer, tol: f64) -> Option<Vec<((f64, f64), f64)>> {
    let style = crate::stroke_style(shape.stroke.as_ref()?);
    if !style.miter_limit.is_finite() {
        return None;
    }
    let flatten_tol = if tol.is_nan() { 0.1 } else { (tol * 4.0).clamp(1e-3, 0.1) };
    let lines = crate::flatten_path(&shape.path, flatten_tol);
    if lines.iter().map(|l| l.pts.len()).sum::<usize>() > 8192 {
        return None;
    }
    let mut bounds = Vec::new();
    for line in lines {
        let (points, knots) = clean_line(&line)?;
        let n = points.len();
        if n < 3 {
            bounds.extend(points.into_iter().map(|p| (p, 0.75)));
            continue;
        }
        let closed = line.closed;
        let range = if closed { 0..n } else { 1..n - 1 };
        for i in range {
            let a = *points.get((i + n - 1) % n)?;
            let b = *points.get(i)?;
            let c = *points.get((i + 1) % n)?;
            let unit = |x: f64, y: f64| {
                let length = x.hypot(y);
                (length > 0.0 && length.is_finite()).then(|| (x / length, y / length))
            };
            let u = unit(b.0 - a.0, b.1 - a.1)?;
            let v = unit(c.0 - b.0, c.1 - b.1)?;
            let cosine = ((1.0 + u.0 * v.0 + u.1 * v.1).clamp(0.0, 2.0) * 0.5).sqrt();
            let limit = if knots.get(i).copied().unwrap_or(true) { style.miter_limit.min(500.0) } else { f64::INFINITY };
            // Bevel/round joins may be smaller; this conservative envelope also
            // bounds the miter vertices of continuous curve ribbons.
            let radius = if cosine >= 1e-9 && 1.0 / cosine <= limit { (0.5 / cosine).max(0.75) } else { 0.75 };
            bounds.push((b, radius));
        }
    }
    Some(bounds)
}
/// Match the stroker's duplicate-point policy while retaining source-knot join flags.
fn clean_line(line: &crate::Polyline) -> Option<CleanLine> {
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut knots = Vec::new();
    for (i, p) in line.pts.iter().enumerate() {
        if !p.0.is_finite() || !p.1.is_finite() {
            return None;
        }
        if points.last().is_some_and(|q| (p.0 - q.0).hypot(p.1 - q.1) < 1e-9) {
            continue;
        }
        points.push(*p);
        knots.push(line.knot.get(i).copied().unwrap_or(true));
    }
    if line.closed && points.len() > 1 && points.first().zip(points.last()).is_some_and(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1) < 1e-9) {
        points.pop();
        knots.pop();
    }
    Some((points, knots))
}

/// Snap source axis-aligned edges on a render-only path. Moving a knot's handles
/// with its anchor preserves the adjoining curve's tangent. Diagonals and curves
/// are not inferred from tessellated chords.
pub(crate) fn aligned_edge_path(shape: &ShapeLayer) -> Option<Path> {
    if !shape.use_aligned_rendering || line_fill_path(shape).is_some() {
        return None;
    }
    let mut path = shape.path.clone();
    for subpath in &mut path.subpaths {
        let source = subpath.knots.clone();
        let count = source.len();
        if count < 2 {
            continue;
        }
        let mut axes = vec![(false, false); count];
        // A vertical tangent identifies a horizontal extremum, and conversely.
        // Snap its normal coordinate, moving handles with the anchor; curve
        // interiors keep area antialiasing instead of thresholding their chords.
        for (index, knot) in source.iter().enumerate() {
            let curved = knot.in_ctrl != knot.anchor || knot.out_ctrl != knot.anchor;
            axes[index] = (
                curved && same(knot.in_ctrl.x, knot.anchor.x) && same(knot.out_ctrl.x, knot.anchor.x),
                curved && same(knot.in_ctrl.y, knot.anchor.y) && same(knot.out_ctrl.y, knot.anchor.y),
            );
        }
        let segments = if subpath.closed { count } else { count - 1 };
        for index in 0..segments {
            let next = (index + 1) % count;
            let a = source[index];
            let b = source[next];
            let points = [a.anchor, a.out_ctrl, b.in_ctrl, b.anchor];
            let vertical = points.iter().all(|point| same(point.x, a.anchor.x));
            let horizontal = points.iter().all(|point| same(point.y, a.anchor.y));
            for knot in [index, next] {
                axes[knot].0 |= vertical;
                axes[knot].1 |= horizontal;
            }
        }
        for ((target, original), (vertical, horizontal)) in subpath.knots.iter_mut().zip(source).zip(axes) {
            if vertical && original.anchor.x.is_finite() {
                let snapped = snap_edge(original.anchor.x);
                let delta = snapped - original.anchor.x;
                target.anchor.x = snapped;
                target.in_ctrl.x = original.in_ctrl.x + delta;
                target.out_ctrl.x = original.out_ctrl.x + delta;
            }
            if horizontal && original.anchor.y.is_finite() {
                let snapped = snap_edge(original.anchor.y);
                let delta = snapped - original.anchor.y;
                target.anchor.y = snapped;
                target.in_ctrl.y = original.in_ctrl.y + delta;
                target.out_ctrl.y = original.out_ctrl.y + delta;
            }
        }
    }
    (path != shape.path).then_some(path)
}
