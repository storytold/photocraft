//! #14450, the dialog half: the Menus tab's Set row, PhotoCraft's own widgets in `menus_body`
//! (the framework draws its Set row only for its own Keyboard Shortcuts tab). The dropdown lists
//! the built-in set first and the saved sets after it, names the set in use "(modified)" while the
//! menus differ from it, and Save / New / Delete follow the measured enablement and tooltips. Save
//! on the built-in set and New go to the system Save dialog, whose default name is "<current set
//! name> Copy.MNU" and whose file name, without the extension, names the new set.
//!
//! The switch-set message box (Yes / No / Cancel and "Don't show again") and the Delete
//! confirmation are here too (#14450, slice 3): both are `StandardPrompt`s on the app's dialog
//! host, opened by the pass that saw the click and answered through `dialogs::PROMPTS`. Switching
//! an unmodified set still asks nothing; switching a modified one asks, and only its "No" (and
//! "Yes" after the save) loads the other set.

use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::menu_sets::{DEFAULTS_NAME, MAX_SETS, modified};
use serde_json::{Value, json};

use super::shortcuts::press_ok;
use super::shortcuts::{SWITCH_PREF, SetDelete, SetSwitch};
use crate::PhotocraftApp;

/// The row's three icon buttons, by their measured tooltips: an icon button's label is its
/// accessible name, so the tooltip text is what a query finds.
const SAVE: &str = "Save all changes to the current set of menus";
const NEW: &str = "Create a new set based on the current set of menus";
const DELETE: &str = "Delete the current set of menus";

/// The two message boxes' buttons, and the switch box's measured text and check box (the double
/// space after "shortcuts." is in the reference's text too).
const YES: &str = "Yes";
const NO: &str = "No";
const CANCEL: &str = "Cancel";
const AGAIN: &str = "Don't show again";
const SWITCH_ASK: &str = "Selecting a different set will result in a loss of all unsaved shortcuts.  Save your changes before switching sets?";
const TITLE: &str = "Keyboard Shortcuts and Menus";

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

/// Opens the dropdown and picks the entry `label`, as its row does. This alone does not answer a
/// switch question that the pick opens. The row is clicked, not focused and confirmed with Enter:
/// the held Enter would answer the question's default button as soon as it opens.
fn pick(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    h.get_by_label(label).click();
    h.run_steps(2);
}

/// Whether the switch question is open.
fn asked(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.query_by_label(SWITCH_ASK).is_some()
}

/// Presses the open message box's button `label`.
fn answer(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(2);
}

/// The open question of the family `T` (`SetSwitch` or `SetDelete`), for the field the test wants
/// to set the way automation does.
fn question<T: std::any::Any>(app: &PhotocraftApp) -> Option<craft::ui::dialog::DialogId> {
    crate::dialogs::alerts::prompt_tagged::<T>(app)
}

/// Chooses the set `label`, answering the switch question "No" when the pick opens one: that drops
/// the unsaved edits and loads the set, which is what the reference's "No" does. A switch from an
/// unmodified set asks nothing, so this is today's pick for that case.
fn choose(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    pick(h, label);
    if asked(h) {
        answer(h, NO);
    }
}

/// Presses the tab's own `Show All Menu Items` button, and holds the pointer over it for a pass
/// first. PhotoCraft draws that button itself (`menus_body`'s `ui.button`), and since the craft-libs
/// repin that landed on `main` (`e7e877ac5`, with its note that a modal cover now shows and absorbs
/// hits from its first pass) a press that arrives in the same pass as the pointer's first touch of
/// one of our own widgets is absorbed by the cover and does nothing - at two passes, and at four and
/// six after a reopen. Move over it, let the pass run, press: the click then lands at every pass
/// count measured (two, three, four, six; on the open and on a reopen). The framework's own controls
/// in this dialog - the Set combo, the three icon buttons - take the press without the hover pass,
/// which is why only the tests that edit the rows were red. A hand always moves before it presses;
/// `click` alone teleports, and production code needed no change.
fn show_all(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    h.get_by_label("Show All Menu Items").hover();
    h.run_steps(1);
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
    // The order over the open list, top to bottom: the built-in set first, the saved sets after it
    // in name order.
    let built_in = h.get_by_label(DEFAULTS_NAME).rect().top();
    let painting = h.get_by_label("Painting").rect().top();
    let web = h.get_by_label("Web").rect().top();
    assert!(built_in < painting && painting < web, "the open list runs {built_in}, {painting}, {web}: built-in first, then the saved sets by name");
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

/// Choosing another set while the rows are as saved loads its menus without a question; while they
/// differ it asks first, and its "No" loads them anyway. OK writes them and the set in use.
#[test]
fn choosing_another_set_asks_only_when_modified_and_no_loads_its_menus_and_ok_uses_it() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    assert_eq!(h.state().session.prefs().menus.hidden, ["edit.fade"], "Painting's menus are in use");
    pick(&mut h, "Web");
    assert!(!asked(&h), "the rows are as saved, so the switch asks nothing");
    assert_eq!(row_value(&h), "Web");
    assert!(greyed(&h, SAVE), "and Web is unmodified as the rows stand");
    show_all(&mut h);
    assert_eq!(row_value(&h), "Web (modified)");
    pick(&mut h, DEFAULTS_NAME);
    assert!(asked(&h), "the unsaved edit makes the switch ask");
    answer(&mut h, NO);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the unsaved edit went with the switch, as after the reference's No");
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
/// "<current set name> Copy.MNU", and the chosen file's name without the extension names the set.
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
    assert_eq!(asked[0].file_name.as_deref(), Some("PhotoCraft Defaults (modified) Copy.MNU"), "the measured default name");
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
    assert_eq!(asked[0].file_name.as_deref(), Some("PhotoCraft Defaults Copy.MNU"), "the measured default name of an unmodified set");
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

/// A dotted set name survives the Save dialog's name: the default carries the `.MNU` extension, so
/// the chosen file's name without its extension is `Web.v2 Copy` and not the `Web` a stem of
/// `Web.v2 Copy` would leave. The existing `Web` set stays exactly as saved.
#[test]
fn a_new_set_from_a_dotted_set_name_keeps_the_whole_name_and_leaves_the_shorter_one() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["Web.v2 Copy.MNU".into()])]));
    let mut h = harness(files.clone());
    h.state_mut().run("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "Web"})).unwrap();
    h.state_mut().run("edit.menus", json!({"hide": ["layer.mergeVisible"], "saveSet": "Web.v2"})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Web.v2", "the dotted set is in use, as saved");
    button(&h, NEW).click();
    h.run_steps(4);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "New goes to the Save dialog");
    assert_eq!(asked[0].file_name.as_deref(), Some("Web.v2 Copy.MNU"), "the default name carries the extension the stem strips");
    assert_eq!(row_value(&h), "Web.v2 Copy", "the new set keeps the whole dotted name");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Web", "Web.v2", "Web.v2 Copy"], "no set is replaced by the stem of the dotted name");
    assert_eq!(p.menu_sets["Web"].hidden, ["edit.fade"], "the shorter name's set is untouched");
    assert_eq!(p.menu_set, "Web.v2 Copy", "the new set is the one in use");
}

