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

/// Caption-only edits infer just one physical row, while explicit null, duplicate captions,
/// and captions shared with legacy sets retain the existing name-only behavior.
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

/// Repeated factory actions keep their display names. A name-only shortcut is removed
/// when that name ceases to identify a physical row, and Undo restores the binding.
#[test]
fn repeated_reset_pins_existing_shortcuts_and_undo_restores_legacy_keys() {
    let mut s = Session::new();
    s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap();
    let shortcut = format!("{PLAY_SHORTCUT_PREFIX}Invert Colors");
    s.edit_prefs(|prefs| {
        prefs.shortcuts.insert(shortcut.clone(), "F2".into());
        prefs.alternate_shortcuts.insert(shortcut.clone(), vec!["F3".into()]);
    });
    let before = s.actions.list.clone();
    let prefs = s.prefs().clone();
    assert!(s.execute("actions.reset", json!({"confirm":false,"mode":"append"})).is_err());
    assert_eq!(s.actions.list, before);
    assert_eq!(s.prefs().shortcuts, prefs.shortcuts);
    assert_eq!(s.prefs().alternate_shortcuts, prefs.alternate_shortcuts);

    s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap();
    let key = crate::actions_cmds::occurrence_key(s.actions.list[0].action_id.as_deref().unwrap());
    assert_eq!(crate::actions_cmds::shortcut_row(&s.actions.list, &key), Some(0));
    assert_eq!(s.prefs().shortcuts[&key], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&key], ["F3"]);
    assert!(!s.prefs().shortcuts.contains_key(&shortcut));
    assert!(!s.prefs().alternate_shortcuts.contains_key(&shortcut));
    assert_eq!(s.actions.list.iter().filter(|action| action.name == "Invert Colors").count(), 2);

    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions.list, before);
    assert_eq!(s.prefs().shortcuts[&shortcut], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&shortcut], ["F3"]);
    assert!(!s.prefs().shortcuts.contains_key(&key));
}

#[test]
fn duplicate_stored_ids_stay_unplayable_after_deleting_one_row() {
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    let store = MemBackend::default();
    let id = "0".repeat(32);
    let file = json!({"version":1,"actions":[{"name":"First","action_id":id},{"name":"Second","action_id":id}]});
    store.write(ACTIONS_FILE, file.to_string().as_bytes()).unwrap();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(store)));
    let key = crate::actions_cmds::occurrence_key(&id);
    assert_eq!(crate::actions_cmds::shortcut_row(&s.actions.list, &key), None);
    s.execute("actions.delete", json!({"action":"First"})).unwrap();
    assert_eq!(crate::actions_cmds::shortcut_row(&s.actions.list, &key), None);
    assert!(s.execute("actions.play", json!({"actionId":id,"action":"Second"})).is_err());
    assert!(s.execute("actions.play", json!({"actionId":"invalid","action":"Second"})).is_err());
    assert_eq!(s.execute("actions.play", json!({"action":"Second"})).unwrap()["ran"], 0);
}

#[test]
fn atn_export_and_load_keep_function_keys_of_duplicate_rows_separate() {
    let mut s = Session::new();
    for _ in 0..2 {
        s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap();
    }
    for (row, key) in [(0, "F2"), (2, "F3")] {
        let id = crate::actions_cmds::prepare_action_id(&s.actions.list, row).unwrap();
        s.edit_prefs(|prefs| prefs.shortcuts.insert(crate::actions_cmds::occurrence_key(&id), key.into()));
        crate::actions_cmds::commit_action_id(&mut s, row, id);
    }
    let archive = s.execute("actions.save", json!({"set":"PhotoCraft Basics","skipUnsupported":true})).unwrap();
    let mut loaded = Session::new();
    let result = loaded.execute("actions.load", json!({"data":archive["data"]})).unwrap();
    assert_eq!(result["functionKeys"], json!([{"action":"Invert Colors","key":"F2"},{"action":"Invert Colors 2","key":"F3"}]));
    assert_eq!(crate::actions_cmds::action_shortcut(&loaded, 0), Some("F2"));
    assert_eq!(crate::actions_cmds::action_shortcut(&loaded, 2), Some("F3"));
}

#[test]
fn attachment_keeps_disk_batch_targets_and_does_not_resolve_foreign_ids() {
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    let store = MemBackend::default();
    let foreign_id = "a".repeat(32);
    let file = json!({"version":1,"actions":[
        {"name":"Target","set":"Saved"},
        {"name":"Caller","steps":[
            ["file.automate.batch",{"action":"Target","set":"Saved"}],
            ["actions.play",{"action":0}],
            ["actions.play",{"action":"Target","actionId":foreign_id}]
        ]}
    ]});
    store.write(ACTIONS_FILE, file.to_string().as_bytes()).unwrap();
    let mut s = Session::new();
    s.actions.list.push(Action { set: Some("Saved".into()), action_id: Some(foreign_id), ..Action::new("Target", Vec::new()) });
    assert!(s.attach_preset_store(open(Box::new(store))).is_empty());
    let list = &s.actions.list;
    let calls = &list[2].steps;
    assert_eq!(crate::file_cmds::batch_action(list, &calls[0].1).unwrap(), 1);
    assert_eq!(crate::actions_cmds::resolve_play_target(list, &calls[1].1, "actions.play").unwrap(), 1);
    assert!(crate::actions_cmds::resolve_play_target(list, &calls[2].1, "actions.play").is_err());
    assert_eq!(crate::actions_cmds::resolve(list, &json!({"action":"Target"}), "actions.play").unwrap(), 0);
}

/// Imported calls follow a local physical folder before falling back to the first archive name.
#[test]
fn whole_replace_resolves_unqualified_calls_in_their_own_occurrence() {
    let mut s = Session::new();
    let mut archive = set("Repeated", Some(("Child", 0)));
    let child = Action::new("Child", Vec::new());
    let parent = Action::new("Parent", vec![("actions.play".into(), json!({"action":"Child"}))]);
    let (second, _) = crate::atn_cmds::export("Repeated", &[&child, &parent], &[], false).unwrap();
    archive.extend_from_slice(&second[4..]);
    s.execute("actions.replace", json!({"confirm":true,"data":photocraft_paint::tile::b64_encode(&archive)})).unwrap();
    assert_eq!(s.actions.list[0].name, "Child");
    assert_ne!(s.actions.list[1].name, "Child");
    assert_eq!(s.actions.list[2].steps[0].1["action"], s.actions.list[1].name.as_str());
}
