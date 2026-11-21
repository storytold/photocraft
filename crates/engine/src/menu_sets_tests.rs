//! #14450: named menu sets for Edit › Menus, as measured in the reference application. A Set
//! dropdown on the Menus tab, separate from the keyboard shortcut sets, starting at the built-in
//! set; the first edit marks it modified; Save only while modified, New always, Delete never for
//! the built-in set. This file covers the engine half: the named sets in the preferences, the
//! modified flag the dialog's row reads, `edit.menus` listing, loading, saving and deleting sets,
//! and the `.mnu` files in the settings directory - the measured layout, the numeric command ids
//! and colour numbers, keeping unknown entries on a rewrite, and a malformed file as an error.
//! The dialog's own row is `crates/ui-egui/src/prefs/prefs_ui/menu_set_tests.rs`.

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
    // The reply of the save itself, not the one read before it: only a fresh reply says what this
    // `saveSet` did to the list of sets.
    let r = menus(&mut s, json!({"saveSet": "Mine"}));
    assert_eq!(r["modified"], json!(false), "the set holds both items now");
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
fn a_replacement_that_leaves_out_the_set_in_use_clears_its_name() {
    // `sets` replaces every saved set, so a set the replacement leaves out is gone - the same as
    // `deleteSet`. Its name does not stay behind as the one in use: a later replacement that holds
    // that name again would otherwise select it with no `useSet` and no `activeSet` in the call.
    let mut s = Session::new();
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "A"})).unwrap();
    s.execute("edit.menus", json!({"sets": {}})).unwrap();
    assert!(s.prefs().menu_set.is_empty(), "the set the replacement left out is not the one in use");
    let r = menus(&mut s, json!({"sets": {"A": {"hidden": ["file.new"]}}}));
    assert_eq!(r["sets"], json!(["A"]), "the name is saved again");
    assert_eq!(r["activeSet"], json!(""), "but nothing loaded it");
    assert_eq!(s.prefs().menu_set, "", "and the stored preferences say the same");
    // A replacement that keeps the set in use keeps its name.
    s.execute("edit.menus", json!({"useSet": "A"})).unwrap();
    let r = menus(&mut s, json!({"sets": {"A": {"hidden": ["file.new"]}, "B": {}}}));
    assert_eq!(r["activeSet"], json!("A"), "A is still saved, so it stays the set in use");
}

#[test]
fn two_names_that_are_one_set_name_after_the_trim_are_refused() {
    // A set name is stored trimmed (`bad_set_names_are_errors_and_a_good_one_is_trimmed`), so in one
    // `sets` map `A` and ` A ` name the same set. Letting the later entry replace the earlier one
    // would answer with one set where the call sent two, and lose one set's menus without a word.
    let mut s = Session::new();
    let err = s.execute("edit.menus", json!({"sets": {"A": {"hidden": ["edit.fade"]}, " A ": {"hidden": ["edit.paste"]}}})).unwrap_err().to_string();
    assert!(err.contains("twice"), "{err}");
    let r = menus(&mut s, json!({}));
    assert_eq!(r["sets"], json!([]), "the refused replacement left the saved sets as they were");
    // One name with spaces around it is still stored trimmed, as it was.
    let r = menus(&mut s, json!({"sets": {"  A  ": {}}}));
    assert_eq!(r["sets"], json!(["A"]), "a lone name keeps its trimmed spelling");
}

