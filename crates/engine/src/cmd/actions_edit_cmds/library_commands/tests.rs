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

#[test]
fn duplicate_set_distinguishes_implicit_caption_and_empty_import() {
    let mut s = Session::new();
    let mut archive = set("Same", Some(("Physical", 0)));
    archive.extend_from_slice(&set("Empty", None)[4..]);
    s.execute("actions.replace", json!({"confirm":true,"data":photocraft_paint::tile::b64_encode(&archive)})).unwrap();
    s.execute("actions.record", json!({"name":"Implicit","set":"Same","setSlot":null})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();

    let copy = s.execute("actions.duplicate", json!({"set":"Same","setSlot":null})).unwrap();
    assert_eq!(copy["actions"], 1);
    assert_eq!(s.actions.list[copy["index"].as_u64().unwrap() as usize].name, "Implicit copy");
    assert_eq!(s.execute("actions.duplicate", json!({"set":"Same"})).unwrap()["actions"], 2);
    assert_eq!(s.execute("actions.duplicate", json!({"set":"Empty"})).unwrap(), json!({"set":"Empty copy","actions":0}));
    assert_eq!(s.actions.sets, ["Empty copy"]);
    assert_eq!(s.actions.set_rows, ["Same", "Empty"]);
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
    assert_eq!(s.actions.list[0].set.as_deref(), Some("Exact "));
    assert_eq!(s.actions.list[0].set_slot, None);
    s.execute("actions.assignSet", json!({"action":0,"set":"Exact "})).unwrap();
    assert_eq!(s.actions.list[0].set.as_deref(), Some("Exact "));
    assert_eq!(s.actions.list[0].set_slot, None);

    s.actions.set_rows = ["X ", "X ", "X"].map(str::to_owned).to_vec();
    s.execute("actions.assignSet", json!({"action":0,"set":"X "})).unwrap();
    assert_eq!(s.actions.list[0].set.as_deref(), Some("X "));
    assert_eq!(s.actions.list[0].set_slot, None);
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
    assert!(s.actions.list[0].legacy_shortcut_owner);
    assert_eq!(crate::actions_cmds::shortcut_row(&s.actions.list, &shortcut), Some(0));
    assert_eq!(s.prefs().shortcuts[&shortcut], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&shortcut], ["F3"]);
    assert_eq!(s.actions.list.iter().filter(|action| action.name == "Invert Colors").count(), 2);

    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions.list, before);
    assert_eq!(s.prefs().shortcuts[&shortcut], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&shortcut], ["F3"]);
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
        crate::actions_cmds::commit_action_id(&mut s, row, id).unwrap();
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

