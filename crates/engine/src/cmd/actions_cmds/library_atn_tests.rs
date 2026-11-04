//! Codec and migration tests never execute the commands carried by untrusted action files.
use super::{Action, ActionState, library};
use crate::{
    atn_cmds,
    preset_store::{self, ACTIONS_FILE, LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES, MIGRATED_ACTIONS_FILE, MemBackend, PresetBackend},
};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use serde_json::json;

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
    assert!(library::decode(&16u32.to_be_bytes()).is_err());
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
