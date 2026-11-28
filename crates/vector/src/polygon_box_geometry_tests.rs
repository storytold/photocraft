//! #13298: the Polygon tool's polygon is a regular polygon stretched so its tight box is the drag box. Odd side counts
//! put a vertex at the top centre, even ones a flat edge at the top; the anchors run clockwise from the first vertex at
//! or right of the top centre. Stars keep their outer points there. Polygons without the flag (older files, the Star
//! tool) keep the inscribed geometry. Box 400,300 to 600,400 throughout, as measured.

use photocraft_doc::vector::LiveShape;

use super::{live_path, polygon, polygon_in_box};

const BOX: [f64; 4] = [400.0, 300.0, 200.0, 100.0];

fn anchors(p: &photocraft_doc::vector::Path) -> Vec<[f64; 2]> {
    p.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn filled(sides: u32, star_ratio: f64) -> Vec<[f64; 2]> {
    anchors(&polygon_in_box(BOX[0], BOX[1], BOX[2], BOX[3], sides, star_ratio))
}

/// Within 0.05 px of the measured values (which carry one decimal).
fn assert_near(got: &[[f64; 2]], want: &[[f64; 2]], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g[0] - w[0]).abs() < 0.05 && (g[1] - w[1]).abs() < 0.05, "{what}: {got:?} vs {want:?}");
    }
}

/// The tight box of the anchors: [x0, y0, x1, y1].
fn tight(points: &[[f64; 2]]) -> [f64; 4] {
    let f = |i: usize, min: bool| points.iter().map(|p| p[i]).fold(if min { f64::MAX } else { f64::MIN }, |a, b| if min { a.min(b) } else { a.max(b) });
    [f(0, true), f(1, true), f(0, false), f(1, false)]
}

#[test]
fn the_measured_anchors_for_three_to_six_sides() {
    assert_near(&filled(3, 1.0), &[[500.0, 300.0], [600.0, 400.0], [400.0, 400.0]], "3 sides");
    assert_near(&filled(4, 1.0), &[[600.0, 300.0], [600.0, 400.0], [400.0, 400.0], [400.0, 300.0]], "4 sides");
    assert_near(&filled(5, 1.0), &[[500.0, 300.0], [600.0, 338.2], [561.8, 400.0], [438.2, 400.0], [400.0, 338.2]], "5 sides");
    assert_near(&filled(6, 1.0), &[[550.0, 300.0], [600.0, 350.0], [550.0, 400.0], [450.0, 400.0], [400.0, 350.0], [450.0, 300.0]], "6 sides");
}

#[test]
fn every_side_count_fills_the_box_with_a_vertex_or_a_flat_edge_on_top() {
    for sides in 3..=12 {
        let p = filled(sides, 1.0);
        let t = tight(&p);
        assert!(t.iter().zip([400.0, 300.0, 600.0, 400.0]).all(|(g, w)| (g - w).abs() < 1e-9), "{sides} sides: tight box {t:?}");
        let on_top = p.iter().filter(|q| (q[1] - 300.0).abs() < 1e-9).count();
        if sides % 2 == 1 {
            assert_eq!(on_top, 1, "{sides} sides: one vertex on top");
            assert!((p[0][0] - 500.0).abs() < 1e-9, "{sides} sides: it is at the top centre and comes first");
        } else {
            assert_eq!(on_top, 2, "{sides} sides: a flat top edge");
            assert!(p[0][0] > 500.0 && (p[0][1] - 300.0).abs() < 1e-9, "{sides} sides: the first anchor is right of the top centre");
        }
    }
    // 8 sides: the first anchor at about 541.6,300.
    assert!((filled(8, 1.0)[0][0] - 541.6).abs() < 0.5);
}

#[test]
fn dragging_up_left_gives_the_same_polygon() {
    for sides in [3, 5, 6, 8] {
        let back = anchors(&polygon_in_box(600.0, 400.0, -200.0, -100.0, sides, 1.0));
        assert_near(&back, &filled(sides, 1.0), "up-left");
    }
}

#[test]
fn a_star_keeps_its_outer_points_on_the_polygon_vertices() {
    for sides in [5, 6] {
        let star = filled(sides, 0.5);
        let outer: Vec<[f64; 2]> = star.iter().step_by(2).copied().collect();
        assert_near(&outer, &filled(sides, 1.0), "outer points");
        // The inner points lie inside the polygon's box, nearer its centre.
        assert!(star.iter().skip(1).step_by(2).all(|q| (q[0] - 500.0).abs() < 100.0 && (q[1] - 350.0).abs() < 50.0));
    }
}

#[test]
fn without_the_flag_a_polygon_keeps_the_inscribed_geometry() {
    let old = LiveShape::Polygon { rect: BOX, sides: 5, star_ratio: 1.0, corner_radius: 0.0, fills_box: false };
    assert_eq!(anchors(&live_path(&old)), anchors(&polygon(BOX[0], BOX[1], BOX[2], BOX[3], 5, 1.0)));
    let new = LiveShape::Polygon { rect: BOX, sides: 5, star_ratio: 1.0, corner_radius: 0.0, fills_box: true };
    assert_near(&anchors(&live_path(&new)), &filled(5, 1.0), "flagged");
}
