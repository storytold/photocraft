//! #14604: Duplicate Layer proposes the layer's own name for another document or New, and "<name> copy" for the
//! source document; the copy keeps that name also beside a layer of the same name. A Background goes to another
//! document as a normal layer named "Background" and to New as a Background. A typed name is used as typed; an empty
//! As takes the destination's default. Dragging between documents keeps its names.

use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{set, top};

/// "source" (active: Background, then "Top") and "target" (Background, then a layer also named "Top").
fn app() -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20, "name": "source"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("file.new", json!({"width": 20, "height": 20, "name": "target"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.set_active(0);
    PhotocraftApp::new(s, crate::Services::default())
}

fn open(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "layer.duplicate", json!({})).unwrap();
    crate::dialogs::testing::show(app);
    top(app)
}

/// Chooses the Document entry `value` and shows the dialog once.
fn choose(app: &mut PhotocraftApp, dialog: DialogId, value: Value) {
    set(app, dialog, "document", value).unwrap();
    crate::dialogs::testing::show(app);
}

fn proposed(app: &PhotocraftApp, dialog: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == dialog).and_then(|d| d.fields).unwrap()["name"].clone()
}

fn doc_key(app: &PhotocraftApp, i: usize) -> Value {
    json!(app.session.documents()[i].doc.id.0.to_string())
}

fn ok(app: &mut PhotocraftApp, dialog: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, dialog).unwrap();
    for _ in 0..2 {
        crate::dialogs::testing::show(app);
    }
}

fn names(app: &PhotocraftApp, i: usize) -> Vec<String> {
    app.session.documents()[i].doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

fn count(app: &PhotocraftApp, i: usize, name: &str) -> usize {
    names(app, i).iter().filter(|n| *n == name).count()
}

#[test]
fn the_document_choice_rewrites_the_proposed_name() {
    let mut app = app();
    let dialog = open(&mut app);
    assert_eq!(proposed(&app, dialog), json!("Top copy"));
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    assert_eq!(proposed(&app, dialog), json!("Top"), "another document: the plain name");
    choose(&mut app, dialog, json!("new"));
    assert_eq!(proposed(&app, dialog), json!("Top"), "New: the plain name");
    let source = doc_key(&app, 0);
    choose(&mut app, dialog, source);
    assert_eq!(proposed(&app, dialog), json!("Top copy"), "the source again: the copy name");
}

#[test]
fn the_plain_name_lands_beside_a_layer_of_the_same_name_and_in_a_new_document() {
    let mut app = app();
    let dialog = open(&mut app);
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    ok(&mut app, dialog);
    assert_eq!(count(&app, 1, "Top"), 2, "{:?}", names(&app, 1));
    assert_eq!(count(&app, 1, "Top copy"), 0);
    app.session.set_active(0);
    let dialog = open(&mut app);
    choose(&mut app, dialog, json!("new"));
    ok(&mut app, dialog);
    let new = app.session.documents().len() - 1;
    assert_eq!(names(&app, new), vec!["Top".to_string()]);
}

#[test]
fn a_background_arrives_as_background_in_another_document_and_in_new() {
    let mut app = app();
    let bg = app.session.active().unwrap().doc.background().unwrap().id;
    app.session.select_layer(bg).unwrap();
    let dialog = open(&mut app);
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    ok(&mut app, dialog);
    let target = &app.session.documents()[1].doc;
    let copy = target.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.name == "Background").find(|l| !target.is_background(l.id));
    assert!(copy.is_some(), "a normal layer named Background: {:?}", names(&app, 1));
    app.session.set_active(0);
    app.session.select_layer(bg).unwrap();
    let dialog = open(&mut app);
    choose(&mut app, dialog, json!("new"));
    ok(&mut app, dialog);
    let new = &app.session.documents().last().unwrap().doc;
    assert!(new.background().is_some_and(|l| l.name == "Background"), "{:?}", names(&app, app.session.documents().len() - 1));
}

#[test]
fn a_typed_name_is_used_and_an_empty_one_takes_the_default() {
    let mut app = app();
    let dialog = open(&mut app);
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target.clone());
    set(&mut app, dialog, "name", json!("Typed copy")).unwrap();
    ok(&mut app, dialog);
    assert_eq!(count(&app, 1, "Typed copy"), 1);
    app.session.set_active(0);
    let dialog = open(&mut app);
    choose(&mut app, dialog, target);
    set(&mut app, dialog, "name", json!("")).unwrap();
    ok(&mut app, dialog);
    assert_eq!(count(&app, 1, "Top"), 2, "empty As into another document: the plain name");
    app.session.set_active(0);
    let dialog = open(&mut app);
    set(&mut app, dialog, "name", json!("")).unwrap();
    ok(&mut app, dialog);
    assert_eq!(count(&app, 0, "Top copy"), 1, "empty As in the source: the copy name");
}

#[test]
fn dragging_between_documents_keeps_its_names() {
    let mut app = app();
    let bg = app.session.active().unwrap().doc.background().unwrap().id;
    let before = names(&app, 1).len();
    app.run("layer.copyToDocument", json!({"source": 0, "document": 1, "layers": [bg.0]})).unwrap();
    let after = names(&app, 1);
    assert_eq!(after.len(), before + 1);
    assert!(!after.iter().any(|n| n == "Background copy"));
    assert_eq!(after.iter().filter(|n| *n == "Background").count(), 1, "a dragged Background is renamed as today: {after:?}");
}

#[test]
fn an_empty_name_for_a_background_into_another_document_is_background() {
    let mut app = app();
    let bg = app.session.active().unwrap().doc.background().unwrap().id;
    app.session.select_layer(bg).unwrap();
    let dialog = open(&mut app);
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    set(&mut app, dialog, "name", json!("")).unwrap();
    ok(&mut app, dialog);
    assert_eq!(count(&app, 1, "Background"), 2, "the target's own Background and the copy: {:?}", names(&app, 1));
}

/// A name the user typed survives a change of destination; only an untouched proposal is rewritten (CodeRabbit on #16090).
#[test]
fn a_typed_name_survives_a_change_of_destination() {
    let mut app = app();
    let dialog = open(&mut app);
    set(&mut app, dialog, "name", json!("Shadow")).unwrap();
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    assert_eq!(proposed(&app, dialog), json!("Shadow"), "another document");
    choose(&mut app, dialog, json!("new"));
    assert_eq!(proposed(&app, dialog), json!("Shadow"), "New");
    let source = doc_key(&app, 0);
    choose(&mut app, dialog, source);
    assert_eq!(proposed(&app, dialog), json!("Shadow"), "the source again");
}

/// A group is outside the new proposal policy: its "<name> copy" stays whatever the destination (CodeRabbit on #16090).
#[test]
fn a_groups_proposal_does_not_change_with_the_destination() {
    let mut app = app();
    app.run("layer.new.group", json!({"name": "Folder"})).unwrap();
    let dialog = open(&mut app);
    assert_eq!(proposed(&app, dialog), json!("Folder copy"));
    let target = doc_key(&app, 1);
    choose(&mut app, dialog, target);
    assert_eq!(proposed(&app, dialog), json!("Folder copy"), "another document");
    choose(&mut app, dialog, json!("new"));
    assert_eq!(proposed(&app, dialog), json!("Folder copy"), "New");
}
