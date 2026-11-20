//! Bindings edited in Edit › Keyboard Shortcuts and saved with OK: the new owner holds the key in
//! the preferences, the old owners no longer do, and the key runs the new owner's command. The
//! dialog's recording, conflict bar and undo are craft-menu's own (its trials); these tests drive
//! the session the app wraps around it.

use craft::keymap::{Binding, CommandRef, Policy};
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use photocraft_engine::prefs::{bindable_items, normalize_shortcut};
use serde_json::json;

use super::shortcuts::{press_ok, rebind};
use crate::PhotocraftApp;

pub(super) fn harness_with(session: photocraft_engine::Session) -> Harness<'static, Box<PhotocraftApp>> {
    let mut session = Some(session);
    Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session.take().expect("one app"), crate::Services::default())
    })
}

pub(super) fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    harness_with(photocraft_engine::Session::new())
}

/// A key tap: press and release, so the next press isn't a held key's repeat.
pub(super) fn tap(h: &mut Harness<'_, Box<PhotocraftApp>>, key: Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
}

/// Opens the dialog, gives `command` the key `text` as the dialog's own edit does (taking it from
/// its holders), and presses OK.
fn edit_and_ok(h: &mut Harness<'_, Box<PhotocraftApp>>, command: &str, text: &str) {
    let ctx = h.ctx.clone();
    crate::prefs_ui::open_shortcuts(h.state_mut(), 0);
    rebind(h.state_mut(), &CommandRef::new(command), Binding::parse(text).unwrap(), Policy::Steal).unwrap();
    press_ok(h.state_mut(), &ctx);
    // The runtime's keymap follows the saved preferences on the next pass.
    h.run_steps(2);
}

/// The ids that hold `key` as a main or further key under the saved preferences.
fn holders(app: &PhotocraftApp, key: &str) -> Vec<String> {
    let key = normalize_shortcut(key).unwrap();
    bindable_items()
        .filter(|(id, _, _, default, _)| {
            let main = crate::shortcuts::effective_shortcut(app, id, *default);
            main.as_deref().and_then(normalize_shortcut).as_ref() == Some(&key) || crate::shortcuts::effective_alternates(app, id).contains(&key)
        })
        .map(|(id, ..)| id.to_owned())
        .collect()
}

#[test]
fn toolbar_shortcut_can_be_captured_and_restored_on_restart() {
    let mut h = harness();
    edit_and_ok(&mut h, "tools.select.pencil", "F6");
    assert_eq!(h.state().session.prefs().shortcuts.get("tools.select.pencil").map(String::as_str), Some("F6"));
    tap(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Pencil);

    // The saved preferences carry the key to the next run.
    let saved = h.state().session.prefs_to_json();
    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(&saved).unwrap();
    let mut restarted = harness_with(session);
    restarted.run_steps(2);
    tap(&mut restarted, Key::F6, Modifiers::NONE);
    assert_eq!(restarted.state().ui.tool, crate::state::Tool::Pencil);
}

#[test]
fn toolbar_shortcut_overrides_remove_old_keys_and_skip_removed_group_members() {
    use crate::state::Tool;
    let mut h = harness();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"tools.select.move": "F6", "tools.select.pencil": ""}, "allowUnknown": true})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Zoom;
    tap(&mut h, Key::V, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Artboard, "the old key only selects unchanged tools");
    tap(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Move);

    h.state_mut().ui.tool = Tool::Pencil;
    tap(&mut h, Key::V, Modifiers::NONE);
    tap(&mut h, Key::B, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush, "a removed tool cannot return through group memory");
    tap(&mut h, Key::B, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement, "Shift cycling skips a removed tool");

    h.state_mut().run("edit.keyboardShortcuts", json!({"reset": true})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Brush;
    tap(&mut h, Key::B, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Pencil, "reset restores the default group");
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    tap(&mut h, Key::B, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement, "plain-key cycling still follows the preference");
}

/// A binding edit in the dialog, saved with OK: the command holds the key, and the commands that
/// held it (one menu command, or several tools sharing a letter) no longer do.
#[test]
fn binding_edits_leave_the_old_owner_after_ok() {
    // (command given the key, the key, the commands that held it before).
    let table: [(&str, &str, &[&str]); 2] = [
        ("edit.search", "Cmd+J", &["layer.new.layerViaCopy"]),
        ("edit.search", "B", &["tools.select.brush", "tools.select.pencil", "tools.select.colorReplacement", "tools.select.mixerBrush"]),
    ];
    for (command, key, old_owners) in table {
        let mut h = harness();
        assert_eq!(holders(h.state(), key), old_owners, "the key's holders before the edit");
        edit_and_ok(&mut h, command, key);
        assert!(!crate::prefs_ui::shortcuts_open(h.state()), "OK closes the dialog");
        assert_eq!(holders(h.state(), key), [command], "one owner for {key}");
        for old in old_owners {
            let default = bindable_items().find(|item| item.0 == *old).and_then(|item| item.3);
            assert_eq!(h.state().session.prefs().shortcut(old, default), None, "{old} lost {key}");
        }
    }
    // Search now answers B: it opens the palette and leaves the tool alone.
    let mut h = harness();
    edit_and_ok(&mut h, "edit.search", "B");
    h.state_mut().ui.tool = crate::state::Tool::Move;
    tap(&mut h, Key::B, Modifiers::NONE);
    assert!(h.state().ui.palette.open, "B now runs Search");
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move);
}

#[test]
fn a_function_key_moved_off_a_panel_toggle_runs_its_new_command() {
    // #1272: F5 given to Select › Modify › Feather… is taken from Window › Brush Settings, whose
    // row in the dialog is Photoshop's `window.panel.brushSettings` while the key ran the shell's
    // `window.toggle.brushSettings`.
    let mut h = harness();
    let ctx = h.ctx.clone();
    edit_and_ok(&mut h, "select.modify.feather", "F5");
    assert_eq!(holders(h.state(), "F5"), ["select.modify.feather"], "one owner for F5");
    // Pressing F5 opens Feather, not Brush Settings.
    crate::shortcut_dispatch::set_dry_run(&ctx, true);
    tap(&mut h, Key::F5, Modifiers::NONE);
    let log = crate::shortcut_dispatch::take_log(&ctx);
    assert!(log.iter().any(|(id, _)| id == "select.modify.feather"), "{log:?}");
    assert!(!crate::panel_docking::is_shown(h.state(), "brushSettings"));
}

#[test]
fn a_user_shortcut_wins_over_a_default_it_was_not_moved_from() {
    // A key assigned by hand (here through the prefs file, so no row was cleared) still reaches
    // the command it was given to rather than the built-in owner.
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"select.modify.feather": "F7"}, "allowUnknown": true, "removeConflicts": false})).unwrap();
    h.run_steps(2);
    crate::shortcut_dispatch::set_dry_run(&ctx, true);
    tap(&mut h, Key::F7, Modifiers::NONE);
    let log = crate::shortcut_dispatch::take_log(&ctx);
    assert_eq!(log.last().map(|i| i.0.as_str()), Some("select.modify.feather"), "{log:?}");
}
