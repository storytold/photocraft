//! #14174: with Check All Layers on, Check Spelling starts at the active layer, goes up the stack
//! and wraps to the bottom-most type layer, stopping just below the start; words within a layer
//! come left to right. Whether hidden and locked layers are checked is #14175, so the order is
//! checked on the layers listed, and the words on the visible unlocked ones.

use serde_json::{Value, json};

use photocraft_doc::LayerId;

use super::enabled_tests::{CMD, blank};
use crate::Session;

/// The issue's document, type layers bottom to top, then an empty pixel layer on top. The hidden
/// and locked layers' texts are not given there; they hold typos so that listing them shows.
const LAYERS: [(&str, &str); 10] = [
    ("T1 typos", "Teh quick brwn fox"),
    ("T2 repeat", "the the end"),
    ("T3 clean", "hello world"),
    ("T4 hidden", "frist hidden"),
    ("T5 locked", "lockd text"),
    ("T6 para", "paragrph text here"),
    ("T7 german", "Hallo Weltt"),
    ("T8 repeat2", "brwn dog Teh"),
    ("T9 hidden2", "secnd hidden"),
    ("T10 locked2", "lockd again"),
];

const HIDDEN: [&str; 2] = ["T4 hidden", "T9 hidden2"];

fn document() -> Session {
    let mut s = blank();
    for (i, (name, text)) in LAYERS.into_iter().enumerate() {
        let y = 30.0 + 40.0 * i as f64;
        let params = if name == "T6 para" {
            json!({"box": [10, y - 20.0, 400, 36], "text": text, "size": 12, "name": name})
        } else {
            json!({"x": 10, "y": y, "text": text, "size": 12, "name": name})
        };
        let id = s.execute("type.create", params).unwrap()["layer"].clone();
        if HIDDEN.contains(&name) {
            s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        }
        if name.contains("locked") {
            s.execute("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
        }
    }
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    s
}

fn name_of(s: &Session, id: &Value) -> String {
    s.active().unwrap().doc.layer(LayerId(id.as_u64().unwrap())).unwrap().name.clone()
}

/// (layer name, word) of each misspelling `{"action":"list"}` returns with `params` added.
fn listed(s: &mut Session, params: Value) -> Vec<(String, String)> {
    let mut p = json!({"action": "list"});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let r = s.execute(CMD, p).unwrap();
    r["misspellings"].as_array().unwrap().iter().map(|m| (name_of(s, &m["layer"]), m["word"].as_str().unwrap().to_string())).collect()
}

/// The list with `start` active and Check All Layers on (the default).
fn check_from(s: &mut Session, start: &str) -> Vec<(String, String)> {
    s.execute("layer.select", json!({"layer": start})).unwrap();
    listed(s, json!({}))
}

/// The words of the visible unlocked layers, as "layer: word". T7's "Hallo" is left out: only the
/// English dictionary flags it, and checking T7 in German is #14189.
fn words(list: &[(String, String)]) -> Vec<String> {
    list.iter()
        .filter(|(l, w)| !HIDDEN.contains(&l.as_str()) && !l.contains("locked") && !(l == "T7 german" && w == "Hallo"))
        .map(|(l, w)| format!("{l}: {w}"))
        .collect()
}

/// Asserts the listed layers come in `order` (a layer not listed is passed over).
fn assert_layer_order(list: &[(String, String)], order: &[&str]) {
    let mut visited: Vec<&str> = list.iter().map(|(l, _)| l.as_str()).collect();
    visited.dedup();
    let expected: Vec<&str> = order.iter().copied().filter(|l| visited.contains(l)).collect();
    assert_eq!(visited, expected, "layers visited out of order: {list:?}");
}

/// T1..T10 starting at `start` and wrapping.
fn rotation(start: usize) -> Vec<&'static str> {
    LAYERS.iter().cycle().skip(start).take(LAYERS.len()).map(|(n, _)| *n).collect()
}

#[test]
fn started_on_t3_goes_up_then_wraps_to_t1_and_t2() {
    let mut s = document();
    let list = check_from(&mut s, "T3 clean");
    assert_eq!(words(&list), ["T6 para: paragrph", "T7 german: Weltt", "T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn"]);
    assert_layer_order(&list, &rotation(2));
}

#[test]
fn started_on_t6_checks_t6_t7_t8_then_t1() {
    let mut s = document();
    let list = check_from(&mut s, "T6 para");
    assert_eq!(words(&list), ["T6 para: paragrph", "T7 german: Weltt", "T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn"]);
    assert_layer_order(&list, &rotation(5));
}