#[test]
fn modified_flag_ignores_the_none_colour_sentinel() {
    // `"none"` is the colour picker's no-colour choice, not a colour: the `color` param removes the
    // entry rather than storing one. A saved set may still carry one (the dialog's OK sends what it
    // has), and then asking for no colour again is a no-op that must not mark the set modified, nor
    // leave a phantom colour in the menus.
    let mut s = Session::new();
    s.execute("edit.menus", json!({"sets": {"A": {"colors": {"edit.fill": "none"}}}})).unwrap();
    let r = menus(&mut s, json!({"useSet": "A"}));
    assert_eq!(r["colors"], json!({}), "the sentinel is not a colour in the menus");
    let r = menus(&mut s, json!({"color": {"edit.fill": "none"}}));
    assert_eq!(r["colors"], json!({}), "asking for no colour again changes nothing");
    assert_eq!(r["modified"], json!(false), "that no-op is not an edit to A, so no \"(modified)\"");
    assert_eq!(menus(&mut s, json!({}))["modified"], json!(false), "and the plain read agrees");
    // A colour the set does not have still marks it, so the flag keeps working.
    let r = menus(&mut s, json!({"color": {"edit.fill": "red"}}));
    assert_eq!(r["modified"], json!(true), "a new colour is an edit");
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
fn a_save_of_a_legacy_set_keeps_the_row_the_engine_cannot_hide() {
    // `saveSet` stores the menus as they stand, including a row that the test above leaves shown.
    // The engine's rule for that row is to load it and show it, never to refuse it, and a copy of it
    // is the same data: checking the snapshot here would answer a Save of the menus in front of the
    // user with "`file.close` can't be hidden" for a row they never hid, and a set from an older or
    // hand-edited file could then never be saved under another name. What a CALL may send stays
    // checked: the same rows through `sets` are an error, pinned below and in
    // `a_hand_edited_set_leaves_an_item_that_cannot_be_hidden_shown`.
    let mut s = Session::new();
    s.execute("prefs.set", json!({"values": {"menuSets": {"Old": {"hidden": ["file.close", "edit.fade"], "colors": {}}}, "menuSet": "Old"}})).unwrap();
    s.execute("edit.menus", json!({"useSet": "Old"})).unwrap();
    let r = menus(&mut s, json!({"saveSet": "Copy"}));
    assert_eq!(r["sets"], json!(["Copy", "Old"]), "the copy is a saved set");
    assert_eq!(r["activeSet"], json!("Copy"), "and the set in use");
    assert_eq!(r["modified"], json!(false), "it holds the menus exactly as they stood");
    assert_eq!(s.prefs().menu_sets["Copy"].hidden, ["edit.fade", "file.close"], "the legacy row came with it, ordered");
    assert!(!s.prefs().menus.is_hidden("file.close"), "and it still reads as shown");
    assert!(s.execute("edit.menus", json!({"sets": {"Copy": {"hidden": ["file.close", "edit.fade"]}}})).is_err(), "a call sending those rows is refused");
    assert_eq!(s.prefs().menu_sets["Copy"].hidden, ["edit.fade", "file.close"], "and the refused call changed nothing");
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

// ---- the `.mnu` files: the sets as files in the settings directory (#14450) ----

use crate::menu_sets::{
    self, MAX_MNU_BYTES, MenuSet, REF_ITEMS, atomic_write, color_name, color_number, file_name, first_per_set_name, folder, from_mnu, merged_with_prefs,
    path_of, read_sets, ref_command, ref_item, set_name_of, to_mnu, write_set,
};

/// A set: its hidden items and its colours, as `saveSet` would store them.
fn set(hidden: &[&str], colors: &[(&str, &str)]) -> MenuSet {
    MenuSet { hidden: hidden.iter().map(|id| (*id).to_owned()).collect(), colors: colors.iter().map(|(id, c)| ((*id).to_owned(), (*c).to_owned())).collect() }
}

/// A set file named "Mine" holding `rows` between the measured root element's tags.
fn mnu(rows: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Mine=Mine\" modified=\"0\">\n\
         {rows}</photocraft-menu-settings>\n"
    )
}

/// One measured row of a set file, in the shape the file is written and read back.
fn row(id: u32, parent: u32, menu_type: u32, hidden: u32, color: u32) -> String {
    format!("  <command kind=\"static\" id=\"{id}\" parentMenuID=\"{parent}\" menuType=\"{menu_type}\" hidden=\"{hidden}\" color=\"{color}\"/>\n")
}

/// A folder with `files` (name, text) written into it.
fn folder_with(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("a temp folder");
    let dir = folder(tmp.path());
    std::fs::create_dir_all(&dir).expect("the folder is there");
    for (name, text) in files {
        std::fs::write(dir.join(name), text).expect("the file is written");
    }
    (tmp, dir)
}

/// The hidden items of `set`, in name order, so a read can be compared with what was written.
fn hidden_of(s: &MenuSet) -> Vec<String> {
    let mut v = s.hidden.clone();
    v.sort();
    v
}

#[test]
fn a_set_written_to_a_file_reads_back_as_the_same_set() {
    // The issue's "writes and reads the measured layout": one element per changed item, and every
    // one of those changes comes back.
    let s = set(&["file.new", "edit.fade"], &[("edit.fade", "red"), ("layer.mergeVisible", "gray")]);
    let read = from_mnu(&to_mnu("Darkroom", &s, &[])).expect("what PhotoCraft writes reads back");
    assert_eq!(hidden_of(&read.set), vec!["edit.fade".to_string(), "file.new".to_string()]);
    assert_eq!(read.set.colors, s.colors, "both colours came back, gray as number 7");
    assert!(read.kept.is_empty(), "every row was ours: {read:?}");
    // And the round trip is stable: writing what was read writes the same file.
    assert_eq!(to_mnu("Darkroom", &read.set, &read.kept), to_mnu("Darkroom", &s, &[]));
}

#[test]
fn the_file_is_the_measured_xml_with_one_element_per_changed_item() {
    // Root attributes in the measured order, and one `<command>` per changed item only: the set
    // hides File › New and colours Edit › Fade, so it is exactly those two rows and no third.
    let text = to_mnu("Darkroom", &set(&["file.new"], &[("edit.fade", "red")]), &[]);
    assert_eq!(
        text,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Darkroom=Darkroom\" modified=\"0\">\n",
            "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
            "  <command kind=\"static\" id=\"1154\" parentMenuID=\"3\" menuType=\"0\" hidden=\"0\" color=\"1\"/>\n",
            "</photocraft-menu-settings>\n"
        ),
        "the measured layout, attribute order as written there"
    );
}

