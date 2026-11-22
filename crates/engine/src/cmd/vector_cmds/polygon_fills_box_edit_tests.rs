//! `shape.edit` with only `fillsBox` switches a live polygon between the inscribed and the
//! box-filling geometry (CodeRabbit on #16066: the flag was missing from the geometry keys).

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

fn info(s: &mut Session, id: u64) -> Value {
    s.execute("shape.info", json!({"layer": id})).unwrap()
}

#[test]
fn a_flag_only_edit_switches_the_polygon_to_fill_its_box() {
    let mut s = session();
    let id =
        s.execute("shape.create", json!({"kind": "polygon", "rect": [400, 300, 200, 100], "sides": 6, "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let before = info(&mut s, id);
    assert!(before["live"].get("fills_box").is_none(), "created inscribed: {}", before["live"]);

    s.execute("shape.edit", json!({"layer": id, "fillsBox": true})).unwrap();
    let after = info(&mut s, id);
    assert_eq!(after["live"]["fills_box"], json!(true), "{}", after["live"]);
    assert_ne!(after["path"], before["path"], "the outline is rebuilt to fill the box");

    s.execute("shape.edit", json!({"layer": id, "fillsBox": false})).unwrap();
    let back = info(&mut s, id);
    assert!(back["live"].get("fills_box").is_none(), "{}", back["live"]);
    assert_eq!(back["path"], before["path"]);
}
