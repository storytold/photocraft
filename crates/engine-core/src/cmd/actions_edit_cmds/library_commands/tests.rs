use crate::Session;
use crate::actions_cmds::ActionsExt;
use crate::actions_cmds::{Action, ActionStep, PLAY_SHORTCUT_PREFIX};
use crate::preset_store::PresetStoreExt;
use crate::preset_store::{ACTIONS_FILE, MemBackend, PresetBackend, open};
use craft_psd::atn::{AtnAction, AtnFile, write};
use craft_psd::descriptor::UnicodeString;
use serde_json::json;

fn names(s: &Session) -> Vec<&str> {
    s.actions().list.actions().iter().map(|action| action.name.as_str()).collect()
}

fn b64(bytes: &[u8]) -> String {
    photocraft_paint::tile::b64_encode(bytes)
}

/// One ATN set holding `actions` (name, function key; 0 is none).
fn set_of(caption: &str, actions: &[(&str, u16)]) -> Vec<u8> {
    write(&AtnFile {
        name: UnicodeString::new_nul(caption),
        actions: actions.iter().map(|(name, key)| AtnAction { name: UnicodeString::new_nul(name), function_key: *key, ..Default::default() }).collect(),
        ..Default::default()
    })
    .unwrap()
}

fn set(caption: &str, action: Option<(&str, u16)>) -> Vec<u8> {
    set_of(caption, action.as_slice())
}

#[test]
fn confirmed_clear_restores_both_shortcut_maps_without_document_history() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    s.actions_mut().list.push(Action::new("Original", Vec::new())).unwrap();
    s.actions_mut().list.new_set("Empty").unwrap();
    let shortcut = format!("{PLAY_SHORTCUT_PREFIX}Original");
    s.edit_prefs(|prefs| {
        prefs.shortcuts.insert(shortcut.clone(), "F2".into());
        prefs.alternate_shortcuts.insert(shortcut.clone(), vec!["F3".into()]);
        prefs.shortcuts.insert("file.save".into(), "Ctrl+S".into());
    });
    let revision = s.actions().list.revision();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("actions.clearAll", json!({"confirm":false})).is_err());
    assert_eq!(s.actions().list.revision(), revision);
    s.execute("actions.record", json!({"action":0})).unwrap();
    assert!(s.execute("actions.clearAll", json!({"confirm":true})).is_err());
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.execute("actions.clearAll", json!({"confirm":true})).unwrap()["cleared"], 1);
    assert!(s.actions().list.actions().is_empty() && s.actions().list.sets().is_empty());
    assert!(!s.prefs().shortcuts.contains_key(&shortcut));
    assert!(!s.prefs().alternate_shortcuts.contains_key(&shortcut));
    assert_eq!(s.prefs().shortcuts["file.save"], "Ctrl+S");
    assert_eq!(crate::actions_edit_cmds::undo_library_label(&s), Some("Undo Clear All Actions"));
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(names(&s), ["Original"]);
    assert_eq!(s.actions().list.sets(), ["Empty"]);
    assert_eq!(s.prefs().shortcuts[&shortcut], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[&shortcut], ["F3"]);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
}

// An edit after a library command ends its undo; so does a change to the action shortcuts.
#[test]
fn any_later_edit_ends_the_library_undo() {
    let mut s = crate::session();
    s.execute("actions.reset", json!({"confirm":true,"mode":"replace"})).unwrap();
    s.execute("actions.newSet", json!({"name":"Later"})).unwrap();
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
    s.execute("actions.clearAll", json!({"confirm":true})).unwrap();
    s.edit_prefs(|prefs| prefs.shortcuts.insert(format!("{PLAY_SHORTCUT_PREFIX}Gone"), "F4".into()));
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
}

