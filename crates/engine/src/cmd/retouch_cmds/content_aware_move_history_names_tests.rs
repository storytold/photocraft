//! History step names of the Content-Aware Move Tool (#12184): "Content-Aware Move Tool" for the
//! lasso it draws with, "Content-Aware Move Selection" for a move and an extend; the plain Lasso
//! tool keeps "Lasso".

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s
}

fn last_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

const TRIANGLE: [[i32; 2]; 3] = [[16, 22], [36, 22], [16, 42]];

#[test]
fn the_lasso_is_named_for_the_tool_that_drew_it() {
    let mut s = session();
    s.execute("select.lasso", json!({"points": TRIANGLE})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Lasso"), "the Lasso tool sends no name");
    for mode in ["replace", "add", "subtract", "intersect"] {
        s.execute("select.lasso", json!({"points": TRIANGLE, "mode": mode, "historyName": "Content-Aware Move Tool"})).unwrap();
        assert_eq!(last_step(&s).as_deref(), Some("Content-Aware Move Tool"), "{mode}");
    }
    s.execute("select.lasso", json!({"points": TRIANGLE, "polygonal": true, "historyName": "Content-Aware Move Tool"})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Polygonal Lasso"), "the Polygonal Lasso keeps its own name");
}

#[test]
fn a_hostile_history_name_is_refused_without_a_step() {
    let mut s = session();
    let steps = s.active().unwrap().history.past_len();
    for name in [json!(""), json!("   "), json!("x".repeat(65)), json!("a\nb"), json!(7), json!(["Patch Tool"])] {
        assert!(s.execute("select.lasso", json!({"points": TRIANGLE, "historyName": name})).is_err(), "{name}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn a_move_and_an_extend_are_one_step_named_content_aware_move_selection() {
    for (mode, scale) in [("move", 100), ("extend", 100), ("move", 150), ("extend", 150)] {
        let mut s = session();
        s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("paint.contentAwareMove", json!({"offset": [60, 0], "mode": mode, "scaleX": scale, "scaleY": scale})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {scale}: one step");
        assert_eq!(last_step(&s).as_deref(), Some("Content-Aware Move Selection"), "{mode} {scale}");
    }
}
