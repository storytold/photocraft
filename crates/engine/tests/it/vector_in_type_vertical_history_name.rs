//! Tests that run vector commands, moved from the engine-core `type_vertical_history_name` tests: 1 test(s).

use serde_json::json;

use photocraft_engine::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    s
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// The Type Mask tools' commands: a hidden working layer from `type.create`, then
/// `type.createSelection` removing it, all coalesced into the session's one step.
#[test]
fn a_type_mask_session_is_still_one_type_mask_state() {
    for orientation in ["horizontal", "vertical"] {
        let mut s = session();
        let before = entries(&s).len();
        let key = json!("type-mask-session");
        let created = json!({"text": "", "x": 150, "y": 40, "orientation": orientation, "coalesce": key});
        let layer = s.execute("type.create", created).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": layer, "visible": false, "coalesce": key})).unwrap();
        s.execute("type.edit", json!({"layer": layer, "text": "AB", "coalesce": key})).unwrap();
        s.execute("type.createSelection", json!({"layer": layer, "removeLayer": true, "coalesce": key})).unwrap();
        assert_eq!(entries(&s)[before..], ["Type Mask"], "{orientation}");
    }
    // Without a layer, `type.createSelection` makes its text itself: the same one state.
    for orientation in ["horizontal", "vertical"] {
        let mut s = session();
        let before = entries(&s).len();
        s.execute("type.createSelection", json!({"text": "AB", "x": 150, "y": 40, "size": 48, "orientation": orientation})).unwrap();
        assert_eq!(entries(&s)[before..], ["Type Mask"], "{orientation}, no layer");
    }
}