#[test]
fn reset_appends_a_numbered_set_and_persists_it() {
    let store = MemBackend::default();
    let mut s = crate::session();
    s.attach_preset_store(open(Box::new(store.clone())));
    s.actions_mut().list.new_set("Existing").unwrap();
    assert!(s.execute("actions.reset", json!({"confirm":true,"mode":"bad"})).is_err());
    assert_eq!(s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap()["set"], "PhotoCraft Basics");
    assert_eq!(s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap()["set"], "PhotoCraft Basics 2");
    assert_eq!(s.actions().list.sets(), ["Existing", "PhotoCraft Basics", "PhotoCraft Basics 2"], "sets in the order they were made");
    assert_eq!(names(&s), ["Invert Colors", "Desaturate", "Invert Colors 2", "Desaturate 2"]);
    assert_eq!(s.actions().list.actions()[0].steps[0].command, "image.adjustments.invert");
    assert_eq!(s.actions().list.actions()[1].steps[0].command, "image.adjustments.desaturate");
    s.sync_preset_store();
    let mut reopened = crate::session();
    reopened.attach_preset_store(open(Box::new(store)));
    assert_eq!(reopened.actions().list.actions(), s.actions().list.actions());
    assert_eq!(reopened.actions().list.sets(), s.actions().list.sets());
    s.execute("actions.reset", json!({"confirm":true,"mode":"replace"})).unwrap();
    assert_eq!(s.actions().list.sets(), ["PhotoCraft Basics"]);
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions().list.sets(), reopened.actions().list.sets());
    assert!(s.execute("actions.undoLibrary", json!({})).is_err());
}

#[test]
fn whole_replace_validates_the_tail_and_keeps_every_set_in_order() {
    let mut s = crate::session();
    s.execute("actions.reset", json!({"confirm":true,"mode":"append"})).unwrap();
    let prior = s.actions().list.actions().to_vec();
    let mut archive = set("Repeated", Some(("Action", 2)));
    archive.extend_from_slice(&set("Empty", None)[4..]);
    archive.extend_from_slice(&set("Repeated", Some(("Action", 3)))[4..]);
    let mut bad = archive.clone();
    bad.pop();
    assert!(s.execute("actions.replace", json!({"confirm":true,"data":b64(&bad)})).is_err());
    assert!(s.execute("actions.replace", json!({"data":b64(&archive)})).is_err());
    assert_eq!(s.actions().list.actions(), prior);
    let result = s.execute("actions.replace", json!({"confirm":true,"data":b64(&archive)})).unwrap();
    assert_eq!(result["sets"], json!(["Repeated", "Empty", "Repeated 2"]));
    assert_eq!(s.actions().list.sets(), ["Repeated", "Repeated 2", "Empty"]);
    assert_eq!(names(&s), ["Action", "Action 2"]);
    assert_eq!(s.prefs().shortcuts[&format!("{PLAY_SHORTCUT_PREFIX}Action")], "F2");
    assert_eq!(s.prefs().shortcuts[&format!("{PLAY_SHORTCUT_PREFIX}Action 2")], "F3");
    let saved = s.execute("actions.save", json!({"set":"Repeated 2"})).unwrap();
    assert_eq!(saved["actions"], json!(["Action 2"]));
    let empty = s.execute("actions.save", json!({"set":"Empty"})).unwrap();
    assert!(empty["actions"].as_array().unwrap().is_empty());
    assert_eq!(crate::actions_edit_cmds::undo_library_label(&s), Some("Undo Replace Actions"));
    s.execute("actions.undoLibrary", json!({})).unwrap();
    assert_eq!(s.actions().list.actions(), prior);
}

// Two actions one set gives the same name keep their own function keys.
#[test]
fn atn_load_keeps_function_keys_of_repeated_names_separate() {
    let mut s = crate::session();
    let result = s.execute("actions.load", json!({"data":b64(&set_of("Keys", &[("Same", 2), ("Same", 3)]))})).unwrap();
    assert_eq!(result["functionKeys"], json!([{"action":"Same","key":"F2"},{"action":"Same 2","key":"F3"}]));
    assert_eq!(s.prefs().shortcuts[&format!("{PLAY_SHORTCUT_PREFIX}Same")], "F2");
    assert_eq!(s.prefs().shortcuts[&format!("{PLAY_SHORTCUT_PREFIX}Same 2")], "F3");
}

