//! Parametric ("live") shapes → paths: rectangles with corner radii, ellipses, polygons, stars
//! and lines. Knots run clockwise on screen (y down), starting at the top.

use photocraft_doc::{Knot, LiveShape, Path, Subpath};
use photocraft_geom::Point;

/// Handle length of a quarter-circle cubic, `4/3·(√2 − 1)`.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

fn smooth(anchor: (f64, f64), inc: (f64, f64), out: (f64, f64)) -> Knot {
    Knot::smooth(Point::new(anchor.0, anchor.1), Point::new(inc.0, inc.1), Point::new(out.0, out.1))
}

/// Axis-aligned rectangle.
pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])])
}

/// Clamps corner radii `[tl, tr, br, bl]` so adjacent radii never exceed a side (scaled down
/// uniformly, like CSS `border-radius`).
pub fn clamp_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let r = radii.map(|v| v.max(0.0));
    let mut f: f64 = 1.0;
    for (a, b, side) in [(r[0], r[1], w), (r[1], r[2], h), (r[2], r[3], w), (r[3], r[0], h)] {
        if a + b > side && a + b > 0.0 {
            f = f.min(side / (a + b));
        }
    }
    r.map(|v| v * f)
}

/// Rectangle with corner radii `[top-left, top-right, bottom-right, bottom-left]`. A negative
/// width or height extends from the origin to the left or up, like [`ellipse`] and [`polygon`].
pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) -> Path {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    let [tl, tr, br, bl] = clamp_radii(w, h, radii);
    if tl <= 0.0 && tr <= 0.0 && br <= 0.0 && bl <= 0.0 {
        return rect(x, y, w, h);
    }
    let (x1, y1) = (x + w, y + h);
    let mut k = Vec::with_capacity(8);
    let corner = |p: (f64, f64)| Knot::corner(p.0, p.1);
    let push_arc = |k: &mut Vec<Knot>, start: (f64, f64), end: (f64, f64), c_start: (f64, f64), c_end: (f64, f64), r: f64| {
        if r <= 0.0 {
            k.push(corner(start));
            return;
        }
        k.push(Knot { anchor: Point::new(start.0, start.1), in_ctrl: Point::new(start.0, start.1), out_ctrl: Point::new(c_start.0, c_start.1), smooth: false });
        k.push(Knot { anchor: Point::new(end.0, end.1), in_ctrl: Point::new(c_end.0, c_end.1), out_ctrl: Point::new(end.0, end.1), smooth: false });
    };
    // Each corner arc from the end of one edge to the start of the next (clockwise).
    push_arc(&mut k, (x1 - tr, y), (x1, y + tr), (x1 - tr + KAPPA * tr, y), (x1, y + tr - KAPPA * tr), tr);
    push_arc(&mut k, (x1, y1 - br), (x1 - br, y1), (x1, y1 - br + KAPPA * br), (x1 - br + KAPPA * br, y1), br);
    push_arc(&mut k, (x + bl, y1), (x, y1 - bl), (x + bl - KAPPA * bl, y1), (x, y1 - bl + KAPPA * bl), bl);
    push_arc(&mut k, (x, y + tl), (x + tl, y), (x, y + tl - KAPPA * tl), (x + tl - KAPPA * tl, y), tl);
    // Start at the top-left arc end so the first knot is on the top edge.
    let n = k.len();
    k.rotate_left(n - 1);
    // Drop duplicated consecutive anchors (zero-length straight edges).
    let mut out: Vec<Knot> = Vec::with_capacity(k.len());
    for kn in k {
        if let Some(last) = out.last_mut()
            && (last.anchor.x - kn.anchor.x).abs() < 1e-12
            && (last.anchor.y - kn.anchor.y).abs() < 1e-12
        {
            last.out_ctrl = kn.out_ctrl;
            continue;
        }
        out.push(kn);
    }
    if out.len() > 1 {
        let (f, l) = (out[0], out[out.len() - 1]);
        if (f.anchor.x - l.anchor.x).abs() < 1e-12 && (f.anchor.y - l.anchor.y).abs() < 1e-12 {
            out[0].in_ctrl = l.in_ctrl;
            out.pop();
        }
    }
    Path::new(vec![Subpath { closed: true, knots: out, op: Default::default() }])
}

