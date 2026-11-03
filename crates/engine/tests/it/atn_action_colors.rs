//! Action-header colours survive the public ATN and native action-library paths.
use photocraft_engine::{
    Session,
    actions_cmds::{Action, ActionsFile},
    atn_cmds,
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, UnicodeString, Value as D},
};
use serde_json::json;

const SET: &str = "Colour set";
const NAME: &str = "Colour action";

fn recorded(color: u16) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul(SET),
        expanded: true,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul(NAME),
            function_key: 6,
            shift: true,
            command: true,
            color,
            items: vec![AtnItem {
                event: AtnEvent::name("futureColourFixture"),
                dictionary: b"Colour fixture".to_vec(),
                enabled: false,
                with_dialog: true,
                dialog_options: 2,
                descriptor: Some(Descriptor::new("null").with("data", D::RawData(vec![3, 1, 4])).with("test", D::Integer(7))),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn load(s: &mut Session, color: u16) {
    let bytes = atn::write(&recorded(color)).unwrap();
    let data = photocraft_engine::paint::tile::b64_encode(&bytes);
    s.execute("actions.load", json!({"data": data})).unwrap();
}

fn saved(s: &mut Session) -> AtnFile {
    let result = s.execute("actions.save", json!({"set": SET})).unwrap();
    assert_eq!(result["skipped"], json!([]));
    let bytes = photocraft_engine::paint::tile::b64_decode(result["data"].as_str().unwrap()).unwrap();
    atn::parse(&bytes).unwrap()
}

fn same_step_wire(actual: &AtnItem, expected: &AtnItem) {
    assert_eq!(actual, expected);
    assert_eq!(actual.descriptor.as_ref().unwrap().to_bytes(), expected.descriptor.as_ref().unwrap().to_bytes());
}

/// Save through the user-facing commands before checking the imported colour, exposing real wire loss.
#[test]
fn recorded_red_survives_public_load_and_save_without_other_edits() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    let original_shortcuts = s.prefs().shortcuts.clone();
    load(&mut s, 1);
    let actions = s.actions.list.clone();
    let revision = s.actions.rev;
    let shortcuts = s.prefs().shortcuts.clone();
    let output = saved(&mut s);
    assert_eq!(output.actions[0].color, 1, "public Load/Save Actions discarded the red header colour");
    assert_eq!(s.execute("actions.get", json!({"action": NAME})).unwrap()["color"], "red");
    let expected = recorded(1);
    assert_eq!(output, expected);
    same_step_wire(&output.actions[0].items[0], &expected.actions[0].items[0]);
    assert_eq!(atn_cmds::import(&atn::write(&output).unwrap()).unwrap().actions, actions);
    assert_eq!(s.actions.list, actions);
    assert_eq!(s.actions.rev, revision);
    assert_eq!(s.prefs().shortcuts, shortcuts);
    assert_eq!(shortcuts[&format!("{}{NAME}", photocraft_engine::actions_cmds::PLAY_SHORTCUT_PREFIX)], "Cmd+Shift+F6");
    for (key, value) in original_shortcuts {
        assert_eq!(shortcuts.get(&key), Some(&value));
    }
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

/// Expectations come from the measured header indices, including the distinct classic and new blues.
#[test]
fn all_twelve_header_colours_and_named_edits_keep_their_indices() {
    for (index, name) in [
        (0, ""),
        (1, "red"),
        (2, "orange"),
        (3, "yellow"),
        (4, "green"),
        (5, "blue"),
        (6, "violet"),
        (7, "gray"),
        (8, "teal"),
        (9, "blue2"),
        (10, "magenta"),
        (11, "purple"),
    ] {
        let mut s = Session::new();
        load(&mut s, index);
        assert_eq!(s.execute("actions.get", json!({"action": NAME})).unwrap()["color"], name, "index {index}");
        assert_eq!(saved(&mut s).actions[0].color, index);
        s.execute("actions.setColor", json!({"action": NAME, "color": name})).unwrap();
        assert_eq!(saved(&mut s).actions[0].color, index);
        if index == 5 {
            s.execute("actions.setColor", json!({"action": NAME, "color": "blue2"})).unwrap();
            assert_eq!(saved(&mut s).actions[0].color, 9);
            s.execute("actions.setColor", json!({"action": NAME, "color": "blue"})).unwrap();
            assert_eq!(saved(&mut s).actions[0].color, 5);
        }
    }
}

/// Unknown header values remain opaque through real native-library writes, reloads and a rename.
#[test]
fn unknown_header_colours_survive_native_persistence_and_unrelated_edits() {
    for index in [12, u16::MAX] {
        let mem = MemBackend::default();
        let mut s = Session::new();
        assert!(s.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
        load(&mut s, index);
        let opaque = format!("@atn-color:{index}");
        assert_eq!(s.actions.list[0].color, opaque);
        s.sync_preset_store();
        let bytes = mem.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
        let native: ActionsFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(native.actions, s.actions.list);
        let mut reopened = Session::new();
        assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
        assert_eq!(reopened.actions.list, native.actions);
        reopened.execute("actions.rename", json!({"action": NAME, "name": "Renamed"})).unwrap();
        assert_eq!(reopened.actions.list[0].color, opaque);
        reopened.sync_preset_store();
        let mut again = Session::new();
        assert!(again.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
        assert_eq!(again.actions.list, reopened.actions.list);
        let output = saved(&mut again);
        assert_eq!(output.actions[0].color, index);
        assert_eq!(output.actions[0].name.to_string_lossy(), "Renamed");
        same_step_wire(&output.actions[0].items[0], &recorded(index).actions[0].items[0]);
    }
}

/// Reserved metadata cannot be chosen as a palette colour; malformed JSON colours never become none.
#[test]
fn invalid_colour_edits_and_exports_leave_the_library_unchanged() {
    let mut s = Session::new();
    load(&mut s, 1);
    for color in ["@atn-color:12", "@atn-color:65535", "unknown", "@atn-color:012", "@atn-color:65536"] {
        let before = s.actions.list.clone();
        let revision = s.actions.rev;
        let shortcuts = s.prefs().shortcuts.clone();
        assert!(s.execute("actions.setColor", json!({"action": NAME, "color": color})).is_err(), "{color}");
        assert_eq!(s.actions.list, before);
        assert_eq!(s.actions.rev, revision);
        assert_eq!(s.prefs().shortcuts, shortcuts);
    }
    for color in [
        "unknown",
        "@atn-color:",
        "@atn-color:0",
        "@atn-color:11",
        "@atn-color:012",
        "@atn-color:+12",
        "@atn-color:-12",
        "@atn-color:65536",
        "@atn-color:12 ",
        "@atn-color:１２",
        "@atn-color:12.0",
    ] {
        let action: Action = serde_json::from_value(json!({"name": NAME, "steps": [], "color": color})).unwrap();
        let original = action.clone();
        for skip in [false, true] {
            assert!(atn_cmds::export(SET, &[&action], &[], skip).is_err(), "{color}, skipUnsupported={skip}");
            assert_eq!(action, original);
        }
    }
    let old: Action = serde_json::from_value(json!({"name": "Old", "steps": []})).unwrap();
    assert!(old.color.is_empty());
    let (bytes, skipped) = atn_cmds::export(SET, &[&old], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].color, 0);
}
