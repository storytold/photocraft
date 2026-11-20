//! #12058: new vertical text is the history state "New Vertical Type Layer", point and paragraph
//! text alike; horizontal text stays "New Type Layer", and a committed Type Mask session, vertical
//! or not, still leaves one "Type Mask" state.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    s
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// The history states `type.create` adds with `params`, and the layer it makes.
fn create(params: Value) -> (Vec<String>, u64) {
    let mut s = session();
    let before = entries(&s).len();
    let layer = s.execute("type.create", params).unwrap()["layer"].as_u64().unwrap();
    (entries(&s)[before..].to_vec(), layer)
}

#[test]
fn vertical_text_is_a_new_vertical_type_layer() {
    for (what, params) in [
        ("point", json!({"text": "AB", "x": 150, "y": 40, "orientation": "vertical"})),
        ("paragraph", json!({"text": "AB", "box": [100, 20, 60, 200], "orientation": "vertical"})),
    ] {
        assert_eq!(create(params).0, ["New Vertical Type Layer"], "{what}");
    }
}

#[test]
fn horizontal_text_is_still_a_new_type_layer() {
    for (what, params) in [
        ("point", json!({"text": "AB", "x": 40, "y": 120})),
        ("point, horizontal", json!({"text": "AB", "x": 40, "y": 120, "orientation": "horizontal"})),
        ("paragraph", json!({"text": "AB", "box": [20, 20, 200, 60]})),
        ("paragraph, horizontal", json!({"text": "AB", "box": [20, 20, 200, 60], "orientation": "horizontal"})),
    ] {
        assert_eq!(create(params).0, ["New Type Layer"], "{what}");
    }
}
