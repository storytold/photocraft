//! #13876: `layer.new.frame` takes a star ratio for Polygon frames: below 100 % the frame is the
//! reference application's star filling its box; at 100 % it is today's plain polygon.
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    s
}

fn anchors(s: &mut Session, p: Value) -> Vec<[f64; 2]> {
    let r = s.execute("layer.new.frame", p).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    let f = s.active().unwrap().doc.layer(id).unwrap().frame.clone().unwrap();
    f.outline().subpaths.iter().flat_map(|sp| sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y])).collect()
}

#[test]
fn five_sides_at_50_percent_give_the_reference_star() {
    let mut s = session();
    let got = anchors(&mut s, json!({"x": 50, "y": 50, "width": 100, "height": 100, "shape": "polygon", "sides": 5, "starRatio": 50}));
    // Measured in the issue: first at top centre, stretched to fill 50..150 x 50..150.
    let want = [[100., 50.], [113., 87.], [150., 88.], [120., 112.], [131., 150.], [100., 128.], [69., 150.], [80., 112.], [50., 88.], [88., 87.]];
    assert_eq!(got.len(), 10, "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g[0] - w[0]).abs() <= 1.0 && (g[1] - w[1]).abs() <= 1.0, "{g:?} vs {w:?} in {got:?}");
    }
}

#[test]
fn at_100_percent_the_frame_is_todays_polygon() {
    let mut s = session();
    let plain = anchors(&mut s, json!({"x": 50, "y": 50, "width": 100, "height": 100, "shape": "polygon", "sides": 5}));
    let full = anchors(&mut s, json!({"x": 50, "y": 50, "width": 100, "height": 100, "shape": "polygon", "sides": 5, "starRatio": 100}));
    assert_eq!(plain.len(), 5);
    assert_eq!(full, plain);
    // Other shapes don't take it.
    let rect = anchors(&mut s, json!({"x": 50, "y": 50, "width": 100, "height": 100, "starRatio": 40}));
    assert_eq!(rect.len(), 4);
}

#[test]
fn a_bad_star_ratio_is_an_error_not_a_panic() {
    let mut s = session();
    for bad in [json!(0), json!(0.5), json!(100.5), json!(-20), json!("50"), json!([50]), json!(false), json!(1e308)] {
        let r = s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 50, "height": 50, "shape": "polygon", "starRatio": bad}));
        let err = r.expect_err(&format!("{bad}")).to_string();
        assert!(err.contains("starRatio"), "{bad}: {err}");
    }
    for ok in [json!(1), json!(33), json!(99.5)] {
        assert!(s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 50, "height": 50, "shape": "polygon", "starRatio": ok})).is_ok(), "{ok}");
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// CodeRabbit on #15172 and #15174: the accepted 1 % ratio is 1 % of the apothem, which is below the
/// shape command's 1 % floor on the radius (0.8 % on five sides, 0.5 % on three). The inner points
/// must keep that ratio, not be pushed out to the floor: the ring of inner points is measured
/// against the ring of outer points on a frame as wide as it is tall.
#[test]
fn a_1_percent_star_keeps_its_inner_radius_below_the_shape_commands_floor() {
    let mean_ring = |points: &[[f64; 2]], centre: [f64; 2]| points.iter().map(|p| dist(*p, centre)).sum::<f64>() / points.len() as f64;
    for (sides, want) in [(5u64, 0.01 * (std::f64::consts::PI / 5.0).cos()), (3, 0.005)] {
        let mut s = session();
        let got = anchors(&mut s, json!({"x": 0, "y": 0, "width": 1000, "height": 1000, "shape": "polygon", "sides": sides, "starRatio": 1}));
        assert_eq!(got.len() as u64, sides * 2);
        let inner: Vec<[f64; 2]> = got.iter().skip(1).step_by(2).copied().collect();
        let outer: Vec<[f64; 2]> = got.iter().step_by(2).copied().collect();
        let centre = [inner.iter().map(|p| p[0]).sum::<f64>() / inner.len() as f64, inner.iter().map(|p| p[1]).sum::<f64>() / inner.len() as f64];
        let ratio = mean_ring(&inner, centre) / mean_ring(&outer, centre);
        // The stretch to the box is not uniform, so the ratio is only about the radius ratio: well
        // clear of the floor's 0.01 (5 sides) and 0.01 (3 sides) it must not be.
        assert!(ratio < want * 1.2 && ratio > want * 0.8, "{sides} sides: inner/outer {ratio}, wanted about {want}");
    }
}

/// CodeRabbit on #15174: the outer points are rounded in document pixels, after the star is stretched
/// to the frame. On a 200 x 100 frame the arc at a tip is circular with the radius asked for: the
/// chord between its tangent points is 2 r cos(half the tip's angle).
#[test]
fn star_corners_are_rounded_in_document_pixels_on_a_wide_frame() {
    let mut s = session();
    let frame = |radius: f64| json!({"x": 0, "y": 0, "width": 200, "height": 100, "shape": "polygon", "sides": 5, "starRatio": 50, "radius": radius});
    let sharp = anchors(&mut s, frame(0.0));
    let round = anchors(&mut s, frame(10.0));
    assert_eq!((sharp.len(), round.len()), (10, 15), "five tips become two knots each");
    let (v, a, b) = (sharp[0], sharp[9], sharp[1]);
    let (u1, u2) = ([a[0] - v[0], a[1] - v[1]], [b[0] - v[0], b[1] - v[1]]);
    let cos = (u1[0] * u2[0] + u1[1] * u2[1]) / (u1[0].hypot(u1[1]) * u2[0].hypot(u2[1]));
    let half = cos.acos() / 2.0;
    let want = 2.0 * 10.0 * half.cos();
    let got = dist(round[0], round[1]);
    assert!((got - want).abs() < 1e-6, "chord {got} vs {want}");
}
