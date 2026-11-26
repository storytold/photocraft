//! #14110: File › Place Embedded's picker is titled "Place Embedded"; every other file picker keeps
//! the system's own title. The requests are read from a scripted file service, so no dialog opens.

use super::*;
use crate::{Services, menus};
use craft::host::ScriptedFiles;
use std::sync::Arc;

/// An app with an untitled document, over a file service that keeps its dialogs open.
fn app() -> (Box<PhotocraftApp>, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::held());
    let services = Services { files: Some(files.clone()), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, files)
}

/// The newest request the file service was given.
fn last_request(files: &ScriptedFiles) -> PickRequest {
    files.requests().pop().expect("a request")
}

/// Cancels the newest dialog, so the next picker can open (one dialog at a time).
fn cancel(app: &mut PhotocraftApp, files: &ScriptedFiles) {
    files.answer(Picked::Cancelled).expect("a dialog is open");
    app.poll_file_dialog();
}

#[test]
fn place_embedded_asks_for_a_picker_titled_place_embedded() {
    let (mut app, files) = app();
    menus::invoke(&mut app, &egui::Context::default(), "file.placeEmbedded", json!({})).unwrap();
    let request = last_request(&files);
    assert_eq!(request.kind, PickKind::OpenMany);
    assert_eq!(request.title.as_deref(), Some("Place Embedded"));
}

#[test]
fn place_embedded_keeps_the_pickers_other_settings() {
    // Only the title is new: several files, with no extension filter, from the last-used folder.
    let (mut app, files) = app();
    menus::invoke(&mut app, &egui::Context::default(), "file.placeEmbedded", json!({})).unwrap();
    let titled = last_request(&files);
    cancel(&mut app, &files);
    app.pick_files_bytes(None, |_, _| Ok(Value::Null)).unwrap();
    let plain = last_request(&files);
    assert_eq!(PickRequest { title: None, ..titled }, plain);
}

#[test]
fn the_other_open_pickers_keep_the_system_title() {
    let (mut app, files) = app();
    let ctx = egui::Context::default();

    menus::invoke(&mut app, &ctx, "file.placeLinked", json!({})).unwrap();
    assert_eq!(last_request(&files).title, None, "Place Linked");
    cancel(&mut app, &files);

    app.open_dialog_file().unwrap();
    assert_eq!(last_request(&files).title, None, "File › Open");
    cancel(&mut app, &files);

    app.open_dialog_as_smart_object().unwrap();
    assert_eq!(last_request(&files).title, None, "File › Open as Smart Object");
    cancel(&mut app, &files);

    app.pick_file_bytes_filtered(&["cube", "3dl", "look"], |_, _, _| Ok(Value::Null)).unwrap();
    assert_eq!(last_request(&files).title, None, "a filtered picker (LUTs, scripts)");
    cancel(&mut app, &files);

    app.pick_file_bytes(|_, _, _| Ok(Value::Null)).unwrap();
    assert_eq!(last_request(&files).title, None, "the plain single-file picker");
    cancel(&mut app, &files);

    app.pick_files_bytes(None, |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(last_request(&files).title, None, "the untitled multi-file picker");
}