/// Delete asks first; its Yes drops the set, and OK moves the row back to the built-in set, which
/// is greyed again; the menus on screen stay, as the engine's `deleteSet` leaves them.
#[test]
fn delete_removes_the_set_with_ok_and_leaves_the_menus_on_screen() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, DELETE).click();
    h.run_steps(2);
    assert!(files.requests().is_empty(), "Delete goes to a message box, not to the file service");
    assert!(question::<SetDelete>(h.state()).is_some(), "the confirmation asks first");
    assert_eq!(row_value(&h), "Painting", "nothing is deleted while the box waits");
    answer(&mut h, YES);
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

/// Cancel drops everything the row did, including a set its Delete question confirmed: the runtime
/// never saw a change.
#[test]
fn cancel_drops_a_set_the_row_deleted() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the row moved to the built-in set");
    crate::prefs_ui::shortcuts::cancel(h.state_mut());
    h.run_steps(2);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "Cancel wrote nothing");
    assert_eq!(p.menu_set, "Painting", "and the set in use stayed");
}

/// A hand-edited settings file can name a set the engine itself refuses to write (the built-in
/// name). The row still lists it and can delete it, but Save is greyed: it could not store under
/// that name. Picking away and pressing OK must not lose the session: the OK payload carries only
/// what the engine would store, so the reserved-name set leaves the preferences with it.
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
    choose(&mut h, DEFAULTS_NAME);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the built-in entry gives the built-in menus");
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    assert_eq!(
        h.query_all_by_label(DEFAULTS_NAME).count(),
        2,
        "the built-in entry and the reserved-name set carry the same label, as the engine keeps both names"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_sets.is_empty(), "the set the engine would refuse is not in the payload, so the preferences lose it");
    assert!(p.menu_set.is_empty(), "and the row is back at the built-in set");
    assert!(p.menus.hidden.is_empty(), "the built-in menus the pick loaded are the preferences' now");
    assert!(!h.state().ui.status_error, "OK wrote, so no failure line");
}

/// A settings file can hold a set that hides an item that cannot be hidden (`file.close`): the
/// prefs paths store it, but the engine's `edit.menus` would refuse it. OK filters its payload to
/// what the engine would store, so picking another set and pressing OK lands every write instead
/// of losing the session, and the stale entry is repaired out of the set.
#[test]
fn ok_writes_through_a_stale_prefs_set_the_engine_would_not_store() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    h.state_mut().run("prefs.set", json!({"values": {"menuSet": "Old", "menuSets": {"Old": {"hidden": ["file.close", "edit.fade"], "colors": {}}}}})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Old (modified)", "the menus in use differ from the stale set");
    choose(&mut h, DEFAULTS_NAME);
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_set.is_empty(), "the set switch landed: the built-in set is in use");
    assert_eq!(p.menu_sets["Old"].hidden, ["edit.fade"], "the id the engine refuses to hide is out of the stored set");
    assert!(h.state().prefs_rt.shortcuts.is_none(), "OK closed the dialog with its writes, not on a refused call");
    assert!(!h.state().ui.status_error, "and no failure line was shown");
}

/// The set list is capped where the engine caps it (`MAX_SETS`). With the list already full, New
/// shows the engine's own cap line (the framework's in-dialog bar is not drawn on the app's own
/// tabs, so the line goes to the app's status line), the dialog stays open, and OK still writes:
/// a full list can no longer drop the session.
#[test]
fn new_at_the_set_cap_refuses_with_an_error_line_and_ok_still_writes() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["Set Sixty Five.MNU".into()])]));
    let mut h = harness(files.clone());
    h.state_mut().session.edit_prefs(|p| {
        for n in 0..MAX_SETS {
            p.menu_sets.insert(format!("Set {n:02}"), Default::default());
        }
    });
    open_menus(&mut h);
    button(&h, NEW).click();
    h.run_steps(3);
    assert!(files.requests().is_empty(), "at the cap New does not ask the Save dialog");
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the row is unchanged");
    let line = format!("at most {MAX_SETS} menu sets");
    assert!(h.state().ui.status_error && h.state().ui.status.contains(&line), "the engine's cap line shows: {}", h.state().ui.status);
    assert!(h.state().prefs_rt.shortcuts.is_some(), "and the dialog stays open");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.len(), MAX_SETS, "no 65th set reached the preferences");
    assert!(h.state().prefs_rt.shortcuts.is_none(), "OK closed the dialog with its writes landed");
}

/// A name the engine refuses cannot become a set: New whose Save dialog answers "PhotoCraft
/// Defaults" leaves the row where it was, stores nothing in the preferences, and shows the
/// engine's refusal line on the app's status line; the dialog stays open.
#[test]
fn a_picked_name_the_engine_refuses_shows_an_error_and_stores_no_set() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["PhotoCraft Defaults.MNU".into()])]));
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, NEW).click();
    h.run_steps(4);
    assert_eq!(files.requests().len(), 1, "New asked the Save dialog for a name");
    assert_eq!(row_value(&h), "Painting", "the refused name stores no set: the row is where it was");
    assert!(h.state().ui.status_error && h.state().ui.status.contains("is the built-in set"), "the engine's refusal line shows: {}", h.state().ui.status);
    assert!(h.state().prefs_rt.shortcuts.is_some(), "and the dialog stays open");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "no set reached the preferences");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "OK wrote the sets as they stood");
    assert!(h.state().prefs_rt.shortcuts.is_none(), "and closed the dialog on its writes, not on a refusal");
}

// ---------------------------------------------------------------- the two message boxes (#14450, slice 3)

/// Answers the open question the way automation does: `set_json_field` picks the button and
/// `dialogs::confirm` presses it. Used where a label would be ambiguous ("Cancel" is also the
/// dialog's own button).
fn answer_by_choice(h: &mut Harness<'_, Box<PhotocraftApp>>, id: craft::ui::dialog::DialogId, choice: &str) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "choice", json!(choice)).unwrap();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(2);
}

/// The switch question's measured box: its text, title, three buttons and check box, and nothing
/// switched while it waits.
#[test]
fn switching_a_modified_set_asks_with_yes_no_cancel_and_a_check_box() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    let titles = h.query_all_by_label(TITLE).count();
    let cancels = h.query_all_by_label(CANCEL).count();
    show_all(&mut h);
    pick(&mut h, "Web");
    assert!(asked(&h), "the measured text, double space and all");
    assert!(question::<SetSwitch>(h.state()).is_some(), "the box is the switch question");
    assert!(h.query_all_by_label(TITLE).count() > titles, "the box carries the dialog's title");
    for label in [YES, NO] {
        assert!(!h.get_by_role_and_label(Role::Button, label).accesskit_node().is_disabled(), "{label} is live");
    }
    assert_eq!(h.query_all_by_label(CANCEL).count(), cancels + 1, "the box adds a Cancel button of its own");
    assert!(h.query_by_label(AGAIN).is_some(), "and the Don't show again check box");
    assert_eq!(row_value(&h), "Painting (modified)", "nothing switched while the box waits");
}

