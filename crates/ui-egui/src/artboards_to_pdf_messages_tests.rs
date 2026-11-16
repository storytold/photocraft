//! #13657: Artboards to PDF with a destination folder that doesn't exist keeps its dialog open with
//! its values and says so; a finished export says it was successful, with the PDF's path.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// A 64 × 64 document with one artboard, and the Artboards to PDF dialog open on it.
fn app_with_dialog() -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.export.artboardsToPdf", json!({})).unwrap();
    let id = app.ui.dialogs.last().expect("the dialog opens").id;
    (app, id)
}

fn set_path(app: &mut PhotocraftApp, id: u64, path: &str) {
    app.ui.dialog_mut(id).unwrap().fields.insert("path".into(), json!(path));
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect()
}

#[test]
fn a_missing_destination_keeps_the_dialog_open_and_says_so() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nodir").join("boards.pdf").to_string_lossy().into_owned();
    let (mut app, id) = app_with_dialog();
    set_path(&mut app, id, &path);
    let err = crate::dialogs::confirm(&mut app, id).unwrap_err();
    assert!(err.contains("Destination does not exist"), "{err}");
    let d = app.ui.dialogs.iter().find(|d| d.id == id).expect("the dialog stays open");
    assert_eq!(d.fields["path"], json!(path), "with its values");
    assert_eq!(alerts(&app), [err], "the error is shown");
    assert!(!root.path().join("nodir").exists(), "no folder is created");
}

#[test]
fn a_finished_export_says_it_was_successful_with_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boards.pdf").to_string_lossy().into_owned();
    let (mut app, id) = app_with_dialog();
    set_path(&mut app, id, &path);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.ui.dialogs.iter().all(|d| d.id != id), "the dialog closes");
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"));
    assert_eq!(alerts(&app), [format!("Artboards To PDF was successful.\n{path}")]);
    let alert = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).unwrap();
    assert_eq!(crate::dialogs::title(alert), "Artboards To PDF", "a success isn't titled Error");
}
