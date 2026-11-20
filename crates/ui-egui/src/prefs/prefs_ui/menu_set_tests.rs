//! #14450, the dialog half: the Menus tab's Set row, PhotoCraft's own widgets in `menus_body`
//! (the framework draws its Set row only for its own Keyboard Shortcuts tab). The dropdown lists
//! the built-in set first and the saved sets after it, names the set in use "(modified)" while the
//! menus differ from it, and Save / New / Delete follow the measured enablement and tooltips. Save
//! on the built-in set and New go to the system Save dialog, whose default name is "<current set
//! name> Copy" and whose file name, without the extension, names the new set.
//!
//! The switch-set message box (Yes / No / Cancel) and the Delete confirmation are the next slice,
//! so these rows show switching dropping the unsaved edits, which is what the reference application
//! does after its "No".

use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::menu_sets::{DEFAULTS_NAME, modified};
use serde_json::json;

use super::shortcuts::press_ok;
use crate::PhotocraftApp;

/// The row's three icon buttons, by their measured tooltips: an icon button's label is its
/// accessible name, so the tooltip text is what a query finds.
const SAVE: &str = "Save all changes to the current set of menus";
const NEW: &str = "Create a new set based on the current set of menus";
const DELETE: &str = "Delete the current set of menus";

/// A harness on a fresh app whose file service answers picks from `files`.
fn harness(files: Arc<ScriptedFiles>) -> Harness<'static, Box<PhotocraftApp>> {
    Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = crate::Services { files: Some(files), ..Default::default() };
        PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), services)
    })
}

/// Two saved sets, and the menus in use are Painting's so the row starts unmodified: Painting
/// hides Edit › Fade, Web hides that and Layers › Merge Visible.
fn with_saved_sets(app: &mut PhotocraftApp) {
    app.run("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "Painting"})).unwrap();
    app.run("edit.menus", json!({"hide": ["layer.mergeVisible"], "saveSet": "Web"})).unwrap();
    app.run("edit.menus", json!({"useSet": "Painting"})).unwrap();
}

/// Opens the dialog on the Menus tab.
fn open_menus(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    crate::prefs_ui::open_shortcuts(h.state_mut(), 1);
    h.run_steps(2);
}

/// The Set dropdown's shown value.
fn row_value(h: &Harness<'_, Box<PhotocraftApp>>) -> String {
    h.get_by_role_and_label(Role::ComboBox, "Set").value().unwrap_or_default().to_string()
}

/// The row's icon button whose accessible name is `label`. The button role, not just the label: a
/// shown tooltip carries the same text.
fn button<'a>(h: &'a Harness<'_, Box<PhotocraftApp>>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(Role::Button, label)
}

/// Whether the row's button named `label` is greyed out.
fn greyed<'a>(h: &'a Harness<'_, Box<PhotocraftApp>>, label: &'a str) -> bool {
    button(h, label).accesskit_node().is_disabled()
}

/// Chooses the set `label` in the dropdown, as picking its row does.
fn choose(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    h.get_by_label(label).focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

/// Shows every menu item, which is an edit of the menus in use.
fn show_all(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    h.get_by_label("Show All Menu Items").click();
    h.run_steps(2);
}

/// Presses the dialog's OK and lets the writes land.
fn ok(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    let ctx = h.ctx.clone();
    press_ok(h.state_mut(), &ctx);
    h.run_steps(2);
}

/// The dropdown lists the built-in set first and the saved sets after it, and names the set in use.
#[test]
fn the_set_dropdown_lists_the_built_in_set_first_and_the_saved_sets_after() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Painting", "the set in use, saved as it is");
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    for name in [DEFAULTS_NAME, "Painting", "Web"] {
        assert!(h.query_by_label(name).is_some(), "{name} is not in the open list");
    }
}

/// The first edit renames the set in use "(modified)" in place; Save drops the marker.
#[test]
fn the_row_shows_modified_only_while_the_menus_differ_from_the_set_in_use() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Painting", "no marker while the set is as saved");
    show_all(&mut h);
    assert_eq!(row_value(&h), "Painting (modified)", "the first edit renames the set in place");
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    assert!(h.query_by_label("Painting (modified)").is_some(), "the marker shows in the list too");
    assert!(h.query_by_label("Web").is_some(), "only the set in use carries it");
    assert!(h.query_by_label(DEFAULTS_NAME).is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    button(&h, SAVE).click();
    h.run_steps(2);
    assert_eq!(row_value(&h), "Painting", "Save drops the marker");
    assert!(
        h.state().session.prefs().menu_sets["Painting"].hidden.iter().any(|id| id == "edit.fade"),
        "Save in the dialog is not yet written to the preferences"
    );
}

