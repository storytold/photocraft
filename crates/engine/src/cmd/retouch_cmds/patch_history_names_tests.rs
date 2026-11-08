//! History step names of the Patch Tool (#12117): "Patch Tool" for the lasso it draws with and
//! "Patch Selection" for applying a patch in every mode; the plain Lasso tool keeps "Lasso".

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s
}

fn last_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

#[test]
fn applying_a_patch_is_one_step_named_patch_selection_in_every_mode() {
    let modes = [
        json!({"offset": [30, 0]}),
        json!({"offset": [30, 0], "mode": "source"}),
        json!({"offset": [30, 0], "mode": "destination"}),
        json!({"offset": [30, 0], "contentAware": true}),
        json!({"offset": [30, 0], "mode": "destination", "contentAware": true}),
    ];
    for p in modes {
        let mut s = session();
        s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("paint.patch", p.clone()).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{p}: one step");
        assert_eq!(last_step(&s).as_deref(), Some("Patch Selection"), "{p}");
    }
}

const TRIANGLE: [[i32; 2]; 3] = [[16, 22], [36, 22], [16, 42]];

#[test]
fn the_lasso_is_named_for_the_tool_that_drew_it() {
    let mut s = session();
    s.execute("select.lasso", json!({"points": TRIANGLE})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Lasso"), "the Lasso tool sends no name");
    for mode in ["replace", "add", "subtract", "intersect"] {
        s.execute("select.lasso", json!({"points": TRIANGLE, "mode": mode, "historyName": "Patch Tool"})).unwrap();
        assert_eq!(last_step(&s).as_deref(), Some("Patch Tool"), "{mode}");
    }
    s.execute("select.lasso", json!({"points": TRIANGLE, "polygonal": true, "historyName": "Patch Tool"})).unwrap();
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
