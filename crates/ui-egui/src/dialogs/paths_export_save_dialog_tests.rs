//! #13778: Paths to Illustrator's first dialog has only the Paths choice; OK opens the save
//! dialog for the `.ai` file, named after the document with its last extension replaced. A name
//! without an extension gets `.ai`, an existing file is asked about first, and Cancel writes
//! nothing.

use super::*;
use crate::file_dialog::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
use std::{cell::RefCell, rc::Rc};

type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;
const COMMAND: &str = "file.export.pathsToIllustrator";

fn app(name: &str, existing: Option<String>) -> (PhotocraftApp, Open) {
    let open = Open::default();
    let held = open.clone();
    let services = crate::Services {
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        file_stamp: Some(Box::new(move |p: &str| (Some(p) == existing.as_deref()).then_some(crate::FileStamp { len: 1, modified: None }))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 300, "height": 200, "name": name})).unwrap();
    app.run("path.set", json!({"name": "Rect A", "path": {"subpaths": [{"closed": true, "knots": [[10.0, 10.0], [60.0, 10.0], [60.0, 40.0]]}]}})).unwrap();
    (app, open)
}

/// Opens the dialog as the menu item does and presses OK.
fn ok(app: &mut PhotocraftApp) -> u64 {
    let r = invoke(app, &egui::Context::default(), COMMAND, &json!({})).unwrap().unwrap();
    let id = r["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(app, id).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    id
}

fn answer(app: &mut PhotocraftApp, open: &Open, answer: Option<FileDialogAnswer>) {
    let (_, reply) = open.borrow_mut().pop().expect("the save dialog is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
}

fn suggested(open: &Open) -> String {
    match &open.borrow().last().expect("a save dialog").0 {
        FileDialogRequest::Save { suggested } => suggested.clone(),
        other => panic!("not a save dialog: {other:?}"),
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-paths-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_first_dialog_has_only_the_paths_choice() {
    let (mut app, _) = app("paths-test", None);
    let r = invoke(&mut app, &egui::Context::default(), COMMAND, &json!({})).unwrap().unwrap();
    let f = &app.ui.dialog_mut(r["dialog"].as_u64().unwrap()).unwrap().fields;
    let shown: Vec<&String> = f.keys().filter(|k| !k.starts_with("__")).collect();
    assert_eq!(shown, ["paths"], "no path text field");
}

#[test]
fn ok_asks_for_the_ai_file_named_after_the_document() {
    for (name, want) in [("paths-test", "paths-test.ai"), ("My Doc.v2.psd", "My Doc.v2.ai")] {
        let (mut app, open) = app(name, None);
        ok(&mut app);
        let s = suggested(&open);
        assert!(s.ends_with(want), "{name}: {s}");
    }
}

#[test]
fn a_name_without_extension_is_saved_with_ai() {
    let dir = scratch("ext");
    let (mut app, open) = app("paths-test", None);
    ok(&mut app);
    answer(&mut app, &open, Some(FileDialogAnswer::SaveTo(dir.join("chosen").to_string_lossy().into_owned())));
    let text = std::fs::read_to_string(dir.join("chosen.ai")).expect("written as chosen.ai");
    assert!(text.starts_with("%!PS-Adobe-2.0"));
    assert!(!dir.join("chosen").exists());
}

#[test]
fn cancel_writes_nothing() {
    let dir = scratch("cancel");
    let (mut app, open) = app("paths-test", None);
    ok(&mut app);
    answer(&mut app, &open, None);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

/// A name the extension completes to an existing file: the app asks first, and No writes nothing.
#[test]
fn an_existing_file_is_asked_about_and_no_writes_nothing() {
    let dir = scratch("replace");
    let target = dir.join("old.ai");
    std::fs::write(&target, "keep me").unwrap();
    let (mut app, open) = app("paths-test", Some(target.to_string_lossy().into_owned()));
    ok(&mut app);
    answer(&mut app, &open, Some(FileDialogAnswer::SaveTo(dir.join("old").to_string_lossy().into_owned())));
    assert!(app.ui.dialogs.iter().any(|d| crate::replace_prompt::owns(&d.fields)), "asked before replacing");
    let _ = app.answer_replace(false);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
}

/// CodeRabbit on #15067: the save dialog answers on a later frame. A document activated meanwhile does
/// not take the export: the file holds the paths of the document whose OK was pressed.
#[test]
fn the_file_is_the_document_that_asked_even_if_another_is_active_by_then() {
    let dir = scratch("other-doc");
    let (mut app, open) = app("first", None);
    let first = app.session.active().unwrap().doc.id;
    ok(&mut app);
    app.run("file.new", json!({"width": 300, "height": 200, "name": "second"})).unwrap();
    app.run("path.set", json!({"name": "Rect A", "path": {"subpaths": [{"closed": true, "knots": [[200.0, 10.0], [260.0, 10.0], [260.0, 40.0]]}]}})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id, first, "another document is active while the picker is open");
    answer(&mut app, &open, Some(FileDialogAnswer::SaveTo(dir.join("out.ai").to_string_lossy().into_owned())));
    let text = std::fs::read_to_string(dir.join("out.ai")).expect("written");
    assert!(text.contains("10.0000 190.0000 m") && !text.contains("200.0000 190.0000 m"), "the first document's paths: {text}");
    assert_eq!(app.session.active().unwrap().doc.id, first, "and it is the active one again");
}