/// Save is greyed while the set is as saved and live once the menus differ from it, including for
/// a modified built-in set.
#[test]
fn save_is_greyed_while_the_set_is_as_saved_and_live_when_it_is_modified() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert!(greyed(&h, SAVE), "Save is greyed while the set is as saved");
    show_all(&mut h);
    assert!(!greyed(&h, SAVE), "Save is live while the set is modified");

    // The built-in set with an edit: live too, and the menus in use are the built-in's once the
    // row is back at the built-in set.
    let mut plain = harness(Arc::new(ScriptedFiles::new()));
    plain.state_mut().run("edit.menus", json!({"hide": ["edit.fade"]})).unwrap();
    open_menus(&mut plain);
    assert_eq!(row_value(&plain), "PhotoCraft Defaults (modified)", "the menus differ from the built-in set");
    assert!(!greyed(&plain, SAVE), "Save is live for a modified built-in set");
    plain.state_mut().run("edit.menus", json!({"reset": true})).unwrap();
    // The dialog's draft is its own: the row reopens as unmodified.
    crate::prefs_ui::shortcuts::cancel(plain.state_mut());
    open_menus(&mut plain);
    assert_eq!(row_value(&plain), DEFAULTS_NAME);
    assert!(greyed(&plain, SAVE), "and Save is greyed again");
}

/// Delete is greyed for the built-in set and live for a saved one; New is always live.
#[test]
fn delete_is_greyed_for_the_built_in_set_and_live_for_a_saved_one() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert!(!greyed(&h, DELETE), "Delete is live for the saved set in use");
    assert!(!greyed(&h, NEW), "New is always enabled");
    choose(&mut h, DEFAULTS_NAME);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the dropdown starts at the built-in set");
    assert!(greyed(&h, DELETE), "Delete is greyed for the built-in set");
    show_all(&mut h);
    assert!(greyed(&h, DELETE), "an edit does not make the built-in set deletable");
}

/// Choosing another set loads that set's menus into the rows; OK writes them and the set in use.
#[test]
fn choosing_another_set_loads_its_menus_and_ok_uses_it() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert_eq!(h.state().session.prefs().menus.hidden, ["edit.fade"], "Painting's menus are in use");
    choose(&mut h, "Web");
    assert_eq!(row_value(&h), "Web");
    assert!(greyed(&h, SAVE), "and Web is unmodified as the rows stand");
    show_all(&mut h);
    assert_eq!(row_value(&h), "Web (modified)");
    choose(&mut h, DEFAULTS_NAME);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the unsaved edit went with the switch, as it does after the reference's No");
    assert!(greyed(&h, SAVE));
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_set.is_empty(), "the built-in set is in use");
    assert!(p.menus.hidden.is_empty(), "its menus are empty: the switch loaded them");
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "the saved sets are still there");
}

/// Save on a modified saved set stores it in the set without asking for a file.
#[test]
fn save_on_a_saved_set_overwrites_it_without_a_dialog() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    assert!(files.requests().is_empty(), "a saved set saves without the Save dialog");
    assert_eq!(row_value(&h), "Painting", "the name loses the marker");
    assert!(greyed(&h, SAVE));
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_sets["Painting"].hidden.is_empty(), "the set holds the menus as they ended up");
    assert!(p.menu_sets["Web"].hidden.iter().any(|id| id == "edit.fade"), "the other set is untouched");
    assert!(!modified(p), "the saved set and the menus in use agree");
}

/// OK does not fold an unsaved edit into the set: only Save does, which is what keeps the marker
/// meaning something.
#[test]
fn ok_writes_the_menus_but_only_save_writes_them_into_the_set() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menus.hidden.is_empty(), "the menus the dialog showed are the preferences'");
    assert_eq!(p.menu_sets["Painting"].hidden, ["edit.fade"], "the set keeps its saved copy");
    assert_eq!(p.menu_set, "Painting");
    assert!(modified(p), "and the row still reads modified when it reopens");
}

