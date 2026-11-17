//! Edit › Keyboard Shortcuts › Set through the dialog's session: a set made in the dialog is saved
//! with OK and in use, an action's function key travels with every set, and a set saves no action
//! keys. Making, choosing and deleting sets is craft-keymap's `Profiles` and the dialog's own.

use craft::keymap::{Binding, CommandRef, Overrides, Policy};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use photocraft_engine::shortcut_sets::{DEFAULTS_NAME, ShortcutSet, modified};
use serde_json::json;

use super::shortcut_capture_tests::{harness, tap};
use super::shortcuts::{press_ok, rebind};
use crate::PhotocraftApp;

/// Makes the dialog's draft use the set `name`, as choosing it in the Set row does.
fn choose(app: &mut PhotocraftApp, name: &str) {
    let session = app.prefs_rt.shortcuts.as_mut().expect("the dialog is open");
    let builtins = [(DEFAULTS_NAME, Overrides::new())];
    session.profiles.select(name, &builtins).unwrap();
    let overrides = session.profiles.active_overrides(&builtins).clone();
    session.keymap.set_overrides(overrides);
}

fn open(h: &mut Harness<'_, PhotocraftApp>) {
    crate::prefs_ui::open_shortcuts(h.state_mut(), 0);
}

fn ok(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    press_ok(h.state_mut(), &ctx);
    h.run_steps(2);
}

/// A set made in the dialog is saved with OK, becomes the set in use, and its keys run.
#[test]
fn sets_saved_in_the_dialog_persist_and_the_active_sets_keys_run() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    open(&mut h);
    // An edit in the dialog, then New Set… with a name: the set holds the edited bindings.
    rebind(h.state_mut(), &CommandRef::new("edit.copy"), Binding::parse("F9").unwrap(), Policy::Steal).unwrap();
    {
        let session = h.state_mut().prefs_rt.shortcuts.as_mut().unwrap();
        let builtins = [(DEFAULTS_NAME, Overrides::new())];
        let overrides = session.keymap.overrides().clone();
        assert_eq!(session.profiles.save_as("Painting", overrides, &builtins).unwrap(), "Painting");
        // The built-in's name is refused.
        assert!(session.profiles.save_as(DEFAULTS_NAME, Overrides::new(), &builtins).is_err());
    }
    // Nothing is saved before OK.
    assert!(h.state().session.prefs().shortcut_sets.is_empty());
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!((p.shortcut_set.as_str(), p.shortcuts.get("edit.copy").map(String::as_str)), ("Painting", Some("F9")));
    assert_eq!(p.shortcut_sets["Painting"].shortcuts.get("edit.copy").map(String::as_str), Some("F9"));
    assert!(!modified(p), "the bindings are the set's");
    // The active set's key runs the command.
    crate::shortcut_dispatch::set_dry_run(&ctx, true);
    tap(&mut h, Key::F9, Modifiers::NONE);
    let log = crate::shortcut_dispatch::take_log(&ctx);
    assert!(log.iter().any(|(id, _)| id == "edit.copy"), "{log:?}");

    // Choosing the defaults and deleting the set wait for OK too; the bindings stay as they were chosen.
    open(&mut h);
    choose(h.state_mut(), DEFAULTS_NAME);
    {
        let session = h.state_mut().prefs_rt.shortcuts.as_mut().unwrap();
        let builtins = [(DEFAULTS_NAME, Overrides::new())];
        session.profiles.delete("Painting", &builtins).unwrap();
    }
    assert!(!h.state().session.prefs().shortcut_sets.is_empty(), "not before OK");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.shortcut_sets.is_empty() && p.shortcut_set.is_empty());
    assert!(p.shortcuts.is_empty(), "the defaults' bindings are in use: {:?}", p.shortcuts);
}

/// #7004: an action's F-key belongs to the action, so picking another set in the dialog keeps it
/// (as `useSet` does) and assigning one doesn't mark the set modified.
#[test]
fn the_set_dropdown_keeps_action_function_keys_like_use_set() {
    let mut h = harness();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.copy": "F9"}, "saveSet": "A"})).unwrap();
    // B, from a settings file written before sets left action keys out: it holds an old key for
    // the action, and gives F2 to a command.
    h.state_mut().session.edit_prefs(|p| {
        let b = ShortcutSet {
            shortcuts: [("actions.play:Sepia".to_string(), "F5".to_string()), ("layer.renameLayer".to_string(), "F2".to_string())].into(),
            alternate_shortcuts: Default::default(),
        };
        p.shortcut_sets.insert("B".into(), b);
    });
    h.state_mut().run("edit.keyboardShortcuts", json!({"useSet": ""})).unwrap();
    crate::actions::assign_shortcut(h.state_mut(), "Sepia", "F2").unwrap();
    assert!(!modified(h.state().session.prefs()), "an action key doesn't modify the defaults");

    open(&mut h);
    choose(h.state_mut(), "A");
    let (shortcuts, _) = photocraft_engine::keymap_defs::overrides_to_prefs(h.state().prefs_rt.shortcuts.as_ref().unwrap().keymap.overrides());
    assert_eq!(shortcuts.get("edit.copy").map(String::as_str), Some("F9"), "A's keys are the draft's");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!((p.shortcut_set.as_str(), p.shortcuts.get("edit.copy").map(String::as_str)), ("A", Some("F9")));
    assert_eq!(p.shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"), "the action keeps its key");
    assert!(!modified(p));

    // Back to the defaults: the action's key stays.
    open(&mut h);
    choose(h.state_mut(), DEFAULTS_NAME);
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.shortcut_set.is_empty());
    assert_eq!(p.shortcuts.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(), [("actions.play:Sepia", "F2")]);

    // B: what wins is what `useSet` gives: the action keeps F2, B's old action key is dropped and
    // its own F2 binding stays.
    open(&mut h);
    choose(h.state_mut(), "B");
    ok(&mut h);
    let mut engine = h.state().session.prefs().clone();
    photocraft_engine::shortcut_sets::load(&mut engine, &json!({"useSet": "B"})).unwrap();
    let p = h.state().session.prefs();
    assert_eq!(p.shortcuts, engine.shortcuts);
    assert_eq!(p.shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"));
    assert_eq!(p.shortcuts.get("layer.renameLayer").map(String::as_str), Some("F2"));
    assert_eq!(p.shortcut_set, "B");
    assert!(p.shortcut_sets.values().all(|s| !s.shortcuts.keys().any(|k| k.starts_with("actions.play:"))), "OK saves no action key in a set");
}

/// A real edit still counts as a change of the set; saving it stores the set without the actions'
/// F-keys.
#[test]
fn an_edit_still_marks_the_set_modified_and_sets_save_no_action_keys() {
    let mut h = harness();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.copy": "F9"}, "saveSet": "A"})).unwrap();
    crate::actions::assign_shortcut(h.state_mut(), "Sepia", "F2").unwrap();
    assert!(!modified(h.state().session.prefs()), "A's keys are the bindings");
    open(&mut h);
    rebind(h.state_mut(), &CommandRef::new("edit.copy"), Binding::parse("F8").unwrap(), Policy::Steal).unwrap();
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.shortcut_sets["A"].shortcuts, [("edit.copy".to_string(), "F8".to_string())].into(), "the set holds the edit and no action key");
    assert_eq!(p.shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"));
    assert!(!modified(p), "OK saved the edit into the set in use");
    // An edit made through the engine, outside the set, is a modification.
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.cut": "F7"}})).unwrap();
    assert!(modified(h.state().session.prefs()));
}
