//! Native portable-file eligibility and lossless local library storage are distinct contracts.
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds,
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use serde_json::json;

/// Serialize through the actual durable writer, including its native fallback.
fn encode(action: &Action) -> Vec<u8> {
    let store = MemBackend::default();
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
    session.actions.list.push(action.clone());
    session.actions.rev += 1;
    session.sync_preset_store();
    store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap()
}

/// Portable Actions files cannot install code or run script wrappers; local bytes remain lossless.
#[test]
fn portable_native_admission_preserves_library_bytes_but_refuses_nonrecordable_effects() {
    for id in ["plugin.install", "file.scripts.browse", "select.isolateLayers"] {
        let action = Action::new("Untrusted", vec![(id.into(), json!({"path":"never-opened.wasm","data":"never-installed"}))]);
        let bytes = encode(&action);
        let store = MemBackend::default();
        store.write(ACTIONS_FILE, &bytes).unwrap();
        let mut reopened = Session::new();
        assert!(reopened.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
        assert_eq!(reopened.actions.list[0].steps, action.steps, "local library remains a complete store");
        assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
        let export_error = atn_cmds::export("Portable", &[&action], &[], false).unwrap_err();
        assert!(export_error.contains(id) && export_error.contains("not permitted"), "{export_error}");
        let (_, skipped) = atn_cmds::export("Portable", &[&action], &[], true).unwrap();
        assert_eq!(skipped, vec![atn_cmds::Skipped { action: "Untrusted".into(), step: 0, id: id.into() }]);
        let before_save = reopened.actions.clone();
        let save_error = reopened.execute("actions.save", json!({"action":0})).unwrap_err().to_string();
        assert!(save_error.contains(id) && save_error.contains("not permitted"), "{save_error}");
        assert_eq!(reopened.actions, before_save, "refused portable export leaves local definitions intact");
        assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
        let error = atn_cmds::import(&bytes).unwrap_err();
        assert!(error.contains(id) && error.contains("not permitted"), "{error}");
        let mut session = Session::new();
        let before = session.actions.clone();
        assert!(session.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).is_err());
        assert_eq!(session.actions, before, "refusal occurs before installing any action");
    }
    for (id, params) in [("filter.distort.ripple", json!({"amount":1.5,"size":"medium"})), ("file.save", json!({"path":"recorded.png","nativeOption":true}))] {
        let action = Action::new("Recorded", vec![(id.into(), params)]);
        let bytes = encode(&action);
        let imported = atn_cmds::import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, action.steps, "required native command+JSON and legitimate saves remain usable");
    }
    let future = Action::new("Future", vec![("future.unregisteredNative".into(), json!({"keep":[1,true,"opaque"]}))]);
    let (portable, skipped) = atn_cmds::export("Portable", &[&future], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn_cmds::import(&portable).unwrap().actions[0].steps, future.steps);
}