#[test]
fn started_on_t1_goes_bottom_up_with_words_left_to_right() {
    let mut s = document();
    let list = check_from(&mut s, "T1 typos");
    assert_eq!(list.first().map(|(l, w)| (l.as_str(), w.as_str())), Some(("T1 typos", "Teh")), "main started at \"secnd\" in T9 hidden2");
    assert_eq!(words(&list), ["T1 typos: Teh", "T1 typos: brwn", "T6 para: paragrph", "T7 german: Weltt", "T8 repeat2: brwn", "T8 repeat2: Teh"]);
    assert_layer_order(&list, &rotation(0));
}

/// The top type layer is T10 locked2: after it the check starts again at the bottom.
#[test]
fn started_on_the_top_type_layer_goes_on_from_the_bottom_up() {
    let mut s = document();
    let list = check_from(&mut s, "T10 locked2");
    assert_eq!(list.first().map(|(l, w)| (l.as_str(), w.as_str())), Some(("T1 typos", "Teh")));
    assert_layer_order(&list, &rotation(9));
}

/// T8 is the top unlocked type layer: it comes first, then T1 upward.
#[test]
fn started_on_t8_checks_it_first_then_from_the_bottom() {
    let mut s = document();
    let list = check_from(&mut s, "T8 repeat2");
    assert_eq!(words(&list), ["T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn", "T6 para: paragrph", "T7 german: Weltt"]);
    assert_layer_order(&list, &rotation(7));
}

/// `from` names the start for a caller whose active layer has moved on, as the dialog does.
#[test]
fn from_sets_the_start_whatever_layer_is_active() {
    let mut s = document();
    s.execute("layer.select", json!({"layer": "T6 para"})).unwrap();
    let t6 = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.select", json!({"layer": "T1 typos"})).unwrap();
    let list = listed(&mut s, json!({"from": t6}));
    assert_eq!(list.first().map(|(l, _)| l.as_str()), Some("T6 para"));
    assert_layer_order(&list, &rotation(5));
    assert!(s.execute(CMD, json!({"action": "list", "from": "T6 para"})).is_err(), "`from` takes an id");
}

/// Not measured: an active layer that is not a type layer keeps today's top-to-bottom order.
#[test]
fn a_pixel_or_background_start_keeps_the_top_to_bottom_order() {
    let mut s = document();
    let mut top_down: Vec<&str> = LAYERS.iter().map(|(n, _)| *n).collect();
    top_down.reverse();
    for start in ["Empty", "Background"] {
        let list = check_from(&mut s, start);
        assert_eq!(words(&list).first().map(String::as_str), Some("T8 repeat2: brwn"), "{start}");
        assert_layer_order(&list, &top_down);
    }
}

/// `from` null, an id that is no layer or a pixel layer's id: top to bottom, as without a type layer
/// to start at, whatever layer is active.
#[test]
fn a_null_unknown_or_pixel_from_checks_top_to_bottom() {
    let mut s = document();
    let mut top_down: Vec<&str> = LAYERS.iter().map(|(n, _)| *n).collect();
    top_down.reverse();
    s.execute("layer.select", json!({"layer": "Empty"})).unwrap();
    let pixel = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.select", json!({"layer": "T1 typos"})).unwrap();
    for from in [json!(null), json!(999_999), json!(pixel)] {
        let list = listed(&mut s, json!({"from": from}));
        assert_eq!(words(&list).first().map(String::as_str), Some("T8 repeat2: brwn"), "from {from}");
        assert_layer_order(&list, &top_down);
    }
}

/// The issue's one measurement with several layers selected: T8 active with T1 also selected
/// starts at T8. (Here `add` makes the added layer active, so T1 is selected first.)
#[test]
fn with_t1_and_t8_selected_and_t8_active_the_check_starts_at_t8() {
    let mut s = document();
    s.execute("layer.select", json!({"layer": "T1 typos"})).unwrap();
    s.execute("layer.select", json!({"layer": "T8 repeat2", "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2);
    let list = listed(&mut s, json!({}));
    assert_eq!(words(&list).first().map(String::as_str), Some("T8 repeat2: brwn"));
    assert_layer_order(&list, &rotation(7));
}

/// Check All Layers off still checks the active layer alone.
#[test]
fn one_layer_check_lists_only_that_layer() {
    let mut s = document();
    s.execute("layer.select", json!({"layer": "T8 repeat2"})).unwrap();
    assert_eq!(words(&listed(&mut s, json!({"allLayers": false}))), ["T8 repeat2: brwn", "T8 repeat2: Teh"]);
}
