//! Codec and migration tests never execute the commands carried by untrusted action files.
use super::{Action, ActionState, library};
use crate::{
    atn_cmds,
    preset_store::{self, ACTIONS_FILE, LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES, MIGRATED_ACTIONS_FILE, MemBackend, PresetBackend},
};
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use serde_json::{Value, json};

fn wire() -> Vec<u8> {
    fn ascii(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut out = 16u32.to_be_bytes().to_vec();
    // Independent wire oracle: duplicate names, unpaired UTF16 and noncanonical true bytes.
    let name = [0, 0, 0, 2, 0xd8, 0, 0, 0];
    for _ in 0..2 {
        out.extend_from_slice(&name);
        out.push(2);
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&[0, 3, 2, 0, 0, 12]);
        out.extend_from_slice(&name);
        out.push(2);
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&[2, 2, 0, 3]);
        out.extend_from_slice(b"TEXT");
        ascii(&mut out, b"futureEvent");
        ascii(&mut out, b"Opaque");
        out.extend_from_slice(&(-1i32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"null");
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"flagbool");
        out.push(2);
    }
    out
}

fn encode(state: &ActionState) -> Vec<u8> {
    library::encode(state, &library::function_keys(state)).unwrap()
}

#[test]
fn untouched_duplicate_physical_records_and_noncanonical_bytes_survive() {
    let bytes = wire();
    let state = library::decode(&bytes).unwrap();
    assert_eq!(state.sets.len(), 2);
    assert_eq!(state.list.len(), 2);
    assert_eq!(state.list[0].name, state.list[1].name);
    assert_eq!(state.list[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(encode(&state), bytes);
    assert_ne!(atn::write_sets(&atn::parse_sets(&bytes).unwrap()).unwrap(), bytes);
}

/// Strict loading preserves repeated physical rows; external import intentionally renames them.
fn ordered_archive(rows: &[(&str, &[Action])]) -> Vec<u8> {
    let files: Vec<_> = rows
        .iter()
        .map(|(name, actions)| {
            let refs: Vec<_> = actions.iter().collect();
            let (bytes, skipped) = atn_cmds::export(name, &refs, &[], false).unwrap();
            assert!(skipped.is_empty());
            atn::parse_sets(&bytes).unwrap().remove(0)
        })
        .collect();
    atn::write_sets(&files).unwrap()
}

#[test]
fn durable_regrouping_keeps_flat_call_targets_and_empty_rows() {
    let first = Action::new("First", vec![]);
    let second = Action::new("Second", vec![]);
    let other = Action::new("Other", vec![]);
    let caller = Action::new("Caller", vec![("actions.play".into(), json!({"action":1}))]);
    let bytes = ordered_archive(&[("Before", &[]), ("A", &[first, second]), ("Between", &[]), ("B", &[other]), ("C", &[caller])]);
    let state = library::decode(&bytes).unwrap();
    assert_eq!(encode(&state), bytes, "untouched empty rows retain exact physical placement");
    let mut session = crate::Session::new();
    session.actions = state;
    session.execute("actions.assignSet", json!({"action":0,"set":"B"})).unwrap();
    let restored = library::decode(&encode(&session.actions)).unwrap();
    assert_eq!(restored.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "Other", "Caller"]);
    assert_eq!(restored.list[3].steps[0].1["action"], 1);
    assert_eq!(restored.list[1].name, "Second");
    for name in ["Before", "Between"] {
        assert_eq!(restored.sets.iter().filter(|row| row.as_str() == name).count(), 1);
    }
}

#[test]
fn duplicate_physical_rows_stay_after_all_original_targets() {
    let first = Action::new("First", vec![]);
    let second = Action::new("Second", vec![]);
    let caller = Action::new("Caller", vec![("actions.play".into(), json!({"action":1}))]);
    let bytes = ordered_archive(&[("Group", &[first]), ("Empty", &[]), ("Group", &[second]), ("Other", &[caller])]);
    let mut session = crate::Session::new();
    session.actions = library::decode(&bytes).unwrap();
    session.execute("actions.duplicate", json!({"set":"Group"})).unwrap();
    let restored = library::decode(&encode(&session.actions)).unwrap();
    assert_eq!(restored.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "First copy", "Second copy", "Caller"]);
    assert_eq!(restored.list[4].steps[0].1["action"], 1);
    assert_eq!(restored.list[1].name, "Second");
    assert_eq!(restored.sets.iter().filter(|name| name.as_str() == "Group").count(), 2);
    assert_eq!(restored.sets.iter().filter(|name| name.as_str() == "Empty").count(), 1);
}

