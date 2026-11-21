//! Edit › Fill's painting modes Behind and Clear (#14646), on the issue's measured document.

use super::*;
use serde_json::json;

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// Each channel within one 8-bit step of `want` (0–255).
fn near(got: [f32; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g * 255.0 - f32::from(w)).abs() <= 1.0)
}

/// 400 × 300 transparent: opaque red at 0,0–200,150, blue (0,128,255) at 50 % at 200,0–400,150, the
/// lower half empty; foreground (200,30,30); 100,50–300,250 selected.
fn document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
    s.execute(CMD, json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 200, "y": 0, "width": 200, "height": 150})).unwrap();
    s.execute(CMD, json!({"contents": "color", "color": "#0080ff", "opacity": 50})).unwrap();
    s.execute("tools.setColors", json!({"foreground": [200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    s
}

/// Red, half-transparent blue and empty pixels inside the selection, then the same three outside it.
const INSIDE: [(i32, i32); 3] = [(150, 100), (250, 100), (200, 200)];
const OUTSIDE: [(i32, i32); 3] = [(50, 25), (350, 25), (50, 200)];

#[test]
fn behind_paints_only_under_what_is_there_and_clear_empties_the_selection() {
    for (mode, want) in [("behind", [[255, 0, 0, 255], [100, 79, 143, 255], [200, 30, 30, 255]]), ("Clear", [[0; 4]; 3])] {
        let mut s = document();
        let outside: Vec<_> = OUTSIDE.iter().map(|&(x, y)| px(&s, x, y)).collect();
        s.execute(CMD, json!({"contents": "foreground", "mode": mode})).unwrap();
        for (&(x, y), w) in INSIDE.iter().zip(want) {
            assert!(near(px(&s, x, y), w), "{mode} at {x},{y}: {:?}, want {w:?}", px(&s, x, y));
        }
        assert_eq!(OUTSIDE.iter().map(|&(x, y)| px(&s, x, y)).collect::<Vec<_>>(), outside, "{mode}: outside the selection changed");
        // One history step: undo brings the half-transparent blue back.
        assert!(s.undo());
        assert!(near(px(&s, 250, 100), [0, 128, 255, 128]), "{mode} undo: {:?}", px(&s, 250, 100));
    }
}

#[test]
fn opacity_scales_behind_and_clear_between_untouched_and_full() {
    let mut s = document();
    s.execute(CMD, json!({"contents": "foreground", "mode": "behind", "opacity": 50})).unwrap();
    let empty = px(&s, 200, 200)[3];
    assert!(empty > 0.0 && empty < 1.0, "Behind at 50 %: alpha {empty}");
    s.execute(CMD, json!({"contents": "foreground", "mode": "clear", "opacity": 50})).unwrap();
    let red = px(&s, 150, 100);
    assert!(red[3] > 0.0 && red[3] < 1.0, "Clear at 50 %: {red:?}");
    assert!(near([red[0], red[1], red[2], 1.0], [255, 0, 0, 255]), "Clear keeps the colour of what it leaves: {red:?}");
}

#[test]
fn behind_and_clear_are_refused_without_transparency() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    for mode in ["behind", "clear"] {
        assert!(s.execute(CMD, json!({"contents": "black", "mode": mode})).is_err(), "{mode} on the Background layer");
        assert!(near(px(&s, 8, 8), [255; 4]), "{mode}: the Background layer changed");
    }
}
