//! Tests that run paint commands, moved from the engine-core `fill_layer_mode` tests: 2 test(s).

use photocraft_engine::Session;

use photocraft_doc::{ColorMode, Fill, LayerContent};

use serde_json::json;

const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s.tools.foreground = BLUE;
    s.tools.background = GREEN;
    s
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

/// The composite of `color` painted with Edit › Fill on a new pixel layer: what a fill layer of
/// that colour must look like.
fn painted(mode: &str, depth: u32, color: [f32; 4]) -> [f32; 4] {
    let mut s = session(mode, depth);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
    pixel(&mut s, 10, 10)
}

fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
}

fn active_fill(s: &Session) -> Fill {
    let st = s.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Fill(f) => f.clone(),
        other => panic!("not a fill layer: {other:?}"),
    }
}

fn fill_colors(f: &Fill) -> Vec<photocraft_doc::Color> {
    match f {
        Fill::Solid(c) => vec![*c],
        Fill::Gradient { stops, .. } => stops.iter().map(|s| s.1).collect(),
        Fill::Pattern { .. } => Vec::new(),
    }
}

fn assert_in_mode(s: &Session, mode: ColorMode, what: &str) {
    let f = active_fill(s);
    for c in fill_colors(&f) {
        assert_eq!(c.mode, mode, "{what}: {f:?}");
    }
}

#[test]
fn solid_color_fill_matches_edit_fill() {
    for (mode, cm) in [("gray", ColorMode::Grayscale), ("cmyk", ColorMode::Cmyk), ("lab", ColorMode::Lab)] {
        for depth in [8, 16, 32] {
            let mut s = session(mode, depth);
            s.execute("layer.newFillLayer.solidColor", json!({"color": "#0000ff"})).unwrap();
            assert_in_mode(&s, cm, mode);
            let got = pixel(&mut s, 10, 10);
            let want = painted(mode, depth, BLUE);
            assert!(close(got, want, 0.01), "{mode}/{depth}: fill layer {got:?}, painted {want:?}");
            if cm == ColorMode::Grayscale {
                assert!((got[0] - 0.114).abs() < 0.01 && got[0] == got[1] && got[1] == got[2], "{mode}/{depth}: {got:?}");
            }
        }
    }
}

#[test]
fn gradient_edits_keep_colours_in_the_document_mode() {
    let mut s = session("cmyk", 8);
    s.execute("gradient.fill.create", json!({"from": [0, 16], "to": [64, 16]})).unwrap();
    s.execute("gradient.fill.set", json!({"stops": [[0, "#ff0000"], [1, "#0000ff"]]})).unwrap();
    assert_in_mode(&s, ColorMode::Cmyk, "gradient.fill.set");
    let start = pixel(&mut s, 0, 16);
    let red = painted("cmyk", 8, [1.0, 0.0, 0.0, 1.0]);
    assert!(close(start, red, 0.02), "{start:?} vs {red:?}");
    s.execute("gradient.presets.select", json!({"preset": "Green 01"})).unwrap();
    assert_in_mode(&s, ColorMode::Cmyk, "gradient.presets.select");
}