/// No drops the unsaved edits and loads the other set, as measured.
#[test]
fn no_of_the_switch_question_drops_the_edits_and_loads_the_other_set() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    pick(&mut h, "Web");
    assert!(asked(&h));
    answer(&mut h, NO);
    assert!(!asked(&h), "the box closed on the answer");
    assert_eq!(row_value(&h), "Web", "the other set loaded");
    assert!(greyed(&h, SAVE), "and the dropped edit is gone: Web is as saved again");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Web", "the set in use is what OK writes");
    assert_eq!(p.menus.hidden, ["edit.fade", "layer.mergeVisible"], "Web's menus are the ones in use");
    assert_eq!(p.menu_sets["Painting"].hidden, ["edit.fade"], "No did not save the edit into its set");
}

/// Cancel (the box's own button) stays on the current set with its edits.
#[test]
fn cancel_of_the_switch_question_keeps_the_set_and_its_edits() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    pick(&mut h, "Web");
    let id = question::<SetSwitch>(h.state()).expect("the switch question");
    answer_by_choice(&mut h, id, "cancel");
    assert!(!asked(&h), "the box closed on the answer");
    assert_eq!(row_value(&h), "Painting (modified)", "the row stayed on its set, marker and all");
    assert!(!greyed(&h, SAVE), "and Save is still live: the edit is there to save");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Painting", "the set in use did not move");
    assert!(p.menus.hidden.is_empty(), "the rows' edit reached the preferences as any other OK would");
    assert_eq!(p.menu_sets["Painting"].hidden, ["edit.fade"], "the saved set kept its copy");
    assert!(modified(p), "and the row still reads modified when it reopens");
}

/// The switch box outlives the dialog it was asked on: the dialog's Cancel and `ui.dialog.cancel`
/// take the draft away and leave the box standing, and the next open starts a fresh draft of the
/// preferences. The answer belongs to the draft that was asked, so in a reopened dialog it loads
/// nothing and stores nothing.
#[test]
fn a_switch_answer_that_arrives_after_a_reopen_changes_nothing() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    pick(&mut h, "Web");
    let id = question::<SetSwitch>(h.state()).expect("the switch question");
    assert_eq!(row_value(&h), "Painting (modified)", "the box waits with the row unmoved");
    crate::prefs_ui::shortcuts::cancel(h.state_mut());
    open_menus(&mut h);
    assert!(question::<SetSwitch>(h.state()).is_some(), "the box stands on after its dialog was cancelled");
    assert_eq!(row_value(&h), "Painting", "the reopened dialog is a fresh draft of the preferences");
    answer_by_choice(&mut h, id, "proceed");
    assert_eq!(row_value(&h), "Painting", "the answer switched nothing in the session that never saw the box");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Painting", "and the reopened dialog wrote the sets as it stood");
    assert_eq!(p.menus.hidden, ["edit.fade"], "Painting's menus are the ones in use");
    assert_eq!(p.menu_sets["Painting"].hidden, ["edit.fade"], "the answer stored nothing into Painting");
    assert_eq!(p.menu_sets["Web"].hidden, ["edit.fade", "layer.mergeVisible"], "nor anything into Web");
}

/// Yes on a modified saved set stores it first, without a file dialog (the measured Save rule for
/// a saved set), and then loads the other set.
#[test]
fn yes_of_the_switch_question_saves_a_modified_saved_set_and_switches() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    pick(&mut h, "Web");
    answer(&mut h, YES);
    assert!(files.requests().is_empty(), "a saved set saves without the Save dialog");
    assert_eq!(row_value(&h), "Web", "and the switch went ahead");
    assert!(greyed(&h, SAVE));
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_sets["Painting"].hidden.is_empty(), "Yes stored the rows as they stood into Painting first");
    assert_eq!(p.menu_set, "Web");
    assert_eq!(p.menus.hidden, ["edit.fade", "layer.mergeVisible"], "Web's menus are the ones in use");
}

/// Yes on the modified built-in set cannot overwrite it: the save asks the system Save dialog for a
/// name, "<current set name> Copy" as its default, and the switch follows once the name is in.
#[test]
fn yes_on_the_modified_built_in_set_asks_for_a_name_and_then_switches() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["My Menus.MNU".into()])]));
    let mut h = harness(files.clone());
    h.state_mut().run("edit.menus", json!({"hide": ["edit.fade"], "saveSet": "Painting"})).unwrap();
    h.state_mut().run("edit.menus", json!({"useSet": "", "hide": ["layer.mergeVisible"]})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the built-in set is in use, with an edit");
    pick(&mut h, "Painting");
    assert!(asked(&h));
    answer(&mut h, YES);
    h.run_steps(3);
    let asked_files = files.requests();
    assert_eq!(asked_files.len(), 1, "Yes on the built-in set asks the Save dialog for a name");
    assert_eq!(asked_files[0].file_name.as_deref(), Some("PhotoCraft Defaults (modified) Copy.MNU"), "the measured default name");
    assert_eq!(row_value(&h), "Painting", "and the switch followed the name");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["My Menus"].hidden, ["layer.mergeVisible"], "the edits were saved under the picked name");
    assert_eq!(p.menu_sets["Painting"].hidden, ["edit.fade"], "the set switched to is unchanged");
    assert_eq!(p.menu_set, "Painting");
    assert_eq!(p.menus.hidden, ["edit.fade"], "Painting's menus are the ones in use");
    assert!(!modified(p), "and the rows match the set they loaded");
}

/// A name dialog cancelled after Yes keeps the current set and its edits, and leaves nothing
/// waiting behind it: the next Save stores under the name it is answered with and moves the row
/// there, not to the set the cancelled switch was heading for.
#[test]
fn yes_whose_name_dialog_is_cancelled_keeps_the_set_and_its_edits() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Cancelled, Picked::Paths(vec!["My Menus.MNU".into()])]));
    let mut h = harness(files.clone());
    with_saved_sets(h.state_mut());
    h.state_mut().run("edit.menus", json!({"useSet": "", "hide": ["layer.mergeVisible"]})).unwrap();
    crate::prefs_ui::shortcuts::cancel(h.state_mut());
    open_menus(&mut h);
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the built-in set, with an edit");
    pick(&mut h, "Web");
    answer(&mut h, YES);
    h.run_steps(3);
    assert_eq!(files.requests().len(), 1, "Yes asked the Save dialog for a name");
    assert_eq!(row_value(&h), "PhotoCraft Defaults (modified)", "the cancelled dialog switched nothing");
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(files.requests().len(), 2, "Save on the built-in set asked its own name");
    assert_eq!(row_value(&h), "My Menus", "and the row moved to that name, not to the set the switch was heading for");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "My Menus");
    assert_eq!(p.menu_sets["My Menus"].hidden, ["layer.mergeVisible"], "the edit was saved once, under the picked name");
    assert_eq!(p.menu_sets["Web"].hidden, ["edit.fade", "layer.mergeVisible"], "Web is as saved");
}

