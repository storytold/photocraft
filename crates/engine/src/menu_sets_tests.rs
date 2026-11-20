//! #14450: named menu sets for Edit › Menus, as measured in the reference application. A Set
//! dropdown on the Menus tab, separate from the keyboard shortcut sets, starting at the built-in
//! set; the first edit marks it modified; Save only while modified, New always, Delete never for
//! the built-in set. This file covers the engine half: the named sets in the preferences, the
//! modified flag the dialog's row reads, and `edit.menus` listing, loading, saving and deleting
//! sets. The dialog's own row and the `.mnu` files are the next steps of the same issue.

use serde_json::{Value, json};

use crate::Session;

/// Run `edit.menus` and take its reply.
fn menus(s: &mut Session, p: Value) -> Value {
    s.execute("edit.menus", p).expect("the call is valid")
}

#[test]
fn menu_sets_are_a_separate_list_from_the_keyboard_shortcut_sets() {
    // The issue measures the two dropdowns as independent: the Menus tab's Set is not the
    // Keyboard Shortcuts tab's, and neither call touches the other's list.
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "Mine"})).unwrap();
    s.execute("edit.keyboardShortcuts", json!({"set": {"edit.copy": "F9"}, "saveSet": "Keys"})).unwrap();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["sets"], json!(["Mine"]), "the Menus row lists only menu sets");
    assert_eq!(r["activeSet"], json!("Mine"));
    assert_eq!(r["modified"], json!(false), "the shortcut call did not mark the menu set modified");
    let k = s.execute("edit.keyboardShortcuts", json!({})).unwrap();
    assert_eq!(k["sets"], json!(["Keys"]), "the shortcuts row lists only shortcut sets");
    assert_eq!(k["activeSet"], json!("Keys"));
    assert_eq!(k["modified"], json!(false));
    assert_eq!(s.prefs().menus.hidden, vec!["edit.fade".to_string()], "the shortcut call left the menus alone");
    assert_eq!(s.prefs().shortcuts["edit.copy"], "F9", "and the menu call left the keys alone");
}

#[test]
fn the_menus_tab_starts_at_the_built_in_default_set() {
    let mut s = Session::new();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["activeSet"], json!(""), "no saved set is in use");
    assert_eq!(r["defaultsName"], json!("PhotoCraft Defaults"), "the built-in set's name");
    assert_eq!(r["sets"], json!([]), "nothing saved yet");
    assert_eq!(r["modified"], json!(false), "the menus are the built-in set");
}

#[test]
fn the_first_edit_marks_the_set_modified_and_save_clears_it() {
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.fade"]})).unwrap();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["modified"], json!(true), "an edit to the built-in set");
    assert_eq!(r["activeSet"], json!(""), "the built-in set keeps its name");
    s.execute("edit.menus", json!({"saveSet": "Mine"})).unwrap();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["activeSet"], json!("Mine"), "Save uses the saved set");
    assert_eq!(r["modified"], json!(false), "the menus are the set again, so the row loses the marker");
    assert_eq!(r["sets"], json!(["Mine"]));
    // Save is only enabled while modified; at the command level saving an unchanged set is fine.
    s.execute("edit.menus", json!({"hide": ["edit.paste"]})).unwrap();
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(true), "a further edit to a saved set");
    s.execute("edit.menus", json!({"saveSet": "Mine"})).unwrap();
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(false), "the set holds both items now");
    assert_eq!(s.prefs().menus.hidden, vec!["edit.fade".to_string(), "edit.paste".to_string()]);
    assert_eq!(r["sets"], json!(["Mine"]), "saving over a set does not add one");
}

#[test]
fn new_always_makes_a_set_from_the_menus_in_use() {
    // New is always enabled, even when nothing was edited: that is how "<defaults> Copy" happens.
    let mut s = Session::new();
    s.execute("edit.menus", json!({"saveSet": "Mine"})).unwrap();
    assert_eq!(s.prefs().menus.hidden, Vec::<String>::new(), "the copy took the unedited menus");
    let r = menus(&mut s, json!({"saveSet": "Mine Copy"}));
    assert_eq!(r["activeSet"], json!("Mine Copy"), "New uses the new set");
    assert_eq!(r["modified"], json!(false));
    assert_eq!(r["sets"], json!(["Mine", "Mine Copy"]));
}

