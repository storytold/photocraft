//! #13848: File › Scripts › Browse… opens again in the folder of the script picked last, also
//! after a restart; a cancelled picker changes nothing, and the other pickers keep their folders.

use std::{cell::RefCell, rc::Rc};

use serde_json::json;

use super::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
use crate::{PhotocraftApp, Services, menus};

type Asked = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app whose file dialogs wait for the test's answer.
fn new_app() -> (PhotocraftApp, Asked) {
    let asked: Asked = Default::default();
    let held = asked.clone();
    let services = Services { file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, asked)
}

/// Opens `command`'s picker and returns where it starts.
fn start_dir(app: &mut PhotocraftApp, asked: &Asked, command: &str) -> Option<String> {
    let ctx = egui::Context::default();
    let _ = menus::invoke(app, &ctx, command, json!({}));
    app.poll_file_dialog(&ctx, None);
    match asked.borrow().last().map(|(r, _)| r.clone()) {
        Some(FileDialogRequest::Open { initial_dir, .. }) => initial_dir,
        other => panic!("{command} asked {other:?}"),
    }
}

/// Answers the open picker with `answer` and lets the app take it.
fn answer(app: &mut PhotocraftApp, asked: &Asked, answer: Option<FileDialogAnswer>) {
    let (_, reply) = asked.borrow_mut().pop().expect("a picker is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
}

/// A script file in a new folder `name` under `root`.
fn script(root: &std::path::Path, name: &str) -> (String, String) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tidy-layers.jsx");
    std::fs::write(&file, "// an example script\n").unwrap();
    (dir.to_string_lossy().into_owned(), file.to_string_lossy().into_owned())
}

#[test]
fn the_next_browse_opens_in_the_folder_of_the_script_picked_last() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, asked) = new_app();
    let before = start_dir(&mut app, &asked, "file.scripts.browse");
    assert_eq!(before, super::last_used_dir(&app.ui.recent_files), "today's start folder before any pick");
    let (dir, file) = script(root.path(), "scripts");
    answer(&mut app, &asked, Some(FileDialogAnswer::Paths(vec![file])));
    assert_eq!(app.session.prefs().file_handling.scripts_folder, dir);
    assert_eq!(start_dir(&mut app, &asked, "file.scripts.browse"), Some(dir.clone()));
    // A pick in another folder moves it there.
    answer(&mut app, &asked, None);
    let (other, file) = script(root.path(), "more scripts");
    start_dir(&mut app, &asked, "file.scripts.browse");
    answer(&mut app, &asked, Some(FileDialogAnswer::Paths(vec![file])));
    assert_eq!(start_dir(&mut app, &asked, "file.scripts.browse"), Some(other));
}

#[test]
fn a_cancelled_picker_keeps_the_remembered_folder() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, asked) = new_app();
    let (dir, file) = script(root.path(), "scripts");
    start_dir(&mut app, &asked, "file.scripts.browse");
    answer(&mut app, &asked, Some(FileDialogAnswer::Paths(vec![file])));
    let rev = app.session.prefs.rev();
    start_dir(&mut app, &asked, "file.scripts.browse");
    answer(&mut app, &asked, None);
    assert_eq!(app.session.prefs().file_handling.scripts_folder, dir);
    assert_eq!(app.session.prefs.rev(), rev, "Cancel changes no preference");
    assert_eq!(start_dir(&mut app, &asked, "file.scripts.browse"), Some(dir));
}

#[test]
fn the_folder_is_kept_across_a_restart_and_older_settings_still_load() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, asked) = new_app();
    let (dir, file) = script(root.path(), "scripts");
    start_dir(&mut app, &asked, "file.scripts.browse");
    answer(&mut app, &asked, Some(FileDialogAnswer::Paths(vec![file])));
    let saved = app.session.prefs_value().to_string();
    // A restart: a new app loading the saved preferences.
    let (mut again, asked) = new_app();
    again.session.load_prefs_json(&saved).unwrap();
    assert_eq!(start_dir(&mut again, &asked, "file.scripts.browse"), Some(dir));
    // A preferences file from before has no such key: today's start folder.
    let mut older: serde_json::Value = serde_json::from_str(&saved).unwrap();
    older["fileHandling"].as_object_mut().unwrap().remove("scriptsFolder");
    let (mut old, asked) = new_app();
    assert!(old.session.load_prefs_json(&older.to_string()).unwrap().is_empty(), "nothing invalid");
    assert_eq!(old.session.prefs().file_handling.scripts_folder, "");
    assert_eq!(start_dir(&mut old, &asked, "file.scripts.browse"), super::last_used_dir(&old.ui.recent_files));
}

#[test]
fn a_folder_that_is_gone_and_the_other_pickers_keep_todays_start() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, asked) = new_app();
    let (dir, file) = script(root.path(), "scripts");
    start_dir(&mut app, &asked, "file.scripts.browse");
    answer(&mut app, &asked, Some(FileDialogAnswer::Paths(vec![file])));
    // File › Open doesn't start in the scripts folder.
    let open = start_dir(&mut app, &asked, "file.open");
    assert_ne!(open, Some(dir.clone()));
    assert_eq!(open, super::last_used_dir(&app.ui.recent_files));
    answer(&mut app, &asked, None);
    // A removed scripts folder isn't offered.
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(start_dir(&mut app, &asked, "file.scripts.browse"), super::last_used_dir(&app.ui.recent_files));
}