/// The "Don't show again" box, ticked with Yes, is kept with the other dialog preferences: the next
/// switch asks nothing and switches at once.
#[test]
fn dont_show_again_on_the_switch_question_skips_it_for_the_next_switch() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    pick(&mut h, "Web");
    let ctx = h.ctx.clone();
    let id = question::<SetSwitch>(h.state()).expect("the switch question");
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    answer(&mut h, YES);
    assert_eq!(row_value(&h), "Web", "Yes saved the modified set and switched");
    let kept = h.state().session.prefs().dialogs.get(SWITCH_PREF).and_then(|v| v.get("skip")).and_then(Value::as_bool);
    assert_eq!(kept, Some(true), "the ticked box is kept with the other dialog preferences");
    // The next switch: no box, and the edits go as they do after the reference's No.
    show_all(&mut h);
    pick(&mut h, DEFAULTS_NAME);
    assert!(!asked(&h), "the next switch asks nothing");
    assert_eq!(row_value(&h), DEFAULTS_NAME, "and it went ahead at once");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menu_set.is_empty(), "the built-in set is in use");
    assert!(p.menus.hidden.is_empty(), "its menus are the ones in use");
    assert_eq!(p.menu_sets["Web"].hidden, ["edit.fade", "layer.mergeVisible"], "Web kept its saved copy");
}

/// Delete asks with the measured question, naming the set, and only Yes removes anything; its No
/// keeps the set, the row and the menus. The question is not one to silence, so it has no check box.
#[test]
fn no_of_the_delete_question_keeps_the_set_the_row_and_the_menus() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    with_saved_sets(h.state_mut());
    open_menus(&mut h);
    button(&h, DELETE).click();
    h.run_steps(2);
    assert!(h.query_by_label("Are you sure you want to send Painting to the recycle bin?").is_some(), "the measured question names the set");
    assert!(question::<SetDelete>(h.state()).is_some(), "the box is the Delete question");
    assert!(!h.get_by_role_and_label(Role::Button, YES).accesskit_node().is_disabled(), "Yes is live");
    assert!(!h.get_by_role_and_label(Role::Button, NO).accesskit_node().is_disabled(), "No is live");
    assert!(h.query_by_label(AGAIN).is_none(), "no Don't show again box on this question");
    assert_eq!(row_value(&h), "Painting", "nothing is deleted while the box waits");
    answer(&mut h, NO);
    assert_eq!(row_value(&h), "Painting", "No keeps the set in use");
    assert!(!greyed(&h, DELETE), "and Delete is still live");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting", "Web"], "No removed nothing");
    assert_eq!(p.menu_set, "Painting");
}

// ---------------------------------------------------------- the `.mnu` files (#14450, slice 4)
//
// A set lives as a `.mnu` file in a menu-sets folder of the settings directory: the row's Save on a
// saved set writes it without a dialog, Save on the built-in set and New ask the Save dialog, which
// starts in that folder offering "Menu Customization (*.MNU)", Delete takes the file with the set, and
// the folder's files are sets the row can pick. These tests give the app a settings store of its own
// (a real temp folder, so `menu_sets::folder` is a real folder) and drive the same row as above.

use photocraft_engine::menu_sets;

/// Two saved sets whose items all have a measured number: Painting hides File › New and colours
/// Edit › Fade red; Web hides Layers › Merge Visible. Painting is the set in use, as saved.
fn with_numbered_sets(app: &mut PhotocraftApp) {
    app.run("edit.menus", json!({"hide": ["file.new"], "color": {"edit.fade": "red"}, "saveSet": "Painting"})).unwrap();
    app.run("edit.menus", json!({"hide": ["layer.mergeVisible"], "saveSet": "Web"})).unwrap();
    app.run("edit.menus", json!({"useSet": "Painting"})).unwrap();
}

/// A harness on an app whose settings store is the folder `dir`, so the menu-sets folder is
/// `dir/menu-sets` for real. The store's own `config.toml` is never flushed by these tests.
fn file_harness(files: Arc<ScriptedFiles>, dir: &std::path::Path) -> Harness<'static, Box<PhotocraftApp>> {
    let dir = dir.to_path_buf();
    Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let options = craft::settings::OpenOptions::new(craft::settings::Location::Dir(dir));
        let services = crate::Services {
            files: Some(files),
            settings: Some(craft::settings::SettingsStore::<photocraft_engine::prefs::SavedPrefs>::open_with(options)),
            ..Default::default()
        };
        PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), services)
    })
}

/// The measured root element of a PhotoCraft set file for the set `name`.
fn root(name: &str) -> String {
    format!("<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/{name}={name}\" modified=\"0\">")
}

/// The text of the set file `name` in the settings folder `dir`, or `None` where it is not there.
fn set_file(dir: &std::path::Path, name: &str) -> Option<String> {
    std::fs::read_to_string(menu_sets::folder(dir).join(menu_sets::file_name(name))).ok()
}

/// Puts `text` in the settings folder `dir` as the file of the set `name`, making the folder.
fn write_set_file(dir: &std::path::Path, name: &str, text: &[u8]) -> std::path::PathBuf {
    let folder = menu_sets::folder(dir);
    std::fs::create_dir_all(&folder).expect("the menu-sets folder");
    let path = folder.join(menu_sets::file_name(name));
    std::fs::write(&path, text).expect("the set file");
    path
}

/// Save on a modified saved set writes its file in the menu-sets folder, without a dialog, and the
/// file holds one row per changed item — only what the set changed, by the measured numbers.
#[test]
fn save_on_a_saved_set_writes_its_file_in_the_menu_sets_folder() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let files = Arc::new(ScriptedFiles::new());
    let mut h = file_harness(files.clone(), tmp.path());
    with_numbered_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    assert!(files.requests().is_empty(), "the measured Save on a saved set asks no dialog");
    assert!(set_file(tmp.path(), "Painting").is_none(), "the file waits for OK, as the preferences do");
    ok(&mut h);
    assert!(menu_sets::folder(tmp.path()).is_dir(), "the menu-sets folder came with the first saved set");
    let text = set_file(tmp.path(), "Painting").expect("Painting's file");
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"), "the measured UTF-8 XML: {text}");
    assert!(text.contains(&root("Painting")), "the root as measured: {text}");
    assert!(text.contains("id=\"1154\" parentMenuID=\"3\" menuType=\"0\" hidden=\"0\" color=\"1\""), "Edit › Fade kept its red: {text}");
    assert!(!text.contains("id=\"10\""), "File › New is shown again, so it has no row: {text}");
    assert_eq!(text.matches("<command").count(), 1, "one element per changed item: {text}");
}

/// The Save dialog of Save on the built-in set and of New starts in the menu-sets folder, suggests
/// "<current set name> Copy.MNU" there, and offers the "Menu Customization (*.MNU)" file type.
#[test]
fn the_save_dialog_starts_in_the_menu_sets_folder_offering_the_menu_customization_type() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let files = Arc::new(ScriptedFiles::new());
    let mut h = file_harness(files.clone(), tmp.path());
    open_menus(&mut h);
    button(&h, NEW).click();
    h.run_steps(4);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "New goes to the Save dialog");
    assert_eq!(asked[0].directory.as_deref(), Some(menu_sets::folder(tmp.path()).as_path()), "the dialog starts in the menu-sets folder");
    assert_eq!(asked[0].file_name.as_deref(), Some("PhotoCraft Defaults Copy.MNU"), "the measured default name");
    let names: Vec<&str> = asked[0].filters.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Menu Customization"], "one file type, as measured");
    let exts: Vec<&str> = asked[0].filters.iter().flat_map(|f| f.extensions.iter().map(String::as_str)).collect();
    assert_eq!(exts, ["mnu"], "(*.MNU): {exts:?}");
    assert_eq!(asked[0].suggested_extension.as_deref(), Some("mnu"), "a name typed without one keeps the set extension");
}