#[test]
fn changing_one_name_preserves_every_other_original_field_and_occurrence() {
    let bytes = wire();
    let spans = atn::parse_sets_with_spans(&bytes).unwrap();
    let mut state = library::decode(&bytes).unwrap();
    state.list[0].name = "Edited".into();
    let output = encode(&state);
    let new = atn::parse_sets_with_spans(&output).unwrap();
    assert_eq!(&output[new.spans[1].range.clone()], &bytes[spans.spans[1].range.clone()]);
    let old = &spans.spans[0].actions[0];
    let changed = &new.spans[0].actions[0];
    assert_eq!(&output[changed.items[0].range.clone()], &bytes[old.items[0].range.clone()]);
    assert_eq!(&output[changed.shift.clone()], &[2]);
    assert_eq!(&output[changed.color.clone()], &[0, 12]);
    assert_eq!(new.sets[0].actions[0].name.to_string_lossy(), "Edited");
}

fn close_archive() -> Vec<u8> {
    atn::write_sets(&[AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Save and close"),
            items: vec![AtnItem {
                event: AtnEvent::name("close"),
                descriptor: Some(Descriptor::new("null").with("Svng", D::Enumerated { type_id: Id::new("YsN "), value: Id::new("Ys  ") })),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }])
    .unwrap()
}

#[test]
fn one_event_expanding_to_two_native_steps_is_written_once_until_edited() {
    let bytes = close_archive();
    let mut state = library::decode(&bytes).unwrap();
    assert_eq!(state.list[0].steps, [("file.save".into(), json!({})), ("file.close".into(), json!({}))]);
    assert_eq!(encode(&state), bytes);
    state.list[0].steps.insert(0, ("future.command".into(), json!({"x": 3})));
    state.list[0].remap_steps(|index| Some(index + 1));
    let output = encode(&state);
    let parsed = atn::parse_sets(&output).unwrap();
    assert_eq!(parsed[0].actions[0].items.len(), 2, "insert plus the original single Close event");
    assert_eq!(library::decode(&output).unwrap().list[0].steps, state.list[0].steps);
    state.list[0].steps[1].1 = json!({"futureOption": true});
    let output = encode(&state);
    assert_eq!(library::decode(&output).unwrap().list[0].steps, state.list[0].steps, "editing a part must not replay the original whole cohort too");
}

#[test]
fn native_unknown_commands_keep_exact_params_and_flags_without_dispatch() {
    let mut action = Action::new("Native", vec![("unregistered.future.command".into(), json!({"float": 2.0, "nested": [true, null]}))]);
    action.disabled.insert(0);
    action.dialogs.insert(0);
    action.description = "Description".into();
    action.icon = "FutureIcon".into();
    let state = ActionState { list: vec![action.clone()], ..Default::default() };
    let bytes = encode(&state);
    let loaded = library::decode(&bytes).unwrap();
    assert_eq!(loaded.list[0].steps, action.steps);
    assert_eq!(loaded.list[0].disabled, action.disabled);
    assert_eq!(loaded.list[0].dialogs, action.dialogs);
    assert_eq!(loaded.list[0].description, action.description);
    assert_eq!(loaded.list[0].icon, action.icon);
    assert_eq!(encode(&loaded), bytes);
}

#[test]
fn invalid_known_native_payload_rejects_the_complete_archive() {
    let mut file = atn::parse_sets(&close_archive()).unwrap().remove(0);
    file.actions[0].items.push(AtnItem {
        event: AtnEvent::name("photocraftCommand"),
        descriptor: Some(
            Descriptor::new("null").with("command", D::Text(UnicodeString::new("future.command"))).with("params", D::Text(UnicodeString::new("{broken"))),
        ),
        ..Default::default()
    });
    assert!(library::decode(&atn::write_sets(&[file]).unwrap()).is_err());
    let mut partial = wire();
    partial.pop();
    assert!(library::decode(&partial).is_err());
}

#[test]
fn empty_library_has_a_known_carrier_and_no_invented_user_folder() {
    let bytes = encode(&ActionState::default());
    let loaded = library::decode(&bytes).unwrap();
    assert!(loaded.list.is_empty());
    assert!(loaded.sets.is_empty());
    assert_eq!(atn::parse_sets(&bytes).unwrap().len(), 1);
    assert_eq!(encode(&loaded), bytes);
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.actions.is_empty());
    assert!(imported.sets.is_empty());
    // A foreign sidecar with no real set is still rejected; only the explicit native
    // empty-library marker permits this otherwise ambiguous input through public import.
    let mut files = atn::parse_sets(&bytes).unwrap();
    let outer = files[0].actions[0].items[0].descriptor.as_mut().unwrap();
    let (_, D::RawData(payload)) = outer.items.iter_mut().find(|(id, _)| id.is("modernizedActionData")).unwrap() else { panic!("native payload") };
    let mut root = Descriptor::from_bytes(&payload[4..]).unwrap();
    root.items.retain(|(id, _)| !id.is("photocraftEmptyLibraryV1"));
    payload.truncate(4);
    payload.extend(root.to_bytes());
    assert!(atn_cmds::import(&atn::write_sets(&files).unwrap()).is_err());
    assert!(library::decode(&16u32.to_be_bytes()).is_err());
}