#[test]
fn delete_is_refused_for_the_built_in_set_and_the_menus_stay() {
    let mut s = Session::new();
    let err = s.execute("edit.menus", json!({"deleteSet": "PhotoCraft Defaults"})).unwrap_err().to_string();
    assert!(err.contains("built-in"), "{err}");
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "Mine"})).unwrap();
    s.execute("edit.menus", json!({"hide": ["edit.paste"]})).unwrap();
    s.execute("edit.menus", json!({"deleteSet": "Mine"})).unwrap();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["activeSet"], json!(""), "the built-in set is in use again, and Delete goes disabled");
    assert_eq!(r["sets"], json!([]));
    assert_eq!(s.prefs().menus.hidden, vec!["edit.fade".to_string(), "edit.paste".to_string()], "deleting a set loses no menus");
    assert_eq!(r["modified"], json!(true), "what is shown differs from the built-in set");
    let err = s.execute("edit.menus", json!({"deleteSet": "Nope"})).unwrap_err().to_string();
    assert!(err.contains("no menu set named `Nope`"), "{err}");
}

#[test]
fn loading_a_set_applies_before_the_call_s_own_changes() {
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "color": {"edit.fill": "red"}, "saveSet": "Mine"})).unwrap();
    s.execute("edit.menus", json!({"reset": true})).unwrap();
    assert!(s.prefs().menus.hidden.is_empty() && s.prefs().menus.colors.is_empty(), "reset clears the menus");
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(true), "the built-in set's menus are empty, Mine is not");
    s.execute("edit.menus", json!({"useSet": "Mine", "hide": ["edit.paste"]})).unwrap();
    assert_eq!(s.prefs().menus.hidden, vec!["edit.fade".to_string(), "edit.paste".to_string()], "the loaded set, plus this call's edit on top");
    assert_eq!(s.prefs().menus.colors["edit.fill"], "red");
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(true), "that extra edit is unsaved");
    // The built-in set by name, as the dropdown's first entry.
    s.execute("edit.menus", json!({"useSet": "PhotoCraft Defaults"})).unwrap();
    assert!(s.prefs().menus.hidden.is_empty() && s.prefs().menus.colors.is_empty());
    let err = s.execute("edit.menus", json!({"useSet": "Nope"})).unwrap_err().to_string();
    assert!(err.contains("no menu set named `Nope`"), "{err}");
}

#[test]
fn the_dialogs_ok_sends_every_set_and_the_one_in_use() {
    let mut s = Session::new();
    s.execute("edit.menus", json!({"sets": {"A": {"hidden": ["edit.fade"], "colors": {"edit.fill": "red"}}, "B": {}}, "activeSet": "A"})).unwrap();
    let r = menus(&mut s, json!({}));
    assert_eq!(r["sets"], json!(["A", "B"]));
    assert_eq!(r["activeSet"], json!("A"));
    assert_eq!(r["modified"], json!(true), "activeSet names the set, it does not load it");
    s.execute("edit.menus", json!({"useSet": "A"})).unwrap();
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(false), "A as saved is what is shown");
    assert!(s.prefs().menus.is_hidden("edit.fade") && !s.prefs().menus.is_hidden("edit.paste"), "A's items only");
    // A set the dialog's OK left out is gone; naming it as the one in use is an error.
    s.execute("edit.menus", json!({"sets": {"B": {}}, "activeSet": "B"})).unwrap();
    assert_eq!(menus(&mut s, json!({}))["sets"], json!(["B"]), "the whole saved list was replaced");
    assert!(s.execute("edit.menus", json!({"sets": {"B": {}}, "activeSet": "A"})).is_err(), "A is not saved");
}

#[test]
fn re_listed_items_in_another_order_are_not_a_change() {
    // The marker compares the sets, not the order they were edited in.
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.paste", "edit.fade"], "saveSet": "Mine"})).unwrap();
    s.execute("edit.menus", json!({"show": ["edit.fade"]})).unwrap();
    s.execute("edit.menus", json!({"hide": ["edit.fade"]})).unwrap();
    assert_eq!(s.prefs().menus.hidden, vec!["edit.paste".to_string(), "edit.fade".to_string()], "the menus' own order moved");
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(false), "the same items in another order are still Mine");
}

