//! Edit › Clear with a selection that follows the pixels' own coverage leaves nothing behind.
//! After Select All and a Move, the selection is the moved pixels' outline (#7538), soft where
//! their edges are; Clear removed only `alpha × selection` of each pixel and left `a·(1 − a)` of
//! every half-transparent edge pixel, a faint outline of what was cleared. Clear now takes
//! `min(alpha, selection)` of each pixel, as the Move tool's cut does (#7655), so a selection made
//! from the pixels themselves clears them whole, and opaque pixels clear as before (#12927).

use crate::Session;
use serde_json::json;

/// A white Background with a transparent layer on top, holding a soft (hardness 0) black stroke.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 80, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[20, 40], [100, 40]], "size": 24, "hardness": 0.0, "color": "#000000"})).unwrap();
    s
}

/// Every alpha value of the active layer over the canvas.
fn alphas(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
    (0..80).flat_map(|y| (0..120).map(move |x| (x, y))).map(|(x, y)| surf.rgba(x, y)[3]).collect()
}

fn soft_edge_count(s: &Session) -> usize {
    alphas(s).iter().filter(|a| **a > 0.02 && **a < 0.98).count()
}

#[test]
fn select_all_move_and_clear_leaves_nothing() {
    let mut s = session();
    assert!(soft_edge_count(&s) > 50, "the stroke has soft edges");
    s.execute("select.all", json!({})).unwrap();
    // The Move tool's drag: the selected pixels float and move.
    s.execute("select.float", json!({"dx": 7, "dy": 3})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    let left = alphas(&s).into_iter().fold(0.0f32, f32::max);
    assert_eq!(left, 0.0, "Clear left a trace of alpha {left}");
}

#[test]
fn a_selection_from_the_layers_own_transparency_clears_it_whole() {
    let mut s = session();
    let id = s.active().unwrap().active_layer.unwrap().0;
    s.execute("select.loadSelection", json!({"channel": "transparency", "layer": id})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    let left = alphas(&s).into_iter().fold(0.0f32, f32::max);
    assert_eq!(left, 0.0, "Clear left a trace of alpha {left}");
}

#[test]
fn opaque_pixels_under_a_feathered_selection_clear_as_before() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 80, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    s.execute("select.rect", json!({"x": 30, "y": 20, "width": 60, "height": 40, "feather": 8})).unwrap();
    let sel = s.active().unwrap().doc.selection.clone().unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
    let mut soft = 0;
    for y in 0..80 {
        for x in 0..120 {
            let k = sel.rgba(x, y)[0];
            let a = surf.rgba(x, y)[3];
            assert!((a - (1.0 - k)).abs() <= 1.5 / 255.0, "({x}, {y}): selection {k}, left {a}");
            if k > 0.05 && k < 0.95 {
                soft += 1;
            }
        }
    }
    assert!(soft > 100, "the selection is feathered");
}