/// The file the dialog chose is the set's file: it is written where the user sent it, and holds the
/// menus as they were saved.
#[test]
fn a_picked_name_writes_the_set_file_where_the_dialog_chose_it() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let chosen = menu_sets::folder(tmp.path()).join("Darkroom Copy.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![chosen.clone()])]));
    let mut h = file_harness(files.clone(), tmp.path());
    h.state_mut().run("edit.menus", json!({"hide": ["file.new"], "color": {"layer.mergeVisible": "blue"}})).unwrap();
    open_menus(&mut h);
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Darkroom Copy", "the chosen name without its extension names the set");
    assert!(std::fs::read_to_string(&chosen).is_err(), "the file waits for OK, as the preferences do");
    ok(&mut h);
    let text = std::fs::read_to_string(&chosen).expect("OK wrote the file where the dialog chose");
    assert!(text.contains(&root("Darkroom Copy")), "the set name is the file stem: {text}");
    assert!(text.contains("id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\""), "hidden without a colour: {text}");
    assert!(text.contains("id=\"1139\" parentMenuID=\"41\" menuType=\"1\" hidden=\"0\" color=\"5\""), "the panel item's own colour: {text}");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["Darkroom Copy"].hidden, ["file.new"], "and the preferences hold it too");
}

/// A `.mnu` file the folder holds is a set the row can name: the reference application's own file,
/// with its root name, its byte-order mark and its numbers, loads as our command ids.
#[test]
fn a_set_file_in_the_menu_sets_folder_is_a_set_the_row_can_choose() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let foreign = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photoshop-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/From Photoshop=From Photoshop\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"1139\" parentMenuID=\"41\" menuType=\"1\" hidden=\"1\" color=\"0\"/>\n",
        "</photoshop-menu-settings>\n",
    );
    write_set_file(tmp.path(), "From Photoshop", foreign.as_bytes());
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    open_menus(&mut h);
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    assert!(h.query_by_label("From Photoshop").is_some(), "the folder's file is in the list");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    choose(&mut h, "From Photoshop");
    assert_eq!(row_value(&h), "From Photoshop");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["From Photoshop"].hidden, ["layer.mergeVisible"], "the file's number became our command id");
    assert_eq!(p.menu_set, "From Photoshop", "and it is the set in use");
    assert_eq!(p.menus.hidden, ["layer.mergeVisible"], "choosing the set loaded its menus");
}

/// Yes of the Delete question takes the set's file with it, so the folder does not offer the set
/// back; No leaves the file where it is. Both act at OK, where the row's writes land.
#[test]
fn yes_of_the_delete_question_takes_the_set_file_with_it() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    with_numbered_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    assert!(set_file(tmp.path(), "Painting").is_none(), "the row's Save waits for OK, like the preferences");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, NO);
    ok(&mut h);
    assert!(set_file(tmp.path(), "Painting").is_some(), "No kept the set and its file");
    assert!(set_file(tmp.path(), "Web").is_none(), "a set the row never saved has no file: only the row's Save writes one");
    // The saved file is now the set: reopening reads it, and Delete with Yes takes it away.
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Painting", "the folder's file reads back as the saved set");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    ok(&mut h);
    assert!(set_file(tmp.path(), "Painting").is_none(), "Yes took the file with the set");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Web"], "and the set went from the preferences with it");
}

/// A file in the folder that does not read is said out loud — the issue's rule that a malformed
/// `.mnu` is an error, never a crash — and the row still opens with the sets that do read.
#[test]
fn a_broken_set_file_in_the_folder_is_said_and_the_row_still_opens() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    write_set_file(tmp.path(), "Broken", b"<photocraft-menu-settings version=\"2\"");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Good=Good\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"31\" parentMenuID=\"2\" menuType=\"0\" hidden=\"0\" color=\"5\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    write_set_file(tmp.path(), "Good", text.as_bytes());
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    open_menus(&mut h);
    assert_eq!(row_value(&h), DEFAULTS_NAME, "the row opened on the set in use");
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error && line.contains("Broken.MNU"), "the unreadable file is named: {line}");
    h.get_by_role_and_label(Role::ComboBox, "Set").click();
    h.run_steps(2);
    assert!(h.query_by_label("Good").is_some(), "and the readable one is still a set");
    assert!(h.query_by_label("Broken").is_none(), "the unreadable file is not a set of garbage");
}

/// A cancelled dialog writes nothing at all, the set's file included: the row's Save only marks the
/// file for writing, and OK is what writes it — the same rule slice 2 pinned for the preferences.
#[test]
fn a_cancelled_dialog_writes_no_set_file() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    with_numbered_sets(h.state_mut());
    open_menus(&mut h);
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    crate::prefs_ui::shortcuts::cancel(h.state_mut());
    h.run_steps(2);
    assert!(set_file(tmp.path(), "Painting").is_none(), "Cancel reached neither the folder nor the preferences");
    assert!(!menu_sets::folder(tmp.path()).exists(), "and it did not make the menu-sets folder either");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["Painting"].hidden, ["file.new"], "the set is exactly as it was saved");
    // The same Save, pressed through OK, is what puts the file there.
    open_menus(&mut h);
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    ok(&mut h);
    let text = set_file(tmp.path(), "Painting").expect("OK wrote the file the row asked for");
    assert_eq!(text.matches("<command").count(), 1, "one row for the one changed item left: {text}");
    assert_eq!(h.state().session.prefs().menu_sets["Painting"].hidden, [] as [String; 0], "and the preferences agree with the file");
}

/// A set's file speaks for the items it numbers and for nothing else: the copy the preferences hold
/// stays authoritative for items with no row in `REF_ITEMS`. The repro: hide Edit › Copy and Edit ›
/// Paste - neither has a measured number, so the file of "Mine" holds no row at all - Save the
/// modified built-in set under that name through the dialog, OK, and open the row again.
#[test]
fn a_set_file_speaks_only_for_the_items_it_numbers_and_the_preferences_fill_the_rest() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let chosen = menu_sets::folder(tmp.path()).join("Mine.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![chosen])]));
    let mut h = file_harness(files, tmp.path());
    h.state_mut().run("edit.menus", json!({"hide": ["edit.copy", "edit.paste"]})).unwrap();
    open_menus(&mut h);
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Mine", "the picked file names the new set");
    ok(&mut h);
    let text = set_file(tmp.path(), "Mine").expect("OK wrote Mine's file");
    assert_eq!(text.matches("<command").count(), 0, "neither item has a measured number, so the file holds no row: {text}");
    assert_eq!(h.state().session.prefs().menu_sets["Mine"].hidden, ["edit.copy", "edit.paste"], "the preferences hold both");
    // Reopening reads the folder: the file has nothing to say about these two items, so the set is
    // still the one that was saved and the row is not "modified" beside it.
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Mine", "the thinner file did not replace the saved set");
    // And choosing away and back keeps both items: choosing a set loads its stored copy, so an empty
    // file must not become an empty set.
    choose(&mut h, DEFAULTS_NAME);
    choose(&mut h, "Mine");
    assert_eq!(row_value(&h), "Mine", "Mine still reads as saved");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["Mine"].hidden, ["edit.copy", "edit.paste"], "reopening lost nothing from the set");
    assert_eq!(p.menus.hidden, ["edit.copy", "edit.paste"], "and nothing from the menus in use");
}

