//! Align Edges (a shape layer's `use_aligned_rendering`): at render time the whole shape is fitted
//! to whole document pixels, as the reference application does.
//!
//! The rule, measured in the reference application: take the bounds of what is painted (the fill
//! area, or the stroke's outer edge when there is a stroke), round the left/top edge to the nearest
//! whole pixel (a half rounds up) and the width/height to whole pixels, then translate and scale the
//! whole path so its painted bounds fill that box. Fill and stroke use the same transform. Curves
//! keep their antialiasing, diagonals just move with the shape, and inner edges of a shape with
//! several subpaths are not snapped one by one. The stored path is never changed: turning the flag
//! off renders the original coverage again.

use photocraft_doc::{Path, ShapeLayer, StrokeAlign};
use photocraft_geom::Affine;

use crate::{flatten_path, stroke_polygons, stroke_style};

/// `(x0, y0, x1, y1)`.
type Bounds = (f64, f64, f64, f64);

/// Bounds `(x0, y0, x1, y1)` of a set of points; `None` when empty or not finite.
fn points_bounds<'a>(pts: impl Iterator<Item = &'a (f64, f64)>) -> Option<Bounds> {
    let mut b: Option<Bounds> = None;
    for &(x, y) in pts {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        b = Some(match b {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    b
}

/// Bounds of the path's own geometry (`path`) and of everything the shape paints (`painted`):
/// the fill area, grown to the stroke's outer edge when the shape has a visible stroke.
fn painted_bounds(shape: &ShapeLayer, tol: f64) -> Option<(Bounds, Bounds)> {
    let lines = flatten_path(&shape.path, tol);
    let path = points_bounds(lines.iter().flat_map(|l| l.pts.iter()))?;
    let stroke = shape.stroke.as_ref().filter(|s| s.width > 0.0 && s.opacity > 0.0);
    if shape.fill.is_none() && stroke.is_none() {
        return None;
    }
    let Some(s) = stroke else { return Some((path, path)) };
    let all_closed = shape.path.subpaths.iter().all(|sp| sp.closed);
    let mut style = stroke_style(s);
    if all_closed && s.align == StrokeAlign::Inside {
        // An inside stroke is clipped by the fill area: nothing is painted beyond the path.
        return Some((path, path));
    }
    if all_closed && s.align == StrokeAlign::Outside {
        // As the renderer does: a centred outline twice as wide, of which the outer half shows.
        style.width *= 2.0;
    }
    let outline = stroke_polygons(&lines, &style, tol);
    let outer = points_bounds(outline.iter().flatten()).map_or(path, |o| (o.0.min(path.0), o.1.min(path.1), o.2.max(path.2), o.3.max(path.3)));
    Some((path, outer))
}

/// One axis: the scale and offset that map the path's `lo..hi` so the painted `plo..phi` lands on
/// whole pixels. `None` for non-finite input.
fn fit_axis(lo: f64, hi: f64, plo: f64, phi: f64) -> Option<(f64, f64)> {
    let size = phi - plo;
    // A half rounds up (10.5 -> 11, 20.5 -> 21), as measured.
    let start = (plo + 0.5).floor();
    let mut target = (size + 0.5).floor();
    if target < 1.0 {
        // Thinner than half a pixel: keep its size rather than collapse it to nothing.
        target = size;
    }
    // The path's own extent inside the painted box (the stroke's reach on either side stays as wide).
    let (before, after) = (lo - plo, phi - hi);
    let (span, new_span) = (hi - lo, target - before - after);
    let scale = if span > 1e-9 && new_span > 0.0 { new_span / span } else { 1.0 };
    let offset = start + before - lo * scale;
    (scale.is_finite() && offset.is_finite()).then_some((scale, offset))
}

/// The path the shape renders with Align Edges on: the stored path translated and scaled so its
/// painted bounds sit on whole pixels. `None` when the flag is off, nothing is painted, the path
/// is inverted (it paints everywhere) or already aligned, so callers render the stored path.
pub fn aligned_path(shape: &ShapeLayer, tol: f64) -> Option<Path> {
    if !shape.use_aligned_rendering || shape.path.inverted {
        return None;
    }
    let (path, painted) = painted_bounds(shape, tol)?;
    let (sx, tx) = fit_axis(path.0, path.2, painted.0, painted.2)?;
    let (sy, ty) = fit_axis(path.1, path.3, painted.1, painted.3)?;
    if sx == 1.0 && sy == 1.0 && tx == 0.0 && ty == 0.0 {
        return None;
    }
    Some(shape.path.transform(&Affine { m: [sx, 0.0, 0.0, sy, tx, ty] }))
}

/// The shape as Align Edges renders it: [`aligned_path`] in place of the stored path, the flag
/// cleared so the result renders as it is. `None` when nothing moves. Code that renders the fill
/// and the stroke separately (the compositor's split for layer effects) must start from this, so
/// both parts take the transform of the whole shape, not each their own.
pub fn aligned_shape(shape: &ShapeLayer, tol: f64) -> Option<ShapeLayer> {
    let path = aligned_path(shape, tol)?;
    Some(ShapeLayer {
        path,
        fill: shape.fill.clone(),
        stroke: shape.stroke.clone(),
        use_aligned_rendering: false,
        live: None,
        cache: None,
        psd_raw: None,
        density: shape.density,
        feather: shape.feather,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, Fill, ShapeStroke};

    fn rect_shape(x: f64, y: f64, w: f64, h: f64) -> ShapeLayer {
        ShapeLayer { path: crate::shapes::rect(x, y, w, h), fill: Some(Fill::Solid(Color::BLACK)), use_aligned_rendering: true, ..Default::default() }
    }

    fn bounds(p: &Path) -> (f64, f64, f64, f64) {
        p.control_bounds().unwrap()
    }

    #[test]
    fn the_flag_off_renders_the_stored_path() {
        let shape = ShapeLayer { use_aligned_rendering: false, ..rect_shape(10.5, 10.5, 20.0, 20.0) };
        assert!(aligned_path(&shape, 0.01).is_none());
    }

    #[test]
    fn a_half_rounds_up_and_the_size_rounds() {
        let b = bounds(&aligned_path(&rect_shape(10.5, 10.5, 20.0, 20.0), 0.01).unwrap());
        assert_eq!(b, (11.0, 11.0, 31.0, 31.0));
        let b = bounds(&aligned_path(&rect_shape(10.25, 10.0, 20.5, 20.0), 0.01).unwrap());
        assert!((b.0 - 10.0).abs() < 1e-9 && (b.2 - 31.0).abs() < 1e-9, "{b:?}");
        let b = bounds(&aligned_path(&rect_shape(-10.5, -3.5, 20.0, 2.0), 0.01).unwrap());
        assert_eq!((b.0, b.1), (-10.0, -3.0), "a half rounds up below zero too");
    }

    #[test]
    fn an_already_aligned_shape_is_left_alone() {
        assert!(aligned_path(&rect_shape(10.0, 10.0, 20.0, 20.0), 0.01).is_none());
        let stroked = ShapeLayer { stroke: Some(ShapeStroke { width: 1.0, ..Default::default() }), ..rect_shape(10.5, 10.5, 20.0, 20.0) };
        assert!(aligned_path(&stroked, 0.01).is_none(), "the stroke's outer edge 10..31 is already whole");
    }

    #[test]
    fn inside_and_outside_strokes_reach_as_far_as_they_paint() {
        let with = |align, width| ShapeLayer { stroke: Some(ShapeStroke { width, align, ..Default::default() }), ..rect_shape(10.5, 10.5, 20.0, 20.0) };
        // Inside: painted bounds are the path's, 10.5 -> 11.
        assert_eq!(bounds(&aligned_path(&with(StrokeAlign::Inside, 2.0), 0.01).unwrap()).0, 11.0);
        // Outside 1 px: painted 9.5..31.5 -> 10..32, the path moves to 11..31.
        assert_eq!(bounds(&aligned_path(&with(StrokeAlign::Outside, 1.0), 0.01).unwrap()).0, 11.0);
        // Outside 2 px: painted 8.5..32.5 -> 9..33, the path moves to 11..31.
        assert_eq!(bounds(&aligned_path(&with(StrokeAlign::Outside, 2.0), 0.01).unwrap()).0, 11.0);
    }

    #[test]
    fn nothing_painted_inverted_or_hostile_input_renders_unchanged() {
        let unpainted = ShapeLayer { fill: None, stroke: None, ..rect_shape(10.5, 10.5, 20.0, 20.0) };
        assert!(aligned_path(&unpainted, 0.01).is_none());
        let mut inverted = rect_shape(10.5, 10.5, 20.0, 20.0);
        inverted.path.inverted = true;
        assert!(aligned_path(&inverted, 0.01).is_none());
        assert!(aligned_path(&rect_shape(f64::NAN, 1.0, 2.0, 2.0), 0.01).is_none());
        assert!(aligned_path(&rect_shape(1.0, 1.0, f64::INFINITY, 2.0), 0.01).is_none());
        assert!(aligned_path(&ShapeLayer { use_aligned_rendering: true, fill: Some(Fill::Solid(Color::BLACK)), ..Default::default() }, 0.01).is_none());
        // Huge but finite coordinates must not panic.
        let _ = aligned_path(&rect_shape(1e300, -1e300, 1e300, 1e-300), 0.01);
    }

    #[test]
    fn a_sliver_keeps_its_size_and_a_line_just_moves() {
        let b = bounds(&aligned_path(&rect_shape(10.6, 10.0, 0.3, 20.0), 0.01).unwrap());
        assert!((b.2 - b.0 - 0.3).abs() < 1e-9 && b.0 == 11.0, "{b:?}");
        // A zero-width path (a vertical line) with a 1 px stroke: painted 9.75..10.75 -> 10..11.
        let line = ShapeLayer {
            path: Path::new(vec![photocraft_doc::Subpath::polyline(&[(10.25, 10.0), (10.25, 30.0)])]),
            fill: None,
            stroke: Some(ShapeStroke { width: 1.0, ..Default::default() }),
            use_aligned_rendering: true,
            ..Default::default()
        };
        let b = bounds(&aligned_path(&line, 0.01).unwrap());
        assert_eq!((b.0, b.2), (10.5, 10.5));
    }
}
