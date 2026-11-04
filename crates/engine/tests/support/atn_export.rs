//! Assertions for the shared ATN library's native fallback and opt-in Photoshop omission.
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds,
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use photocraft_psd::{
    atn::{self, AtnItem},
    descriptor::Value as D,
};

/// Persist and reopen every native ID and JSON value, then check the portable export/import contract.
pub fn preserved(set: &str, action: &Action) -> Vec<AtnItem> {
    let store = MemBackend::default();
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
    let mut stored = action.clone();
    stored.set = Some(set.into());
    session.actions.list.push(stored);
    session.actions.rev += 1;
    session.sync_preset_store();
    let bytes = store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let mut loaded = Session::new();
    assert!(loaded.attach_preset_store(preset_store::open(Box::new(store))).is_empty());
    assert_eq!(loaded.actions.list[0].steps, action.steps);
    // Admission is tested independently with known forbidden IDs. This shared helper also
    // requires export and import to agree, without treating portable export as local storage.
    let bytes = match atn_cmds::import(&bytes) {
        Ok(_) => {
            let (portable, skipped) = atn_cmds::export(set, &[action], &[], false).unwrap();
            assert!(skipped.is_empty());
            assert_eq!(atn_cmds::import(&portable).unwrap().actions[0].steps, action.steps);
            portable
        }
        Err(error) => {
            assert!(error.contains("not permitted in a portable Actions file"), "{error}");
            assert_eq!(atn_cmds::export(set, &[action], &[], false).unwrap_err(), error);
            bytes
        }
    };
    atn::parse_sets(&bytes)
        .unwrap()
        .into_iter()
        .flat_map(|file| file.actions)
        .find(|record| record.name.to_string_lossy() == action.name)
        .unwrap()
        .items
}

/// A one-step command cannot be emitted as a lossy Photoshop approximation.
pub fn fallback(set: &str, action: &Action) {
    assert_eq!(action.steps.len(), 1);
    let items = preserved(set, action);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].event, atn::AtnEvent::name("photocraftCommand"));
    let descriptor = items[0].descriptor.as_ref().unwrap();
    let Some(D::Text(command)) = descriptor.get("command") else { panic!("missing command TEXT") };
    let Some(D::Text(params)) = descriptor.get("params") else { panic!("missing params TEXT") };
    assert_eq!(command.to_string_lossy(), action.steps[0].0);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&params.to_string_lossy()).unwrap(), action.steps[0].1);
    let (_, skipped) = atn_cmds::export(set, &[action], &[], true).unwrap();
    assert_eq!(skipped, [atn_cmds::Skipped { action: action.name.clone(), step: 0, id: action.steps[0].0.clone() }]);
}
