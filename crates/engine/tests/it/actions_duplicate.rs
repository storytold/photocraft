//! Duplicate keeps action identities, complete settings and per-step flags stable.
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::{Value, json};

#[test]
fn a_copied_group_keeps_numeric_call_identity_through_native_store_reload() {
    use photocraft_engine::preset_store::{self, MemBackend};
    let store = MemBackend::default();
    let mut s = session();
    assert!(s.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
    let mut first = Action::new("First", vec![("layer.new.layer".into(), json!({"name":"First marker"}))]);
    first.set = Some("Group".into());
    let mut second = Action::new("Second", vec![("layer.new.layer".into(), json!({"name":"Second marker"}))]);
    second.set = Some("Group".into());
    let mut caller = Action::new("Caller", vec![("actions.play".into(), json!({"action":1}))]);
    caller.set = Some("Other".into());
    s.actions.list = vec![first, second, caller];
    s.execute("actions.duplicate", json!({"set":"Group"})).unwrap();
    assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "First copy", "Second copy", "Caller"]);
    assert_eq!(s.actions.list[4].steps[0].1["action"], 1);
    let mut restored = Session::new();
    assert!(restored.attach_preset_store(preset_store::open(Box::new(store))).is_empty());
    assert_eq!(restored.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "First copy", "Second copy", "Caller"]);
    assert_eq!(restored.actions.list[4].steps[0].1["action"], 1);
    restored.execute("file.new", json!({"width":8,"height":6})).unwrap();
    let result = restored.execute("actions.play", json!({"action":"Caller"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(restored.active().unwrap().doc.layers.last().unwrap().name, "Second marker");
}

/// A tiny document to distinguish definition edits from document mutations and playback.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    s
}

#[test]
fn action_count_admission_reserves_the_metadata_carrier_before_publication() {
    for metadata in [false, true] {
        for operation in ["action", "set", "editor"] {
            let mut s = Session::new();
            let capacity = photocraft_psd::atn::MAX_TOTAL_ACTIONS - usize::from(metadata);
            s.actions.list = (0..capacity - 1).map(|index| Action::new(&format!("Action {index}"), vec![])).collect();
            s.actions.list[0].set = Some("Source".into());
            if metadata {
                s.actions.list[0].description = "Requires the hidden metadata action".into();
            }
            for admitted in [true, false] {
                let (command, params) = match operation {
                    "action" => ("actions.duplicate", json!({"action":0})),
                    "set" => ("actions.duplicate", json!({"set":"Source"})),
                    _ => {
                        let draft = s.execute("actions.editor.open", json!({"name":format!("New {admitted}")})).unwrap()["draft"].as_u64().unwrap();
                        s.execute("actions.editor.step", json!({"draft":draft,"op":"insert","step":0,"id":"select.all","params":{}})).unwrap();
                        ("actions.editor.save", json!({"draft":draft}))
                    }
                };
                let before = s.actions.clone();
                let result = s.execute(command, params.clone());
                assert_eq!(result.is_ok(), admitted, "{operation}, metadata={metadata}: {result:?}");
                if admitted {
                    assert_eq!(s.actions.list.len(), capacity);
                    if operation == "editor" {
                        s.execute("actions.editor.discard", json!({"draft":params["draft"]})).unwrap();
                    }
                } else {
                    assert_eq!(s.actions, before, "a full archive retains its list, rows and unpublished draft");
                }
            }
        }
    }
}

/// Copies retain metadata and call targets without taking the original function key.
#[test]
fn action_copies_follow_the_source_without_retargeting_calls_or_function_keys() {
    let mut s = session();
    let mut source = Action::new("Source", vec![("actions.play".into(), json!({"action": 1})), ("layer.select".into(), json!({"layer": "Photo"}))]);
    source.disabled.insert(1);
    source.dialogs.insert(0);
    source.set = Some("Retouch".into());
    source.color = "red".into();
    let foreign = json!({"actions": [{"name": "Foreign", "steps": [["actions.play", {"action": 1}]]}]});
    s.actions.list = vec![
        source.clone(),
        Action::new("Target", vec![("select.all".into(), json!({}))]),
        Action::new(
            "Caller",
            vec![
                ("actions.play".into(), json!({"action": 1})),
                ("actions.play".into(), json!({"action": 3})), // Unresolved before and after insertion.
                ("actions.play".into(), json!({"action": 1.0})),
                ("file.automate.batch".into(), foreign.clone()),
                ("actions.conditional".into(), json!({"if": "always", "then": "Target", "else": "Source"})),
            ],
        ),
    ];
    s.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:Source".into(), "F6".into());
    });
    let revision = s.active().unwrap().revision;
    let result = s.execute("actions.duplicate", json!({"action": "Source"})).unwrap();
    assert_eq!(result["index"], 1);
    assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Source", "Source copy", "Target", "Caller"]);
    let mut expected = source;
    expected.name = "Source copy".into();
    expected.steps[0].1["action"] = json!(2);
    assert_eq!(s.actions.list[1], expected);
    assert_eq!(s.actions.list[0].steps[0].1["action"], 2);
    let caller = &s.actions.list[3];
    assert_eq!(caller.steps[0].1["action"], 2);
    assert_eq!(caller.steps[1].1["action"], 4);
    assert_eq!(caller.steps[2].1["action"], 2.0);
    assert_eq!(caller.steps[3].1, foreign, "embedded libraries keep their own indices");
    assert_eq!(caller.steps[4].1, json!({"if": "always", "then": "Target", "else": "Source"}));
    assert_eq!(s.prefs().shortcuts.get("actions.play:Source").map(String::as_str), Some("F6"));
    assert!(!s.prefs().shortcuts.contains_key("actions.play:Source copy"));
    assert_eq!(s.active().unwrap().revision, revision);
    let played = s.execute("actions.play", json!({"action": "Source copy"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(s.active().unwrap().doc.selection.is_some(), "the copied call still plays Target rather than itself");
    s.execute("actions.duplicate", json!({"action": 0})).unwrap();
    assert_eq!(s.actions.list[1].name, "Source copy 2");
    assert_eq!(s.actions.list[2].name, "Source copy");
    assert_eq!(s.actions.list[1].steps[0].1["action"], 3);
}

/// Step copies own their settings and retain both flag columns after insertion.
#[test]
fn step_copies_preserve_payloads_and_shift_both_flag_columns() {
    let mut s = session();
    let payload = json!({"layer": "Photo", "_actionLayer": {"name": "Photo", "ordinal": 2}});
    let mut a = Action::new("Steps", vec![("layer.select".into(), payload.clone()), ("select.all".into(), Value::Null), ("select.deselect".into(), json!({}))]);
    a.disabled.extend([0, 2]);
    a.dialogs.extend([0, 1]);
    s.actions.list.push(a);
    let revision = s.active().unwrap().revision;
    let result = s.execute("actions.duplicate", json!({"action": 0, "step": 0})).unwrap();
    assert_eq!(result["step"], 1);
    assert_eq!(result["index"], 0);
    let a = &s.actions.list[0];
    assert_eq!(a.name, "Steps");
    assert_eq!(a.steps[0], a.steps[1]);
    assert_eq!(a.disabled.iter().copied().collect::<Vec<_>>(), [0, 1, 3]);
    assert_eq!(a.dialogs.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
    s.execute("actions.duplicate", json!({"action": 0, "step": 2})).unwrap();
    assert!(s.actions.list[0].steps[2].1.is_null());
    assert!(s.actions.list[0].steps[3].1.is_null(), "copying does not normalize null settings to an empty object");
    s.actions.list[0].steps[1].1["layer"] = json!(99);
    assert_eq!(s.actions.list[0].steps[0].1, payload, "the copy owns its settings");
    assert_eq!(s.active().unwrap().revision, revision);
}

/// Refusals preserve pending recording; valid step copies keep recording usable.
#[test]
fn invalid_copies_do_not_flush_recording_and_valid_step_copies_keep_recording() {
    let mut s = session();
    s.execute("actions.record", json!({"name": "Recording"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let before = s.actions.list.clone();
    let recording = s.actions.recording;
    let revision = s.active().unwrap().revision;
    for params in [
        json!({}),
        json!({"action": 9}),
        json!({"action": 0}),
        json!({"action": 0, "step": -1}),
        json!({"action": 0, "step": 5}),
        json!({"action": 0, "step": "x"}),
    ] {
        assert!(s.execute("actions.duplicate", params.clone()).is_err(), "{params}");
        assert_eq!(s.actions.list, before);
        assert_eq!(s.actions.recording, recording);
        assert_eq!(s.active().unwrap().revision, revision);
    }
    s.execute("actions.duplicate", json!({"action": 0, "step": 0})).unwrap();
    assert_eq!(s.actions.list[0].steps, vec![("select.all".into(), json!({})); 2]);
    assert!(s.actions.recording.is_some());
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps.len(), 3);
    s.actions.list.push(Action::new("Edit during playback", vec![("actions.duplicate".into(), json!({"action": 0}))]));
    let before = s.actions.list.clone();
    let played = s.execute("actions.play", json!({"action": "Edit during playback"})).unwrap();
    assert!(played["failed"]["error"].as_str().is_some_and(|e| e.contains("playback")), "{played}");
    assert_eq!(s.actions.list, before);
}