/// Ellipse inscribed in the rectangle.
pub fn ellipse(x: f64, y: f64, w: f64, h: f64) -> Path {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let knots = vec![
        smooth((cx, cy - ry), (cx - kx, cy - ry), (cx + kx, cy - ry)),
        smooth((cx + rx, cy), (cx + rx, cy - ky), (cx + rx, cy + ky)),
        smooth((cx, cy + ry), (cx + kx, cy + ry), (cx - kx, cy + ry)),
        smooth((cx - rx, cy), (cx - rx, cy + ky), (cx - rx, cy - ky)),
    ];
    Path::new(vec![Subpath { closed: true, knots, op: Default::default() }])
}

/// Regular polygon (or star when `star_ratio < 1`) inscribed in the rectangle's ellipse, first
/// vertex at the top.
pub fn polygon(x: f64, y: f64, w: f64, h: f64, sides: u32, star_ratio: f64) -> Path {
    let sides = sides.clamp(3, 100) as usize;
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let star = star_ratio > 0.0 && star_ratio < 1.0;
    let count = if star { sides * 2 } else { sides };
    let pts: Vec<(f64, f64)> = (0..count)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / count as f64;
            let k = if star && i % 2 == 1 { star_ratio } else { 1.0 };
            (cx + rx * k * a.cos(), cy + ry * k * a.sin())
        })
        .collect();
    Path::new(vec![Subpath::polygon(&pts)])
}

/// Round polygon vertices with circular cubic arcs, limiting trims to half each edge.
pub fn rounded_polygon(x: f64, y: f64, w: f64, h: f64, sides: u32, ratio: f64, radius: f64) -> Path {
    polygon_with_indents([x, y, w, h], sides, ratio, radius, false)
}

fn polygon_with_indents([x, y, w, h]: [f64; 4], sides: u32, ratio: f64, radius: f64, smooth_indents: bool) -> Path {
    let mut path = polygon(x, y, w, h, sides, ratio);
    let smooth_indents = smooth_indents && ratio > 0.0 && ratio < 1.0;
    if (!radius.is_finite() || radius <= 0.0) && !smooth_indents {
        return path;
    }
    for sp in &mut path.subpaths {
        let points: Vec<_> = sp.knots.iter().map(|k| k.anchor).collect();
        let mut knots = Vec::with_capacity(points.len() * 2);
        for (i, p) in points.iter().enumerate() {
            let Some(prev) = points.get(if i == 0 { points.len().saturating_sub(1) } else { i - 1 }) else { continue };
            let Some(next) = points.get((i + 1) % points.len()) else { continue };
            let (ax, ay, bx, by) = (prev.x - p.x, prev.y - p.y, next.x - p.x, next.y - p.y);
            let (la, lb) = (ax.hypot(ay), bx.hypot(by));
            if la <= 1e-12 || lb <= 1e-12 {
                knots.push(Knot::corner(p.x, p.y));
                continue;
            }
            let (ax, ay, bx, by) = (ax / la, ay / la, bx / lb, by / lb);
            let angle = (ax * bx + ay * by).clamp(-1.0, 1.0).acos();
            let tangent = (angle * 0.5).tan();
            if tangent <= 1e-12 || !tangent.is_finite() {
                knots.push(Knot::corner(p.x, p.y));
                continue;
            }
            let trim = if smooth_indents && i % 2 == 1 { la.min(lb) * 0.5 } else { (radius.max(0.0) / tangent).min(la * 0.5).min(lb * 0.5) };
            if trim <= 0.0 {
                knots.push(Knot::corner(p.x, p.y));
                continue;
            }
            let handle = 4.0 / 3.0 * ((std::f64::consts::PI - angle) * 0.25).tan() * trim * tangent;
            let a = Point::new(p.x + ax * trim, p.y + ay * trim);
            let b = Point::new(p.x + bx * trim, p.y + by * trim);
            knots.push(Knot { anchor: a, in_ctrl: a, out_ctrl: Point::new(a.x - ax * handle, a.y - ay * handle), smooth: false });
            knots.push(Knot { anchor: b, in_ctrl: Point::new(b.x - bx * handle, b.y - by * handle), out_ctrl: b, smooth: false });
        }
        sp.knots = knots;
    }
    path
}

/// A line drawn as a filled bar of `weight` px from `a` to `b`.
pub fn line(a: (f64, f64), b: (f64, f64), weight: f64) -> Path {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx.hypot(dy);
    let hw = weight.max(0.0) / 2.0;
    let (nx, ny) = if l > 0.0 { (-dy / l * hw, dx / l * hw) } else { (0.0, hw) };
    Path::new(vec![Subpath::polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)])])
}

