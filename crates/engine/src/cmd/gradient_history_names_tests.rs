//! #12014: the Gradient tool's History states take the reference application's names.

use serde_json::json;

use crate::Session;
use crate::gradient_fill_cmds::{CREATE, SET};

fn last(s: &Session) -> Option<String> {
    s.active().and_then(|d| d.history.undo_label()).map(str::to_string)
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

#[test]
fn a_classic_gradient_is_named_by_its_target() {
    let mut s = session();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [40, 0]})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Draw classic gradient"));
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [40, 0], "target": "mask"})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Draw classic gradient on mask"));
    // Not measured, so as today: an alpha channel keeps "Gradient".
    s.execute("channel.new", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [40, 0], "target": {"channel": 0}})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Gradient"));
}

#[test]
fn a_canvas_redraw_of_a_gradient_fill_is_redraw_gradient() {
    let mut s = session();
    s.execute(CREATE, json!({"from": [0, 0], "to": [40, 0]})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("New Gradient Fill Layer"));
    s.execute(SET, json!({"from": [0, 10], "to": [40, 30], "redraw": true})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Redraw gradient"));
    // Other callers (the Properties panel, the stops) keep "Edit Gradient Fill".
    s.execute(SET, json!({"from": [5, 0], "to": [30, 0]})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Edit Gradient Fill"));
}