#[test]
fn literal_ungrouped_rows_remain_distinct_from_the_unnamed_category() {
    let rows = [("", "Loose"), (atn_cmds::UNGROUPED_SET, "Named"), ("Ungrouped Actions 2", "Collision")];
    let files: Vec<_> = rows
        .iter()
        .map(|(set, action)| AtnFile {
            name: UnicodeString::new_nul(set),
            actions: vec![AtnAction { name: UnicodeString::new_nul(action), ..Default::default() }],
            ..Default::default()
        })
        .collect();
    let bytes = atn::write_sets(&files).unwrap();
    let state = library::decode(&bytes).unwrap();
    assert_eq!(state.sets, ["Ungrouped Actions", "Ungrouped Actions 3", "Ungrouped Actions 2"]);
    assert_eq!(state.list[0].set, None);
    assert_eq!(state.list[1].set.as_deref(), Some("Ungrouped Actions 3"));
    assert_eq!(encode(&state), bytes, "logical disambiguation keeps untouched original wire names");
    let mut session = crate::Session::new();
    session.actions = state;
    session.execute("actions.delete", json!({"set":"Ungrouped Actions"})).unwrap();
    assert_eq!(session.actions.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["Named", "Collision"]);
    let loaded = library::decode(&encode(&session.actions)).unwrap();
    assert_eq!(loaded.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["Named", "Collision"]);
    session.execute("actions.delete", json!({"set":"Ungrouped Actions 2"})).unwrap();
    let loaded = library::decode(&encode(&session.actions)).unwrap();
    assert_eq!(loaded.list[0].set.as_deref(), Some("Ungrouped Actions 3"), "removing the collision cannot change the surviving logical folder on reload");
}

fn legacy() -> Vec<u8> {
    br#" {"version":1,"actions":[{"name":"Legacy","description":"kept","steps":[["future.command",{"x":1}]],"disabled":[0]}]} "#.to_vec()
}

#[test]
fn migration_publishes_one_atn_and_renames_exact_json_bytes_once() {
    for bytes in [legacy(), br#"{"version":1,"actions":[]}"#.to_vec()] {
        let mem = MemBackend::default();
        mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), bytes.clone());
        let opened = preset_store::open(Box::new(mem.clone()));
        assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
        assert!(matches!(opened.actions, preset_store::LoadedActions::Loaded(_) | preset_store::LoadedActions::Migrated(_)));
        assert!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
        assert_eq!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
        let atn = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
        let state = library::decode(&atn).unwrap();
        assert!(state.library.legacy.as_ref().unwrap().matches(&bytes));
        let reopened = preset_store::open(Box::new(mem.clone()));
        assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
        assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), atn);
    }
}

#[test]
fn portable_exports_do_not_inherit_full_library_migration_receipts() {
    let bytes = legacy();
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), bytes.clone());
    assert!(preset_store::open(Box::new(mem.clone())).warnings.is_empty());
    let state = library::decode(&mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap()).unwrap();
    assert!(state.library.legacy.as_ref().unwrap().matches(&bytes));
    let (portable, skipped) = atn_cmds::export("Portable", &[&state.list[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert!(library::decode(&portable).unwrap().library.legacy.is_none());
    let unrelated_store = MemBackend::default();
    unrelated_store.files.lock().unwrap().extend([(ACTIONS_FILE.into(), portable), (LEGACY_ACTIONS_FILE.into(), bytes.clone())]);
    let _ = preset_store::open(Box::new(unrelated_store.clone()));
    assert_eq!(unrelated_store.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    assert!(unrelated_store.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
}

#[test]
fn corrupt_atn_has_precedence_and_corrupt_legacy_is_never_overwritten() {
    for (name, bytes) in [(ACTIONS_FILE, b"broken ATN".to_vec()), (LEGACY_ACTIONS_FILE, b"broken JSON".to_vec())] {
        let mem = MemBackend::default();
        mem.files.lock().unwrap().insert(name.into(), bytes.clone());
        if name == ACTIONS_FILE {
            mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), legacy());
        }
        let opened = preset_store::open(Box::new(mem.clone()));
        assert!(matches!(opened.actions, preset_store::LoadedActions::Failed));
        assert!(!opened.warnings.is_empty());
        let mut session = crate::Session::new();
        session.attach_preset_store(opened);
        session.actions.list.push(Action::new("Local", vec![]));
        session.actions.rev += 1;
        session.sync_preset_store();
        assert_eq!(mem.read(name, MAX_ACTIONS_BYTES).unwrap(), bytes);
        assert!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
    }
}

#[test]
fn retirement_conflict_keeps_all_copies_and_restart_does_not_guess_a_backup_name() {
    let mem = MemBackend::default();
    let bytes = legacy();
    mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), bytes.clone());
    mem.files.lock().unwrap().insert(MIGRATED_ACTIONS_FILE.into(), b"older backup".to_vec());
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(matches!(opened.actions, preset_store::LoadedActions::Loaded(_) | preset_store::LoadedActions::Migrated(_)));
    assert!(opened.warnings.iter().any(|warning| warning.contains("pending")));
    let atn = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    preset_store::open(Box::new(mem.clone()));
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), atn);
    assert_eq!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    assert_eq!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), b"older backup");
}

