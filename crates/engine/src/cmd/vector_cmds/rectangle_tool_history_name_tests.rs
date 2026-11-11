//! #13332: a Rectangle tool drag or Create Rectangle OK is one History state "Rectangle Tool",
//! in Shape and Path mode; without the tool the commands keep their own names.

use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

fn undo_label(s: &Session) -> String {
    s.active().and_then(|d| d.history.undo_label()).unwrap_or_default().to_string()
}

fn past(s: &Session) -> usize {
    s.active().map_or(0, |d| d.history.past_len())
}

const RECT: [[f64; 2]; 4] = [[400.0, 300.0], [600.0, 300.0], [600.0, 400.0], [400.0, 400.0]];

fn rect_path(dx: f64) -> serde_json::Value {
    let knots: Vec<_> = RECT.iter().map(|p| json!([p[0] + dx, p[1]])).collect();
    json!({"subpaths": [{"closed": true, "knots": knots}]})
}

#[test]
fn shape_mode_is_one_rectangle_tool_state() {
    for p in [
        json!({"kind": "rect", "rect": [400, 300, 200, 100], "historyName": "Rectangle Tool"}),
        json!({"kind": "roundedRect", "rect": [400, 300, 200, 100], "radii": [10, 10, 10, 10], "historyName": "Rectangle Tool"}),
    ] {
        let mut s = session();
        let n = past(&s);
        s.execute("shape.create", p).unwrap();
        assert_eq!((past(&s), undo_label(&s).as_str()), (n + 1, "Rectangle Tool"));
    }
}

#[test]
fn path_mode_drags_are_rectangle_tool_states() {
    let mut s = session();
    s.execute("path.set", json!({"name": "work", "path": rect_path(0.0), "historyName": "Rectangle Tool"})).unwrap();
    assert_eq!(undo_label(&s), "Rectangle Tool");
    let n = past(&s);
    s.execute("path.set", json!({"name": "work", "path": rect_path(-300.0), "historyName": "Rectangle Tool"})).unwrap();
    assert_eq!((past(&s), undo_label(&s).as_str()), (n + 1, "Rectangle Tool"));
}

#[test]
fn without_the_tool_the_names_stay_and_unknown_names_are_refused() {
    let mut s = session();
    s.execute("shape.create", json!({"kind": "rect", "rect": [400, 300, 200, 100]})).unwrap();
    assert_eq!(undo_label(&s), "New Shape Layer");
    s.execute("path.set", json!({"name": "work", "path": rect_path(0.0)})).unwrap();
    assert_eq!(undo_label(&s), "Work Path");
    let n = past(&s);
    for bad in [json!("Ellipse Tool"), json!(7), json!(["Rectangle Tool"])] {
        assert!(s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 10, 10], "historyName": bad.clone()})).is_err());
        assert!(s.execute("path.set", json!({"name": "work", "path": rect_path(0.0), "historyName": bad})).is_err());
    }
    assert_eq!(past(&s), n);
}
