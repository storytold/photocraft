use super::{LayerContent, LayerId, Session, Value, json};

const FIND_REPLACE: &str = "edit.findAndReplaceText";

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 500, "depth": 8, "background": "white"})).unwrap();
    s
}

fn type_layer(s: &mut Session, text: &str) -> u64 {
    s.execute("type.create", json!({"text": text, "x": 4, "y": 20})).unwrap()["layer"].as_u64().unwrap()
}

fn group_active(s: &mut Session) -> u64 {
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.active().unwrap().active_layer.unwrap().0
}

fn set_visible(s: &mut Session, layer: u64, visible: bool) {
    s.execute("layer.setProps", json!({"layer": layer, "visible": visible})).unwrap();
}

fn text(s: &Session, id: u64) -> &str {
    let LayerContent::Text(t) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!("expected type layer {id}") };
    &t.text
}

#[test]
fn change_all_counts_only_visible_unlocked_text() {
    for (case_sensitive, whole_word, expected) in [(false, false, 8), (true, false, 6), (false, true, 6), (true, true, 4)] {
        let mut s = session();
        type_layer(&mut s, "cat Cat CAT cats");
        type_layer(&mut s, "the cat sat");
        let hidden = type_layer(&mut s, "cat hidden");
        set_visible(&mut s, hidden, false);
        let locked = type_layer(&mut s, "cat locked");
        s.execute("layer.setProps", json!({"layer": locked, "locked": true})).unwrap();
        s.execute("type.create", json!({"text": "concatenate cat", "box": [4, 20, 300, 80]})).unwrap();
        type_layer(&mut s, "café cafe");
        type_layer(&mut s, "dog");
        let grouped = type_layer(&mut s, "grouped cat");
        group_active(&mut s);
        s.execute("layer.select", json!({"layer": grouped})).unwrap();
        let nested = type_layer(&mut s, "cat in hidden group");
        group_active(&mut s);
        let outer = group_active(&mut s);
        set_visible(&mut s, outer, false);
        let doc = &s.active().unwrap().doc;
        let nested_path = doc.path_of(LayerId(nested)).unwrap();
        assert!(nested_path.len() >= 3 && doc.layer_at(&nested_path).unwrap().visible && !doc.shown_at(&nested_path));
        s.execute("layer.new.layer", json!({})).unwrap();

        let history = s.active().unwrap().history.past_len();
        let reply = s
            .execute(FIND_REPLACE, json!({"action": "changeAll", "find": "cat", "replace": "dog", "caseSensitive": case_sensitive, "wholeWord": whole_word}))
            .unwrap();
        assert_eq!(reply, json!({"count": expected, "layers": 4}));
        assert_eq!(text(&s, hidden), "cat hidden");
        assert_eq!(text(&s, nested), "cat in hidden group");
        assert_eq!(text(&s, locked), "cat locked");
        assert_eq!(text(&s, grouped), "grouped dog");
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        s.undo();
        assert_eq!(text(&s, grouped), "grouped cat");
    }
}

#[test]
fn find_and_change_all_in_a_hidden_only_document_are_no_ops() {
    let mut s = session();
    let id = type_layer(&mut s, "cat");
    set_visible(&mut s, id, false);
    let history = s.active().unwrap().history.past_len();
    for all_layers in [true, false] {
        for forward in [true, false] {
            let reply = s.execute(FIND_REPLACE, json!({"action": "find", "find": "cat", "allLayers": all_layers, "forward": forward})).unwrap();
            assert_eq!(reply["found"], Value::Null);
        }
        let reply = s.execute(FIND_REPLACE, json!({"action": "changeAll", "find": "cat", "replace": "dog", "allLayers": all_layers})).unwrap();
        assert_eq!(reply, json!({"count": 0, "layers": 0}));
    }
    assert_eq!(text(&s, id), "cat");
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn hiding_a_saved_match_prevents_change_and_change_find() {
    for action in ["change", "changeFind"] {
        for hide_group in [false, true] {
            let mut s = session();
            let target = type_layer(&mut s, "cat target");
            let group = group_active(&mut s);
            s.execute("layer.select", json!({"layer": target})).unwrap();
            let reply = s.execute(FIND_REPLACE, json!({"action": "find", "find": "cat"})).unwrap();
            assert_eq!(reply["found"]["layer"], target);

            let hidden = if hide_group { group } else { target };
            set_visible(&mut s, hidden, false);
            // Put another match outside the hidden group. Change/Find should reach it.
            s.execute("layer.select", json!({"layer": s.active().unwrap().doc.background().unwrap().id.0})).unwrap();
            let fallback = type_layer(&mut s, "cat fallback");
            let history = s.active().unwrap().history.past_len();
            let reply = s.execute(FIND_REPLACE, json!({"action": action, "find": "cat", "replace": "dog"})).unwrap();
            assert_eq!(reply["changed"], Value::Null, "{action}, group={hide_group}");
            if action == "changeFind" {
                assert_eq!(reply["found"]["layer"], fallback);
            }
            assert_eq!(text(&s, target), "cat target");
            assert_eq!(text(&s, fallback), "cat fallback");
            assert_eq!(s.active().unwrap().history.past_len(), history);

            if action == "change" {
                set_visible(&mut s, hidden, true);
                let reply = s.execute(FIND_REPLACE, json!({"action": "change", "find": "cat", "replace": "dog"})).unwrap();
                assert_eq!(reply["changed"], Value::Null, "showing the layer cannot restore the discarded match");
            }
        }
    }
}

#[test]
fn find_warning_respects_direction_scope_and_the_current_match() {
    use super::{BlockedTextLayer, find_replace_warning};

    let mut s = session();
    let first = type_layer(&mut s, "cat first cat");
    let locked = type_layer(&mut s, "cat locked");
    s.execute("layer.setProps", json!({"layer": locked, "locked": true})).unwrap();
    type_layer(&mut s, "cat last");
    let params = |forward| json!({"action": "find", "find": "cat", "forward": forward});
    for (forward, starts) in [(true, [0, 10]), (false, [10, 0])] {
        s.execute("layer.select", json!({"layer":first})).unwrap();
        let p = params(forward);
        assert!(find_replace_warning(&s, &p, None, None, |_, _| false).unwrap().is_none());
        for start in starts {
            let found = s.execute(FIND_REPLACE, p.clone()).unwrap()["found"].clone();
            assert_eq!(found["layer"], first);
            assert_eq!(found["start"], start);
        }
        let cursor = s.edit_state.find_cursor.clone();
        let warning = find_replace_warning(&s, &p, cursor.as_ref(), s.edit_state.find_start, |_, _| false).unwrap().unwrap();
        assert_eq!((warning.layer.0, warning.reason), (locked, BlockedTextLayer::Locked));
        assert_eq!(s.edit_state.find_cursor, cursor);
    }
    let scoped = json!({"action": "find", "find": "cat", "allLayers": false, "layer": first});
    assert!(find_replace_warning(&s, &scoped, None, None, |_, _| false).unwrap().is_none());
    let scoped = json!({"action": "find", "find": "cat", "allLayers": false, "layer": locked});
    assert_eq!(find_replace_warning(&s, &scoped, None, None, |_, _| false).unwrap().unwrap().layer.0, locked);
}
