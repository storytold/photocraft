//! Tests that run paint commands, moved from the engine-core `hidden_target` tests: 4 test(s).

use serde_json::json;

use photocraft_engine::hidden_target::HIDDEN;

use photocraft_engine::Session;

fn session() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    (s, id)
}

fn pixel(s: &mut Session) -> serde_json::Value {
    s.execute("document.pixel", json!({"x": 10, "y": 10})).unwrap()
}

#[test]
fn painting_or_moving_a_hidden_layer_is_refused() {
    let (mut s, id) = session();
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    for (cmd, p) in [
        ("paint.stroke", json!({"points": [[10, 10], [20, 20]], "color": "#ff0000"})),
        ("paint.bucket", json!({"x": 10, "y": 10})),
        ("paint.gradient", json!({"from": [0, 0], "to": [60, 60]})),
        ("layer.translate", json!({"dx": 5, "dy": 0})),
    ] {
        let e = s.execute(cmd, p).unwrap_err().to_string();
        assert!(e.contains(HIDDEN), "{cmd}: {e}");
    }
    assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
    // Shown again, it paints.
    s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
    s.execute("paint.stroke", json!({"points": [[10, 10], [12, 12]], "color": "#ff0000", "size": 9})).unwrap();
    assert_ne!(pixel(&mut s), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn a_layer_in_a_hidden_group_is_hidden() {
    let (mut s, id) = session();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let g = s.active().unwrap().active_layer.unwrap().0;
    assert_ne!(g, id, "the new group is active");
    s.execute("layer.setProps", json!({"layer": g, "visible": false})).unwrap();
    s.execute("layer.select", json!({"layer": id})).unwrap();
    assert!(s.execute("paint.stroke", json!({"points": [[10, 10]]})).unwrap_err().to_string().contains(HIDDEN));
}

#[test]
fn alpha_channels_of_a_hidden_layer_still_paint() {
    let (mut s, id) = session();
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[10, 10], [20, 20]], "target": {"channel": 0}})).unwrap();
}

#[test]
fn named_destinations_cannot_bypass_hidden_layer_protection() {
    let (mut s, hidden) = session();
    s.execute("layer.setProps", json!({"layer":hidden,"name":"hidden","visible":false})).unwrap();
    s.execute("layer.new.layer", json!({"name":"visible"})).unwrap();
    for (command, mut params) in [
        ("paint.stroke", json!({"points":[[8,8]]})),
        ("paint.pencil", json!({"points":[[8,8]]})),
        ("paint.bucket", json!({"x":8,"y":8})),
        ("paint.gradient", json!({"from":[0,0],"to":[16,16]})),
    ] {
        params["layer"] = json!("hidden");
        params["target"] = json!({"colors":[0,2]});
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        let err = s.execute(command, params).unwrap_err().to_string();
        assert!(err.contains(HIDDEN), "{command}: {err}");
        assert_eq!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(hidden)), before.layer(photocraft_doc::LayerId(hidden)));
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }
}
