//! #13875: `layer.new.frame` takes a corner radius in document pixels for Rectangle, Triangle and
//! Polygon frames; a rounded rectangle has 8 anchors, a sharp one 4, and a non-square frame keeps
//! circular corners.
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    s
}

fn frame(s: &mut Session, p: Value) -> photocraft_doc::Frame {
    let r = s.execute("layer.new.frame", p).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    *s.active().unwrap().doc.layer(id).unwrap().frame.clone().unwrap()
}

fn anchors(f: &photocraft_doc::Frame) -> Vec<[f64; 2]> {
    f.outline().subpaths.iter().flat_map(|sp| sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y])).collect()
}

#[test]
fn a_rounded_rectangle_has_8_anchors_and_a_sharp_one_4() {
    let mut s = session();
    let sharp = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 100}));
    assert_eq!(anchors(&sharp).len(), 4, "radius 0: today's rectangle");
    let zero = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 100, "radius": 0}));
    assert_eq!(zero.path, sharp.path, "radius 0 is the default");
    let round = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 100, "radius": 20}));
    assert_eq!(anchors(&round).len(), 8);
    assert_eq!(round.bounds(), sharp.bounds(), "same box");
}

#[test]
fn corners_stay_circular_on_a_frame_that_is_not_square() {
    let mut s = session();
    let f = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 100, "radius": 20}));
    let a = anchors(&f);
    let near = |v: f64, w: f64| (v - w).abs() < 1e-6;
    // On the top edge the arcs end 20 px in from each side; on the left edge 20 px down and up.
    let top: Vec<f64> = a.iter().filter(|p| near(p[1], 40.0)).map(|p| p[0]).collect();
    let left: Vec<f64> = a.iter().filter(|p| near(p[0], 50.0)).map(|p| p[1]).collect();
    assert!(top.iter().any(|x| near(*x, 70.0)) && top.iter().any(|x| near(*x, 230.0)), "{top:?}");
    assert!(left.iter().any(|y| near(*y, 60.0)) && left.iter().any(|y| near(*y, 120.0)), "{left:?}");
}

#[test]
fn triangle_and_polygon_frames_round_their_corners() {
    let mut s = session();
    for (shape, sides) in [("triangle", 3), ("polygon", 6)] {
        let sharp = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 160, "shape": shape, "sides": sides}));
        let round = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 160, "shape": shape, "sides": sides, "radius": 15.5}));
        assert_eq!(anchors(&sharp).len(), sides, "{shape}: sharp");
        assert!(anchors(&round).len() > sides, "{shape}: rounded, {:?}", anchors(&round));
        assert_ne!(round.path, sharp.path);
    }
    // An ellipse has no corners: the radius changes nothing.
    let e0 = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 160, "shape": "ellipse"}));
    let e1 = frame(&mut s, json!({"x": 50, "y": 40, "width": 200, "height": 160, "shape": "ellipse", "radius": 30}));
    assert_eq!(e0.path, e1.path);
}

#[test]
fn a_bad_radius_is_an_error_not_a_panic() {
    let mut s = session();
    for bad in [json!(-1), json!(150_000.5), json!("12"), json!([3]), json!(true), json!(1e308)] {
        let r = s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 50, "height": 50, "radius": bad}));
        let err = r.expect_err(&format!("{bad}")).to_string();
        assert!(err.contains("radius"), "{bad}: {err}");
    }
    // The ends are allowed, as is a fraction.
    for ok in [json!(0), json!(150_000), json!(12.5)] {
        assert!(s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 50, "height": 50, "radius": ok})).is_ok());
    }
}
