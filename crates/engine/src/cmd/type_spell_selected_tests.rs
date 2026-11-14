//! #14184: with Check All Layers off, Check Spelling checks every selected type layer (`layers`),
//! in the order Check All Layers uses (start at the active layer, go up, wrap), and nothing else.
//! Non-type layers in the selection are passed; an explicit `layer` still checks that layer alone.

use serde_json::{Value, json};

use super::enabled_tests::CMD;
use super::order_tests::{assert_layer_order, document, listed, words};
use crate::Session;

/// The id of the layer named `name`.
fn id(s: &Session, name: &str) -> u64 {
    s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id.0).unwrap()
}

/// Selects `names` in order with `add`; the last one ends up active.
fn select(s: &mut Session, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let p = if i == 0 { json!({"layer": name}) } else { json!({"layer": name, "mode": "add"}) };
        s.execute("layer.select", p).unwrap();
    }
}

/// The session's selected layers as the dialog passes them, Check All Layers off.
fn selected(s: &Session) -> Value {
    json!({"allLayers": false, "layers": s.active().unwrap().selected_layers().iter().map(|l| l.0).collect::<Vec<_>>()})
}

/// The issue's measurement: the upper T8 active and the lower T1 added to the selection checks T8,
/// then T1, and none of the unselected type layers in between.
#[test]
fn t8_active_with_t1_selected_checks_t8_then_t1_only() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2"]);
    let p = selected(&s);
    let list = listed(&mut s, p);
    assert_eq!(words(&list), ["T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn"]);
    assert_eq!(list.len(), 4, "only the selected layers: {list:?}");
}

/// T6 active with T1 and T8 selected: up from T6 to T8, then wrap to T1.
#[test]
fn three_selected_layers_go_up_from_the_active_one_and_wrap() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2", "T6 para"]);
    let p = selected(&s);
    let list = listed(&mut s, p);
    assert_eq!(words(&list), ["T6 para: paragrph", "T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn"]);
    assert_layer_order(&list, &["T6 para", "T8 repeat2", "T1 typos"]);
}

/// `from` sets the start among the selected layers, whatever layer is active, as for all layers.
#[test]
fn from_sets_the_start_among_the_selected_layers() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2"]);
    let mut p = selected(&s);
    p["from"] = json!(id(&s, "T1 typos"));
    assert_eq!(words(&listed(&mut s, p)), ["T1 typos: Teh", "T1 typos: brwn", "T8 repeat2: brwn", "T8 repeat2: Teh"]);
}

/// The empty pixel layer (active) and Background in the selection are passed silently.
#[test]
fn non_type_layers_in_the_selection_are_passed() {
    let mut s = document();
    select(&mut s, &["T1 typos", "Background", "Empty"]);
    let p = selected(&s);
    assert_eq!(p["layers"].as_array().map(Vec::len), Some(3));
    assert_eq!(words(&listed(&mut s, p)), ["T1 typos: Teh", "T1 typos: brwn"]);
}

/// One selected layer checks that layer alone, as `allLayers: false` does today.
#[test]
fn one_selected_layer_checks_only_it() {
    let mut s = document();
    select(&mut s, &["T8 repeat2"]);
    let today = listed(&mut s, json!({"allLayers": false}));
    let p = selected(&s);
    assert_eq!(listed(&mut s, p), today);
    assert_eq!(words(&today), ["T8 repeat2: brwn", "T8 repeat2: Teh"]);
}

/// An explicit `layer` (scripts, Change, Change All) still checks only that layer, also with `layers`.
#[test]
fn an_explicit_layer_still_checks_only_that_layer() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2"]);
    let mut p = selected(&s);
    p["layer"] = json!(id(&s, "T1 typos"));
    assert_eq!(words(&listed(&mut s, p)), ["T1 typos: Teh", "T1 typos: brwn"]);
    assert_eq!(words(&listed(&mut s, json!({"layer": "T1 typos"}))), ["T1 typos: Teh", "T1 typos: brwn"]);
}

/// Headless `allLayers: false` without `layers` checks the active layer alone, as today.
#[test]
fn all_layers_off_without_layers_checks_the_active_layer_only() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2"]);
    assert_eq!(words(&listed(&mut s, json!({"allLayers": false}))), ["T8 repeat2: brwn", "T8 repeat2: Teh"]);
}

/// `layers` that is not a list of ids of this document's layers is an error.
#[test]
fn layers_that_are_not_layer_ids_are_errors() {
    let mut s = document();
    let t1 = id(&s, "T1 typos");
    for layers in [json!(t1), json!("T1 typos"), json!(["T1 typos"]), json!([t1, 999_999]), json!([-1]), json!([null])] {
        let r = s.execute(CMD, json!({"action": "list", "allLayers": false, "layers": layers}));
        assert!(r.is_err(), "layers {layers}: {r:?}");
    }
}

/// A recorded Check Spelling with `layers` is a Check All Layers off step.
#[test]
fn a_recorded_check_with_layers_records_all_layers_off() {
    let mut s = document();
    select(&mut s, &["T1 typos", "T8 repeat2"]);
    s.execute("actions.record", json!({"name": "Selected"})).unwrap();
    let p = selected(&s);
    s.execute(CMD, json!({"layers": p["layers"]})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps, [(CMD.to_string(), json!({"allLayers": false}))]);
}
