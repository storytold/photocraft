//! BUG-TYPE-TOOL-UNCHANGED-EDIT-NO-HISTORY-STATE (#12050): ending a text edit that changed
//! nothing still adds one `Edit Type` history state.
//!
//! Measured in the reference application: click into an existing text layer, change nothing and
//! end the edit with Escape (when it commits), Ctrl+Enter, the Commit button or a click on the
//! canvas outside the text: one **Edit Type Layer** state is added and the layer is unchanged.
//! The Cancel button discards the session and adds no state. An edit that did change something
//! still gives one state, not two.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// A 400x200 document with one committed type layer: that layer active and the Type tool picked.
fn app() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    let id = LayerId(app.run("type.create", json!({"text": "Hello", "size": 24, "x": 40, "y": 100})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    app.ui.tool = Tool::Type;
    (app, id)
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn text_of(app: &PhotocraftApp, id: LayerId) -> String {
    let LayerContent::Text(t) = &app.session.active().unwrap().doc.layer(id).unwrap().content else {
        panic!("the layer is not a type layer");
    };
    t.text.clone()
}

fn first_size(app: &PhotocraftApp, id: LayerId) -> f32 {
    let LayerContent::Text(t) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { return 0.0 };
    t.char_runs().first().map(|r| r.style.size_pt).unwrap_or_default()
}

/// Click into the existing layer: an edit session with nothing changed yet.
fn click_into(app: &mut PhotocraftApp, id: LayerId) {
    crate::type_tool::edit_active(app).unwrap();
    let ed = app.ui.text_edit.as_ref().expect("the edit session is open");
    assert_eq!(ed.layer, id.0, "the session edits the layer");
    assert!(!ed.created, "it is an existing layer");
}

/// What ending the session with `how` left behind: exactly one state, named like any other edit
/// of an existing type layer, on an unchanged layer.
fn ended(app: &PhotocraftApp, id: LayerId, before: &[String], how: &str) {
    let after = history(app);
    assert!(app.ui.text_edit.is_none(), "{how}: the session ended");
    assert_eq!(after.len(), before.len() + 1, "{how}: exactly one new history state, in {after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Edit Type"), "{how}: the name other edits of an existing type layer carry");
    assert_eq!(text_of(app, id), "Hello", "{how}: the layer is unchanged");
}

/// Escape (with Preferences > Type > Use Escape to Commit on, the default), the Commit button and
/// Ctrl+Enter all end the session through `commit` / `commit_with`.
#[test]
fn an_unchanged_edit_records_one_edit_type_state_on_every_commit_path() {
    // How each gesture ends the session: Escape runs the plain `commit`, the Commit button and
    // Ctrl+Enter pass their held modifiers to `commit_with`.
    for (how, mods) in [("Escape (Use Escape to Commit)", None), ("Commit button", Some(egui::Modifiers::NONE)), ("Ctrl+Enter", Some(egui::Modifiers::COMMAND))]
    {
        let (mut app, id) = app();
        click_into(&mut app, id);
        let before = history(&app);
        match mods {
            None => crate::type_tool::commit(&mut app),
            Some(m) => crate::type_tool::commit_with(&mut app, m),
        }
        ended(&app, id, &before, how);
    }
}

/// A click on the canvas outside the text commits the session, and records it too.
#[test]
fn a_click_outside_the_text_records_one_edit_type_state() {
    let (mut app, id) = app();
    click_into(&mut app, id);
    let before = history(&app);
    // Far from the glyphs and from the box: the Type tool's press commits the session.
    assert!(crate::type_tool::pointer_down(&mut app, 380.0, 20.0, egui::Modifiers::NONE), "the Type tool took the click");
    ended(&app, id, &before, "click outside the text");
}

/// The Cancel button discards the session and adds no history state.
#[test]
fn cancel_still_adds_no_history_state() {
    let (mut app, id) = app();
    click_into(&mut app, id);
    let before = history(&app);
    crate::type_tool::cancel(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(history(&app), before, "Cancel adds nothing");
    assert_eq!(text_of(&app, id), "Hello");
}

/// An edit that did change something gives one state, not two: the commit joins the state the
/// session already opened instead of adding an empty one of its own.
#[test]
fn a_changed_edit_still_gives_one_state_not_two() {
    let (mut app, id) = app();
    click_into(&mut app, id);
    let before = history(&app);
    super::insert(&mut app, "x");
    assert_eq!(text_of(&app, id), "x", "the session changed the text");
    assert_eq!(history(&app).len(), before.len() + 1, "typing opened the session's state");
    crate::type_tool::commit(&mut app);
    let after = history(&app);
    assert_eq!(after.len(), before.len() + 1, "ending it adds no second state, in {after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Edit Type"));
}

/// Undo of the state an unchanged edit left behind takes the document back to where it was.
#[test]
fn undoing_the_state_of_an_unchanged_edit_leaves_the_document_as_it_was() {
    let (mut app, id) = app();
    let (text, size) = (text_of(&app, id), first_size(&app, id));
    click_into(&mut app, id);
    let before = history(&app);
    crate::type_tool::commit(&mut app);
    assert_eq!(history(&app).len(), before.len() + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(text_of(&app, id), text, "the text is what it was");
    assert_eq!(first_size(&app, id), size, "the style is what it was");
}

/// Not measured in #12050, so it keeps today's behaviour: an unchanged session ended because
/// another layer took the focus records nothing.
#[test]
fn an_unchanged_edit_ended_by_another_layer_records_nothing() {
    let (mut app, id) = app();
    // Made before the session opens, so its own history step is not counted.
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    click_into(&mut app, id);
    let before = history(&app);
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none(), "selecting another layer ends the session");
    assert_eq!(app.session.active().unwrap().active_layer, Some(other));
    assert_eq!(history(&app), before, "and adds no state to the edited layer's document");
    assert_eq!(text_of(&app, id), "Hello");
}

/// Also not measured, so unchanged: switching tools commits the session, and an unchanged one
/// still adds nothing.
#[test]
fn an_unchanged_edit_ended_by_another_tool_records_nothing() {
    let (mut app, id) = app();
    click_into(&mut app, id);
    let before = history(&app);
    app.ui.tool = Tool::Move;
    crate::type_tool::commit(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(history(&app), before, "the tool switch is not an edit of the layer");
    assert_eq!(text_of(&app, id), "Hello");
}
