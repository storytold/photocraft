use crate::Session;
use crate::actions_cmds::{Action, PLAY_SHORTCUT_PREFIX};
use crate::preset_store::{MemBackend, open};
use photocraft_psd::atn::{AtnAction, AtnFile, write};
use photocraft_psd::descriptor::UnicodeString;
use serde_json::json;

#[test]
fn confirmed_clear_restores_both_shortcut_maps_without_document_history() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    s.actions.list.push(Action::new("Original", Vec::new()));
    s.actions.sets.push("Empty".into());
    let shortcut = format!("{PLAY_SHORTCUT_PREFIX}Original");
    s.edit_prefs(|prefs| {
        prefs.shortcuts.insert(shortcut.clone(), "F2".into());
        prefs.alternate_shortcuts.insert(shortcut.clone(), vec!["F3".into()]);
        prefs.shortcuts.insert("file.save".into(), "Ctrl+S".into());
    });
    let revision = s.actions.rev;
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("actions.clearAll", json!({"confirm":false})).is_err());
    assert_eq!(s.actions.rev, revision);
    s.execute("actions.record", json!({"action":0})).unwrap();
    assert!(s.execute("actions.clearAll", json!({"confirm":true})).is_err());
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.execute("actions.clearAll", json!({"confirm":true})).unwrap()["cleared"], 1);
    assert!(s.actions.list.is_empty() && s.actions.sets.is_empty());
    assert!(!s.prefs().shortcuts.contains_key(&shortcut));
    assert!(!s.prefs().alternate_shortcuts.contains_key(&shortcut));
    assert_eq!(s.prefs().shortcuts["file.save"], "Ctrl+S");
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions.list[0].name, "Original");
    assert_eq!(s.actions.sets, ["Empty"]);
    assert_eq!(s.prefs().shortcuts[&shortcut], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&shortcut], ["F3"]);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
}

#[test]
fn reset_appends_separate_same_caption_rows_and_persists_them() {
    let store = MemBackend::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(store.clone())));
    s.actions.sets.push("Existing".into());
    assert!(s.execute("actions.reset", json!({"confirm":true,"mode":"bad"})).is_err());
    for slot in 0..2 {
        assert_eq!(s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap()["slot"], slot);
    }
    assert_eq!(s.actions.set_rows, ["PhotoCraft Basics", "PhotoCraft Basics"]);
    assert_eq!(s.actions.list.iter().map(|a| a.set_slot).collect::<Vec<_>>(), [Some(0), Some(0), Some(1), Some(1)]);
    assert_eq!(s.actions.list[0].steps[0].0, "image.adjustments.invert");
    assert_eq!(s.actions.list[1].steps[0].0, "image.adjustments.desaturate");
    s.sync_preset_store();
    let mut reopened = Session::new();
    reopened.attach_preset_store(open(Box::new(store)));
    assert_eq!(reopened.actions.list, s.actions.list);
    assert_eq!(reopened.actions.set_rows, s.actions.set_rows);
    assert_eq!(reopened.actions.sets, ["Existing"]);
    s.execute("actions.reset", json!({"confirm":true,"mode":"replace"})).unwrap();
    assert_eq!(s.actions.set_rows, ["PhotoCraft Basics"]);
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions.set_rows, reopened.actions.set_rows);
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
}

fn set(caption: &str, action: Option<(&str, u16)>) -> Vec<u8> {
    write(&AtnFile {
        name: UnicodeString::new_nul(caption),
        actions: action.into_iter().map(|(name, key)| AtnAction { name: UnicodeString::new_nul(name), function_key: key, ..Default::default() }).collect(),
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn whole_replace_validates_the_tail_and_keeps_every_set_in_order() {
    let mut s = Session::new();
    s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap();
    let prior = s.actions.list.clone();
    let mut archive = set("Repeated", Some(("Action", 2)));
    archive.extend_from_slice(&set("Empty", None)[4..]);
    archive.extend_from_slice(&set("Repeated", Some(("Action", 3)))[4..]);
    let b64 = photocraft_paint::tile::b64_encode;
    let mut bad = archive.clone();
    bad.pop();
    assert!(s.execute("actions.replace", json!({"confirm":true,"data":b64(&bad)})).is_err());
    assert!(s.execute("actions.replace", json!({"data":b64(&archive)})).is_err());
    assert_eq!(s.actions.list, prior);
    s.execute("actions.replace", json!({"confirm":true,"data":b64(&archive)})).unwrap();
    assert_eq!(s.actions.set_rows, ["Repeated", "Empty", "Repeated"]);
    assert_eq!(s.actions.list[0].set_slot, Some(0));
    assert_eq!(s.actions.list[1].set_slot, Some(2));
    assert_ne!(s.actions.list[0].name, s.actions.list[1].name);
    assert_eq!(s.prefs().shortcuts[&format!("{PLAY_SHORTCUT_PREFIX}{}", s.actions.list[1].name)], "F3");
    let saved = s.execute("actions.save", json!({"set":"Repeated","setSlot":2})).unwrap();
    assert_eq!(saved["actions"].as_array().unwrap().len(), 1);
    let empty = s.execute("actions.save", json!({"set":"Empty","setSlot":1})).unwrap();
    assert!(empty["actions"].as_array().unwrap().is_empty());
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions.list, prior);
}

/// Caption-only edits infer an explicit folder only when the caption is unambiguous.
#[test]
fn recording_and_assignment_infer_only_unambiguous_folder_occurrences() {
    let mut s = Session::new();
    s.actions.set_rows = vec!["Exact ".into(), "Shared".into(), "Shared".into(), "Legacy".into()];
    s.actions.sets.push("Legacy".into());
    s.execute("actions.record", json!({"name":"Recorded","set":"Exact "})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].set_slot, Some(0));
    for set in ["Shared", "Legacy"] {
        s.execute("actions.assignSet", json!({"action":0,"set":set})).unwrap();
        assert_eq!(s.actions.list[0].set_slot, None);
    }
    s.execute("actions.assignSet", json!({"action":0,"set":"Exact ","setSlot":null})).unwrap();
    assert_eq!(s.actions.list[0].set_slot, None);
    s.execute("actions.assignSet", json!({"action":0,"set":"Exact "})).unwrap();
    assert_eq!(s.actions.list[0].set_slot, Some(0));
}