#[test]
fn preattachment_edits_empty_rows_and_same_name_disk_occurrences_all_survive() {
    let mut live = ActionState { list: vec![Action::new("Same", vec![])], sets: vec!["Live empty".into()], rev: 1, ..Default::default() };
    let disk = library::decode(&wire()).unwrap();
    library::attach(&mut live, disk);
    assert_eq!(live.list.len(), 3);
    assert_eq!(live.sets[0], "Live empty");
    let output = encode(&live);
    let loaded = library::decode(&output).unwrap();
    assert_eq!(loaded.list.len(), 3);
    assert!(loaded.sets.iter().any(|name| name == "Live empty"));
}

#[test]
fn changing_one_step_flag_preserves_other_raw_flag_bytes_and_descriptor() {
    let bytes = wire();
    let old = atn::parse_sets_with_spans(&bytes).unwrap();
    let mut state = library::decode(&bytes).unwrap();
    state.list[0].disabled.insert(0);
    let output = encode(&state);
    let parsed = atn::parse_sets_with_spans(&output).unwrap();
    let original = &old.spans[0].actions[0].items[0];
    let changed = &parsed.spans[0].actions[0].items[0];
    assert_eq!(&output[changed.flags.clone()], &[2, 0, 0, 3]);
    assert_eq!(&output[changed.flags.end..changed.range.end], &bytes[original.flags.end..original.range.end]);
}

#[test]
fn deleting_one_native_part_does_not_resurrect_the_original_multi_step_event() {
    let mut state = library::decode(&close_archive()).unwrap();
    state.list[0].steps.remove(0);
    state.list[0].remap_steps(|index| index.checked_sub(1));
    let output = encode(&state);
    assert_eq!(library::decode(&output).unwrap().list[0].steps, [("file.close".into(), json!({}))]);
}

#[test]
fn shortcut_only_edits_change_headers_without_touching_original_event_bytes() {
    let bytes = close_archive();
    let state = library::decode(&bytes).unwrap();
    let output = library::encode(&state, &[("Save and close".into(), "Cmd+Shift+F8".into())]).unwrap();
    let old = atn::parse_sets_with_spans(&bytes).unwrap();
    let new = atn::parse_sets_with_spans(&output).unwrap();
    let header = &new.sets[0].actions[0];
    assert_eq!((header.function_key, header.shift, header.command), (8, true, true));
    assert_eq!(&output[new.spans[0].actions[0].items[0].range.clone()], &bytes[old.spans[0].actions[0].items[0].range.clone()]);
}

#[test]
fn portable_export_keeps_or_reports_native_steps_instead_of_truncating_options() {
    let action = Action::new("Native", vec![("future.command".into(), json!({"option": true}))]);
    let (kept, skipped) = atn_cmds::export("S", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn_cmds::import(&kept).unwrap().actions[0].steps, action.steps);
    let (portable, skipped) = atn_cmds::export("S", &[&action], &[], true).unwrap();
    assert_eq!(skipped.len(), 1);
    assert!(atn::parse_sets(&portable).unwrap()[0].actions[0].items.is_empty());
}

fn interrupted_migration(mem: &MemBackend, bytes: &[u8]) -> Vec<u8> {
    let mut state = ActionState::default();
    state.library.legacy = Some(library::LegacyOrigin::new(bytes).unwrap());
    let atn = encode(&state);
    let mut files = mem.files.lock().unwrap();
    files.insert(ACTIONS_FILE.into(), atn.clone());
    files.insert(LEGACY_ACTIONS_FILE.into(), bytes.to_vec());
    atn
}

#[test]
fn restart_after_atn_commit_completes_exact_legacy_retirement_without_rewriting_atn() {
    let mem = MemBackend::default();
    let bytes = legacy();
    let atn = interrupted_migration(&mem, &bytes);
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), atn);
    assert_eq!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    assert!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
}

