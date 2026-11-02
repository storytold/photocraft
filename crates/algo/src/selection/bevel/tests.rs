use super::*;
use crate::selection::expand;

fn mask(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Vec<f32> {
    (0..w * h).map(|i| if f(i % w, i / w) { 1.0 } else { 0.0 }).collect()
}

fn bevelled(m: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut g = expand(m, w, h, r as f32);
    bevel_corners(&mut g, m, w, h, r);
    g
}

/// The stroke of a rectangle `x0..=x1`, `y0..=y1` grown by `r` with bevelled corners.
fn rect_model(x: usize, y: usize, (x0, x1, y0, y1): (usize, usize, usize, usize), r: usize) -> bool {
    let out = |v: usize, lo: usize, hi: usize| lo.saturating_sub(v).max(v.saturating_sub(hi));
    match (out(x, x0, x1), out(y, y0, y1)) {
        (0, d) | (d, 0) => d <= r,
        (dx, dy) => dx + dy <= r + 1,
    }
}

#[test]
fn a_rectangle_gets_45_degree_corners_and_straight_edges_stay() {
    let (w, h) = (90, 80);
    let rect = (30, 59, 25, 54);
    let m = mask(w, h, |x, y| (rect.0..=rect.1).contains(&x) && (rect.2..=rect.3).contains(&y));
    for r in [2, 3, 5, 12, 24] {
        let g = bevelled(&m, w, h, r);
        for (i, v) in g.iter().enumerate() {
            let want = rect_model(i % w, i / w, rect, r);
            assert_eq!(*v > 0.0, want, "r {r} at ({}, {}): {v}", i % w, i / w);
        }
    }
    // Below 2 px the round corner already is the cut.
    for r in [0, 1] {
        assert_eq!(bevelled(&m, w, h, r), expand(&m, w, h, r as f32), "r {r}");
    }
}

#[test]
fn a_single_pixel_grows_into_an_octagon() {
    let (w, h) = (21, 21);
    let m = mask(w, h, |x, y| (x, y) == (10, 10));
    let g = bevelled(&m, w, h, 4);
    for (i, v) in g.iter().enumerate() {
        assert_eq!(*v > 0.0, rect_model(i % w, i / w, (10, 10, 10, 10), 4), "({}, {})", i % w, i / w);
    }
}

#[test]
fn hard_curves_keep_their_round_grow() {
    for radius in [3.0f32, 7.0, 20.0, 45.0] {
        let n = (2.0 * radius) as usize + 60;
        let c = n as f32 / 2.0;
        // A disc and a ring (the ring's outside is a curve too).
        for ring in [false, true] {
            let m = mask(n, n, |x, y| {
                let d = (x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c);
                d <= radius && (!ring || d >= radius * 0.5)
            });
            for r in [2usize, 6, 25] {
                assert_eq!(bevelled(&m, n, n, r), expand(&m, n, n, r as f32), "radius {radius} ring {ring} r {r}");
            }
        }
    }
}

#[test]
fn a_neighbour_within_reach_is_not_notched() {
    // Two squares 10 px apart, grown 20 px: the facing corners keep their round grow, so the
    // band along each square's top edge is whole; the far corners are cut.
    let (w, h) = (130, 90);
    let (a, b) = ((30, 49, 30, 59), (60, 79, 30, 59));
    let inside = |(x0, x1, y0, y1): (usize, usize, usize, usize), x: usize, y: usize| (x0..=x1).contains(&x) && (y0..=y1).contains(&y);
    let m = mask(w, h, |x, y| inside(a, x, y) || inside(b, x, y));
    let g = bevelled(&m, w, h, 20);
    let round = expand(&m, w, h, 20.0);
    for (i, (v, o)) in g.iter().zip(&round).enumerate() {
        let (x, y) = (i % w, i / w);
        let want = if (40..=69).contains(&x) { *o > 0.0 } else { rect_model(x, y, a, 20) || rect_model(x, y, b, 20) };
        assert_eq!(*v > 0.0, want, "({x}, {y})");
    }
}

#[test]
fn concave_corners_and_the_selection_are_untouched() {
    // An L: its inner corner is concave and its arms' ends are convex corners.
    let (w, h) = (100, 100);
    let m = mask(w, h, |x, y| (30..=39).contains(&x) && (30..=69).contains(&y) || (30..=69).contains(&x) && (60..=69).contains(&y));
    let g = bevelled(&m, w, h, 8);
    let round = expand(&m, w, h, 8.0);
    // Concave corner at (39, 59): the pixels just outside it are kept.
    for (x, y) in [(40, 59), (40, 52), (47, 59), (47, 52)] {
        assert!(g[y * w + x] > 0.0 && round[y * w + x] > 0.0, "({x}, {y})");
    }
    // The arms' outer corners are cut: past (39, 30) up and right, and past (69, 69).
    assert_eq!((g[24 * w + 45], round[24 * w + 45] > 0.0), (0.0, true));
    assert_eq!((g[75 * w + 75], round[75 * w + 75] > 0.0), (0.0, true));
    for (i, v) in m.iter().enumerate() {
        if *v > 0.0 {
            assert_eq!(g[i], 1.0);
        }
    }
}

#[test]
fn inconsistent_input_changes_nothing() {
    let m = mask(10, 10, |x, y| x > 3 && y > 3);
    let mut g = expand(&m, 10, 10, 4.0);
    let before = g.clone();
    bevel_corners(&mut g, &m, 9, 10, 4);
    bevel_corners(&mut g, &m, 0, 0, 4);
    bevel_corners(&mut g[..50], &m, 10, 10, 4);
    assert_eq!(g, before);
    // A reach far past the canvas cuts as much as one the canvas size.
    let mut huge = expand(&m, 10, 10, 30.0);
    bevel_corners(&mut huge, &m, 10, 10, usize::MAX);
    let mut ten = expand(&m, 10, 10, 30.0);
    bevel_corners(&mut ten, &m, 10, 10, 10);
    assert_eq!(huge, ten);
}
