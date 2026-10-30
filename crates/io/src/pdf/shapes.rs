//! Faithful vector subset for PDF shape export; unsupported shapes use raster fallback.

use photocraft_color::ColorMode;
use photocraft_doc::{Fill, FillRule, PathOp, ShapeLayer};

/// Return a PDF path and fill operator for a directly representable shape.
/// Coordinates remain in document pixels with y increasing down; the caller applies transforms.
pub(super) fn shape_path(layer: &ShapeLayer, mode: ColorMode) -> Option<String> {
    if layer.stroke.is_some() || layer.has_soft_mask() || layer.path.inverted || layer.path.subpaths.len() != 1 {
        return None;
    }
    let Fill::Solid(color) = layer.fill.as_ref()? else { return None };
    let channels = match mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        _ => return None,
    };
    if color.mode != mode || !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite()) {
        return None;
    }
    let subpath = layer.path.subpaths.first()?;
    if !subpath.closed || subpath.knots.len() < 2 || subpath.op != PathOp::Combine {
        return None;
    }
    let mut out = String::new();
    use std::fmt::Write;
    for value in color.c.iter().take(channels) {
        if !(0.0..=1.0).contains(value) {
            return None;
        }
        write!(out, "{value} ").ok()?;
    }
    out.push_str(match mode {
        ColorMode::Rgb => "rg\n",
        ColorMode::Grayscale => "g\n",
        ColorMode::Cmyk => "k\n",
        _ => return None,
    });
    let first = subpath.knots.first()?;
    if !point_finite(first.anchor) {
        return None;
    }
    write!(out, "{} {} m\n", first.anchor.x, first.anchor.y).ok()?;
    for segment in subpath.segments() {
        if segment.iter().any(|p| !point_finite(*p)) {
            return None;
        }
        write!(out, "{} {} {} {} {} {} c\n", segment[1].x, segment[1].y, segment[2].x, segment[2].y, segment[3].x, segment[3].y).ok()?;
    }
    out.push_str("h\n");
    out.push_str(match layer.path.fill_rule {
        FillRule::NonZero => "f\n",
        FillRule::EvenOdd => "f*\n",
    });
    Some(out)
}

fn point_finite(p: photocraft_geom::Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Knot, Path, Subpath};
    use photocraft_geom::Point;

    #[test]
    fn closed_cubic_emits_path_and_fill() {
        let mut layer = ShapeLayer::default();
        layer.fill = Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0)));
        layer.path = Path::new(vec![Subpath {
            closed: true,
            knots: vec![Knot::smooth(Point::new(0.0, 0.0), Point::new(-1.0, 0.0), Point::new(1.0, 0.0)), Knot::corner(10.0, 0.0)],
            ..Subpath::default()
        }]);
        let pdf = shape_path(&layer, ColorMode::Rgb).unwrap();
        assert!(pdf.contains("1 0 10 0 10 0 c"));
        assert!(pdf.ends_with("h\nf\n"));
    }

    #[test]
    fn rejects_open_path_and_mismatched_mode() {
        let mut layer = ShapeLayer::default();
        layer.fill = Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0)));
        layer.path = Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (1.0, 1.0)])]);
        assert!(shape_path(&layer, ColorMode::Rgb).is_none());
        layer.path.subpaths[0].closed = true;
        assert!(shape_path(&layer, ColorMode::Cmyk).is_none());
    }
}