#[test]
fn restart_preserves_changed_sources_and_reports_missing_recovery_copies() {
    let mem = MemBackend::default();
    let atn = interrupted_migration(&mem, &legacy());
    mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), b"changed source".to_vec());
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(opened.warnings.iter().any(|warning| warning.contains("differing legacy")));
    assert_eq!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), b"changed source");
    assert!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
    mem.files.lock().unwrap().remove(LEGACY_ACTIONS_FILE);
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(opened.warnings.iter().any(|warning| warning.contains("recovery copy")));
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), atn);
}

#[test]
fn unrelated_json_is_never_retired_merely_because_valid_atn_exists() {
    let mem = MemBackend::default();
    let atn = close_archive();
    let mut files = mem.files.lock().unwrap();
    files.insert(ACTIONS_FILE.into(), atn.clone());
    files.insert(LEGACY_ACTIONS_FILE.into(), legacy());
    drop(files);
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), atn);
    assert_eq!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), legacy());
    assert!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
}

#[test]
fn backend_migration_failure_preserves_json_and_keeps_definitions_in_memory() {
    struct NoMigration(MemBackend);
    impl PresetBackend for NoMigration {
        fn list(&self) -> Result<Vec<(String, u64)>, String> {
            self.0.list()
        }
        fn read(&self, name: &str, max: u64) -> Result<Vec<u8>, String> {
            self.0.read(name, max)
        }
        fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
            self.0.write(name, bytes)
        }
        fn remove(&self, name: &str) -> Result<(), String> {
            self.0.remove(name)
        }
    }
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), legacy());
    let opened = preset_store::open(Box::new(NoMigration(mem.clone())));
    assert!(matches!(opened.actions, preset_store::LoadedActions::MigrationBlocked(_)));
    assert_eq!(opened.actions[0].name, "Legacy");
    let mut session = crate::Session::new();
    session.attach_preset_store(opened);
    session.actions.list[0].name = "Memory edit".into();
    session.actions.rev += 1;
    session.sync_preset_store();
    assert_eq!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), legacy());
    assert!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
    assert!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
}

