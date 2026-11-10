//! #12071: a committed Type Mask is one history state named "Type Mask Tool", as in the reference
//! application, for horizontal and vertical, point and paragraph text.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s
}

#[test]
fn a_type_mask_is_one_type_mask_tool_state() {
    let shapes = [json!({"x": 40, "y": 100}), json!({"box": [40, 40, 200, 120]})];
    for orientation in ["horizontal", "vertical"] {
        for shape in &shapes {
            let mut s = session();
            let before = s.active().unwrap().history.past_len();
            let mut p = shape.clone();
            p["text"] = json!("Mask");
            p["size"] = json!(40);
            p["orientation"] = Value::from(orientation);
            let r = s.execute("type.createSelection", p).unwrap();
            assert_eq!(r["selected"], true, "{orientation} {shape}: {r}");
            let st = s.active().unwrap();
            assert_eq!(st.history.past_len(), before + 1, "{orientation} {shape}: one state");
            assert_eq!(st.history.entries().last().map(String::as_str), Some("Type Mask Tool"), "{orientation} {shape}");
        }
    }
}
