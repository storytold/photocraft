//! #14310: in Edit › Keyboard Shortcuts a pressed key first sits in the open field as a pending
//! edit. Accept, Enter or moving to another row commits it, Escape reverts it, and the
//! reference application's warning texts and instructions box are shown.

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
    h.state().ui.dialogs.iter().find(|d| d.id == id).map_or(Value::Null, |d| d.fields.get(name).cloned().unwrap_or(Value::Null))
}

fn open(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    h.state().ui.dialogs.iter().any(|d| d.id == id)
}

fn press(h: &mut Harness<'_, PhotocraftApp>, k: Key, m: Modifiers) {
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.run_steps(2);
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

/// Clicks the Shortcut cell showing `shortcut` (in the platform's notation).
fn click_cell(h: &mut Harness<'_, PhotocraftApp>, shortcut: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, &crate::shortcuts::pretty(shortcut)).click();
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(3);
}

fn accept_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    !h.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").accesskit_node().is_disabled()
}

fn main_key(h: &Harness<'_, PhotocraftApp>, id: u64, command: &str) -> Value {
    field(h, id, "overrides").get(command).cloned().unwrap_or(Value::Null)
}

// A free combination with no function key: no holder, no warning.
const FREE: (Key, Modifiers) = (Key::Y, Modifiers::COMMAND.plus(Modifiers::ALT).plus(Modifiers::SHIFT));
const FREE_TEXT: &str = "Cmd+Alt+Shift+Y";

#[test]
fn a_pressed_key_only_fills_the_field_and_enter_commits_it() {
    let (mut h, id) = dialog("edit.cut");
    assert!(!accept_enabled(&h), "Accept is off while no field is open");
    click_cell(&mut h, "Cmd+X");
    assert_eq!(field(&h, id, "capture"), json!(true));
    assert!(accept_enabled(&h), "and on while one is");
    press(&mut h, FREE.0, FREE.1);
    assert_eq!(field(&h, id, super::PENDING), json!(FREE_TEXT), "the key is in the field");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "but not in the set");
    assert_eq!(field(&h, id, "capture"), json!(true), "and the field stays open");
    assert_eq!(field(&h, id, "message"), json!(""), "a free key: no warning text");
    assert_eq!(field(&h, id, "messageWarning"), json!(false));
    assert!(h.query_by_label(&crate::shortcuts::pretty(FREE_TEXT)).is_some(), "the field shows the key pressed");
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(main_key(&h, id, "edit.cut"), json!(FREE_TEXT), "Enter commits it");
    assert_eq!(field(&h, id, "capture"), json!(false));
    assert_eq!(field(&h, id, super::PENDING), Value::Null);
    assert!(open(&h, id), "and the dialog stays open");
}

#[test]
fn escape_reverts_the_field_and_keeps_the_dialog_open() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, FREE.0, FREE.1);
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(open(&h, id), "Escape doesn't close the dialog");
    assert_eq!(field(&h, id, "capture"), json!(false));
    assert_eq!(field(&h, id, super::PENDING), Value::Null);
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "the old key stays");
    assert!(h.query_by_label(&crate::shortcuts::pretty("Cmd+X")).is_some(), "the cell shows it again");
}

#[test]
fn enter_and_escape_are_never_recorded() {
    let (mut h, id) = dialog("edit.cut");
    for m in [Modifiers::NONE, Modifiers::COMMAND, Modifiers::SHIFT] {
        click_cell(&mut h, "Cmd+X");
        press(&mut h, Key::Enter, m);
        assert!(open(&h, id), "{m:?}");
        assert_eq!(field(&h, id, "capture"), json!(false), "Enter closes the field ({m:?})");
        assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "an empty field commits nothing ({m:?})");
        click_cell(&mut h, "Cmd+X");
        press(&mut h, Key::Escape, m);
        assert!(open(&h, id), "{m:?}");
        assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "Escape isn't a key either ({m:?})");
    }
    assert!(field(&h, id, "alternates").get("edit.cut").is_none());
}

#[test]
fn accept_commits_the_field() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, FREE.0, FREE.1);
    click(&mut h, "Accept");
    assert_eq!(main_key(&h, id, "edit.cut"), json!(FREE_TEXT));
    assert_eq!(field(&h, id, "capture"), json!(false));
    assert!(!accept_enabled(&h), "the field is closed");
    // Add Shortcut's field commits the same way, as one more key.
    click(&mut h, "Add Shortcut");
    press(&mut h, Key::U, FREE.1);
    assert!(field(&h, id, "alternates").get("edit.cut").is_none(), "pending");
    click(&mut h, "Accept");
    assert_eq!(field(&h, id, "alternates")["edit.cut"], json!(["F2", "Cmd+Alt+Shift+U"]), "beside the default F2");
}