/// Yes of the Delete question takes the file the row was going to write for that set, where the Save
/// dialog chose to put it - not only the file the menu-sets folder would have held. The picked path is
/// that set's file.
#[test]
fn yes_of_the_delete_question_takes_the_file_the_dialog_chose() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    // A file outside the menu-sets folder, already holding an older copy of the set: the Save dialog
    // picks exactly this name, so this is the set's file.
    let chosen = tmp.path().join("Elsewhere.MNU");
    std::fs::write(&chosen, b"an older copy of the set, written elsewhere").expect("the file the dialog will pick");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![chosen.clone()])]));
    let mut h = file_harness(files, tmp.path());
    h.state_mut().run("edit.menus", json!({"hide": ["file.new"]})).unwrap();
    open_menus(&mut h);
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Elsewhere", "the picked name named the set");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    ok(&mut h);
    assert!(!chosen.exists(), "the Delete took the set's own file, where it stands");
    assert!(set_file(tmp.path(), "Elsewhere").is_none(), "the folder never got a copy of its own");
    let p = h.state().session.prefs();
    assert!(!p.menu_sets.contains_key("Elsewhere"), "and the set went from the preferences with it");
}

/// Three files in the folder that do not read are said as three: the first named in full and how many
/// more there were, so one broken file does not hide two.
#[test]
fn several_unreadable_set_files_are_said_together() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    write_set_file(tmp.path(), "Broken One", b"<photocraft-menu-settings version=\"2\"");
    write_set_file(tmp.path(), "Broken Two", b"menus, not xml");
    write_set_file(tmp.path(), "Broken Three", b"<photocraft-menu-settings version=\"3\" />");
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    open_menus(&mut h);
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error, "the line is an error line");
    assert!(line.contains("Broken One.MNU"), "the first is named: {line}");
    assert!(line.contains("and 2 more"), "and the other two are counted, not left unsaid: {line}");
}

/// The same at OK, for writes that failed: both refusals are said, not only the one that happened last.
#[test]
fn two_sets_whose_files_cannot_be_written_are_said_together() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let one = tmp.path().join("Blocked One.MNU");
    let two = tmp.path().join("Blocked Two.MNU");
    std::fs::create_dir(&one).expect("a folder in the way of the first set's file");
    std::fs::create_dir(&two).expect("and in the way of the second's");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![one.clone()]), Picked::Paths(vec![two.clone()])]));
    let mut h = file_harness(files, tmp.path());
    h.state_mut().run("edit.menus", json!({"hide": ["file.new"]})).unwrap();
    open_menus(&mut h);
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Blocked One", "the first pick named its set");
    button(&h, NEW).click();
    h.run_steps(4);
    ok(&mut h);
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error, "the line is an error line");
    assert!(line.contains("and 1 more"), "both refused writes are said: {line}");
}

/// Where the file and the preferences disagree about an item the file does number, the file wins:
/// the folder is where a saved set lives, so its row is the newer word on that item. Items the file
/// has no row for keep the preferences' copy.
#[test]
fn a_file_row_decides_its_own_item_and_the_preferences_keep_the_rest() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Mine=Mine\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"0\" color=\"4\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    write_set_file(tmp.path(), "Mine", text.as_bytes());
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    // The preferences' copy of Mine hides File › New (the file shows it and colours it green) and
    // hides Edit › Copy, which the file has no number for.
    h.state_mut()
        .run("edit.menus", json!({"sets": {"Mine": {"hidden": ["file.new", "edit.copy"], "colors": {"edit.paste": "red"}}}, "activeSet": "Mine"}))
        .unwrap();
    h.state_mut().run("edit.menus", json!({"useSet": "Mine"})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Mine (modified)", "the folder's row is the newer word on File › New, so the menus in use differ from the merged set");
    choose(&mut h, DEFAULTS_NAME);
    choose(&mut h, "Mine");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets["Mine"].hidden, ["edit.copy"], "the file's row for File › New won, the unnumbered item stayed: {:?}", p.menu_sets["Mine"].hidden);
    assert_eq!(p.menu_sets["Mine"].colors["edit.paste"], "red", "an item the file has no row for keeps its colour");
    assert_eq!(p.menu_sets["Mine"].colors["file.new"], "green", "the file's own row brings its own colour");
}

/// The file the Save dialog chose stays that set's file: the row's Save on a saved set re-queues
/// the set for writing, and that must not throw the chosen destination away and send the set back to
/// the menu-sets folder. The mirror of `yes_of_the_delete_question_takes_the_file_the_dialog_chose`,
/// for the Save half.
#[test]
fn a_second_save_on_a_set_keeps_the_file_the_dialog_chose() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    // Outside the menu-sets folder: the folder's own name for the set is a different file.
    let chosen = tmp.path().join("Elsewhere.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![chosen.clone()])]));
    let mut h = file_harness(files, tmp.path());
    h.state_mut().run("edit.menus", json!({"hide": ["file.new"]})).unwrap();
    open_menus(&mut h);
    // The built-in set has no saved copy: the name and the file come from the Save dialog.
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Elsewhere", "the picked name named the set");
    // A second edit and a second Save, which asks no dialog: the set's file is the one already chosen.
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    assert!(!chosen.exists(), "the second Save still waits for OK, like the first");
    ok(&mut h);
    let text = std::fs::read_to_string(&chosen).expect("OK wrote the set where the dialog chose it, not back in the menu-sets folder");
    assert!(set_file(tmp.path(), "Elsewhere").is_none(), "the menu-sets folder got no copy of a set the dialog put elsewhere");
    assert!(text.contains(&root("Elsewhere")), "the file is that set's: {text}");
    assert_eq!(text.matches("<command").count(), 0, "the file holds the set as the second Save left it: {text}");
}

/// Yes of the switch question saves the set the way its Save button does, so it too keeps the file
/// the Save dialog chose for that set instead of re-queuing it to the menu-sets folder.
#[test]
fn yes_of_the_switch_question_saves_to_the_file_the_dialog_chose() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let chosen = tmp.path().join("Elsewhere.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![chosen.clone()])]));
    let mut h = file_harness(files, tmp.path());
    h.state_mut().run("edit.menus", json!({"sets": {"Web": {"hidden": ["layer.mergeVisible"], "colors": {}}}, "activeSet": ""})).unwrap();
    h.state_mut().run("edit.menus", json!({"hide": ["file.new"]})).unwrap();
    open_menus(&mut h);
    assert_eq!(row_value(&h), format!("{DEFAULTS_NAME} (modified)"), "the built-in set was edited, so Save asks for a name");
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Elsewhere", "the picked file named the set");
    show_all(&mut h);
    pick(&mut h, "Web");
    answer(&mut h, YES);
    ok(&mut h);
    let text = std::fs::read_to_string(&chosen).expect("Yes saved the set into the file the dialog chose");
    assert!(set_file(tmp.path(), "Elsewhere").is_none(), "and not into a file of the folder's own naming");
    assert!(text.contains(&root("Elsewhere")), "the file is that set's: {text}");
    assert_eq!(text.matches("<command").count(), 0, "the file holds the rows as Yes left them: {text}");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Web", "and the switch it promised came after the save");
}