/// Save on the modified built-in set asks the system Save dialog for a name: the default is
/// "<current set name> Copy", and the chosen file's name without the extension names the set.
#[test]
fn save_on_the_built_in_set_asks_for_a_name_and_the_file_names_the_set() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["My Menus.MNU".into()])]));
    let mut h = harness(files.clone());
    h.state_mut().run("edit.menus", json!({"hide": ["edit.fade"]})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)");
    button(&h, SAVE).click();
    h.run_steps(4);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "Save on the built-in set goes to the Save dialog");
    assert_eq!(asked[0].file_name.as_deref(), Some("PhotoCraft Defaults (modified) Copy"), "the measured default name");
    assert_eq!(row_value(&h), "My Menus", "the set name is the file name without the extension");
    assert!(greyed(&h, SAVE), "the new set holds the menus as they are");
    assert!(!greyed(&h, DELETE), "and it is a saved set, so Delete is live");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "My Menus");
    assert_eq!(p.menu_sets["My Menus"].hidden, ["edit.fade"], "the menus as they ended up");
    assert!(!modified(p), "the saved set and the menus in use agree");
}

/// New, always live, makes a set from the menus in use under the name the dialog picks.
#[test]
fn new_always_asks_for_a_name_and_saves_the_menus_in_use_under_it() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["Copy Of Defaults.MNU".into()])]));
    let mut h = harness(files.clone());
    open_menus(&mut h);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the row starts at the built-in set");
    assert!(!greyed(&h, NEW), "New is always enabled");
    button(&h, NEW).click();
    h.run_steps(4);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "New goes to the Save dialog");
    assert_eq!(asked[0].file_name.as_deref(), Some("PhotoCraft Defaults Copy"), "the measured default name of an unmodified set");
    assert_eq!(row_value(&h), "Copy Of Defaults");
    assert!(greyed(&h, SAVE), "the new set is exactly the menus in use");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(
        (p.menu_set.as_str(), p.menu_sets["Copy Of Defaults"].hidden.as_slice()),
        ("Copy Of Defaults", &[] as &[String]),
        "New saved the built-in menus"
    );
    assert!(!modified(p));
}

/// Delete drops the set with OK and moves the row back to the built-in set, which is greyed again;
/// the menus on screen stay, as the engine's `deleteSet` leaves them.
#[test]
fn delete_removes_the_set_with_ok_and_leaves_the_menus_on_screen() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, DELETE).click();
    h.run_steps(3);
    assert!(files.requests().is_empty(), "the Delete confirmation is the next slice");
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the row is back at the built-in set; the shown menus still hide Edit › Fade");
    assert!(greyed(&h, DELETE), "and Delete is greyed again");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Web"], "Painting is gone");
    assert!(p.menu_set.is_empty(), "the built-in set is in use");
    assert_eq!(p.menus.hidden, ["edit.fade"], "the menus on screen stayed");
    assert!(modified(p), "and they differ from the built-in set");
}

/// A cancelled Save dialog leaves the row and the sets exactly as they were.
#[test]
fn a_cancelled_name_dialog_changes_nothing() {
    let files = Arc::new(ScriptedFiles::new()); // nothing queued: every pick is Cancelled
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, NEW).click();
    h.run_steps(4);
    assert_eq!(files.requests().len(), 1, "the dialog was asked");
    assert_eq!(row_value(&h), "Painting", "Cancel leaves the row on its set");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "and the sets");
}

/// Cancel drops everything the row did, including a deleted set: the runtime never saw a change.
#[test]
fn cancel_drops_a_set_the_row_deleted() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, DELETE).click();
    h.run_steps(2);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the row moved to the built-in set");
    crate::prefs_ui::shortcuts::cancel(h.state_mut());
    h.run_steps(2);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "Cancel wrote nothing");
    assert_eq!(p.menu_set, "Painting", "and the set in use stayed");
}

/// A hand-edited settings file can name a set the engine itself refuses to write (the built-in
/// name). The row still lists it and can delete it, but Save is greyed: it could not store under
/// that name.
#[test]
fn a_hand_edited_set_named_like_the_built_in_one_cannot_be_saved_but_can_be_deleted() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    h.state_mut().session.edit_prefs(|p| {
        p.menu_set = DEFAULTS_NAME.into();
        p.menu_sets.insert(DEFAULTS_NAME.into(), Default::default());
        p.menus.hidden = vec!["edit.fade".into()];
    });
    open_menus(&mut h);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the row names the set in use, marker and all");
    assert!(greyed(&h, SAVE), "Save is greyed for a name the engine refuses to write");
    assert!(!greyed(&h, DELETE), "Delete can still clear it");
}
