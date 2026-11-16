//! #14325: Edit › Keyboard Shortcuts › Use Legacy Channel Shortcuts. A stored choice, off by
//! default and taking effect at once: in an RGB document ⌘1…⌘4 target Red, Green, Blue and the
//! first alpha channel ([`LEGACY_CHANNEL_SHORTCUTS`]), and ⌘1 / ⌘2 no longer zoom or target the
//! composite. The key dispatch itself is the shell's; this file checks the table it follows.

use crate::cmd::channel_cmds::ChannelTarget;
use serde_json::{Value, json};

use super::{LEGACY_CHANNEL_SHORTCUTS, replaced_by_legacy_channel_keys};
use crate::Session;

fn set(s: &mut Session, value: Value) -> Result<Value, String> {
    s.execute("prefs.set", json!({"path": "useLegacyChannelShortcuts", "value": value})).map_err(|e| e.to_string())
}

/// The command the legacy keys give `key`, in our notation.
fn legacy(key: &str) -> Option<&'static str> {
    LEGACY_CHANNEL_SHORTCUTS.iter().find(|(k, _)| *k == key).map(|(_, id)| *id)
}

/// An RGB document with one alpha channel.
fn rgb_with_alpha() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    s
}

fn target(s: &Session) -> ChannelTarget {
    s.active().unwrap().channel_view.target
}

#[test]
fn it_is_off_by_default_settable_and_kept_across_a_restart() {
    let mut s = Session::new();
    assert_eq!(s.execute("prefs.get", json!({"path": "useLegacyChannelShortcuts"})).unwrap(), json!(false));
    set(&mut s, json!(true)).unwrap();
    assert!(s.prefs().use_legacy_channel_shortcuts, "at once, no restart");
    let mut next = Session::new();
    next.load_prefs_json(&s.prefs_to_json()).unwrap();
    assert!(next.prefs().use_legacy_channel_shortcuts, "and it survives a restart");
    set(&mut next, json!(false)).unwrap();
    assert!(!next.prefs().use_legacy_channel_shortcuts);
}

#[test]
fn a_bad_value_is_an_error_and_a_bad_stored_value_loads_as_off() {
    let mut s = Session::new();
    for bad in [json!("yes"), json!(1), json!(null), json!([true])] {
        assert!(set(&mut s, bad.clone()).is_err(), "{bad}");
        assert!(!s.prefs().use_legacy_channel_shortcuts, "{bad}");
    }
    let mut s = Session::new();
    s.load_prefs_json(r#"{"useLegacyChannelShortcuts": "yes"}"#).unwrap();
    assert!(!s.prefs().use_legacy_channel_shortcuts);
}

#[test]
fn the_legacy_keys_target_red_green_blue_and_the_first_alpha_channel() {
    let mut s = rgb_with_alpha();
    for (key, want) in
        [("Cmd+1", ChannelTarget::Color(0)), ("Cmd+2", ChannelTarget::Color(1)), ("Cmd+3", ChannelTarget::Color(2)), ("Cmd+4", ChannelTarget::Alpha(0))]
    {
        let id = legacy(key).unwrap_or_else(|| panic!("{key} has a legacy command"));
        s.execute(id, json!({})).unwrap();
        assert_eq!(target(&s), want, "{key} → {id}");
    }
    // One key lower than with the box off, up to ⌘7; ⌘8 and ⌘9 have none.
    for n in 1..=7 {
        assert_eq!(legacy(&format!("Cmd+{n}")), Some(format!("channel.target.slot{}", n + 2)).as_deref());
    }
    assert_eq!(legacy("Cmd+8"), None);
    assert_eq!(legacy("Cmd+9"), None);
    assert_eq!(legacy("Cmd+0"), None, "⌘0 (Fit on Screen) is not a channel key");
}

#[test]
fn the_keys_it_takes_over_are_100_percent_the_composite_and_the_slots() {
    for id in ["view.actualPixels", "channel.target.composite", "channel.target.slot3", "channel.target.slot9"] {
        assert!(replaced_by_legacy_channel_keys(id), "{id}");
    }
    for id in ["view.fitOnScreen", "channel.target.layerMask", "select.loadLuminosity", "edit.undo"] {
        assert!(!replaced_by_legacy_channel_keys(id), "{id}");
    }
}

#[test]
fn with_it_off_every_command_keeps_its_key() {
    // The setting changes no stored or default key: dispatch alone reads it.
    let mut s = Session::new();
    let before: Vec<(&str, Option<String>)> = crate::prefs::shortcuts::bindable().map(|(id, d)| (id, s.prefs().shortcut(id, d).map(str::to_string))).collect();
    set(&mut s, json!(true)).unwrap();
    let after: Vec<(&str, Option<String>)> = crate::prefs::shortcuts::bindable().map(|(id, d)| (id, s.prefs().shortcut(id, d).map(str::to_string))).collect();
    assert_eq!(before, after, "the shown keys (⌘1 beside 100%) stay");
    assert!(s.prefs().shortcuts.is_empty(), "and the shortcut set isn't modified");
}

#[test]
fn a_missing_slot_names_the_key_that_reached_it() {
    let mut s = rgb_with_alpha();
    let key = |n: usize| format!("{}{n}", if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" });
    let err = s.execute("channel.target.slot7", json!({})).unwrap_err().to_string();
    assert!(err.contains(&format!("no channel for {}", key(7))), "{err}");
    set(&mut s, json!(true)).unwrap();
    let err = s.execute("channel.target.slot7", json!({})).unwrap_err().to_string();
    assert!(err.contains(&format!("no channel for {}", key(5))), "with the legacy keys, ⌘5 reaches slot 7: {err}");
    // Other modes keep today's keys (not measured).
    let mut g = Session::new();
    g.execute("file.new", json!({"width": 4, "height": 4, "mode": "gray"})).unwrap();
    set(&mut g, json!(true)).unwrap();
    let err = g.execute("channel.target.slot4", json!({})).unwrap_err().to_string();
    assert!(err.contains(&format!("no channel for {}", key(4))), "{err}");
}
