//! #14319: Edit › Keyboard Shortcuts › Use Legacy Undo Shortcuts. A stored choice, off by
//! default, read when the preferences load at launch: a run started with it on gives ⌘Z to
//! Toggle Last State (Undo/Redo), ⌥⌘Z to Step Backward and ⇧⌘Z to Step Forward.

use serde_json::{Value, json};

use crate::Session;

/// The undo commands and their keys in a run started with the box off (today's keys).
const TODAY: &[(&str, Option<&str>)] = &[
    ("edit.undo", Some("Cmd+Z")),
    ("edit.redo", Some("Cmd+Shift+Z")),
    ("edit.toggleLastState", Some("Cmd+Alt+Z")),
    ("history.stepBackward", None),
    ("history.stepForward", None),
];

/// The same commands in a run started with it on.
const LEGACY: &[(&str, Option<&str>)] = &[
    ("edit.undo", None),
    ("edit.redo", None),
    ("edit.toggleLastState", Some("Cmd+Z")),
    ("history.stepBackward", Some("Cmd+Alt+Z")),
    ("history.stepForward", Some("Cmd+Shift+Z")),
];

/// `id`'s key in effect, with `default` as the command's registry default.
fn key(s: &Session, id: &str) -> Option<String> {
    let default = TODAY.iter().find(|(c, _)| *c == id).and_then(|(_, k)| *k);
    s.prefs().shortcut(id, default).map(str::to_string)
}

fn keys(s: &Session) -> Vec<(&'static str, Option<String>)> {
    TODAY.iter().map(|(id, _)| (*id, key(s, id))).collect()
}

fn expected(table: &[(&'static str, Option<&str>)]) -> Vec<(&'static str, Option<String>)> {
    table.iter().map(|(id, k)| (*id, k.map(str::to_string))).collect()
}

/// A new run that loads what `s` saved.
fn restart(s: &Session) -> Session {
    let mut next = Session::new();
    next.load_prefs_json(&s.prefs_to_json()).unwrap();
    next
}

fn set(s: &mut Session, on: bool) {
    s.execute("prefs.set", json!({"path": "useLegacyUndoShortcuts", "value": on})).unwrap();
}

#[test]
fn it_is_off_by_default_and_readable_and_settable() {
    let mut s = Session::new();
    assert_eq!(s.execute("prefs.get", json!({"path": "useLegacyUndoShortcuts"})).unwrap(), json!(false));
    set(&mut s, true);
    assert_eq!(s.execute("prefs.get", json!({"path": "useLegacyUndoShortcuts"})).unwrap(), json!(true));
    assert!(s.prefs().use_legacy_undo_shortcuts);
    let saved: Value = serde_json::from_str(&s.prefs_to_json()).unwrap();
    assert_eq!(saved["useLegacyUndoShortcuts"], json!(true), "stored in the preferences file");
}

#[test]
fn ticking_it_changes_no_key_until_the_next_start() {
    let mut s = Session::new();
    set(&mut s, true);
    assert_eq!(keys(&s), expected(TODAY), "this run keeps its keys");
    let next = restart(&s);
    assert!(next.prefs().use_legacy_undo_shortcuts, "it survives the restart");
    assert_eq!(keys(&next), expected(LEGACY), "the next run has the legacy keys");
}

#[test]
fn unticking_it_also_waits_for_the_next_start() {
    let mut s = Session::new();
    set(&mut s, true);
    let mut legacy = restart(&s);
    set(&mut legacy, false);
    assert_eq!(keys(&legacy), expected(LEGACY), "the run keeps the keys it started with");
    // Nor does a reset of the preferences change them mid-run.
    legacy.execute("prefs.reset", json!({})).unwrap();
    assert!(!legacy.prefs().use_legacy_undo_shortcuts);
    assert_eq!(keys(&legacy), expected(LEGACY));
    assert_eq!(keys(&restart(&legacy)), expected(TODAY), "and the next run has today's keys again");
}

#[test]
fn with_it_off_every_key_stays_exactly_as_today() {
    let s = restart(&Session::new());
    assert_eq!(keys(&s), expected(TODAY));
    // Every bindable command, not only the undo ones.
    let fresh = Session::new();
    for (id, default) in crate::prefs::shortcuts::bindable() {
        assert_eq!(s.prefs().shortcut(id, default), fresh.prefs().shortcut(id, default), "{id}");
        assert_eq!(s.prefs().shortcut(id, default), default, "{id}");
    }
}

#[test]
fn in_a_legacy_run_each_undo_key_has_one_owner_and_a_user_key_still_wins() {
    let mut s = Session::new();
    set(&mut s, true);
    s.execute("prefs.set", json!({"path": "shortcuts", "value": {"history.stepForward": "F12"}})).unwrap();
    let next = restart(&s);
    assert_eq!(key(&next, "history.stepForward").as_deref(), Some("F12"), "the user's own key wins");
    let bound: Vec<(&str, String)> = crate::prefs::shortcuts::bindable()
        .chain(TODAY.iter().map(|(id, k)| (*id, *k)))
        .filter_map(|(id, default)| next.prefs().shortcut(id, default).map(|k| (id, k.to_string())))
        .collect();
    for want in ["Cmd+Z", "Cmd+Alt+Z"] {
        let owners: std::collections::BTreeSet<&str> = bound.iter().filter(|(_, k)| k == want).map(|(id, _)| *id).collect();
        assert_eq!(owners.len(), 1, "{want}: {owners:?}");
    }
}

#[test]
fn a_bad_or_missing_value_loads_as_off() {
    for text in [r#"{"useLegacyUndoShortcuts": "yes"}"#, r#"{"useLegacyUndoShortcuts": null}"#, r#"{"useLegacyUndoShortcuts": 1}"#, "{}"] {
        let mut s = Session::new();
        s.load_prefs_json(text).unwrap();
        assert!(!s.prefs().use_legacy_undo_shortcuts, "{text}");
        assert_eq!(keys(&s), expected(TODAY), "{text}");
    }
    // A value that is true loads as on.
    let mut s = Session::new();
    s.load_prefs_json(r#"{"useLegacyUndoShortcuts": true}"#).unwrap();
    assert_eq!(keys(&s), expected(LEGACY));
}
