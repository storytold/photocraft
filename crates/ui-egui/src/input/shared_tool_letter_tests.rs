//! A letter the user gives several tools (#14208): it enters their group, Shift+letter cycles it in
//! toolbar order, and a held letter springs, as with the default letters.

use super::{Key, Modifiers, PhotocraftApp, Tool};
use crate::shortcut_dispatch::tests::{app_harness, tap};
use egui_kittest::Harness;
use serde_json::json;

/// Gives `key` to each tool of `tools`, as Edit › Keyboard Shortcuts saves it.
fn assign(app: &mut PhotocraftApp, key: &str, tools: &[Tool]) {
    for tool in tools {
        app.run("prefs.set", json!({"path": format!("shortcuts.{}", crate::tool_shortcuts::id(*tool)), "value": key})).unwrap();
    }
}

/// An app whose three lassos share Q, which Quick Mask gave up (what OK in the dialog leaves).
fn lassos_on_q(shift_switch: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("prefs.set", json!({"path": "shortcuts.select.editInQuickMaskMode", "value": ""})).unwrap();
    assign(&mut app, "Q", &[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso]);
    app.session.edit_prefs(|prefs| prefs.tools.use_shift_key_for_tool_switch = shift_switch);
    let mut h = app_harness(app);
    h.state_mut().ui.tool = Tool::Brush;
    h
}

#[test]
fn shift_and_a_shared_letter_cycle_its_tools_in_toolbar_order() {
    let mut h = lassos_on_q(true);
    tap(&mut h, Key::Q, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    for want in [Tool::PolygonLasso, Tool::MagneticLasso, Tool::Lasso, Tool::PolygonLasso] {
        tap(&mut h, Key::Q, Modifiers::SHIFT);
        assert_eq!(h.state().ui.tool, want);
    }
    // With the Shift preference on, the plain letter keeps the group's tool.
    tap(&mut h, Key::Q, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    // From another tool, the letter comes back to the group's last tool.
    tap(&mut h, Key::B, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    tap(&mut h, Key::Q, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    // The lassos' default L / Shift+L are gone with the reassignment; the defaults elsewhere still cycle.
    tap(&mut h, Key::W, Modifiers::NONE);
    tap(&mut h, Key::W, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::QuickSelection);
}

#[test]
fn without_the_shift_preference_the_shared_letter_cycles() {
    let mut h = lassos_on_q(false);
    tap(&mut h, Key::Q, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    for want in [Tool::PolygonLasso, Tool::MagneticLasso, Tool::Lasso] {
        tap(&mut h, Key::Q, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, want);
    }
}

#[test]
fn a_held_shared_letter_springs_back() {
    let mut h = lassos_on_q(true);
    h.event(egui::Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    assert!(h.state().ui.spring.is_some());
    h.run_steps(30);
    h.event(egui::Event::Key { key: Key::Q, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert_eq!(h.state().ui.spring, None);
}

/// A key only one tool has selects just that tool (#4086), and its default letter skips it.
#[test]
fn a_letter_one_tool_has_selects_only_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assign(&mut app, "N", &[Tool::PolygonLasso]);
    let mut h = app_harness(app);
    h.state_mut().ui.tool = Tool::Brush;
    tap(&mut h, Key::N, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    tap(&mut h, Key::N, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    tap(&mut h, Key::L, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    for want in [Tool::MagneticLasso, Tool::Lasso] {
        tap(&mut h, Key::L, Modifiers::SHIFT);
        assert_eq!(h.state().ui.tool, want);
    }
}

/// L given explicitly to Polygonal Lasso, whose group mates keep L by default: L still cycles all three.
#[test]
fn a_group_default_letter_assigned_explicitly_keeps_cycling() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assign(&mut app, "L", &[Tool::PolygonLasso]);
    let mut h = app_harness(app);
    h.state_mut().ui.tool = Tool::Brush;
    tap(&mut h, Key::L, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    for want in [Tool::PolygonLasso, Tool::MagneticLasso, Tool::Lasso] {
        tap(&mut h, Key::L, Modifiers::SHIFT);
        assert_eq!(h.state().ui.tool, want);
    }
}

/// Q is held for Lasso when the binding moves Lasso off Q: the key-up still releases the Q its press
/// stored, so the spring gives the previous tool back.
#[test]
fn a_binding_change_while_held_still_releases_the_letter() {
    let mut h = lassos_on_q(true);
    h.event(egui::Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    assert_eq!(h.state().ui.spring.as_ref().map(|s| s.keys.clone()), Some(vec!['Q']));
    assign(h.state_mut(), "N", &[Tool::Lasso]);
    h.run_steps(30);
    h.event(egui::Event::Key { key: Key::Q, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    assert_eq!(h.state().ui.spring, None);
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

/// The Shift+letter row of a default letter names the tools that letter cycles now.
fn next_in_group_label(app: &PhotocraftApp, letter: char) -> Option<String> {
    let id = crate::shortcut_dispatch::next_in_group_id(letter);
    crate::shortcut_dispatch::command_rows(app).into_iter().find(|r| r.command.id == id).map(|r| r.label)
}

/// B given to the three lassos and taken from the brushes: Shift+B's row names Lasso, keeps its id and
/// binding, and the rows of letters nobody changed keep their labels.
#[test]
fn a_reassigned_default_letter_names_the_tools_it_cycles() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert_eq!(next_in_group_label(&app, 'B').as_deref(), Some("Next Brush Tool"));
    let brushes: Vec<Tool> = Tool::ALL.into_iter().filter(|t| t.key() == 'B').collect();
    assign(&mut app, "", &brushes);
    assign(&mut app, "B", &[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso]);
    assert_eq!(next_in_group_label(&app, 'B').as_deref(), Some("Next Lasso Tool (B)"));
    let id = crate::shortcut_dispatch::next_in_group_id('B');
    let row = crate::shortcut_dispatch::command_rows(&app).into_iter().find(|r| r.command.id == id).unwrap();
    assert_eq!(row.defaults, craft::keymap::Binding::parse("Shift+B").into_iter().collect::<Vec<_>>());
    assert_eq!(next_in_group_label(&app, 'W').as_deref(), Some("Next Object Selection Tool"));
    let mut h = app_harness(app);
    h.state_mut().ui.tool = Tool::Move;
    tap(&mut h, Key::B, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    tap(&mut h, Key::B, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
}
