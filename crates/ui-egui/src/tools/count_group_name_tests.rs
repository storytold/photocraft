//! #11766, #14707: the Count tool's folder button opens the "Count Group Name" dialog on the dialog host, pre-filled
//! with the next group's name; OK makes the group with the typed name and makes it current, Cancel makes nothing. The
//! group menu lists the groups, a separator and Rename…, which opens the same dialog for the current group. The
//! dialog belongs to the document activation that opened it, and a rename also to the document state it read.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// A 400 x 300 document with one count group (made by a canvas click) and the Count tool; the harness draws the
/// options bar.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = Tool::Count;
    app.run("count.add", json!({"x": 50, "y": 50})).unwrap();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            // The app's font families arrive with `setup_context`, after the first frame.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(3);
    h
}

fn groups(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().session.active().unwrap().doc.measurement.count_groups.iter().map(|g| g.name.clone()).collect()
}

fn history(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// The values of the one open dialog, after a host pass has loaded them.
fn dialog_fields(h: &mut Harness<'static, Box<PhotocraftApp>>) -> Option<Value> {
    crate::dialogs::testing::show(h.state_mut());
    let shown = h.state().framework.dialogs.open_dialogs();
    assert!(shown.len() <= 1, "at most one dialog is open");
    shown.first().map(|d| d.fields.clone().expect("the dialog lists its values"))
}

fn dialog_name(h: &mut Harness<'static, Box<PhotocraftApp>>) -> Option<String> {
    dialog_fields(h).map(|f| f["name"].as_str().expect("a name field").to_owned())
}

/// Types `name` into the open dialog and answers OK; the error is the refusal text.
fn type_and_ok(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str) -> Result<(), String> {
    let id = h.state().framework.dialogs.open_dialogs()[0].id;
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "name", json!(name)).unwrap();
    crate::dialogs::testing::confirm(h.state_mut(), id)
}

fn cancel(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let id = h.state().framework.dialogs.open_dialogs()[0].id;
    crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
}

#[test]
fn the_first_canvas_click_still_makes_count_group_1_without_a_dialog() {
    let mut h = harness();
    assert_eq!(groups(&h), vec!["Count Group 1".to_string()]);
    assert!(dialog_fields(&mut h).is_none());
}

#[test]
fn the_folder_button_opens_the_dialog_and_ok_makes_the_named_group() {
    let mut h = harness();
    let before = history(&h);
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert_eq!(dialog_name(&mut h).as_deref(), Some("Count Group 2"), "pre-filled with the next group's name");
    assert_eq!(groups(&h).len(), 1, "nothing made yet");
    type_and_ok(&mut h, "Cells").unwrap();
    assert!(dialog_fields(&mut h).is_none(), "OK closes the dialog");
    assert_eq!(groups(&h), vec!["Count Group 1".to_string(), "Cells".to_string()]);
    assert_eq!(h.state().session.active().unwrap().doc.measurement.active_count_group, 1, "the new group is the current one");
    assert_eq!(history(&h), before + 1, "one history state");
}

#[test]
fn cancel_makes_nothing() {
    let mut h = harness();
    let before = history(&h);
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert!(dialog_fields(&mut h).is_some());
    cancel(&mut h);
    assert!(dialog_fields(&mut h).is_none());
    assert_eq!(groups(&h).len(), 1);
    assert_eq!(history(&h), before, "no history state");
}

#[test]
fn the_group_menu_lists_rename_after_the_groups_and_it_renames_the_current_group() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "Second"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Group").click();
    h.run_steps(2);
    let rename = h.get_by_label("Rename…");
    let second = h.get_by_label("Second");
    assert!(rename.rect().top() > second.rect().top(), "Rename… follows the groups");
    rename.click();
    h.run_steps(3);
    assert_eq!(dialog_name(&mut h).as_deref(), Some("Second"), "Rename… renames the current group");
    assert_eq!(groups(&h).len(), 2, "picking Rename… doesn't switch groups");
    let before = history(&h);
    type_and_ok(&mut h, "Nuclei").unwrap();
    assert_eq!(groups(&h), vec!["Count Group 1".to_string(), "Nuclei".to_string()]);
    assert_eq!(history(&h), before + 1);
}

/// CodeRabbit on #14707: the dialog belongs to the document that opened it. After a switch to another document, OK is
/// refused, so it can't rename a group of another document or name its new group.
#[test]
fn switching_documents_refuses_ok() {
    for rename in [false, true] {
        let mut h = harness();
        if rename {
            crate::count_group_name_ui::open_rename(h.state_mut(), 0).unwrap();
        } else {
            crate::count_group_name_ui::open_new(h.state_mut()).unwrap();
        }
        assert!(dialog_fields(&mut h).is_some(), "the dialog is open (rename {rename})");
        h.state_mut().run("file.new", json!({"width": 100, "height": 100})).unwrap();
        h.state_mut().run("count.newGroup", json!({"name": "Mine"})).unwrap();
        assert!(type_and_ok(&mut h, "Wrong place").is_err(), "OK is refused (rename {rename})");
        assert_eq!(groups(&h), vec!["Mine".to_string()], "the other document's groups are untouched (rename {rename})");
    }
}

/// Coming back to a document is a new activation, so the dialog opened before the round trip refuses OK too.
#[test]
fn leaving_a_document_and_coming_back_refuses_ok() {
    let mut h = harness();
    crate::count_group_name_ui::open_new(h.state_mut()).unwrap();
    assert!(dialog_fields(&mut h).is_some());
    h.state_mut().run("file.new", json!({"width": 100, "height": 100})).unwrap();
    h.state_mut().session.set_active(0);
    assert!(type_and_ok(&mut h, "Late").is_err());
    assert_eq!(groups(&h), vec!["Count Group 1".to_string()], "nothing made");
}

/// CodeRabbit on #14707: groups may share a name and any edit can shift the indexes, so any change to the document
/// after the rename dialog opened refuses OK, even one that restores the same names.
#[test]
fn a_document_change_refuses_the_rename_but_not_a_new_group() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    crate::count_group_name_ui::open_rename(h.state_mut(), 1).unwrap();
    assert!(dialog_fields(&mut h).is_some());
    h.state_mut().run("count.deleteGroup", json!({"group": 0})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    assert!(type_and_ok(&mut h, "Renamed").is_err(), "the rename is refused though the names are [B, B, B] again");
    cancel(&mut h);
    assert!(groups(&h).iter().all(|g| g == "B"), "no group renamed");
    // A new group does not depend on an index: a change to the document leaves OK working.
    crate::count_group_name_ui::open_new(h.state_mut()).unwrap();
    assert!(dialog_fields(&mut h).is_some());
    h.state_mut().run("count.setGroup", json!({"group": 0, "active": true})).unwrap();
    type_and_ok(&mut h, "Fresh").unwrap();
    assert_eq!(groups(&h).last().map(String::as_str), Some("Fresh"));
}
