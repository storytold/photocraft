//! #14667: every replacement is the history state "Find Replace", and the replacements of one
//! window session (one `coalesce` key) share that one state, which a single undo takes back.

use super::*;
use photocraft_doc::LayerId;

fn session(texts: &[&str]) -> (Session, Vec<LayerId>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
    let ids = texts.iter().map(|text| LayerId(s.execute("type.create", json!({"text": text, "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap())).collect();
    (s, ids)
}

fn text_of(s: &Session, id: LayerId) -> String {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// The history states `edit.findAndReplaceText` adds for each of `calls`.
fn added_states(s: &mut Session, calls: &[Value]) -> Vec<String> {
    let before = entries(s).len();
    for params in calls {
        s.execute("edit.findAndReplaceText", params.clone()).unwrap();
    }
    entries(s)[before..].to_vec()
}

#[test]
fn a_single_call_adds_one_find_replace_state() {
    for action in ["changeAll", "change", "changeFind"] {
        let (mut s, _) = session(&["cat Cat"]);
        s.execute("edit.findAndReplaceText", json!({"find": "cat", "action": "find"})).unwrap();
        let added = added_states(&mut s, &[json!({"find": "cat", "replace": "dog", "action": action})]);
        assert_eq!(added, ["Find Replace"], "{action}");
    }
}

#[test]
fn one_window_session_is_one_state_that_one_undo_takes_back() {
    let (mut s, ids) = session(&["cat Cat CAT cats", "a cat"]);
    let session = |action: &str| json!({"find": "cat", "replace": "dog", "action": action, "coalesce": "findReplace:1"});
    let calls = [session("find"), session("change"), session("changeFind"), session("changeFind"), session("changeFind"), session("changeAll")];
    assert_eq!(added_states(&mut s, &calls), ["Find Replace"]);
    assert_eq!((text_of(&s, ids[0]), text_of(&s, ids[1])), ("dog dog dog dogs".into(), "a dog".into()));

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((text_of(&s, ids[0]), text_of(&s, ids[1])), ("cat Cat CAT cats".into(), "a cat".into()));
}

#[test]
fn find_next_alone_adds_no_state() {
    let (mut s, _) = session(&["cat Cat"]);
    let find = json!({"find": "cat", "action": "find", "coalesce": "findReplace:1"});
    assert!(added_states(&mut s, &[find.clone(), find]).is_empty());
}
