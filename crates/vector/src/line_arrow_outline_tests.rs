//! A line with arrowheads is one closed outline of corner points, the shaft joined to each head
//! (#13260), as the reference application draws it: no edge across a head's base or the shaft's
//! end, so a stroke follows only the outer outline. The anchor lists below were measured there,
//! each for a drag from 400,300 to 600,300.

use super::*;

fn anchors(sp: &Subpath) -> Vec<[f64; 2]> {
    sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
}

/// `p` is one closed subpath of corner points whose anchors are `want`, in the same order up to
/// the start point.
fn one_outline(p: &Path, want: &[[f64; 2]], what: &str) {
    assert_eq!(p.subpaths.len(), 1, "{what}: one closed outline, got {:?}", p.subpaths.iter().map(anchors).collect::<Vec<_>>());
    let sp = &p.subpaths[0];
    assert!(sp.closed, "{what}: closed");
    for k in &sp.knots {
        assert!(!k.smooth && k.in_ctrl == k.anchor && k.out_ctrl == k.anchor, "{what}: corner points only");
    }
    let got = anchors(sp);
    let n = want.len();
    let rotated = (0..n).any(|r| got.len() == n && (0..n).all(|i| close(got[(i + r) % n], want[i])));
    assert!(rotated, "{what}: {got:?} != {want:?}");
}

#[test]
fn end_arrow_weight_4_is_seven_anchors() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, false, true, 5.0, 10.0, 0.0);
    let want = [[400.0, 302.0], [400.0, 298.0], [590.0, 298.0], [590.0, 297.5], [600.0, 300.0], [590.0, 302.5], [590.0, 302.0]];
    one_outline(&p, &want, "end arrow");
}

#[test]
fn start_arrow_weight_4_is_seven_anchors() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, true, false, 5.0, 10.0, 0.0);
    let want = [[410.0, 302.0], [410.0, 302.5], [400.0, 300.0], [410.0, 297.5], [410.0, 298.0], [600.0, 298.0], [600.0, 302.0]];
    one_outline(&p, &want, "start arrow");
}

#[test]
fn end_arrow_weight_1_on_the_pixel_centre() {
    // A horizontal line is snapped to the pixel centre (y 299.5) before it reaches the outline.
    let p = arrow_line((400.0, 299.5), (600.0, 299.5), 1.0, false, true, 5.0, 10.0, 0.0);
    let want = [[400.0, 300.0], [400.0, 299.0], [590.0, 299.0], [590.0, 297.0], [600.0, 299.5], [590.0, 302.0], [590.0, 300.0]];
    one_outline(&p, &want, "weight 1");
}

#[test]
fn end_arrow_width_20_length_30() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, false, true, 20.0, 30.0, 0.0);
    let want = [[400.0, 302.0], [400.0, 298.0], [570.0, 298.0], [570.0, 290.0], [600.0, 300.0], [570.0, 310.0], [570.0, 302.0]];
    one_outline(&p, &want, "width 20 length 30");
    assert_eq!(p.control_bounds(), Some((400.0, 290.0, 600.0, 310.0)));
}

#[test]
fn both_arrows_with_concavity_50_is_ten_anchors_without_the_notch_middle() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, true, true, 5.0, 10.0, 50.0);
    let want = [
        [409.0, 302.0],
        [410.0, 302.5],
        [400.0, 300.0],
        [410.0, 297.5],
        [409.0, 298.0],
        [591.0, 298.0],
        [590.0, 297.5],
        [600.0, 300.0],
        [590.0, 302.5],
        [591.0, 302.0],
    ];
    one_outline(&p, &want, "both arrows, concavity 50");
}

#[test]
fn a_head_narrower_than_the_shaft_is_still_one_outline() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 6.0, false, true, 5.0, 10.0, 0.0);
    let want = [[400.0, 303.0], [400.0, 297.0], [590.0, 297.0], [590.0, 297.5], [600.0, 300.0], [590.0, 302.5], [590.0, 303.0]];
    one_outline(&p, &want, "narrow head");
}

#[test]
fn a_line_without_arrowheads_keeps_its_four_anchors() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, false, false, 5.0, 10.0, 0.0);
    one_outline(&p, &[[400.0, 302.0], [600.0, 302.0], [600.0, 298.0], [400.0, 298.0]], "no arrows");
}

#[test]
fn a_head_as_wide_as_the_shaft_has_no_doubled_anchor() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 5.0, false, true, 5.0, 10.0, 0.0);
    one_outline(&p, &[[400.0, 302.5], [400.0, 297.5], [590.0, 297.5], [600.0, 300.0], [590.0, 302.5]], "head as wide as the shaft");
}

#[test]
fn a_diagonal_line_is_the_same_outline_rotated() {
    // Not measured; assumed to be the horizontal outline rotated about the start point.
    let (c, s) = (0.6, 0.8);
    let rot = |p: [f64; 2]| [400.0 + (p[0] - 400.0) * c - (p[1] - 300.0) * s, 300.0 + (p[0] - 400.0) * s + (p[1] - 300.0) * c];
    let flat = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, true, true, 5.0, 10.0, 50.0);
    let end = rot([600.0, 300.0]);
    let tilted = arrow_line((400.0, 300.0), (end[0], end[1]), 4.0, true, true, 5.0, 10.0, 50.0);
    let want: Vec<[f64; 2]> = anchors(&flat.subpaths[0]).into_iter().map(rot).collect();
    one_outline(&tilted, &want, "diagonal");
}

#[test]
fn degenerate_or_non_finite_input_is_an_empty_path() {
    let n = f64::NAN;
    let i = f64::INFINITY;
    for (a, b, weight) in [
        ((2.0, 2.0), (2.0, 2.0), 4.0),
        ((400.0, 300.0), (400.5, 300.0), 4.0),
        ((n, 300.0), (600.0, 300.0), 4.0),
        ((400.0, 300.0), (i, 300.0), 4.0),
        ((400.0, 300.0), (600.0, 300.0), n),
        ((400.0, 300.0), (600.0, 300.0), i),
    ] {
        for (start, end) in [(true, false), (false, true), (true, true)] {
            let p = arrow_line(a, b, weight, start, end, 5.0, 10.0, 50.0);
            assert!(p.subpaths.is_empty(), "{a:?} {b:?} {weight} {start} {end}: {p:?}");
        }
    }
    // Non-finite head sizes fall back to 1 px and never yield a non-finite anchor.
    for (w, l, c) in [(n, 10.0, 0.0), (5.0, n, 0.0), (i, 10.0, 0.0), (5.0, i, n), (-5.0, -10.0, -1e300)] {
        let p = arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, true, true, w, l, c);
        assert!(p.subpaths.iter().flat_map(anchors).all(|a| a[0].is_finite() && a[1].is_finite()), "{w} {l} {c}");
    }
}
