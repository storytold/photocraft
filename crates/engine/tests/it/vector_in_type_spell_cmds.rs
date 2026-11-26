//! Tests that run vector commands, moved from the engine-core `type_spell_cmds` tests: 1 test(s).

use photocraft_engine::Session;
use serde_json::json;

fn session(text: &str) -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 100, "background": "white"})).unwrap();
    let r = s.execute("type.create", json!({"x": 10, "y": 50, "text": text, "size": 12})).unwrap();
    (s, r["layer"].as_u64().unwrap())
}

#[test]
fn insert_crlf_reports_caret_in_normalized_text() {
    let (mut s, id) = session("");
    let inserted = s.execute("type.insertText", json!({"layer": id, "text": "x\r\ny"})).unwrap();

    assert_eq!(inserted["caret"], 3);
    assert_eq!(s.execute("type.info", json!({"layer": id})).unwrap()["text"], "x\ny");
    assert_eq!(s.execute("type.caret", json!({"layer": id, "index": inserted["caret"]})).unwrap()["index"], inserted["caret"]);
}
