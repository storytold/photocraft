//! Cross-contract controls: canonical IDs/folders survive the native library but never portable import.
use super::*;
use craft_psd::{atn::{self, AtnAction, AtnFile}, descriptor::UnicodeString};

#[test]
fn same_caption_physical_folders_keep_ids_and_distinct_shortcut_headers() {
    let files: Vec<_> = ["First", "Second"].into_iter().map(|name| AtnFile { name: UnicodeString::new_nul("Same"), actions: vec![AtnAction { name: UnicodeString::new_nul(name), ..Default::default() }], ..Default::default() }).collect();
    let mut state = library::decode(&atn::write_sets(&files).unwrap()).unwrap();
    let ids = ["11111111111111111111111111111111", "22222222222222222222222222222222"];
    for (action, id) in state.list.iter_mut().zip(ids) { action.action_id = Some(id.into()); }
    let keys = vec![(occurrence_key(ids[0]), "F6".into()), (occurrence_key(ids[1]), "F7".into())];
    let restored = library::decode(&library::encode(&state, &keys).unwrap()).unwrap();
    assert_eq!(restored.set_rows, ["Same", "Same"]);
    assert_eq!(restored.list.iter().map(|action| action.set_slot).collect::<Vec<_>>(), [Some(0), Some(1)]);
    assert_eq!(restored.list.iter().map(|action| action.action_id.as_deref()).collect::<Vec<_>>(), ids.map(Some));
    assert_eq!(library::function_keys(&restored), keys);
}

#[test]
fn implicit_empty_wire_legacy_empty_group_and_explicit_empty_folder_remain_distinct() {
    let bytes = atn::write_sets(&[AtnFile { name: UnicodeString::new_nul(""), ..Default::default() }, AtnFile { name: UnicodeString::new_nul("Explicit"), ..Default::default() }]).unwrap();
    let mut state = library::decode(&bytes).unwrap(); state.sets.push("Legacy".into());
    let restored = library::decode(&library::encode(&state, &[]).unwrap()).unwrap();
    assert_eq!(restored.set_rows, ["Explicit"]); assert_eq!(restored.sets, ["Legacy"]); assert!(restored.list.is_empty());
    let encoded = atn::parse_sets(&library::encode(&restored, &[]).unwrap()).unwrap();
    let visible: Vec<_> = encoded.iter().filter(|set| !crate::atn_cmds::sidecar::is_wrapper(set)).map(|set| set.name.to_string_lossy()).collect();
    assert_eq!(visible, ["", "Explicit", "Legacy"]);
}

#[test]
fn local_identity_restore_does_not_authorize_or_identify_a_portable_import() {
    let id = "33333333333333333333333333333333";
    let mut action = Action::new("Native", vec![("layer.new.layer".into(), json!({"name":"kept"}))]);
    action.action_id = Some(id.into()); action.legacy_shortcut_owner = true;
    let state = ActionState { list: vec![action], ..Default::default() };
    let bytes = library::encode(&state, &[]).unwrap();
    assert_eq!(library::decode(&bytes).unwrap().list[0].action_id.as_deref(), Some(id));
    let mut session = Session::new();
    session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(session.actions.list[0].action_id, None); assert!(!session.actions.list[0].legacy_shortcut_owner);
    let saved = session.execute("actions.save", json!({})).unwrap();
    let portable = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert!(!portable.windows(b"photocraftLibraryIdentityV1".len()).any(|part| part == b"photocraftLibraryIdentityV1"));
    let mut forbidden = Action::new("Policy", vec![("plugin.install".into(), json!({"name":"bad"}))]); forbidden.action_id = Some(id.into());
    let local = library::encode(&ActionState { list: vec![forbidden], ..Default::default() }, &[]).unwrap();
    assert!(library::decode(&local).is_ok());
    assert!(session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&local)})).is_err());
}


#[test]
fn attaching_compatible_local_libraries_does_not_treat_private_identity_as_foreign_notes() {
    let local = |name: &str, id: &str| {
        let mut action = Action::new(name, Vec::new()); action.action_id = Some(id.into());
        library::decode(&library::encode(&ActionState { list: vec![action], ..Default::default() }, &[]).unwrap()).unwrap()
    };
    let mut left = local("Left", "11111111111111111111111111111111");
    let right = local("Right", "22222222222222222222222222222222");
    library::attach(&mut left, right);
    let restored = library::decode(&library::encode(&left, &[]).unwrap()).unwrap();
    assert_eq!(restored.list.iter().map(|action| (action.name.as_str(), action.action_id.as_deref())).collect::<Vec<_>>(), [("Left", Some("11111111111111111111111111111111")), ("Right", Some("22222222222222222222222222222222"))]);
}

