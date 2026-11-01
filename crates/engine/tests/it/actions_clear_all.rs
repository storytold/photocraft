//! Confirmation clears definitions and their own shortcuts without editing the document.
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::json;

/// Clearing requires consent and idle playback/recording, and preserves document history.
#[test]
fn confirmed_clear_is_atomic_for_definitions_shortcuts_and_recording() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    for (name, set) in [("A", "One"), ("B", "Two")] {
        let mut a = Action::new(name, vec![]);
        a.set = Some(set.into());
        s.actions.list.push(a);
    }
    s.prefs.edit(|p| {
        for (id, key) in [("actions.play:A", "F5"), ("actions.play:B", "F6"), ("actions.play:Unloaded", "F7"), ("view.zoomIn", "Cmd+0")] {
            p.shortcuts.insert(id.into(), key.into());
        }
    });
    let definitions = s.actions.list.clone();
    let shortcuts = s.prefs().shortcuts.clone();
    let rev = s.actions.rev;
    for params in [json!({}), json!({"confirm": false}), json!({"confirm": null}), json!({"confirm": "yes"})] {
        assert!(s.execute("actions.clearAll", params.clone()).is_err(), "{params}");
        assert_eq!(s.actions.list, definitions);
        assert_eq!(s.prefs().shortcuts, shortcuts);
        assert_eq!(s.actions.rev, rev);
    }
    s.execute("actions.record", json!({"action": "A"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let recording = s.actions.recording;
    assert!(s.execute("actions.clearAll", json!({"confirm": true})).is_err());
    assert_eq!(s.actions.list, definitions, "refusal does not flush pending recording");
    assert_eq!(s.actions.recording, recording);
    s.execute("actions.stop", json!({})).unwrap();
    s.actions.list.push(Action::new("Try Clear", vec![("actions.clearAll".into(), json!({"confirm": true}))]));
    let definitions = s.actions.list.clone();
    let result = s.execute("actions.play", json!({"action": "Try Clear"})).unwrap();
    assert!(result["failed"]["error"].as_str().is_some_and(|e| e.contains("playback")), "{result}");
    assert_eq!(s.actions.list, definitions);
    let revision = s.active().unwrap().revision;
    let history = s.active().unwrap().history.entries();
    let journal = s.journal.clone();
    assert_eq!(s.execute("actions.clearAll", json!({"confirm": true})).unwrap()["cleared"], 3);
    assert!(s.actions.list.is_empty());
    assert!(!s.prefs().shortcuts.contains_key("actions.play:A"));
    assert!(!s.prefs().shortcuts.contains_key("actions.play:B"));
    assert_eq!(s.prefs().shortcuts.get("actions.play:Unloaded"), shortcuts.get("actions.play:Unloaded"));
    assert_eq!(s.prefs().shortcuts.get("view.zoomIn"), shortcuts.get("view.zoomIn"));
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.journal, journal);
    let rev = s.actions.rev;
    assert_eq!(s.execute("actions.clearAll", json!({"confirm": true})).unwrap()["cleared"], 0);
    assert_eq!(s.actions.rev, rev);
}
