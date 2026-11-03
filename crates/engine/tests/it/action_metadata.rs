//! Action metadata compatibility, editor staging and independent ATN sidecar output checks.
use photocraft_engine::{
    Session,
    actions_cmds::{Action, ActionsFile},
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use serde_json::json;

const DESCRIPTION: &str = "  First line\nCafé 🌿\0  ";
const UNKNOWN_ICON: &str = " future/editor-icon-图标 ";

/// Independent PSD descriptor oracle: inspect the emitted sidecar without the Engine's
/// importer or sidecar decoder agreeing with its own writer.
fn assert_exported_metadata(bytes: &[u8], icon: &str) {
    use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    let files = photocraft_psd::atn::parse_sets(bytes).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].name.to_string_lossy(), "Metadata set");
    assert_eq!(files[0].actions.len(), 1);
    assert_eq!(files[0].actions[0].name.to_string_lossy(), "Editable");
    let wrapper = files[1].actions[0].items[0].descriptor.as_ref().unwrap();
    let Some(D::RawData(payload)) = wrapper.get("modernizedActionData") else { panic!("sidecar raw payload missing") };
    assert_eq!(payload.get(..4), Some(16u32.to_be_bytes().as_slice()));
    let root = Descriptor::from_bytes(payload.get(4..).unwrap()).unwrap();
    let Some(D::List(sets)) = root.get("sets") else { panic!("sidecar sets missing") };
    assert_eq!(sets.len(), 1);
    let D::Descriptor(set) = &sets[0] else { panic!("set descriptor expected") };
    let Some(D::List(actions)) = set.get("actions") else { panic!("sidecar actions missing") };
    assert_eq!(actions.len(), 1);
    let D::Descriptor(action) = &actions[0] else { panic!("action descriptor expected") };
    assert_eq!(action.get("actionDescriptionKey"), Some(&D::Text(UnicodeString::new(DESCRIPTION))));
    assert_eq!(action.get("actionIconKey"), Some(&D::Text(UnicodeString::new(icon))));
    assert_eq!(action.get("actionThumbnailTypeKey"), Some(&D::Integer(3)));
    assert_eq!(action.get("actionPreconditionVal"), Some(&D::Integer(-1)));
    assert_eq!(action.get("actionPreviewKey"), Some(&D::Boolean(false)));
    assert_eq!(action.get("actionGenerativeKey"), Some(&D::Boolean(false)));
}

fn described(name: &str, icon: &str) -> Action {
    let mut action = Action::new(name, vec![("future.step".into(), json!({"raw": [0, 255], "nested": {"keep": true}}))]);
    action.description = DESCRIPTION.into();
    action.icon = icon.into();
    action.set = Some("Metadata set".into());
    action.color = "@atn-color:65535".into();
    action.disabled.insert(0);
    action.dialogs.insert(0);
    action
}

#[test]
fn old_native_library_defaults_metadata_without_rewriting_on_attach() {
    let legacy = br#"{"version":1,"actions":[{"name":"Legacy","steps":[["future.step",{"raw":[0,255]}]]}]}"#;
    let mem = MemBackend::default();
    mem.write(ACTIONS_FILE, legacy).unwrap();
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let action = &session.actions.list[0];
    assert!(action.description.is_empty());
    assert!(action.icon.is_empty());
    assert_eq!(*action, Action::new("Legacy", vec![("future.step".into(), json!({"raw": [0, 255]}))]));
    assert_eq!(serde_json::to_value(action).unwrap(), json!({"name": "Legacy", "steps": [["future.step", {"raw": [0, 255]}]]}));
    assert!(Action::default().description.is_empty());
    assert!(Action::default().icon.is_empty());
    session.sync_preset_store();
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap().as_slice(), legacy);
}

#[test]
fn native_json_preserves_descriptions_and_known_or_unknown_icon_identifiers() {
    for icon in ["ClickAction", UNKNOWN_ICON] {
        let original = described("Described", icon);
        let file = ActionsFile { version: 1, actions: vec![original.clone()] };
        let bytes = serde_json::to_vec_pretty(&file).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(stored["actions"][0]["description"], DESCRIPTION);
        assert_eq!(stored["actions"][0]["icon"], icon);
        let reopened: ActionsFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reopened, file);
        assert_eq!(reopened.actions[0].description, DESCRIPTION);
        assert_eq!(reopened.actions[0].icon, icon);
        assert_eq!(original.description, DESCRIPTION);
        assert_eq!(original.icon, icon);
    }
}

