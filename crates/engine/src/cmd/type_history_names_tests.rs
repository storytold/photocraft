//! #12048: type steps carry the reference application's History names. Editing a type layer is
//! Edit Type Layer, a whole-layer character change Set Character Style, a paragraph change Set
//! Paragraph Style, and the orientation toggle Vertical / Horizontal Orientation. New Type Layer
//! and Kerning stay; a call mixing character and paragraph keys was not measured and keeps Set
//! Type Style.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::Session;

fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"x": 40, "y": 100, "text": "Hello", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    (s, id)
}

fn last(s: &Session) -> String {
    s.active().unwrap().history.entries().last().cloned().unwrap_or_default()
}

/// Runs `cmd` and returns the one History state it added.
fn step(s: &mut Session, cmd: &str, p: Value) -> String {
    let before = s.active().unwrap().history.past_len();
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "{cmd}: one History state");
    last(s)
}

#[test]
fn new_type_layer_and_edit_type_layer() {
    let (mut s, id) = session();
    assert_eq!(last(&s), "New Type Layer");
    assert_eq!(step(&mut s, "type.edit", json!({"layer": id.0, "text": "Hello there"})), "Edit Type Layer");
    assert_eq!(step(&mut s, "type.edit", json!({"layer": id.0, "kernPair": {"at": 1, "by": 20}})), "Kerning");
}

#[test]
fn character_and_paragraph_style_changes_are_named_apart() {
    let (mut s, id) = session();
    for p in [json!({"size": 24}), json!({"font": "Inter"}), json!({"color": "#ff0000"}), json!({"color": "#00ff00", "coalesce": "swatch"})] {
        let mut p = p;
        p["layer"] = json!(id.0);
        assert_eq!(step(&mut s, "type.setStyle", p.clone()), "Set Character Style", "{p}");
    }
    assert_eq!(step(&mut s, "type.setStyle", json!({"layer": id.0, "align": "center"})), "Set Paragraph Style");
    // Not measured: both kinds in one call keep today's name.
    assert_eq!(step(&mut s, "type.setStyle", json!({"layer": id.0, "size": 30, "align": "right"})), "Set Type Style");
}

#[test]
fn the_orientation_toggle_names_the_result() {
    let (mut s, id) = session();
    assert_eq!(step(&mut s, "type.orientation.vertical", json!({"layer": id.0})), "Vertical Orientation");
    assert_eq!(step(&mut s, "type.orientation.horizontal", json!({"layer": id.0})), "Horizontal Orientation");
}