/// A set the menu-sets folder was found holding keeps that very file: the folder's own name for it,
/// not the name `path_of` builds back from the set name. A file the folder lists as `Darkroom.mnu`
/// is Darkroom's file, and a Save on the set writes it there rather than starting a second file.
/// Its Delete says the same: Yes of the Delete question takes the found file away, whatever it is
/// spelled as, and the set does not stand in the folder under the name's own spelling.
#[test]
fn a_set_the_folder_found_is_saved_back_to_the_file_it_was_found_in() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let folder = menu_sets::folder(tmp.path());
    std::fs::create_dir_all(&folder).expect("the menu-sets folder");
    // The extension as the folder's own list spells it; the measured one is "MNU".
    let found = folder.join("Darkroom.mnu");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Darkroom=Darkroom\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    std::fs::write(&found, text).expect("the set file the folder holds");
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    open_menus(&mut h);
    choose(&mut h, "Darkroom");
    assert_eq!(row_value(&h), "Darkroom", "the folder's file is a set the row can use");
    show_all(&mut h);
    button(&h, SAVE).click();
    h.run_steps(3);
    assert!(std::fs::read_to_string(&found).expect("the file the folder found").contains("id=\"10\""), "the row's Save waits for OK, like any other");
    ok(&mut h);
    let left: Vec<String> = std::fs::read_dir(&folder).expect("the menu-sets folder").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["Darkroom.mnu".to_string()], "the write went to the file the folder scan found, not to a name of the set's own");
    let kept = std::fs::read_to_string(&found).expect("the found file is still there");
    assert!(!kept.contains("id=\"10\""), "and it holds the set as the row saved it: File › New is shown again: {kept}");
    assert!(kept.contains(&root("Darkroom")), "the set keeps its name: {kept}");
    // Its Delete says the same. Yes takes the very file the folder was found holding - spelled as
    // the folder spells it - away, rather than aiming at `Darkroom.MNU`, the file the set's own
    // name would name and this folder does not hold. On a case-insensitive folder the two spellings
    // are one file and no aim can miss; on a case-sensitive one (CI) an aim at the wrong spelling
    // would leave the found file standing, and the listing below says it went.
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Darkroom", "the saved file reads back as the set in use");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    ok(&mut h);
    assert!(!found.exists(), "the Delete took the differently spelled file the folder was found holding");
    let left: Vec<String> = std::fs::read_dir(&folder).expect("the menu-sets folder").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(left.is_empty(), "and nothing stands under the set name's own spelling either: {left:?}");
    let p = h.state().session.prefs();
    assert!(!p.menu_sets.contains_key("Darkroom"), "the set went from the preferences with the file");
}

/// File discovery cannot make the OK payload bigger than the engine's cap: with the preferences
/// holding one set and the folder holding `MAX_SETS` files, the set that does not fit never reaches
/// the row, the engine's own cap line says why, and OK still writes what the row does carry. A set
/// the preferences hold is not the one dropped.
#[test]
fn the_folder_s_sets_join_the_row_only_up_to_the_engine_s_cap() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let folder = menu_sets::folder(tmp.path());
    std::fs::create_dir_all(&folder).expect("the menu-sets folder");
    for n in 0..MAX_SETS {
        let name = format!("Folder Set {n:02}");
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/{name}={name}\" modified=\"0\">\n</photocraft-menu-settings>\n"
        );
        std::fs::write(folder.join(menu_sets::file_name(&name)), body).expect("a set file the folder holds");
    }
    let mut h = file_harness(Arc::new(ScriptedFiles::new()), tmp.path());
    h.state_mut().session.edit_prefs(|p| {
        p.menu_sets.insert("Held By Prefs".into(), Default::default());
    });
    open_menus(&mut h);
    let session = h.state().prefs_rt.shortcuts.as_ref().expect("the dialog is open");
    assert_eq!(session.menus.sets.len(), MAX_SETS, "the row never holds more sets than the engine stores: {:?}", session.menus.sets.keys().collect::<Vec<_>>());
    assert!(session.menus.sets.contains_key("Held By Prefs"), "a set the preferences hold is not the one dropped");
    assert!(!session.menus.sets.contains_key("Folder Set 63"), "the file past the cap is not offered: {:?}", session.menus.sets.keys().collect::<Vec<_>>());
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error && line.contains(&format!("at most {MAX_SETS} menu sets")), "the engine's cap line says why: {line}");
    // The row can still commit what it carries: choosing a set marks the draft, and OK's payload fits.
    choose(&mut h, "Folder Set 00");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_sets.len(), MAX_SETS, "OK wrote the sets the row carried, no 65th with it: {:?}", p.menu_sets.keys().collect::<Vec<_>>());
    assert!(p.menu_sets.contains_key("Held By Prefs"), "and the preferences' own set is among them");
    assert!(!p.menu_sets.contains_key("Folder Set 63"), "the file past the cap never reached the preferences");
    assert!(h.state().prefs_rt.shortcuts.is_none(), "the dialog closed on its writes, not on a refused call");
}

/// OK writes each subject it was asked to write: the shortcut call and the menus are separate, and a
/// refused `edit.keyboardShortcuts` must not take the Menus tab's staged sets, hidden items, colours
/// and set-file writes down with it. The line that is said names the write that actually failed.
/// The repro: a hand-edited settings file holds an action's function key the engine cannot store, so
/// the shortcut write is refused; the menus edit still lands.
#[test]
fn ok_writes_the_menus_even_when_the_keyboard_shortcuts_are_refused() {
    let mut h = harness(Arc::new(ScriptedFiles::new()));
    let broken = format!("{}Broken", photocraft_engine::actions_cmds::PLAY_SHORTCUT_PREFIX);
    h.state_mut().session.edit_prefs(|p| {
        // `save_keys` copies the actions' function keys back out of the preferences, so this entry is
        // in OK's shortcut payload whatever the dialog's rows hold.
        p.shortcuts.insert(broken.clone(), "Cmd+Banana+Q".into());
        p.menus.hidden = vec!["edit.fade".into()];
    });
    open_menus(&mut h);
    show_all(&mut h);
    ok(&mut h);
    let p = h.state().session.prefs();
    assert!(p.menus.hidden.is_empty(), "the Menus tab's write landed even though the shortcuts were refused: {:?}", p.menus.hidden);
    assert_eq!(p.shortcuts.get(&broken).map(String::as_str), Some("Cmd+Banana+Q"), "the refused shortcut call changed no binding");
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error, "the refused write is said as an error");
    assert!(line.contains("is not a valid shortcut") && line.contains("Cmd+Banana+Q"), "and it names the shortcut failure: {line}");
    assert!(!line.contains("The menus were not saved"), "the menus were saved, so nothing claims they were not: {line}");
    assert!(h.state().prefs_rt.shortcuts.is_none(), "OK closed the dialog on the writes it could make");
}