#[test]
fn a_hidden_item_without_a_colour_is_written_with_colour_zero() {
    // Measured: "A hidden item without a colour is written with color="0"" - and it reads back as
    // hidden with no colour, not as a `none` colour entry.
    let text = to_mnu("Mine", &set(&["file.open"], &[]), &[]);
    assert!(text.contains(&row(20, 2, 0, 1, 0)), "{text}");
    let read = from_mnu(&text).expect("the row reads");
    assert_eq!(hidden_of(&read.set), vec!["file.open".to_string()]);
    assert!(read.set.colors.is_empty(), "no colour entry for color=0: {:?}", read.set.colors);
}

#[test]
fn the_measured_command_numbers_name_our_commands_in_both_directions() {
    // The ids measured in the issue, and the menu each sits in: 2 File, 3 Edit, 41 Layers panel,
    // with menuType 1 for the panel menu.
    let measured = [
        (10, "file.new", 2, false),
        (20, "file.open", 2, false),
        (21, "file.openAs", 2, false),
        (22, "file.openAsSmartObject", 2, false),
        (31, "file.close", 2, false),
        (37, "file.closeAll", 2, false),
        (46, "file.closeOthers", 2, false),
        (133, "edit.toggleLastState", 3, false),
        (1154, "edit.fade", 3, false),
        (1099, "layer.new.layer", 41, true),
        (1139, "layer.mergeVisible", 41, true),
    ];
    for (number, command, parent, panel) in measured {
        let item = ref_item(command).unwrap_or_else(|| panic!("{command} has no measured number"));
        assert_eq!((item.number, item.parent, item.panel), (number, parent, panel), "{command}");
        assert_eq!(ref_command(number), Some(command), "{number} is {command}");
    }
    // Only the measured ids: a row of another number is an unknown entry, not a guess. The file
    // browser rows (1520, 1530), Paste Special (1298) and the Info panel rows (3127, 7959) name
    // items PhotoCraft's menu tree has no command for yet.
    assert_eq!(REF_ITEMS.len(), measured.len(), "no number beyond the measured list");
    for number in [1520, 1530, 1298, 3127, 7959] {
        assert_eq!(ref_command(number), None, "{number} is not one of ours");
    }
}

#[test]
fn the_colour_numbers_are_the_measured_zero_to_seven() {
    // None 0, Red 1, Orange 2, Yellow 3, Green 4, Blue 5, Violet 6, Gray 7. 8 to 11 are #14336's
    // twelve-entry list, so they are not this issue's table.
    for (name, number) in [("none", 0), ("red", 1), ("orange", 2), ("yellow", 3), ("green", 4), ("blue", 5), ("violet", 6), ("gray", 7)] {
        assert_eq!(color_number(name), Some(number), "{name}");
        assert_eq!(color_name(number), Some(name), "{number}");
    }
    for number in [8, 9, 10, 11, 99] {
        assert_eq!(color_name(number), None, "{number} is #14336's, not ours");
    }
    // Every colour of ours writes its own number, so the two directions agree.
    for (name, number) in [("red", 1), ("orange", 2), ("yellow", 3), ("green", 4), ("blue", 5), ("violet", 6), ("gray", 7)] {
        let text = to_mnu("Mine", &set(&[], &[("edit.fade", name)]), &[]);
        assert!(text.contains(&row(1154, 3, 0, 0, number)), "{name} is {number}: {text}");
        assert_eq!(from_mnu(&text).expect("{name} reads").set.colors["edit.fade"], name);
    }
}

#[test]
fn a_file_holding_rows_we_do_not_know_writes_them_back() {
    // "Unknown entries are kept when we rewrite a file": the numbers we have no command for, a row
    // that is not a static one, and an element that is not a command at all all stay as they were.
    let unknown = [
        row(1520, 2, 0, 1, 0).trim().to_string(),
        row(3127, 40, 1, 0, 4).trim().to_string(),
        "<command kind=\"dynamic\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>".to_string(),
        "<panel name=\"Layers\"/>".to_string(),
    ];
    let text = mnu(&format!("{}{}", row(10, 2, 0, 1, 0), unknown.iter().map(|entry| format!("  {entry}\n")).collect::<String>()));
    let read = from_mnu(&text).expect("the known row reads and the rest is kept");
    assert_eq!(hidden_of(&read.set), vec!["file.new".to_string()], "only id 10 is one of ours");
    assert!(read.set.colors.is_empty(), "3127's colour stayed in its own row: {:?}", read.set.colors);
    assert_eq!(read.kept, unknown.to_vec(), "kept in the order they were read");
    // The rewrite holds both our row and every one of theirs, and reads the same way again.
    let again = from_mnu(&to_mnu("Mine", &read.set, &read.kept)).expect("the rewrite reads back");
    assert_eq!(again, read, "reading and writing a file is stable");
}

