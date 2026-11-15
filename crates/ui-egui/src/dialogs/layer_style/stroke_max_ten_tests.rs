//! #15004: a layer holds at most ten strokes from the dialog. The eleventh "+"
//! (every Stroke row's, and the fx menu's Stroke) adds nothing; removing one
//! re-enables adding. Files with more keep them all; other kinds are uncapped.
use super::*;

fn fields_with(kind: &str, n: usize) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("globalLight".into(), json!(120.0));
    f.insert("selected".into(), json!("fx1"));
    let effects: Vec<Value> = (1..=n).map(|i| json!({"id": format!("fx{i}"), "kind": kind, "on": true, "params": {}})).collect();
    f.insert("effects".into(), Value::Array(effects));
    f
}

#[test]
fn stroke_add_stops_at_ten() {
    let mut nine = fields_with("stroke", 9);
    assert!(!at_limit(&nine, "stroke"));
    add_instance(&mut nine, "stroke");
    assert_eq!(ids_of(&nine, "stroke").len(), 10);

    let mut ten = fields_with("stroke", 10);
    assert!(at_limit(&ten, "stroke"));
    let before = ten.clone();
    add_instance(&mut ten, "stroke");
    assert_eq!(ids_of(&ten, "stroke").len(), 10, "an eleventh stroke is refused");
    assert_eq!(ten["effects"], before["effects"], "nothing is inserted or reordered");
    assert_eq!(ten["selected"], before["selected"], "the selection does not move");
}

#[test]
fn removing_a_stroke_reenables_adding() {
    let mut f = fields_with("stroke", 10);
    assert!(at_limit(&f, "stroke"));
    remove_instance(&mut f, "fx10");
    assert_eq!(ids_of(&f, "stroke").len(), 9);
    assert!(!at_limit(&f, "stroke"), "dropping below ten lifts the cap");
    add_instance(&mut f, "stroke");
    assert_eq!(ids_of(&f, "stroke").len(), 10);
    assert!(at_limit(&f, "stroke"));
}

#[test]
fn more_than_ten_strokes_are_kept() {
    let mut f = fields_with("stroke", 11);
    assert!(at_limit(&f, "stroke"));
    add_instance(&mut f, "stroke");
    assert_eq!(ids_of(&f, "stroke").len(), 11, "no stroke is dropped, none added");
    assert_eq!(effects_of(&f).len(), 11);
    // The fx menu goes through the same guard.
    pick_from_fx_menu(&mut f, "stroke");
    assert_eq!(ids_of(&f, "stroke").len(), 11);
}

#[test]
fn other_kinds_are_not_limited() {
    let mut ten = fields_with("dropShadow", 10);
    assert!(!at_limit(&ten, "dropShadow"));
    add_instance(&mut ten, "dropShadow");
    assert_eq!(ids_of(&ten, "dropShadow").len(), 11, "dropShadow has no cap");

    let mut eleven = fields_with("dropShadow", 11);
    add_instance(&mut eleven, "dropShadow");
    assert_eq!(ids_of(&eleven, "dropShadow").len(), 12);
}

#[test]
fn fx_menu_does_not_add_an_eleventh_stroke() {
    let mut f = fields_with("stroke", 10);
    let before = f.clone();
    pick_from_fx_menu(&mut f, "stroke");
    assert_eq!(ids_of(&f, "stroke").len(), 10);
    assert_eq!(f["effects"], before["effects"]);
}
