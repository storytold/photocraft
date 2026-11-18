//! #10878: the Kind filter lists the same layers as the reference application. Fill layers (solid
//! colour without a vector shape, gradient, pattern) match no Kind button, and a solid colour fill
//! with a vector mask lists as a shape.

use craft::layers::ui::filter::{FilterBy, FilterKind, LayerFilter};
use serde_json::json;

use super::tests::{filter, listed};

fn new_session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

/// Make a layer with `cmd`, name it `name` and return its id.
fn make(s: &mut photocraft_engine::Session, cmd: &str, params: serde_json::Value, name: &str) -> u64 {
    let id = s.execute(cmd, params).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
    id
}

fn kinds(kinds: &[FilterKind]) -> LayerFilter {
    filter(FilterBy::Kind, |f| f.kinds = kinds.to_vec())
}

/// Every kind of fill layer, a solid fill with a vector mask, and a Gradient Map adjustment.
fn fills() -> photocraft_engine::Session {
    let mut s = new_session();
    make(&mut s, "layer.newFillLayer.solidColor", json!({"color": "#112233"}), "Solid");
    let shaped = make(&mut s, "layer.newFillLayer.solidColor", json!({"color": "#445566"}), "Solid shape");
    s.execute("layer.vectorMask.revealAll", json!({"layer": shaped})).unwrap();
    make(&mut s, "layer.newFillLayer.gradient", json!({}), "Gradient");
    make(&mut s, "layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"}), "Pattern");
    make(&mut s, "layer.newAdjustmentLayer.gradientMap", json!({}), "Gradient Map");
    s
}

#[test]
fn fill_layers_match_no_kind_button_and_a_shaped_solid_fill_is_a_shape() {
    let s = fills();
    let doc = &s.active().unwrap().doc;
    // Each button on its own: the measured table, row by row.
    assert_eq!(listed(doc, &kinds(&[FilterKind::Adjustment])), ["Gradient Map"], "only the adjustment layer is an adjustment");
    assert_eq!(listed(doc, &kinds(&[FilterKind::Shape])), ["Solid shape"], "a solid fill with a vector mask lists as a shape");
    assert!(listed(doc, &kinds(&[FilterKind::Pixel])).iter().all(|n| n == "Background"), "no fill layer is a pixel layer");
    // With all five buttons on, the plain solid, gradient and pattern fills are still not listed.
    let all = listed(doc, &kinds(&FilterKind::ALL));
    for fill in ["Solid", "Gradient", "Pattern"] {
        assert!(!all.iter().any(|n| n == fill), "{fill} matches no Kind button: {all:?}");
    }
    assert!(all.iter().any(|n| n == "Solid shape") && all.iter().any(|n| n == "Gradient Map"));
}