#[test]
fn original_native_json_text_survives_an_unrelated_name_edit() {
    let text = " { \"x\" : 2.0, \"future\" : [true, null] }\n";
    let file = AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Native text"),
            items: vec![AtnItem {
                event: AtnEvent::name("photocraftCommand"),
                descriptor: Some(
                    Descriptor::new("null").with("command", D::Text(UnicodeString::new("future.command"))).with("params", D::Text(UnicodeString::new(text))),
                ),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let bytes = atn::write_sets(&[file]).unwrap();
    let mut state = library::decode(&bytes).unwrap();
    assert_eq!(state.list[0].steps[0].1, json!({"x": 2.0, "future": [true, null]}));
    assert_eq!(encode(&state), bytes);
    state.list[0].name = "Renamed".into();
    let output = encode(&state);
    let old = atn::parse_sets_with_spans(&bytes).unwrap();
    let new = atn::parse_sets_with_spans(&output).unwrap();
    assert_eq!(&output[new.spans[0].actions[0].items[0].range.clone()], &bytes[old.spans[0].actions[0].items[0].range.clone()]);
}

#[test]
fn shortcut_change_without_action_revision_is_persisted() {
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert(ACTIONS_FILE.into(), close_archive());
    let mut session = crate::Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let revision = session.actions.rev;
    session.edit_prefs(|prefs| {
        prefs.shortcuts.insert(format!("{}Save and close", super::PLAY_SHORTCUT_PREFIX), "Cmd+Shift+F9".into());
    });
    session.sync_preset_store();
    assert_eq!(session.actions.rev, revision);
    let bytes = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let files = atn::parse_sets(&bytes).unwrap();
    let action = &files[0].actions[0];
    assert_eq!((action.function_key, action.shift, action.command), (9, true, true));
}

/// Build reserved native events directly so the writer under test is not the input oracle.
fn native_archive(rows: &[(&str, &[Action])]) -> Vec<u8> {
    let files: Vec<_> = rows
        .iter()
        .map(|(name, actions)| AtnFile {
            name: UnicodeString::new_nul(name),
            actions: actions
                .iter()
                .map(|action| AtnAction {
                    name: UnicodeString::new_nul(&action.name),
                    items: action
                        .steps
                        .iter()
                        .map(|(id, params)| AtnItem {
                            event: AtnEvent::name("photocraftCommand"),
                            descriptor: Some(
                                Descriptor::new("null")
                                    .with("command", D::Text(UnicodeString::new(id)))
                                    .with("params", D::Text(UnicodeString::new(&params.to_string()))),
                            ),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    atn::write_sets(&files).unwrap()
}

#[test]
fn disk_attachment_and_portable_append_rebase_only_the_added_numeric_calls() {
    let first = Action::new("First", vec![]);
    let second = Action::new("Second", vec![]);
    let caller = Action::new(
        "Caller",
        vec![
            ("actions.play".into(), json!({"action":1})),
            ("file.automate.batch".into(), json!({"action":1})),
            ("file.automate.batch".into(), json!({"action":0,"calledActions":[{"name":"Embedded","steps":[]}]})),
            ("file.automate.batch".into(), json!({"steps":[["actions.play",{"action":0}]],"calledActions":[{"name":"Embedded","steps":[]}]})),
        ],
    );
    let bytes = native_archive(&[("Disk", &[first, second, caller])]);
    for portable in [false, true] {
        let mut session = crate::Session::new();
        session.actions.list.push(Action::new("Local", vec![("actions.play".into(), json!({"action":0}))]));
        if portable {
            session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes),"replace":false})).unwrap();
        } else {
            library::attach(&mut session.actions, library::decode(&bytes).unwrap());
        }
        let restored = library::decode(&encode(&session.actions)).unwrap();
        assert_eq!(restored.list[0].steps[0].1["action"], 0);
        assert_eq!(restored.list[3].steps[0].1["action"], 2);
        assert_eq!(restored.list[3].steps[1].1["action"], 2);
        assert_eq!(restored.list[3].steps[2].1["action"], 1, "named Batch selects its source from the caller list");
        assert_eq!(restored.list[3].steps[3].1["steps"][0][1]["action"], 0, "inline Batch uses its embedded action namespace");
        assert_eq!(restored.list[2].name, "Second");
    }
}

#[test]
fn scalar_native_call_parameters_survive_projection_without_panicking() {
    for id in ["actions.play", "actions.conditional"] {
        for params in [json!(7), json!(true), json!("scalar"), json!([]), Value::Null] {
            let action = Action::new("Malformed call", vec![(id.into(), params)]);
            let bytes = native_archive(&[("S", std::slice::from_ref(&action))]);
            let state = library::decode(&bytes).unwrap();
            // An unrelated edit forces the native projection path through library_items.
            let fresh = ActionState {
                list: state
                    .list
                    .into_iter()
                    .map(|mut action| {
                        action.library_origin = None;
                        action.library_steps.clear();
                        action
                    })
                    .collect(),
                ..Default::default()
            };
            assert_eq!(library::decode(&encode(&fresh)).unwrap().list[0].steps, action.steps);
            let (portable, _) = atn_cmds::export("S", &[&action], &[], false).unwrap();
            assert_eq!(atn_cmds::import(&portable).unwrap().actions[0].steps, action.steps);
        }
    }
}

#[test]
fn imported_explicit_wire_scope_wins_over_the_empty_set_display_alias() {
    let child = Action::new("Child", vec![]);
    let caller = Action::new(
        "Caller",
        vec![
            ("actions.play".into(), json!({"action":"Child","set":"Ungrouped Actions"})),
            ("actions.conditional".into(), json!({"then":"Child","thenSet":"Ungrouped Actions","else":"Child","elseSet":""})),
            ("actions.conditional".into(), json!({"then":"Child","else":"Child","set":""})),
        ],
    );
    let bytes = native_archive(&[("", std::slice::from_ref(&child)), ("Ungrouped Actions", &[child.clone(), caller])]);
    let mut imported = atn_cmds::import(&bytes).unwrap();
    atn_cmds::normalize_import(&mut imported, [], false).unwrap();
    let steps = &imported.actions[2].steps;
    assert_eq!(steps[0].1, json!({"action":"Child 2","set":"Ungrouped Actions 2"}));
    assert_eq!(steps[1].1["then"], "Child 2");
    assert_eq!(steps[1].1["thenSet"], "Ungrouped Actions 2");
    assert_eq!(steps[1].1["else"], "Child");
    assert_eq!(steps[1].1["elseSet"], "Ungrouped Actions");
    assert_eq!(steps[2].1, json!({"then":"Child","else":"Child","set":"Ungrouped Actions"}));
}

#[test]
fn set_override_retargets_internal_play_and_conditional_scopes_before_collision_renaming() {
    let child = Action::new("Child", vec![]);
    let caller = Action::new(
        "Caller",
        vec![
            ("actions.play".into(), json!({"action":"Child","set":"S"})),
            ("actions.conditional".into(), json!({"then":"Child","set":"S","thenSet":"S","else":"Child","elseSet":"Foreign"})),
        ],
    );
    let bytes = native_archive(&[("S", &[child, caller])]);
    let mut session = crate::Session::new();
    session.actions.list.push(Action::new("Child", vec![]));
    session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes),"set":"T","replace":false})).unwrap();
    let restored = library::decode(&encode(&session.actions)).unwrap();
    let steps = &restored.list[2].steps;
    assert_eq!(steps[0].1, json!({"action":"Child 2","set":"T"}));
    assert_eq!(steps[1].1["then"], "Child 2");
    assert_eq!(steps[1].1["set"], "T");
    assert_eq!(steps[1].1["thenSet"], "T");
    assert_eq!(steps[1].1["else"], "Child");
    assert_eq!(steps[1].1["elseSet"], "Foreign");
}