#[test]
fn moving_to_another_row_commits_the_field() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, FREE.0, FREE.1);
    // A click on Copy's Shortcut cell: Cut keeps the key, Copy's field opens.
    click_cell(&mut h, "Cmd+C");
    assert_eq!(main_key(&h, id, "edit.cut"), json!(FREE_TEXT), "Cut got the key");
    assert_eq!(field(&h, id, "selected"), json!("edit.copy"));
    assert_eq!(field(&h, id, "capture"), json!(true), "Copy's field is open");
    assert_eq!(field(&h, id, super::PENDING), Value::Null, "and empty");
    assert_eq!(main_key(&h, id, "edit.copy"), Value::Null);
}

#[test]
fn a_key_in_use_warns_with_the_reference_text_until_accepted() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::J, Modifiers::COMMAND);
    let want = format!("{} is already in use and will be removed from Layer > New > Layer Via Copy if accepted.", crate::shortcuts::pretty("Cmd+J"));
    assert_eq!(field(&h, id, "message"), json!(want));
    assert_eq!(field(&h, id, "messageWarning"), json!(true));
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "pending only");
    assert!(h.query_by_label("Accept and Go to Conflict").is_some());
    assert!(h.query_by_label("Undo Changes").is_some());
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+J"));
    assert_eq!(field(&h, id, "conflict")["with"], json!("layer.new.layerViaCopy"), "the conflict outlives the commit");
    assert_eq!(field(&h, id, "message"), json!(want));
    // A path ending in "…" drops it: Edit › Fade… reads "Edit > Fade".
    click_cell(&mut h, "Cmd+J");
    press(&mut h, Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    let fade = format!("{} is already in use and will be removed from Edit > Fade if accepted.", crate::shortcuts::pretty("Cmd+Shift+F"));
    assert_eq!(field(&h, id, "message"), json!(fade));
}

#[test]
fn accept_and_go_to_conflict_and_undo_changes_work_on_a_pending_key() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::C, Modifiers::COMMAND);
    click(&mut h, "Undo Changes");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null, "Undo Changes drops the pending key");
    assert_eq!(field(&h, id, "capture"), json!(false));
    assert!(field(&h, id, "conflict").is_null());
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::C, Modifiers::COMMAND);
    click(&mut h, "Accept and Go to Conflict");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+C"), "the pending key is accepted");
    assert_eq!(main_key(&h, id, "edit.copy"), json!(""), "and taken from Copy");
    assert_eq!(field(&h, id, "selected"), json!("edit.copy"));
    assert_eq!(field(&h, id, "capture"), json!(false));
}

#[test]
fn a_free_function_key_with_modifiers_warns() {
    let (mut h, id) = dialog("edit.cut");
    click_cell(&mut h, "Cmd+X");
    let m = Modifiers::COMMAND | Modifiers::SHIFT;
    press(&mut h, Key::F8, m);
    assert!(field(&h, id, "conflict").is_null(), "no command holds it");
    let want = format!("{} can be assigned to actions which will override this command.", crate::shortcuts::pretty("Cmd+Shift+F8"));
    assert_eq!(field(&h, id, "message"), json!(want));
    assert_eq!(field(&h, id, "messageWarning"), json!(true));
    click(&mut h, "Undo Changes");
    assert_eq!(main_key(&h, id, "edit.cut"), Value::Null);
    assert_eq!(field(&h, id, "capture"), json!(false));
    // Accept takes it.
    click_cell(&mut h, "Cmd+X");
    press(&mut h, Key::F8, m);
    click(&mut h, "Accept");
    assert_eq!(main_key(&h, id, "edit.cut"), json!("Cmd+Shift+F8"));
    // A plain function key has no warning.
    click_cell(&mut h, "Cmd+Shift+F8");
    press(&mut h, Key::F13, Modifiers::NONE);
    assert_eq!(field(&h, id, "messageWarning"), json!(false), "{}", field(&h, id, "message"));
}

#[test]
fn the_instructions_show_while_nothing_is_being_edited() {
    let (mut h, _) = dialog("edit.cut");
    let text = super::INSTRUCTIONS;
    assert!(text.starts_with("To edit keyboard shortcuts:\n1) Click on the New Set button"));
    assert!(h.query_by_label(text).is_some(), "no field open: the box is there");
    click_cell(&mut h, "Cmd+X");
    assert!(h.query_by_label(text).is_none(), "a field open: it goes");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.query_by_label(text).is_some(), "and comes back");
}
