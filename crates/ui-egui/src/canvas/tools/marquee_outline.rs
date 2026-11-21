//! The outline a Rectangular or Elliptical Marquee shows while it is dragged, clipped to the canvas.
//!
//! Only pixels on the canvas can be selected, so a drag that starts on the pasteboard shows only the part of the box
//! inside the canvas, with its cut edges running along the canvas border. Ants over the pasteboard would outline an
//! area the release never selects and hide where the canvas ends. The shared marquee tool paints the whole box, so
//! the app paints this outline instead.

use craft::behavior::kurbo::{Point, Rect};

/// Points on the ellipse outline, as many as the shared marquee tool uses for its own.
const ELLIPSE_SEGMENTS: usize = 96;

/// The outline of the marquee `rect` (the ellipse inscribed in it when `ellipse`) clipped to a `width` by `height`
/// canvas, as document points; empty when no part of it is on the canvas.
pub(crate) fn clipped(rect: Rect, ellipse: bool, width: f64, height: f64) -> Vec<Point> {
    let canvas = Rect::new(0.0, 0.0, width, height);
    if width.is_nan() || height.is_nan() || width <= 0.0 || height <= 0.0 || !rect.is_finite() || rect.intersect(canvas).area() <= 0.0 {
        return Vec::new();
    }
    if !ellipse {
        let r = rect.intersect(canvas);
        return vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
    }
    let (centre, rx, ry) = (rect.center(), rect.width() / 2.0, rect.height() / 2.0);
    let mut points: Vec<Point> = (0..ELLIPSE_SEGMENTS)
        .map(|step| {
            let angle = std::f64::consts::TAU * step as f64 / ELLIPSE_SEGMENTS as f64;
            Point::new(centre.x + rx * angle.cos(), centre.y + ry * angle.sin())
        })
        .collect();
    // Sutherland-Hodgman against each canvas edge: the ellipse and the canvas are convex, so one polygon remains.
    points = clip_edge(&points, |p| p.x >= canvas.x0, |a, b| at_x(a, b, canvas.x0));
    points = clip_edge(&points, |p| p.x <= canvas.x1, |a, b| at_x(a, b, canvas.x1));
    points = clip_edge(&points, |p| p.y >= canvas.y0, |a, b| at_y(a, b, canvas.y0));
    clip_edge(&points, |p| p.y <= canvas.y1, |a, b| at_y(a, b, canvas.y1))
}

/// Keeps the part of closed polygon `points` on the `inside` side of one edge; `cross` is where a side meets it.
fn clip_edge(points: &[Point], inside: impl Fn(Point) -> bool, cross: impl Fn(Point, Point) -> Point) -> Vec<Point> {
    let Some(&last) = points.last() else { return Vec::new() };
    let mut out = Vec::with_capacity(points.len() + 2);
    let mut prev = last;
    for &p in points {
        match (inside(prev), inside(p)) {
            (true, true) => out.push(p),
            (true, false) => out.push(cross(prev, p)),
            (false, true) => {
                out.push(cross(prev, p));
                out.push(p);
            }
            (false, false) => {}
        }
        prev = p;
    }
    out
}

/// Where segment `a`–`b` crosses the vertical line `x` (the caller has checked that it does).
fn at_x(a: Point, b: Point, x: f64) -> Point {
    let dx = b.x - a.x;
    let t = if dx == 0.0 { 0.0 } else { (x - a.x) / dx };
    Point::new(x, a.y + t * (b.y - a.y))
}

/// Where segment `a`–`b` crosses the horizontal line `y` (the caller has checked that it does).
fn at_y(a: Point, b: Point, y: f64) -> Point {
    let dy = b.y - a.y;
    let t = if dy == 0.0 { 0.0 } else { (y - a.y) / dy };
    Point::new(a.x + t * (b.x - a.x), y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_canvas(points: &[Point], w: f64, h: f64) -> bool {
        points.iter().all(|p| (0.0..=w).contains(&p.x) && (0.0..=h).contains(&p.y))
    }

    #[test]
    fn a_rectangle_inside_the_canvas_is_unchanged() {
        let points = clipped(Rect::new(10.0, 20.0, 110.0, 80.0), false, 400.0, 300.0);
        assert_eq!(points, vec![Point::new(10.0, 20.0), Point::new(110.0, 20.0), Point::new(110.0, 80.0), Point::new(10.0, 80.0)]);
    }

    #[test]
    fn a_rectangle_from_the_pasteboard_runs_along_the_canvas_edges() {
        let points = clipped(Rect::new(-60.0, -40.0, 100.0, 80.0), false, 400.0, 300.0);
        assert_eq!(points, vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 80.0), Point::new(0.0, 80.0)]);
    }

    #[test]
    fn a_rectangle_covering_the_canvas_is_the_canvas() {
        let points = clipped(Rect::new(-50.0, -50.0, 500.0, 400.0), false, 400.0, 300.0);
        assert_eq!(points, vec![Point::new(0.0, 0.0), Point::new(400.0, 0.0), Point::new(400.0, 300.0), Point::new(0.0, 300.0)]);
    }

    #[test]
    fn nothing_shows_for_a_marquee_wholly_on_the_pasteboard() {
        assert!(clipped(Rect::new(-90.0, -60.0, -10.0, -5.0), false, 400.0, 300.0).is_empty());
        assert!(clipped(Rect::new(-90.0, -60.0, -10.0, -5.0), true, 400.0, 300.0).is_empty());
        // Touching the edge only has no area on the canvas.
        assert!(clipped(Rect::new(-90.0, 10.0, 0.0, 50.0), false, 400.0, 300.0).is_empty());
    }

    #[test]
    fn an_ellipse_inside_the_canvas_keeps_every_point() {
        let points = clipped(Rect::new(10.0, 10.0, 110.0, 60.0), true, 400.0, 300.0);
        assert_eq!(points.len(), ELLIPSE_SEGMENTS);
    }

    #[test]
    fn an_ellipse_from_the_pasteboard_is_cut_along_the_canvas_edges() {
        let points = clipped(Rect::new(-100.0, -50.0, 100.0, 150.0), true, 400.0, 300.0);
        assert!(on_canvas(&points, 400.0, 300.0), "{points:?}");
        // The cut runs along the left and top edges.
        assert!(points.iter().filter(|p| p.x == 0.0).count() >= 2);
        assert!(points.iter().filter(|p| p.y == 0.0).count() >= 2);
        // The rest stays on the ellipse.
        let (c, rx, ry) = (Point::new(0.0, 50.0), 100.0, 100.0);
        for p in points.iter().filter(|p| p.x > 0.0 && p.y > 0.0) {
            let d = ((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2);
            assert!((d - 1.0).abs() < 1e-9, "{p:?} is off the ellipse");
        }
    }

    #[test]
    fn bad_sizes_and_rectangles_show_nothing() {
        assert!(clipped(Rect::new(0.0, 0.0, 10.0, 10.0), false, 0.0, 300.0).is_empty());
        assert!(clipped(Rect::new(0.0, 0.0, 10.0, 10.0), true, 400.0, f64::NAN).is_empty());
        assert!(clipped(Rect::new(f64::NAN, 0.0, 10.0, 10.0), false, 400.0, 300.0).is_empty());
        assert!(clipped(Rect::new(0.0, 0.0, f64::INFINITY, 10.0), true, 400.0, 300.0).is_empty());
    }
}