/// The queued set-file operations are resolved per set name at OK, not by running every write and
/// then every delete: a Delete the user answered Yes to, followed by a Save that makes the same name
/// again before OK, leaves that set's file standing with the new set in it. A Delete with no later
/// Save still takes the file away, which `yes_of_the_delete_question_takes_the_set_file_with_it` and
/// `yes_of_the_delete_question_takes_the_file_the_dialog_chose` pin.
#[test]
fn a_set_recreated_under_a_deleted_name_keeps_its_file_through_ok() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Painting=Painting\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    write_set_file(tmp.path(), "Painting", text.as_bytes());
    // The recreation answers the Save dialog with exactly that file: one name, one path, two queued
    // operations.
    let same = menu_sets::folder(tmp.path()).join("Painting.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![same.clone()])]));
    let mut h = file_harness(files, tmp.path());
    open_menus(&mut h);
    choose(&mut h, "Painting");
    assert_eq!(row_value(&h), "Painting", "the folder's file is the set the row works on");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    assert_eq!(row_value(&h), format!("{DEFAULTS_NAME} (modified)"), "the Delete moved the row back to the built-in set and left its menus on screen");
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Painting", "and the same name is a set again before OK");
    ok(&mut h);
    let kept = std::fs::read_to_string(&same).expect("the later Save won: the file stands, not written and then deleted");
    assert!(kept.contains("id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\""), "it holds the recreated set: {kept}");
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Painting", "and the set is in the preferences again");
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting"], "one set of that name, not a leftover");
}

/// The queued delete answers for the file the deleted set stood in, taken when the Delete question
/// was answered - not for whatever path the set's name later travels to. The repro: the folder holds
/// `menu-sets/Painting.MNU`, the row deletes Painting, and a Save gives the same name a different
/// file, `elsewhere/Painting.MNU`. OK writes the new file and must still take the old one away, or
/// the next open reads the deleted set back from the folder.
#[test]
fn a_set_recreated_under_a_deleted_name_elsewhere_takes_the_old_file_with_it() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Painting=Painting\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    let old = write_set_file(tmp.path(), "Painting", text.as_bytes());
    // The recreation answers the Save dialog with a different file of the same name: one name, two
    // paths, and the delete still stands for the path it was made for.
    let elsewhere = tmp.path().join("elsewhere");
    let recreated = elsewhere.join("Painting.MNU");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![recreated.clone()])]));
    let mut h = file_harness(files, tmp.path());
    open_menus(&mut h);
    choose(&mut h, "Painting");
    assert_eq!(row_value(&h), "Painting", "the folder's file is the set the row works on");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    assert_eq!(row_value(&h), format!("{DEFAULTS_NAME} (modified)"), "the Delete moved the row back to the built-in set and left its menus on screen");
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Painting", "and the same name is a set again before OK");
    ok(&mut h);
    assert!(!old.exists(), "the queued delete took the file at its own path; the later Save elsewhere does not spare it");
    let kept = std::fs::read_to_string(&recreated).expect("the later Save wrote where the dialog sent it");
    assert!(kept.contains("id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\""), "it holds the recreated set: {kept}");
    // The next open reads the folder again: with the old path gone the set comes from the
    // preferences only, and the folder offers no file of that name back to the row.
    open_menus(&mut h);
    assert_eq!(row_value(&h), "Painting", "no leftover file makes the reopened set read as modified");
    let session = h.state().prefs_rt.shortcuts.as_ref().expect("the dialog is open");
    assert_eq!(session.menus.file_paths.get("Painting"), None, "the menu-sets folder holds no file of that name any more");
    ok(&mut h);
    let p = h.state().session.prefs();
    assert_eq!(p.menu_set, "Painting", "and the set is still the one in use");
    assert_eq!(p.menu_sets.keys().collect::<Vec<_>>(), ["Painting"], "one set of that name, not a leftover");
    assert_eq!(p.menu_sets["Painting"].hidden, ["file.new"], "the reopened set is the one that was saved elsewhere");
}

/// A Save that cannot write must not cost the user the file of the set they deleted: only a write that
/// LANDED answers for a path. The repro: the folder holds `menu-sets/Painting.MNU`, the row deletes
/// Painting, and a Save sends the same name to `elsewhere/Painting.MNU` - a folder is standing there,
/// so the atomic move is refused, as in `two_sets_whose_files_cannot_be_written_are_said_together`. OK
/// says the refused write and leaves the original file standing with its set in it, so the next open
/// reads the set back from the folder. The sibling
/// `a_set_recreated_under_a_deleted_name_elsewhere_takes_the_old_file_with_it` pins the other half:
/// when that write does land, the old file goes with the set.
#[test]
fn a_recreated_set_the_save_cannot_write_leaves_the_old_file_standing() {
    let tmp = tempfile::tempdir().expect("a settings folder");
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<photocraft-menu-settings version=\"2\" filename=\"$$$/FileName/Presets/MenuCustomization/Painting=Painting\" modified=\"0\">\n",
        "  <command kind=\"static\" id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\"/>\n",
        "</photocraft-menu-settings>\n",
    );
    let old = write_set_file(tmp.path(), "Painting", text.as_bytes());
    // The recreation answers the Save dialog with a path no write can take: a folder stands where the
    // file would go, so the staged file cannot be moved in and `write_set_at` comes back refused.
    let recreated = tmp.path().join("elsewhere").join("Painting.MNU");
    std::fs::create_dir_all(&recreated).expect("a folder in the way of the Save");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![recreated.clone()])]));
    let mut h = file_harness(files, tmp.path());
    open_menus(&mut h);
    choose(&mut h, "Painting");
    assert_eq!(row_value(&h), "Painting", "the folder's file is the set the row works on");
    button(&h, DELETE).click();
    h.run_steps(2);
    answer(&mut h, YES);
    assert_eq!(row_value(&h), format!("{DEFAULTS_NAME} (modified)"), "the Delete moved the row back to the built-in set and left its menus on screen");
    button(&h, SAVE).click();
    h.run_steps(4);
    assert_eq!(row_value(&h), "Painting", "and the same name is a set again before OK");
    ok(&mut h);
    let kept = std::fs::read_to_string(&old).expect("a refused Save leaves the file the Delete was made for standing, or the set is gone from the disk");
    assert!(kept.contains("id=\"10\" parentMenuID=\"2\" menuType=\"0\" hidden=\"1\" color=\"0\""), "and it still holds the set it was written with: {kept}");
    let (found, unreadable) = menu_sets::read_sets(&menu_sets::folder(tmp.path()));
    assert!(unreadable.is_empty(), "the folder still reads clean: {unreadable:?}");
    let names: Vec<String> = found.iter().map(|file| file.name.clone()).collect();
    assert!(names.iter().any(|name| name == "Painting"), "the next open reads the set back from the folder: {names:?}");
    let line = h.state().ui.status.clone();
    assert!(h.state().ui.status_error, "the refused Save is said as an error: {line}");
    assert!(line.contains("elsewhere"), "and the line names the destination that refused it: {line}");
    let p = h.state().session.prefs();
    assert!(p.menu_sets.contains_key("Painting"), "the set is still a saved set in the preferences: {:?}", p.menu_sets.keys().collect::<Vec<_>>());
}