#[test]
fn whole_library_save_exports_an_empty_physical_folder_without_actions() {
    let bytes = atn::write_sets(&[AtnFile { name: UnicodeString::new_nul("Only empty"), ..Default::default() }]).unwrap();
    let mut session = Session::new(); session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert!(session.actions.list.is_empty()); assert!(session.actions.sets.is_empty()); assert_eq!(session.actions.set_rows, ["Only empty"]);
    let explicit = session.execute("actions.save", json!({"set":"Only empty","setSlot":0})).unwrap();
    let all = session.execute("actions.save", json!({})).unwrap();
    for (response, expected) in [(explicit, "Only empty"), (all, crate::atn_cmds::DEFAULT_SET)] {
        let bytes = photocraft_paint::tile::b64_decode(response["data"].as_str().unwrap()).unwrap(); let files = atn::parse_sets(&bytes).unwrap();
        let visible: Vec<_> = files.iter().filter(|file| !crate::atn_cmds::sidecar::is_wrapper(file)).collect();
        assert_eq!(visible.len(), 1); assert_eq!(visible[0].name.to_string_lossy(), expected); assert!(visible[0].actions.is_empty());
    }
}


#[test]
fn a_durable_call_keeps_its_target_after_rename_or_folder_assignment() {
    let id = "44444444444444444444444444444444";
    let mut target = Action::new("Renamed", Vec::new()); target.action_id = Some(id.into()); target.set = Some("Moved".into());
    let list = vec![Action { set: Some("Old".into()), ..Action::new("Original", Vec::new()) }, target];
    assert_eq!(resolve_play_target(&list, &json!({"action":"Original","set":"Old","actionId":id}), "actions.play").unwrap(), 1);
    assert!(resolve_play_target(&list, &json!({"action":"Original","set":"Old","actionId":"missing"}), "actions.play").is_err());
    assert_eq!(resolve_play_target(&list, &json!({"action":"Original","set":"Old"}), "actions.play").unwrap(), 0);
}


#[test]
fn whole_library_export_keeps_named_and_id_calls_in_the_targets_physical_set() {
    for use_id in [false, true] {
        let id = "55555555555555555555555555555555";
        let mut target = Action::new("Target", Vec::new()); target.set = Some("A".into()); target.set_slot = Some(0); target.action_id = Some(id.into());
        let call = if use_id { json!({"action":"Target","actionId":id}) } else { json!({"action":"Target"}) };
        let mut caller = Action::new("Caller", vec![("actions.play".into(), call)]); caller.set = Some("B".into()); caller.set_slot = Some(1);
        let mut session = Session::new(); session.actions.list = vec![target, caller]; session.actions.set_rows = vec!["A".into(), "B".into()];
        let response = session.execute("actions.save", json!({})).unwrap(); let bytes = photocraft_paint::tile::b64_decode(response["data"].as_str().unwrap()).unwrap();
        let mut imported = crate::atn_cmds::import(&bytes).unwrap(); crate::atn_cmds::normalize_import(&mut imported, [], false).unwrap();
        assert_eq!(imported.actions[1].steps, [("actions.play".into(), json!({"action":"Target","set":"A"}))]);
        assert_eq!(imported.actions[0].action_id, None);
    }
}


#[test]
fn stale_identity_metadata_refuses_native_record_reordering_without_rebinding_shortcuts() {
    let mut state = ActionState::default();
    for (name, id) in [("First", "11111111111111111111111111111111"), ("Second", "22222222222222222222222222222222")] {
        let mut action = Action::new(name, Vec::new()); action.action_id = Some(id.into()); state.list.push(action);
    }
    let original = library::encode(&state, &[]).unwrap(); let mut files = atn::parse_sets(&original).unwrap();
    assert_eq!(files[0].actions.len(), 2); files[0].actions.swap(0, 1);
    let changed = atn::write_sets(&files).unwrap();
    assert!(library::decode(&changed).unwrap_err().contains("no longer matches its native records"));
    assert_eq!(library::decode(&original).unwrap().list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    assert!(library::decode_import(&changed).unwrap().list.iter().all(|action| action.action_id.is_none()));
}
