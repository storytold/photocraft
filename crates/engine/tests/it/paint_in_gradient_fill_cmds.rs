//! Tests that run paint commands, moved from the engine-core `gradient_fill_cmds` tests: 2 test(s).

use photocraft_engine::gradient_fill_cmds::*;
use serde_json::{Value, json};

use photocraft_engine::Session;

fn session(depth: u32, background: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "background": background})).unwrap();
    s
}

fn layer_count(s: &Session) -> usize {
    s.active().unwrap().doc.layers.len()
}

const STYLES: [&str; 5] = ["linear", "radial", "angle", "reflected", "diamond"];

/// Pixel parity: a live gradient composites exactly like painting the same drag.
fn parity(depth: u32, background: &str, extra: Value, select: bool, tol: f32) {
    for style in STYLES {
        for (from, to) in [([10.0, 9.0], [34.0, 22.0]), ([30.0, 16.0], [22.0, 3.0])] {
            let mut p = json!({"from": from, "to": to, "style": style});
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            let run = |live: bool| {
                let mut s = session(depth, background);
                if background != "transparent" {
                    s.execute("layer.new.layer", json!({})).unwrap();
                }
                if select {
                    s.execute("select.rect", json!({"x": 6, "y": 3, "width": 30, "height": 22, "ellipse": true, "antiAlias": true})).unwrap();
                }
                // Linear and Reflected fill layers sample each pixel's top-left corner
                // (compose::effects::gradient_t, fitted on Photoshop's fills), the others its
                // centre; the painted gradient samples a pixel up and left of the centre (#12018),
                // so its drag is moved by the difference.
                let mut p = p.clone();
                if !live {
                    let d = if matches!(style, "linear" | "reflected") { -0.5 } else { -1.0 };
                    p["from"] = json!([from[0] + d, from[1] + d]);
                    p["to"] = json!([to[0] + d, to[1] + d]);
                }
                s.execute(if live { CREATE } else { "paint.gradient" }, p).unwrap();
                photocraft_compose::flatten(&s.active().unwrap().doc)
            };
            let (classic, live) = (run(false), run(true));
            let mut worst = 0.0f32;
            for (a, b) in classic.px.iter().zip(&live.px) {
                for c in 0..4 {
                    worst = worst.max((a[c] * a[3] - b[c] * b[3]).abs());
                }
            }
            assert!(worst <= tol, "{style} {from:?}->{to:?} {extra} select={select} depth={depth}: worst {worst}");
        }
    }
}

#[test]
fn live_gradient_renders_like_the_classic_gradient() {
    let stops = json!({"stops": [[0, "#ff2000"], [0.4, "#20c040"], [1, "#1010e0"]], "dither": false});
    parity(32, "transparent", stops.clone(), false, 1e-4);
    parity(8, "transparent", stops.clone(), false, 0.51 / 255.0);
    parity(8, "white", json!({"stops": [[0, "#ff2000"], [1, "#1010e0"]], "reverse": true, "dither": true}), true, 1.01 / 255.0);
    parity(16, "transparent", json!({"gradient": "Foreground to Transparent", "dither": true}), false, 1.01 / 255.0);
    parity(32, "transparent", json!({"gradient": "Black, White", "transparency": [[0, 65], [1, 0]], "dither": false}), false, 1e-4);
}

#[test]
fn classic_gradient_still_paints_pixels() {
    let mut s = session(8, "white");
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [48, 0], "colors": ["#000000", "#ffffff"], "dither": false})).unwrap();
    assert_eq!(layer_count(&s), 1, "no fill layer");
    let d = s.active().unwrap();
    let px = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 5);
    assert!(px[0] < 0.1);
}