#[test]
fn a_kept_row_keeps_its_prefix_its_own_declaration_and_its_markup() {
    // "Kept as it was read" has to mean the entry as it stood in the file: a name of theirs with its
    // prefix, the namespace declaration that gives that row's prefix its meaning, an attribute of
    // theirs we have no name for, a nested element, a CDATA section, and a comment or a processing
    // instruction standing between the rows.
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photoshop-menu-settings version=\"2\" xmlns:ps=\"http://ns.adobe.com/photoshop/\" filename=\"$$$/FileName/Presets/MenuCustomization/My Menus=My Menus\" modified=\"0\">\n",
        "  <!-- the set as the reference application saved it -->\n",
        "  <ps:command xmlns:ps=\"http://ns.adobe.com/photoshop/\" ps:extra=\"1\" kind=\"static\" id=\"1520\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "  <ps:panel><ps:name>Layers</ps:name><![CDATA[a < b]]></ps:panel>\n",
        "  <?mnu-note kept?>\n",
        "</photoshop-menu-settings>\n",
    );
    let read = from_mnu(text).expect("the reference application's file reads");
    assert_eq!(read.kept.len(), 4, "the comment, both elements and the instruction, in the order they were read: {read:?}");
    assert!(read.kept[1].starts_with("<ps:command"), "its prefix as it was written: {:?}", read.kept[1]);
    assert!(read.kept[1].contains("xmlns:ps=\"http://ns.adobe.com/photoshop/\""), "the declaration of that prefix stays with the row: {:?}", read.kept[1]);
    assert!(read.kept[1].contains("ps:extra=\"1\""), "an attribute of theirs we have no name for: {:?}", read.kept[1]);
    assert!(read.kept[2].contains("<ps:name>Layers</ps:name>"), "a nested element keeps its prefix too: {:?}", read.kept[2]);
    assert!(read.kept[2].contains("<![CDATA[a < b]]>"), "a section stays a section: {:?}", read.kept[2]);
    assert_eq!(read.kept[0], "<!-- the set as the reference application saved it -->", "a comment between the rows is one of the entries");
    assert_eq!(read.kept[3], "<?mnu-note kept?>", "and so is a processing instruction");
    assert!(read.set.hidden.is_empty() && read.set.colors.is_empty(), "none of these rows names one of our items: {:?}", read.set);
    // Our rewrite holds every one of them, and reading our own file gives the same entries again.
    let again = from_mnu(&to_mnu("My Menus", &read.set, &read.kept)).expect("our rewrite reads back");
    assert_eq!(again.kept, read.kept, "a kept entry is written as it was read: {:?} against {:?}", again.kept, read.kept);
}

#[test]
fn what_a_kept_entry_does_not_keep_is_named_and_pinned() {
    // The two limits of the pinned `craft_xml` reader, stated as tests rather than claimed away.
    // One: an entity reference inside a kept entry is what its reader makes of it - the numeric form
    // becomes its character and a predefined one comes back as a reference, while a name the reader
    // does not know is dropped rather than guessed at.
    let read = from_mnu(&mnu("  <note>a &#65; and &amp; and &unknownthing; b</note>\n")).expect("a foreign element is kept");
    assert_eq!(read.kept.len(), 1, "{read:?}");
    assert!(read.kept[0].contains("a A and "), "the numeric reference became its character: {:?}", read.kept[0]);
    assert!(read.kept[0].contains("&amp;"), "the predefined reference comes back as a reference: {:?}", read.kept[0]);
    assert!(!read.kept[0].contains("unknownthing"), "a reference name the reader does not know is not carried over: {:?}", read.kept[0]);
    // Two: a namespace declaration the SOURCE root carries belongs to that root. We write our own
    // root, so a prefixed row we keep carries its prefix but not the URI their root declared for it.
    let foreign = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photoshop-menu-settings version=\"2\" xmlns:ps=\"http://ns.adobe.com/photoshop/\">\n",
        "  <ps:command kind=\"static\" id=\"1520\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "</photoshop-menu-settings>\n",
    );
    let read = from_mnu(foreign).expect("the reference's file reads");
    let written = to_mnu("Mine", &read.set, &read.kept);
    assert!(written.contains("<ps:command"), "the row keeps its prefix: {written}");
    assert!(!written.contains("xmlns:ps"), "the foreign root's declaration does not move onto ours: {written}");
}

#[test]
fn writing_a_set_over_a_file_keeps_the_rows_that_file_held() {
    // The row's Save overwrites the set's file without a dialog; the entries the file held that we
    // cannot name do not go with it.
    let (_tmp, dir) = folder_with(&[("Mine.MNU", &mnu(&row(1520, 2, 0, 1, 0)))]);
    write_set(&dir, "Mine", &set(&["file.new"], &[])).expect("the file was written");
    let text = std::fs::read_to_string(dir.join("Mine.MNU")).expect("the set's file");
    assert!(text.contains(&row(10, 2, 0, 1, 0)), "the new set: {text}");
    assert!(text.contains(&row(1520, 2, 0, 1, 0)), "the unknown row stayed: {text}");
    assert_eq!(text.matches("<command").count(), 2, "one row for the one changed item, plus theirs: {text}");
    assert!(!text.contains("edit.fade"), "the file holds numbers, not our command ids: {text}");
}