/// Imported calls look in their own set before falling back to the first archive name.
#[test]
fn whole_replace_resolves_unqualified_calls_in_their_own_set() {
    let mut s = crate::session();
    let mut archive = set("Repeated", Some(("Child", 0)));
    let child = Action::new("Child", Vec::new());
    let parent = Action::new("Parent", vec![ActionStep::new("actions.play", json!({"action":"Child"}))]);
    let (second, _) = crate::atn_cmds::export("Repeated", &[&child, &parent], &[], false).unwrap();
    archive.extend_from_slice(&second[4..]);
    s.execute("actions.replace", json!({"confirm":true,"data":b64(&archive)})).unwrap();
    assert_eq!(names(&s), ["Child", "Child 2", "Parent"]);
    assert_eq!(s.actions().list.actions()[2].steps[0].params["action"], "Child 2");
}

// A call that names its action's set reaches that set's action, renamed or not, and names the set
// as the list has it.
#[test]
fn whole_replace_follows_explicit_atn_set_scopes() {
    let second = Action::new("Target", Vec::new());
    let caller = Action::new(
        "Caller",
        vec![
            ActionStep::new("file.automate.createDroplet", json!({"path":"/d/S.pcdroplet","output":"/d/out","action":"Target","set":"Second"})),
            ActionStep::new("actions.conditional", json!({"if":"always","then":"Target","thenSet":"Second","else":"Target","elseSet":"First"})),
            ActionStep::new("actions.play", json!({"action":"Target"})),
        ],
    );
    let mut archive = set("First", Some(("Target", 0)));
    let (tail, _) = crate::atn_cmds::export("Second", &[&second, &caller], &[], false).unwrap();
    archive.extend_from_slice(&tail[4..]);
    let mut s = crate::session();
    let result = s.execute("actions.replace", json!({"confirm":true,"data":b64(&archive)})).unwrap();
    assert_eq!(result["actions"], json!(["Target", "Target 2", "Caller"]));
    let list = s.actions().list.actions();
    let calls: Vec<&serde_json::Value> = list[2].steps.iter().map(|step| &step.params).collect();
    assert_eq!((calls[0]["action"].as_str(), calls[0]["set"].as_str()), (Some("Target 2"), Some("Second")));
    assert_eq!(crate::file_cmds::batch_action(list, calls[0]).unwrap(), 1);
    assert_eq!((calls[1]["then"].as_str(), calls[1]["else"].as_str()), (Some("Target 2"), Some("Target")));
    assert_eq!(calls[2]["action"], "Target 2");
}

#[test]
fn whole_replace_keeps_diagnostics_on_their_archive_actions() {
    let first = Action::new("Target", Vec::new());
    let second = Action::new(
        "Target",
        vec![ActionStep::new(crate::atn_cmds::PHOTOSHOP_STEP, json!({"event":"unknownReplacementEvent","eventKind":"name","name":"Unsupported"}))],
    );
    let mut archive = set("First", Some(("Target", 0)));
    let (tail, _) = crate::atn_cmds::export("Second", &[&first, &second], &[], false).unwrap();
    archive.extend_from_slice(&tail[4..]);
    let mut s = crate::session();
    let result = s.execute("actions.replace", json!({"confirm":true,"data":b64(&archive)})).unwrap();
    assert_eq!(result["actions"], json!(["Target", "Target 2", "Target 3"]));
    assert_eq!(result["unsupported"].as_array().unwrap().len(), 1);
    let diagnostic = &result["unsupported"][0];
    assert_eq!(diagnostic["action"], "Target 3");
    assert_eq!(diagnostic["step"], 0);
    assert_eq!(diagnostic["event"], "unknownReplacementEvent");
}