#[test]
fn cloned_actions_keep_metadata_through_rename_and_real_preset_reload() {
    let initial = ActionsFile { version: 1, actions: vec![described("Known", "ClickAction"), described("Unknown", UNKNOWN_ICON)] };
    let mem = MemBackend::default();
    mem.write(ACTIONS_FILE, &serde_json::to_vec_pretty(&initial).unwrap()).unwrap();
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    assert_eq!(session.actions.list, initial.actions);
    // Exercise the model clone here; native duplication has its own regression tests.
    let mut copy = session.actions.list[1].clone();
    copy.name = "Copy".into();
    assert_eq!(copy.description, DESCRIPTION);
    assert_eq!(copy.icon, UNKNOWN_ICON);
    session.actions.list.push(copy);
    for (old, new) in [("Known", "Renamed known"), ("Unknown", "Renamed unknown"), ("Copy", "Renamed copy")] {
        session.execute("actions.rename", json!({"action": old, "name": new})).unwrap();
    }
    for (action, icon) in session.actions.list.iter().zip(["ClickAction", UNKNOWN_ICON, UNKNOWN_ICON]) {
        assert_eq!(action.description, DESCRIPTION);
        assert_eq!(action.icon, icon);
        assert_eq!(action.steps, initial.actions[0].steps);
        assert_eq!(action.disabled, initial.actions[0].disabled);
        assert_eq!(action.dialogs, initial.actions[0].dialogs);
        assert_eq!(action.set, initial.actions[0].set);
        assert_eq!(action.color, initial.actions[0].color);
    }
    session.sync_preset_store();
    let bytes = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let saved: ActionsFile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved.version, 1);
    assert_eq!(saved.actions, session.actions.list);
    assert_eq!(saved.actions.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["Renamed known", "Renamed unknown", "Renamed copy"]);
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    assert_eq!(reopened.actions.list, saved.actions);
    assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
}

/// The combined editor API stages metadata privately and publishes it only through Save.
#[test]
fn editor_metadata_stays_draft_until_save_and_discard_keeps_saved_values() {
    for icon in ["ClickAction", UNKNOWN_ICON] {
        let mut original = Action::new("Editable", Vec::new());
        let initial = ActionsFile { version: 1, actions: vec![original.clone()] };
        let mem = MemBackend::default();
        mem.write(ACTIONS_FILE, &serde_json::to_vec_pretty(&initial).unwrap()).unwrap();
        let mut session = Session::new();
        assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
        let committed = session.actions.list.clone();
        let original_bytes = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
        let opened = session.execute("actions.editor.open", json!({"action": "Editable"})).unwrap();
        let draft = opened["draft"].as_u64().unwrap();
        let staged = session.execute("actions.editor.update", json!({"draft": draft, "description": DESCRIPTION, "icon": icon})).unwrap();
        assert_eq!(staged["action"]["description"], DESCRIPTION);
        assert_eq!(staged["action"]["icon"], icon);
        assert_eq!(staged["dirty"], true);
        assert_eq!(session.execute("actions.editor.get", json!({"draft": draft})).unwrap(), staged);
        assert_eq!(session.actions.list, committed);
        assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), original_bytes);
        for invalid in [
            json!({"draft": draft, "description": false, "icon": "Must not replace the icon"}),
            json!({"draft": draft, "description": "Must not replace the description", "icon": 7}),
        ] {
            assert!(session.execute("actions.editor.update", invalid).is_err());
            assert_eq!(session.execute("actions.editor.get", json!({"draft": draft})).unwrap(), staged);
            assert_eq!(session.actions.list, committed);
            assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), original_bytes);
        }
        let saved = session.execute("actions.editor.save", json!({"draft": draft})).unwrap();
        assert_eq!(saved["dirty"], false);
        original.description = DESCRIPTION.into();
        original.icon = icon.into();
        assert_eq!(session.actions.list, vec![original.clone()]);
        let classic = session.execute("actions.get", json!({"action": "Editable"})).unwrap();
        assert_eq!(classic["description"], DESCRIPTION);
        assert_eq!(classic["icon"], icon);
        session.sync_preset_store();
        let saved_bytes = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
        let native: ActionsFile = serde_json::from_slice(&saved_bytes).unwrap();
        assert_eq!(native.actions, vec![original.clone()]);
        let mut reopened = Session::new();
        assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
        assert_eq!(reopened.actions.list, native.actions);
        // Metadata now exports through the real sidecar, including an unknown glyph identifier.
        let committed = session.actions.list.clone();
        let mut plain = original.clone();
        plain.description.clear();
        plain.icon.clear();
        for skip in [false, true] {
            let exported = session.execute("actions.save", json!({"action": "Editable", "set": "Metadata set", "skipUnsupported": skip})).unwrap();
            assert!(exported["skipped"].as_array().unwrap().is_empty());
            let bytes = photocraft_engine::paint::tile::b64_decode(exported["data"].as_str().unwrap()).unwrap();
            assert_exported_metadata(&bytes, icon);
            let (bytes, skipped) = photocraft_engine::atn_cmds::export("Metadata set", &[&plain], &[], skip).unwrap();
            assert!(!bytes.is_empty());
            assert!(skipped.is_empty());
            assert_eq!(session.actions.list, committed);
            assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), saved_bytes);
        }
        let mut unsupported = original.clone();
        unsupported.steps.push(("future.step".into(), json!({"raw": [0,255]})));
        assert!(photocraft_engine::atn_cmds::export("Metadata set", &[&unsupported], &[], false).is_err());
        let (bytes, skipped) = photocraft_engine::atn_cmds::export("Metadata set", &[&unsupported], &[], true).unwrap();
        assert_eq!(skipped.len(), 1, "skipUnsupported affects steps, never metadata");
        assert_exported_metadata(&bytes, icon);
        session.execute("actions.editor.update", json!({"draft": draft, "description": "Discard me", "icon": "UncommittedIcon"})).unwrap();
        session.execute("actions.editor.discard", json!({"draft": draft})).unwrap();
        assert_eq!(session.actions.list, vec![original]);
        assert_eq!(mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), saved_bytes);
    }
}