#[test]
fn xml_that_is_not_a_set_is_an_error_and_not_a_crash() {
    // The issue's first malformed case.
    for text in ["", "menus, not xml", "<", "<<<>>>", "<a><b></a>", "\u{0}", "</photocraft-menu-settings>"] {
        assert!(from_mnu(text).is_err(), "{text:?} read as a set");
    }
    // Well-formed XML that is not a menu set file at all (a `.kys` root, say), and a menu set file
    // of a version we have not measured.
    assert!(from_mnu("<photoshop-keyboard-shortcuts version=\"2\" />").is_err(), "the wrong root");
    assert!(from_mnu("<photocraft-menu-settings version=\"3\" />").is_err(), "a version we do not know");
    assert!(from_mnu("<photocraft-menu-settings />").is_err(), "no version at all");
}

#[test]
fn an_unknown_colour_number_is_an_error_and_not_a_crash() {
    // The issue's second malformed case: the reference's number, or anything that is not one.
    for color in ["8", "9", "11", "12", "-1", "seven", ""] {
        let text = mnu(&format!("  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"{color}\"/>\n"));
        assert!(from_mnu(&text).is_err(), "colour `{color}` read as a set");
    }
    // A hidden value that is not 0 or 1 is malformed the same way; a row without an id, or one
    // whose id is not a number, is just an entry we do not know.
    assert!(from_mnu(&mnu("  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"2\" color=\"0\"/>\n")).is_err());
    let read = from_mnu(&mnu("  <command kind=\"static\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n  \
         <command kind=\"static\" id=\"x\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n"))
    .expect("unknown rows are kept");
    assert_eq!(read.kept.len(), 2, "{read:?}");
    assert!(read.set.hidden.is_empty());
}

