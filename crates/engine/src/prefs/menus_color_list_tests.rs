//! #14336: Edit › Menus offers the reference application's twelve colour entries, in its order,
//! and `edit.menus` accepts the four it lacked (Seafoam, Indigo, Magenta, Fuchsia) beside today's.

use serde_json::json;

use super::MENU_COLORS;
use crate::Session;

#[test]
fn the_list_has_the_reference_entries_in_its_order() {
    assert_eq!(MENU_COLORS, ["none", "red", "orange", "yellow", "green", "seafoam", "blue", "indigo", "magenta", "fuchsia", "violet", "gray"]);
}

#[test]
fn edit_menus_accepts_the_four_new_colours() {
    let mut s = Session::new();
    for (id, c) in [("file.new", "seafoam"), ("file.open", "indigo"), ("edit.fade", "magenta"), ("edit.copy", "fuchsia")] {
        s.execute("edit.menus", json!({"color": {id: c}})).unwrap();
        assert_eq!(s.prefs().menus.colors.get(id).map(String::as_str), Some(c), "{id}");
    }
}

#[test]
fn todays_colours_still_work_and_none_clears() {
    let mut s = Session::new();
    for c in ["red", "orange", "yellow", "green", "blue", "violet", "gray"] {
        s.execute("edit.menus", json!({"color": {"file.new": c}})).unwrap();
        assert_eq!(s.prefs().menus.colors.get("file.new").map(String::as_str), Some(c));
    }
    s.execute("edit.menus", json!({"color": {"file.new": "none"}})).unwrap();
    assert!(s.prefs().menus.colors.is_empty());
}

#[test]
fn an_unknown_colour_is_refused_naming_the_whole_list() {
    let mut s = Session::new();
    for c in ["teal", "Seafoam", "", "cyan"] {
        let err = s.execute("edit.menus", json!({"color": {"file.new": c}})).unwrap_err().to_string();
        assert!(err.contains("unknown colour") && err.contains("seafoam|blue|indigo|magenta|fuchsia"), "{c}: {err}");
    }
    assert!(s.prefs().menus.colors.is_empty(), "nothing stored");
}

#[test]
fn saved_preferences_with_the_colours_load() {
    let mut s = Session::new();
    s.execute("edit.menus", json!({"color": {"file.new": "seafoam", "file.open": "blue"}})).unwrap();
    let mut next = Session::new();
    next.load_prefs_json(&s.prefs_to_json()).unwrap();
    assert_eq!(next.prefs().menus.colors, s.prefs().menus.colors);
}