#[test]
fn import_checks_exact_durable_group_capacity_before_changing_actions_or_preferences() {
    let mut session = crate::Session::new();
    for index in 0..atn::MAX_SETS {
        let mut action = Action::new(format!("Local {index}"), vec![]);
        action.set = Some(if index % 2 == 0 { "A" } else { "B" }.into());
        session.actions.list.push(action);
    }
    let before = encode(&session.actions);
    let preferences = session.prefs().clone();
    let revision = session.actions.rev;
    let bytes = native_archive(&[("C", &[Action::new("Imported", vec![])])]);
    assert!(session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes),"replace":false})).is_err());
    assert_eq!(encode(&session.actions), before);
    assert_eq!(session.prefs(), &preferences);
    assert_eq!(session.actions.rev, revision);
    // Replacing the interleaved A rows reduces the actual durable group count and is admitted.
    let bytes = native_archive(&[("A", &[Action::new("Replacement", vec![])])]);
    session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert!(library::decode(&encode(&session.actions)).is_ok());
}

#[test]
fn default_single_set_export_retargets_calls_in_copies_and_reimports_standalone() {
    let mut child = Action::new("Child", vec![]);
    child.set = Some("S".into());
    let mut parent = Action::new(
        "Parent",
        vec![
            ("actions.play".into(), json!({"action":"Child","set":"S"})),
            ("actions.conditional".into(), json!({"then":"Child","thenSet":"S","else":"Child","set":"S"})),
        ],
    );
    parent.set = Some("S".into());
    let mut session = crate::Session::new();
    session.actions.list = vec![child, parent];
    let original = session.actions.list.clone();
    let output = session.execute("actions.save", json!({})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(output["data"].as_str().unwrap()).unwrap();
    let mut imported = atn_cmds::import(&bytes).unwrap();
    atn_cmds::normalize_import(&mut imported, [], false).unwrap();
    assert_eq!(imported.sets[0].name, atn_cmds::DEFAULT_SET);
    assert_eq!(imported.actions[1].steps[0].1["set"], atn_cmds::DEFAULT_SET);
    assert_eq!(session.actions.list, original, "Save only retargets export copies");
}

#[test]
fn empty_library_source_can_accept_its_metadata_bearing_export_after_reopen() {
    let empty = encode(&ActionState::default());
    let mut session = crate::Session::new();
    session.actions = library::decode(&empty).unwrap();
    let mut action = Action::new("Metadata", vec![]);
    action.description = "kept".into();
    session.actions.list.push(action);
    let populated = encode(&session.actions);
    // Import retains the reopened empty source while adding a separately decoded row.
    let mut reopened = crate::Session::new();
    reopened.actions = library::decode(&empty).unwrap();
    reopened.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&populated)})).unwrap();
    let restored = library::decode(&encode(&reopened.actions)).unwrap();
    assert_eq!(restored.list[0].description, "kept");
}

#[test]
fn unencodable_legacy_library_remains_available_and_never_replaces_legacy_bytes() {
    let payload = "x".repeat(MAX_ACTIONS_BYTES as usize / 2 + 1024);
    let bytes = serde_json::to_vec(&json!({"version":1,"actions":[{"name":"Large legacy","steps":[["future.command",{"text":payload}]]}]})).unwrap();
    assert!(bytes.len() < MAX_ACTIONS_BYTES as usize);
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), bytes.clone());
    let opened = preset_store::open(Box::new(mem.clone()));
    assert!(matches!(opened.actions, preset_store::LoadedActions::MigrationBlocked(_)));
    assert_eq!(opened.actions[0].name, "Large legacy");
    assert!(opened.warnings.iter().any(|warning| warning.contains("migration blocked")));
    let mut session = crate::Session::new();
    session.attach_preset_store(opened);
    assert_eq!(session.actions.list[0].steps[0].1["text"].as_str().unwrap().len(), MAX_ACTIONS_BYTES as usize / 2 + 1024);
    session.sync_preset_store();
    assert_eq!(mem.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    assert!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
    assert!(mem.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).is_err());
}