#[test]
fn a_huge_set_file_is_an_error_and_not_a_crash() {
    // The issue's third malformed case. The file is well-formed XML: it is simply far larger than
    // a menu set anyone edits, and is refused without being parsed.
    let (_tmp, dir) = folder_with(&[("Big.MNU", "")]);
    let big = format!("{}{}", " ".repeat(MAX_MNU_BYTES + 1024), mnu(&row(10, 2, 0, 1, 0)));
    std::fs::write(dir.join("Big.MNU"), &big).expect("the big file is written");
    let (sets, errors) = read_sets(&dir);
    assert!(sets.is_empty(), "the huge file did not come in: {sets:?}");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let first = &errors[0];
    assert!(first.contains("Big.MNU"), "{first} names the file");
    assert!(from_mnu(&big).is_err(), "the same text read directly is refused too");
    // And the folder is still writable beside it.
    write_set(&dir, "Mine", &set(&["file.new"], &[])).expect("a set beside a refused file still writes");
    assert_eq!(read_sets(&dir).0.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Mine"]);
}

#[test]
fn a_broken_file_in_the_folder_is_reported_and_the_others_come_in() {
    let (_tmp, dir) = folder_with(&[("Good.MNU", &mnu(&row(31, 2, 0, 0, 5))), ("Broken.MNU", "<photocraft-menu-settings"), ("Notes.txt", "not a set")]);
    let (files, errors) = read_sets(&dir);
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Good"], "the readable set came in");
    assert_eq!(files[0].set.colors["file.close"], "blue", "id 31 with colour 5");
    assert_eq!(errors.len(), 1, "{errors:?}: only the malformed .mnu is a problem, not the text file");
    let first = &errors[0];
    assert!(first.contains("Broken.MNU"), "{first}");
    // A file we write and read again is the set it was, named by its own stem.
    assert_eq!(set_name_of(&dir.join("Good.MNU")).as_deref(), Some("Good"));
    assert_eq!(file_name("Good"), "Good.MNU");
    assert_eq!(set_name_of(&dir.join("Notes.txt")).as_deref(), None, "not a set file");
}

#[test]
fn the_first_set_makes_the_menu_sets_folder_and_writes_its_file_there() {
    // The folder is there only when the first set is saved: the settings directory already exists,
    // the menu-sets folder inside it is made on demand.
    let tmp = tempfile::tempdir().expect("a temp folder");
    let dir = folder(tmp.path());
    assert!(!dir.exists(), "nothing was written yet");
    write_set(&dir, "Dark Room", &set(&["file.new"], &[("edit.fade", "violet")])).expect("the write made the folder");
    assert!(dir.is_dir(), "the folder came with it");
    assert!(dir.join("Dark Room.MNU").is_file(), "the set's file, named by the set");
    let (files, errors) = read_sets(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Dark Room"]);
    assert_eq!(hidden_of(&files[0].set), vec!["file.new".to_string()]);
    assert_eq!(files[0].set.colors["edit.fade"], "violet");
    // Deleting it takes the file, and a file that is already gone is not an error.
    menu_sets::delete_set(&dir, "Dark Room").expect("the file went");
    assert!(!dir.join("Dark Room.MNU").exists());
    assert!(menu_sets::delete_set(&dir, "Dark Room").is_ok(), "already gone is deleted");
    // A file whose existing text is malformed is not overwritten blind.
    std::fs::create_dir_all(&dir).expect("the folder is there");
    std::fs::write(dir.join("Broken.MNU"), "<photocraft-menu-settings").expect("the broken file");
    assert!(write_set(&dir, "Broken", &set(&["file.new"], &[])).is_err(), "a malformed file is not rewritten");
    assert_eq!(std::fs::read_to_string(dir.join("Broken.MNU")).expect("still there"), "<photocraft-menu-settings", "the write refused, the file stayed");
}

#[test]
fn a_folder_wearing_a_set_file_s_name_is_left_alone() {
    // `read_sets` reads the files of the folder, not whatever else it holds: a subfolder wearing a
    // set file's name is neither a set nor a file we failed to read, so it says nothing at all.
    let tmp = tempfile::tempdir().expect("a settings folder");
    let dir = folder(tmp.path());
    std::fs::create_dir_all(dir.join("Mine.MNU")).expect("a folder wearing a set file's name");
    std::fs::write(dir.join("Good.MNU"), mnu(&row(31, 2, 0, 0, 5))).expect("a real set file");
    let (files, errors) = read_sets(&dir);
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Good"], "the subfolder is not a set: {files:?}");
    assert!(errors.is_empty(), "and it is not a file that could not be read: {errors:?}");
    // The same name as a set to write is an error rather than a crash: the folder is in the way.
    assert!(write_set(&dir, "Mine", &set(&["file.new"], &[])).is_err(), "a folder is in the way of that set's file");
    assert!(dir.join("Mine.MNU").is_dir(), "and nothing was written through it");
}

#[test]
fn a_set_write_leaves_one_file_in_the_folder_and_no_staging_behind() {
    // The write lands as a whole file: staged beside the target and moved in, so the folder never
    // keeps a second file of its own and the target is always a set file that reads - the old bytes
    // until the move happens, the new ones after it, never half of either.
    let (_tmp, dir) = folder_with(&[("Mine.MNU", &mnu(&row(1520, 2, 0, 1, 0)))]);
    write_set(&dir, "Mine", &set(&["file.new"], &[])).expect("the rewrite");
    let left: Vec<String> = std::fs::read_dir(&dir).expect("the folder").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["Mine.MNU".to_string()], "one file in the folder, nothing staged left behind");
    let text = std::fs::read_to_string(dir.join("Mine.MNU")).expect("the file");
    assert!(text.contains(&row(10, 2, 0, 1, 0)), "the new set is whole: {text}");
    assert!(text.contains(&row(1520, 2, 0, 1, 0)), "and the row the file held is still there: {text}");
    assert!(from_mnu(&text).is_ok(), "what the folder holds reads as a set");
}

#[test]
fn a_refused_move_leaves_the_old_file_and_no_staging() {
    // The other half of the atomic write: the staged file is moved in over the target, so a move that
    // is refused - the one moment a crash can catch a write - leaves the previous bytes whole rather
    // than a truncated beginning of the new text, and takes its staging file with it. This names the
    // function the move goes through, so it was written with that function and does not compile
    // without it; the successful case above runs the same code through `write_set`.
    let (_tmp, dir) = folder_with(&[("Mine.MNU", &mnu(&row(1520, 2, 0, 1, 0)))]);
    let path = dir.join("Mine.MNU");
    let before = std::fs::read_to_string(&path).expect("the old file");
    let half = "menu set text that never arrives";
    let e = atomic_write(&path, half, &|_, _| Err(std::io::Error::other("the move is refused"))).expect_err("the move was refused");
    assert!(e.contains("Mine.MNU"), "{e}");
    assert_eq!(std::fs::read_to_string(&path).expect("the target is still readable"), before, "the old bytes, not part of the new text");
    let left: Vec<String> = std::fs::read_dir(&dir).expect("the folder").flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["Mine.MNU".to_string()], "the staged file went with the refusal");
    // And the same path with a real move writes the whole new file, once.
    atomic_write(&path, half, &|from, to| std::fs::rename(from, to)).expect("the move went through");
    assert_eq!(std::fs::read_to_string(&path).expect("the new file"), half);
    let left: Vec<String> = std::fs::read_dir(&dir).expect("the folder").flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["Mine.MNU".to_string()], "and no staging file was left behind");
}

#[test]
fn a_file_the_reference_application_wrote_reads_as_a_set() {
    // Photoshop's own file: its root name, the byte-order mark its writer puts first, and two rows
    // for File › Close — a colour and a hide. An item PhotoCraft can colour but not hide (#14335)
    // still reads as hidden here, which is what the settings-file path does too:
    // `a_hand_edited_set_leaves_an_item_that_cannot_be_hidden_shown` pins that the menus show it.
    let text = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <photoshop-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/My Menus=My Menus\" modified=\"1\">\n{}{}{}\
         </photoshop-menu-settings>\n",
        row(31, 2, 0, 0, 5),
        row(31, 2, 0, 1, 0),
        row(1139, 41, 1, 1, 0),
    );
    let read = from_mnu(&text).expect("the reference application's file reads");
    assert_eq!(hidden_of(&read.set), vec!["file.close".to_string(), "layer.mergeVisible".to_string()]);
    assert_eq!(read.set.colors["file.close"], "blue", "both rows of File › Close came together");
    assert!(read.kept.is_empty(), "{read:?}");
}

