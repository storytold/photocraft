//! #12099: commands without brush params paint with the default brush, not the Healing Brush's
//! reset tip (a 19 px hard round at 25 % spacing). The tip values themselves are checked in
//! `reset_tip_tests.rs`.

use serde_json::{Value, json};

/// The layer's pixels after `cmd` paints a short stroke in a fresh session, with `brush` params
/// added. The Healing Brush gets a black line to sample, as white healed into white is white
/// whatever the tip.
fn stroke(cmd: &str, brush: Value) -> Vec<f32> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let mut p = json!({"points": [[100, 100], [140, 100]]});
    if cmd == "paint.healingBrush" {
        s.execute("paint.stroke", json!({"points": [[50, 60], [110, 60]], "color": "#000000", "brush": {"size": 6}})).unwrap();
        p["offset"] = json!([-40, -40]);
    }
    p.as_object_mut().unwrap().extend(brush.as_object().unwrap().clone());
    s.execute(cmd, p).unwrap();
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn commands_without_brush_params_paint_with_the_default_brush() {
    // `paint.healingBrush` takes its tip as percent scalars, `paint.stroke` as a `brush` object.
    let cases = [
        ("paint.healingBrush", json!({"size": 20, "hardness": 100, "spacing": 10}), json!({"size": 19, "hardness": 100, "spacing": 25})),
        ("paint.stroke", json!({"brush": {"size": 20, "hardness": 1.0, "spacing": 0.1}}), json!({"brush": {"size": 19, "hardness": 1.0, "spacing": 0.25}})),
    ];
    for (cmd, default_tip, healing_tip) in cases {
        let default = stroke(cmd, default_tip);
        assert!(stroke(cmd, json!({})) == default, "{cmd}");
        // The pixels tell the Healing Brush's reset tip apart, so the comparison would catch it.
        assert!(stroke(cmd, healing_tip) != default, "{cmd}");
    }
}
