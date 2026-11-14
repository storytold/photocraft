//! #14314: Edit › Keyboard Shortcuts has an Undo button that reverts the last committed key
//! change and then reads Redo, one level only; starting a new edit disables it.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The Keyboard Shortcuts dialog with `command` selected, filtered to the edit.c… commands.
fn dialog(command: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("selected".into(), json!(command));
    // Only Edit › Cut, Copy and their neighbours, so every row is in view.
    f.insert("filter".into(), json!("edit.c"));
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

fn main_key(h: &Harness<'_, PhotocraftApp>, id: u64, command: &str) -> Value {
    field(h, id, "overrides").get(command).cloned().unwrap_or(Value::Null)
}

fn press(h: &mut Harness<'_, PhotocraftApp>, k: Key, m: Modifiers) {
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.run_steps(2);
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn click_cell(h: &mut Harness<'_, PhotocraftApp>, shortcut: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, &crate::shortcuts::pretty(shortcut)).click();
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(3);
}

/// The Undo / Redo button: its label and whether it is enabled.
fn undo_button(h: &Harness<'_, PhotocraftApp>) -> (&'static str, bool) {
    for label in ["Undo", "Redo"] {
        if let Some(b) = h.query_by_role_and_label(egui::accesskit::Role::Button, label) {
            return (label, !b.accesskit_node().is_disabled());
        }
    }
    panic!("no Undo or Redo button");
}

/// Gives `command`'s cell (showing `shown`) the key `k` + `m`, committed with Enter.
fn commit(h: &mut Harness<'_, PhotocraftApp>, shown: &str, k: Key, m: Modifiers) {
    click_cell(h, shown);
    press(h, k, m);
    press(h, Key::Enter, Modifiers::NONE);
}

const M: Modifiers = Modifiers::COMMAND.plus(Modifiers::ALT).plus(Modifiers::SHIFT);

#[test]
fn undo_is_disabled_when_the_dialog_opens() {
    let (h, _) = dialog("edit.cut");
    assert_eq!(undo_button(&h), ("Undo", false));
}

#[test]
fn undo_reverts_the_last_committed_key_and_then_reads_redo() {
    let (mut h, id) = dialog("edit.cut");
    commit(&mut h, "Cmd+X", Key::Y, M);
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+Y"));
    assert_eq!(undo_button(&h), ("Undo", true), "a committed change can be undone");
    click(&mut h, "Undo");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "Cut has its old key again");
    assert_eq!(undo_button(&h), ("Redo", true));
    click(&mut h, "Redo");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+Y"), "Redo re-applies it");
    assert_eq!(undo_button(&h), ("Undo", true));
    // The dialog's OK keeps what the button left.
    click(&mut h, "Undo");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.cut", Some("Cmd+X")).as_deref(), Some("Cmd+X"));
}

#[test]
fn undo_is_one_level_only() {
    let (mut h, id) = dialog("edit.cut");
    commit(&mut h, "Cmd+X", Key::Y, M);
    // Accept and moving to another row commit too; the last commit is Copy's.
    click_cell(&mut h, "Cmd+C");
    press(&mut h, Key::U, M);
    click(&mut h, "Accept");
    assert_eq!(main_key(&h, id, "edit.copy"), json!("Cmd+Alt+Shift+U"));
    click(&mut h, "Undo");
    assert_eq!(main_key(&h, id, "edit.copy"), Value::Null, "Copy's change is undone");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+Y"), "Cut's earlier one stays");
    click(&mut h, "Redo");
    click(&mut h, "Undo");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+Y"), "still one level");
}

#[test]
fn a_row_change_commit_is_undone_too() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::Y, M);
    // A click on Copy's name: Cut's key is committed, and no field opens.
    h.get_by_label("   Copy").click();
    h.run_steps(3);
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+Y"));
    assert_eq!(field(&h, id, "capture"), json!(false));
    assert_eq!(undo_button(&h), ("Undo", true));
    click(&mut h, "Undo");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null);
    click(&mut h, "Redo");
    // A click on Copy's Shortcut cell after a pending key: committed, but Copy's field opens, a new edit.
    click_cell(&mut h, "Cmd+Alt+Shift+Y");
    press(&mut h, Key::U, M);
    click_cell(&mut h, "Cmd+C");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Alt+Shift+U"));
    assert_eq!(undo_button(&h), ("Undo", false), "the new edit disables it");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(undo_button(&h), ("Undo", false), "and it stays off");
}

#[test]
fn undo_puts_back_a_key_taken_from_a_conflicting_command() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::C, Modifiers::COMMAND);
    click(&mut h, "Accept and Go to Conflict");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+C"));
    assert_eq!(main_key(&h, id, "edit.copy"), json!(""), "taken from Copy");
    click(&mut h, "Undo");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "Cut is back to its key");
    assert_eq!(main_key(&h, id, "edit.copy"), Value::Null, "and Copy has ⌘C again");
    click(&mut h, "Redo");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+C"));
    assert_eq!(main_key(&h, id, "edit.copy"), json!(""));
}

#[test]
fn starting_a_new_edit_disables_it() {
    let (mut h, _) = dialog("edit.cut");
    commit(&mut h, "Cmd+X", Key::Y, M);
    click_cell(&mut h, "Cmd+Alt+Shift+Y");
    assert_eq!(undo_button(&h), ("Undo", false), "a field opened");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(undo_button(&h), ("Undo", false), "the earlier change is out of reach");
}

#[test]
fn other_changes_are_not_undone_by_it() {
    // Not measured: Use Default, Delete Shortcut and the rest keep today's behaviour, so Undo
    // never reaches back across them.
    let (mut h, id) = dialog("edit.cut");
    commit(&mut h, "Cmd+X", Key::Y, M);
    click(&mut h, "Use Default");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null);
    assert_eq!(undo_button(&h), ("Undo", false));
}
