//! Folder edits are atomic definition changes, preserving surviving call identities.
use photocraft_engine::{
    Session,
    actions_cmds::{Action, playback},
};
use serde_json::{Value, json};

fn grouped(name: &str, set: &str) -> Action {
    Action { set: Some(set.into()), ..Action::new(name, Vec::new()) }
}

#[test]
fn delete_group_removes_contents_keys_and_retargets_only_surviving_local_calls() {
    let mut s = Session::new();
    let foreign = json!({"actions":[{"steps":[["actions.play",{"action":1}]]}]});
    s.actions.list = vec![
        grouped("Removed one", "Group"),
        grouped("Survivor", "Other"),
        grouped("Removed two", "Group"),
        Action::new(
            "Caller",
            vec![
                ("actions.play".into(), json!({"action":0})),
                ("actions.play".into(), json!({"action":1.2})),
                ("actions.play".into(), json!({"action":2})),
                ("actions.play".into(), json!({"action":3})),
                ("actions.play".into(), json!({"action":99})),
                ("actions.play".into(), json!({"action":"Removed two"})),
                ("actions.conditional".into(), json!({"if":"always","then":"Removed one","else":"Survivor"})),
                ("file.automate.batch".into(), foreign.clone()),
            ],
        ),
    ];
    s.actions.sets = vec!["Group".into(), "Empty".into()];
    s.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:Removed one".into(), "F2".into());
        p.alternate_shortcuts.insert("actions.play:Removed two".into(), vec!["F3".into()]);
        p.shortcuts.insert("actions.play:Survivor".into(), "F4".into());
    });
    let result = s.execute("actions.delete", json!({"set":"Group"})).unwrap();
    assert_eq!(result, json!({"deletedSet":"Group","deleted":["Removed one","Removed two"]}));
    assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Survivor", "Caller"]);
    assert_eq!(s.actions.sets, ["Empty"]);
    let steps = &s.actions.list[1].steps;
    assert!(steps[0].1["action"].is_null());
    assert_eq!(steps[1].1["action"], 0);
    assert!(steps[2].1["action"].is_null());
    assert_eq!(steps[3].1["action"], 1);
    assert_eq!(steps[4].1["action"], 99);
    assert!(steps[5].1["action"].is_null());
    assert_eq!(steps[6].1, json!({"if":"always","then":null,"else":"Survivor"}));
    assert_eq!(steps[7].1, foreign);
    assert!(!s.prefs().shortcuts.contains_key("actions.play:Removed one"));
    assert!(!s.prefs().alternate_shortcuts.contains_key("actions.play:Removed two"));
    assert_eq!(s.prefs().shortcuts["actions.play:Survivor"], "F4");
    // A later reuse of either a name or an index must not revive a deleted call.
    s.actions.list.push(Action::new("Removed one", Vec::new()));
    assert!(s.execute("actions.play", json!({"action":Value::Null})).is_err());
    assert_eq!(s.execute("actions.delete", json!({"set":"Empty"})).unwrap()["deleted"], json!([]));
    assert!(s.actions.sets.is_empty());
}

#[test]
fn invalid_group_delete_and_recording_or_preview_guards_leave_definitions_unchanged() {
    let mut s = Session::new();
    s.actions.list.push(grouped("A", "Group"));
    s.actions.sets.push("Empty".into());
    for params in [json!({"set":null}), json!({"set":2}), json!({"set":"Missing"}), json!({"set":"Group","action":0}), json!({"set":"Group","step":0})] {
        let before = s.actions.clone();
        assert!(s.execute("actions.delete", params).is_err());
        assert_eq!(s.actions, before);
    }
    s.execute("actions.record", json!({"action":"A"})).unwrap();
    let before = s.actions.clone();
    assert!(s.execute("actions.delete", json!({"set":"Group"})).is_err());
    assert_eq!(s.actions, before);
    s.execute("actions.stop", json!({})).unwrap();
    let id = s.execute("actions.editor.open", json!({"action":"A"})).unwrap()["draft"].as_u64().unwrap();
    s.execute("actions.editor.record", json!({"draft":id})).unwrap();
    let before = s.actions.clone();
    for command in ["actions.delete", "actions.duplicate"] {
        assert!(s.execute(command, json!({"set":"Group"})).is_err());
        assert_eq!(s.actions, before);
    }
    let (_, owner) = playback::preview(&mut s, id, 0).unwrap();
    let before = s.actions.clone();
    for command in ["actions.delete", "actions.duplicate"] {
        assert!(s.execute(command, json!({"set":"Group"})).is_err());
        assert_eq!(s.actions, before);
    }
    owner.finish(&mut s);
    assert_eq!(s.execute("actions.editor.get", json!({"draft":id})).unwrap()["recording"], true);
}

#[test]
fn duplicate_group_preserves_complete_actions_and_keeps_original_shortcuts() {
    let mut s = Session::new();
    let mut source = grouped("A", "Group");
    source.icon = "FutureIcon".into();
    source.description = "Retain metadata".into();
    source.steps.push(("actions.play".into(), json!({"action":1})));
    source.disabled.insert(0);
    source.dialogs.insert(0);
    s.actions.list = vec![source.clone(), grouped("Target", "Other")];
    s.actions.sets.push("Empty".into());
    s.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:A".into(), "F5".into());
    });
    let result = s.execute("actions.duplicate", json!({"set":"Group"})).unwrap();
    assert_eq!(result["set"], "Group copy");
    source.name = "A copy".into();
    source.set = Some("Group copy".into());
    source.steps[0].1["action"] = json!(2);
    assert_eq!(s.actions.list[1], source);
    assert_eq!(s.prefs().shortcuts["actions.play:A"], "F5");
    assert!(!s.prefs().shortcuts.contains_key("actions.play:A copy"));
    assert_eq!(s.execute("actions.duplicate", json!({"set":"Empty"})).unwrap()["actions"], json!([]));
    assert!(s.actions.sets.iter().any(|name| name == "Empty copy"));
}

#[test]
fn ungrouped_group_commands_share_one_identity_for_legacy_none_and_explicit_category() {
    let mut s = Session::new();
    s.actions.list = vec![Action::new("Legacy", Vec::new()), grouped("Explicit", photocraft_engine::atn_cmds::UNGROUPED_SET), grouped("Other", "Named")];
    let groups = photocraft_engine::actions_edit_cmds::set_names(&s);
    assert_eq!(groups, [photocraft_engine::atn_cmds::UNGROUPED_SET, "Named"]);
    let copied = s.execute("actions.duplicate", json!({"set":photocraft_engine::atn_cmds::UNGROUPED_SET})).unwrap();
    assert_eq!(copied["actions"].as_array().unwrap().len(), 2);
    let deleted = s.execute("actions.delete", json!({"set":photocraft_engine::atn_cmds::UNGROUPED_SET})).unwrap();
    assert_eq!(deleted["deleted"], json!(["Legacy", "Explicit"]));
    assert_eq!(s.actions.list.len(), 3);
    assert!(!photocraft_engine::actions_edit_cmds::set_names(&s).contains(&photocraft_engine::atn_cmds::UNGROUPED_SET.to_string()));
}
