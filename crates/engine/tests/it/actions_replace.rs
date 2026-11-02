//! Whole-library replacement shares import semantics without clearing before validation.
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::{Value, json};

/// A caller, its named target and an action sharing an old name expose reference/key ownership.
fn input() -> Vec<u8> {
    let mut caller = Action::new("Caller", vec![("actions.play".into(), json!({"action": "NewChild"})), ("select.deselect".into(), json!({}))]);
    caller.disabled.insert(1);
    caller.dialogs.insert(0);
    let child = Action::new("NewChild", vec![("select.all".into(), json!({}))]);
    let reused = Action::new("OldA", vec![("select.deselect".into(), json!({}))]);
    atn_cmds::export("Incoming", &[&caller, &child, &reused], &[("NewChild".into(), "F8".into())], false).unwrap().0
}

/// Two old sets and owned/unloaded/global bindings distinguish whole from same-set replacement.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("actions.newSet", json!({"name": "Old Empty"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    for (name, set) in [("OldA", "Incoming"), ("OldB", "Other")] {
        let mut a = Action::new(name, vec![("select.deselect".into(), json!({}))]);
        a.set = Some(set.into());
        s.actions.list.push(a);
    }
    s.prefs.edit(|p| {
        for (id, key) in [("actions.play:OldA", "F5"), ("actions.play:OldB", "F6"), ("actions.play:Unloaded", "F9"), ("view.zoomIn", "Cmd+0")] {
            p.shortcuts.insert(id.into(), key.into());
        }
    });
    s
}

/// Same-set Load keeps other sets; confirmed Replace swaps all definitions and their keys.
#[test]
fn replace_preserves_imported_flags_references_and_keys_without_inheriting_old_bindings() {
    let data = photocraft_engine::paint::tile::b64_encode(&input());
    let mut loaded = session();
    loaded.execute("actions.load", json!({"data": data})).unwrap();
    assert!(loaded.actions.list.iter().any(|a| a.name == "OldB"), "ordinary Load still replaces only the matching set");
    let mut s = session();
    let revision = s.active().unwrap().revision;
    let history = s.active().unwrap().history.entries();
    let journal = s.journal.clone();
    let result = s.execute("actions.replace", json!({"data": data, "confirm": true})).unwrap();
    assert_eq!(result["actions"], json!(["Caller", "NewChild", "OldA"]));
    assert_eq!(s.actions.list.len(), 3);
    assert!(s.actions.sets.is_empty(), "old empty folders are replaced too");
    assert!(s.actions.list.iter().all(|a| a.set.as_deref() == Some("Incoming")));
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(s.actions.list[0].dialogs.iter().copied().collect::<Vec<_>>(), [0]);
    assert!(!s.prefs().shortcuts.contains_key("actions.play:OldA"), "an identically named replacement does not inherit the old function key");
    assert!(!s.prefs().shortcuts.contains_key("actions.play:OldB"));
    assert_eq!(s.prefs().shortcuts["actions.play:NewChild"], "F8");
    assert_eq!(s.prefs().shortcuts["actions.play:Unloaded"], "F9");
    assert_eq!(s.prefs().shortcuts["view.zoomIn"], "Cmd+0");
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.journal, journal);
    let played = s.execute("actions.play", json!({"action": "Caller"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(s.active().unwrap().doc.selection.is_some(), "the imported named call still targets NewChild");
    let empty = atn_cmds::export("Empty", &[], &[], false).unwrap().0;
    let result = s.execute("actions.replace", json!({"data": photocraft_engine::paint::tile::b64_encode(&empty), "confirm": true})).unwrap();
    assert_eq!(result["set"], "Empty");
    assert!(s.actions.list.is_empty());
    assert_eq!(s.actions.sets, ["Empty"]);
    assert!(!s.prefs().shortcuts.contains_key("actions.play:NewChild"));
}

/// Rejected input, absent consent and active recording/playback never clear or flush definitions.
#[test]
fn invalid_or_unconfirmed_replacement_preserves_the_library_and_preferences() {
    let data = photocraft_engine::paint::tile::b64_encode(&input());
    let mut s = session();
    let list = s.actions.list.clone();
    let sets = s.actions.sets.clone();
    let shortcuts = s.prefs().shortcuts.clone();
    let revision = s.active().unwrap().revision;
    let rev = s.actions.rev;
    for params in [
        json!({"data": data}),
        json!({"data": data, "confirm": false}),
        json!({"data": data, "confirm": "yes"}),
        json!({"data": data, "confirm": true, "replace": false}),
        json!({"data": "!invalid!", "confirm": true}),
        json!({"data": "QUJD", "confirm": true}),
        json!({"confirm": true}),
        json!({"data": data, "confirm": true, "set": " "}),
        json!({"data": data, "confirm": true, "set": 7}),
    ] {
        assert!(s.execute("actions.replace", params.clone()).is_err(), "{params}");
        assert_eq!(s.actions.list, list);
        assert_eq!(s.actions.sets, sets);
        assert_eq!(s.prefs().shortcuts, shortcuts);
        assert_eq!(s.actions.rev, rev);
        assert_eq!(s.active().unwrap().revision, revision);
    }
    s.execute("actions.record", json!({"action": "OldA"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let recording = s.actions.recording;
    assert!(s.execute("actions.replace", json!({"data": data, "confirm": true})).is_err());
    assert_eq!(s.actions.list, list, "pending recorded steps were not flushed");
    assert_eq!(s.actions.recording, recording);
    s.execute("actions.stop", json!({})).unwrap();
    s.actions.list.push(Action::new("Try Replace", vec![("actions.replace".into(), json!({"data": data, "confirm": true}))]));
    let list = s.actions.list.clone();
    let result: Value = s.execute("actions.play", json!({"action": "Try Replace"})).unwrap();
    assert!(result["failed"].is_object(), "{result}");
    assert_eq!(s.actions.list, list);
    assert_eq!(s.prefs().shortcuts, shortcuts);
}