#[test]
fn a_file_and_the_preferences_copy_of_a_name_make_one_set() {
    // The file speaks for the items its rows number; the preferences speak for the items it has no
    // row for. This is the engine half of the row's open(): a file read whole instead would lose
    // every change of a set whose items have no measured number.
    let file = set(&["file.new", "edit.fade"], &[("layer.mergeVisible", "gray")]);
    let prefs = set(&["edit.copy", "file.new"], &[("edit.paste", "red"), ("edit.fade", "blue")]);
    let merged = merged_with_prefs(&file, Some(&prefs));
    assert_eq!(
        hidden_of(&merged),
        vec!["edit.copy".to_string(), "edit.fade".to_string(), "file.new".to_string()],
        "Edit › Copy has no row so the preferences keep it; the file's rows decide File › New and Edit › Fade"
    );
    assert_eq!(merged.colors.get("edit.paste").map(String::as_str), Some("red"), "an item with no row keeps its colour");
    assert_eq!(merged.colors.get("layer.mergeVisible").map(String::as_str), Some("gray"), "a file row brings its own colour");
    assert!(!merged.colors.contains_key("edit.fade"), "the file's row for Edit › Fade carries no colour, so the preferences' blue does not survive it");
    // A row that shows an item again overrides the preferences' hide of it: the file's row is the
    // whole word on its own item, both attributes of it.
    let file = set(&[], &[("edit.fade", "red")]);
    let prefs = set(&["edit.fade", "edit.copy"], &[]);
    let merged = merged_with_prefs(&file, Some(&prefs));
    assert_eq!(hidden_of(&merged), vec!["edit.copy".to_string()], "the file shows Edit › Fade again: {merged:?}");
    assert_eq!(merged.colors.get("edit.fade").map(String::as_str), Some("red"));
    // The repro: a set whose every change is on unnumbered items writes a file that holds no row.
    let written = to_mnu("Mine", &set(&["edit.copy", "edit.paste"], &[]), &[]);
    assert_eq!(written.matches("<command").count(), 0, "no row for either item: {written}");
    let from_file = from_mnu(&written).expect("our own file reads");
    assert!(from_file.set.hidden.is_empty() && from_file.set.colors.is_empty(), "the file alone is an empty set: {from_file:?}");
    let merged = merged_with_prefs(&from_file.set, Some(&set(&["edit.copy", "edit.paste"], &[])));
    assert_eq!(hidden_of(&merged), vec!["edit.copy".to_string(), "edit.paste".to_string()], "with the preferences' copy both are there");
    // Where the preferences hold no copy, the file is the whole set, and a file that reads as nothing
    // gives nothing rather than guessing.
    let merged = merged_with_prefs(&set(&["file.open"], &[("edit.fade", "blue")]), None);
    assert_eq!(hidden_of(&merged), vec!["file.open".to_string()]);
    assert_eq!(merged.colors.get("edit.fade").map(String::as_str), Some("blue"));
    assert_eq!(merged_with_prefs(&set(&[], &[]), None), MenuSet::default(), "a file with no rows is an empty set");
}

#[test]
fn a_name_that_cannot_be_one_file_name_never_reaches_the_folder() {
    // A set name can come from a hand-edited preferences file, and `path_of` is the one place that
    // turns a name into a path. It refuses a name that would not name exactly one file inside the
    // folder: `.` and `..` are the folder and its parent, and a name ending in a dot is not the
    // name the folder ends up holding - `...` would become `...MNU`, whose stem is `..`, so the set
    // written under it could never be read back, rewritten or deleted under the name it was saved
    // under, and on a file system that drops trailing dots `Mine.` and `Mine` are one file.
    let (_tmp, dir) = folder_with(&[]);
    for name in [".", "..", "...", "....", "Mine.", "Mine..", "Web.", "x."] {
        assert!(path_of(&dir, name).is_err(), "`{name:?}` was taken as a file of the folder");
    }
    // The separators, the drive-letter forms and the empty names were refused before and stay so.
    for name in ["", "   ", "a/b", "a\\b", "C:", "C:\\windows", "a\u{7}b", "/etc/passwd"] {
        assert!(path_of(&dir, name).is_err(), "`{name:?}` was taken as a file of the folder");
    }
}

#[test]
fn a_name_that_works_names_one_file_and_names_the_set_back() {
    // The names the row and the Save dialog really produce still get their one file in the folder,
    // and the file names that set back (`set_name_of(path_of(name)) == name`).
    let (_tmp, dir) = folder_with(&[]);
    for name in ["Mine", "Dark Room", "Web.v2", "PhotoCraft Defaults Copy", "Menus (modified)", ".Hidden"] {
        let want = file_name(name);
        let path = path_of(&dir, name).expect("`{name}` names a set file");
        assert_eq!(path, dir.join(&want), "one file inside the folder, not a path through it");
        assert_eq!(path.file_name().and_then(|s| s.to_str()), Some(want.as_str()), "{name} is the whole file name");
        assert_eq!(set_name_of(&path).as_deref(), Some(name), "the file names the set back");
    }
    // The name is stored trimmed the way `set_name` trims it, so a trailing space is neither part of
    // the set's name nor part of the file's name.
    let path = path_of(&dir, "x ").expect("a trimmed name names a file");
    assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("x.MNU"), "the space went with the trim");
    assert_eq!(set_name_of(&path).as_deref(), Some("x"));
}