#[test]
fn a_hand_edited_set_leaves_an_item_that_cannot_be_hidden_shown() {
    // #14335: File › Close and the rest can't be hidden. A set that came from an older or
    // hand-edited preferences file may still list one: it loads and stays shown.
    let mut s = Session::new();
    s.execute("prefs.set", json!({"values": {"menuSets": {"Old": {"hidden": ["file.close", "edit.fade"], "colors": {}}}, "menuSet": "Old"}})).unwrap();
    s.execute("edit.menus", json!({"useSet": "Old"})).unwrap();
    assert!(!s.prefs().menus.is_hidden("file.close"), "shown, as measured");
    assert!(s.prefs().menus.is_hidden("edit.fade"));
    assert_eq!(menus(&mut s, json!({}))["activeSet"], json!("Old"));
    // A set sent by a call is checked the same way the command checks `hide`.
    assert!(s.execute("edit.menus", json!({"sets": {"A": {"hidden": ["file.close"]}}})).is_err(), "file.close can't be hidden");
}

#[test]
fn a_saved_set_holds_only_valid_colours_and_a_bounded_count() {
    let mut s = Session::new();
    assert!(s.execute("edit.menus", json!({"sets": {"A": {"colors": {"edit.fill": "plaid"}}}})).is_err(), "unknown colour");
    assert!(s.execute("edit.menus", json!({"sets": {"PhotoCraft Defaults": {}}})).is_err(), "the built-in name is reserved");
    let many: serde_json::Map<String, Value> = (0..=64).map(|i| (format!("S{i}"), json!({}))).collect();
    assert!(s.execute("edit.menus", json!({"sets": many})).is_err(), "65 sets in one call");
    for i in 0..64 {
        s.execute("edit.menus", json!({"saveSet": format!("S{i}")})).unwrap();
    }
    assert!(s.execute("edit.menus", json!({"saveSet": "one more"})).is_err(), "the cap holds");
    assert!(s.execute("edit.menus", json!({"saveSet": "S3"})).is_ok(), "saving over a set is fine at the cap");
}

#[test]
fn bad_set_names_are_errors_and_a_good_one_is_trimmed() {
    let mut s = Session::new();
    for name in ["", "   ", "photocraft defaults", "a\u{7}b", &"x".repeat(65)] {
        assert!(s.execute("edit.menus", json!({"saveSet": name})).is_err(), "{name:?}");
        assert!(s.execute("edit.menus", json!({"deleteSet": name})).is_err(), "{name:?}");
    }
    // The built-in set's own name is reserved, but spelling it loads the built-in menus; a name
    // nothing holds is an error, loading or switching.
    assert!(s.execute("edit.menus", json!({"useSet": "photocraft defaults"})).is_ok(), "the built-in set by any spelling");
    for name in ["Nope", "a\u{7}b", &"x".repeat(65)] {
        assert!(s.execute("edit.menus", json!({"useSet": name})).is_err(), "{name:?}");
        assert!(s.execute("edit.menus", json!({"activeSet": name})).is_err(), "{name:?}");
    }
    assert!(s.execute("edit.menus", json!({"useSet": 3})).is_err(), "not a name");
    assert!(s.execute("edit.menus", json!({"sets": []})).is_err(), "not a map");
    assert!(s.execute("edit.menus", json!({"sets": {"A": 5}})).is_err(), "not a set");
    let r = menus(&mut s, json!({"saveSet": "  Mine  "}));
    assert_eq!(r["activeSet"], json!("Mine"), "the name is stored trimmed");
    assert_eq!(r["sets"], json!(["Mine"]));
}

#[test]
fn menu_sets_are_saved_with_the_preferences_and_an_older_file_loads_without_them() {
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "color": {"edit.fill": "red"}, "saveSet": "Mine"})).unwrap();
    let saved = s.prefs_to_json();
    assert!(saved.contains("\"menuSets\"") && saved.contains("\"Mine\""), "{saved}");
    let mut t = Session::new();
    assert!(t.load_prefs_json(&saved).unwrap().is_empty(), "the saved file loads clean");
    let r = menus(&mut t, json!({}));
    assert_eq!(r["activeSet"], json!("Mine"), "the set in use is remembered");
    assert_eq!(r["sets"], json!(["Mine"]));
    assert_eq!(r["modified"], json!(false), "and the menus are it, not an unsaved edit");
    s.execute("edit.menus", json!({"useSet": "Mine"})).unwrap();
    assert_eq!(s.prefs().menus.colors["edit.fill"], "red", "the set's colours came back");
    // A preferences file from before menu sets: the built-in set, nothing saved.
    let mut old = Session::new();
    assert!(old.load_prefs_json(r#"{"menus":{"hidden":["edit.fade"]}}"#).unwrap().is_empty());
    let r = menus(&mut old, json!({}));
    assert_eq!(r["activeSet"], json!(""));
    assert_eq!(r["sets"], json!([]));
    assert_eq!(r["modified"], json!(true), "edit.fade is hidden, which is not the built-in set");
}
