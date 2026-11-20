//! Paths to Illustrator: a segment's operator follows the anchor it ends on. `c` / `l` when that
//! knot is smooth, `C` / `L` when it is a corner; the closing segment ends on the first knot.

use super::paths_export_path_blocks_tests::{body, doc_with};
use super::*;
use photocraft_doc::{Knot, Subpath};
use photocraft_geom::Point;

/// A smooth knot at `(ax, ay)` with its incoming and outgoing handles.
fn smooth(ax: f64, ay: f64, ix: f64, iy: f64, ox: f64, oy: f64) -> Knot {
    Knot::smooth(Point::new(ax, ay), Point::new(ix, iy), Point::new(ox, oy))
}

/// A smooth knot whose handles are retracted onto its anchor, so its segments stay straight.
fn smooth_straight(x: f64, y: f64) -> Knot {
    smooth(x, y, x, y, x, y)
}

/// A subpath made of exactly `knots`, closed or open.
fn subpath(knots: Vec<Knot>, closed: bool) -> Subpath {
    let mut sp = Subpath::polyline(&[(0.0, 0.0), (1.0, 1.0)]);
    sp.knots = knots;
    sp.closed = closed;
    sp
}

/// The path-construction lines of the only exported path: the `m` line through `n` / `N`.
fn drawing(ai: &str) -> Vec<&str> {
    let lines: Vec<&str> = body(ai).lines().collect();
    let from = lines.iter().position(|l| *l == "1 XR").unwrap() + 1;
    let to = lines.iter().position(|l| *l == "*U").unwrap();
    lines[from..to].to_vec()
}

#[test]
fn the_measured_smooth_circle_is_four_lower_case_curves() {
    // The issue's test document: 300 x 200 px at 72 ppi, a closed smooth circle centred on
    // 200,100 with radius 50, drawn from its top anchor clockwise; handles 27.615 long.
    let (cx, cy, r, k) = (200.0, 100.0, 50.0, 27.615);
    let circle = subpath(
        vec![
            smooth(cx, cy - r, cx - k, cy - r, cx + k, cy - r),
            smooth(cx + r, cy, cx + r, cy - k, cx + r, cy + k),
            smooth(cx, cy + r, cx + k, cy + r, cx - k, cy + r),
            smooth(cx - r, cy, cx - r, cy + k, cx - r, cy - k),
        ],
        true,
    );
    let doc = doc_with(vec![("Circle", vec![circle])]);
    let (ai, n) = paths_to_ai(&doc, "Circle").unwrap();
    assert_eq!(n, 1);
    let want = "%Adobe_Photoshop_Path_Begin:<< /defaultFill false >>\n*u\n%AI3_Note:<< /operation /union >>\n1 XR\n\
                200.0000 150.0000 m\n\
                227.6150 150.0000 250.0000 127.6150 250.0000 100.0000 c\n\
                250.0000 72.3850 227.6150 50.0000 200.0000 50.0000 c\n\
                172.3850 50.0000 150.0000 72.3850 150.0000 100.0000 c\n\
                150.0000 127.6150 172.3850 150.0000 200.0000 150.0000 c\n\
                n\n*U\n%Adobe_Photoshop_Path_End\n";
    assert_eq!(body(&ai), want);
}

#[test]
fn an_open_path_writes_each_operator_from_its_end_knot() {
    let knots = vec![
        Knot::corner(10.0, 100.0),
        smooth(60.0, 50.0, 40.0, 50.0, 80.0, 50.0),
        Knot::corner(110.0, 100.0),
        smooth_straight(160.0, 100.0),
        Knot::corner(210.0, 100.0),
    ];
    let doc = doc_with(vec![("Open", vec![subpath(knots, false)])]);
    let (ai, _) = paths_to_ai(&doc, "Open").unwrap();
    assert_eq!(
        drawing(&ai),
        [
            "10.0000 100.0000 m",
            // Curve onto the smooth knot, curve onto a corner, straight onto a smooth knot,
            // straight onto a corner.
            "10.0000 100.0000 40.0000 150.0000 60.0000 150.0000 c",
            "80.0000 150.0000 110.0000 100.0000 110.0000 100.0000 C",
            "160.0000 100.0000 l",
            "210.0000 100.0000 L",
            "N",
        ]
    );
}

#[test]
fn the_closing_segment_follows_the_first_knot() {
    // First knot smooth, the others corners: only the closing segment is lower case.
    let smooth_first = subpath(vec![smooth_straight(10.0, 10.0), Knot::corner(110.0, 10.0), Knot::corner(110.0, 110.0)], true);
    // First knot a corner, the others smooth: only the closing segment is upper case.
    let corner_first = subpath(vec![Knot::corner(10.0, 10.0), smooth_straight(110.0, 10.0), smooth_straight(110.0, 110.0)], true);
    let doc = doc_with(vec![("Smooth first", vec![smooth_first]), ("Corner first", vec![corner_first])]);
    let (ai, _) = paths_to_ai(&doc, "Smooth first").unwrap();
    assert_eq!(drawing(&ai), ["10.0000 190.0000 m", "110.0000 190.0000 L", "110.0000 90.0000 L", "10.0000 190.0000 l", "n"]);
    let (ai, _) = paths_to_ai(&doc, "Corner first").unwrap();
    assert_eq!(drawing(&ai), ["10.0000 190.0000 m", "110.0000 190.0000 l", "110.0000 90.0000 l", "10.0000 190.0000 L", "n"]);
}