#[test]
fn attachment_keeps_unresolved_disk_calls_from_binding_to_early_actions() {
    use crate::actions_cmds::{conditional_branch, resolve_play_target};
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    let store = MemBackend::default();
    let file = json!({"version":1,"actions":[{"name":"Disk caller","steps":[
        ["actions.play",{"action":"Early"}],
        ["actions.conditional",{"if":"always","then":"Early"}],
        ["file.automate.batch",{"action":"Early","set":"Session"}],
        ["actions.play",{"action":1}]
    ]}]});
    store.write(ACTIONS_FILE, file.to_string().as_bytes()).unwrap();
    let mut s = Session::new();
    s.actions.list.push(Action { set: Some("Session".into()), ..Action::new("Early", Vec::new()) });
    assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
    s.sync_preset_store();

    let mut s = Session::new();
    assert!(s.attach_preset_store(open(Box::new(store))).is_empty());
    let list = &s.actions.list;
    let steps = &list[1].steps;
    assert!(resolve_play_target(list, &steps[0].1, "actions.play").is_err());
    let branch = conditional_branch(&steps[1].1, "then").unwrap().unwrap();
    assert!(resolve_play_target(list, &branch, "actions.conditional").is_err());
    assert!(crate::file_cmds::batch_action(list, &steps[2].1).is_err());
    assert!(resolve_play_target(list, &steps[3].1, "actions.play").is_err());
    assert_eq!(s.execute("actions.play", json!({"action":"Early"})).unwrap()["action"], "Early");
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

#[test]
fn shortcut_only_clear_restores_orphan_bindings_on_undo() {
    let mut s = Session::new();
    let orphan = "actions.play:Missing";
    s.edit_prefs(|prefs| {
        prefs.shortcuts.insert(orphan.into(), "F2".into());
        prefs.alternate_shortcuts.insert(orphan.into(), vec!["F3".into()]);
        prefs.alternate_shortcuts.insert("file.save".into(), vec!["F4".into()]);
    });

    assert_eq!(s.execute("actions.clearAll", json!({"confirm":true})).unwrap()["cleared"], 0);
    assert!(!s.prefs().shortcuts.contains_key(orphan));
    assert!(!s.prefs().alternate_shortcuts.contains_key(orphan));
    assert_eq!(s.prefs().alternate_shortcuts["file.save"], ["F4"]);
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert!(s.actions.list.is_empty());
    assert_eq!(s.prefs().shortcuts[orphan], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[orphan], ["F3"]);
}

#[test]
fn pre_attachment_replacement_persists_without_reloading_disk_and_keeps_undo() {
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    for undo_before_attachment in [false, true] {
        let store = MemBackend::default();
        let disk = json!({"version":1,"set_rows":["Saved"],"actions":[{"name":"Disk","set":"Saved","set_slot":0}]});
        store.write(ACTIONS_FILE, disk.to_string().as_bytes()).unwrap();
        let mut s = Session::new();
        s.actions.list.push(Action::new("Early", Vec::new()));
        s.execute("actions.reset", json!({"confirm":true,"mode":"replace"})).unwrap();
        if undo_before_attachment {
            s.execute("actions.undoLibrary", json!({})).unwrap();
        }
        assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
        if undo_before_attachment {
            assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Early", "Disk"]);
            assert_eq!(s.actions.set_rows, ["Saved"]);
            continue;
        }

        assert_eq!(s.actions.set_rows, ["PhotoCraft Basics"]);
        assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Invert Colors", "Desaturate"]);
        let mut reopened = Session::new();
        assert!(reopened.attach_preset_store(open(Box::new(store.clone()))).is_empty());
        assert_eq!(reopened.actions.list, s.actions.list);
        s.execute("actions.undoLibrary", json!({})).unwrap();
        assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Early"]);
        assert!(s.actions.set_rows.is_empty());
        let mut reopened = Session::new();
        assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
        assert_eq!(reopened.actions.list, s.actions.list);
    }
}

#[test]
fn pre_attachment_clear_remains_authoritative_after_an_ordinary_edit() {
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    let store = MemBackend::default();
    store.write(ACTIONS_FILE, br#"{"version":1,"actions":[{"name":"Disk"}]}"#).unwrap();
    let mut s = Session::new();
    s.actions.list.push(Action::new("Early", Vec::new()));
    s.execute("actions.clearAll", json!({"confirm":true})).unwrap();
    s.execute("actions.newSet", json!({"name":"New"})).unwrap();
    assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
    assert!(s.actions.list.is_empty());
    assert_eq!(s.actions.sets, ["New"]);
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
    assert!(reopened.actions.list.is_empty());
    assert_eq!(reopened.actions.sets, ["New"]);
}

#[test]
fn attachment_does_not_fill_an_unresolved_early_numeric_call() {
    use crate::actions_cmds::resolve_play_target;
    use crate::preset_store::{ACTIONS_FILE, PresetBackend};

    let store = MemBackend::default();
    store.write(ACTIONS_FILE, br#"{"version":1,"actions":[{"name":"Disk"}]}"#).unwrap();
    let mut s = Session::new();
    s.actions.list.push(Action::new("Early", Vec::new()));
    s.actions.list.push(Action::new(
        "Caller",
        vec![
            ("actions.play".into(), json!({"action":2})),
            ("file.automate.batch".into(), json!({"action":2})),
            ("actions.play".into(), json!({"action":0})),
            ("actions.play".into(), json!({"action":"Disk"})),
        ],
    ));
    assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
    for library in [&s.actions.list, &reopened.actions.list] {
        assert_eq!(library[2].name, "Disk");
        let calls = &library[1].steps;
        assert!(resolve_play_target(library, &calls[0].1, "actions.play").is_err());
        assert!(crate::file_cmds::batch_action(library, &calls[1].1).is_err());
        assert_eq!(resolve_play_target(library, &calls[2].1, "actions.play").unwrap(), 0);
        assert_eq!(resolve_play_target(library, &calls[3].1, "actions.play").unwrap(), 2);
    }
}

#[test]
fn whole_replace_keeps_diagnostics_and_calls_on_their_archive_targets() {
    use crate::actions_cmds::resolve_play_target;

    let first = Action::new("Target", Vec::new());
    let second = Action::new(
        "Target",
        vec![(crate::atn_cmds::PHOTOSHOP_STEP.into(), json!({"event":"unknownReplacementEvent","eventKind":"name","name":"Unsupported"}))],
    );
    let caller = Action::new(
        "Caller",
        vec![
            ("file.automate.createDroplet".into(), json!({"path":"/d/S.pcdroplet","output":"/d/out","action":"Target"})),
            ("file.automate.createDroplet".into(), json!({"path":"/d/S.pcdroplet","output":"/d/out","action":"Target","set":"Second"})),
            ("file.automate.createDroplet".into(), json!({"path":"/d/S.pcdroplet","output":"/d/out","action":"Target 2","set":"Second"})),
            ("actions.play".into(), json!({"action":"Target 2"})),
            ("actions.play".into(), json!({"action":"Target"})),
        ],
    );
    let mut archive = set("First", Some(("Target", 0)));
    let (tail, _) = crate::atn_cmds::export("Second", &[&first, &second, &caller], &[], false).unwrap();
    archive.extend_from_slice(&tail[4..]);
    let mut s = Session::new();
    let result = s.execute("actions.replace", json!({"confirm":true,"data":photocraft_paint::tile::b64_encode(&archive)})).unwrap();
    assert_eq!(result["actions"], json!(["Target", "Target 2", "Target 3", "Caller"]));
    assert_eq!(result["unsupported"].as_array().unwrap().len(), 1);
    let diagnostic = &result["unsupported"][0];
    assert_eq!(diagnostic["action"], "Target 3");
    assert_eq!(diagnostic["step"], 0);
    assert_eq!(diagnostic["event"], "unknownReplacementEvent");

    let list = &s.actions.list;
    let calls = &list[3].steps;
    for (_, params) in &calls[..2] {
        assert_eq!(params["action"], "Target 2");
        assert_eq!(crate::file_cmds::batch_action(list, params).unwrap(), 1);
    }
    assert_eq!(calls[2].1["action"], "Target 2");
    assert_eq!(calls[2].1.get("actionId"), Some(&serde_json::Value::Null));
    assert!(crate::automate_cmds::named_action_steps(&s, &calls[2].1, "file.automate.createDroplet").is_err());
    assert!(resolve_play_target(list, &calls[3].1, "actions.play").is_err());
    assert_eq!(resolve_play_target(list, &calls[4].1, "actions.play").unwrap(), 1);
}
