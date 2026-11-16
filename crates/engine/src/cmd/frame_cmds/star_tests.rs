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
