use super::{Key, Modifiers, PhotocraftApp, Tool};
use crate::panels::toolbar::TOOL_SECTIONS;
use crate::shortcut_dispatch::tests::{app_harness, tap};
use crate::spring_tools::key_of;
use std::collections::HashSet;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}

/// A fresh app enters each default letter group at its first toolbar tool under either preference.
#[test]
fn fresh_tool_letters_pick_the_first_tool_in_toolbar_order() {
    for shift_switch in [true, false] {
        let mut h = app_harness(app());
        h.state_mut().session.edit_prefs(|prefs| prefs.tools.use_shift_key_for_tool_switch = shift_switch);
        let mut checked = HashSet::new();
        for tool in TOOL_SECTIONS.iter().flat_map(|section| section.iter()).flat_map(|slot| slot.iter()).copied() {
            let Some(key) = key_of(tool.key()) else { continue };
            if !checked.insert(key) {
                continue;
            }
            // Nothing is remembered from the letters before.
            h.state_mut().ui.group_tools.clear();
            h.state_mut().ui.tool = if tool.key() == Tool::Move.key() { Tool::Brush } else { Tool::Move };
            tap(&mut h, key, Modifiers::NONE);
            assert_eq!(h.state().ui.tool, tool, "{key:?}, Shift switch: {shift_switch}");
        }
        assert_eq!(checked, Tool::ALL.into_iter().filter_map(|tool| key_of(tool.key())).collect::<HashSet<_>>());
    }
}

/// A remembered Magic Wand takes precedence over the fresh W default, even when hidden.
#[test]
fn plain_w_returns_to_the_remembered_magic_wand() {
    for hidden in [false, true] {
        let mut h = app_harness(app());
        if hidden {
            h.state_mut().session.edit_prefs(|prefs| prefs.toolbar.hidden = vec!["MagicWand".to_owned()]);
        }
        h.state_mut().ui.tool = Tool::MagicWand;
        h.run_steps(2);
        tap(&mut h, Key::B, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, Tool::Brush);
        tap(&mut h, Key::W, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, Tool::MagicWand, "hidden: {hidden}");
    }
}

/// Shift+W follows the flyout order and wraps. With the default preference, plain W keeps
/// the current tool when it already belongs to the group.
#[test]
fn shift_w_cycles_in_toolbar_order() {
    let mut h = app_harness(app());
    h.state_mut().ui.tool = Tool::ObjectSelection;

    for expected in [Tool::QuickSelection, Tool::MagicWand, Tool::ObjectSelection] {
        tap(&mut h, Key::W, Modifiers::SHIFT);
        assert_eq!(h.state().ui.tool, expected);
    }
    tap(&mut h, Key::W, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ObjectSelection);
}

#[test]
fn remapped_first_tool_is_skipped_by_its_default_letter() {
    let mut app = app();
    app.run("prefs.set", serde_json::json!({"path": "shortcuts.tools.select.objectSelection", "value": "F6"})).unwrap();
    let mut h = app_harness(app);
    tap(&mut h, Key::W, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::QuickSelection);
    tap(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ObjectSelection);
}