#[test]
fn authoritative_atn_function_keys_survive_later_preference_hydration() {
    let mut files = atn::parse_sets(&close_archive()).unwrap();
    files[0].actions[0].function_key = 8;
    files[0].actions[0].shift = true;
    files[0].actions.push(AtnAction { name: UnicodeString::new_nul("No header"), ..Default::default() });
    let bytes = atn::write_sets(&files).unwrap();
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert(ACTIONS_FILE.into(), bytes.clone());
    let mut session = crate::Session::new();
    session.attach_preset_store(preset_store::open(Box::new(mem.clone())));
    let mut stale = crate::Session::new();
    stale.edit_prefs(|prefs| {
        prefs.shortcuts.insert(format!("{}Save and close", super::PLAY_SHORTCUT_PREFIX), "F2".into());
        prefs.shortcuts.insert(format!("{}No header", super::PLAY_SHORTCUT_PREFIX), "F3".into());
        prefs.shortcuts.insert(format!("{}Legacy", super::PLAY_SHORTCUT_PREFIX), "F4".into());
    });
    session.load_prefs_json(&serde_json::to_string(stale.prefs()).unwrap()).unwrap();
    session.reconcile_actions_function_keys();
    assert_eq!(session.prefs().shortcuts.get(&format!("{}Save and close", super::PLAY_SHORTCUT_PREFIX)).map(String::as_str), Some("Shift+F8"));
    assert!(!session.prefs().shortcuts.contains_key(&format!("{}No header", super::PLAY_SHORTCUT_PREFIX)));
    session.sync_preset_store();
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);

    let migrated = MemBackend::default();
    migrated.files.lock().unwrap().insert(LEGACY_ACTIONS_FILE.into(), legacy());
    let mut session = crate::Session::new();
    session.attach_preset_store(preset_store::open(Box::new(migrated)));
    session.load_prefs_json(&serde_json::to_string(stale.prefs()).unwrap()).unwrap();
    let hydrated = session.prefs().clone();
    session.reconcile_actions_function_keys();
    assert_eq!(session.prefs(), &hydrated, "migrated libraries retain preference-owned bindings");
}

#[test]
fn legacy_actions_with_trailing_bytes_cannot_mutate_the_live_library() {
    // With no descriptors or steps, the version12 and version16 record layouts agree.
    let mut valid = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Legacy input"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Incoming"), function_key: 7, ..Default::default() }],
        ..Default::default()
    })
    .unwrap();
    valid[..4].copy_from_slice(&12u32.to_be_bytes());
    let mut suffixed = valid.clone();
    suffixed.extend_from_slice(b"unparsed legacy suffix");

    let mut positive = crate::Session::new();
    positive.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&valid)})).unwrap();
    assert_eq!(positive.actions.list.len(), 1, "valid version12 remains accepted");
    assert_eq!(positive.actions.list[0].name, "Incoming");

    let mem = MemBackend::default();
    let mut state = ActionState { sets: vec!["Kept group".into()], ..Default::default() };
    let mut kept = Action::new("Kept", vec![("file.close".into(), json!({}))]);
    kept.set = Some("Kept group".into());
    state.list.push(kept);
    mem.files.lock().unwrap().insert(ACTIONS_FILE.into(), library::encode(&state, &[]).unwrap());
    let mut session = crate::Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    session.execute("file.new", json!({"width":8,"height":8})).unwrap();
    let key = format!("{}Kept", super::PLAY_SHORTCUT_PREFIX);
    session.edit_prefs(|prefs| {
        prefs.shortcuts.insert(key.clone(), "F2".into());
        prefs.alternate_shortcuts.insert(key.clone(), vec!["F3".into()]);
        prefs.shortcuts.insert("file.save".into(), "Ctrl+S".into());
    });
    session.sync_preset_store();
    assert!(session.preset_store.as_mut().unwrap().take_warnings().is_empty());
    let actions_before = session.actions.clone();
    let bytes_before = encode(&session.actions);
    let prefs_before = session.prefs().clone();
    let files_before = mem.files.lock().unwrap().clone();
    let document = session.active().unwrap();
    let dirty_before = document.is_dirty();
    let history_before = (document.history.past_len(), document.history.can_undo(), document.history.can_redo());

    let error = session.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&suffixed)})).unwrap_err();
    assert!(error.to_string().contains("trailing bytes after legacy action set"), "{error}");
    assert_eq!(session.actions, actions_before, "definitions, raw provenance and revisions stay intact");
    assert_eq!(encode(&session.actions), bytes_before, "durable library bytes stay intact");
    assert_eq!(session.prefs(), &prefs_before);
    let document = session.active().unwrap();
    assert_eq!(document.is_dirty(), dirty_before);
    assert_eq!((document.history.past_len(), document.history.can_undo(), document.history.can_redo()), history_before);
    assert!(session.preset_store.as_mut().unwrap().take_warnings().is_empty());
    assert_eq!(*mem.files.lock().unwrap(), files_before, "rejection writes no preset files");
    session.sync_preset_store();
    assert_eq!(*mem.files.lock().unwrap(), files_before, "rejection leaves no deferred preset write");
    assert_eq!(session.actions, actions_before, "sync cannot acknowledge a rejected definition");
    assert!(session.preset_store.as_mut().unwrap().take_warnings().is_empty());
}