/// The path of a live shape.
pub fn live_path(s: &LiveShape) -> Path {
    match s {
        LiveShape::Rect { rect: [x, y, w, h], radii } => rounded_rect(*x, *y, *w, *h, *radii),
        LiveShape::Ellipse { rect: [x, y, w, h] } => ellipse(*x, *y, *w, *h),
        LiveShape::Polygon { rect, sides, star_ratio, corner_radius, smooth_indents } => {
            polygon_with_indents(*rect, *sides, *star_ratio, *corner_radius, *smooth_indents)
        }
        LiveShape::Line { from, to, weight, arrowheads } => {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            let distance = dx.hypot(dy);
            let Some(heads) = arrowheads.as_ref().filter(|a| (a.start || a.end) && a.width > 0.0 && a.length > 0.0 && distance > 1e-12) else {
                return line((from[0], from[1]), (to[0], to[1]), *weight);
            };
            let (ux, uy) = (dx / distance, dy / distance);
            let concavity = if heads.concavity.is_finite() { heads.concavity.clamp(-50.0, 50.0) / 100.0 } else { 0.0 };
            let (length, half_width, half_weight) =
                (heads.length.min(distance * 0.5 / (1.0 + (-concavity).max(0.0))), heads.width * 0.5, weight.max(0.0) * 0.5);
            let notch = length * concavity * (1.0 - half_weight / half_width).clamp(0.0, 1.0);
            let start = if heads.start { length } else { 0.0 };
            let end = if heads.end { distance - length } else { distance };
            let shaft_start = if heads.start { start - notch } else { start };
            let shaft_end = if heads.end { end + notch } else { end };
            let mut points = Vec::with_capacity(10);
            if heads.start {
                points.extend([(0.0, 0.0), (start, half_width)]);
            }
            points.extend([(shaft_start, half_weight), (shaft_end, half_weight)]);
            if heads.end {
                points.extend([(end, half_width), (distance, 0.0), (end, -half_width)]);
            }
            points.extend([(shaft_end, -half_weight), (shaft_start, -half_weight)]);
            if heads.start {
                points.push((start, -half_width));
            }
            let points: Vec<_> = points.into_iter().map(|(x, y)| (from[0] + ux * x - uy * y, from[1] + uy * x + ux * y)).collect();
            Path::new(vec![Subpath::polygon(&points)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_knots() {
        let p = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0; 4]);
        assert_eq!(p.subpaths[0].knots.len(), 8);
        let sharp = rounded_rect(0.0, 0.0, 100.0, 50.0, [0.0; 4]);
        assert_eq!(sharp.subpaths[0].knots.len(), 4);
        let mixed = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0, 0.0, 10.0, 0.0]);
        assert_eq!(mixed.subpaths[0].knots.len(), 6);
        assert_eq!(clamp_radii(10.0, 10.0, [10.0; 4]), [5.0; 4]);
    }

    #[test]
    fn negative_size_rect_extends_from_its_origin_like_the_ellipse() {
        // Issue #4203: a negative width/height drew the rectangle on the far side of its origin.
        let want = ellipse(50.0, 10.0, -20.0, 30.0).control_bounds().unwrap();
        let want_neg_h = ellipse(50.0, 40.0, -20.0, -30.0).control_bounds().unwrap();
        for radii in [[0.0; 4], [5.0; 4]] {
            let b = rounded_rect(50.0, 10.0, -20.0, 30.0, radii).control_bounds().unwrap();
            assert_eq!(b, want, "radii {radii:?}");
            assert_eq!(b, (30.0, 10.0, 50.0, 40.0), "radii {radii:?}");
            let b = rounded_rect(50.0, 40.0, -20.0, -30.0, radii).control_bounds().unwrap();
            assert_eq!(b, want_neg_h, "radii {radii:?}");
        }
    }

    #[test]
    fn star_vertices() {
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 5, 0.5).subpaths[0].knots.len(), 10);
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 6, 1.0).subpaths[0].knots.len(), 6);
        let top = polygon(0.0, 0.0, 10.0, 10.0, 3, 1.0).subpaths[0].knots[0].anchor;
        assert!((top.x - 5.0).abs() < 1e-9 && top.y.abs() < 1e-9);
    }

    #[test]
    fn rounded_polygons_have_finite_tangent_arcs_even_when_radius_exceeds_edges() {
        for sides in [3, 5, 100] {
            for ratio in [0.3, 1.0] {
                let sharp = polygon(0.0, 0.0, 100.0, 70.0, sides, ratio);
                assert_eq!(rounded_polygon(0.0, 0.0, 100.0, 70.0, sides, ratio, 0.0), sharp);
                for radius in [1.0, 10000.0] {
                    let rounded = rounded_polygon(0.0, 0.0, 100.0, 70.0, sides, ratio, radius);
                    assert_eq!(rounded.subpaths[0].knots.len(), sharp.subpaths[0].knots.len() * 2);
                    assert!(rounded.subpaths[0].knots.iter().all(|k| [k.anchor, k.in_ctrl, k.out_ctrl].iter().all(|p| p.x.is_finite() && p.y.is_finite())));
                    let area = crate::fill_rasterizer(&rounded, crate::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::new(0, 0, 100, 70)).iter().sum::<f32>();
                    assert!(area > 0.0 && area < 7000.0);
                }
            }
        }
    }

    #[test]
    fn smooth_star_indents_curve_inner_vertices_and_keep_outer_points() {
        let area = photocraft_geom::Rect::new(0, 0, 100, 70);
        for sides in [3, 5, 100] {
            let sharp = polygon(0.0, 0.0, 100.0, 70.0, sides, 0.4);
            let smooth = polygon_with_indents([0.0, 0.0, 100.0, 70.0], sides, 0.4, 0.0, true);
            assert_eq!(smooth.subpaths[0].knots.len(), sides as usize * 3);
            for tip in sharp.subpaths[0].knots.iter().step_by(2) {
                assert!(smooth.subpaths[0].knots.contains(tip), "smoothing an indent changed a star point");
            }
            assert!(smooth.subpaths[0].knots.iter().any(|k| k.out_ctrl != k.anchor || k.in_ctrl != k.anchor));
            let coverage = crate::fill_rasterizer(&smooth, crate::DEFAULT_TOLERANCE).render(area);
            assert!(coverage.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
            assert!(coverage.iter().sum::<f32>() > 0.0);
        }
        assert_eq!(polygon_with_indents([0.0, 0.0, 100.0, 70.0], 5, 1.0, 0.0, true), polygon(0.0, 0.0, 100.0, 70.0, 5, 1.0));
    }

    #[test]
    fn arrowhead_concavity_changes_joined_outline_without_a_shaft_seam() {
        for (start, end) in [(true, false), (false, true), (true, true)] {
            let path = |concavity| {
                live_path(&LiveShape::Line {
                    from: [10.0, 30.0],
                    to: [110.0, 30.0],
                    weight: 4.0,
                    arrowheads: Some(photocraft_doc::vector::LineArrowheads { start, end, width: 30.0, length: 20.0, concavity }),
                })
            };
            let area = |p: &Path| crate::fill_rasterizer(p, crate::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::new(0, 0, 120, 60)).iter().sum::<f32>();
            let flat = path(0.0);
            assert!(area(&path(50.0)) < area(&flat));
            assert!(area(&path(-50.0)) > area(&flat));
            for concavity in [-50.0, 0.0, 50.0] {
                let p = path(concavity);
                assert_eq!(p.subpaths.len(), 1);
                assert!(p.subpaths[0].closed);
                let shaft = crate::fill_rasterizer(&p, crate::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::new(30, 29, 90, 31));
                assert!(shaft.iter().all(|v| *v > 0.99));
            }
        }
    }

    #[test]
    fn short_degenerate_and_rotated_arrows_stay_finite() {
        for to in [[10.0, 10.0], [10.1, 10.1], [30.0, 50.0], [5.0, 8.0]] {
            for concavity in [-50.0, 50.0] {
                let path = live_path(&LiveShape::Line {
                    from: [10.0, 10.0],
                    to,
                    weight: 2.0,
                    arrowheads: Some(photocraft_doc::vector::LineArrowheads { start: true, end: true, width: 10.0, length: 100.0, concavity }),
                });
                assert!(
                    path.subpaths.iter().flat_map(|s| &s.knots).all(|k| [k.anchor, k.in_ctrl, k.out_ctrl].iter().all(|p| p.x.is_finite() && p.y.is_finite()))
                );
            }
        }
    }
}
