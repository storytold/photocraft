//! #12010: the transparency of a gradient drawn with the Gradient tool follows the interpolation
//! method (Smooth by default) instead of a straight line. Reference: Foreground to Transparent,
//! Smooth, dragged (100, 300) to (700, 300): alpha 255 / 179 / 100 / 35 / 0 at x 50 / 250 / 400 /
//! 550 / 750.

use serde_json::{Value, json};

use crate::Session;

fn drawn(extra: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    let mut p = json!({"gradient": "Foreground to Transparent", "from": [100, 300], "to": [700, 300], "dither": false});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("paint.gradient", p).unwrap();
    s
}

fn alpha_at(s: &Session, x: i32) -> f32 {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &l.content else { panic!("a pixel layer") };
    surface.pixel(x, 300)[3] * 255.0
}

#[test]
fn smooth_transparency_ramp_matches_the_reference() {
    let s = drawn(json!({"method": "smooth"}));
    for (x, want) in [(50, 255.0), (250, 179.0), (400, 100.0), (550, 35.0), (750, 0.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 2.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

#[test]
fn without_a_method_the_ramp_stays_straight() {
    let s = drawn(json!({}));
    for (x, want) in [(250, 191.0), (400, 128.0), (550, 64.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 2.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

#[test]
fn opaque_gradients_paint_the_same_with_a_method() {
    let paint = |extra: Value| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 8, "background": "white"})).unwrap();
        let mut p = json!({"colors": ["#000000", "#ffffff"], "from": [0, 4], "to": [64, 4], "dither": false});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("paint.gradient", p).unwrap();
        (0..64).map(|x| alpha_and_red(&s, x)).collect::<Vec<_>>()
    };
    fn alpha_and_red(s: &Session, x: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Raster(surface) = &l.content else { panic!() };
        surface.pixel(x, 4)
    }
    // The colours still follow the method; the alpha stays opaque.
    let smooth = paint(json!({"method": "smooth"}));
    assert!(smooth.iter().all(|p| (p[3] - 1.0).abs() < 1e-3));
    assert_ne!(smooth, paint(json!({})));
}

#[test]
fn a_transparent_ramp_with_other_methods_still_paints() {
    for m in ["classic", "perceptual", "linear"] {
        let s = drawn(json!({"method": m}));
        let (a, b) = (alpha_at(&s, 250), alpha_at(&s, 550));
        assert!(a > b && (0.0..=255.0).contains(&a), "{m}: {a} {b}");
    }
}