#[test]
fn shortcut_only_clear_restores_orphan_bindings_on_undo() {
    let mut s = crate::session();
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
    assert!(s.actions().list.actions().is_empty());
    assert_eq!(s.prefs().shortcuts[orphan], "F2");
    assert_eq!(s.prefs().alternate_shortcuts[orphan], ["F3"]);
}

#[test]
fn pre_attachment_replacement_persists_without_reloading_disk_and_keeps_undo() {
    for undo_before_attachment in [false, true] {
        let store = MemBackend::default();
        store.write(ACTIONS_FILE, br#"{"version":1,"actions":[{"name":"Disk","set":"Saved"}]}"#).unwrap();
        let mut s = crate::session();
        s.actions_mut().list.push(Action::new("Early", Vec::new())).unwrap();
        s.execute("actions.reset", json!({"confirm":true,"mode":"replace"})).unwrap();
        if undo_before_attachment {
            s.execute("actions.undoLibrary", json!({})).unwrap();
        }
        assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
        if undo_before_attachment {
            assert_eq!(names(&s), ["Early", "Disk"]);
            continue;
        }

        assert_eq!(s.actions().list.sets(), ["PhotoCraft Basics"]);
        assert_eq!(names(&s), ["Invert Colors", "Desaturate"]);
        let mut reopened = crate::session();
        assert!(reopened.attach_preset_store(open(Box::new(store.clone()))).is_empty());
        assert_eq!(reopened.actions().list.actions(), s.actions().list.actions());
        s.execute("actions.undoLibrary", json!({})).unwrap();
        assert_eq!(names(&s), ["Early"]);
        s.sync_preset_store();
        let mut reopened = crate::session();
        assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
        assert_eq!(reopened.actions().list.actions(), s.actions().list.actions());
    }
}

#[test]
fn pre_attachment_clear_remains_authoritative_after_an_ordinary_edit() {
    let store = MemBackend::default();
    store.write(ACTIONS_FILE, br#"{"version":1,"actions":[{"name":"Disk"}]}"#).unwrap();
    let mut s = crate::session();
    s.actions_mut().list.push(Action::new("Early", Vec::new())).unwrap();
    s.execute("actions.clearAll", json!({"confirm":true})).unwrap();
    s.execute("actions.newSet", json!({"name":"New"})).unwrap();
    assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
    assert!(s.actions().list.actions().is_empty());
    assert_eq!(s.actions().list.sets(), ["New"]);
    let mut reopened = crate::session();
    assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
    assert!(reopened.actions().list.actions().is_empty());
    assert_eq!(reopened.actions().list.sets(), ["New"]);
}

#[test]
fn empty_pre_attachment_clear_replaces_the_saved_library() {
    for undo in [false, true] {
        let store = MemBackend::default();
        store.write(ACTIONS_FILE, br#"{"version":1,"actions":[{"name":"Disk"}]}"#).unwrap();
        let mut s = crate::session();
        assert_eq!(s.execute("actions.clearAll", json!({"confirm":true})).unwrap()["cleared"], 0);
        let revision = s.actions().list.revision();
        assert_eq!(s.execute("actions.clearAll", json!({"confirm":true})).unwrap()["cleared"], 0);
        assert_eq!(s.actions().list.revision(), revision, "a second empty clear keeps the first undo");
        if undo {
            s.execute("actions.undoLibrary", json!({})).unwrap();
        }
        assert!(s.attach_preset_store(open(Box::new(store.clone()))).is_empty());
        assert_eq!(s.actions().list.actions().first().map(|action| action.name.as_str()), undo.then_some("Disk"));
        let mut reopened = crate::session();
        assert!(reopened.attach_preset_store(open(Box::new(store))).is_empty());
        assert_eq!(reopened.actions().list.actions(), s.actions().list.actions());
    }
}