/// A set file is reported at the name the folder lists it under, which the set's own name does not
/// always reproduce: `set_name_of` takes the stem and `path_of` puts the measured extension back, so
/// on a case-sensitive file system `Darkroom.mnu` and `Darkroom.MNU` are two different files. The
/// dialog needs the path a set was found at to save it back into its own file (#14450).
#[test]
fn a_set_file_is_read_at_the_name_the_folder_lists_it_under() {
    let (_tmp, dir) = folder_with(&[("Darkroom.mnu", &mnu(&row(31, 2, 0, 0, 5)))]);
    let (files, errors) = read_sets(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].name, "Darkroom", "the stem names the set");
    assert_eq!(files[0].path, dir.join("Darkroom.mnu"), "the path is the file as the folder lists it");
    assert_eq!(files[0].set.colors["file.close"], "blue", "and the set its rows describe comes through");
    assert_ne!(files[0].path, path_of(&dir, "Darkroom").expect("`Darkroom` names a set file"), "which is not the file the set's name would name");
}

/// A case-sensitive folder can hold `x.mnu` and `x.MNU` side by side, and both name the set `x`.
/// The row keys sets by name, so only the first file (in sort order) is read and the pair is said
/// as an error: reading both would union two files into one set and keep only the later path, so
/// the earlier file could never be saved back to or deleted, and a Delete would resurrect the set
/// from it (#14450, round-3 nit). Windows cannot make this folder - its file system folds case, so
/// the two spellings are one file - hence the helper itself is tested here and the folder pair on a
/// case-sensitive runner below.
#[test]
fn the_dedupe_keeps_the_first_file_of_a_case_colliding_name_and_says_the_rest() {
    let dir = std::path::Path::new("menu-sets");
    let files = vec![
        ("Darkroom".to_string(), dir.join("Darkroom.MNU")),
        ("Darkroom".to_string(), dir.join("Darkroom.mnu")),
        ("darkroom".to_string(), dir.join("darkroom.MNU")),
        ("Mine".to_string(), dir.join("Mine.MNU")),
    ];
    let mut errors = Vec::new();
    let kept = first_per_set_name(files, &mut errors);
    assert_eq!(
        kept.iter().map(|(_, path)| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>(),
        vec!["Darkroom.MNU".to_string(), "Mine.MNU".to_string()],
        "one file per set name, the first in sort order"
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
    let first = &errors[0];
    assert!(first.contains("Darkroom.MNU") && first.contains("Darkroom.mnu"), "{first} names the pair");
    assert!(first.contains("name the same set"), "{first}");
    assert!(first.contains("only the first is used"), "{first}");
    assert!(errors[1].contains("Darkroom.MNU") && errors[1].contains("darkroom.MNU"), "{:?}", errors[1]);
}

/// The same rule end to end, on a folder that really holds both spellings. Windows refuses to make
/// the pair - `x.MNU` and `x.mnu` are one file there - so this runs on case-sensitive runners only.
#[cfg(not(windows))]
#[test]
fn two_case_colliding_files_in_one_folder_are_one_set_and_are_said() {
    let (_tmp, dir) = folder_with(&[("x.MNU", &mnu(&row(10, 2, 0, 1, 0))), ("x.mnu", &mnu(&row(31, 2, 0, 0, 5)))]);
    let (files, errors) = read_sets(&dir);
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["x"], "one set, from the first file in sort order");
    assert_eq!(files[0].path, dir.join("x.MNU"), "{:?}", files[0]);
    assert_eq!(hidden_of(&files[0].set), vec!["file.new".to_string()], "the first file's rows, not a union of both");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let first = &errors[0];
    assert!(first.contains("x.MNU") && first.contains("x.mnu"), "{first} names the pair");
    assert!(first.contains("name the same set"), "{first}");
}

/// The Windows guard the pair above cannot be: the file system itself folds the extension case, so
/// the menu-sets folder can never hold two spellings of one set name. This pins that practical
/// half honestly: writing `x.MNU` over the standing `x.mnu` rewrites one file, and the folder
/// reads back as one set with nothing said.
#[cfg(windows)]
#[test]
fn a_case_insensitive_folder_cannot_hold_two_spellings_of_one_set_name() {
    let (_tmp, dir) = folder_with(&[("x.mnu", &mnu(&row(10, 2, 0, 1, 0)))]);
    std::fs::write(dir.join("x.MNU"), mnu(&row(31, 2, 0, 0, 5))).expect("the second spelling is the same file");
    let left: Vec<String> = std::fs::read_dir(&dir).expect("the folder").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["x.mnu".to_string()], "one file, under the name it was made with: {left:?}");
    let (files, errors) = read_sets(&dir);
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["x"], "one set");
    assert!(errors.is_empty(), "{errors:?}");
}
