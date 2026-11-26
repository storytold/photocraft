//! Tests that run paint commands, moved from the engine-core `history_select_cmds` tests: 6 test(s).

use photocraft_engine::Session;
use serde_json::json;

use std::sync::Arc;

/// A document with four steps after Open, each targeting different layers.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s
}

type Look = (Arc<photocraft_doc::Document>, Option<photocraft_doc::LayerId>, Vec<photocraft_doc::LayerId>, Vec<String>, Vec<String>);

/// What the panel can see of a session: the document, the targeted layers, the entries and
/// the redo states.
fn look(s: &Session) -> Look {
    let st = s.active().unwrap();
    (st.doc.clone(), st.active_layer, st.selected_layers.clone(), st.history.entries(), st.history.redo_labels().map(str::to_string).collect())
}

/// Step from the current state to `target` with Edit › Undo / Redo, note what that gives, and
/// step back.
fn stepped(s: &mut Session, target: usize) -> Look {
    let current = s.active().unwrap().history.past_len();
    let (there, back) = if target < current { ("edit.undo", "edit.redo") } else { ("edit.redo", "edit.undo") };
    let n = target.abs_diff(current);
    for _ in 0..n {
        s.execute(there, json!({})).unwrap();
    }
    let seen = look(s);
    for _ in 0..n {
        s.execute(back, json!({})).unwrap();
    }
    seen
}

fn same(a: &Look, b: &Look) {
    assert!(Arc::ptr_eq(&a.0, &b.0), "the same document");
    assert_eq!((&a.1, &a.2, &a.3, &a.4), (&b.1, &b.2, &b.3, &b.4));
}

#[test]
fn jumping_back_and_forward_matches_stepping() {
    for target in 0..=4usize {
        for from_back in [0usize, 2, 4] {
            // Start `from_back` states before the end, then jump to `target`.
            let mut s = session();
            for _ in 0..from_back {
                s.execute("edit.undo", json!({})).unwrap();
            }
            let current = 4 - from_back;
            let expected = stepped(&mut s, target);
            let rev = s.active().unwrap().revision;
            let r = s.execute("history.selectState", json!({"state": target})).unwrap();
            assert_eq!(r["state"], json!(target));
            assert_eq!(r["changed"], json!(target != current));
            same(&look(&s), &expected);
            assert_eq!(s.active().unwrap().history.past_len(), target);
            let bumps = s.active().unwrap().revision - rev;
            assert_eq!(bumps, u64::from(target != current), "{from_back} → {target}: one revision for the jump");
        }
    }
}

#[test]
fn a_jump_reports_only_the_pixels_it_changed() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let past = s.active().unwrap().history.past_len();
    s.execute("history.selectState", json!({"state": past - 4})).unwrap();
    let damage = s.active().unwrap().last_damage.expect("a bounded damage rect, not a full recomposite");
    assert!(damage.width() < 400 && damage.height() < 300, "{damage:?}");
    // And forward again.
    s.execute("history.selectState", json!({"state": past})).unwrap();
    assert!(s.active().unwrap().last_damage.is_some());
}

#[test]
fn redo_states_survive_a_jump_and_go_with_the_next_edit() {
    let mut s = session();
    let all = s.active().unwrap().history.entries();
    s.execute("history.selectState", json!({"state": 1})).unwrap();
    let (_, _, _, entries, redo) = look(&s);
    assert_eq!(entries, all[..2].to_vec());
    assert_eq!(redo, all[2..].to_vec(), "the later states are dimmed, next one first");
    assert!(s.is_enabled("history.selectState"));
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert!(!s.active().unwrap().history.can_redo(), "a new step discards them");
}

#[test]
fn select_state_restores_the_layers_the_state_targeted() {
    let mut s = session();
    let bottom = s.active().unwrap().doc.layers.first().unwrap().id;
    // Target the bottom layer, then jump back to the first fill: its layer comes back, as
    // with Undo.
    s.select_layer(bottom).unwrap();
    let expected = stepped(&mut s, 2);
    s.select_layer(bottom).unwrap();
    s.execute("history.selectState", json!({"state": 2})).unwrap();
    same(&look(&s), &expected);
    assert_ne!(s.active().unwrap().active_layer, Some(bottom));
}

#[test]
fn a_floating_piece_goes_back_and_the_jump_still_happens() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    s.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    let all = s.active().unwrap().history.entries();
    let doc = s.active().unwrap().doc.clone();
    assert!(s.active().unwrap().floating.is_some());
    s.execute("history.selectState", json!({"state": 1})).unwrap();
    let st = s.active().unwrap();
    assert!(st.floating.is_none(), "the piece went back, it wasn't dropped");
    assert_eq!(st.history.past_len(), 1);
    let redo: Vec<String> = st.history.redo_labels().map(str::to_string).collect();
    assert_eq!(redo, all[2..].to_vec(), "no Drop step was added");
    // Forward to the state the piece was floated on: the document as it was, without the piece.
    s.execute("history.selectState", json!({"state": all.len() - 1})).unwrap();
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc));
}

#[test]
fn select_state_refuses_bad_params_and_changes_nothing() {
    let mut s = Session::new();
    assert!(s.execute("history.selectState", json!({"state": 0})).is_err(), "no document");
    assert!(!s.is_enabled("history.selectState"));
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(!s.is_enabled("history.selectState"), "only the opening state");
    let mut s = session();
    s.execute("edit.undo", json!({})).unwrap();
    let before = look(&s);
    let rev = s.active().unwrap().revision;
    for p in [
        json!({}),
        json!({"state": null}),
        json!({"state": -1}),
        json!({"state": 5}),
        json!({"state": 99}),
        json!({"state": i64::MAX}),
        json!({"state": f64::MAX}),
        json!({"state": "x"}),
        json!({"state": "1"}),
        json!({"state": [1]}),
        json!({"state": {"i": 1}}),
        json!({"state": true}),
        json!([1]),
    ] {
        assert!(s.execute("history.selectState", p.clone()).is_err(), "{p}");
        let now = look(&s);
        assert!(Arc::ptr_eq(&now.0, &before.0) && now.3 == before.3 && now.4 == before.4, "{p} changed nothing");
    }
    assert_eq!(s.active().unwrap().revision, rev);
    // The last redo state is reachable.
    assert!(s.execute("history.selectState", json!({"state": 4})).is_ok());
}
